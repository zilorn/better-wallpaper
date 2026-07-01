use std::collections::{HashMap, HashSet};

use better_wallpaper_scene_format::{SceneGraph, SceneNodeKind};
use thiserror::Error;

/// Column-major affine 2D matrix. Points are multiplied as `matrix * [x, y, 1]`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Mat3(pub [f32; 9]);

impl Mat3 {
    pub const IDENTITY: Self = Self([1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]);

    pub fn transform_point(self, point: [f32; 2]) -> [f32; 2] {
        let m = self.0;
        [
            m[0] * point[0] + m[3] * point[1] + m[6],
            m[1] * point[0] + m[4] * point[1] + m[7],
        ]
    }

    fn translation(x: f32, y: f32) -> Self {
        Self([1.0, 0.0, 0.0, 0.0, 1.0, 0.0, x, y, 1.0])
    }

    fn scale(x: f32, y: f32) -> Self {
        Self([x, 0.0, 0.0, 0.0, y, 0.0, 0.0, 0.0, 1.0])
    }

    fn rotation(degrees: f32) -> Self {
        let (sin, cos) = degrees.to_radians().sin_cos();
        Self([cos, sin, 0.0, -sin, cos, 0.0, 0.0, 0.0, 1.0])
    }
}

impl std::ops::Mul for Mat3 {
    type Output = Self;

    fn mul(self, rhs: Self) -> Self::Output {
        let mut out = [0.0; 9];
        for column in 0..3 {
            for row in 0..3 {
                out[column * 3 + row] = (0..3)
                    .map(|index| self.0[index * 3 + row] * rhs.0[column * 3 + index])
                    .sum();
            }
        }
        Self(out)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Scene2dOptions {
    pub viewport_width: u32,
    pub viewport_height: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Scene2dQuad {
    pub node_id: String,
    pub resource: String,
    /// Quad corners in NDC, ordered top-left, top-right, bottom-right, bottom-left.
    pub vertices: [[f32; 2]; 4],
    pub opacity: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Scene2dPlan {
    pub quads: Vec<Scene2dQuad>,
    pub skipped_nodes: usize,
}

#[derive(Debug, Error, PartialEq)]
pub enum Scene2dError {
    #[error("scene viewport must be non-zero")]
    EmptyViewport,
    #[error("scene projection must contain finite positive dimensions")]
    InvalidProjection,
    #[error("duplicate scene node id: {0}")]
    DuplicateNodeId(String),
    #[error("scene node {node} references missing parent {parent}")]
    MissingParent { node: String, parent: String },
    #[error("scene node parent cycle contains: {0}")]
    ParentCycle(String),
    #[error("scene node {0} has an invalid opacity")]
    InvalidOpacity(String),
}

/// Builds a deterministic backend-independent list of 2D draws from validated scene IR.
pub fn build_scene_2d_plan(
    graph: &SceneGraph,
    options: Scene2dOptions,
) -> Result<Scene2dPlan, Scene2dError> {
    if options.viewport_width == 0 || options.viewport_height == 0 {
        return Err(Scene2dError::EmptyViewport);
    }
    let mut indexes = HashMap::with_capacity(graph.nodes.len());
    for (index, node) in graph.nodes.iter().enumerate() {
        if indexes.insert(node.id.as_str(), index).is_some() {
            return Err(Scene2dError::DuplicateNodeId(node.id.clone()));
        }
    }
    for node in &graph.nodes {
        if let Some(parent) = node.parent.as_deref()
            && !indexes.contains_key(parent)
        {
            return Err(Scene2dError::MissingParent {
                node: node.id.clone(),
                parent: parent.to_owned(),
            });
        }
    }

    let projection = graph
        .camera
        .projection_size
        .unwrap_or(better_wallpaper_scene_format::Vec2 {
            x: options.viewport_width as f32,
            y: options.viewport_height as f32,
        });
    if !projection.x.is_finite()
        || !projection.y.is_finite()
        || projection.x <= 0.0
        || projection.y <= 0.0
    {
        return Err(Scene2dError::InvalidProjection);
    }
    let center = graph
        .camera
        .center
        .unwrap_or(better_wallpaper_scene_format::Vec3 {
            x: projection.x * 0.5,
            y: projection.y * 0.5,
            z: 0.0,
        });
    let view = Mat3::scale(2.0 / projection.x, -2.0 / projection.y)
        * Mat3::translation(-center.x, -center.y);

    let mut worlds = vec![None; graph.nodes.len()];
    let mut visible = vec![None; graph.nodes.len()];
    let mut visiting = HashSet::new();
    for index in 0..graph.nodes.len() {
        resolve_node(
            index,
            graph,
            &indexes,
            &mut worlds,
            &mut visible,
            &mut visiting,
        )?;
    }

    let mut quads = Vec::new();
    let mut skipped_nodes = 0;
    for (index, node) in graph.nodes.iter().enumerate() {
        let SceneNodeKind::Image(resource) = &node.kind else {
            skipped_nodes += 1;
            continue;
        };
        if !visible[index].unwrap_or(false) {
            continue;
        }
        let Some(size) = node.transform.size else {
            skipped_nodes += 1;
            continue;
        };
        let opacity = node.transform.opacity.unwrap_or(1.0);
        if !opacity.is_finite() || !(0.0..=1.0).contains(&opacity) {
            return Err(Scene2dError::InvalidOpacity(node.id.clone()));
        }
        let transform = view * worlds[index].unwrap_or(Mat3::IDENTITY);
        let half_x = size.x * 0.5;
        let half_y = size.y * 0.5;
        quads.push(Scene2dQuad {
            node_id: node.id.clone(),
            resource: resource.clone(),
            vertices: [
                transform.transform_point([-half_x, -half_y]),
                transform.transform_point([half_x, -half_y]),
                transform.transform_point([half_x, half_y]),
                transform.transform_point([-half_x, half_y]),
            ],
            opacity,
        });
    }
    Ok(Scene2dPlan {
        quads,
        skipped_nodes,
    })
}

fn resolve_node(
    index: usize,
    graph: &SceneGraph,
    indexes: &HashMap<&str, usize>,
    worlds: &mut [Option<Mat3>],
    visible: &mut [Option<bool>],
    visiting: &mut HashSet<usize>,
) -> Result<(), Scene2dError> {
    if worlds[index].is_some() {
        return Ok(());
    }
    if !visiting.insert(index) {
        return Err(Scene2dError::ParentCycle(graph.nodes[index].id.clone()));
    }
    let node = &graph.nodes[index];
    let origin = node
        .transform
        .origin
        .unwrap_or(better_wallpaper_scene_format::Vec3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        });
    let scale = node
        .transform
        .scale
        .unwrap_or(better_wallpaper_scene_format::Vec3 {
            x: 1.0,
            y: 1.0,
            z: 1.0,
        });
    let rotation = node.transform.angles.map_or(0.0, |angles| angles.z);
    let local = Mat3::translation(origin.x, origin.y)
        * Mat3::rotation(rotation)
        * Mat3::scale(scale.x, scale.y);
    let (world, inherited_visibility) = if let Some(parent) = node.parent.as_deref() {
        let parent_index = indexes[parent];
        resolve_node(parent_index, graph, indexes, worlds, visible, visiting)?;
        (
            worlds[parent_index].unwrap_or(Mat3::IDENTITY) * local,
            visible[parent_index].unwrap_or(false),
        )
    } else {
        (local, true)
    };
    worlds[index] = Some(world);
    visible[index] = Some(inherited_visibility && node.visible);
    visiting.remove(&index);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use better_wallpaper_scene_format::parse_scene_graph;

    fn plan(json: &str) -> Result<Scene2dPlan, Scene2dError> {
        build_scene_2d_plan(
            &parse_scene_graph(json).unwrap(),
            Scene2dOptions {
                viewport_width: 1920,
                viewport_height: 1080,
            },
        )
    }

    #[test]
    fn builds_centered_quad_in_source_order() {
        let result = plan(
            r#"{
            "general":{"orthogonalprojection":{"width":100,"height":100}},
            "camera":{"center":"50 50 0"},
            "objects":[
                {"id":1,"image":"models/a.json","origin":"25 50 0","size":"50 100"},
                {"id":2,"image":"models/b.json","origin":"75 50 0","size":"50 100","alpha":0.5}
            ]
        }"#,
        )
        .unwrap();
        assert_eq!(result.quads[0].vertices[0], [-1.0, 1.0]);
        assert_eq!(result.quads[0].vertices[2], [0.0, -1.0]);
        assert_eq!(result.quads[1].resource, "models/b.json");
        assert_eq!(result.quads[1].opacity, 0.5);
    }

    #[test]
    fn composes_parent_transform_and_visibility() {
        let result = plan(r#"{
            "general":{"orthogonalprojection":{"width":100,"height":100}},
            "objects":[
                {"id":"parent","container":true,"origin":"50 50 0","scale":"2 2 1","visible":false},
                {"id":"child","parent":"parent","image":"models/a.json","origin":"10 0 0","size":"10 10"}
            ]
        }"#).unwrap();
        assert!(result.quads.is_empty());
    }

    #[test]
    fn rejects_invalid_hierarchy() {
        let duplicate = plan(r#"{"objects":[{"id":1},{"id":1}]}"#).unwrap_err();
        assert_eq!(duplicate, Scene2dError::DuplicateNodeId("1".into()));

        let missing = plan(r#"{"objects":[{"id":1,"parent":2}]}"#).unwrap_err();
        assert!(matches!(missing, Scene2dError::MissingParent { .. }));

        let cycle = plan(r#"{"objects":[{"id":1,"parent":2},{"id":2,"parent":1}]}"#).unwrap_err();
        assert!(matches!(cycle, Scene2dError::ParentCycle(_)));
    }

    #[test]
    fn rejects_invalid_projection_and_opacity() {
        assert_eq!(
            plan(r#"{"general":{"orthogonalprojection":{"width":0,"height":100}}}"#).unwrap_err(),
            Scene2dError::InvalidProjection
        );
        assert!(matches!(
            plan(r#"{"objects":[{"id":1,"image":"a","size":"1 1","alpha":2}]}"#),
            Err(Scene2dError::InvalidOpacity(_))
        ));
    }
}
