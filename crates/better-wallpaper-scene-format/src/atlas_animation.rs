//! Shared sprite admission and affine coordinates for metadata and rendering.
use crate::{TexAtlas, TexError};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextureUv {
    pub origin: [f32; 2],
    pub horizontal: [f32; 2],
    pub vertical: [f32; 2],
}

impl From<[f32; 4]> for TextureUv {
    fn from([left, top, right, bottom]: [f32; 4]) -> Self {
        Self {
            origin: [left, top],
            horizontal: [right - left, 0.0],
            vertical: [0.0, bottom - top],
        }
    }
}

impl TextureUv {
    pub fn at(self, [u, v]: [f32; 2]) -> [f32; 2] {
        std::array::from_fn(|axis| {
            self.origin[axis] + u * self.horizontal[axis] + v * self.vertical[axis]
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SpriteFrame {
    pub image_index: usize,
    pub uv: TextureUv,
    pub duration_seconds: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SpriteAnimation {
    pub frames: Vec<SpriteFrame>,
    pub duration_seconds: f32,
}

impl SpriteAnimation {
    pub fn frame_at(&self, elapsed_seconds: f64) -> Option<&SpriteFrame> {
        if !elapsed_seconds.is_finite() || self.duration_seconds <= 0.0 {
            return self.frames.first();
        }
        let mut cursor = elapsed_seconds.rem_euclid(f64::from(self.duration_seconds));
        for frame in &self.frames {
            if cursor < f64::from(frame.duration_seconds) {
                return Some(frame);
            }
            cursor -= f64::from(frame.duration_seconds);
        }
        self.frames.last()
    }

    pub fn uv_at(&self, elapsed_seconds: f64) -> TextureUv {
        self.frame_at(elapsed_seconds)
            .map_or([0.0, 0.0, 1.0, 1.0].into(), |frame| frame.uv)
    }
}

impl TexAtlas {
    /// Admit the whole frame sequence or return a specific error. Never discard
    /// individual frames or interpret zero-duration particle data as animation.
    /// This uses Better Wallpaper's half-texel inset, not a native parity claim.
    pub fn sprite_animation(&self) -> Result<Option<SpriteAnimation>, TexError> {
        let invalid = |detail: &str| TexError::InvalidData(detail.into());
        if self.is_video {
            return Ok(None);
        }
        if self.frames.is_empty() {
            if self.images.len() != 1 || self.is_animated {
                return Err(invalid("Atlas playback requires a nonempty frame sequence"));
            }
            return Ok(None);
        }
        let images = self.to_texture_images()?;
        let mut frames = Vec::with_capacity(self.frames.len());
        for frame in &self.frames {
            let base = images
                .get(frame.image_index as usize)
                .and_then(|image| image.levels.first())
                .ok_or_else(|| invalid("Atlas frame references a missing image"))?;
            let size = [base.width as f32, base.height as f32];
            let [hx, hy] = frame.horizontal;
            let [vx, vy] = frame.vertical;
            let horizontal_length = hx.hypot(hy);
            let vertical_length = vx.hypot(vy);
            let determinant = hx * vy - hy * vx;
            let raw = TextureUv {
                origin: frame.origin,
                horizontal: frame.horizontal,
                vertical: frame.vertical,
            };
            if !frame.duration_seconds.is_finite() || frame.duration_seconds <= 0.0 {
                return Err(invalid(
                    "Atlas playback requires positive finite frame durations",
                ));
            }
            if !horizontal_length.is_finite()
                || !vertical_length.is_finite()
                || horizontal_length < 1.0
                || vertical_length < 1.0
                || !determinant.is_finite()
                || determinant.abs() < 0.0001
            {
                return Err(invalid(
                    "Atlas playback requires nondegenerate bases of at least one texel",
                ));
            }
            for corner in [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0]] {
                if raw
                    .at(corner)
                    .iter()
                    .zip(size)
                    .any(|(&value, extent)| !value.is_finite() || value < 0.0 || value > extent)
                {
                    return Err(invalid(
                        "Atlas frame corners exceed the selected image bounds",
                    ));
                }
            }
            // Inset along each authored basis, preserving rotation/reflection.
            // A one-texel axis intentionally samples its center throughout.
            let inset = [0.5 / horizontal_length, 0.5 / vertical_length];
            let origin = raw.at(inset);
            frames.push(SpriteFrame {
                image_index: frame.image_index as usize,
                duration_seconds: frame.duration_seconds,
                uv: TextureUv {
                    origin: std::array::from_fn(|axis| origin[axis] / size[axis]),
                    horizontal: std::array::from_fn(|axis| {
                        frame.horizontal[axis] * (1.0 - 2.0 * inset[0]) / size[axis]
                    }),
                    vertical: std::array::from_fn(|axis| {
                        frame.vertical[axis] * (1.0 - 2.0 * inset[1]) / size[axis]
                    }),
                },
            });
        }
        let duration_seconds = frames
            .iter()
            .map(|frame| frame.duration_seconds)
            .sum::<f32>();
        if !duration_seconds.is_finite() || duration_seconds <= 0.0 {
            return Err(invalid(
                "Atlas animation duration exceeds the supported range",
            ));
        }
        Ok(Some(SpriteAnimation {
            frames,
            duration_seconds,
        }))
    }
}
