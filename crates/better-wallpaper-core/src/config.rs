use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::Write,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use tempfile::NamedTempFile;
use thiserror::Error;
use tracing::{debug, info, warn};

pub const CURRENT_CONFIG_VERSION: u32 = 2;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error(
        "Unable to determine user home directory; please set HOME or specify a config file via --config"
    )]
    HomeNotFound,
    #[error("Failed to read config {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("Failed to parse config {path}: {source}")]
    Parse {
        path: PathBuf,
        source: toml::de::Error,
    },
    #[error("Config validation failed: {0}")]
    Validation(String),
    #[error("Failed to write config {path}: {source}")]
    Write {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("Failed to serialize config: {0}")]
    Serialize(#[from] toml::ser::Error),
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BackendKind {
    #[default]
    Auto,
    Niri,
    Kde,
    Headless,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FillMode {
    #[default]
    Cover,
    Contain,
    Stretch,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HardwareDecode {
    #[default]
    Auto,
    Software,
}

/// Wallpaper type. Scene is new in v2 config.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WallpaperType {
    #[default]
    Video,
    Web,
    Scene,
}

/// Scene-specific settings (v2 config)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct SceneConfig {
    pub quality: SceneQuality,
    /// Enable wallpaper-surface pointer input for basic niri scene parallax.
    pub mouse: bool,
    /// Enable authored basic camera parallax in the niri scene renderer.
    pub parallax: bool,
    pub audio_processing: bool,
    /// Reserved for particle rendering; validated and persisted but not enforced.
    pub particle_limit: u32,
    pub script_enabled: bool,
    pub properties: BTreeMap<String, String>,
}

impl Default for SceneConfig {
    fn default() -> Self {
        Self {
            quality: SceneQuality::High,
            mouse: true,
            parallax: true,
            audio_processing: false,
            particle_limit: 10_000,
            script_enabled: false,
            properties: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SceneQuality {
    #[default]
    High,
    Medium,
    Low,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct AppConfig {
    pub version: u32,
    pub general: GeneralConfig,
    pub wallpaper: WallpaperConfig,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scene: Option<SceneConfig>,
    pub library: LibraryConfig,
    pub decode: DecodeConfig,
    pub outputs: Vec<OutputConfig>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            version: CURRENT_CONFIG_VERSION,
            general: GeneralConfig::default(),
            wallpaper: WallpaperConfig::default(),
            scene: Some(SceneConfig::default()),
            library: LibraryConfig::default(),
            decode: DecodeConfig::default(),
            outputs: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct LibraryConfig {
    pub paths: Vec<PathBuf>,
}

impl Default for LibraryConfig {
    fn default() -> Self {
        Self {
            paths: vec![PathBuf::from("~/Videos")],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct GeneralConfig {
    pub backend: BackendKind,
    pub restore_on_start: bool,
    pub log_level: String,
}

impl Default for GeneralConfig {
    fn default() -> Self {
        Self {
            backend: BackendKind::Auto,
            restore_on_start: true,
            log_level: "info".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct WallpaperConfig {
    pub path: Option<PathBuf>,
    /// v1 compat: engine_mode is true for WE projects (migrated from v1)
    #[serde(skip_serializing_if = "is_false")]
    pub engine_mode: bool,
    pub wallpaper_type: WallpaperType,
    pub loop_playback: bool,
    pub muted: bool,
    pub fill_mode: FillMode,
    pub fps_limit: u16,
}

fn is_false(v: &bool) -> bool {
    !v
}

impl Default for WallpaperConfig {
    fn default() -> Self {
        Self {
            path: None,
            engine_mode: false,
            wallpaper_type: WallpaperType::Video,
            loop_playback: true,
            muted: true,
            fill_mode: FillMode::Cover,
            fps_limit: 60,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct DecodeConfig {
    pub hardware: HardwareDecode,
    pub max_height: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OutputConfig {
    pub name: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

const fn default_true() -> bool {
    true
}

impl AppConfig {
    pub fn validate_and_normalize(&mut self, home: &Path) -> Result<(), ConfigError> {
        match self.version {
            1 => self.migrate_v1_to_v2(),
            CURRENT_CONFIG_VERSION => {}
            other => {
                return Err(ConfigError::Validation(format!(
                    "Unsupported config version {}, current version is {CURRENT_CONFIG_VERSION}",
                    other
                )));
            }
        }
        if !(1..=240).contains(&self.wallpaper.fps_limit) {
            return Err(ConfigError::Validation(
                "wallpaper.fps_limit must be in the range 1..=240".into(),
            ));
        }
        if !matches!(self.general.log_level.as_str(), "info" | "debug") {
            return Err(ConfigError::Validation(
                "general.log_level must be either info or debug".into(),
            ));
        }
        if let Some(scene) = &self.scene {
            if scene.particle_limit > 100_000 {
                return Err(ConfigError::Validation(
                    "scene.particle_limit must be in the range 0..=100000".into(),
                ));
            }
            if scene.properties.len() > 256 {
                return Err(ConfigError::Validation(
                    "scene.properties cannot contain more than 256 entries".into(),
                ));
            }
            for (key, value) in &scene.properties {
                if key.is_empty() || key.len() > 128 || value.len() > 4096 {
                    return Err(ConfigError::Validation(
                        "scene property names must be 1..=128 bytes and values at most 4096 bytes"
                            .into(),
                    ));
                }
            }
        }
        if let Some(path) = &self.wallpaper.path {
            self.wallpaper.path = Some(expand_path(path, home));
        }
        self.library.paths = self
            .library
            .paths
            .iter()
            .filter(|path| !path.as_os_str().is_empty())
            .map(|path| expand_path(path, home))
            .collect();
        self.library.paths.sort();
        self.library.paths.dedup();
        Ok(())
    }

    /// Migrate config from v1 to v2
    fn migrate_v1_to_v2(&mut self) {
        info!(old_version = 1, new_version = 2, "migrating config");

        // engine_mode was the old way to indicate engine-type wallpapers.
        // Now we use wallpaper_type directly.
        if self.wallpaper.engine_mode {
            // If wallpaper_type was default (Video), check what it actually is
            // from the path — but we can't infer perfectly, so keep the flag
            // and let the library scanner override it next time.
            warn!(
                "v1 config had engine_mode=true; wallpaper_type will be re-detected on next library scan"
            );
        }

        // Ensure scene config exists
        if self.scene.is_none() {
            self.scene = Some(SceneConfig::default());
        }

        self.version = CURRENT_CONFIG_VERSION;
        self.wallpaper.engine_mode = false; // v2 no longer uses this flag
    }
}

fn expand_path(path: &Path, home: &Path) -> PathBuf {
    if path == Path::new("~") {
        return home.to_path_buf();
    }
    if let Ok(rest) = path.strip_prefix("~/") {
        return home.join(rest);
    }
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        home.join(path)
    }
}

#[derive(Debug, Clone)]
pub struct ConfigStore {
    path: PathBuf,
    home: PathBuf,
}

impl ConfigStore {
    pub fn new(path: PathBuf, home: PathBuf) -> Self {
        Self { path, home }
    }

    pub fn default_path() -> Result<PathBuf, ConfigError> {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .map(|p| p.join(".better-wallpaper/config.toml"))
            .ok_or(ConfigError::HomeNotFound)
    }

    pub fn load_or_create(&self) -> Result<AppConfig, ConfigError> {
        if !self.path.exists() {
            info!(path = %self.path.display(), "Config does not exist, creating default config");
            let mut config = AppConfig::default();
            config.validate_and_normalize(&self.home)?;
            self.save(&config)?;
            return Ok(config);
        }
        debug!(path = %self.path.display(), "Reading config");
        let text = fs::read_to_string(&self.path).map_err(|source| ConfigError::Read {
            path: self.path.clone(),
            source,
        })?;
        let mut config: AppConfig = toml::from_str(&text).map_err(|source| ConfigError::Parse {
            path: self.path.clone(),
            source,
        })?;
        config.validate_and_normalize(&self.home)?;
        info!(path = %self.path.display(), version = config.version, "Config loaded");
        Ok(config)
    }

    pub fn save(&self, config: &AppConfig) -> Result<(), ConfigError> {
        let mut normalized = config.clone();
        normalized.validate_and_normalize(&self.home)?;
        let parent = self.path.parent().unwrap_or(Path::new("."));
        fs::create_dir_all(parent).map_err(|source| ConfigError::Write {
            path: self.path.clone(),
            source,
        })?;
        let content = toml::to_string_pretty(&normalized)?;
        let mut temp = NamedTempFile::new_in(parent).map_err(|source| ConfigError::Write {
            path: self.path.clone(),
            source,
        })?;
        temp.write_all(content.as_bytes())
            .and_then(|_| temp.as_file().sync_all())
            .map_err(|source| ConfigError::Write {
                path: self.path.clone(),
                source,
            })?;
        temp.persist(&self.path).map_err(|e| ConfigError::Write {
            path: self.path.clone(),
            source: e.error,
        })?;
        File::open(parent)
            .and_then(|dir| dir.sync_all())
            .map_err(|source| ConfigError::Write {
                path: self.path.clone(),
                source,
            })?;
        info!(path = %self.path.display(), "Config atomically written");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expands_only_supported_paths_and_validates() {
        let home = Path::new("/home/test");
        let mut config = AppConfig::default();
        config.wallpaper.path = Some("~/Videos/a.mp4".into());
        config.validate_and_normalize(home).unwrap();
        assert_eq!(
            config.wallpaper.path.unwrap(),
            Path::new("/home/test/Videos/a.mp4")
        );
        assert_eq!(
            config.library.paths,
            vec![PathBuf::from("/home/test/Videos")]
        );
    }

    #[test]
    fn normalizes_and_deduplicates_library_paths() {
        let mut config = AppConfig::default();
        config.library.paths = vec!["~/Videos".into(), "media".into(), "~/Videos".into()];
        config
            .validate_and_normalize(Path::new("/home/test"))
            .unwrap();
        assert_eq!(
            config.library.paths,
            vec![
                PathBuf::from("/home/test/Videos"),
                PathBuf::from("/home/test/media")
            ]
        );
    }

    #[test]
    fn creates_and_round_trips_config_atomically() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested/config.toml");
        let store = ConfigStore::new(path.clone(), dir.path().into());
        let mut config = store.load_or_create().unwrap();
        assert!(path.exists());
        config.general.backend = BackendKind::Headless;
        store.save(&config).unwrap();
        assert_eq!(
            store.load_or_create().unwrap().general.backend,
            BackendKind::Headless
        );
    }

    #[test]
    fn rejects_unsupported_version_and_bad_fps() {
        let mut config = AppConfig {
            version: 99,
            ..Default::default()
        };
        assert!(config.validate_and_normalize(Path::new("/tmp")).is_err());
        config.version = CURRENT_CONFIG_VERSION;
        config.wallpaper.fps_limit = 0;
        assert!(config.validate_and_normalize(Path::new("/tmp")).is_err());
    }

    #[test]
    fn invalid_config_is_never_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(&path, "this is not = valid toml [[[").unwrap();
        let original = fs::read(&path).unwrap();
        let store = ConfigStore::new(path.clone(), dir.path().into());
        assert!(matches!(
            store.load_or_create(),
            Err(ConfigError::Parse { .. })
        ));
        assert_eq!(fs::read(path).unwrap(), original);
    }

    #[test]
    fn v1_config_is_migrated_to_v2() {
        // Simulate a v1 config load
        let mut config = AppConfig {
            version: 1,
            wallpaper: WallpaperConfig {
                engine_mode: true,
                ..Default::default()
            },
            ..Default::default()
        };
        config.validate_and_normalize(Path::new("/tmp")).unwrap();
        assert_eq!(config.version, CURRENT_CONFIG_VERSION);
        assert!(!config.wallpaper.engine_mode); // cleared after migration
        assert!(config.scene.is_some()); // scene config was added
    }

    #[test]
    fn scene_config_defaults_are_sensible() {
        let scene = SceneConfig::default();
        assert!(!scene.script_enabled);
        assert_eq!(scene.particle_limit, 10_000);
        assert_eq!(scene.quality, SceneQuality::High);
    }

    #[test]
    fn toml_round_trip_with_scene_config() {
        let mut config = AppConfig::default();
        config.wallpaper.wallpaper_type = WallpaperType::Scene;
        config.scene = Some(SceneConfig {
            quality: SceneQuality::Medium,
            mouse: false,
            particle_limit: 5000,
            properties: BTreeMap::from([("rain".into(), "0.5".into())]),
            ..Default::default()
        });
        let toml_str = toml::to_string_pretty(&config).unwrap();
        let decoded: AppConfig = toml::from_str(&toml_str).unwrap();
        assert_eq!(decoded.wallpaper.wallpaper_type, WallpaperType::Scene);
        let scene = decoded.scene.unwrap();
        assert_eq!(scene.particle_limit, 5000);
        assert_eq!(
            scene.properties.get("rain").map(String::as_str),
            Some("0.5")
        );
    }

    #[test]
    fn rejects_scene_resource_limits() {
        let mut config = AppConfig::default();
        config.scene.as_mut().unwrap().particle_limit = 100_001;
        assert!(config.validate_and_normalize(Path::new("/tmp")).is_err());
    }
}
