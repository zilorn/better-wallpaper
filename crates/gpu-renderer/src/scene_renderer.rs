use better_wallpaper_renderer::{Scene2dAssets, font_text_texture};
use better_wallpaper_scene_format::{
    BlendMode, DynamicScaleKind, DynamicTextKind, TexFormat, TextureImage,
};
use glow::HasContext;
use std::collections::HashMap;
use tracing::{debug, trace, warn};

pub struct SceneGpuRenderer {
    gl: glow::Context,
    program: glow::Program,
    vbo: glow::Buffer,
    index_buffer: glow::Buffer,
    textures: HashMap<String, GpuTextureState>,
    output_size: (u32, u32),
    supports_npot_mipmaps: bool,
    max_texture_size: u32,
}

#[allow(dead_code)]
struct GpuTextureState {
    texture: glow::Texture,
    width: u32,
    height: u32,
    dynamic_label: Option<String>,
}

const VERTEX_SHADER: &str = include_str!("shaders/scene.vert");
const FRAGMENT_SHADER: &str = include_str!("shaders/scene.frag");

const MAX_GPU_TEXTURES: usize = 256;

impl SceneGpuRenderer {
    pub fn new(gl: glow::Context, output_width: u32, output_height: u32) -> Result<Self, String> {
        unsafe {
            let version = gl.get_parameter_string(glow::VERSION);
            let supports_npot_mipmaps = !version.contains("OpenGL ES")
                || version.contains("OpenGL ES 3")
                || gl.supported_extensions().contains("GL_OES_texture_npot");
            let max_texture_size = gl.get_parameter_i32(glow::MAX_TEXTURE_SIZE).max(1) as u32;
            let program = compile_program(&gl, VERTEX_SHADER, FRAGMENT_SHADER)?;
            let vbo = gl
                .create_buffer()
                .map_err(|msg| format!("scene VBO allocate: {msg}"))?;
            let index_buffer = gl
                .create_buffer()
                .map_err(|msg| format!("scene index buffer allocate: {msg}"))?;
            Ok(Self {
                gl,
                program,
                vbo,
                index_buffer,
                textures: HashMap::new(),
                output_size: (output_width, output_height),
                supports_npot_mipmaps,
                max_texture_size,
            })
        }
    }
    pub fn upload_scene_textures(&mut self, assets: &Scene2dAssets) -> Result<(), String> {
        unsafe {
            for draw in &assets.draws {
                let mut pending = vec![(&draw.texture_path, &draw.texture)];
                for mask in draw.water_wave_masks.iter().flatten() {
                    pending.push((&mask.path, &mask.texture));
                }
                if let Some(mask) = &draw.water_flow_mask {
                    pending.push((&mask.path, &mask.texture));
                }
                for normal in draw.water_wave_normals.iter().flatten() {
                    pending.push((&normal.path, &normal.texture));
                }
                if let Some(phase) = &draw.water_flow_phase {
                    pending.push((&phase.path, &phase.texture));
                }
                if let Some(mask) = &draw.iris_mask {
                    pending.push((&mask.path, &mask.texture));
                }
                for mask in draw.foliage_masks.iter().flatten() {
                    pending.push((&mask.path, &mask.texture));
                }
                if let Some(mask) = &draw.shine_mask {
                    pending.push((&mask.path, &mask.texture));
                }
                for map in draw.shake_maps.iter().flatten() {
                    pending.push((&map.path, &map.texture));
                }
                for mask in draw.pulse_masks.iter().flatten() {
                    pending.push((&mask.path, &mask.texture));
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
        let source_base = image.levels.first().ok_or("texture has no mipmap levels")?;
        let first_supported =
            first_supported_mip(image, self.max_texture_size).ok_or_else(|| {
                format!(
                    "texture has no mip level within GPU limit {}x{}",
                    self.max_texture_size, self.max_texture_size
                )
            })?;
        let upload_image = TextureImage {
            format: image.format,
            color_space: image.color_space,
            alpha_mode: image.alpha_mode,
            levels: image.levels[first_supported..].to_vec(),
        };
        let base = upload_image
            .levels
            .first()
            .ok_or("texture has no supported mipmap levels")?;
        if first_supported > 0 {
            debug!(
                source_width = source_base.width,
                source_height = source_base.height,
                upload_width = base.width,
                upload_height = base.height,
                gpu_max_texture_size = self.max_texture_size,
                skipped_mip_levels = first_supported,
                "Selected a smaller scene texture mip for GPU compatibility"
            );
        }
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

        let result = match upload_image.format {
            TexFormat::DXT1 | TexFormat::DXT3 | TexFormat::DXT5 | TexFormat::BC7 => unsafe {
                self.upload_compressed_mipmaps(gl_tex, &upload_image)
            },
            _ => unsafe { self.upload_rgba8_mipmaps(gl_tex, &upload_image) },
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
            dynamic_label: None,
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
        if self.can_use_mipmaps(image) && mip_chain_is_complete(image) {
            unsafe {
                self.gl.tex_parameter_i32(
                    glow::TEXTURE_2D,
                    glow::TEXTURE_MIN_FILTER,
                    glow::LINEAR_MIPMAP_LINEAR as i32,
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
        let mut last_rgba = None;
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
            if level + 1 == image.levels.len() {
                last_rgba = Some(rgba.into_owned());
            }
        }
        if self.can_use_mipmaps(image) && mip_chain_is_contiguous(image) {
            let last = image.levels.last().ok_or("texture has no mipmap levels")?;
            let mut width = last.width;
            let mut height = last.height;
            let mut rgba = last_rgba.ok_or("texture has no decoded mipmap data")?;
            let mut level = image.levels.len() as i32;
            while width > 1 || height > 1 {
                let (next_width, next_height, next_rgba) = downsample_rgba8(&rgba, width, height);
                unsafe {
                    self.gl.tex_image_2d(
                        glow::TEXTURE_2D,
                        level,
                        glow::RGBA as i32,
                        next_width as i32,
                        next_height as i32,
                        0,
                        glow::RGBA,
                        glow::UNSIGNED_BYTE,
                        Some(&next_rgba),
                    );
                }
                width = next_width;
                height = next_height;
                rgba = next_rgba;
                level += 1;
            }
            unsafe {
                self.gl.tex_parameter_i32(
                    glow::TEXTURE_2D,
                    glow::TEXTURE_MIN_FILTER,
                    glow::LINEAR_MIPMAP_LINEAR as i32,
                );
            }
        }
        Ok(())
    }

    fn can_use_mipmaps(&self, image: &TextureImage) -> bool {
        image.levels.first().is_some_and(|base| {
            self.supports_npot_mipmaps
                || (base.width.is_power_of_two() && base.height.is_power_of_two())
        })
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        self.output_size = (width, height);
    }

    pub fn draw_scene(
        &mut self,
        assets: &Scene2dAssets,
        elapsed_seconds: f64,
    ) -> Result<(), String> {
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
            let effect_coord_loc = self
                .gl
                .get_attrib_location(self.program, "a_effect_coord")
                .unwrap_or(2);
            let opacity_loc = self.gl.get_uniform_location(self.program, "u_opacity");
            let sampler_loc = self.gl.get_uniform_location(self.program, "u_texture");
            let time_loc = self.gl.get_uniform_location(self.program, "u_time");
            let texture_uv_scale_loc = self
                .gl
                .get_uniform_location(self.program, "u_texture_uv_scale");
            let layout_scale_loc = self.gl.get_uniform_location(self.program, "u_layout_scale");
            let water_wave_locs = (0..3)
                .map(|index| {
                    self.gl
                        .get_uniform_location(self.program, &format!("u_water_wave{index}"))
                })
                .collect::<Vec<_>>();
            let water_flow_loc = self.gl.get_uniform_location(self.program, "u_water_flow");
            let water_wave_mask_locs = (0..3)
                .map(|index| {
                    self.gl
                        .get_uniform_location(self.program, &format!("u_water_wave_mask{index}"))
                })
                .collect::<Vec<_>>();
            let water_flow_mask_loc = self
                .gl
                .get_uniform_location(self.program, "u_water_flow_mask");
            let water_wave_normal_loc = self
                .gl
                .get_uniform_location(self.program, "u_water_wave_normal");
            let water_flow_phase_loc = self
                .gl
                .get_uniform_location(self.program, "u_water_flow_phase");
            let has_water_wave_mask_locs = (0..3)
                .map(|index| {
                    self.gl.get_uniform_location(
                        self.program,
                        &format!("u_has_water_wave_mask{index}"),
                    )
                })
                .collect::<Vec<_>>();
            let has_water_flow_mask_loc = self
                .gl
                .get_uniform_location(self.program, "u_has_water_flow_mask");
            let has_water_wave_normal_loc = self
                .gl
                .get_uniform_location(self.program, "u_has_water_wave_normal");
            let has_water_flow_phase_loc = self
                .gl
                .get_uniform_location(self.program, "u_has_water_flow_phase");
            let iris_loc = self.gl.get_uniform_location(self.program, "u_iris");
            let iris_scale_loc = self.gl.get_uniform_location(self.program, "u_iris_scale");
            let iris_mask_loc = self.gl.get_uniform_location(self.program, "u_iris_mask");
            let has_iris_mask_loc = self
                .gl
                .get_uniform_location(self.program, "u_has_iris_mask");
            let foliage0_loc = self.gl.get_uniform_location(self.program, "u_foliage0");
            let foliage0_extra_loc = self
                .gl
                .get_uniform_location(self.program, "u_foliage0_extra");
            let foliage1_loc = self.gl.get_uniform_location(self.program, "u_foliage1");
            let foliage1_extra_loc = self
                .gl
                .get_uniform_location(self.program, "u_foliage1_extra");
            let foliage_mask0_loc = self
                .gl
                .get_uniform_location(self.program, "u_foliage_mask0");
            let foliage_mask1_loc = self
                .gl
                .get_uniform_location(self.program, "u_foliage_mask1");
            let has_foliage_mask0_loc = self
                .gl
                .get_uniform_location(self.program, "u_has_foliage_mask0");
            let has_foliage_mask1_loc = self
                .gl
                .get_uniform_location(self.program, "u_has_foliage_mask1");
            let shine_loc = self.gl.get_uniform_location(self.program, "u_shine");
            let shine_color_loc = self.gl.get_uniform_location(self.program, "u_shine_color");
            let shine_mask_loc = self.gl.get_uniform_location(self.program, "u_shine_mask");
            let has_shine_mask_loc = self
                .gl
                .get_uniform_location(self.program, "u_has_shine_mask");
            let shake_locs = (0..3)
                .map(|index| {
                    self.gl
                        .get_uniform_location(self.program, &format!("u_shake{index}"))
                })
                .collect::<Vec<_>>();
            let shake_friction_locs = (0..3)
                .map(|index| {
                    self.gl
                        .get_uniform_location(self.program, &format!("u_shake_friction{index}"))
                })
                .collect::<Vec<_>>();
            let shake_map_locs = (0..3)
                .map(|index| {
                    self.gl
                        .get_uniform_location(self.program, &format!("u_shake_map{index}"))
                })
                .collect::<Vec<_>>();
            let has_shake_map_locs = (0..3)
                .map(|index| {
                    self.gl
                        .get_uniform_location(self.program, &format!("u_has_shake_map{index}"))
                })
                .collect::<Vec<_>>();
            let pulse_locs = (0..3)
                .map(|index| {
                    self.gl
                        .get_uniform_location(self.program, &format!("u_pulse{index}"))
                })
                .collect::<Vec<_>>();
            let pulse_bounds_locs = (0..3)
                .map(|index| {
                    self.gl
                        .get_uniform_location(self.program, &format!("u_pulse_bounds{index}"))
                })
                .collect::<Vec<_>>();
            let pulse_low_locs = (0..3)
                .map(|index| {
                    self.gl
                        .get_uniform_location(self.program, &format!("u_pulse_low{index}"))
                })
                .collect::<Vec<_>>();
            let pulse_high_locs = (0..3)
                .map(|index| {
                    self.gl
                        .get_uniform_location(self.program, &format!("u_pulse_high{index}"))
                })
                .collect::<Vec<_>>();
            let pulse_mask_locs = (0..3)
                .map(|index| {
                    self.gl
                        .get_uniform_location(self.program, &format!("u_pulse_mask{index}"))
                })
                .collect::<Vec<_>>();
            let has_pulse_mask_locs = (0..3)
                .map(|index| {
                    self.gl
                        .get_uniform_location(self.program, &format!("u_has_pulse_mask{index}"))
                })
                .collect::<Vec<_>>();
            let spin_loc = self.gl.get_uniform_location(self.program, "u_spin");
            let spin_aspect_loc = self.gl.get_uniform_location(self.program, "u_spin_aspect");
            let has_spin_loc = self.gl.get_uniform_location(self.program, "u_has_spin");
            self.gl.enable_vertex_attrib_array(position_loc);
            self.gl.enable_vertex_attrib_array(tex_coord_loc);
            self.gl.enable_vertex_attrib_array(effect_coord_loc);
            self.gl.uniform_1_i32(sampler_loc.as_ref(), 0);
            let mut layout_scale = scene_layout_scale(assets, self.output_size);
            let camera_zoom = assets
                .camera_zoom_animation
                .as_ref()
                .map_or(assets.camera_zoom, |animation| {
                    animation.value_at(elapsed_seconds)
                })
                .clamp(0.01, 100.0);
            layout_scale[0] *= camera_zoom;
            layout_scale[1] *= camera_zoom;
            self.gl
                .uniform_2_f32(layout_scale_loc.as_ref(), layout_scale[0], layout_scale[1]);
            for (location, unit) in water_wave_mask_locs.iter().zip([1, 9, 10]) {
                self.gl.uniform_1_i32(location.as_ref(), unit);
            }
            self.gl.uniform_1_i32(water_flow_mask_loc.as_ref(), 2);
            self.gl.uniform_1_i32(water_wave_normal_loc.as_ref(), 3);
            self.gl.uniform_1_i32(water_flow_phase_loc.as_ref(), 4);
            self.gl.uniform_1_i32(iris_mask_loc.as_ref(), 5);
            self.gl.uniform_1_i32(foliage_mask0_loc.as_ref(), 6);
            self.gl.uniform_1_i32(foliage_mask1_loc.as_ref(), 7);
            self.gl.uniform_1_i32(shine_mask_loc.as_ref(), 8);
            for (location, unit) in shake_map_locs.iter().zip([11, 12, 3]) {
                self.gl.uniform_1_i32(location.as_ref(), unit);
            }
            for (location, unit) in pulse_mask_locs.iter().zip([13, 14, 15]) {
                self.gl.uniform_1_i32(location.as_ref(), unit);
            }
            self.gl
                .uniform_1_f32(time_loc.as_ref(), elapsed_seconds.rem_euclid(3600.0) as f32);
            for draw in &assets.draws {
                let opacity = draw
                    .quad
                    .opacity_animation
                    .as_ref()
                    .map_or(draw.quad.opacity, |animation| {
                        animation.value_at(elapsed_seconds)
                    })
                    .clamp(0.0, 1.0);
                if opacity <= f32::EPSILON {
                    continue;
                }
                if let Some(render) = draw
                    .text_render
                    .as_ref()
                    .filter(|render| render.dynamic.is_some())
                {
                    let local = local_date_time();
                    let label = dynamic_text_label(
                        render.dynamic.expect("filtered dynamic text renderer"),
                        local,
                    );
                    let texture = font_text_texture(&label, render);
                    let level = texture
                        .levels
                        .first()
                        .ok_or("dynamic text texture has no base level")?;
                    let state = self
                        .textures
                        .get_mut(&draw.texture_path)
                        .ok_or("dynamic text GPU texture is missing")?;
                    if state.dynamic_label.as_deref() != Some(&label) {
                        self.gl.active_texture(glow::TEXTURE0);
                        self.gl.bind_texture(glow::TEXTURE_2D, Some(state.texture));
                        self.gl.tex_parameter_i32(
                            glow::TEXTURE_2D,
                            glow::TEXTURE_MIN_FILTER,
                            glow::LINEAR as i32,
                        );
                        if state.width == level.width && state.height == level.height {
                            self.gl.tex_sub_image_2d(
                                glow::TEXTURE_2D,
                                0,
                                0,
                                0,
                                level.width as i32,
                                level.height as i32,
                                glow::RGBA,
                                glow::UNSIGNED_BYTE,
                                glow::PixelUnpackData::Slice(&level.data),
                            );
                        } else {
                            self.gl.tex_image_2d(
                                glow::TEXTURE_2D,
                                0,
                                glow::RGBA as i32,
                                level.width as i32,
                                level.height as i32,
                                0,
                                glow::RGBA,
                                glow::UNSIGNED_BYTE,
                                Some(&level.data),
                            );
                            state.width = level.width;
                            state.height = level.height;
                        }
                        state.dynamic_label = Some(label);
                    }
                }
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
                self.gl.uniform_1_f32(opacity_loc.as_ref(), opacity);
                for (index, location) in water_wave_locs.iter().enumerate() {
                    if let Some(wave) = draw.quad.water_waves.get(index) {
                        self.gl.uniform_4_f32(
                            location.as_ref(),
                            wave.direction,
                            wave.scale,
                            wave.speed,
                            wave.strength,
                        );
                    } else {
                        self.gl.uniform_4_f32(location.as_ref(), 0.0, 1.0, 0.0, 0.0);
                    }
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
                if let Some(iris) = &draw.quad.iris {
                    self.gl.uniform_4_f32(
                        iris_loc.as_ref(),
                        iris.speed,
                        iris.roughness,
                        iris.noise_amount,
                        iris.phase,
                    );
                    self.gl
                        .uniform_2_f32(iris_scale_loc.as_ref(), iris.scale.x, iris.scale.y);
                } else {
                    self.gl.uniform_4_f32(iris_loc.as_ref(), 0.0, 0.2, 0.0, 0.0);
                    self.gl.uniform_2_f32(iris_scale_loc.as_ref(), 1.0, 1.0);
                }
                set_foliage_uniforms(
                    &self.gl,
                    draw.quad.foliage_sway.first(),
                    foliage0_loc.as_ref(),
                    foliage0_extra_loc.as_ref(),
                );
                set_foliage_uniforms(
                    &self.gl,
                    draw.quad.foliage_sway.get(1),
                    foliage1_loc.as_ref(),
                    foliage1_extra_loc.as_ref(),
                );
                if let Some(shine) = &draw.quad.shine {
                    self.gl.uniform_4_f32(
                        shine_loc.as_ref(),
                        shine.direction,
                        shine.speed,
                        shine.intensity,
                        shine.length,
                    );
                    self.gl.uniform_3_f32(
                        shine_color_loc.as_ref(),
                        shine.color.x,
                        shine.color.y,
                        shine.color.z,
                    );
                } else {
                    self.gl
                        .uniform_4_f32(shine_loc.as_ref(), 0.0, 0.0, 0.0, 0.1);
                    self.gl
                        .uniform_3_f32(shine_color_loc.as_ref(), 1.0, 1.0, 1.0);
                }
                for index in 0..3 {
                    if let Some(shake) = draw.quad.shakes.get(index) {
                        self.gl.uniform_4_f32(
                            shake_locs[index].as_ref(),
                            shake.speed,
                            shake.strength,
                            shake.bounds.x,
                            shake.bounds.y,
                        );
                        self.gl.uniform_2_f32(
                            shake_friction_locs[index].as_ref(),
                            shake.friction.x,
                            shake.friction.y,
                        );
                    } else {
                        self.gl
                            .uniform_4_f32(shake_locs[index].as_ref(), 0.0, 0.0, 0.0, 1.0);
                        self.gl
                            .uniform_2_f32(shake_friction_locs[index].as_ref(), 1.0, 1.0);
                    }
                    if let Some(pulse) = draw.quad.pulses.get(index) {
                        self.gl.uniform_4_f32(
                            pulse_locs[index].as_ref(),
                            pulse.speed,
                            pulse.phase,
                            pulse.amount,
                            pulse.power,
                        );
                        self.gl.uniform_2_f32(
                            pulse_bounds_locs[index].as_ref(),
                            pulse.bounds.x,
                            pulse.bounds.y,
                        );
                        self.gl.uniform_3_f32(
                            pulse_low_locs[index].as_ref(),
                            pulse.tint_low.x,
                            pulse.tint_low.y,
                            pulse.tint_low.z,
                        );
                        self.gl.uniform_3_f32(
                            pulse_high_locs[index].as_ref(),
                            pulse.tint_high.x,
                            pulse.tint_high.y,
                            pulse.tint_high.z,
                        );
                    } else {
                        self.gl
                            .uniform_4_f32(pulse_locs[index].as_ref(), 0.0, 0.0, 0.0, 1.0);
                        self.gl
                            .uniform_2_f32(pulse_bounds_locs[index].as_ref(), 0.0, 1.0);
                        self.gl
                            .uniform_3_f32(pulse_low_locs[index].as_ref(), 1.0, 1.0, 1.0);
                        self.gl
                            .uniform_3_f32(pulse_high_locs[index].as_ref(), 1.0, 1.0, 1.0);
                    }
                }
                if let Some(spin) = draw.quad.spin {
                    self.gl.uniform_3_f32(
                        spin_loc.as_ref(),
                        spin.speed,
                        spin.center.x,
                        spin.center.y,
                    );
                    self.gl.uniform_1_f32(has_spin_loc.as_ref(), 1.0);
                } else {
                    self.gl.uniform_3_f32(spin_loc.as_ref(), 0.0, 0.5, 0.5);
                    self.gl.uniform_1_f32(has_spin_loc.as_ref(), 0.0);
                }
                bind_optional_texture(
                    &self.gl,
                    &self.textures,
                    glow::TEXTURE1,
                    draw.water_wave_masks
                        .first()
                        .and_then(Option::as_ref)
                        .map(|texture| texture.path.as_str()),
                    has_water_wave_mask_locs[0].as_ref(),
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
                    draw.water_wave_normals
                        .first()
                        .and_then(Option::as_ref)
                        .map(|texture| texture.path.as_str()),
                    has_water_wave_normal_loc.as_ref(),
                    true,
                );
                bind_optional_texture(
                    &self.gl,
                    &self.textures,
                    glow::TEXTURE5,
                    draw.iris_mask.as_ref().map(|texture| texture.path.as_str()),
                    has_iris_mask_loc.as_ref(),
                    false,
                );
                bind_optional_texture(
                    &self.gl,
                    &self.textures,
                    glow::TEXTURE6,
                    draw.foliage_masks
                        .first()
                        .and_then(Option::as_ref)
                        .map(|texture| texture.path.as_str()),
                    has_foliage_mask0_loc.as_ref(),
                    false,
                );
                bind_optional_texture(
                    &self.gl,
                    &self.textures,
                    glow::TEXTURE7,
                    draw.foliage_masks
                        .get(1)
                        .and_then(Option::as_ref)
                        .map(|texture| texture.path.as_str()),
                    has_foliage_mask1_loc.as_ref(),
                    false,
                );
                bind_optional_texture(
                    &self.gl,
                    &self.textures,
                    glow::TEXTURE8,
                    draw.shine_mask
                        .as_ref()
                        .map(|texture| texture.path.as_str()),
                    has_shine_mask_loc.as_ref(),
                    false,
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
                for (index, unit) in [glow::TEXTURE9, glow::TEXTURE10].into_iter().enumerate() {
                    bind_optional_texture(
                        &self.gl,
                        &self.textures,
                        unit,
                        draw.water_wave_masks
                            .get(index + 1)
                            .and_then(Option::as_ref)
                            .map(|texture| texture.path.as_str()),
                        has_water_wave_mask_locs[index + 1].as_ref(),
                        false,
                    );
                }
                for (index, unit) in [glow::TEXTURE11, glow::TEXTURE12, glow::TEXTURE3]
                    .into_iter()
                    .enumerate()
                {
                    bind_optional_texture(
                        &self.gl,
                        &self.textures,
                        unit,
                        draw.shake_maps
                            .get(index)
                            .and_then(Option::as_ref)
                            .map(|texture| texture.path.as_str()),
                        has_shake_map_locs[index].as_ref(),
                        false,
                    );
                }
                for (index, unit) in [glow::TEXTURE13, glow::TEXTURE14, glow::TEXTURE15]
                    .into_iter()
                    .enumerate()
                {
                    bind_optional_texture(
                        &self.gl,
                        &self.textures,
                        unit,
                        draw.pulse_masks
                            .get(index)
                            .and_then(Option::as_ref)
                            .map(|texture| texture.path.as_str()),
                        has_pulse_mask_locs[index].as_ref(),
                        false,
                    );
                }
                self.gl.active_texture(glow::TEXTURE0);
                let mut uv = draw
                    .animation
                    .as_ref()
                    .map_or(draw.uv, |animation| animation.uv_at(elapsed_seconds));
                let texture_uv_scale = [uv[2] - uv[0], uv[3] - uv[1]];
                self.gl.uniform_2_f32(
                    texture_uv_scale_loc.as_ref(),
                    texture_uv_scale[0],
                    texture_uv_scale[1],
                );
                self.gl.uniform_1_f32(
                    spin_aspect_loc.as_ref(),
                    state.width as f32 * texture_uv_scale[0].abs()
                        / (state.height.max(1) as f32 * texture_uv_scale[1].abs().max(0.0001)),
                );
                if let Some(scroll) = draw.quad.scroll {
                    uv = scroll_uv(uv, scroll, elapsed_seconds);
                }
                let mut vertices = if let Some(mesh) = &draw.mesh {
                    puppet_vertices(mesh.positions_at(elapsed_seconds), &mesh.uv, uv)
                } else {
                    scene_vertices(&draw.quad.vertices, uv).to_vec()
                };
                if draw.quad.dynamic_scale == Some(DynamicScaleKind::ClockSecondX) {
                    apply_clock_second_scale(&mut vertices, local_date_time().second as f32 / 60.0);
                }
                if let Some(response) = draw.quad.audio_response {
                    let amplitude = assets
                        .audio_spectrum
                        .as_ref()
                        .map_or(0.0, |spectrum| spectrum.get(response.bin));
                    apply_centered_y_scale(&mut vertices, amplitude * response.height_scale);
                }
                let vertex_bytes: &[u8] = std::slice::from_raw_parts(
                    vertices.as_ptr().cast::<u8>(),
                    vertices.len() * std::mem::size_of::<f32>(),
                );
                self.gl
                    .buffer_data_u8_slice(glow::ARRAY_BUFFER, vertex_bytes, glow::DYNAMIC_DRAW);
                self.gl
                    .vertex_attrib_pointer_f32(position_loc, 2, glow::FLOAT, false, 24, 0);
                self.gl
                    .vertex_attrib_pointer_f32(tex_coord_loc, 2, glow::FLOAT, false, 24, 8);
                self.gl
                    .vertex_attrib_pointer_f32(effect_coord_loc, 2, glow::FLOAT, false, 24, 16);
                if let Some(mesh) = &draw.mesh {
                    let index_bytes = std::slice::from_raw_parts(
                        mesh.indices.as_ptr().cast::<u8>(),
                        mesh.indices.len() * std::mem::size_of::<u16>(),
                    );
                    self.gl
                        .bind_buffer(glow::ELEMENT_ARRAY_BUFFER, Some(self.index_buffer));
                    self.gl.buffer_data_u8_slice(
                        glow::ELEMENT_ARRAY_BUFFER,
                        index_bytes,
                        glow::STATIC_DRAW,
                    );
                    self.gl.draw_elements(
                        glow::TRIANGLES,
                        mesh.indices.len() as i32,
                        glow::UNSIGNED_SHORT,
                        0,
                    );
                } else {
                    self.gl.bind_buffer(glow::ELEMENT_ARRAY_BUFFER, None);
                    self.gl.draw_arrays(glow::TRIANGLE_FAN, 0, 4);
                }
                drawn += 1;
            }
            self.gl.disable(glow::BLEND);
            self.gl.disable_vertex_attrib_array(position_loc);
            self.gl.disable_vertex_attrib_array(tex_coord_loc);
            self.gl.disable_vertex_attrib_array(effect_coord_loc);
            self.gl.bind_buffer(glow::ARRAY_BUFFER, None);
            self.gl.bind_buffer(glow::ELEMENT_ARRAY_BUFFER, None);
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LocalDateTime {
    year: i32,
    month: i32,
    day: i32,
    weekday: i32,
    hour: i32,
    minute: i32,
    second: i32,
}

fn local_date_time() -> LocalDateTime {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs() as libc::time_t);
    let timestamp = seconds;
    let mut local = std::mem::MaybeUninit::<libc::tm>::uninit();
    let result = unsafe { libc::localtime_r(&timestamp, local.as_mut_ptr()) };
    if result.is_null() {
        return LocalDateTime {
            year: 1970,
            month: 1,
            day: 1,
            weekday: 4,
            hour: 0,
            minute: 0,
            second: 0,
        };
    }
    let local = unsafe { local.assume_init() };
    LocalDateTime {
        year: local.tm_year + 1900,
        month: local.tm_mon + 1,
        day: local.tm_mday,
        weekday: local.tm_wday,
        hour: local.tm_hour,
        minute: local.tm_min,
        second: local.tm_sec,
    }
}

fn dynamic_text_label(kind: DynamicTextKind, local: LocalDateTime) -> String {
    match kind {
        DynamicTextKind::Clock => format!("{:02}:{:02}", local.hour, local.minute),
        DynamicTextKind::Date => {
            let weekday = [
                "星期日",
                "星期一",
                "星期二",
                "星期三",
                "星期四",
                "星期五",
                "星期六",
            ]
            .get(local.weekday as usize)
            .copied()
            .unwrap_or("星期日");
            format!(
                "{:04} | {:02} | {:02} | {weekday}",
                local.year, local.month, local.day
            )
        }
        DynamicTextKind::Media => String::new(),
    }
}

fn apply_clock_second_scale(vertices: &mut [f32], fraction: f32) {
    if vertices.len() < 24 {
        return;
    }
    let fraction = fraction.clamp(0.0, 1.0);
    for (left, right) in [(0, 6), (18, 12)] {
        let left_x = vertices[left];
        let left_y = vertices[left + 1];
        vertices[right] = left_x + (vertices[right] - left_x) * fraction;
        vertices[right + 1] = left_y + (vertices[right + 1] - left_y) * fraction;
    }
}

fn apply_centered_y_scale(vertices: &mut [f32], scale: f32) {
    if vertices.len() < 24 {
        return;
    }
    let center = (vertices[1] + vertices[13]) * 0.5;
    for index in [1, 7, 13, 19] {
        vertices[index] = center + (vertices[index] - center) * scale.max(0.0);
    }
}

unsafe fn set_foliage_uniforms(
    gl: &glow::Context,
    effect: Option<&better_wallpaper_scene_format::FoliageSwayEffect>,
    effect_location: Option<&glow::UniformLocation>,
    extra_location: Option<&glow::UniformLocation>,
) {
    unsafe {
        if let Some(effect) = effect {
            gl.uniform_4_f32(
                effect_location,
                effect.direction,
                effect.scale,
                effect.speed,
                effect.strength,
            );
            gl.uniform_3_f32(extra_location, effect.phase, effect.power, effect.ratio);
        } else {
            gl.uniform_4_f32(effect_location, 0.0, 0.05, 0.0, 0.0);
            gl.uniform_3_f32(extra_location, 0.5, 1.0, 0.3);
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
        uv[0] + phase_x * scroll.repeat_x * width,
        uv[1] + phase_y * scroll.repeat_y * height,
        uv[0] + (phase_x + 1.0) * scroll.repeat_x * width,
        uv[1] + (phase_y + 1.0) * scroll.repeat_y * height,
    ]
}

fn scene_vertices(positions: &[[f32; 2]; 4], uv: [f32; 4]) -> [f32; 24] {
    let [left, top, right, bottom] = uv;
    let tex_coords = [[left, top], [right, top], [right, bottom], [left, bottom]];
    let effect_coords = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
    let mut vertices = [0.0; 24];
    for (index, ((position, tex_coord), effect_coord)) in positions
        .iter()
        .zip(tex_coords)
        .zip(effect_coords)
        .enumerate()
    {
        let offset = index * 6;
        vertices[offset..offset + 2].copy_from_slice(position);
        vertices[offset + 2..offset + 4].copy_from_slice(&tex_coord);
        vertices[offset + 4..offset + 6].copy_from_slice(&effect_coord);
    }
    vertices
}

fn puppet_vertices(
    positions: &[[f32; 2]],
    logical_uv: &[[f32; 2]],
    storage_uv: [f32; 4],
) -> Vec<f32> {
    let width = storage_uv[2] - storage_uv[0];
    let height = storage_uv[3] - storage_uv[1];
    let mut vertices = Vec::with_capacity(positions.len().min(logical_uv.len()) * 6);
    for (position, effect_coord) in positions.iter().zip(logical_uv) {
        vertices.extend_from_slice(position);
        vertices.push(storage_uv[0] + effect_coord[0] * width);
        vertices.push(storage_uv[1] + effect_coord[1] * height);
        vertices.extend_from_slice(effect_coord);
    }
    vertices
}

/// Correct a plan built before connecting to Wayland for the output's actual
/// aspect ratio. Absolute resolution changes with the same aspect remain 1:1
/// in NDC; only an aspect change needs an additional cover transform.
fn scene_layout_scale(assets: &Scene2dAssets, output_size: (u32, u32)) -> [f32; 2] {
    let [projection_width, projection_height] = assets.projection_size;
    let [layout_width, layout_height] = assets.layout_viewport;
    let output_width = output_size.0.max(1) as f32;
    let output_height = output_size.1.max(1) as f32;
    let layout_width = layout_width.max(1) as f32;
    let layout_height = layout_height.max(1) as f32;
    let layout_cover = (layout_width / projection_width).max(layout_height / projection_height);
    let output_cover = (output_width / projection_width).max(output_height / projection_height);
    [
        (output_cover / output_width) / (layout_cover / layout_width),
        (output_cover / output_height) / (layout_cover / layout_height),
    ]
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

fn mip_chain_is_contiguous(image: &TextureImage) -> bool {
    image.levels.windows(2).all(|levels| {
        levels[1].width == (levels[0].width / 2).max(1)
            && levels[1].height == (levels[0].height / 2).max(1)
    })
}

fn first_supported_mip(image: &TextureImage, max_texture_size: u32) -> Option<usize> {
    image
        .levels
        .iter()
        .position(|level| level.width <= max_texture_size && level.height <= max_texture_size)
}

fn mip_chain_is_complete(image: &TextureImage) -> bool {
    mip_chain_is_contiguous(image)
        && image
            .levels
            .last()
            .is_some_and(|level| level.width == 1 && level.height == 1)
}

fn downsample_rgba8(data: &[u8], width: u32, height: u32) -> (u32, u32, Vec<u8>) {
    let next_width = (width / 2).max(1);
    let next_height = (height / 2).max(1);
    let mut output = Vec::with_capacity((next_width * next_height * 4) as usize);
    for y in 0..next_height {
        for x in 0..next_width {
            let mut sums = [0_u32; 4];
            let mut samples = 0_u32;
            let start_x = x * width / next_width;
            let end_x = (x + 1) * width / next_width;
            let start_y = y * height / next_height;
            let end_y = (y + 1) * height / next_height;
            for source_y in start_y..end_y {
                for source_x in start_x..end_x {
                    let offset = ((source_y * width + source_x) * 4) as usize;
                    if let Some(pixel) = data.get(offset..offset + 4) {
                        for channel in 0..4 {
                            sums[channel] += u32::from(pixel[channel]);
                        }
                        samples += 1;
                    }
                }
            }
            for sum in sums {
                output.push((sum / samples.max(1)) as u8);
            }
        }
    }
    (next_width, next_height, output)
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
            self.gl.delete_buffer(self.index_buffer);
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
    use std::sync::Arc;

    use super::{
        LocalDateTime, apply_centered_y_scale, apply_clock_second_scale, convert_to_rgba8,
        downsample_rgba8, dynamic_text_label, first_supported_mip, puppet_vertices,
        scene_layout_scale, scene_vertices, scroll_uv,
    };
    use better_wallpaper_renderer::Scene2dAssets;
    use better_wallpaper_scene_format::{
        DynamicTextKind, ScrollEffect, TexFormat, TextureAlphaMode, TextureColorSpace,
        TextureImage, TextureMipLevel,
    };

    #[test]
    fn interleaves_scene_positions_with_storage_and_logical_coordinates() {
        let positions = [[-1.0, -0.5], [0.5, -0.5], [0.5, 1.0], [-1.0, 1.0]];
        assert_eq!(
            scene_vertices(&positions, [0.0, 0.0, 1.0, 1.0]),
            [
                -1.0, -0.5, 0.0, 0.0, 0.0, 0.0, 0.5, -0.5, 1.0, 0.0, 1.0, 0.0, 0.5, 1.0, 1.0, 1.0,
                1.0, 1.0, -1.0, 1.0, 0.0, 1.0, 0.0, 1.0,
            ]
        );
    }

    #[test]
    fn maps_a_sprite_subrectangle_to_quad_uvs() {
        let positions = [[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]];
        let vertices = scene_vertices(&positions, [0.25, 0.5, 0.5, 1.0]);
        assert_eq!(&vertices[2..4], &[0.25, 0.5]);
        assert_eq!(&vertices[14..16], &[0.5, 1.0]);
        assert_eq!(&vertices[4..6], &[0.0, 0.0]);
        assert_eq!(&vertices[16..18], &[1.0, 1.0]);
    }

    #[test]
    fn formats_native_clock_and_date_labels() {
        let local = LocalDateTime {
            year: 2026,
            month: 7,
            day: 18,
            weekday: 6,
            hour: 9,
            minute: 47,
            second: 30,
        };
        assert_eq!(dynamic_text_label(DynamicTextKind::Clock, local), "09:47");
        assert_eq!(
            dynamic_text_label(DynamicTextKind::Date, local),
            "2026 | 07 | 18 | 星期六"
        );
    }

    #[test]
    fn clock_second_scale_anchors_the_left_edge() {
        let positions = [[-1.0, -0.1], [1.0, -0.1], [1.0, 0.1], [-1.0, 0.1]];
        let mut vertices = scene_vertices(&positions, [0.0, 0.0, 1.0, 1.0]);
        apply_clock_second_scale(&mut vertices, 0.25);
        assert_eq!(&vertices[0..2], &[-1.0, -0.1]);
        assert_eq!(&vertices[6..8], &[-0.5, -0.1]);
        assert_eq!(&vertices[12..14], &[-0.5, 0.1]);
        assert_eq!(&vertices[18..20], &[-1.0, 0.1]);
    }

    #[test]
    fn audio_response_scales_a_quad_around_its_center() {
        let positions = [[-1.0, -0.1], [1.0, -0.1], [1.0, 0.1], [-1.0, 0.1]];
        let mut vertices = scene_vertices(&positions, [0.0, 0.0, 1.0, 1.0]);
        apply_centered_y_scale(&mut vertices, 2.0);
        assert!((vertices[1] + 0.2).abs() < 0.0001);
        assert!((vertices[7] + 0.2).abs() < 0.0001);
        assert!((vertices[13] - 0.2).abs() < 0.0001);
        assert!((vertices[19] - 0.2).abs() < 0.0001);
    }

    #[test]
    fn maps_puppet_logical_uvs_into_padded_texture_storage() {
        let vertices = puppet_vertices(
            &[[-0.5, 0.25], [0.75, -0.25]],
            &[[0.0, 0.0], [1.0, 1.0]],
            [0.0, 0.0, 0.898_437_5, 0.747_070_3],
        );
        assert_eq!(&vertices[0..6], &[-0.5, 0.25, 0.0, 0.0, 0.0, 0.0]);
        assert_eq!(
            &vertices[6..12],
            &[0.75, -0.25, 0.898_437_5, 0.747_070_3, 1.0, 1.0]
        );
    }

    #[test]
    fn updates_cover_transform_for_the_actual_output_aspect_ratio() {
        let assets = Scene2dAssets {
            draws: Vec::new(),
            skipped_nodes: 0,
            projection_size: [3840.0, 2160.0],
            layout_viewport: [1920, 1080],
            camera_zoom: 1.0,
            camera_zoom_animation: None,
            audio_spectrum: None,
        };
        assert_eq!(scene_layout_scale(&assets, (3840, 2160)), [1.0, 1.0]);

        let squareish = scene_layout_scale(&assets, (1280, 1024));
        assert!((squareish[0] - 1.422_222_3).abs() < 0.0001);
        assert!((squareish[1] - 1.0).abs() < 0.0001);
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
            [1.0, 0.0, 3.0, 1.0]
        );
    }

    #[test]
    fn downsamples_high_resolution_scene_textures_with_box_filtering() {
        let pixels = [
            0, 0, 0, 255, 100, 0, 0, 255, 0, 100, 0, 255, 100, 100, 0, 255,
        ];
        let (width, height, downsampled) = downsample_rgba8(&pixels, 2, 2);
        assert_eq!((width, height), (1, 1));
        assert_eq!(downsampled, [50, 50, 0, 255]);

        let odd_pixels = [0, 0, 0, 255, 90, 0, 0, 255, 180, 0, 0, 255];
        let (_, _, odd_downsampled) = downsample_rgba8(&odd_pixels, 3, 1);
        assert_eq!(odd_downsampled, [90, 0, 0, 255]);
    }

    #[test]
    fn selects_a_smaller_authored_mip_for_limited_gpus() {
        let image = TextureImage {
            format: TexFormat::RGBA8888,
            color_space: TextureColorSpace::Unknown,
            alpha_mode: TextureAlphaMode::Unknown,
            levels: [(5760, 2880), (2880, 1440), (1440, 720)]
                .into_iter()
                .map(|(width, height)| TextureMipLevel {
                    width,
                    height,
                    data: Arc::from([]),
                })
                .collect(),
        };
        assert_eq!(first_supported_mip(&image, 4096), Some(1));
        assert_eq!(first_supported_mip(&image, 1024), None);
    }
}
