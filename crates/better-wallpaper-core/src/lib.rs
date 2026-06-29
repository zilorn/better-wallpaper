pub mod config;
pub mod desktop;
pub mod playback;
pub mod video;

pub use config::{AppConfig, BackendKind, ConfigError, ConfigStore};
pub use desktop::{DesktopDetection, DesktopKind, detect_desktop, select_backend};
pub use playback::PlaybackControl;
pub use video::{
    ColorInfo, DecodeOptions, DecodedFrame, MediaInfo, PixelFormat, Rational, VideoDecoder,
    VideoError,
};
