use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Map, Value};

use crate::error::SceneParseError;

pub const MAX_SCENE_OBJECTS: usize = 16_384;
pub const MAX_WATER_WAVE_EFFECTS: usize = 3;
pub const MAX_SHAKE_EFFECTS: usize = 3;
pub const MAX_PULSE_EFFECTS: usize = 3;

/// Version-independent, data-only scene representation consumed by future runtimes.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SceneGraph {
    pub camera: SceneCamera,
    pub nodes: Vec<SceneNode>,
    pub unsupported_features: Vec<UnsupportedFeature>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SceneCamera {
    pub eye: Option<Vec3>,
    pub center: Option<Vec3>,
    pub projection_size: Option<Vec2>,
    #[serde(default = "default_one")]
    pub zoom: f32,
    #[serde(default)]
    pub zoom_animation: Option<ScalarAnimation>,
}

impl Default for SceneCamera {
    fn default() -> Self {
        Self {
            eye: None,
            center: None,
            projection_size: None,
            zoom: 1.0,
            zoom_animation: None,
        }
    }
}

const fn default_one() -> f32 {
    1.0
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ScalarAnimation {
    pub keyframes: Vec<ScalarKeyframe>,
    pub fps: f32,
    pub length_frames: f32,
    pub mode: AnimationMode,
}

impl ScalarAnimation {
    pub fn value_at(&self, elapsed_seconds: f64) -> f32 {
        let Some(first) = self.keyframes.first() else {
            return 0.0;
        };
        let Some(last) = self.keyframes.last() else {
            return first.value;
        };
        if self.fps <= 0.0 || self.length_frames <= 0.0 {
            return last.value;
        }
        let mut frame = elapsed_seconds.max(0.0) as f32 * self.fps;
        if self.mode == AnimationMode::Loop {
            frame = frame.rem_euclid(self.length_frames);
        } else {
            frame = frame.min(self.length_frames);
        }
        if frame <= first.frame {
            return first.value;
        }
        for pair in self.keyframes.windows(2) {
            let [left, right] = pair else { continue };
            if frame <= right.frame {
                let span = right.frame - left.frame;
                if span <= f32::EPSILON {
                    return right.value;
                }
                let amount = ((frame - left.frame) / span).clamp(0.0, 1.0);
                return left.value + (right.value - left.value) * amount;
            }
        }
        last.value
    }
}

#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ScalarKeyframe {
    pub frame: f32,
    pub value: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnimationMode {
    Single,
    Loop,
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
    /// First authored instance texture, used when a system texture is unavailable.
    #[serde(default)]
    pub texture_override: Option<String>,
    /// Safe approximation of an animated color-grading fade on a composition layer.
    #[serde(default)]
    pub backdrop_fade: Option<ScalarAnimation>,
    pub dynamic_scale: Option<DynamicScaleKind>,
    #[serde(default)]
    pub audio_visualizer: Option<SceneAudioVisualizer>,
    pub effects: Vec<String>,
    pub scroll: Option<ScrollEffect>,
    pub water_waves: Vec<WaterWaveEffect>,
    pub water_flow: Option<WaterFlowEffect>,
    pub shakes: Vec<ShakeEffect>,
    pub pulses: Vec<PulseEffect>,
    pub spin: Option<SpinEffect>,
    pub iris: Option<IrisEffect>,
    pub foliage_sway: Vec<FoliageSwayEffect>,
    pub shine: Option<ShineEffect>,
    /// Fields retained by name so unsupported input cannot silently change rendering.
    pub unknown_fields: Vec<String>,
}

/// A safely recognized, script-authored desktop audio spectrum.
///
/// The original script is never executed. This configuration is only emitted
/// when the parser recognizes the complete, bounded spectrum pattern.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SceneAudioVisualizer {
    pub bins: usize,
    pub bar_width: f32,
    pub height_scale: f32,
    pub spacing: f32,
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
    /// Parameters used only by Wallpaper Engine's normal-map water ripple.
    /// A ripple is not equivalent to the procedural sine displacement used by
    /// `waterwaves`, so retaining these values prevents the two effects from
    /// being accidentally rendered with the same shader path.
    #[serde(default)]
    pub ripple: Option<WaterRippleEffect>,
}

#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct WaterRippleEffect {
    pub animation_speed: f32,
    pub scroll_speed: f32,
    pub ratio: f32,
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
pub struct ShakeEffect {
    pub speed: f32,
    pub strength: f32,
    pub bounds: Vec2,
    pub friction: Vec2,
    pub direction_map: Option<String>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PulseEffect {
    pub speed: f32,
    pub phase: f32,
    pub amount: f32,
    pub bounds: Vec2,
    pub power: f32,
    pub tint_low: Vec3,
    pub tint_high: Vec3,
    pub mask: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SpinEffect {
    pub speed: f32,
    pub center: Vec2,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct IrisEffect {
    pub speed: f32,
    pub roughness: f32,
    pub noise_amount: f32,
    pub phase: f32,
    pub scale: Vec2,
    pub mask: Option<String>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct FoliageSwayEffect {
    pub direction: f32,
    pub scale: f32,
    pub speed: f32,
    pub strength: f32,
    pub phase: f32,
    pub power: f32,
    pub ratio: f32,
    pub mask: Option<String>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ShineEffect {
    pub direction: f32,
    pub speed: f32,
    pub intensity: f32,
    pub length: f32,
    pub color: Vec3,
    pub mask: Option<String>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", content = "resource", rename_all = "snake_case")]
pub enum SceneNodeKind {
    Image(String),
    Model(String),
    Sound(SceneSound),
    Particle(String),
    Text(SceneText),
    Container,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SceneSound {
    pub resource: String,
    pub playback_mode: SoundPlaybackMode,
    pub volume: f32,
    pub start_silent: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SoundPlaybackMode {
    Loop,
    Once,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DynamicTextKind {
    Clock,
    Date,
    /// Wallpaper Engine media-event text has no authoring fallback at runtime.
    Media,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SceneText {
    pub value: String,
    pub dynamic: Option<DynamicTextKind>,
    pub color: Vec3,
    pub font: String,
    pub point_size: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DynamicScaleKind {
    ClockSecondX,
}

#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct SceneTransform {
    pub origin: Option<Vec3>,
    pub size: Option<Vec2>,
    pub scale: Option<Vec3>,
    pub angles: Option<Vec3>,
    pub opacity: Option<f32>,
    #[serde(default)]
    pub opacity_animation: Option<ScalarAnimation>,
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
            let binding = object.get("user").and_then(|user| match user {
                Value::String(key) => Some((key.as_str(), None)),
                Value::Object(user) => user
                    .get("name")
                    .and_then(Value::as_str)
                    .map(|key| (key, user.get("condition").and_then(Value::as_str))),
                _ => None,
            });
            let binding = binding.and_then(|(key, condition)| {
                properties
                    .get(key)
                    .map(|value| (key.to_owned(), condition.map(str::to_owned), value.clone()))
            });
            if let Some((key, condition, override_value)) = binding
                && let Some(fallback) = object.get_mut("value")
            {
                *fallback = match fallback {
                    Value::Bool(_) => Value::Bool(if let Some(condition) = condition {
                        override_value == condition
                    } else {
                        match override_value.as_str() {
                            "true" | "1" => true,
                            "false" | "0" => false,
                            _ => {
                                return Err(SceneParseError::InvalidValue {
                                    field: format!("scene.properties.{key}"),
                                    detail: "expected true, false, 1, or 0".into(),
                                });
                            }
                        }
                    }),
                    Value::Number(_) => {
                        let number = match override_value.as_str() {
                            "true" => 1.0,
                            "false" => 0.0,
                            _ => override_value.parse::<f64>().map_err(|_| {
                                SceneParseError::InvalidValue {
                                    field: format!("scene.properties.{key}"),
                                    detail: "expected a finite number or boolean".into(),
                                }
                            })?,
                        };
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
    let zoom_value = root
        .get("general")
        .and_then(Value::as_object)
        .and_then(|general| general.get("zoom"));

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
        zoom: zoom_value
            .map(unwrap_script_value)
            .map(|value| number_value("general.zoom", value))
            .transpose()?
            .unwrap_or(1.0),
        zoom_animation: zoom_value
            .map(|value| parse_scalar_animation(value, "general.zoom"))
            .transpose()?
            .flatten(),
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
        "font",
        "horizontalalign",
        "id",
        "image",
        "instanceoverride",
        "instance",
        "dependencies",
        "name",
        "origin",
        "parallaxDepth",
        "parent",
        "particle",
        "playbackmode",
        "pointsize",
        "scale",
        "size",
        "sound",
        "startsilent",
        "text",
        "transform",
        "visible",
        "verticalalign",
        "volume",
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
        SceneNodeKind::Sound(parse_scene_sound(object, resource, &path, unsupported)?)
    } else if let Some(text) = object.get("text") {
        SceneNodeKind::Text(parse_scene_text(object, text, &path)?)
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
    let water_waves = parse_water_wave_effects(object.get("effects"), &path)?;
    let water_flow = parse_water_flow_effect(object.get("effects"), &path)?;
    let shakes = parse_shake_effects(object.get("effects"), &path)?;
    let pulses = parse_pulse_effects(object.get("effects"), &path)?;
    let spin = parse_spin_effect(object.get("effects"), &path)?;
    let iris = parse_iris_effect(object.get("effects"), &path)?;
    let foliage_sway = parse_foliage_sway_effects(object.get("effects"), &path)?;
    let shine = parse_shine_effect(object.get("effects"), &path)?;
    let backdrop_fade = parse_backdrop_fade(object, &path)?;
    let audio_visualizer = parse_audio_visualizer(object.get("visible"));
    if object.get("visible").and_then(script_source).is_some() && audio_visualizer.is_none() {
        mark(
            unsupported,
            &format!("{path}.visible.script"),
            "unrecognized visible script",
        );
    }
    for effect in &effects {
        if !effect.ends_with("/scroll/effect.json")
            && effect != "effects/waterwaves/effect.json"
            && effect != "effects/waterripple/effect.json"
            && effect != "effects/waterflow/effect.json"
            && effect != "effects/shake/effect.json"
            && effect != "effects/pulse/effect.json"
            && effect != "effects/spin/effect.json"
            && effect != "effects/iris/effect.json"
            && effect != "effects/foliagesway/effect.json"
            && effect != "effects/shine/effect.json"
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

    let texture_override = object
        .get("instance")
        .and_then(Value::as_object)
        .and_then(|instance| instance.get("textures"))
        .and_then(Value::as_array)
        .and_then(|textures| textures.iter().find_map(Value::as_str))
        .map(validate_resource_path)
        .transpose()?;

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
            opacity_animation: object
                .get("alpha")
                .map(|value| parse_scalar_animation(value, &format!("{path}.alpha")))
                .transpose()?
                .flatten(),
        },
        texture_override,
        backdrop_fade,
        dynamic_scale: parse_dynamic_scale(object.get("scale")),
        audio_visualizer,
        effects,
        scroll,
        water_waves,
        water_flow,
        shakes,
        pulses,
        spin,
        iris,
        foliage_sway,
        shine,
        unknown_fields,
    })
}

fn parse_scene_sound(
    object: &Map<String, Value>,
    resource: String,
    path: &str,
    unsupported: &mut BTreeSet<UnsupportedFeature>,
) -> Result<SceneSound, SceneParseError> {
    let playback_mode = match object.get("playbackmode").map(unwrap_script_value) {
        None => SoundPlaybackMode::Once,
        Some(Value::String(mode)) if mode == "once" => SoundPlaybackMode::Once,
        Some(Value::String(mode)) if mode == "loop" => SoundPlaybackMode::Loop,
        Some(Value::String(mode)) => {
            mark(
                unsupported,
                &format!("{path}.playbackmode"),
                &format!("sound playback mode: {mode}"),
            );
            SoundPlaybackMode::Once
        }
        Some(_) => {
            return Err(SceneParseError::InvalidValue {
                field: format!("{path}.playbackmode"),
                detail: "expected a playback mode string".into(),
            });
        }
    };
    let volume = object
        .get("volume")
        .map(unwrap_script_value)
        .map(|value| number_value(&format!("{path}.volume"), value))
        .transpose()?
        .unwrap_or(1.0);
    if !(0.0..=1.0).contains(&volume) {
        return Err(SceneParseError::InvalidValue {
            field: format!("{path}.volume"),
            detail: "sound volume must be between 0 and 1".into(),
        });
    }
    let start_silent = object
        .get("startsilent")
        .map(unwrap_script_value)
        .map(|value| {
            value
                .as_bool()
                .ok_or_else(|| SceneParseError::InvalidValue {
                    field: format!("{path}.startsilent"),
                    detail: "expected a boolean or bound boolean value".into(),
                })
        })
        .transpose()?
        .unwrap_or(false);
    Ok(SceneSound {
        resource,
        playback_mode,
        volume,
        start_silent,
    })
}

fn parse_scene_text(
    object: &Map<String, Value>,
    text: &Value,
    path: &str,
) -> Result<SceneText, SceneParseError> {
    let value = unwrap_script_value(text)
        .as_str()
        .ok_or_else(|| SceneParseError::InvalidValue {
            field: format!("{path}.text"),
            detail: "expected text or a script-driven text value".into(),
        })?
        .to_owned();
    if value.chars().count() > 256 {
        return Err(SceneParseError::InvalidValue {
            field: format!("{path}.text"),
            detail: "text exceeds the 256 character safety limit".into(),
        });
    }
    let script = script_source(text).unwrap_or_default();
    let dynamic = if script.contains("mediaPropertiesChanged") {
        Some(DynamicTextKind::Media)
    } else if script.contains("getHours()") && script.contains("getMinutes()") {
        Some(DynamicTextKind::Clock)
    } else if script.contains("getFullYear()")
        && script.contains("getMonth()")
        && script.contains("getDate()")
    {
        Some(DynamicTextKind::Date)
    } else {
        None
    };
    let color = object
        .get("color")
        .map(|value| parse_vec3(&format!("{path}.color"), value))
        .transpose()?
        .unwrap_or(Vec3 {
            x: 1.0,
            y: 1.0,
            z: 1.0,
        });
    let font = object
        .get("font")
        .and_then(Value::as_str)
        .unwrap_or("systemfont_sans")
        .to_owned();
    if font.contains('/') || font.contains('\\') {
        validate_resource_path(&font)?;
    }
    let point_size = object
        .get("pointsize")
        .map(|value| number_value(&format!("{path}.pointsize"), value))
        .transpose()?
        .unwrap_or(32.0);
    if !(1.0..=512.0).contains(&point_size) {
        return Err(SceneParseError::InvalidValue {
            field: format!("{path}.pointsize"),
            detail: "point size must be within 1..=512".into(),
        });
    }
    Ok(SceneText {
        value,
        dynamic,
        color,
        font,
        point_size,
    })
}

fn parse_scalar_animation(
    value: &Value,
    path: &str,
) -> Result<Option<ScalarAnimation>, SceneParseError> {
    const MAX_KEYFRAMES: usize = 1024;
    let Some(animation) = value
        .as_object()
        .and_then(|object| object.get("animation"))
        .and_then(Value::as_object)
    else {
        return Ok(None);
    };
    let options = animation
        .get("options")
        .and_then(Value::as_object)
        .ok_or_else(|| SceneParseError::InvalidValue {
            field: format!("{path}.animation.options"),
            detail: "expected an animation options object".into(),
        })?;
    let fps = number_field(options, "fps", &format!("{path}.animation.options"))?;
    let length_frames = number_field(options, "length", &format!("{path}.animation.options"))?;
    if fps <= 0.0 || length_frames <= 0.0 {
        return Err(SceneParseError::InvalidValue {
            field: format!("{path}.animation.options"),
            detail: "animation fps and length must be positive".into(),
        });
    }
    let mode = match options
        .get("mode")
        .and_then(Value::as_str)
        .unwrap_or("single")
    {
        "single" => AnimationMode::Single,
        "loop" => AnimationMode::Loop,
        other => {
            return Err(SceneParseError::InvalidValue {
                field: format!("{path}.animation.options.mode"),
                detail: format!("unsupported scalar animation mode: {other}"),
            });
        }
    };
    let values = animation
        .get("c0")
        .and_then(Value::as_array)
        .ok_or_else(|| SceneParseError::InvalidValue {
            field: format!("{path}.animation.c0"),
            detail: "expected a scalar keyframe array".into(),
        })?;
    if values.is_empty() || values.len() > MAX_KEYFRAMES {
        return Err(SceneParseError::InvalidValue {
            field: format!("{path}.animation.c0"),
            detail: format!("keyframe count must be within 1..={MAX_KEYFRAMES}"),
        });
    }
    let mut keyframes = Vec::with_capacity(values.len());
    for (index, value) in values.iter().enumerate() {
        let keyframe = value
            .as_object()
            .ok_or_else(|| SceneParseError::InvalidValue {
                field: format!("{path}.animation.c0[{index}]"),
                detail: "expected a keyframe object".into(),
            })?;
        keyframes.push(ScalarKeyframe {
            frame: number_field(keyframe, "frame", &format!("{path}.animation.c0[{index}]"))?,
            value: number_field(keyframe, "value", &format!("{path}.animation.c0[{index}]"))?,
        });
    }
    keyframes.sort_by(|left, right| left.frame.total_cmp(&right.frame));
    Ok(Some(ScalarAnimation {
        keyframes,
        fps,
        length_frames,
        mode,
    }))
}

fn parse_backdrop_fade(
    object: &Map<String, Value>,
    path: &str,
) -> Result<Option<ScalarAnimation>, SceneParseError> {
    let is_composition_layer = object
        .get("image")
        .and_then(Value::as_str)
        .is_some_and(|image| image == "models/util/composelayer.json");
    if !is_composition_layer {
        return Ok(None);
    }
    let Some(effects) = object.get("effects").and_then(Value::as_array) else {
        return Ok(None);
    };
    for (effect_index, effect) in effects.iter().enumerate() {
        let Some(effect) = effect.as_object() else {
            continue;
        };
        if !effect
            .get("file")
            .and_then(Value::as_str)
            .is_some_and(|file| file.ends_with("/color_grading/effect.json"))
        {
            continue;
        }
        let Some(passes) = effect.get("passes").and_then(Value::as_array) else {
            continue;
        };
        for (pass_index, pass) in passes.iter().enumerate() {
            let Some(brightness) = pass
                .get("constantshadervalues")
                .and_then(Value::as_object)
                .and_then(|values| values.get("Brightness"))
            else {
                continue;
            };
            let animation_path = format!(
                "{path}.effects[{effect_index}].passes[{pass_index}].constantshadervalues.Brightness"
            );
            if let Some(mut animation) = parse_scalar_animation(brightness, &animation_path)? {
                for keyframe in &mut animation.keyframes {
                    keyframe.value = (-keyframe.value).clamp(0.0, 1.0);
                }
                return Ok(Some(animation));
            }
        }
    }
    Ok(None)
}

fn parse_dynamic_scale(value: Option<&Value>) -> Option<DynamicScaleKind> {
    let script = value.and_then(script_source)?;
    (script.contains("getSeconds()") && script.contains("/ 60"))
        .then_some(DynamicScaleKind::ClockSecondX)
}

fn script_source(value: &Value) -> Option<&str> {
    value
        .as_object()
        .and_then(|object| object.get("script"))
        .and_then(Value::as_str)
}

fn parse_audio_visualizer(visible: Option<&Value>) -> Option<SceneAudioVisualizer> {
    let visible = visible?.as_object()?;
    let script = visible.get("script")?.as_str()?;
    let bins = parse_script_call_usize(script, "engine.registerAudioBuffers")?;
    if !(8..=128).contains(&bins)
        || !script_contains_ignoring_ascii_whitespace(script, "audioData.average[i]")
        || !script_contains_ignoring_ascii_whitespace(script, "createLayer(")
    {
        return None;
    }

    let properties = visible.get("scriptproperties")?.as_object()?;
    let bar_width = bounded_script_property(properties, "barWidth", 0.0..=100.0)?;
    let height_scale = bounded_script_property(properties, "scaleY", 0.0..=500.0)?;
    let spacing = bounded_script_property(properties, "originX", -500.0..=500.0)?;
    Some(SceneAudioVisualizer {
        bins,
        bar_width,
        height_scale,
        spacing,
    })
}

fn bounded_script_property(
    properties: &Map<String, Value>,
    name: &str,
    range: std::ops::RangeInclusive<f32>,
) -> Option<f32> {
    let value = properties.get(name).map(unwrap_script_value)?;
    let value = value.as_f64()? as f32;
    (value.is_finite() && range.contains(&value)).then_some(value)
}

fn parse_script_call_usize(script: &str, callee: &str) -> Option<usize> {
    let mut remaining = script;
    while let Some(offset) = remaining.find(callee) {
        let after_name = &remaining[offset + callee.len()..];
        let after_name = after_name.trim_start_matches(char::is_whitespace);
        let Some(after_open) = after_name.strip_prefix('(') else {
            remaining = after_name;
            continue;
        };
        let after_open = after_open.trim_start_matches(char::is_whitespace);
        let digit_count = after_open
            .as_bytes()
            .iter()
            .take_while(|byte| byte.is_ascii_digit())
            .count();
        if digit_count == 0 {
            remaining = after_open;
            continue;
        }
        let (digits, after_digits) = after_open.split_at(digit_count);
        if after_digits
            .trim_start_matches(char::is_whitespace)
            .starts_with(')')
        {
            return digits.parse().ok();
        }
        remaining = after_digits;
    }
    None
}

fn script_contains_ignoring_ascii_whitespace(script: &str, pattern: &str) -> bool {
    let pattern = pattern.as_bytes();
    let mut matched = 0;
    for byte in script.bytes().filter(|byte| !byte.is_ascii_whitespace()) {
        if byte == pattern[matched] {
            matched += 1;
            if matched == pattern.len() {
                return true;
            }
        } else {
            matched = usize::from(byte == pattern[0]);
        }
    }
    false
}

fn parse_iris_effect(
    effects: Option<&Value>,
    path: &str,
) -> Result<Option<IrisEffect>, SceneParseError> {
    let Some(effects) = effects.and_then(Value::as_array) else {
        return Ok(None);
    };
    for (index, effect) in effects.iter().enumerate() {
        let Some(effect) = effect.as_object() else {
            continue;
        };
        if effect.get("file").and_then(Value::as_str) != Some("effects/iris/effect.json")
            || !effect_visible(effect)
        {
            continue;
        }
        let values = first_effect_values(effect, path, index, "iris")?;
        let scale = values
            .get("scale")
            .map(|value| parse_vec2(&format!("{path}.effects[{index}].scale"), value))
            .transpose()?
            .unwrap_or(Vec2 { x: 1.0, y: 1.0 });
        let effect = IrisEffect {
            speed: optional_finite_number(values, "speed").unwrap_or(1.0),
            roughness: optional_finite_number(values, "rough").unwrap_or(0.2),
            noise_amount: optional_finite_number(values, "noiseamount").unwrap_or(0.5),
            phase: optional_finite_number(values, "phase").unwrap_or(0.0),
            scale,
            mask: effect_texture(effect, index, 1)?,
        };
        if effect.speed < 0.0
            || !(0.0..=1.0).contains(&effect.roughness)
            || effect.noise_amount < 0.0
            || effect.scale.x <= 0.0
            || effect.scale.y <= 0.0
        {
            return Err(SceneParseError::InvalidValue {
                field: format!("{path}.effects[{index}]"),
                detail: "iris parameters are outside supported ranges".into(),
            });
        }
        return Ok(Some(effect));
    }
    Ok(None)
}

fn parse_foliage_sway_effects(
    effects: Option<&Value>,
    path: &str,
) -> Result<Vec<FoliageSwayEffect>, SceneParseError> {
    let Some(effects) = effects.and_then(Value::as_array) else {
        return Ok(Vec::new());
    };
    let mut parsed = Vec::new();
    for (index, effect) in effects.iter().enumerate() {
        let Some(effect) = effect.as_object() else {
            continue;
        };
        if effect.get("file").and_then(Value::as_str) != Some("effects/foliagesway/effect.json")
            || !effect_visible(effect)
        {
            continue;
        }
        let values = first_effect_values(effect, path, index, "foliage sway")?;
        let sway = FoliageSwayEffect {
            direction: optional_finite_number(values, "scrolldirection").unwrap_or(0.0),
            scale: optional_finite_number(values, "scale").unwrap_or(0.05),
            speed: optional_finite_number(values, "speeduv").unwrap_or(5.0),
            strength: optional_finite_number(values, "strength").unwrap_or(0.4),
            phase: optional_finite_number(values, "phase").unwrap_or(0.5),
            power: optional_finite_number(values, "power").unwrap_or(1.0),
            ratio: optional_finite_number(values, "ratio").unwrap_or(0.3),
            mask: effect_texture(effect, index, 1)?,
        };
        if sway.scale < 0.0
            || sway.speed < 0.0
            || sway.strength < 0.0
            || sway.power <= 0.0
            || sway.ratio <= 0.0
        {
            return Err(SceneParseError::InvalidValue {
                field: format!("{path}.effects[{index}]"),
                detail: "foliage sway parameters are outside supported ranges".into(),
            });
        }
        if parsed.len() < 2 {
            parsed.push(sway);
        }
    }
    Ok(parsed)
}

fn parse_shine_effect(
    effects: Option<&Value>,
    path: &str,
) -> Result<Option<ShineEffect>, SceneParseError> {
    let Some(effects) = effects.and_then(Value::as_array) else {
        return Ok(None);
    };
    for (index, effect) in effects.iter().enumerate() {
        let Some(effect) = effect.as_object() else {
            continue;
        };
        if effect.get("file").and_then(Value::as_str) != Some("effects/shine/effect.json")
            || !effect_visible(effect)
        {
            continue;
        }
        let passes = effect
            .get("passes")
            .and_then(Value::as_array)
            .ok_or_else(|| SceneParseError::InvalidValue {
                field: format!("{path}.effects[{index}]"),
                detail: "shine effect is missing passes".into(),
            })?;
        let values = passes
            .iter()
            .filter_map(|pass| pass.get("constantshadervalues").and_then(Value::as_object))
            .find(|values| values.contains_key("rayintensity"))
            .ok_or_else(|| SceneParseError::InvalidValue {
                field: format!("{path}.effects[{index}]"),
                detail: "shine effect is missing ray parameters".into(),
            })?;
        let color = values
            .get("color")
            .map(|value| parse_vec3(&format!("{path}.effects[{index}].color"), value))
            .transpose()?
            .unwrap_or(Vec3 {
                x: 1.0,
                y: 1.0,
                z: 1.0,
            });
        let shine = ShineEffect {
            direction: optional_finite_number(values, "direction").unwrap_or(0.0),
            speed: optional_finite_number(values, "speed").unwrap_or(0.0),
            intensity: optional_finite_number(values, "rayintensity").unwrap_or(1.0),
            length: optional_finite_number(values, "raylength").unwrap_or(0.1),
            color,
            mask: effect_texture(effect, index, 1)?,
        };
        if shine.intensity < 0.0 || shine.length <= 0.0 {
            return Err(SceneParseError::InvalidValue {
                field: format!("{path}.effects[{index}]"),
                detail: "shine intensity must be non-negative and length positive".into(),
            });
        }
        return Ok(Some(shine));
    }
    Ok(None)
}

fn effect_visible(effect: &Map<String, Value>) -> bool {
    effect
        .get("visible")
        .map(|value| unwrap_script_value(value).as_bool().unwrap_or(true))
        .unwrap_or(true)
}

fn first_effect_values<'a>(
    effect: &'a Map<String, Value>,
    path: &str,
    index: usize,
    name: &str,
) -> Result<&'a Map<String, Value>, SceneParseError> {
    effect
        .get("passes")
        .and_then(Value::as_array)
        .and_then(|passes| passes.first())
        .and_then(|pass| pass.get("constantshadervalues"))
        .and_then(Value::as_object)
        .ok_or_else(|| SceneParseError::InvalidValue {
            field: format!("{path}.effects[{index}]"),
            detail: format!("{name} effect is missing constant shader values"),
        })
}

fn parse_shake_effects(
    effects: Option<&Value>,
    path: &str,
) -> Result<Vec<ShakeEffect>, SceneParseError> {
    let Some(effects) = effects.and_then(Value::as_array) else {
        return Ok(Vec::new());
    };
    let mut parsed = Vec::new();
    for (index, effect) in effects.iter().enumerate() {
        let Some(effect) = effect.as_object() else {
            continue;
        };
        if effect.get("file").and_then(Value::as_str) != Some("effects/shake/effect.json")
            || !effect_visible(effect)
        {
            continue;
        }
        let values = first_effect_values(effect, path, index, "shake")?;
        let bounds = values
            .get("bounds")
            .map(|value| parse_vec2(&format!("{path}.effects[{index}].bounds"), value))
            .transpose()?
            .unwrap_or(Vec2 { x: 0.0, y: 1.0 });
        let friction = values
            .get("friction")
            .map(|value| parse_vec2(&format!("{path}.effects[{index}].friction"), value))
            .transpose()?
            .unwrap_or(Vec2 { x: 1.0, y: 1.0 });
        let shake = ShakeEffect {
            speed: optional_finite_number(values, "speed").unwrap_or(1.0),
            strength: optional_finite_number(values, "strength").unwrap_or(0.1),
            bounds,
            friction,
            direction_map: effect_texture(effect, index, 1)?,
        };
        if shake.speed < 0.0
            || shake.strength < 0.0
            || shake.bounds.y <= shake.bounds.x
            || shake.friction.x <= 0.0
            || shake.friction.y <= 0.0
        {
            return Err(SceneParseError::InvalidValue {
                field: format!("{path}.effects[{index}]"),
                detail: "shake parameters are outside supported ranges".into(),
            });
        }
        if parsed.len() < MAX_SHAKE_EFFECTS {
            parsed.push(shake);
        }
    }
    Ok(parsed)
}

fn parse_pulse_effects(
    effects: Option<&Value>,
    path: &str,
) -> Result<Vec<PulseEffect>, SceneParseError> {
    let Some(effects) = effects.and_then(Value::as_array) else {
        return Ok(Vec::new());
    };
    let mut parsed = Vec::new();
    for (index, effect) in effects.iter().enumerate() {
        let Some(effect) = effect.as_object() else {
            continue;
        };
        if effect.get("file").and_then(Value::as_str) != Some("effects/pulse/effect.json")
            || !effect_visible(effect)
        {
            continue;
        }
        let values = first_effect_values(effect, path, index, "pulse")?;
        let bounds = values
            .get("bounds")
            .map(|value| parse_vec2(&format!("{path}.effects[{index}].bounds"), value))
            .transpose()?
            .unwrap_or(Vec2 { x: 0.0, y: 1.0 });
        let tint_low = values
            .get("tintlow")
            .map(|value| parse_vec3(&format!("{path}.effects[{index}].tintlow"), value))
            .transpose()?
            .unwrap_or(Vec3 {
                x: 1.0,
                y: 1.0,
                z: 1.0,
            });
        let tint_high = values
            .get("tinthigh")
            .map(|value| parse_vec3(&format!("{path}.effects[{index}].tinthigh"), value))
            .transpose()?
            .unwrap_or(Vec3 {
                x: 1.0,
                y: 1.0,
                z: 1.0,
            });
        let pulse = PulseEffect {
            speed: optional_finite_number(values, "speed").unwrap_or(3.0),
            phase: optional_finite_number(values, "phase").unwrap_or(0.0),
            amount: optional_finite_number(values, "amount").unwrap_or(1.0),
            bounds,
            power: optional_finite_number(values, "power").unwrap_or(1.0),
            tint_low,
            tint_high,
            mask: effect_texture(effect, index, 2)?,
        };
        if pulse.speed < 0.0
            || pulse.amount < 0.0
            || pulse.bounds.y <= pulse.bounds.x
            || pulse.power <= 0.0
        {
            return Err(SceneParseError::InvalidValue {
                field: format!("{path}.effects[{index}]"),
                detail: "pulse parameters are outside supported ranges".into(),
            });
        }
        if parsed.len() < MAX_PULSE_EFFECTS {
            parsed.push(pulse);
        }
    }
    Ok(parsed)
}

fn parse_spin_effect(
    effects: Option<&Value>,
    path: &str,
) -> Result<Option<SpinEffect>, SceneParseError> {
    let Some(effects) = effects.and_then(Value::as_array) else {
        return Ok(None);
    };
    for (index, effect) in effects.iter().enumerate() {
        let Some(effect) = effect.as_object() else {
            continue;
        };
        if effect.get("file").and_then(Value::as_str) != Some("effects/spin/effect.json")
            || !effect_visible(effect)
        {
            continue;
        }
        let values = first_effect_values(effect, path, index, "spin")?;
        let center = values
            .get("center")
            .map(|value| parse_vec2(&format!("{path}.effects[{index}].center"), value))
            .transpose()?
            .unwrap_or(Vec2 { x: 0.5, y: 0.5 });
        return Ok(Some(SpinEffect {
            speed: optional_finite_number(values, "speed").unwrap_or(1.0),
            center,
        }));
    }
    Ok(None)
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

fn parse_water_wave_effects(
    effects: Option<&Value>,
    path: &str,
) -> Result<Vec<WaterWaveEffect>, SceneParseError> {
    let Some(effects) = effects.and_then(Value::as_array) else {
        return Ok(Vec::new());
    };
    let mut parsed = Vec::new();
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
        let wave = WaterWaveEffect {
            direction,
            scale,
            speed,
            strength,
            mask: effect_texture(effect, index, 1)?,
            normal: None,
            ripple: None,
        };
        if parsed.len() < MAX_WATER_WAVE_EFFECTS {
            parsed.push(wave);
        }
    }
    if !parsed.is_empty() {
        return Ok(parsed);
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
        let ratio = optional_finite_number(values, "ratio").unwrap_or(1.0);
        let strength = optional_finite_number(values, "ripplestrength").unwrap_or(0.0);
        if scale <= 0.0 || ratio <= 0.0 || strength < 0.0 {
            return Err(SceneParseError::InvalidValue {
                field: format!("{path}.effects[{index}]"),
                detail: "waterripple scale and ratio must be positive and strength non-negative"
                    .into(),
            });
        }
        parsed.push(WaterWaveEffect {
            direction,
            scale,
            speed: 0.0,
            strength,
            mask: effect_texture(effect, index, 1)?,
            normal: effect_texture(effect, index, 2)?,
            ripple: Some(WaterRippleEffect {
                animation_speed,
                scroll_speed,
                ratio,
            }),
        });
        break;
    }
    Ok(parsed)
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
        if !effect
            .get("file")
            .and_then(Value::as_str)
            .is_some_and(|file| file.ends_with("/scroll/effect.json"))
        {
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
        .map(unwrap_script_value)
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

    fn audio_visualizer_scene(bins: usize, bar_width: f64, scale_y: f64, origin_x: f64) -> String {
        serde_json::json!({
            "objects": [{
                "id": 12,
                "name": "Audio spectrum",
                "image": "models/util/solidlayer.json",
                "visible": {
                    "script": format!(r#"
                        'use strict';
                        const audioData = engine.registerAudioBuffers( {bins} );
                        const bars = [];
                        for (let i = 0; i < {bins}; ++i) {{
                            const bar = thisScene.createLayer('models/util/solidlayer.json');
                            bar.scale.y = audioData.average[ i ] * scaleY;
                            bars.push(bar);
                        }}
                    "#),
                    "scriptproperties": {
                        "barWidth": {"order": 1, "type": "slider", "value": bar_width},
                        "scaleY": {"order": 2, "type": "slider", "value": scale_y},
                        "originX": {"order": 3, "type": "slider", "value": origin_x}
                    },
                    "value": true
                }
            }]
        })
        .to_string()
    }

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
    fn recognizes_complete_audio_visualizer_visible_script_without_executing_it() {
        let graph = parse_scene_graph(&audio_visualizer_scene(64, 14.5, 320.0, -7.25)).unwrap();

        assert_eq!(
            graph.nodes[0].audio_visualizer,
            Some(SceneAudioVisualizer {
                bins: 64,
                bar_width: 14.5,
                height_scale: 320.0,
                spacing: -7.25,
            })
        );
        assert!(graph.unsupported_features.is_empty());
    }

    #[test]
    fn requires_the_complete_audio_visualizer_pattern() {
        let complete = audio_visualizer_scene(64, 10.0, 100.0, 5.0);
        for missing in [
            "engine.registerAudioBuffers",
            "audioData.average[ i ]",
            "thisScene.createLayer",
        ] {
            let scene = complete.replacen(missing, "missingFeature", 1);
            let graph = parse_scene_graph(&scene).unwrap();
            assert_eq!(
                graph.nodes[0].audio_visualizer, None,
                "accepted script without {missing}"
            );
            assert!(graph.unsupported_features.iter().any(|feature| {
                feature.path == "objects[0].visible.script"
                    && feature.feature == "unrecognized visible script"
            }));
        }

        for missing in ["barWidth", "scaleY", "originX"] {
            let mut scene: Value = serde_json::from_str(&complete).unwrap();
            scene["objects"][0]["visible"]["scriptproperties"]
                .as_object_mut()
                .unwrap()
                .remove(missing);
            let graph = parse_scene_graph(&scene.to_string()).unwrap();
            assert_eq!(
                graph.nodes[0].audio_visualizer, None,
                "accepted script without {missing}"
            );
            assert!(graph.unsupported_features.iter().any(|feature| {
                feature.path == "objects[0].visible.script"
                    && feature.feature == "unrecognized visible script"
            }));
        }
    }

    #[test]
    fn enforces_audio_visualizer_safety_boundaries() {
        for (bins, bar_width, scale_y, origin_x) in
            [(8, 0.0, 0.0, -500.0), (128, 100.0, 500.0, 500.0)]
        {
            let graph =
                parse_scene_graph(&audio_visualizer_scene(bins, bar_width, scale_y, origin_x))
                    .unwrap();
            assert!(graph.nodes[0].audio_visualizer.is_some());
        }

        for (bins, bar_width, scale_y, origin_x) in [
            (7, 10.0, 100.0, 0.0),
            (129, 10.0, 100.0, 0.0),
            (64, -0.01, 100.0, 0.0),
            (64, 100.01, 100.0, 0.0),
            (64, 10.0, -0.01, 0.0),
            (64, 10.0, 500.01, 0.0),
            (64, 10.0, 100.0, -500.01),
            (64, 10.0, 100.0, 500.01),
        ] {
            let graph =
                parse_scene_graph(&audio_visualizer_scene(bins, bar_width, scale_y, origin_x))
                    .unwrap();
            assert_eq!(graph.nodes[0].audio_visualizer, None);
        }
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
                {"id":1,"sound":"audio/music.ogg","playbackmode":"random"}
            ]}"#,
        )
        .unwrap();
        assert_eq!(graph.unsupported_features[0].path, "objects[0]");
        assert_eq!(
            graph.unsupported_features[1].path,
            "objects[1].playbackmode"
        );
    }

    #[test]
    fn parses_sound_fields_from_singleton_array_and_string() {
        let graph = parse_scene_graph(
            r#"{"objects":[
                {"sound":["sounds/music.flac"],"playbackmode":"loop","volume":0.75,"startsilent":true},
                {"sound":"sounds/chime.ogg"}
            ]}"#,
        )
        .unwrap();
        let SceneNodeKind::Sound(music) = &graph.nodes[0].kind else {
            panic!("expected a sound node");
        };
        assert_eq!(music.resource, "sounds/music.flac");
        assert_eq!(music.playback_mode, SoundPlaybackMode::Loop);
        assert_eq!(music.volume, 0.75);
        assert!(music.start_silent);
        assert!(graph.nodes[0].unknown_fields.is_empty());

        let SceneNodeKind::Sound(chime) = &graph.nodes[1].kind else {
            panic!("expected a sound node");
        };
        assert_eq!(chime.resource, "sounds/chime.ogg");
        assert_eq!(chime.playback_mode, SoundPlaybackMode::Once);
        assert_eq!(chime.volume, 1.0);
        assert!(!chime.start_silent);
        assert!(graph.unsupported_features.is_empty());
    }

    #[test]
    fn reports_advanced_and_unknown_sound_fields_as_unsupported() {
        let graph = parse_scene_graph(
            r#"{"objects":[{"sound":"sounds/music.flac","playbackmode":"random","mintime":1,"maxtime":5}]}"#,
        )
        .unwrap();
        let SceneNodeKind::Sound(sound) = &graph.nodes[0].kind else {
            panic!("expected a sound node");
        };
        assert_eq!(sound.playback_mode, SoundPlaybackMode::Once);
        assert_eq!(graph.nodes[0].unknown_fields, ["maxtime", "mintime"]);
        assert!(graph.unsupported_features.iter().any(|feature| {
            feature.path == "objects[0].playbackmode"
                && feature.feature == "sound playback mode: random"
        }));
        assert!(
            graph
                .unsupported_features
                .iter()
                .any(|feature| feature.path == "objects[0].mintime")
        );
    }

    #[test]
    fn rejects_invalid_sound_fields() {
        for json in [
            r#"{"objects":[{"sound":["a.ogg","b.ogg"]}]}"#,
            r#"{"objects":[{"sound":"a.ogg","volume":-0.1}]}"#,
            r#"{"objects":[{"sound":"a.ogg","volume":1.1}]}"#,
            r#"{"objects":[{"sound":"a.ogg","startsilent":"false"}]}"#,
            r#"{"objects":[{"sound":"a.ogg","playbackmode":1}]}"#,
        ] {
            assert!(
                parse_scene_graph(json).is_err(),
                "accepted invalid input: {json}"
            );
        }
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
    fn recognizes_safe_native_clock_date_and_second_patterns() {
        let graph = parse_scene_graph(
            r#"{
                "objects":[
                    {"id":1,"name":"Clock","text":{"script":"new Date().getHours(); new Date().getMinutes();","value":"12:34"},"size":"100 30"},
                    {"id":2,"name":"Date","font":"fonts/date.ttf","pointsize":28,"text":{"script":"new Date().getFullYear(); new Date().getMonth(); new Date().getDate();","value":"2021 | 02 | 01"},"size":"200 30"},
                    {"id":3,"image":"models/util/solidlayer.json","scale":{"script":"value.x = new Date().getSeconds() / 60;","value":"1 1 1"},"size":"100 2"}
                ]
            }"#,
        )
        .unwrap();

        let SceneNodeKind::Text(clock) = &graph.nodes[0].kind else {
            panic!("clock should parse as text");
        };
        assert_eq!(clock.dynamic, Some(DynamicTextKind::Clock));
        let SceneNodeKind::Text(date) = &graph.nodes[1].kind else {
            panic!("date should parse as text");
        };
        assert_eq!(date.dynamic, Some(DynamicTextKind::Date));
        assert_eq!(date.font, "fonts/date.ttf");
        assert_eq!(date.point_size, 28.0);
        assert_eq!(
            graph.nodes[2].dynamic_scale,
            Some(DynamicScaleKind::ClockSecondX)
        );
        assert!(
            graph
                .unsupported_features
                .iter()
                .all(|feature| feature.feature != "text object")
        );
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
    fn applies_combo_conditions_from_object_style_user_bindings() {
        let properties = BTreeMap::from([("mode".into(), "0".into())]);
        let graph = parse_scene_graph_with_properties(
            r#"{"objects":[{"image":"bg.json","visible":{"user":{"name":"mode","condition":"1"},"value":true}}]}"#,
            &properties,
        )
        .unwrap();
        assert!(!graph.nodes[0].visible);

        let properties = BTreeMap::from([("mode".into(), "1".into())]);
        let graph = parse_scene_graph_with_properties(
            r#"{"objects":[{"image":"bg.json","visible":{"user":{"name":"mode","condition":"1"},"value":false}}]}"#,
            &properties,
        )
        .unwrap();
        assert!(graph.nodes[0].visible);

        let properties = BTreeMap::from([("enabled".into(), "false".into())]);
        let graph = parse_scene_graph_with_properties(
            r#"{"objects":[{"image":"bg.json","alpha":{"user":"enabled","value":1.0}}]}"#,
            &properties,
        )
        .unwrap();
        assert_eq!(graph.nodes[0].transform.opacity, Some(0.0));
    }

    #[test]
    fn parses_masked_iris_foliage_and_shine_animations() {
        let properties = BTreeMap::from([("eye_speed".into(), "0.75".into())]);
        let graph = parse_scene_graph_with_properties(
            r#"{"objects":[{"image":"models/scene.json","effects":[
                {"file":"effects/shine/effect.json","passes":[
                    {"constantshadervalues":{"noiseamount":0.4},"textures":[null,"masks/shine"]},
                    {"constantshadervalues":{"color":"0.8 0.5 0.4","direction":1.1,"rayintensity":0.24,"raylength":0.42,"speed":0.2}}
                ]},
                {"file":"effects/iris/effect.json","passes":[{"constantshadervalues":{"noiseamount":0.5,"phase":0,"rough":0.2,"scale":"1 1","speed":{"user":"eye_speed","value":0.44}},"textures":[null,"masks/iris"]}]},
                {"file":"effects/foliagesway/effect.json","passes":[{"constantshadervalues":{"phase":0.5,"power":1,"ratio":0.3,"scale":0.05,"scrolldirection":2.2,"speeduv":2.29,"strength":0.59},"textures":[null,"masks/foliage-a"]}]},
                {"file":"effects/foliagesway/effect.json","passes":[{"constantshadervalues":{"phase":0.4,"power":1.2,"ratio":0.4,"scale":0.06,"scrolldirection":0.4,"speeduv":2.48,"strength":0.68},"textures":[null,"masks/foliage-b"]}]}
            ]}]}"#,
            &properties,
        )
        .unwrap();
        let node = &graph.nodes[0];
        assert_eq!(node.iris.as_ref().unwrap().speed, 0.75);
        assert_eq!(
            node.iris.as_ref().unwrap().mask.as_deref(),
            Some("masks/iris")
        );
        assert_eq!(node.foliage_sway.len(), 2);
        assert_eq!(
            node.foliage_sway[1].mask.as_deref(),
            Some("masks/foliage-b")
        );
        assert_eq!(node.shine.as_ref().unwrap().intensity, 0.24);
        assert_eq!(
            node.shine.as_ref().unwrap().mask.as_deref(),
            Some("masks/shine")
        );
        assert!(graph.unsupported_features.is_empty());
    }

    #[test]
    fn parses_enabled_real_scroll_effect_parameters() {
        let graph = parse_scene_graph(
            r#"{"objects":[{"image":"clouds.json","effects":[{"file":"effects/workshop/2098390419/scroll/effect.json","visible":true,"passes":[{"constantshadervalues":{"repeat":"2 1","speedx":-0.3,"speedy":0.2}}]}]}]}"#,
        )
        .unwrap();
        let scroll = graph.nodes[0].scroll.unwrap();
        assert_eq!(scroll.speed_x, -0.3);
        assert_eq!(scroll.speed_y, 0.2);
        assert_eq!(scroll.repeat_x, 2.0);
        assert!(graph.unsupported_features.is_empty());
    }

    #[test]
    fn parses_masked_shake_pulse_and_spin_effects() {
        let graph = parse_scene_graph(
            r#"{"objects":[{"image":"character.json","effects":[
                {"file":"effects/pulse/effect.json","passes":[{"constantshadervalues":{"amount":1,"bounds":"0 1","phase":0.2,"power":1,"speed":3,"tintlow":"1 0.7 0.5","tinthigh":"1 1 1"},"textures":[null,null,"masks/pulse"]}]},
                {"file":"effects/shake/effect.json","passes":[{"constantshadervalues":{"bounds":"0.2 1","friction":"1 2","speed":1.2,"strength":0.15},"textures":[null,"masks/direction"]}]},
                {"file":"effects/spin/effect.json","passes":[{"constantshadervalues":{"center":"0.5 0.4","speed":0.5}}]}
            ]}]}"#,
        )
        .unwrap();
        let node = &graph.nodes[0];
        assert_eq!(node.pulses.len(), 1);
        assert_eq!(node.pulses[0].mask.as_deref(), Some("masks/pulse"));
        assert_eq!(node.shakes.len(), 1);
        assert_eq!(
            node.shakes[0].direction_map.as_deref(),
            Some("masks/direction")
        );
        assert_eq!(node.spin.unwrap().speed, 0.5);
        assert!(graph.unsupported_features.is_empty());
    }

    #[test]
    fn parses_real_waterwaves_parameters() {
        let graph = parse_scene_graph(
            r#"{"objects":[{"image":"foreground.json","effects":[{"file":"effects/waterwaves/effect.json","visible":true,"passes":[{"constantshadervalues":{"direction":1.5772198,"scale":0.01,"speed":1.41,"strength":0.06}}]}]}]}"#,
        )
        .unwrap();
        let wave = &graph.nodes[0].water_waves[0];
        assert_eq!(wave.direction, 1.5772198);
        assert_eq!(wave.scale, 0.01);
        assert_eq!(wave.speed, 1.41);
        assert_eq!(wave.strength, 0.06);
        assert!(graph.unsupported_features.is_empty());
    }

    #[test]
    fn parses_waterflow_alongside_waterripple() {
        let graph = parse_scene_graph(
            r#"{"objects":[{"image":"water.json","effects":[{"file":"effects/waterripple/effect.json","passes":[{"constantshadervalues":{"animationspeed":0.03,"scrollspeed":0.08,"scale":1.0,"ratio":3.08,"ripplestrength":0.1},"textures":[null,"masks/ripple","effects/normal"]}]},{"file":"effects/waterflow/effect.json","passes":[{"constantshadervalues":{"phasescale":0.62,"speed":0.08,"strength":1.0},"textures":[null,"masks/flow","effects/phase"]}]}]}]}"#,
        )
        .unwrap();
        let node = &graph.nodes[0];
        let wave = &node.water_waves[0];
        assert_eq!(wave.speed, 0.0);
        assert_eq!(wave.mask.as_deref(), Some("masks/ripple"));
        assert_eq!(wave.normal.as_deref(), Some("effects/normal"));
        let ripple = wave.ripple.unwrap();
        assert_eq!(ripple.animation_speed, 0.03);
        assert_eq!(ripple.scroll_speed, 0.08);
        assert_eq!(ripple.ratio, 3.08);
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

    #[test]
    fn parses_single_scalar_timelines_and_clamps_at_the_last_keyframe() {
        let graph = parse_scene_graph(
            r#"{
                "general":{"zoom":{"animation":{"c0":[{"frame":0,"value":1.2},{"frame":210,"value":1.02}],"options":{"fps":30,"length":210,"mode":"single"}},"value":1.02}},
                "objects":[{"image":"logo.json","alpha":{"animation":{"c0":[{"frame":0,"value":1},{"frame":120,"value":0}],"options":{"fps":30,"length":120,"mode":"single"}},"value":1}}]
            }"#,
        )
        .unwrap();
        let zoom = graph.camera.zoom_animation.as_ref().unwrap();
        assert!((zoom.value_at(3.5) - 1.11).abs() < 0.0001);
        assert!((zoom.value_at(30.0) - 1.02).abs() < 0.0001);
        let opacity = graph.nodes[0].transform.opacity_animation.as_ref().unwrap();
        assert_eq!(opacity.value_at(0.0), 1.0);
        assert_eq!(opacity.value_at(30.0), 0.0);
    }

    #[test]
    fn recognizes_media_text_and_instance_texture_fallbacks() {
        let graph = parse_scene_graph(
            r#"{"objects":[{"image":"models/album.json","instance":{"textures":["album-fallback"],"usertextures":[{"name":"$mediaThumbnail","type":"system"}]}},{"text":{"script":"export function mediaPropertiesChanged(event) { value = event.title; }","value":"Song Title"}}]}"#,
        )
        .unwrap();
        assert_eq!(
            graph.nodes[0].texture_override.as_deref(),
            Some("album-fallback")
        );
        let SceneNodeKind::Text(text) = &graph.nodes[1].kind else {
            panic!("media text should parse as text");
        };
        assert_eq!(text.dynamic, Some(DynamicTextKind::Media));
    }

    #[test]
    fn converts_a_single_color_grading_entrance_into_a_black_fade() {
        let graph = parse_scene_graph(
            r#"{"objects":[{"image":"models/util/composelayer.json","effects":[{"file":"effects/workshop/example/color_grading/effect.json","passes":[{"constantshadervalues":{"Brightness":{"animation":{"c0":[{"frame":0,"value":-1},{"frame":120,"value":0}],"options":{"fps":30,"length":120,"mode":"single"}},"value":0}}}]}]}]}"#,
        )
        .unwrap();
        let fade = graph.nodes[0].backdrop_fade.as_ref().unwrap();
        assert_eq!(fade.value_at(0.0), 1.0);
        assert_eq!(fade.value_at(4.0), 0.0);
        assert_eq!(fade.value_at(30.0), 0.0);
    }
}
