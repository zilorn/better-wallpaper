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
    pub object_types: Vec<String>,
}

/// Parse and analyse a scene.json file
pub fn analyse_scene(scene_json: &str) -> Result<SceneMetadata, SceneParseError> {
    let json: serde_json::Value =
        serde_json::from_str(scene_json).map_err(SceneParseError::Json)?;

    let mut meta = SceneMetadata::default();

    // Camera
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
        meta.bloom = general
            .get("bloom")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        meta.parallax = general
            .get("cameraparallax")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        meta.shake = general
            .get("camerashake")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        meta.camera_fade = general
            .get("camerafade")
            .and_then(|v| v.as_bool())
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
                        if file.contains("shader")
                            || file.contains("bloom")
                            || file.contains("godrays")
                        {
                            // These are standard effects
                        } else {
                            meta.has_custom_shaders = true;
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
        }
    }

    Ok(meta)
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
}
