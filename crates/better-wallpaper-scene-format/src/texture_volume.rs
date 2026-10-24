//! Inspection-only volume subset. See docs/reverse-engineering/wallpaper-engine-volumes.md.
//! This model deliberately cannot be converted to the renderer's 2D TextureImage.

use super::{
    FreeImageFormat, MAX_COMPRESSION_RATIO, MAX_MIPMAP_SIZE, MAX_TEXTURE_DIMENSION, TexError,
    TexFormat, decode_png_to_rgba8, get_slice, read_i32_le, read_magic, read_u32_le,
};
use std::sync::Arc;

pub const MAX_VOLUME_DIMENSION: u32 = 8192;
pub const MAX_VOLUME_DEPTH: u32 = 128;
pub const MAX_VOLUME_MIPS: u32 = 32;
pub const MAX_VOLUME_DECODED_BYTES: u64 = 128 * 1024 * 1024;

/// One tightly packed volume mip: x varies fastest, then y, then z.
/// PNG payloads are canonical RGBA8; raw payloads retain their TEX format.
#[derive(Debug, Clone)]
pub struct TexVolumeMip {
    pub width: u32,
    pub height: u32,
    pub depth: u32,
    pub data: Arc<[u8]>,
}

/// A bounded, single-image TEX volume for inspection, without playback admission.
/// Only raw ARGB8888 and vertically stacked PNG payloads are currently supported.
/// Color transfer, LUT coordinates/interpolation and GPU upload are not inferred.
#[derive(Debug, Clone)]
pub struct TexVolume {
    pub format: TexFormat,
    pub flags: u32,
    /// Authored source dimensions, distinct from volume storage dimensions.
    pub width: u32,
    pub height: u32,
    pub texture_width: u32,
    pub texture_height: u32,
    pub depth: u32,
    /// Opaque final TEXI0001 word. Local LUTs have nonzero values here.
    pub info_extra: u32,
    pub container_version: u32,
    pub free_image_format: Option<FreeImageFormat>,
    pub mipmaps: Vec<TexVolumeMip>,
}

fn invalid(message: impl Into<String>) -> TexError {
    TexError::InvalidData(message.into())
}

fn volume_size(width: u32, height: u32, depth: u32) -> Result<u64, TexError> {
    if !(1..=MAX_VOLUME_DIMENSION).contains(&width)
        || !(1..=MAX_VOLUME_DIMENSION).contains(&height)
        || !(1..=MAX_VOLUME_DEPTH).contains(&depth)
    {
        return Err(invalid(format!(
            "Volume dimensions {width}x{height}x{depth} exceed the supported range"
        )));
    }
    let size = u64::from(width)
        .checked_mul(u64::from(height))
        .and_then(|size| size.checked_mul(u64::from(depth)))
        .and_then(|size| size.checked_mul(4))
        .ok_or(TexError::MipmapTooLarge {
            size: u64::MAX,
            max: MAX_MIPMAP_SIZE,
        })?;
    if size > MAX_MIPMAP_SIZE {
        return Err(TexError::MipmapTooLarge {
            size,
            max: MAX_MIPMAP_SIZE,
        });
    }
    Ok(size)
}

impl TexVolume {
    pub fn parse(data: &[u8]) -> Result<Self, TexError> {
        Self::parse_with_budget(data, MAX_VOLUME_DECODED_BYTES)
    }

    /// Limit retained decoded bytes over the entire mip chain, checked before
    /// allocating each payload. One bounded encoded/decompression buffer and PNG
    /// decoder scratch may coexist with the retained pixels.
    pub fn parse_with_budget(data: &[u8], max_decoded_bytes: u64) -> Result<Self, TexError> {
        let max_decoded_bytes = max_decoded_bytes.min(MAX_VOLUME_DECODED_BYTES);
        let mut offset = 0;
        let magic = read_magic(data, &mut offset)?;
        if magic != "TEXV0005" {
            return Err(TexError::BadMagic(magic.into()));
        }
        let info_magic = read_magic(data, &mut offset)?;
        if info_magic != "TEXI0001" {
            return Err(TexError::BadInfoBlock(info_magic.into()));
        }
        let source_format = TexFormat::try_from(read_u32_le(data, &mut offset)?)?;
        let flags = read_u32_le(data, &mut offset)?;
        if flags & 0x40 == 0 {
            return Err(invalid("Expected a volume texture (flag 0x40)"));
        }
        if flags & (4 | 32) != 0 {
            return Err(invalid(
                "Animated or video volume textures are not supported",
            ));
        }
        if source_format != TexFormat::ARGB8888 {
            return Err(invalid(format!(
                "Unsupported volume pixel format {source_format:?}; expected ARGB8888"
            )));
        }
        let texture_width = read_u32_le(data, &mut offset)?;
        let texture_height = read_u32_le(data, &mut offset)?;
        let width = read_u32_le(data, &mut offset)?;
        let height = read_u32_le(data, &mut offset)?;
        let depth = read_u32_le(data, &mut offset)?;
        let info_extra = read_u32_le(data, &mut offset)?;
        volume_size(texture_width, texture_height, depth)?;
        if !(1..=MAX_TEXTURE_DIMENSION).contains(&width)
            || !(1..=MAX_TEXTURE_DIMENSION).contains(&height)
        {
            return Err(invalid(
                "Volume source dimensions exceed the supported range",
            ));
        }

        let container_magic = read_magic(data, &mut offset)?;
        let container_version = match container_magic {
            "TEXB0001" => 1,
            "TEXB0002" => 2,
            "TEXB0003" => 3,
            "TEXB0004" => 4,
            other => return Err(TexError::BadContainerBlock(other.into())),
        };
        let image_count = read_u32_le(data, &mut offset)?;
        if image_count != 1 {
            return Err(invalid(format!(
                "Expected one volume image, got {image_count}"
            )));
        }
        let free_image_format = if container_version >= 3 {
            Some(FreeImageFormat::from(read_u32_le(data, &mut offset)?))
        } else {
            None
        };
        if !matches!(
            free_image_format,
            None | Some(FreeImageFormat::Unknown | FreeImageFormat::Png)
        ) {
            return Err(invalid(format!(
                "Unsupported embedded volume codec {free_image_format:?}"
            )));
        }
        if container_version == 4 && read_u32_le(data, &mut offset)? != 0 {
            return Err(invalid("Video TEXB0004 volumes are not supported"));
        }
        let mip_count = read_u32_le(data, &mut offset)?;
        let complete_levels = texture_width.max(texture_height).max(depth).ilog2() + 1;
        if !(1..=MAX_VOLUME_MIPS.min(complete_levels)).contains(&mip_count) {
            return Err(invalid(format!("Invalid volume mip count {mip_count}")));
        }
        let is_png = free_image_format == Some(FreeImageFormat::Png);
        let mut mipmaps = Vec::new();
        let mut decoded_bytes = 0_u64;
        let mut expected_dimensions = [texture_width, texture_height, depth];
        for level in 0..mip_count {
            let dimensions = [
                read_u32_le(data, &mut offset)?,
                read_u32_le(data, &mut offset)?,
                read_u32_le(data, &mut offset)?,
            ];
            let [mip_w, mip_h, mip_d] = dimensions;
            let size = volume_size(mip_w, mip_h, mip_d)?;
            if dimensions != expected_dimensions {
                return Err(invalid(format!(
                    "Volume mip {level} dimensions {dimensions:?} do not match {expected_dimensions:?}"
                )));
            }
            decoded_bytes = decoded_bytes
                .checked_add(size)
                .ok_or(TexError::MipmapTooLarge {
                    size: u64::MAX,
                    max: max_decoded_bytes,
                })?;
            if decoded_bytes > max_decoded_bytes {
                return Err(TexError::MipmapTooLarge {
                    size: decoded_bytes,
                    max: max_decoded_bytes,
                });
            }
            let (compression, uncompressed_size) = if container_version >= 2 {
                (
                    read_u32_le(data, &mut offset)?,
                    read_i32_le(data, &mut offset)?,
                )
            } else {
                (0, 0)
            };
            if compression > 1 {
                return Err(invalid(format!(
                    "Unsupported volume compression {compression}"
                )));
            }
            let encoded_size = u64::from(read_u32_le(data, &mut offset)?);
            let payload_size = if compression == 1 {
                if uncompressed_size <= 0 {
                    return Err(invalid("Volume LZ4 size must be positive"));
                }
                uncompressed_size as u64
            } else {
                encoded_size
            };
            for size in [encoded_size, payload_size] {
                if size == 0 || size > MAX_MIPMAP_SIZE {
                    return Err(invalid("Volume encoded payload size is outside 1..=64 MiB"));
                }
            }
            if !is_png && payload_size != size {
                return Err(TexError::InvalidMipmapDataSize {
                    level: level as usize,
                    expected: size,
                    actual: payload_size as usize,
                });
            }
            if compression == 1 && payload_size > encoded_size.saturating_mul(MAX_COMPRESSION_RATIO)
            {
                return Err(TexError::CompressionRatioExceeded {
                    compressed: encoded_size,
                    uncompressed: payload_size,
                });
            }
            let encoded = get_slice(data, &mut offset, encoded_size as usize)?;
            let decompressed;
            let payload = if compression == 1 {
                decompressed = {
                    let mut bytes = vec![0; payload_size as usize];
                    let decoded = lz4_flex::decompress_into(encoded, &mut bytes)
                        .map_err(|error| TexError::Lz4Error(error.to_string()))?;
                    if decoded != bytes.len() {
                        return Err(invalid(format!(
                            "Volume LZ4 decoded {decoded} bytes, expected {}",
                            bytes.len()
                        )));
                    }
                    bytes
                };
                decompressed.as_slice()
            } else {
                encoded
            };
            let pixels: Arc<[u8]> = if is_png {
                // Native decoder receives width and height * depth. No slice
                // transposition or per-slice inversion is applied here.
                Arc::from(decode_png_to_rgba8(payload, mip_w, mip_h * mip_d)?)
            } else {
                Arc::from(payload)
            };
            mipmaps.push(TexVolumeMip {
                width: mip_w,
                height: mip_h,
                depth: mip_d,
                data: pixels,
            });
            expected_dimensions = dimensions.map(|value| (value / 2).max(1));
        }
        if offset != data.len() {
            return Err(invalid("Unexpected trailing data after volume mip chain"));
        }
        Ok(Self {
            format: if is_png {
                TexFormat::RGBA8888
            } else {
                source_format
            },
            flags,
            width,
            height,
            texture_width,
            texture_height,
            depth,
            info_extra,
            container_version,
            free_image_format,
            mipmaps,
        })
    }
}
