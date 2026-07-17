use std::path::PathBuf;

use crate::error::SceneParseError;

/// A project identified as a Wallpaper Engine scene
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SceneProject {
    /// The project title from project.json
    pub title: String,
    /// Path to the project directory
    pub project_dir: PathBuf,
    /// Path to the scene.json (inside the .pkg)
    pub scene_json: Option<String>,
    /// Whether a scene.pkg was found
    pub has_pkg: bool,
    /// Whether a gifscene.pkg was found
    pub has_gif_pkg: bool,
    /// Compatibility level based on feature analysis
    pub compatibility: CompatibilityReport,
    /// Parsed scene metadata (may be partial)
    pub metadata: SceneMetadata,
}

/// Compatibility report for a scene
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CompatibilityReport {
    pub level: CompatibilityLevel,
    pub unsupported_features: Vec<String>,
    pub warnings: Vec<String>,
    pub parse_version: String,
}

/// Compatibility level matching the plan's L0-L4
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum CompatibilityLevel {
    L0 = 0,
    L1 = 1,
    L2 = 2,
    L3 = 3,
    L4 = 4,
}

/// Parsed metadata from a scene.json file
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct SceneMetadata {
    pub camera_eye: Option<String>,
    pub camera_center: Option<String>,
    pub orthogonal_width: Option<u32>,
    pub orthogonal_height: Option<u32>,
    pub bloom: bool,
    pub parallax: bool,
    pub shake: bool,
    pub camera_fade: bool,
    pub object_count: usize,
    pub has_particles: bool,
    pub has_sounds: bool,
    pub has_text: bool,
    pub has_3d_materials: bool,
    pub has_custom_shaders: bool,
    pub has_scene_script: bool,
    pub has_effects: bool,
    pub unsupported_effects: Vec<String>,
    pub object_types: Vec<String>,
}

/// User-adjustable property definition from project.json's general.properties
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct UserProperty {
    /// Property key (used in conditions and value references)
    pub key: String,
    /// Display label
    pub text: String,
    /// Property type
    pub prop_type: PropertyType,
    /// Default value
    pub value: PropertyValue,
    /// Optional condition expression (e.g. "mouseactions.value == true")
    pub condition: Option<String>,
    /// UI sorting order
    pub order: Option<u32>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PropertyType {
    Slider {
        min: f64,
        max: f64,
        step: Option<f64>,
        fraction: bool,
    },
    Bool,
    Combo {
        options: Vec<(String, String)>, // (label, value)
    },
    Color,
    File,
    TextInput,
    Text,
    Group,
    Unknown {
        value: String,
    },
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(untagged)]
pub enum PropertyValue {
    Number(f64),
    Bool(bool),
    String(String),
    #[default]
    None,
}

/// Parse user-adjustable properties from a project.json string.
///
/// Returns a Vec of (key, property) pairs in their original JSON order (keys in a
/// JSON object have no guaranteed order, but we preserve as-read order).
pub fn parse_project_properties(
    project_json: &str,
) -> Result<Vec<(String, UserProperty)>, SceneParseError> {
    let json: serde_json::Value =
        serde_json::from_str(project_json).map_err(SceneParseError::Json)?;
    let props = json
        .pointer("/general/properties")
        .and_then(|v| v.as_object());

    let Some(props) = props else {
        return Ok(Vec::new());
    };

    let mut result = Vec::with_capacity(props.len());
    for (key, prop) in props {
        let prop = prop
            .as_object()
            .ok_or_else(|| SceneParseError::InvalidValue {
                field: format!("general.properties.{key}"),
                detail: "expected an object".into(),
            })?;

        let text = prop
            .get("text")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let condition = prop
            .get("condition")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let order = prop.get("order").and_then(|v| v.as_u64()).map(|v| v as u32);

        let type_str = prop.get("type").and_then(|v| v.as_str()).unwrap_or("");
        let prop_type = match type_str {
            "slider" => PropertyType::Slider {
                min: prop.get("min").and_then(|v| v.as_f64()).unwrap_or(0.0),
                max: prop.get("max").and_then(|v| v.as_f64()).unwrap_or(100.0),
                step: prop.get("step").and_then(|v| v.as_f64()),
                fraction: prop
                    .get("fraction")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false),
            },
            "bool" => PropertyType::Bool,
            "combo" | "color" => {
                if type_str == "combo" {
                    let options = prop
                        .get("options")
                        .and_then(|v| v.as_array())
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|opt| {
                                    let obj = opt.as_object()?;
                                    Some((
                                        obj.get("label")?.as_str()?.to_string(),
                                        obj.get("value")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("")
                                            .to_string(),
                                    ))
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    PropertyType::Combo { options }
                } else {
                    PropertyType::Color
                }
            }
            "file" => PropertyType::File,
            "textinput" => PropertyType::TextInput,
            "text" => PropertyType::Text,
            "group" => PropertyType::Group,
            other => PropertyType::Unknown {
                value: other.to_string(),
            },
        };

        let value = match &prop_type {
            PropertyType::Slider { .. } => prop
                .get("value")
                .and_then(|v| v.as_f64())
                .map(PropertyValue::Number)
                .unwrap_or(PropertyValue::None),
            PropertyType::Bool => prop
                .get("value")
                .and_then(|v| v.as_bool())
                .map(PropertyValue::Bool)
                .unwrap_or(PropertyValue::None),
            _ => prop
                .get("value")
                .and_then(|v| v.as_str())
                .map(|s| PropertyValue::String(s.to_string()))
                .unwrap_or(PropertyValue::None),
        };

        result.push((
            key.clone(),
            UserProperty {
                key: key.clone(),
                text,
                prop_type,
                value,
                condition,
                order,
            },
        ));
    }

    Ok(result)
}

/// Parse and analyse a scene.json file
pub fn analyse_scene(scene_json: &str) -> Result<SceneMetadata, SceneParseError> {
    let json: serde_json::Value =
        serde_json::from_str(scene_json).map_err(SceneParseError::Json)?;

    let mut meta = SceneMetadata::default();
    if let Some(camera) = json.get("camera") {
        meta.camera_eye = camera
            .get("eye")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        meta.camera_center = camera
            .get("center")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
    }

    // General settings
    if let Some(general) = json.get("general") {
        meta.bloom = general.get("bloom").and_then(bound_bool).unwrap_or(false);
        meta.parallax = general
            .get("cameraparallax")
            .and_then(bound_bool)
            .unwrap_or(false);
        meta.shake = general
            .get("camerashake")
            .and_then(bound_bool)
            .unwrap_or(false);
        meta.camera_fade = general
            .get("camerafade")
            .and_then(bound_bool)
            .unwrap_or(false);

        if let Some(proj) = general.get("orthogonalprojection") {
            meta.orthogonal_width = proj.get("width").and_then(|v| v.as_u64()).map(|v| v as u32);
            meta.orthogonal_height = proj
                .get("height")
                .and_then(|v| v.as_u64())
                .map(|v| v as u32);
        }
    }

    // Objects analysis
    if let Some(objects) = json.get("objects").and_then(|v| v.as_array()) {
        meta.object_count = objects.len();

        for obj in objects {
            let name = obj
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("<unnamed>")
                .to_string();

            if obj.get("particle").is_some() || obj.get("instanceoverride").is_some() {
                meta.has_particles = true;
                meta.object_types.push(format!("particle: {name}"));
            }

            if let Some(_sound) = obj.get("sound") {
                meta.has_sounds = true;
                meta.object_types.push(format!("sound: {name}"));
            }

            if let Some(_text) = obj.get("text") {
                meta.has_text = true;
                meta.object_types.push(format!("text: {name}"));
            }

            if let Some(image_val) = obj.get("image") {
                if image_val.is_null() || image_val.as_str().is_none_or(|s| s.is_empty()) {
                    // empty image = non-image or particle
                } else if let Some(img_path) = image_val.as_str() {
                    // Check model reference for 3D
                    if img_path.ends_with(".mdl") {
                        // Model files can be Spriter (2D) - check context
                    }
                    meta.object_types.push(format!("image: {name}"));
                }
            }

            // Effects indicate post-processing
            if let Some(effects) = obj.get("effects").and_then(|v| v.as_array())
                && !effects.is_empty()
            {
                meta.has_effects = true;
                for eff in effects {
                    if let Some(file) = eff.get("file").and_then(|v| v.as_str()) {
                        if is_supported_effect(file) {
                            continue;
                        }
                        if file.contains("/workshop/") || !file.starts_with("effects/") {
                            meta.has_custom_shaders = true;
                        } else if !meta.unsupported_effects.iter().any(|effect| effect == file) {
                            meta.unsupported_effects.push(file.to_owned());
                        }
                    }
                }
            }

            // Check for scene script (script property)
            if let Some(text_val) = obj.get("text").and_then(|v| v.as_str())
                && (text_val.contains("script") || text_val.ends_with(".js"))
            {
                meta.has_scene_script = true;
            }
            if contains_script(obj, 0) {
                meta.has_scene_script = true;
            }
        }
    }

    Ok(meta)
}

fn is_supported_effect(file: &str) -> bool {
    matches!(
        file,
        "effects/scroll/effect.json"
            | "effects/waterwaves/effect.json"
            | "effects/waterripple/effect.json"
            | "effects/waterflow/effect.json"
            | "effects/iris/effect.json"
            | "effects/foliagesway/effect.json"
            | "effects/shine/effect.json"
    )
}

fn bound_bool(value: &serde_json::Value) -> Option<bool> {
    value
        .as_bool()
        .or_else(|| value.get("value").and_then(serde_json::Value::as_bool))
}

fn contains_script(value: &serde_json::Value, depth: usize) -> bool {
    if depth > 64 {
        return false;
    }
    match value {
        serde_json::Value::Object(object) => {
            object
                .get("script")
                .is_some_and(|script| script.as_str().is_some_and(|script| !script.is_empty()))
                || object
                    .values()
                    .any(|value| contains_script(value, depth + 1))
        }
        serde_json::Value::Array(values) => {
            values.iter().any(|value| contains_script(value, depth + 1))
        }
        _ => false,
    }
}

/// Compute compatibility based on scene metadata
pub fn compute_compatibility(meta: &SceneMetadata) -> CompatibilityReport {
    let mut unsupported = Vec::new();
    let mut warnings = Vec::new();

    // L1 checks (2D images, transforms)
    if meta.has_text {
        // Text is L2 in the plan
        unsupported.push("Text objects (planned for L1/L2)".into());
    }
    if meta.has_sounds {
        unsupported.push("Sound objects (planned for L2)".into());
    }
    if meta.has_particles {
        unsupported.push("Particle systems (planned for L3)".into());
    }
    if meta.has_scene_script {
        unsupported.push("SceneScript (planned for L4)".into());
    }
    if meta.has_custom_shaders {
        unsupported.push("Custom shaders (planned for L4)".into());
    }
    unsupported.extend(
        meta.unsupported_effects
            .iter()
            .map(|effect| format!("Unsupported effect: {effect}")),
    );

    if meta.bloom {
        warnings.push("Bloom effect enabled (will be ignored until L3)".into());
    }
    if meta.parallax {
        warnings.push("Parallax effect enabled (will be ignored until L2)".into());
    }
    if meta.shake {
        warnings.push("Camera shake enabled (will be ignored)".into());
    }

    let level = if unsupported.is_empty() {
        CompatibilityLevel::L1
    } else {
        CompatibilityLevel::L0
    };

    CompatibilityReport {
        level,
        unsupported_features: unsupported,
        warnings,
        parse_version: "scene-format v0.1.0".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINIMAL_SCENE: &str = r#"{
        "camera": {
            "center": "540.0 960.0 0.0",
            "eye": "540.0 960.0 1.0",
            "up": "0.0 1.0 0.0"
        },
        "general": {
            "orthogonalprojection": {
                "width": 1080,
                "height": 1920
            },
            "clearcolor": "0.7 0.7 0.7",
            "bloom": false
        },
        "objects": [
            {
                "id": 1,
                "name": "background",
                "image": "models/bg.json",
                "origin": "540.0 960.0 0.0",
                "size": "1080.0 1920.0",
                "visible": true
            }
        ]
    }"#;

    #[test]
    fn parse_minimal_scene() {
        let meta = analyse_scene(MINIMAL_SCENE).unwrap();
        assert_eq!(meta.object_count, 1);
        assert!(!meta.bloom);
        assert!(!meta.has_particles);
        assert!(!meta.has_sounds);
        assert!(!meta.has_effects);
        assert_eq!(meta.orthogonal_width, Some(1080));
        assert_eq!(meta.orthogonal_height, Some(1920));
    }

    #[test]
    fn detects_bound_general_flags_and_property_scripts() {
        let meta = analyse_scene(
            r#"{
                "general":{"bloom":{"user":"hdr","value":true}},
                "objects":[{"image":"models/bg.json","scale":{"script":"return value;","value":"1 1 1"}}]
            }"#,
        )
        .unwrap();
        assert!(meta.bloom);
        assert!(meta.has_scene_script);
    }

    #[test]
    fn detect_unsupported_features() {
        let scene = r#"{
            "camera": {"center": "0 0 0", "eye": "0 0 1", "up": "0 1 0"},
            "general": {"orthogonalprojection": {"width": 1920, "height": 1080}},
            "objects": [
                {"id": 1, "name": "bg", "image": "models/bg.json", "origin": "0 0 0", "size": "100 100", "visible": true},
                {"id": 2, "name": "particles", "particle": "particles/snow.json", "visible": true}
            ]
        }"#;
        let meta = analyse_scene(scene).unwrap();
        assert!(meta.has_particles);
        let compat = compute_compatibility(&meta);
        assert!(
            compat
                .unsupported_features
                .iter()
                .any(|f| f.contains("Particle"))
        );
    }

    #[test]
    fn compute_l1_compatibility() {
        let meta = analyse_scene(MINIMAL_SCENE).unwrap();
        let compat = compute_compatibility(&meta);
        assert_eq!(compat.level, CompatibilityLevel::L1);
    }

    #[test]
    fn distinguishes_supported_builtin_and_unknown_effects() {
        let meta = analyse_scene(
            r#"{"objects":[{"image":"models/bg.json","effects":[
                {"file":"effects/iris/effect.json"},
                {"file":"effects/foliagesway/effect.json"},
                {"file":"effects/twirl/effect.json"}
            ]}]}"#,
        )
        .unwrap();
        assert!(!meta.has_custom_shaders);
        assert_eq!(meta.unsupported_effects, ["effects/twirl/effect.json"]);
        let compatibility = compute_compatibility(&meta);
        assert!(
            compatibility
                .unsupported_features
                .iter()
                .any(|feature| feature.contains("effects/twirl/effect.json"))
        );
    }

    #[test]
    fn reject_missing_camera() {
        let result = analyse_scene(r#"{"general":{}}"#);
        assert!(result.is_ok()); // missing camera is non-fatal, just empty
        let meta = result.unwrap();
        assert!(meta.camera_eye.is_none());
    }

    #[test]
    fn detect_sound_objects() {
        let scene = r#"{
            "camera": {"center": "0 0 0", "eye": "0 0 1", "up": "0 1 0"},
            "general": {"orthogonalprojection": {"width": 100, "height": 100}},
            "objects": [
                {"id": 1, "name": "music", "sound": "sounds/bg.ogg", "visible": true}
            ]
        }"#;
        let meta = analyse_scene(scene).unwrap();
        assert!(meta.has_sounds);
    }

    // ── project.json property parsing ─────────────────────────────────

    #[test]
    fn parse_slider_property() {
        let json = r#"{
            "general": {
                "properties": {
                    "scale": {
                        "fraction": true,
                        "max": 2,
                        "min": 0.7,
                        "order": 100,
                        "step": 0.01,
                        "text": "Scale",
                        "type": "slider",
                        "value": 0.8
                    }
                }
            }
        }"#;
        let props = parse_project_properties(json).unwrap();
        assert_eq!(props.len(), 1);
        assert_eq!(props[0].0, "scale");
        match &props[0].1.prop_type {
            PropertyType::Slider { min, max, .. } => {
                assert_eq!(*min, 0.7);
                assert_eq!(*max, 2.0);
            }
            _ => panic!("expected slider"),
        }
    }

    #[test]
    fn parse_bool_property() {
        let json = r#"{
            "general": {
                "properties": {
                    "show": {
                        "order": 101,
                        "text": "Show",
                        "type": "bool",
                        "value": true
                    }
                }
            }
        }"#;
        let props = parse_project_properties(json).unwrap();
        match &props[0].1.value {
            PropertyValue::Bool(true) => {}
            _ => panic!("expected bool true"),
        }
    }

    #[test]
    fn parse_combo_property() {
        let json = r#"{
            "general": {
                "properties": {
                    "bgm": {
                        "options": [
                            {"label": "Theme 1", "value": "1"},
                            {"label": "Theme 2", "value": "2"}
                        ],
                        "order": 103,
                        "text": "BGM",
                        "type": "combo",
                        "value": "1"
                    }
                }
            }
        }"#;
        let props = parse_project_properties(json).unwrap();
        match &props[0].1.prop_type {
            PropertyType::Combo { options } => {
                assert_eq!(options.len(), 2);
                assert_eq!(options[0].0, "Theme 1");
            }
            _ => panic!("expected combo"),
        }
    }

    #[test]
    fn no_properties_returns_empty() {
        let props = parse_project_properties(r#"{"title": "test", "type": "scene"}"#).unwrap();
        assert!(props.is_empty());
    }

    #[test]
    fn unknown_property_type_serializes_for_library_api() {
        let props = parse_project_properties(
            r#"{"general":{"properties":{"custom":{"type":"custom_widget","value":"x"}}}}"#,
        )
        .unwrap();
        let json = serde_json::to_value(&props[0].1).unwrap();
        assert_eq!(json["prop_type"]["kind"], "unknown");
        assert_eq!(json["prop_type"]["value"], "custom_widget");
    }

    #[test]
    fn parses_condition_field() {
        let json = r#"{
            "general": {
                "properties": {
                    "sub": {
                        "condition": "parent.value == true",
                        "order": 105,
                        "text": "Sub",
                        "type": "bool",
                        "value": false
                    }
                }
            }
        }"#;
        let props = parse_project_properties(json).unwrap();
        assert_eq!(
            props[0].1.condition.as_deref(),
            Some("parent.value == true")
        );
    }
}
