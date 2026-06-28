use std::{path::Path, time::Duration};

use thiserror::Error;

#[derive(Debug, Error)]
pub enum VideoError {
    #[error("Failed to open video {path}: {message}")]
    Open { path: String, message: String },
    #[error("No decodable video stream")]
    NoVideoStream,
    #[error("No decodable audio stream")]
    NoAudioStream,
    #[error("Failed to decode video frame: {0}")]
    Decode(String),
    #[error("Failed to decode audio samples: {0}")]
    AudioDecode(String),
    #[error("Video playback finished")]
    EndOfStream,
    #[error("Failed to seek to start of video: {0}")]
    Seek(String),
    #[error("Failed to seek to start of audio: {0}")]
    AudioSeek(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rational {
    pub numerator: i32,
    pub denominator: i32,
}

impl Rational {
    pub fn duration(self, timestamp: i64) -> Duration {
        if timestamp <= 0 || self.numerator <= 0 || self.denominator <= 0 {
            return Duration::ZERO;
        }
        Duration::from_secs_f64(timestamp as f64 * self.numerator as f64 / self.denominator as f64)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelFormat {
    Rgba,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ColorInfo {
    pub space: Option<i32>,
    pub range: Option<i32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedFrame {
    pub pixels: Vec<u8>,
    pub format: PixelFormat,
    pub width: u32,
    pub height: u32,
    pub stride: usize,
    pub pts: i64,
    pub time_base: Rational,
    pub color: ColorInfo,
}

impl DecodedFrame {
    pub fn presentation_time(&self) -> Duration {
        self.time_base.duration(self.pts)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DecodeOptions {
    pub hardware: bool,
    /// 解码输出最大高度（px），0 表示不限制
    pub max_height: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MediaInfo {
    pub width: u32,
    pub height: u32,
    pub duration: Option<Duration>,
    pub frame_rate: Option<f64>,
    pub time_base: Rational,
}

pub trait VideoDecoder {
    fn open(&mut self, path: &Path, options: DecodeOptions) -> Result<MediaInfo, VideoError>;
    fn next_frame(&mut self) -> Result<DecodedFrame, VideoError>;
    fn seek_start(&mut self) -> Result<(), VideoError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_pts_using_time_base() {
        assert_eq!(
            Rational {
                numerator: 1,
                denominator: 1_000,
            }
            .duration(1_500),
            Duration::from_millis(1_500)
        );
    }

    #[test]
    fn invalid_or_negative_timestamps_are_zero() {
        assert_eq!(
            Rational {
                numerator: 1,
                denominator: 0,
            }
            .duration(10),
            Duration::ZERO
        );
        assert_eq!(
            Rational {
                numerator: 1,
                denominator: 30,
            }
            .duration(-1),
            Duration::ZERO
        );
    }
}
