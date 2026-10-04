use std::{io::Cursor, sync::Arc};

use crate::error::TexError;

// ── Limits ──────────────────────────────────────────────────────────────────

/// Maximum texture dimension (width or height) in pixels
pub const MAX_TEXTURE_DIMENSION: u32 = 16_384;

/// Maximum mipmap data size. This matches the package single-entry ceiling.
pub const MAX_MIPMAP_SIZE: u64 = 64 * 1024 * 1024;

/// Maximum compression ratio allowed. Real single-channel masks commonly
/// exceed 200:1, while the exact decoded size remains bounded above.
pub const MAX_COMPRESSION_RATIO: u64 = 512;

/// Maximum number of frames in an animated texture
pub const MAX_ANIMATION_FRAMES: u32 = 10_000;

// ── Data model ──────────────────────────────────────────────────────────────

/// Pixel format of a Wallpaper Engine texture
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum TexFormat {
    ARGB8888 = 0,
    RGB888 = 1,
    RGB565 = 2,
    DXT5 = 4,
    DXT3 = 6,
    DXT1 = 7,
    RG88 = 8,
    R8 = 9,
    RG1616f = 10,
    R16f = 11,
    BC7 = 12,
    RGBa1010102 = 13,
    RGBA16161616f = 14,
    RGB161616f = 15,
    /// Canonical RGBA8 produced after decoding an embedded image.
    /// This value is internal and is never accepted from a TEXI format id.
    RGBA8888 = 16,
}

impl TryFrom<u32> for TexFormat {
    type Error = TexError;

    fn try_from(v: u32) -> Result<Self, Self::Error> {
        match v {
            0 => Ok(TexFormat::ARGB8888),
            1 => Ok(TexFormat::RGB888),
            2 => Ok(TexFormat::RGB565),
            4 => Ok(TexFormat::DXT5),
            6 => Ok(TexFormat::DXT3),
            7 => Ok(TexFormat::DXT1),
            8 => Ok(TexFormat::RG88),
            9 => Ok(TexFormat::R8),
            10 => Ok(TexFormat::RG1616f),
            11 => Ok(TexFormat::R16f),
            12 => Ok(TexFormat::BC7),
            13 => Ok(TexFormat::RGBa1010102),
            14 => Ok(TexFormat::RGBA16161616f),
            15 => Ok(TexFormat::RGB161616f),
            _ => Err(TexError::UnknownFormat(v)),
        }
    }
}

/// Embedded image format inside a TEXB0003/0004 container
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FreeImageFormat {
    Unknown,
    Png,
    Jpeg,
    Dds,
    Gif,
    Webp,
    Bmp,
    Targa,
    Tiff,
    Mp4,
    Other(u32),
}

const FIF_UNKNOWN: u32 = u32::MAX; // -1 as u32

impl From<u32> for FreeImageFormat {
    fn from(v: u32) -> Self {
        match v {
            FIF_UNKNOWN => FreeImageFormat::Unknown,
            13 => FreeImageFormat::Png,
            2 => FreeImageFormat::Jpeg,
            24 => FreeImageFormat::Dds,
            25 => FreeImageFormat::Gif,
            35 => FreeImageFormat::Webp,
            0 => FreeImageFormat::Bmp,
            17 => FreeImageFormat::Targa,
            18 => FreeImageFormat::Tiff,
            other => FreeImageFormat::Other(other),
        }
    }
}

/// A single mipmap level within a texture image
#[derive(Debug, Clone)]
pub struct Mipmap {
    pub width: u32,
    pub height: u32,
    pub data: Arc<[u8]>,
    pub json: Option<String>,
}

/// An animation frame in a spritesheet or GIF texture
#[derive(Debug, Clone)]
pub struct AnimationFrame {
    pub frame_number: u32,
    pub frametime: f32,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// A parsed .tex texture file
#[derive(Debug, Clone)]
pub struct TexTexture {
    pub format: TexFormat,
    pub width: u32,
    pub height: u32,
    pub texture_width: u32,
    pub texture_height: u32,
    pub flags: u32,
    pub mipmaps: Vec<Mipmap>,
    pub frames: Vec<AnimationFrame>,
    pub is_animated: bool,
    pub container_version: u32,
    pub free_image_format: Option<FreeImageFormat>,
    pub is_video: bool,
    pub spritesheet_cols: u32,
    pub spritesheet_rows: u32,
    pub spritesheet_frames: u32,
    pub spritesheet_duration: f32,
}

/// Color transfer function carried into the renderer without relying on GPU defaults.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum TextureColorSpace {
    Srgb,
    Linear,
    Unknown,
}

/// Alpha interpretation used when creating a renderer texture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum TextureAlphaMode {
    Opaque,
    Straight,
    Unknown,
}

/// A validated mip level. Block-compressed formats remain compressed for direct GPU upload.
#[derive(Debug, Clone)]
pub struct TextureMipLevel {
    pub width: u32,
    pub height: u32,
    pub data: Arc<[u8]>,
}

/// Version-independent texture payload consumed by the renderer/runtime boundary.
#[derive(Debug, Clone)]
pub struct TextureImage {
    pub format: TexFormat,
    pub color_space: TextureColorSpace,
    pub alpha_mode: TextureAlphaMode,
    pub levels: Vec<TextureMipLevel>,
}

impl TexTexture {
    /// Whether this is BCn/DXT compressed (not raw pixel data)
    pub fn is_compressed_format(&self) -> bool {
        matches!(
            self.format,
            TexFormat::DXT1 | TexFormat::DXT3 | TexFormat::DXT5 | TexFormat::BC7
        )
    }

    /// Bytes per pixel for uncompressed formats
    pub fn bytes_per_pixel(&self) -> Option<u32> {
        match self.format {
            TexFormat::ARGB8888 => Some(4),
            TexFormat::RGBA8888 => Some(4),
            TexFormat::RGB888 => Some(3),
            TexFormat::RGB565 => Some(2),
            TexFormat::RG88 => Some(2),
            TexFormat::R8 => Some(1),
            TexFormat::RGBA16161616f => Some(8),
            TexFormat::RGB161616f => Some(6),
            TexFormat::RG1616f => Some(4),
            TexFormat::R16f => Some(2),
            TexFormat::RGBa1010102 => Some(4),
            _ => None, // compressed
        }
    }

    /// Convert container-specific mipmaps into a size-validated renderer input.
    pub fn to_texture_image(&self) -> Result<TextureImage, TexError> {
        if self.mipmaps.is_empty() {
            return Err(TexError::MissingMipmaps);
        }

        let mut levels = Vec::with_capacity(self.mipmaps.len());
        for (level, mipmap) in self.mipmaps.iter().enumerate() {
            if mipmap.width == 0
                || mipmap.height == 0
                || mipmap.width > MAX_TEXTURE_DIMENSION
                || mipmap.height > MAX_TEXTURE_DIMENSION
            {
                return Err(TexError::InvalidDimensions {
                    level,
                    width: mipmap.width,
                    height: mipmap.height,
                });
            }
            let expected = expected_mipmap_size(self.format, mipmap.width, mipmap.height)?;
            let divisor = 1_u32.checked_shl(level as u32).unwrap_or(u32::MAX);
            let storage_width = self.texture_width.div_ceil(divisor).max(1);
            let storage_height = self.texture_height.div_ceil(divisor).max(1);
            let storage_expected =
                expected_mipmap_size(self.format, storage_width, storage_height)?;
            let (upload_width, upload_height) = if mipmap.data.len() as u64 == expected {
                (mipmap.width, mipmap.height)
            } else if mipmap.data.len() as u64 == storage_expected {
                (storage_width, storage_height)
            } else {
                return Err(TexError::InvalidMipmapDataSize {
                    level,
                    expected,
                    actual: mipmap.data.len(),
                });
            };
            levels.push(TextureMipLevel {
                width: upload_width,
                height: upload_height,
                data: Arc::clone(&mipmap.data),
            });
        }

        Ok(TextureImage {
            format: self.format,
            color_space: color_space(self.format),
            alpha_mode: alpha_mode(self.format),
            levels,
        })
    }
}

fn expected_mipmap_size(format: TexFormat, width: u32, height: u32) -> Result<u64, TexError> {
    let size = match format {
        TexFormat::DXT1 => block_compressed_size(width, height, 8),
        TexFormat::DXT3 | TexFormat::DXT5 | TexFormat::BC7 => {
            block_compressed_size(width, height, 16)
        }
        _ => {
            let bytes_per_pixel = match format {
                TexFormat::ARGB8888 | TexFormat::RGBA8888 | TexFormat::RGBa1010102 => 4,
                TexFormat::RGB888 => 3,
                TexFormat::RGB565 | TexFormat::RG88 | TexFormat::R16f => 2,
                TexFormat::R8 => 1,
                TexFormat::RG1616f => 4,
                TexFormat::RGBA16161616f => 8,
                TexFormat::RGB161616f => 6,
                _ => unreachable!("compressed formats handled above"),
            };
            u64::from(width)
                .checked_mul(u64::from(height))
                .and_then(|pixels| pixels.checked_mul(bytes_per_pixel))
                .ok_or(TexError::MipmapTooLarge {
                    size: u64::MAX,
                    max: MAX_MIPMAP_SIZE,
                })?
        }
    };
    if size > MAX_MIPMAP_SIZE {
        return Err(TexError::MipmapTooLarge {
            size,
            max: MAX_MIPMAP_SIZE,
        });
    }
    Ok(size)
}

fn block_compressed_size(width: u32, height: u32, bytes_per_block: u64) -> u64 {
    let blocks_w = u64::from(width).div_ceil(4);
    let blocks_h = u64::from(height).div_ceil(4);
    blocks_w
        .saturating_mul(blocks_h)
        .saturating_mul(bytes_per_block)
}

fn color_space(format: TexFormat) -> TextureColorSpace {
    let _ = format;
    // Pixel storage alone does not prove whether authored values use an sRGB transfer function.
    TextureColorSpace::Unknown
}

/// Returns true when `v` matches a known FreeImage format identifier used in
/// Wallpaper Engine TEXB0003/0004 containers.
fn is_known_freeimage_format_u32(v: u32) -> bool {
    matches!(v, 0 | 2 | 13 | 17 | 18 | 24 | 25 | 35)
}

fn alpha_mode(format: TexFormat) -> TextureAlphaMode {
    match format {
        TexFormat::RGB888
        | TexFormat::RGB565
        | TexFormat::RG88
        | TexFormat::R8
        | TexFormat::RG1616f
        | TexFormat::R16f
        | TexFormat::RGB161616f => TextureAlphaMode::Opaque,
        TexFormat::ARGB8888
        | TexFormat::RGBA8888
        | TexFormat::DXT1
        | TexFormat::DXT3
        | TexFormat::DXT5
        | TexFormat::BC7
        | TexFormat::RGBa1010102
        | TexFormat::RGBA16161616f => TextureAlphaMode::Unknown,
    }
}

// ── Parser ──────────────────────────────────────────────────────────────────

fn validated_rgba_size(width: u32, height: u32) -> Result<usize, TexError> {
    let size = expected_mipmap_size(TexFormat::RGBA8888, width, height)?;
    usize::try_from(size).map_err(|_| TexError::MipmapTooLarge {
        size,
        max: MAX_MIPMAP_SIZE,
    })
}

/// Decode a PNG byte buffer into canonical RGBA8 pixels, validating dimensions.
fn decode_png_to_rgba8(data: &[u8], expected_w: u32, expected_h: u32) -> Result<Vec<u8>, TexError> {
    let expected_size = validated_rgba_size(expected_w, expected_h)?;
    let mut decoder = png::Decoder::new(data);
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder
        .read_info()
        .map_err(|e| TexError::InvalidData(format!("PNG decode: {e}")))?;
    let info = reader.info();
    if info.width != expected_w || info.height != expected_h {
        return Err(TexError::InvalidMipmapDataSize {
            level: 0,
            expected: (expected_w as u64) * (expected_h as u64) * 4,
            actual: (info.width as usize) * (info.height as usize) * 4,
        });
    }
    let mut decoded = vec![0_u8; reader.output_buffer_size()];
    let output = reader
        .next_frame(&mut decoded)
        .map_err(|e| TexError::InvalidData(format!("PNG frame: {e}")))?;
    let decoded = &decoded[..output.buffer_size()];
    let mut rgba = Vec::with_capacity(expected_size);
    match output.color_type {
        png::ColorType::Rgba => rgba.extend_from_slice(decoded),
        png::ColorType::Rgb => {
            for pixel in decoded.chunks_exact(3) {
                rgba.extend_from_slice(&[pixel[0], pixel[1], pixel[2], 255]);
            }
        }
        png::ColorType::Grayscale => {
            for &value in decoded {
                rgba.extend_from_slice(&[value, value, value, 255]);
            }
        }
        png::ColorType::GrayscaleAlpha => {
            for pixel in decoded.chunks_exact(2) {
                rgba.extend_from_slice(&[pixel[0], pixel[0], pixel[0], pixel[1]]);
            }
        }
        png::ColorType::Indexed => {
            return Err(TexError::InvalidData(
                "PNG palette was not expanded by the decoder".into(),
            ));
        }
    }
    if rgba.len() != expected_size {
        return Err(TexError::InvalidMipmapDataSize {
            level: 0,
            expected: expected_size as u64,
            actual: rgba.len(),
        });
    }
    Ok(rgba)
}

/// Decode a JPEG byte buffer into canonical RGBA8 pixels, validating dimensions.
fn decode_jpeg_to_rgba8(
    data: &[u8],
    expected_w: u32,
    expected_h: u32,
) -> Result<Vec<u8>, TexError> {
    let expected_size = validated_rgba_size(expected_w, expected_h)?;
    let mut decoder = jpeg_decoder::Decoder::new(Cursor::new(data));
    decoder
        .read_info()
        .map_err(|e| TexError::InvalidData(format!("JPEG header: {e}")))?;
    let info = decoder
        .info()
        .ok_or_else(|| TexError::InvalidData("JPEG is missing image metadata".into()))?;
    if u32::from(info.width) != expected_w || u32::from(info.height) != expected_h {
        return Err(TexError::InvalidMipmapDataSize {
            level: 0,
            expected: expected_size as u64,
            actual: usize::from(info.width) * usize::from(info.height) * 4,
        });
    }
    let decoded = decoder
        .decode()
        .map_err(|e| TexError::InvalidData(format!("JPEG decode: {e}")))?;
    let mut rgba = Vec::with_capacity(expected_size);
    match info.pixel_format {
        jpeg_decoder::PixelFormat::L8 => {
            for value in decoded {
                rgba.extend_from_slice(&[value, value, value, 255]);
            }
        }
        jpeg_decoder::PixelFormat::L16 => {
            for pixel in decoded.chunks_exact(2) {
                let value = pixel[0];
                rgba.extend_from_slice(&[value, value, value, 255]);
            }
        }
        jpeg_decoder::PixelFormat::RGB24 => {
            for pixel in decoded.chunks_exact(3) {
                rgba.extend_from_slice(&[pixel[0], pixel[1], pixel[2], 255]);
            }
        }
        jpeg_decoder::PixelFormat::CMYK32 => {
            for pixel in decoded.chunks_exact(4) {
                let c = u16::from(pixel[0]);
                let m = u16::from(pixel[1]);
                let y = u16::from(pixel[2]);
                let k = u16::from(pixel[3]);
                rgba.extend_from_slice(&[
                    (255 - (c + k).min(255)) as u8,
                    (255 - (m + k).min(255)) as u8,
                    (255 - (y + k).min(255)) as u8,
                    255,
                ]);
            }
        }
    }
    if rgba.len() != expected_size {
        return Err(TexError::InvalidMipmapDataSize {
            level: 0,
            expected: expected_size as u64,
            actual: rgba.len(),
        });
    }
    Ok(rgba)
}

fn decode_embedded_image(
    image_format: Option<FreeImageFormat>,
    encoded: &[u8],
    width: u32,
    height: u32,
) -> Result<Option<Vec<u8>>, TexError> {
    match image_format {
        Some(FreeImageFormat::Png) if encoded.starts_with(b"\x89PNG\r\n\x1a\n") => {
            decode_png_to_rgba8(encoded, width, height).map(Some)
        }
        Some(FreeImageFormat::Jpeg) if encoded.starts_with(&[0xff, 0xd8, 0xff]) => {
            decode_jpeg_to_rgba8(encoded, width, height).map(Some)
        }
        _ => Ok(None),
    }
}

impl TexTexture {
    /// Parse a .tex texture from raw bytes
    pub fn parse(data: &[u8]) -> Result<Self, TexError> {
        let len = data.len();
        let mut offset: usize = 0;

        // Read 9-byte magic: TEXV0005
        let magic = read_magic(data, &mut offset)?;
        if magic != "TEXV0005" {
            return Err(TexError::BadMagic(magic.to_string()));
        }

        // TEXI0001 info block
        let info_magic = read_magic(data, &mut offset)?;
        if info_magic != "TEXI0001" {
            return Err(TexError::BadInfoBlock(info_magic.to_string()));
        }

        let format_raw = read_u32_le(data, &mut offset)?;
        let mut format = TexFormat::try_from(format_raw)?;
        let flags = read_u32_le(data, &mut offset)?;
        let texture_width = read_u32_le(data, &mut offset)?;
        let texture_height = read_u32_le(data, &mut offset)?;
        let width = read_u32_le(data, &mut offset)?;
        let height = read_u32_le(data, &mut offset)?;

        if width > MAX_TEXTURE_DIMENSION || height > MAX_TEXTURE_DIMENSION {
            return Err(TexError::MipmapTooLarge {
                size: width.max(height) as u64,
                max: MAX_TEXTURE_DIMENSION as u64,
            });
        }

        // Skip one uint32 (padding)
        read_u32_le(data, &mut offset)?;

        let is_animated = (flags & 4) == 4; // TextureFlags_IsGif
        let is_video = (flags & 32) == 32;

        // Container block: TEXB0001/0002/0003/0004
        let container_magic = read_magic(data, &mut offset)?;
        let container_version = match container_magic {
            "TEXB0001" => 1u32,
            "TEXB0002" => 2,
            "TEXB0003" => 3,
            "TEXB0004" => 4,
            other => {
                return Err(TexError::BadContainerBlock(other.to_string()));
            }
        };

        let mut free_image_format = None;
        let mut is_video_texb4 = false;
        let mut image_count_override = None;

        if container_version == 4 {
            let image_count = read_u32_le(data, &mut offset)?;
            let fif_raw = read_u32_le(data, &mut offset)?;
            is_video_texb4 = read_u32_le(data, &mut offset)? == 1;
            image_count_override = Some(image_count);
            free_image_format = if is_video_texb4 {
                Some(FreeImageFormat::Mp4)
            } else {
                Some(FreeImageFormat::from(fif_raw))
            };
        }

        if container_version == 3 {
            // TEXB0003 has two layout variants in the wild:
            //   A) TEXB0003 freeimage_fmt image_count …  (our test fixtures)
            //   B) TEXB0003 image_count freeimage_fmt …  (many real scenes)
            // Heuristic: peek ahead four bytes. A known FreeImage format value
            // (0,2,13,17,18,24,25,35) signals variant A; a small plausible
            // image count like 1 signals variant B.
            let next = read_u32_le(data, &mut offset)?;
            let (fif_raw, is_variant_b) = if is_known_freeimage_format_u32(next) {
                (next, false)
            } else {
                let fif = read_u32_le(data, &mut offset)?;
                (fif, true)
            };
            free_image_format = Some(FreeImageFormat::from(fif_raw));
            // For variant B, `next` was the image count — skip the
            // standard image_count read below.
            if is_variant_b {
                image_count_override = Some(next);
            }
        }

        // Read mipmap count — stored differently per version
        // For TEXB0002/0003/0004: the count is the number of images,
        // each image has its own mipmap count
        let mut mipmaps = Vec::new();
        let image_count = if let Some(count) = image_count_override {
            count
        } else if container_version >= 2 {
            read_u32_le(data, &mut offset)?
        } else {
            1
        };

        let mut frames = Vec::new();
        let mut spritesheet_cols = 0u32;
        let mut spritesheet_rows = 0u32;
        let mut _spritesheet_frames = 0u32;
        let mut spritesheet_duration = 0f32;

        for _image_idx in 0..image_count {
            let mip_count = read_u32_le(data, &mut offset)?;

            for _mip_idx in 0..mip_count {
                // Video-flavoured TEXB0004 entries carry editor metadata before
                // the payload. Non-video TEXB0004 FreeImage mip chains use the
                // same per-mip layout as TEXB0003.
                if container_version == 4 && is_video_texb4 {
                    // skip 3 uint32 fields (editor metadata)
                    read_u32_le(data, &mut offset)?;
                    read_u32_le(data, &mut offset)?;
                    // json string
                    let _json = read_null_terminated(data, &mut offset);
                    read_u32_le(data, &mut offset)?;
                }

                let mip_w = read_u32_le(data, &mut offset)?;
                let mip_h = read_u32_le(data, &mut offset)?;

                let (compression, uncompressed_size) = if container_version >= 2 {
                    let comp = read_u32_le(data, &mut offset)?;
                    let uncomp = read_i32_le(data, &mut offset)?;
                    (comp, uncomp)
                } else {
                    (0u32, 0i32)
                };

                let compressed_size = read_u32_le(data, &mut offset)?;

                let actual_uncompressed = if compression == 0 {
                    compressed_size as i32
                } else {
                    uncompressed_size
                };

                if actual_uncompressed <= 0 {
                    continue;
                }

                let uncomp_u64 = actual_uncompressed as u64;

                // Safety: check size limits
                if uncomp_u64 > MAX_MIPMAP_SIZE {
                    return Err(TexError::MipmapTooLarge {
                        size: uncomp_u64,
                        max: MAX_MIPMAP_SIZE,
                    });
                }

                let encoded = if compression == 1 {
                    // LZ4 compressed
                    let comp_size = compressed_size as usize;
                    let comp_bytes = get_slice(data, &mut offset, comp_size)?;

                    let maximum_uncompressed =
                        (comp_size as u64).saturating_mul(MAX_COMPRESSION_RATIO);
                    if uncomp_u64 > maximum_uncompressed {
                        return Err(TexError::CompressionRatioExceeded {
                            compressed: comp_size as u64,
                            uncompressed: uncomp_u64,
                        });
                    }

                    let mut decomp_buf = vec![0u8; actual_uncompressed as usize];
                    let result = lz4_flex::decompress_into(comp_bytes, &mut decomp_buf);
                    match result {
                        Ok(_) => decomp_buf,
                        Err(e) => {
                            return Err(TexError::Lz4Error(e.to_string()));
                        }
                    }
                } else {
                    let raw = get_slice(data, &mut offset, actual_uncompressed as usize)?;
                    raw.to_vec()
                };
                // FreeImage payloads may themselves be LZ4-compressed. Decode
                // every concatenated mip image instead of returning after the
                // first payload, preserving the complete GPU mip chain.
                let data_slice: Arc<[u8]> = if let Some(decoded) =
                    decode_embedded_image(free_image_format, &encoded, mip_w, mip_h)?
                {
                    format = TexFormat::RGBA8888;
                    Arc::from(decoded)
                } else {
                    Arc::from(encoded)
                };

                mipmaps.push(Mipmap {
                    width: mip_w,
                    height: mip_h,
                    data: data_slice,
                    json: None,
                });
            }
        }

        // Optional animation frames
        if is_animated && offset + 9 <= len {
            // Try to read TEXS block
            let anim_magic = std::str::from_utf8(&data[offset..offset + 9]).unwrap_or("");
            if anim_magic.starts_with("TEXS") {
                offset += 9;
                let frame_count = read_u32_le(data, &mut offset)?;

                let anim_version = anim_magic;
                if anim_version == "TEXS0003" {
                    // read gif dimensions
                    _spritesheet_frames = frame_count;
                    read_u32_le(data, &mut offset)?; // gif_width
                    read_u32_le(data, &mut offset)?; // gif_height
                }

                for _ in 0..frame_count.min(MAX_ANIMATION_FRAMES) {
                    let frame: AnimationFrame;
                    if anim_version == "TEXS0001" {
                        frame = AnimationFrame {
                            frame_number: read_u32_le(data, &mut offset)?,
                            frametime: read_f32_le(data, &mut offset)?,
                            x: read_u32_le(data, &mut offset)? as f32,
                            y: read_u32_le(data, &mut offset)? as f32,
                            width: read_u32_le(data, &mut offset)? as f32,
                            height: read_u32_le(data, &mut offset)? as f32,
                        };
                        // skip unknown fields
                        read_u32_le(data, &mut offset)?;
                        read_u32_le(data, &mut offset)?;
                    } else {
                        frame = AnimationFrame {
                            frame_number: read_u32_le(data, &mut offset)?,
                            frametime: read_f32_le(data, &mut offset)?,
                            x: read_f32_le(data, &mut offset)?,
                            y: read_f32_le(data, &mut offset)?,
                            width: read_f32_le(data, &mut offset)?,
                            height: read_f32_le(data, &mut offset)?,
                        };
                        // skip additional fields
                        read_f32_le(data, &mut offset)?;
                        read_f32_le(data, &mut offset)?;
                    };
                    frames.push(frame);
                }

                // Calculate spritesheet grid
                if !frames.is_empty() && width > 0 && height > 0 {
                    let fw = frames[0].width;
                    let fh = frames[0].height;
                    if fw > 0.0 && fh > 0.0 {
                        spritesheet_cols = (width as f32 / fw).round() as u32;
                        spritesheet_rows = (height as f32 / fh).round() as u32;
                        if spritesheet_cols > 0
                            && spritesheet_rows > 0
                            && spritesheet_cols.saturating_mul(spritesheet_rows) >= frame_count
                        {
                            spritesheet_duration = frames.iter().map(|f| f.frametime).sum();
                        }
                    }
                }
            }
        }

        Ok(TexTexture {
            format,
            width,
            height,
            texture_width,
            texture_height,
            flags,
            mipmaps,
            frames: frames.clone(),
            is_animated,
            container_version,
            free_image_format,
            is_video: is_video || is_video_texb4 || free_image_format == Some(FreeImageFormat::Mp4),
            spritesheet_cols,
            spritesheet_rows,
            spritesheet_frames: frames.len() as u32,
            spritesheet_duration,
        })
    }
}

// ── Binary reading helpers ──────────────────────────────────────────────────

fn read_magic<'a>(data: &'a [u8], offset: &mut usize) -> Result<&'a str, TexError> {
    let end_offset = offset.checked_add(9).ok_or(TexError::UnexpectedEof)?;
    if end_offset > data.len() {
        return Err(TexError::UnexpectedEof);
    }
    // magic includes a trailing NUL that we strip
    let end = data[*offset..*offset + 9]
        .iter()
        .position(|b| *b == 0)
        .unwrap_or(9);
    let s = std::str::from_utf8(&data[*offset..*offset + end])
        .map_err(|_| TexError::BadMagic("<invalid utf8>".into()))?;
    *offset += 9;
    Ok(s)
}

fn read_u32_le(data: &[u8], offset: &mut usize) -> Result<u32, TexError> {
    let end = offset.checked_add(4).ok_or(TexError::UnexpectedEof)?;
    if end > data.len() {
        return Err(TexError::UnexpectedEof);
    }
    let b = &data[*offset..end];
    *offset = end;
    Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

fn read_i32_le(data: &[u8], offset: &mut usize) -> Result<i32, TexError> {
    let end = offset.checked_add(4).ok_or(TexError::UnexpectedEof)?;
    if end > data.len() {
        return Err(TexError::UnexpectedEof);
    }
    let b = &data[*offset..end];
    *offset = end;
    Ok(i32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

fn read_f32_le(data: &[u8], offset: &mut usize) -> Result<f32, TexError> {
    let end = offset.checked_add(4).ok_or(TexError::UnexpectedEof)?;
    if end > data.len() {
        return Err(TexError::UnexpectedEof);
    }
    let b = &data[*offset..end];
    *offset = end;
    Ok(f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

fn read_null_terminated<'a>(data: &'a [u8], offset: &mut usize) -> &'a str {
    let start = *offset;
    let end = data[start..]
        .iter()
        .position(|b| *b == 0)
        .unwrap_or(data.len() - start);
    *offset += end + 1; // +1 for NUL
    std::str::from_utf8(&data[start..start + end]).unwrap_or("")
}

fn get_slice<'a>(data: &'a [u8], offset: &mut usize, size: usize) -> Result<&'a [u8], TexError> {
    let end = offset.checked_add(size).ok_or(TexError::UnexpectedEof)?;
    if end > data.len() {
        return Err(TexError::UnexpectedEof);
    }
    let slice = &data[*offset..end];
    *offset = end;
    Ok(slice)
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn rgb_png(width: u32, height: u32, pixels: &[u8]) -> Vec<u8> {
        let mut encoded = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut encoded, width, height);
            encoder.set_color(png::ColorType::Rgb);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().unwrap();
            writer.write_image_data(pixels).unwrap();
        }
        encoded
    }

    fn build_embedded_png_tex(container_version: u32) -> Vec<u8> {
        let mips = [
            (2_u32, 1_u32, rgb_png(2, 1, &[10, 20, 30, 40, 50, 60])),
            (1_u32, 1_u32, rgb_png(1, 1, &[70, 80, 90])),
        ];
        let mut buf = b"TEXV0005\0TEXI0001\0".to_vec();
        buf.extend(0_u32.to_le_bytes());
        buf.extend(2_u32.to_le_bytes());
        for value in [2_u32, 1, 2, 1, 0] {
            buf.extend(value.to_le_bytes());
        }
        match container_version {
            3 => {
                buf.extend(b"TEXB0003\0");
                // Observed layout: image count precedes the FreeImage format.
                buf.extend(1_u32.to_le_bytes());
                buf.extend(13_u32.to_le_bytes());
            }
            4 => {
                buf.extend(b"TEXB0004\0");
                buf.extend(1_u32.to_le_bytes());
                buf.extend(13_u32.to_le_bytes());
                buf.extend(0_u32.to_le_bytes());
            }
            _ => unreachable!(),
        }
        buf.extend((mips.len() as u32).to_le_bytes());
        for (width, height, png) in mips {
            buf.extend(width.to_le_bytes());
            buf.extend(height.to_le_bytes());
            buf.extend(0_u32.to_le_bytes());
            buf.extend(0_i32.to_le_bytes());
            buf.extend((png.len() as u32).to_le_bytes());
            buf.extend(png);
        }
        buf
    }

    fn build_tex_rgba8(width: u32, height: u32) -> Vec<u8> {
        let mut buf = Vec::new();
        // TEXV0005
        buf.extend(b"TEXV0005\0");
        // TEXI0001
        buf.extend(b"TEXI0001\0");
        buf.extend(0u32.to_le_bytes()); // ARGB8888
        buf.extend(0u32.to_le_bytes()); // flags
        buf.extend(width.to_le_bytes()); // texture_width
        buf.extend(height.to_le_bytes()); // texture_height
        buf.extend(width.to_le_bytes()); // width
        buf.extend(height.to_le_bytes()); // height
        buf.extend(0u32.to_le_bytes()); // padding
        // TEXB0003 container
        buf.extend(b"TEXB0003\0");
        buf.extend(13u32.to_le_bytes()); // FIF_PNG = 13
        // image count = 1
        buf.extend(1u32.to_le_bytes());
        // mip count per image = 1
        buf.extend(1u32.to_le_bytes());
        // mipmap: width, height
        buf.extend(width.to_le_bytes());
        buf.extend(height.to_le_bytes());
        // compression = 0, uncompressed_size = 0
        buf.extend(0u32.to_le_bytes());
        buf.extend(0i32.to_le_bytes());
        // data size
        let pixel_count = width as u64 * height as u64 * 4;
        buf.extend((pixel_count as u32).to_le_bytes());
        // dummy pixel data
        let dummy: Vec<u8> = (0..pixel_count).map(|i| i as u8).collect();
        buf.extend(dummy);
        buf
    }

    #[test]
    fn parse_simple_rgba8_tex() {
        let data = build_tex_rgba8(64, 64);
        let tex = TexTexture::parse(&data).unwrap();
        assert_eq!(tex.format, TexFormat::ARGB8888);
        assert_eq!(tex.width, 64);
        assert_eq!(tex.height, 64);
        assert_eq!(tex.mipmaps.len(), 1);
    }

    #[test]
    fn decodes_every_rgb_png_mip_in_texb3_and_texb4() {
        for container_version in [3, 4] {
            let tex = TexTexture::parse(&build_embedded_png_tex(container_version)).unwrap();
            assert_eq!(tex.container_version, container_version);
            assert_eq!(tex.format, TexFormat::RGBA8888);
            assert_eq!(tex.free_image_format, Some(FreeImageFormat::Png));
            assert_eq!(tex.mipmaps.len(), 2);
            assert_eq!(
                tex.mipmaps[0].data.as_ref(),
                &[10, 20, 30, 255, 40, 50, 60, 255]
            );
            assert_eq!(tex.mipmaps[1].data.as_ref(), &[70, 80, 90, 255]);
            let image = tex.to_texture_image().unwrap();
            assert_eq!((image.levels[0].width, image.levels[0].height), (2, 1));
            assert_eq!((image.levels[1].width, image.levels[1].height), (1, 1));
        }
    }

    #[test]
    fn reject_bad_magic() {
        let err = TexTexture::parse(b"XXXX0005\0TEXI0001\0...").unwrap_err();
        assert!(matches!(err, TexError::BadMagic(_)));
    }

    #[test]
    fn parse_lz4_compressed_tex() {
        // Build a small texture and compress with LZ4
        let mut buf = Vec::new();
        buf.extend(b"TEXV0005\0");
        buf.extend(b"TEXI0001\0");
        buf.extend(0u32.to_le_bytes()); // ARGB8888
        buf.extend(0u32.to_le_bytes());
        buf.extend(64u32.to_le_bytes());
        buf.extend(64u32.to_le_bytes());
        buf.extend(64u32.to_le_bytes());
        buf.extend(64u32.to_le_bytes());
        buf.extend(0u32.to_le_bytes());
        buf.extend(b"TEXB0003\0");
        buf.extend(13u32.to_le_bytes()); // FIF_PNG
        buf.extend(1u32.to_le_bytes()); // image count
        buf.extend(1u32.to_le_bytes()); // mip count
        buf.extend(64u32.to_le_bytes()); // w
        buf.extend(64u32.to_le_bytes()); // h
        buf.extend(1u32.to_le_bytes()); // compression = LZ4
        let uncomp_size: i32 = 64 * 64 * 4;
        buf.extend(uncomp_size.to_le_bytes()); // uncompressed size
        // compress with lz4_flex
        // Use deterministic, low-compressibility data so this valid fixture stays
        // below the parser's anti-bomb ratio limit.
        let mut state = 0x1234_5678_u32;
        let raw_pixels: Vec<u8> = (0..uncomp_size)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                state as u8
            })
            .collect();
        let compressed = lz4_flex::compress(&raw_pixels);
        buf.extend((compressed.len() as u32).to_le_bytes()); // compressed size
        buf.extend(compressed);

        let tex = TexTexture::parse(&buf).unwrap();
        assert_eq!(tex.mipmaps.len(), 1);
        assert!(
            tex.mipmaps[0].data.len() == uncomp_size as usize
                || tex.mipmaps[0].data.len() >= raw_pixels.len()
        );
    }

    #[test]
    fn reject_excessive_lz4_expansion_ratio() {
        let mut buf = Vec::new();
        buf.extend(b"TEXV0005\0");
        buf.extend(b"TEXI0001\0");
        buf.extend(0u32.to_le_bytes());
        buf.extend(0u32.to_le_bytes());
        buf.extend(1u32.to_le_bytes());
        buf.extend(1u32.to_le_bytes());
        buf.extend(1u32.to_le_bytes());
        buf.extend(1u32.to_le_bytes());
        buf.extend(0u32.to_le_bytes());
        buf.extend(b"TEXB0003\0");
        buf.extend(13u32.to_le_bytes());
        buf.extend(1u32.to_le_bytes());
        buf.extend(1u32.to_le_bytes());
        buf.extend(1u32.to_le_bytes());
        buf.extend(1u32.to_le_bytes());
        buf.extend(1u32.to_le_bytes());
        buf.extend(1024i32.to_le_bytes());
        buf.extend(1u32.to_le_bytes());
        buf.push(0);

        assert!(matches!(
            TexTexture::parse(&buf),
            Err(TexError::CompressionRatioExceeded {
                compressed: 1,
                uncompressed: 1024
            })
        ));
    }

    #[test]
    fn empty_data_rejected() {
        assert!(TexTexture::parse(b"").is_err());
    }

    #[test]
    fn tex_format_conversion() {
        assert_eq!(TexFormat::try_from(0).unwrap(), TexFormat::ARGB8888);
        assert_eq!(TexFormat::try_from(4).unwrap(), TexFormat::DXT5);
        assert_eq!(TexFormat::try_from(12).unwrap(), TexFormat::BC7);
        assert!(TexFormat::try_from(999).is_err());
    }

    #[test]
    fn creates_validated_texture_image() {
        let tex = TexTexture::parse(&build_tex_rgba8(2, 2)).unwrap();
        let image = tex.to_texture_image().unwrap();
        assert_eq!(image.format, TexFormat::ARGB8888);
        assert_eq!(image.color_space, TextureColorSpace::Unknown);
        assert_eq!(image.alpha_mode, TextureAlphaMode::Unknown);
        assert_eq!(image.levels[0].data.len(), 16);
    }

    #[test]
    fn validates_raw_and_block_compressed_mipmap_sizes() {
        assert_eq!(expected_mipmap_size(TexFormat::RGBA8888, 3, 2).unwrap(), 24);
        assert_eq!(expected_mipmap_size(TexFormat::R8, 3, 2).unwrap(), 6);
        assert_eq!(expected_mipmap_size(TexFormat::DXT1, 1, 1).unwrap(), 8);
        assert_eq!(expected_mipmap_size(TexFormat::DXT5, 5, 4).unwrap(), 32);

        let mut tex = TexTexture::parse(&build_tex_rgba8(2, 2)).unwrap();
        tex.mipmaps[0].data = Arc::from([0_u8; 15]);
        assert!(matches!(
            tex.to_texture_image(),
            Err(TexError::InvalidMipmapDataSize {
                level: 0,
                expected: 16,
                actual: 15
            })
        ));
    }

    #[test]
    fn accepts_exact_storage_padding_dimensions() {
        let mut tex = TexTexture::parse(&build_tex_rgba8(2, 2)).unwrap();
        tex.texture_height = 4;
        tex.mipmaps[0].data = Arc::from([0_u8; 32]);
        let image = tex.to_texture_image().unwrap();
        assert_eq!((image.levels[0].width, image.levels[0].height), (2, 4));
    }
}
