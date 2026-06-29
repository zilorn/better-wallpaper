use std::{
    fs::{self, File},
    io::Write,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use tempfile::NamedTempFile;
use thiserror::Error;
use tracing::{debug, info};

pub const CURRENT_CONFIG_VERSION: u32 = 1;

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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct AppConfig {
    pub version: u32,
    pub general: GeneralConfig,
    pub wallpaper: WallpaperConfig,
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
    pub engine_mode: bool,
    pub loop_playback: bool,
    pub muted: bool,
    pub fill_mode: FillMode,
    pub fps_limit: u16,
}

impl Default for WallpaperConfig {
    fn default() -> Self {
        Self {
            path: None,
            engine_mode: false,
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
        if self.version != CURRENT_CONFIG_VERSION {
            return Err(ConfigError::Validation(format!(
                "Unsupported config version {}, current version is {CURRENT_CONFIG_VERSION}",
                self.version
            )));
        }
        if !(1..=240).contains(&self.wallpaper.fps_limit) {
            return Err(ConfigError::Validation(
                "wallpaper.fps_limit must be in the range 1..=240".into(),
            ));
        }
        if self.general.log_level.trim().is_empty() {
            return Err(ConfigError::Validation(
                "general.log_level cannot be empty".into(),
            ));
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
        config.version = 1;
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
}
