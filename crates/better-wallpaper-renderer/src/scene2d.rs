use std::collections::{HashMap, HashSet};

use better_wallpaper_scene_format::{
    BlendMode, MaterialManifest, ModelManifest, PkgReader, SceneGraph, SceneNodeKind, TexTexture,
    TextureImage, resolve_texture_path,
};
use thiserror::Error;
use tracing::{debug, warn};

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

    fn rotation(radians: f32) -> Self {
        let (sin, cos) = radians.sin_cos();
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

/// A draw whose model/material/texture dependency chain has been fully resolved.
/// This is the shared CPU-side submission boundary used by niri and Plasma.
#[derive(Debug, Clone)]
pub struct Scene2dDraw {
    pub quad: Scene2dQuad,
    pub blend_mode: BlendMode,
    pub texture_path: String,
    pub texture: TextureImage,
    pub animation: Option<SpriteAnimation>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SpriteFrame {
    /// Normalized top-left and bottom-right texture coordinates.
    pub uv: [f32; 4],
    pub duration_seconds: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SpriteAnimation {
    pub frames: Vec<SpriteFrame>,
    pub duration_seconds: f32,
}

impl SpriteAnimation {
    pub fn uv_at(&self, elapsed_seconds: f64) -> [f32; 4] {
        if self.frames.is_empty() || self.duration_seconds <= 0.0 {
            return [0.0, 0.0, 1.0, 1.0];
        }
        let mut cursor = elapsed_seconds.rem_euclid(f64::from(self.duration_seconds)) as f32;
        for frame in &self.frames {
            if cursor < frame.duration_seconds {
                return frame.uv;
            }
            cursor -= frame.duration_seconds;
        }
        self.frames
            .last()
            .map_or([0.0, 0.0, 1.0, 1.0], |frame| frame.uv)
    }
}

#[derive(Debug, Clone)]
pub struct Scene2dAssets {
    pub draws: Vec<Scene2dDraw>,
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
    #[error("scene image node {node} references unknown model {model}")]
    MissingModel { node: String, model: String },
    #[error("scene model {model} references unknown material {material}")]
    MissingMaterial { model: String, material: String },
    #[error("scene material {0} has no render pass")]
    MissingMaterialPass(String),
    #[error("scene material {0} uses an unsupported blend mode")]
    UnsupportedBlendMode(String),
    #[error("scene material {0} does not reference exactly one texture")]
    UnsupportedTextureCount(String),
    #[error("scene texture is missing from package: {0}")]
    MissingTexture(String),
    #[error("scene texture {path} is invalid: {detail}")]
    InvalidTexture { path: String, detail: String },
    #[error("scene asset manifest is invalid: {0}")]
    InvalidAssetManifest(String),
}

/// Resolves a validated draw plan into immutable texture payloads without
/// creating GPU objects. Desktop backends can upload this data in their own GL
/// context while retaining identical scene semantics.
pub fn resolve_scene_2d_assets(
    package: &PkgReader,
    plan: Scene2dPlan,
) -> Result<Scene2dAssets, Scene2dError> {
    let models = ModelManifest::from_package(package)
        .map_err(|error| Scene2dError::InvalidAssetManifest(error.to_string()))?;
    let materials = MaterialManifest::from_package(package, &models)
        .map_err(|error| Scene2dError::InvalidAssetManifest(error.to_string()))?;
    let model_by_path = models
        .models
        .iter()
        .map(|model| (model.path.as_str(), model.clone()))
        .collect::<HashMap<_, _>>();
    let material_by_path = materials
        .materials
        .iter()
        .map(|material| (material.path.as_str(), material.clone()))
        .collect::<HashMap<_, _>>();

    let mut draws = Vec::with_capacity(plan.quads.len());
    let mut skipped_nodes = plan.skipped_nodes;
    for quad in plan.quads {
        let resolved = (|| {
            let model = model_by_path.get(quad.resource.as_str()).ok_or_else(|| {
                Scene2dError::MissingModel {
                    node: quad.node_id.clone(),
                    model: quad.resource.clone(),
                }
            })?;
            let material_path = &model.definition.material;
            let material = material_by_path
                .get(material_path.as_str())
                .ok_or_else(|| Scene2dError::MissingMaterial {
                    model: model.path.clone(),
                    material: material_path.clone(),
                })?;
            let pass = material
                .definition
                .passes
                .first()
                .ok_or_else(|| Scene2dError::MissingMaterialPass(material.path.clone()))?;
            if matches!(pass.blend_mode, BlendMode::Unknown(_)) {
                return Err(Scene2dError::UnsupportedBlendMode(material.path.clone()));
            }
            let [texture_name] = pass.textures.as_slice() else {
                return Err(Scene2dError::UnsupportedTextureCount(material.path.clone()));
            };
            let texture_path = resolve_texture_path(&material.path, texture_name);
            let entry = package
                .find(&texture_path)
                .ok_or_else(|| Scene2dError::MissingTexture(texture_path.clone()))?;
            let bytes = package.read_entry(entry);
            let parsed =
                TexTexture::parse(bytes).map_err(|error| Scene2dError::InvalidTexture {
                    path: texture_path.clone(),
                    detail: error.to_string(),
                })?;
            let animation = sprite_animation(&parsed);
            let texture =
                parsed
                    .to_texture_image()
                    .map_err(|error| Scene2dError::InvalidTexture {
                        path: texture_path.clone(),
                        detail: error.to_string(),
                    })?;
            Ok(Scene2dDraw {
                quad,
                blend_mode: pass.blend_mode.clone(),
                texture_path,
                texture,
                animation,
            })
        })();
        match resolved {
            Ok(draw) => draws.push(draw),
            Err(error) => {
                skipped_nodes += 1;
                warn!(%error, "Skipping unsupported scene image node");
            }
        }
    }
    Ok(Scene2dAssets {
        draws,
        skipped_nodes,
    })
}

fn sprite_animation(texture: &TexTexture) -> Option<SpriteAnimation> {
    let width = texture.texture_width.max(texture.width) as f32;
    let height = texture.texture_height.max(texture.height) as f32;
    if width <= 0.0 || height <= 0.0 || texture.frames.len() < 2 {
        return None;
    }
    let frames = texture
        .frames
        .iter()
        .filter_map(|frame| {
            if !frame.frametime.is_finite()
                || frame.frametime <= 0.0
                || !frame.x.is_finite()
                || !frame.y.is_finite()
                || !frame.width.is_finite()
                || !frame.height.is_finite()
                || frame.x < 0.0
                || frame.y < 0.0
                || frame.width <= 0.0
                || frame.height <= 0.0
                || frame.x + frame.width > width
                || frame.y + frame.height > height
            {
                return None;
            }
            Some(SpriteFrame {
                uv: [
                    frame.x / width,
                    frame.y / height,
                    (frame.x + frame.width) / width,
                    (frame.y + frame.height) / height,
                ],
                duration_seconds: frame.frametime,
            })
        })
        .collect::<Vec<_>>();
    if frames.len() < 2 {
        return None;
    }
    let duration_seconds = frames
        .iter()
        .map(|frame| frame.duration_seconds)
        .sum::<f32>();
    if !duration_seconds.is_finite() || duration_seconds <= 0.0 {
        return None;
    }
    Some(SpriteAnimation {
        frames,
        duration_seconds,
    })
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
    // Wallpaper Engine stores the orthographic camera X/Y as an offset from
    // the centre of the project canvas. Scene objects, on the other hand, use
    // canvas coordinates (for example 1920,1080 in a 3840x2160 project).
    // Treating the camera value as an absolute canvas position sends every
    // layer far outside the viewport for ordinary scene projects.
    let camera_offset = graph
        .camera
        .center
        .unwrap_or(better_wallpaper_scene_format::Vec3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        });
    let center_x = projection.x * 0.5 + camera_offset.x;
    let center_y = projection.y * 0.5 + camera_offset.y;
    let view = Mat3::scale(2.0 / projection.x, -2.0 / projection.y)
        * Mat3::translation(-center_x, -center_y);
    debug!(
        projection_width = projection.x,
        projection_height = projection.y,
        camera_offset_x = camera_offset.x,
        camera_offset_y = camera_offset.y,
        view_center_x = center_x,
        view_center_y = center_y,
        "Building scene layout from project canvas coordinates"
    );

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

    #[test]
    fn sprite_animation_uses_frame_durations_and_loops() {
        let animation = SpriteAnimation {
            frames: vec![
                SpriteFrame {
                    uv: [0.0, 0.0, 0.5, 1.0],
                    duration_seconds: 0.1,
                },
                SpriteFrame {
                    uv: [0.5, 0.0, 1.0, 1.0],
                    duration_seconds: 0.2,
                },
            ],
            duration_seconds: 0.3,
        };
        assert_eq!(animation.uv_at(0.05), [0.0, 0.0, 0.5, 1.0]);
        assert_eq!(animation.uv_at(0.15), [0.5, 0.0, 1.0, 1.0]);
        assert_eq!(animation.uv_at(0.35), [0.0, 0.0, 0.5, 1.0]);
    }

    fn plan(json: &str) -> Result<Scene2dPlan, Scene2dError> {
        build_scene_2d_plan(
            &parse_scene_graph(json).unwrap(),
            Scene2dOptions {
                viewport_width: 1920,
                viewport_height: 1080,
            },
        )
    }

    fn sized_string(value: &str) -> Vec<u8> {
        let mut bytes = (value.len() as u32).to_le_bytes().to_vec();
        bytes.extend_from_slice(value.as_bytes());
        bytes
    }

    fn package(entries: &[(&str, Vec<u8>)]) -> PkgReader {
        let mut bytes = sized_string("PKGV0001");
        bytes.extend_from_slice(&(entries.len() as u32).to_le_bytes());
        let mut offset = 0u32;
        for (name, data) in entries {
            bytes.extend_from_slice(&sized_string(name));
            bytes.extend_from_slice(&offset.to_le_bytes());
            bytes.extend_from_slice(&(data.len() as u32).to_le_bytes());
            offset += data.len() as u32;
        }
        for (_, data) in entries {
            bytes.extend_from_slice(data);
        }
        PkgReader::parse(bytes).unwrap()
    }

    fn rgba_tex(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = b"TEXV0005\0TEXI0001\0".to_vec();
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        for value in [width, height, width, height, 0] {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        bytes.extend_from_slice(b"TEXB0003\0");
        for value in [13u32, 1, 1, width, height, 0, 0, width * height * 4] {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        bytes.resize(bytes.len() + (width * height * 4) as usize, 255);
        bytes
    }

    #[test]
    fn builds_centered_quad_in_source_order() {
        let result = plan(
            r#"{
            "general":{"orthogonalprojection":{"width":100,"height":100}},
            "camera":{"center":"0 0 -1"},
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
    fn applies_camera_as_project_center_offset() {
        let result = plan(
            r#"{
                "general":{"orthogonalprojection":{"width":3840,"height":2160}},
                "camera":{"center":"85.98602 -93.05257 -1"},
                "objects":[{
                    "id":"background",
                    "image":"models/background.json",
                    "origin":"2005.98602 986.94743 0",
                    "size":"3840 2160"
                }]
            }"#,
        )
        .unwrap();

        assert!((result.quads[0].vertices[0][0] + 1.0).abs() < 0.0001);
        assert!((result.quads[0].vertices[0][1] - 1.0).abs() < 0.0001);
        assert!((result.quads[0].vertices[2][0] - 1.0).abs() < 0.0001);
        assert!((result.quads[0].vertices[2][1] + 1.0).abs() < 0.0001);
    }

    #[test]
    fn interprets_scene_angles_as_radians() {
        let result = plan(
            r#"{
                "general":{"orthogonalprojection":{"width":100,"height":100}},
                "objects":[{
                    "id":"rotated",
                    "image":"models/a.json",
                    "origin":"50 50 0",
                    "size":"20 10",
                    "angles":"0 0 1.57079632679"
                }]
            }"#,
        )
        .unwrap();

        let top_left = result.quads[0].vertices[0];
        assert!((top_left[0] - 0.1).abs() < 0.0001);
        assert!((top_left[1] - 0.2).abs() < 0.0001);
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

    #[test]
    fn resolves_shared_model_material_texture_submission() {
        let package = package(&[
            (
                "models/bg.json",
                br#"{"material":"materials/bg.json"}"#.to_vec(),
            ),
            (
                "materials/bg.json",
                br#"{"passes":[{"blending":"translucent","shader":"genericimage4","textures":["bg"]}]}"#.to_vec(),
            ),
            ("materials/bg.tex", rgba_tex(2, 2)),
        ]);
        let plan =
            plan(r#"{"objects":[{"id":"bg","image":"models/bg.json","size":"100 100"}]}"#).unwrap();

        let assets = resolve_scene_2d_assets(&package, plan).unwrap();
        assert_eq!(assets.draws.len(), 1);
        assert_eq!(assets.draws[0].texture_path, "materials/bg.tex");
        assert_eq!(assets.draws[0].blend_mode, BlendMode::Translucent);
        assert_eq!(assets.draws[0].texture.levels[0].data.len(), 16);
    }

    #[test]
    fn skips_an_unsupported_draw_without_discarding_valid_draws() {
        let package = package(&[
            (
                "models/good.json",
                br#"{"material":"materials/good.json"}"#.to_vec(),
            ),
            (
                "models/unsupported.json",
                br#"{"material":"materials/unsupported.json"}"#.to_vec(),
            ),
            (
                "materials/good.json",
                br#"{"passes":[{"blending":"translucent","shader":"genericimage4","textures":["good"]}]}"#.to_vec(),
            ),
            (
                "materials/unsupported.json",
                br#"{"passes":[{"blending":"unimplemented","shader":"genericimage4","textures":["bad"]}]}"#.to_vec(),
            ),
            ("materials/good.tex", rgba_tex(1, 1)),
        ]);
        let plan = plan(
            r#"{"objects":[
                {"id":"good","image":"models/good.json","size":"10 10"},
                {"id":"unsupported","image":"models/unsupported.json","size":"10 10"}
            ]}"#,
        )
        .unwrap();

        let assets = resolve_scene_2d_assets(&package, plan).unwrap();
        assert_eq!(assets.draws.len(), 1);
        assert_eq!(assets.draws[0].quad.node_id, "good");
        assert_eq!(assets.skipped_nodes, 1);
    }

    #[test]
    fn skips_builtin_post_processing_layers_instead_of_covering_real_images() {
        let package = package(&[
            (
                "models/bg.json",
                br#"{"material":"materials/bg.json"}"#.to_vec(),
            ),
            (
                "materials/bg.json",
                br#"{"passes":[{"blending":"translucent","shader":"genericimage4","textures":["bg"]}]}"#.to_vec(),
            ),
            ("materials/bg.tex", rgba_tex(1, 1)),
        ]);
        let plan = plan(
            r#"{"objects":[
                {"id":"bg","image":"models/bg.json","size":"100 100"},
                {"id":"fx","image":"models/util/projectlayer.json","size":"100 100"}
            ]}"#,
        )
        .unwrap();

        let assets = resolve_scene_2d_assets(&package, plan).unwrap();
        assert_eq!(assets.draws.len(), 1);
        assert_eq!(assets.draws[0].quad.node_id, "bg");
        assert_eq!(assets.skipped_nodes, 1);
    }
}
