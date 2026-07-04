use better_wallpaper_renderer::Scene2dAssets;
use better_wallpaper_scene_format::{BlendMode, TexFormat, TextureImage};
use glow::HasContext;
use std::collections::HashMap;
use tracing::{debug, trace, warn};

pub struct SceneGpuRenderer {
    gl: glow::Context,
    program: glow::Program,
    vbo: glow::Buffer,
    textures: HashMap<String, GpuTextureState>,
    output_size: (u32, u32),
}

#[allow(dead_code)]
struct GpuTextureState {
    texture: glow::Texture,
    width: u32,
    height: u32,
}

const VERTEX_SHADER: &str = "attribute vec2 a_position;\nattribute vec2 a_tex_coord;\nvarying vec2 v_tex_coord;\nvoid main() {\n    v_tex_coord = a_tex_coord;\n    gl_Position = vec4(a_position, 0.0, 1.0);\n}";
const FRAGMENT_SHADER: &str = "precision highp float;\nuniform sampler2D u_texture;\nuniform sampler2D u_water_wave_mask;\nuniform sampler2D u_water_wave_normal;\nuniform sampler2D u_water_flow_mask;\nuniform sampler2D u_water_flow_phase;\nuniform float u_has_water_wave_mask;\nuniform float u_has_water_wave_normal;\nuniform float u_has_water_flow_mask;\nuniform float u_has_water_flow_phase;\nuniform float u_opacity;\nuniform float u_time;\nuniform vec4 u_water_wave;\nuniform vec3 u_water_flow;\nvarying vec2 v_tex_coord;\nvoid main() {\n    vec2 uv = v_tex_coord;\n    if (u_water_wave.w > 0.0) {\n        float mask = mix(1.0, texture2D(u_water_wave_mask, v_tex_coord).r, u_has_water_wave_mask);\n        vec2 direction = vec2(cos(u_water_wave.x), sin(u_water_wave.x));\n        float frequency = 6.2831853 / max(u_water_wave.y, 0.01);\n        float phase = dot(uv, direction) * frequency + u_time * u_water_wave.z;\n        vec2 procedural = direction * sin(phase);\n        vec2 normal_uv = v_tex_coord / max(u_water_wave.y, 0.01) + direction * u_time * u_water_wave.z * 0.02;\n        vec2 normal = texture2D(u_water_wave_normal, normal_uv).rg * 2.0 - 1.0;\n        uv += mix(procedural, normal, u_has_water_wave_normal) * u_water_wave.w * 0.01 * mask;\n    }\n    if (u_water_flow.z > 0.0) {\n        float mask = mix(1.0, texture2D(u_water_flow_mask, v_tex_coord).r, u_has_water_flow_mask);\n        float phase = u_time * u_water_flow.y;\n        float frequency = 6.2831853 * max(u_water_flow.x, 0.01);\n        vec2 procedural = vec2(sin(uv.y * frequency + phase), cos(uv.x * frequency - phase));\n        vec2 phase_uv = v_tex_coord * max(u_water_flow.x, 0.01) + vec2(phase * 0.02, -phase * 0.015);\n        vec2 phase_flow = texture2D(u_water_flow_phase, phase_uv).rg * 2.0 - 1.0;\n        uv += mix(procedural, phase_flow, u_has_water_flow_phase) * u_water_flow.z * 0.003 * mask;\n    }\n    vec4 color = texture2D(u_texture, uv);\n    gl_FragColor = vec4(color.rgb, color.a * u_opacity);\n}";

const MAX_GPU_TEXTURES: usize = 256;

impl SceneGpuRenderer {
    pub fn new(gl: glow::Context, output_width: u32, output_height: u32) -> Result<Self, String> {
        unsafe {
            let program = compile_program(&gl, VERTEX_SHADER, FRAGMENT_SHADER)?;
            let vbo = gl
                .create_buffer()
                .map_err(|msg| format!("scene VBO allocate: {msg}"))?;
            Ok(Self {
                gl,
                program,
                vbo,
                textures: HashMap::new(),
                output_size: (output_width, output_height),
            })
        }
    }
    pub fn upload_scene_textures(&mut self, assets: &Scene2dAssets) -> Result<(), String> {
        unsafe {
            for draw in &assets.draws {
                let mut pending = vec![(&draw.texture_path, &draw.texture)];
                if let Some(mask) = &draw.water_wave_mask {
                    pending.push((&mask.path, &mask.texture));
                }
                if let Some(mask) = &draw.water_flow_mask {
                    pending.push((&mask.path, &mask.texture));
                }
                if let Some(normal) = &draw.water_wave_normal {
                    pending.push((&normal.path, &normal.texture));
                }
                if let Some(phase) = &draw.water_flow_phase {
                    pending.push((&phase.path, &phase.texture));
                }
                for (path, texture) in pending {
                    if self.textures.contains_key(path) {
                        continue;
                    }
                    let state = match self.upload_texture(texture) {
                        Ok(state) => state,
                        Err(error) => {
                            let base = texture.levels.first();
                            warn!(
                                texture = %path,
                                format = ?texture.format,
                                width = base.map_or(0, |level| level.width),
                                height = base.map_or(0, |level| level.height),
                                bytes = base.map_or(0, |level| level.data.len()),
                                %error,
                                "Skipping unsupported scene texture format"
                            );
                            continue;
                        }
                    };
                    #[allow(clippy::collapsible_if)]
                    if self.textures.len() >= MAX_GPU_TEXTURES {
                        if let Some(key) = self.textures.keys().next().cloned() {
                            if let Some(state) = self.textures.remove(&key) {
                                self.gl.delete_texture(state.texture);
                            }
                        }
                    }
                    self.textures.insert(path.clone(), state);
                }
            }
        }
        Ok(())
    }

    unsafe fn upload_texture(&mut self, image: &TextureImage) -> Result<GpuTextureState, String> {
        let base = image.levels.first().ok_or("texture has no mipmap levels")?;
        let gl_tex = unsafe {
            self.gl
                .create_texture()
                .map_err(|msg| format!("scene texture allocate: {msg}"))?
        };
        unsafe {
            self.gl.bind_texture(glow::TEXTURE_2D, Some(gl_tex));
            self.gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_MIN_FILTER,
                glow::LINEAR as i32,
            );
            self.gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_MAG_FILTER,
                glow::LINEAR as i32,
            );
            self.gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_WRAP_S,
                glow::CLAMP_TO_EDGE as i32,
            );
            self.gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_WRAP_T,
                glow::CLAMP_TO_EDGE as i32,
            );
        }

        let result = match image.format {
            TexFormat::DXT1 | TexFormat::DXT3 | TexFormat::DXT5 | TexFormat::BC7 => unsafe {
                self.upload_compressed_mipmaps(gl_tex, image)
            },
            _ => unsafe { self.upload_rgba8_mipmaps(gl_tex, image) },
        };
        unsafe {
            self.gl.bind_texture(glow::TEXTURE_2D, None);
        }
        result?;

        let error = unsafe { self.gl.get_error() };
        if error != glow::NO_ERROR {
            unsafe { self.gl.delete_texture(gl_tex) };
            return Err(format!(
                "OpenGL texture upload failed with error 0x{error:04x}"
            ));
        }

        Ok(GpuTextureState {
            texture: gl_tex,
            width: base.width,
            height: base.height,
        })
    }

    unsafe fn upload_compressed_mipmaps(
        &self,
        _tex: glow::Texture,
        image: &TextureImage,
    ) -> Result<(), String> {
        let internal_format = match image.format {
            TexFormat::DXT1 => glow::COMPRESSED_RGBA_S3TC_DXT1_EXT,
            TexFormat::DXT3 => glow::COMPRESSED_RGBA_S3TC_DXT3_EXT,
            TexFormat::DXT5 => glow::COMPRESSED_RGBA_S3TC_DXT5_EXT,
            TexFormat::BC7 => 0x8E8C, // GL_COMPRESSED_RGBA_BPTC_UNORM
            _ => unreachable!(),
        };
        for (level, mip) in image.levels.iter().enumerate() {
            unsafe {
                self.gl.compressed_tex_image_2d(
                    glow::TEXTURE_2D,
                    level as i32,
                    internal_format as i32,
                    mip.width as i32,
                    mip.height as i32,
                    0,
                    mip.data.len() as i32,
                    &mip.data,
                );
            }
        }
        Ok(())
    }

    unsafe fn upload_rgba8_mipmaps(
        &self,
        _tex: glow::Texture,
        image: &TextureImage,
    ) -> Result<(), String> {
        for (level, mip) in image.levels.iter().enumerate() {
            let rgba = convert_to_rgba8(image.format, &mip.data, mip.width, mip.height);
            unsafe {
                self.gl.tex_image_2d(
                    glow::TEXTURE_2D,
                    level as i32,
                    glow::RGBA as i32,
                    mip.width as i32,
                    mip.height as i32,
                    0,
                    glow::RGBA,
                    glow::UNSIGNED_BYTE,
                    Some(&rgba),
                );
            }
        }
        Ok(())
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        self.output_size = (width, height);
    }

    pub fn draw_scene(&self, assets: &Scene2dAssets, elapsed_seconds: f64) -> Result<(), String> {
        let start = std::time::Instant::now();
        let mut drawn = 0u32;
        unsafe {
            self.gl
                .viewport(0, 0, self.output_size.0 as i32, self.output_size.1 as i32);
            self.gl.clear_color(0.0, 0.0, 0.0, 1.0);
            self.gl.clear(glow::COLOR_BUFFER_BIT);
            self.gl.use_program(Some(self.program));
            self.gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.vbo));
            let position_loc = self
                .gl
                .get_attrib_location(self.program, "a_position")
                .unwrap_or(0);
            let tex_coord_loc = self
                .gl
                .get_attrib_location(self.program, "a_tex_coord")
                .unwrap_or(1);
            let opacity_loc = self.gl.get_uniform_location(self.program, "u_opacity");
            let sampler_loc = self.gl.get_uniform_location(self.program, "u_texture");
            let time_loc = self.gl.get_uniform_location(self.program, "u_time");
            let water_wave_loc = self.gl.get_uniform_location(self.program, "u_water_wave");
            let water_flow_loc = self.gl.get_uniform_location(self.program, "u_water_flow");
            let water_wave_mask_loc = self
                .gl
                .get_uniform_location(self.program, "u_water_wave_mask");
            let water_flow_mask_loc = self
                .gl
                .get_uniform_location(self.program, "u_water_flow_mask");
            let water_wave_normal_loc = self
                .gl
                .get_uniform_location(self.program, "u_water_wave_normal");
            let water_flow_phase_loc = self
                .gl
                .get_uniform_location(self.program, "u_water_flow_phase");
            let has_water_wave_mask_loc = self
                .gl
                .get_uniform_location(self.program, "u_has_water_wave_mask");
            let has_water_flow_mask_loc = self
                .gl
                .get_uniform_location(self.program, "u_has_water_flow_mask");
            let has_water_wave_normal_loc = self
                .gl
                .get_uniform_location(self.program, "u_has_water_wave_normal");
            let has_water_flow_phase_loc = self
                .gl
                .get_uniform_location(self.program, "u_has_water_flow_phase");
            self.gl.enable_vertex_attrib_array(position_loc);
            self.gl.enable_vertex_attrib_array(tex_coord_loc);
            self.gl.uniform_1_i32(sampler_loc.as_ref(), 0);
            self.gl.uniform_1_i32(water_wave_mask_loc.as_ref(), 1);
            self.gl.uniform_1_i32(water_flow_mask_loc.as_ref(), 2);
            self.gl.uniform_1_i32(water_wave_normal_loc.as_ref(), 3);
            self.gl.uniform_1_i32(water_flow_phase_loc.as_ref(), 4);
            self.gl
                .uniform_1_f32(time_loc.as_ref(), elapsed_seconds.rem_euclid(3600.0) as f32);
            for draw in &assets.draws {
                let Some(state) = self.textures.get(&draw.texture_path) else {
                    continue;
                };
                match draw.blend_mode {
                    BlendMode::Opaque => {
                        self.gl.disable(glow::BLEND);
                    }
                    BlendMode::Translucent => {
                        self.gl.enable(glow::BLEND);
                        self.gl
                            .blend_func(glow::SRC_ALPHA, glow::ONE_MINUS_SRC_ALPHA);
                    }
                    BlendMode::Additive => {
                        self.gl.enable(glow::BLEND);
                        self.gl.blend_func(glow::SRC_ALPHA, glow::ONE);
                    }
                    _ => {
                        self.gl.enable(glow::BLEND);
                        self.gl
                            .blend_func(glow::SRC_ALPHA, glow::ONE_MINUS_SRC_ALPHA);
                    }
                }
                self.gl.active_texture(glow::TEXTURE0);
                self.gl.bind_texture(glow::TEXTURE_2D, Some(state.texture));
                let wrap = if draw.quad.scroll.is_some() {
                    glow::REPEAT
                } else {
                    glow::CLAMP_TO_EDGE
                } as i32;
                self.gl
                    .tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_S, wrap);
                self.gl
                    .tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_T, wrap);
                self.gl
                    .uniform_1_f32(opacity_loc.as_ref(), draw.quad.opacity);
                if let Some(wave) = &draw.quad.water_wave {
                    self.gl.uniform_4_f32(
                        water_wave_loc.as_ref(),
                        wave.direction,
                        wave.scale,
                        wave.speed,
                        wave.strength,
                    );
                } else {
                    self.gl
                        .uniform_4_f32(water_wave_loc.as_ref(), 0.0, 1.0, 0.0, 0.0);
                }
                if let Some(flow) = &draw.quad.water_flow {
                    self.gl.uniform_3_f32(
                        water_flow_loc.as_ref(),
                        flow.phase_scale,
                        flow.speed,
                        flow.strength,
                    );
                } else {
                    self.gl
                        .uniform_3_f32(water_flow_loc.as_ref(), 1.0, 0.0, 0.0);
                }
                bind_optional_texture(
                    &self.gl,
                    &self.textures,
                    glow::TEXTURE1,
                    draw.water_wave_mask
                        .as_ref()
                        .map(|texture| texture.path.as_str()),
                    has_water_wave_mask_loc.as_ref(),
                    false,
                );
                bind_optional_texture(
                    &self.gl,
                    &self.textures,
                    glow::TEXTURE2,
                    draw.water_flow_mask
                        .as_ref()
                        .map(|texture| texture.path.as_str()),
                    has_water_flow_mask_loc.as_ref(),
                    false,
                );
                bind_optional_texture(
                    &self.gl,
                    &self.textures,
                    glow::TEXTURE3,
                    draw.water_wave_normal
                        .as_ref()
                        .map(|texture| texture.path.as_str()),
                    has_water_wave_normal_loc.as_ref(),
                    true,
                );
                bind_optional_texture(
                    &self.gl,
                    &self.textures,
                    glow::TEXTURE4,
                    draw.water_flow_phase
                        .as_ref()
                        .map(|texture| texture.path.as_str()),
                    has_water_flow_phase_loc.as_ref(),
                    true,
                );
                self.gl.active_texture(glow::TEXTURE0);
                let mut uv = draw
                    .animation
                    .as_ref()
                    .map_or(draw.uv, |animation| animation.uv_at(elapsed_seconds));
                if let Some(scroll) = draw.quad.scroll {
                    uv = scroll_uv(uv, scroll, elapsed_seconds);
                }
                let vertices = scene_vertices(&draw.quad.vertices, uv);
                let vertex_bytes: &[u8] = std::slice::from_raw_parts(
                    vertices.as_ptr().cast::<u8>(),
                    vertices.len() * std::mem::size_of::<f32>(),
                );
                self.gl
                    .buffer_data_u8_slice(glow::ARRAY_BUFFER, vertex_bytes, glow::DYNAMIC_DRAW);
                self.gl
                    .vertex_attrib_pointer_f32(position_loc, 2, glow::FLOAT, false, 16, 0);
                self.gl
                    .vertex_attrib_pointer_f32(tex_coord_loc, 2, glow::FLOAT, false, 16, 8);
                self.gl.draw_arrays(glow::TRIANGLE_FAN, 0, 4);
                drawn += 1;
            }
            self.gl.disable(glow::BLEND);
            self.gl.disable_vertex_attrib_array(position_loc);
            self.gl.disable_vertex_attrib_array(tex_coord_loc);
            self.gl.bind_buffer(glow::ARRAY_BUFFER, None);
            self.gl.bind_texture(glow::TEXTURE_2D, None);
            self.gl.use_program(None);
        }
        trace!(
            drawn,
            total = assets.draws.len(),
            elapsed_us = start.elapsed().as_micros() as u64,
            "scene frame rendered"
        );
        let error = unsafe { self.gl.get_error() };
        if error != glow::NO_ERROR {
            return Err(format!("OpenGL scene draw failed with error 0x{error:04x}"));
        }
        Ok(())
    }

    pub fn clear_textures(&mut self) {
        unsafe {
            for (_, state) in self.textures.drain() {
                self.gl.delete_texture(state.texture);
            }
        }
    }
}

unsafe fn bind_optional_texture(
    gl: &glow::Context,
    textures: &HashMap<String, GpuTextureState>,
    unit: u32,
    path: Option<&str>,
    present_location: Option<&glow::UniformLocation>,
    repeat: bool,
) {
    unsafe {
        gl.active_texture(unit);
        let texture = path.and_then(|path| textures.get(path));
        gl.bind_texture(glow::TEXTURE_2D, texture.map(|state| state.texture));
        if texture.is_some() {
            let wrap = if repeat {
                glow::REPEAT
            } else {
                glow::CLAMP_TO_EDGE
            } as i32;
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_S, wrap);
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_T, wrap);
        }
        gl.uniform_1_f32(present_location, f32::from(texture.is_some()));
    }
}

fn scroll_uv(
    uv: [f32; 4],
    scroll: better_wallpaper_scene_format::ScrollEffect,
    elapsed_seconds: f64,
) -> [f32; 4] {
    let width = uv[2] - uv[0];
    let height = uv[3] - uv[1];
    let phase_x = (elapsed_seconds * f64::from(scroll.speed_x)).rem_euclid(1.0) as f32;
    let phase_y = (elapsed_seconds * f64::from(scroll.speed_y)).rem_euclid(1.0) as f32;
    [
        uv[0] + phase_x * width,
        uv[1] + phase_y * height,
        uv[0] + (phase_x + scroll.repeat_x) * width,
        uv[1] + (phase_y + scroll.repeat_y) * height,
    ]
}

fn scene_vertices(positions: &[[f32; 2]; 4], uv: [f32; 4]) -> [f32; 16] {
    let [left, top, right, bottom] = uv;
    let tex_coords = [[left, top], [right, top], [right, bottom], [left, bottom]];
    let mut vertices = [0.0; 16];
    for (index, (position, tex_coord)) in positions.iter().zip(tex_coords).enumerate() {
        let offset = index * 4;
        vertices[offset..offset + 2].copy_from_slice(position);
        vertices[offset + 2..offset + 4].copy_from_slice(&tex_coord);
    }
    vertices
}

/// Convert a Wallpaper Engine texture payload to canonical RGBA8 for GPU upload.
///
/// Wallpaper Engine's format id 0 is historically named ARGB8888, but observed
/// TEX payloads store bytes in RGBA order. Preserve that byte order for the
/// portable GL_RGBA upload path.
fn convert_to_rgba8(
    format: TexFormat,
    data: &[u8],
    width: u32,
    height: u32,
) -> std::borrow::Cow<'_, [u8]> {
    let pixel_count = (width as usize) * (height as usize);
    match format {
        TexFormat::RGBA8888 | TexFormat::ARGB8888 => std::borrow::Cow::Borrowed(data),
        TexFormat::RGB888 => {
            let mut out = Vec::with_capacity(pixel_count * 4);
            for chunk in data.chunks_exact(3) {
                out.extend_from_slice(&[chunk[0], chunk[1], chunk[2], 255]);
            }
            std::borrow::Cow::Owned(out)
        }
        TexFormat::RGB565 => {
            let mut out = Vec::with_capacity(pixel_count * 4);
            for chunk in data.chunks_exact(2) {
                let pixel = u16::from_le_bytes([chunk[0], chunk[1]]);
                let r = ((pixel >> 11) & 0x1F) as u8 * 255 / 31;
                let g = ((pixel >> 5) & 0x3F) as u8 * 255 / 63;
                let b = (pixel & 0x1F) as u8 * 255 / 31;
                out.extend_from_slice(&[r, g, b, 255]);
            }
            std::borrow::Cow::Owned(out)
        }
        TexFormat::R8 => {
            let mut out = Vec::with_capacity(pixel_count * 4);
            for &value in data.iter().take(pixel_count) {
                out.extend_from_slice(&[value, value, value, 255]);
            }
            std::borrow::Cow::Owned(out)
        }
        TexFormat::RG88 => {
            let mut out = Vec::with_capacity(pixel_count * 4);
            for chunk in data.chunks_exact(2).take(pixel_count) {
                out.extend_from_slice(&[chunk[0], chunk[1], 0, 255]);
            }
            std::borrow::Cow::Owned(out)
        }
        TexFormat::RGBa1010102 => {
            let mut out = Vec::with_capacity(pixel_count * 4);
            for chunk in data.chunks_exact(4) {
                let pixel = u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
                let r = ((pixel & 0x3FF) as f32 / 1023.0 * 255.0) as u8;
                let g = (((pixel >> 10) & 0x3FF) as f32 / 1023.0 * 255.0) as u8;
                let b = (((pixel >> 20) & 0x3FF) as f32 / 1023.0 * 255.0) as u8;
                let a = ((pixel >> 30) as f32 / 3.0 * 255.0) as u8;
                out.extend_from_slice(&[r, g, b, a]);
            }
            std::borrow::Cow::Owned(out)
        }
        TexFormat::RGBA16161616f | TexFormat::RGB161616f | TexFormat::RG1616f | TexFormat::R16f => {
            std::borrow::Cow::Owned(convert_float_to_rgba8(format, data, pixel_count))
        }
        TexFormat::DXT1 | TexFormat::DXT3 | TexFormat::DXT5 | TexFormat::BC7 => {
            // Compressed formats handled by upload_compressed_mipmaps
            std::borrow::Cow::Owned(Vec::new())
        }
    }
}

fn convert_float_to_rgba8(format: TexFormat, data: &[u8], pixel_count: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(pixel_count * 4);
    match format {
        TexFormat::RGBA16161616f => {
            for chunk in data.chunks_exact(8).take(pixel_count) {
                let r = half::f16::from_le_bytes([chunk[0], chunk[1]]).to_f32();
                let g = half::f16::from_le_bytes([chunk[2], chunk[3]]).to_f32();
                let b = half::f16::from_le_bytes([chunk[4], chunk[5]]).to_f32();
                let a = half::f16::from_le_bytes([chunk[6], chunk[7]]).to_f32();
                out.extend_from_slice(&[
                    (r.clamp(0.0, 1.0) * 255.0) as u8,
                    (g.clamp(0.0, 1.0) * 255.0) as u8,
                    (b.clamp(0.0, 1.0) * 255.0) as u8,
                    (a.clamp(0.0, 1.0) * 255.0) as u8,
                ]);
            }
        }
        TexFormat::RGB161616f => {
            for chunk in data.chunks_exact(6).take(pixel_count) {
                let r = half::f16::from_le_bytes([chunk[0], chunk[1]]).to_f32();
                let g = half::f16::from_le_bytes([chunk[2], chunk[3]]).to_f32();
                let b = half::f16::from_le_bytes([chunk[4], chunk[5]]).to_f32();
                out.extend_from_slice(&[
                    (r.clamp(0.0, 1.0) * 255.0) as u8,
                    (g.clamp(0.0, 1.0) * 255.0) as u8,
                    (b.clamp(0.0, 1.0) * 255.0) as u8,
                    255,
                ]);
            }
        }
        TexFormat::RG1616f => {
            for chunk in data.chunks_exact(4).take(pixel_count) {
                let r = half::f16::from_le_bytes([chunk[0], chunk[1]]).to_f32();
                let g = half::f16::from_le_bytes([chunk[2], chunk[3]]).to_f32();
                out.extend_from_slice(&[
                    (r.clamp(0.0, 1.0) * 255.0) as u8,
                    (g.clamp(0.0, 1.0) * 255.0) as u8,
                    0,
                    255,
                ]);
            }
        }
        TexFormat::R16f => {
            for chunk in data.chunks_exact(2).take(pixel_count) {
                let r = half::f16::from_le_bytes([chunk[0], chunk[1]]).to_f32();
                let v = (r.clamp(0.0, 1.0) * 255.0) as u8;
                out.extend_from_slice(&[v, v, v, 255]);
            }
        }
        _ => {}
    }
    out
}

impl Drop for SceneGpuRenderer {
    fn drop(&mut self) {
        unsafe {
            self.clear_textures();
            self.gl.delete_buffer(self.vbo);
            self.gl.delete_program(self.program);
        }
        debug!("scene gpu renderer resources released");
    }
}

unsafe fn compile_program(
    gl: &glow::Context,
    vertex: &str,
    fragment: &str,
) -> Result<glow::Program, String> {
    unsafe {
        let program = gl
            .create_program()
            .map_err(|msg| format!("scene program create: {msg}"))?;
        let vert = gl
            .create_shader(glow::VERTEX_SHADER)
            .map_err(|msg| format!("scene vert shader: {msg}"))?;
        let frag = gl
            .create_shader(glow::FRAGMENT_SHADER)
            .map_err(|msg| format!("scene frag shader: {msg}"))?;
        for (shader, source) in [(vert, vertex), (frag, fragment)] {
            gl.shader_source(shader, source);
            gl.compile_shader(shader);
            if !gl.get_shader_compile_status(shader) {
                let log = gl.get_shader_info_log(shader);
                return Err(format!("shader compile: {log}"));
            }
        }
        gl.attach_shader(program, vert);
        gl.attach_shader(program, frag);
        gl.link_program(program);
        if !gl.get_program_link_status(program) {
            let log = gl.get_program_info_log(program);
            gl.delete_shader(vert);
            gl.delete_shader(frag);
            gl.delete_program(program);
            return Err(format!("program link: {log}"));
        }
        gl.delete_shader(vert);
        gl.delete_shader(frag);
        Ok(program)
    }
}

#[cfg(test)]
mod tests {
    use super::{convert_to_rgba8, scene_vertices, scroll_uv};
    use better_wallpaper_scene_format::{ScrollEffect, TexFormat};

    #[test]
    fn interleaves_scene_positions_with_quad_texture_coordinates() {
        let positions = [[-1.0, -0.5], [0.5, -0.5], [0.5, 1.0], [-1.0, 1.0]];
        assert_eq!(
            scene_vertices(&positions, [0.0, 0.0, 1.0, 1.0]),
            [
                -1.0, -0.5, 0.0, 0.0, 0.5, -0.5, 1.0, 0.0, 0.5, 1.0, 1.0, 1.0, -1.0, 1.0, 0.0, 1.0,
            ]
        );
    }

    #[test]
    fn maps_a_sprite_subrectangle_to_quad_uvs() {
        let positions = [[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]];
        let vertices = scene_vertices(&positions, [0.25, 0.5, 0.5, 1.0]);
        assert_eq!(&vertices[2..4], &[0.25, 0.5]);
        assert_eq!(&vertices[10..12], &[0.5, 1.0]);
    }

    #[test]
    fn preserves_observed_format_zero_rgba_channel_order() {
        let blue_pixel = [7, 20, 240, 255];
        let rgba = convert_to_rgba8(TexFormat::ARGB8888, &blue_pixel, 1, 1);
        assert_eq!(rgba.as_ref(), &blue_pixel);
    }

    #[test]
    fn preserves_decoded_rgba8_channel_order() {
        let pixels = [240, 20, 7, 128];
        let rgba = convert_to_rgba8(TexFormat::RGBA8888, &pixels, 1, 1);
        assert_eq!(rgba.as_ref(), &pixels);
    }

    #[test]
    fn scroll_effect_moves_and_repeats_texture_coordinates() {
        let effect = ScrollEffect {
            speed_x: 0.25,
            speed_y: -0.5,
            repeat_x: 2.0,
            repeat_y: 1.0,
        };
        assert_eq!(
            scroll_uv([0.0, 0.0, 1.0, 1.0], effect, 2.0),
            [0.5, 0.0, 2.5, 1.0]
        );
    }
}
