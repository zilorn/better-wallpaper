use std::collections::BTreeSet;

use serde_json::{Map, Value};

use crate::error::SceneParseError;

pub const MAX_SCENE_OBJECTS: usize = 16_384;

/// Version-independent, data-only scene representation consumed by future runtimes.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SceneGraph {
    pub camera: SceneCamera,
    pub nodes: Vec<SceneNode>,
    pub unsupported_features: Vec<UnsupportedFeature>,
}

#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct SceneCamera {
    pub eye: Option<Vec3>,
    pub center: Option<Vec3>,
    pub projection_size: Option<Vec2>,
}

#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Vec2 {
    pub x: f32,
    pub y: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Vec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SceneNode {
    pub id: String,
    pub name: String,
    pub parent: Option<String>,
    pub visible: bool,
    pub kind: SceneNodeKind,
    pub transform: SceneTransform,
    pub effects: Vec<String>,
    /// Fields retained by name so unsupported input cannot silently change rendering.
    pub unknown_fields: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", content = "resource", rename_all = "snake_case")]
pub enum SceneNodeKind {
    Image(String),
    Sound(String),
    Particle(String),
    Text,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct SceneTransform {
    pub origin: Option<Vec3>,
    pub size: Option<Vec2>,
    pub scale: Option<Vec3>,
    pub angles: Option<Vec3>,
    pub opacity: Option<f32>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
pub struct UnsupportedFeature {
    pub path: String,
    pub feature: String,
}

pub fn parse_scene_graph(scene_json: &str) -> Result<SceneGraph, SceneParseError> {
    let root: Value = serde_json::from_str(scene_json)?;
    let root = root
        .as_object()
        .ok_or_else(|| SceneParseError::InvalidValue {
            field: "root".into(),
            detail: "expected a JSON object".into(),
        })?;

    let camera = parse_camera(root)?;
    let objects = match root.get("objects") {
        None => &[][..],
        Some(Value::Array(objects)) => objects,
        Some(_) => {
            return Err(SceneParseError::InvalidValue {
                field: "objects".into(),
                detail: "expected an array".into(),
            });
        }
    };
    if objects.len() > MAX_SCENE_OBJECTS {
        return Err(SceneParseError::InvalidValue {
            field: "objects".into(),
            detail: format!("object count exceeds safety limit {MAX_SCENE_OBJECTS}"),
        });
    }

    let mut unsupported = BTreeSet::new();
    let mut nodes = Vec::with_capacity(objects.len());
    for (index, object) in objects.iter().enumerate() {
        let object = object
            .as_object()
            .ok_or_else(|| SceneParseError::InvalidValue {
                field: format!("objects[{index}]"),
                detail: "expected an object".into(),
            })?;
        nodes.push(parse_node(index, object, &mut unsupported)?);
    }

    Ok(SceneGraph {
        camera,
        nodes,
        unsupported_features: unsupported.into_iter().collect(),
    })
}

fn parse_camera(root: &Map<String, Value>) -> Result<SceneCamera, SceneParseError> {
    let camera = root.get("camera").and_then(Value::as_object);
    let projection = root
        .get("general")
        .and_then(Value::as_object)
        .and_then(|general| general.get("orthogonalprojection"))
        .and_then(Value::as_object);

    Ok(SceneCamera {
        eye: camera
            .and_then(|camera| camera.get("eye"))
            .map(|value| parse_vec3("camera.eye", value))
            .transpose()?,
        center: camera
            .and_then(|camera| camera.get("center"))
            .map(|value| parse_vec3("camera.center", value))
            .transpose()?,
        projection_size: projection
            .map(|projection| {
                let width = number_field(projection, "width", "general.orthogonalprojection")?;
                let height = number_field(projection, "height", "general.orthogonalprojection")?;
                Ok::<_, SceneParseError>(Vec2 {
                    x: width,
                    y: height,
                })
            })
            .transpose()?,
    })
}

fn parse_node(
    index: usize,
    object: &Map<String, Value>,
    unsupported: &mut BTreeSet<UnsupportedFeature>,
) -> Result<SceneNode, SceneParseError> {
    const KNOWN_FIELDS: &[&str] = &[
        "alpha",
        "angles",
        "effects",
        "id",
        "image",
        "instanceoverride",
        "name",
        "origin",
        "parent",
        "particle",
        "scale",
        "size",
        "sound",
        "text",
        "visible",
    ];
    let path = format!("objects[{index}]");
    let unknown_fields = object
        .keys()
        .filter(|key| !KNOWN_FIELDS.contains(&key.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    for field in &unknown_fields {
        unsupported.insert(UnsupportedFeature {
            path: format!("{path}.{field}"),
            feature: "unknown object field".into(),
        });
    }

    let kind = if let Some(resource) = string_resource(object, "particle", &path)? {
        mark(unsupported, &path, "particle system");
        SceneNodeKind::Particle(resource)
    } else if object.contains_key("instanceoverride") {
        mark(unsupported, &path, "particle instance override");
        SceneNodeKind::Particle(String::new())
    } else if let Some(resource) = string_resource(object, "sound", &path)? {
        mark(unsupported, &path, "sound object");
        SceneNodeKind::Sound(resource)
    } else if object.contains_key("text") {
        mark(unsupported, &path, "text object");
        SceneNodeKind::Text
    } else if let Some(resource) = string_resource(object, "image", &path)? {
        SceneNodeKind::Image(resource)
    } else {
        mark(unsupported, &path, "unknown object type");
        SceneNodeKind::Unknown
    };

    let effects = object
        .get("effects")
        .and_then(Value::as_array)
        .map(|effects| {
            effects
                .iter()
                .enumerate()
                .filter_map(|(effect_index, effect)| {
                    let file = effect.get("file").and_then(Value::as_str);
                    if file.is_none() {
                        mark(
                            unsupported,
                            &format!("{path}.effects[{effect_index}]"),
                            "effect without a file identifier",
                        );
                    }
                    file.map(str::to_owned)
                })
                .collect()
        })
        .unwrap_or_default();
    for effect in &effects {
        mark(unsupported, &path, &format!("effect: {effect}"));
    }

    Ok(SceneNode {
        id: scalar_id(object.get("id")).unwrap_or_else(|| format!("index-{index}")),
        name: object
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("<unnamed>")
            .to_owned(),
        parent: scalar_id(object.get("parent")),
        visible: object
            .get("visible")
            .and_then(Value::as_bool)
            .unwrap_or(true),
        kind,
        transform: SceneTransform {
            origin: optional_vec3(object, "origin", &path)?,
            size: optional_vec2(object, "size", &path)?,
            scale: optional_vec3(object, "scale", &path)?,
            angles: optional_vec3(object, "angles", &path)?,
            opacity: object
                .get("alpha")
                .map(|value| number_value(&format!("{path}.alpha"), value))
                .transpose()?,
        },
        effects,
        unknown_fields,
    })
}

fn string_resource(
    object: &Map<String, Value>,
    field: &str,
    path: &str,
) -> Result<Option<String>, SceneParseError> {
    let Some(value) = object.get(field) else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    let value = value
        .as_str()
        .ok_or_else(|| SceneParseError::InvalidValue {
            field: format!("{path}.{field}"),
            detail: "expected a resource path string".into(),
        })?;
    if value.is_empty() {
        return Ok(None);
    }
    validate_resource_path(value).map(Some)
}

fn validate_resource_path(path: &str) -> Result<String, SceneParseError> {
    let normalized = path.replace('\\', "/");
    let has_drive = normalized.len() >= 2
        && normalized.as_bytes()[0].is_ascii_alphabetic()
        && normalized.as_bytes()[1] == b':';
    if normalized.starts_with('/')
        || has_drive
        || normalized.contains('\0')
        || normalized.split('/').any(|part| part == "..")
    {
        return Err(SceneParseError::InvalidPath(path.into()));
    }
    Ok(normalized)
}

fn optional_vec3(
    object: &Map<String, Value>,
    field: &str,
    path: &str,
) -> Result<Option<Vec3>, SceneParseError> {
    object
        .get(field)
        .map(|value| parse_vec3(&format!("{path}.{field}"), value))
        .transpose()
}

fn optional_vec2(
    object: &Map<String, Value>,
    field: &str,
    path: &str,
) -> Result<Option<Vec2>, SceneParseError> {
    object
        .get(field)
        .map(|value| parse_vec2(&format!("{path}.{field}"), value))
        .transpose()
}

fn parse_vec3(field: &str, value: &Value) -> Result<Vec3, SceneParseError> {
    let values = vector_values(field, value, 3)?;
    Ok(Vec3 {
        x: values[0],
        y: values[1],
        z: values[2],
    })
}

fn parse_vec2(field: &str, value: &Value) -> Result<Vec2, SceneParseError> {
    let values = vector_values(field, value, 2)?;
    Ok(Vec2 {
        x: values[0],
        y: values[1],
    })
}

fn vector_values(field: &str, value: &Value, expected: usize) -> Result<Vec<f32>, SceneParseError> {
    let values = match value {
        Value::String(value) => value
            .split_whitespace()
            .map(|part| part.parse::<f32>().ok())
            .collect::<Option<Vec<_>>>(),
        Value::Array(values) => values
            .iter()
            .map(|value| value.as_f64().map(|number| number as f32))
            .collect::<Option<Vec<_>>>(),
        _ => None,
    }
    .filter(|values| values.len() == expected)
    .filter(|values| values.iter().all(|value| value.is_finite()))
    .ok_or_else(|| SceneParseError::InvalidValue {
        field: field.into(),
        detail: format!("expected {expected} finite numeric components"),
    })?;
    Ok(values)
}

fn number_field(
    object: &Map<String, Value>,
    field: &str,
    path: &str,
) -> Result<f32, SceneParseError> {
    let value = object
        .get(field)
        .ok_or_else(|| SceneParseError::MissingField(format!("{path}.{field}")))?;
    number_value(&format!("{path}.{field}"), value)
}

fn number_value(field: &str, value: &Value) -> Result<f32, SceneParseError> {
    let number = value
        .as_f64()
        .map(|number| number as f32)
        .filter(|number| number.is_finite())
        .ok_or_else(|| SceneParseError::InvalidValue {
            field: field.into(),
            detail: "expected a finite number".into(),
        })?;
    Ok(number)
}

fn scalar_id(value: Option<&Value>) -> Option<String> {
    match value {
        Some(Value::String(value)) => Some(value.clone()),
        Some(Value::Number(value)) => Some(value.to_string()),
        _ => None,
    }
}

fn mark(unsupported: &mut BTreeSet<UnsupportedFeature>, path: &str, feature: &str) {
    unsupported.insert(UnsupportedFeature {
        path: path.into(),
        feature: feature.into(),
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn produces_deterministic_ir_and_reports_unknown_fields() {
        let graph = parse_scene_graph(
            r#"{
                "camera":{"eye":"0 0 1","center":[0,0,0]},
                "general":{"orthogonalprojection":{"width":1920,"height":1080}},
                "objects":[{
                    "id":7,"name":"background","image":"models\\bg.json",
                    "origin":"1 2 3","size":[1920,1080],"visible":true,
                    "mystery":42
                }]
            }"#,
        )
        .unwrap();

        assert_eq!(graph.camera.projection_size.unwrap().x, 1920.0);
        assert_eq!(graph.nodes[0].id, "7");
        assert_eq!(
            graph.nodes[0].kind,
            SceneNodeKind::Image("models/bg.json".into())
        );
        assert_eq!(graph.nodes[0].unknown_fields, ["mystery"]);
        assert_eq!(
            graph.unsupported_features,
            [UnsupportedFeature {
                path: "objects[0].mystery".into(),
                feature: "unknown object field".into(),
            }]
        );

        let first = serde_json::to_string_pretty(&graph).unwrap();
        let second = serde_json::to_string_pretty(&graph).unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn rejects_escaping_resource_path() {
        let error = parse_scene_graph(r#"{"objects":[{"image":"../secret"}]}"#).unwrap_err();
        assert!(matches!(error, SceneParseError::InvalidPath(_)));
    }

    #[test]
    fn rejects_non_finite_or_malformed_vectors() {
        assert!(parse_scene_graph(r#"{"camera":{"eye":"NaN 0 1"}}"#).is_err());
        assert!(parse_scene_graph(r#"{"objects":[{"image":"x","size":[1]}]}"#).is_err());
    }

    #[test]
    fn reports_features_in_stable_order() {
        let graph = parse_scene_graph(
            r#"{"objects":[
                {"id":2,"particle":"particles/snow.json"},
                {"id":1,"sound":"audio/music.ogg"}
            ]}"#,
        )
        .unwrap();
        assert_eq!(graph.unsupported_features[0].path, "objects[0]");
        assert_eq!(graph.unsupported_features[1].path, "objects[1]");
    }
}
