use std::collections::{BTreeMap, BTreeSet};

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
    pub scroll: Option<ScrollEffect>,
    pub water_wave: Option<WaterWaveEffect>,
    pub water_flow: Option<WaterFlowEffect>,
    /// Fields retained by name so unsupported input cannot silently change rendering.
    pub unknown_fields: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ScrollEffect {
    pub speed_x: f32,
    pub speed_y: f32,
    pub repeat_x: f32,
    pub repeat_y: f32,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct WaterWaveEffect {
    pub direction: f32,
    pub scale: f32,
    pub speed: f32,
    pub strength: f32,
    pub mask: Option<String>,
    pub normal: Option<String>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct WaterFlowEffect {
    pub phase_scale: f32,
    pub speed: f32,
    pub strength: f32,
    pub mask: Option<String>,
    pub phase: Option<String>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", content = "resource", rename_all = "snake_case")]
pub enum SceneNodeKind {
    Image(String),
    Model(String),
    Sound(String),
    Particle(String),
    Text,
    Container,
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
    parse_scene_graph_with_properties(scene_json, &BTreeMap::new())
}

pub fn parse_scene_graph_with_properties(
    scene_json: &str,
    properties: &BTreeMap<String, String>,
) -> Result<SceneGraph, SceneParseError> {
    let mut root: Value = serde_json::from_str(scene_json)?;
    apply_property_overrides(&mut root, properties, 0)?;
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

fn apply_property_overrides(
    value: &mut Value,
    properties: &BTreeMap<String, String>,
    depth: usize,
) -> Result<(), SceneParseError> {
    if depth > 64 {
        return Err(SceneParseError::InvalidValue {
            field: "property binding".into(),
            detail: "binding nesting exceeds safety limit".into(),
        });
    }
    match value {
        Value::Array(values) => {
            for value in values {
                apply_property_overrides(value, properties, depth + 1)?;
            }
        }
        Value::Object(object) => {
            let binding = object.get("user").and_then(Value::as_str).and_then(|key| {
                properties
                    .get(key)
                    .map(|value| (key.to_owned(), value.clone()))
            });
            if let Some((key, override_value)) = binding
                && let Some(fallback) = object.get_mut("value")
            {
                *fallback = match fallback {
                    Value::Bool(_) => Value::Bool(override_value.parse().map_err(|_| {
                        SceneParseError::InvalidValue {
                            field: format!("scene.properties.{key}"),
                            detail: "expected true or false".into(),
                        }
                    })?),
                    Value::Number(_) => {
                        let number = override_value.parse::<f64>().map_err(|_| {
                            SceneParseError::InvalidValue {
                                field: format!("scene.properties.{key}"),
                                detail: "expected a finite number".into(),
                            }
                        })?;
                        if !number.is_finite() {
                            return Err(SceneParseError::InvalidValue {
                                field: format!("scene.properties.{key}"),
                                detail: "expected a finite number".into(),
                            });
                        }
                        Value::from(number)
                    }
                    Value::String(_) => Value::String(override_value),
                    _ => fallback.clone(),
                };
            }
            for value in object.values_mut() {
                apply_property_overrides(value, properties, depth + 1)?;
            }
        }
        _ => {}
    }
    Ok(())
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
        "animationlayers",
        "attachment",
        "bone_animations",
        "bones",
        "castshadow",
        "collision",
        "color",
        "colorBlendMode",
        "container",
        "effects",
        "id",
        "image",
        "instanceoverride",
        "name",
        "origin",
        "parallaxDepth",
        "parent",
        "particle",
        "scale",
        "size",
        "sound",
        "text",
        "transform",
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
    } else if object.contains_key("container") {
        let is_array = object.get("container").and_then(Value::as_array).is_some();
        if is_array {
            mark(unsupported, &path, "container group (child nodes)");
        }
        SceneNodeKind::Container
    } else if let Some(resource) = string_resource(object, "sound", &path)? {
        mark(unsupported, &path, "sound object");
        SceneNodeKind::Sound(resource)
    } else if object.contains_key("text") {
        mark(unsupported, &path, "text object");
        SceneNodeKind::Text
    } else if let Some(resource) = string_resource(object, "image", &path)? {
        if resource.ends_with(".mdl") {
            mark(unsupported, &path, &format!("Spriter model: {resource}"));
            SceneNodeKind::Model(resource)
        } else {
            SceneNodeKind::Image(resource)
        }
    } else if (object.contains_key("bones") || object.contains_key("bone_animations"))
        && !object.contains_key("image")
    {
        mark(unsupported, &path, "3D model / bone animation");
        SceneNodeKind::Model(String::new())
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
                    file.map(validate_resource_path)
                })
                .collect::<Result<Vec<_>, _>>()
        })
        .transpose()?
        .unwrap_or_default();
    let scroll = parse_scroll_effect(object.get("effects"), &path)?;
    let water_wave = parse_water_wave_effect(object.get("effects"), &path)?;
    let water_flow = parse_water_flow_effect(object.get("effects"), &path)?;
    for effect in &effects {
        if effect != "effects/scroll/effect.json"
            && effect != "effects/waterwaves/effect.json"
            && effect != "effects/waterripple/effect.json"
            && effect != "effects/waterflow/effect.json"
        {
            mark(unsupported, &path, &format!("effect: {effect}"));
        }
    }
    for (field, feature) in [
        ("animationlayers", "model animation layers"),
        ("attachment", "model attachment"),
        ("color", "layer color modulation"),
        ("colorBlendMode", "layer color blend mode"),
        ("parallaxDepth", "per-layer parallax depth"),
        ("transform", "extended layer transform"),
    ] {
        if object.contains_key(field) {
            mark(unsupported, &format!("{path}.{field}"), feature);
        }
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
            .map(|value| {
                unwrap_script_value(value)
                    .as_bool()
                    .ok_or_else(|| SceneParseError::InvalidValue {
                        field: format!("{path}.visible"),
                        detail: "expected a boolean or bound boolean value".into(),
                    })
            })
            .transpose()?
            .unwrap_or(true),
        kind,
        transform: SceneTransform {
            origin: object
                .get("origin")
                .map(|value| parse_vec3_lenient(&format!("{path}.origin"), value))
                .transpose()?,
            size: optional_vec2(object, "size", &path)?,
            scale: optional_vec3(object, "scale", &path)?,
            angles: optional_vec3(object, "angles", &path)?,
            opacity: object
                .get("alpha")
                .map(|value| {
                    let value = unwrap_script_value(value);
                    number_value(&format!("{path}.alpha"), value)
                })
                .transpose()?,
        },
        effects,
        scroll,
        water_wave,
        water_flow,
        unknown_fields,
    })
}

fn parse_water_flow_effect(
    effects: Option<&Value>,
    path: &str,
) -> Result<Option<WaterFlowEffect>, SceneParseError> {
    let Some(effects) = effects.and_then(Value::as_array) else {
        return Ok(None);
    };
    for (index, effect) in effects.iter().enumerate() {
        let Some(effect) = effect.as_object() else {
            continue;
        };
        if effect.get("file").and_then(Value::as_str) != Some("effects/waterflow/effect.json") {
            continue;
        }
        if !effect
            .get("visible")
            .map(|value| unwrap_script_value(value).as_bool().unwrap_or(true))
            .unwrap_or(true)
        {
            continue;
        }
        let values = effect
            .get("passes")
            .and_then(Value::as_array)
            .and_then(|passes| passes.first())
            .and_then(|pass| pass.get("constantshadervalues"))
            .and_then(Value::as_object)
            .ok_or_else(|| SceneParseError::InvalidValue {
                field: format!("{path}.effects[{index}]"),
                detail: "waterflow effect is missing constant shader values".into(),
            })?;
        let phase_scale = optional_finite_number(values, "phasescale").unwrap_or(1.0);
        let speed = optional_finite_number(values, "speed").unwrap_or(1.0);
        let strength = optional_finite_number(values, "strength").unwrap_or(0.0);
        if phase_scale <= 0.0 || strength < 0.0 {
            return Err(SceneParseError::InvalidValue {
                field: format!("{path}.effects[{index}]"),
                detail: "waterflow phase scale must be positive and strength non-negative".into(),
            });
        }
        return Ok(Some(WaterFlowEffect {
            phase_scale,
            speed,
            strength,
            mask: effect_texture(effect, index, 1)?,
            phase: effect_texture(effect, index, 2)?,
        }));
    }
    Ok(None)
}

fn parse_water_wave_effect(
    effects: Option<&Value>,
    path: &str,
) -> Result<Option<WaterWaveEffect>, SceneParseError> {
    let Some(effects) = effects.and_then(Value::as_array) else {
        return Ok(None);
    };
    for (index, effect) in effects.iter().enumerate() {
        let Some(effect) = effect.as_object() else {
            continue;
        };
        if effect.get("file").and_then(Value::as_str) != Some("effects/waterwaves/effect.json") {
            continue;
        }
        if !effect
            .get("visible")
            .map(|value| unwrap_script_value(value).as_bool().unwrap_or(true))
            .unwrap_or(true)
        {
            continue;
        }
        let values = effect
            .get("passes")
            .and_then(Value::as_array)
            .and_then(|passes| passes.first())
            .and_then(|pass| pass.get("constantshadervalues"))
            .and_then(Value::as_object)
            .ok_or_else(|| SceneParseError::InvalidValue {
                field: format!("{path}.effects[{index}]"),
                detail: "waterwaves effect is missing constant shader values".into(),
            })?;
        let direction = optional_finite_number(values, "direction").unwrap_or(0.0);
        let scale = optional_finite_number(values, "scale").unwrap_or(1.0);
        let speed = optional_finite_number(values, "speed").unwrap_or(1.0);
        let strength = optional_finite_number(values, "strength").unwrap_or(0.0);
        if scale <= 0.0 || strength < 0.0 {
            return Err(SceneParseError::InvalidValue {
                field: format!("{path}.effects[{index}]"),
                detail: "waterwaves scale must be positive and strength non-negative".into(),
            });
        }
        return Ok(Some(WaterWaveEffect {
            direction,
            scale,
            speed,
            strength,
            mask: effect_texture(effect, index, 1)?,
            normal: None,
        }));
    }
    for (index, effect) in effects.iter().enumerate() {
        let Some(effect) = effect.as_object() else {
            continue;
        };
        if effect.get("file").and_then(Value::as_str) != Some("effects/waterripple/effect.json") {
            continue;
        }
        if !effect
            .get("visible")
            .map(|value| unwrap_script_value(value).as_bool().unwrap_or(true))
            .unwrap_or(true)
        {
            continue;
        }
        let values = effect
            .get("passes")
            .and_then(Value::as_array)
            .and_then(|passes| passes.first())
            .and_then(|pass| pass.get("constantshadervalues"))
            .and_then(Value::as_object)
            .ok_or_else(|| SceneParseError::InvalidValue {
                field: format!("{path}.effects[{index}]"),
                detail: "waterripple effect is missing constant shader values".into(),
            })?;
        let direction = optional_finite_number(values, "scrolldirection").unwrap_or(0.0);
        let scale = optional_finite_number(values, "scale").unwrap_or(1.0);
        let animation_speed = optional_finite_number(values, "animationspeed").unwrap_or(0.0);
        let scroll_speed = optional_finite_number(values, "scrollspeed").unwrap_or(0.0);
        let strength = optional_finite_number(values, "ripplestrength").unwrap_or(0.0);
        if scale <= 0.0 || strength < 0.0 {
            return Err(SceneParseError::InvalidValue {
                field: format!("{path}.effects[{index}]"),
                detail: "waterripple scale must be positive and strength non-negative".into(),
            });
        }
        return Ok(Some(WaterWaveEffect {
            direction,
            scale,
            speed: animation_speed + scroll_speed,
            strength,
            mask: effect_texture(effect, index, 1)?,
            normal: effect_texture(effect, index, 2)?,
        }));
    }
    Ok(None)
}

fn effect_texture(
    effect: &Map<String, Value>,
    effect_index: usize,
    texture_index: usize,
) -> Result<Option<String>, SceneParseError> {
    let Some(value) = effect
        .get("passes")
        .and_then(Value::as_array)
        .and_then(|passes| passes.first())
        .and_then(|pass| pass.get("textures"))
        .and_then(Value::as_array)
        .and_then(|textures| textures.get(texture_index))
    else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    let texture = value
        .as_str()
        .ok_or_else(|| SceneParseError::InvalidValue {
            field: format!("effects[{effect_index}].textures[{texture_index}]"),
            detail: "effect texture must be a string or null".into(),
        })?;
    validate_resource_path(texture).map(Some)
}

fn parse_scroll_effect(
    effects: Option<&Value>,
    path: &str,
) -> Result<Option<ScrollEffect>, SceneParseError> {
    let Some(effects) = effects.and_then(Value::as_array) else {
        return Ok(None);
    };
    for (index, effect) in effects.iter().enumerate() {
        let Some(effect) = effect.as_object() else {
            continue;
        };
        if effect.get("file").and_then(Value::as_str) != Some("effects/scroll/effect.json") {
            continue;
        }
        let visible = effect
            .get("visible")
            .map(|value| unwrap_script_value(value).as_bool().unwrap_or(true))
            .unwrap_or(true);
        if !visible {
            continue;
        }
        let values = effect
            .get("passes")
            .and_then(Value::as_array)
            .and_then(|passes| passes.first())
            .and_then(|pass| pass.get("constantshadervalues"))
            .and_then(Value::as_object)
            .ok_or_else(|| SceneParseError::InvalidValue {
                field: format!("{path}.effects[{index}]"),
                detail: "scroll effect is missing constant shader values".into(),
            })?;
        let speed_x = optional_finite_number(values, "speedx").unwrap_or(0.0);
        let speed_y = optional_finite_number(values, "speedy").unwrap_or(0.0);
        let repeat = values
            .get("repeat")
            .map(|value| parse_vec2(&format!("{path}.effects[{index}].repeat"), value))
            .transpose()?
            .unwrap_or(Vec2 { x: 1.0, y: 1.0 });
        if repeat.x <= 0.0 || repeat.y <= 0.0 {
            return Err(SceneParseError::InvalidValue {
                field: format!("{path}.effects[{index}].repeat"),
                detail: "scroll repeat must be finite and positive".into(),
            });
        }
        return Ok(Some(ScrollEffect {
            speed_x,
            speed_y,
            repeat_x: repeat.x,
            repeat_y: repeat.y,
        }));
    }
    Ok(None)
}

fn optional_finite_number(object: &Map<String, Value>, field: &str) -> Option<f32> {
    object
        .get(field)
        .and_then(Value::as_f64)
        .map(|value| value as f32)
        .filter(|value| value.is_finite())
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
        .or_else(|| {
            // Handle array-of-one-string: ["resource/path"]
            value.as_array().and_then(|arr| {
                if arr.len() == 1 {
                    arr[0].as_str()
                } else {
                    None
                }
            })
        })
        .ok_or_else(|| SceneParseError::InvalidValue {
            field: format!("{path}.{field}"),
            detail: "expected a resource path string or single-element array".into(),
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
    let value = unwrap_script_value(value);
    let values = vector_values(field, value, 3)?;
    Ok(Vec3 {
        x: values[0],
        y: values[1],
        z: values[2],
    })
}

/// Parse a vector that may have 2 or 3 components, or be a script-driven object.
/// Common for `origin` fields in 2D-orthographic scenes.
fn parse_vec3_lenient(field: &str, value: &Value) -> Result<Vec3, SceneParseError> {
    let value = unwrap_script_value(value);
    let values = vector_values(field, value, 3).or_else(|_| {
        let values = vector_values(field, value, 2)?;
        Ok::<_, SceneParseError>(vec![values[0], values[1], 0.0])
    })?;
    Ok(Vec3 {
        x: values[0],
        y: values[1],
        z: values[2],
    })
}

fn parse_vec2(field: &str, value: &Value) -> Result<Vec2, SceneParseError> {
    let value = unwrap_script_value(value);
    let values = vector_values(field, value, 2)?;
    Ok(Vec2 {
        x: values[0],
        y: values[1],
    })
}

/// Unwrap a script-driven or property-bound value.
/// Handles: `{"script": "...","value":"x y z"}`, `{"user":"prop","value":1.0}`, etc.
/// Falls back to the original value if not an object with a `.value` field.
fn unwrap_script_value(value: &Value) -> &Value {
    if let Some(inner) = value.as_object().and_then(|obj| obj.get("value")) {
        inner
    } else {
        value
    }
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

        let error =
            parse_scene_graph(r#"{"objects":[{"image":"safe","effects":[{"file":"../shader"}]}]}"#)
                .unwrap_err();
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

    #[test]
    fn parses_2_component_origin_as_z_zero() {
        let graph = parse_scene_graph(
            r#"{
                "general":{"orthogonalprojection":{"width":1920,"height":1080}},
                "objects":[{"image":"bg.json","origin":"960 540"}]
            }"#,
        )
        .unwrap();
        let origin = graph.nodes[0].transform.origin.unwrap();
        assert_eq!(origin.x, 960.0);
        assert_eq!(origin.y, 540.0);
        assert_eq!(origin.z, 0.0);
    }

    #[test]
    fn parses_script_driven_origin() {
        let graph = parse_scene_graph(
            r#"{
                "general":{"orthogonalprojection":{"width":1920,"height":1080}},
                "objects":[{
                    "id":1,"name":"scripted",
                    "image":"bg.json",
                    "origin":{"script":"// update()","value":"1920 1080 0"}
                }]
            }"#,
        )
        .unwrap();
        let origin = graph.nodes[0].transform.origin.unwrap();
        assert_eq!(origin.x, 1920.0);
        assert_eq!(origin.y, 1080.0);
        assert_eq!(origin.z, 0.0);
    }

    #[test]
    fn uses_bound_visibility_fallback_value() {
        let graph = parse_scene_graph(
            r#"{"objects":[{"image":"bg.json","visible":{"user":"show_bg","value":false}}]}"#,
        )
        .unwrap();
        assert!(!graph.nodes[0].visible);
    }

    #[test]
    fn applies_scene_property_overrides_to_bound_values() {
        let properties = BTreeMap::from([
            ("show_bg".into(), "true".into()),
            ("scale".into(), "2 3 1".into()),
        ]);
        let graph = parse_scene_graph_with_properties(
            r#"{"objects":[{"image":"bg.json","visible":{"user":"show_bg","value":false},"scale":{"user":"scale","value":"1 1 1"}}]}"#,
            &properties,
        )
        .unwrap();
        assert!(graph.nodes[0].visible);
        assert_eq!(graph.nodes[0].transform.scale.unwrap().x, 2.0);
        assert_eq!(graph.nodes[0].transform.scale.unwrap().y, 3.0);
    }

    #[test]
    fn parses_enabled_real_scroll_effect_parameters() {
        let graph = parse_scene_graph(
            r#"{"objects":[{"image":"clouds.json","effects":[{"file":"effects/scroll/effect.json","visible":true,"passes":[{"constantshadervalues":{"repeat":"2 1","speedx":-0.3,"speedy":0.2}}]}]}]}"#,
        )
        .unwrap();
        let scroll = graph.nodes[0].scroll.unwrap();
        assert_eq!(scroll.speed_x, -0.3);
        assert_eq!(scroll.speed_y, 0.2);
        assert_eq!(scroll.repeat_x, 2.0);
        assert!(graph.unsupported_features.is_empty());
    }

    #[test]
    fn parses_real_waterwaves_parameters() {
        let graph = parse_scene_graph(
            r#"{"objects":[{"image":"foreground.json","effects":[{"file":"effects/waterwaves/effect.json","visible":true,"passes":[{"constantshadervalues":{"direction":1.5772198,"scale":0.01,"speed":1.41,"strength":0.06}}]}]}]}"#,
        )
        .unwrap();
        let wave = graph.nodes[0].water_wave.as_ref().unwrap();
        assert_eq!(wave.direction, 1.5772198);
        assert_eq!(wave.scale, 0.01);
        assert_eq!(wave.speed, 1.41);
        assert_eq!(wave.strength, 0.06);
        assert!(graph.unsupported_features.is_empty());
    }

    #[test]
    fn parses_waterflow_alongside_waterripple() {
        let graph = parse_scene_graph(
            r#"{"objects":[{"image":"water.json","effects":[{"file":"effects/waterripple/effect.json","passes":[{"constantshadervalues":{"animationspeed":0.03,"scrollspeed":0.08,"scale":1.0,"ripplestrength":0.1},"textures":[null,"masks/ripple","effects/normal"]}]},{"file":"effects/waterflow/effect.json","passes":[{"constantshadervalues":{"phasescale":0.62,"speed":0.08,"strength":1.0},"textures":[null,"masks/flow","effects/phase"]}]}]}]}"#,
        )
        .unwrap();
        let node = &graph.nodes[0];
        let wave = node.water_wave.as_ref().unwrap();
        assert_eq!(wave.speed, 0.11);
        assert_eq!(wave.mask.as_deref(), Some("masks/ripple"));
        assert_eq!(wave.normal.as_deref(), Some("effects/normal"));
        let flow = node.water_flow.as_ref().unwrap();
        assert_eq!(flow.phase_scale, 0.62);
        assert_eq!(flow.speed, 0.08);
        assert_eq!(flow.strength, 1.0);
        assert_eq!(flow.mask.as_deref(), Some("masks/flow"));
        assert_eq!(flow.phase.as_deref(), Some("effects/phase"));
        assert!(graph.unsupported_features.is_empty());
    }

    #[test]
    fn detects_container_and_model_nodes() {
        let graph = parse_scene_graph(
            r#"{
                "objects":[
                    {"id":1,"container":[]},
                    {"id":2,"image":"model.mdl"},
                    {"id":3,"bones":[{"name":"root"}],"bone_animations":[]}
                ]
            }"#,
        )
        .unwrap();
        assert!(matches!(graph.nodes[0].kind, SceneNodeKind::Container));
        assert!(matches!(graph.nodes[1].kind, SceneNodeKind::Model(_)));
        assert!(matches!(graph.nodes[2].kind, SceneNodeKind::Model(_)));
    }
}
