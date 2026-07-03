use better_wallpaper_renderer::Scene2dAssets;
use better_wallpaper_scene_format::BlendMode;
use glow::HasContext;
use std::collections::HashMap;
use tracing::debug;

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
const FRAGMENT_SHADER: &str = "precision mediump float;\nuniform sampler2D u_texture;\nuniform float u_opacity;\nvarying vec2 v_tex_coord;\nvoid main() {\n    vec4 color = texture2D(u_texture, v_tex_coord);\n    gl_FragColor = vec4(color.rgb, color.a * u_opacity);\n}";

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
                if self.textures.contains_key(&draw.texture_path) {
                    continue;
                }
                let texture = self
                    .gl
                    .create_texture()
                    .map_err(|msg| format!("scene texture allocate: {msg}"))?;
                self.gl.bind_texture(glow::TEXTURE_2D, Some(texture));
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
                let base = &draw.texture.levels[0];
                self.gl.tex_image_2d(
                    glow::TEXTURE_2D,
                    0,
                    glow::RGBA as i32,
                    base.width as i32,
                    base.height as i32,
                    0,
                    glow::RGBA,
                    glow::UNSIGNED_BYTE,
                    Some(&base.data),
                );
                for (idx, level) in draw.texture.levels.iter().enumerate().skip(1) {
                    self.gl.tex_image_2d(
                        glow::TEXTURE_2D,
                        idx as i32,
                        glow::RGBA as i32,
                        level.width as i32,
                        level.height as i32,
                        0,
                        glow::RGBA,
                        glow::UNSIGNED_BYTE,
                        Some(&level.data),
                    );
                }
                self.gl.bind_texture(glow::TEXTURE_2D, None);
                #[allow(clippy::collapsible_if)]
                if self.textures.len() >= MAX_GPU_TEXTURES {
                    // evict an existing texture to stay under the GPU texture limit
                    if let Some(key) = self.textures.keys().next().cloned() {
                        if let Some(state) = self.textures.remove(&key) {
                            self.gl.delete_texture(state.texture);
                        }
                    }
                }
                self.textures.insert(
                    draw.texture_path.clone(),
                    GpuTextureState {
                        texture,
                        width: base.width,
                        height: base.height,
                    },
                );
            }
        }
        Ok(())
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        self.output_size = (width, height);
    }

    pub fn draw_scene(&self, assets: &Scene2dAssets) {
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
            self.gl.enable_vertex_attrib_array(position_loc);
            self.gl.enable_vertex_attrib_array(tex_coord_loc);
            self.gl.uniform_1_i32(sampler_loc.as_ref(), 0);
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
                self.gl
                    .uniform_1_f32(opacity_loc.as_ref(), draw.quad.opacity);
                let vertices = scene_vertices(&draw.quad.vertices);
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
            }
            self.gl.disable(glow::BLEND);
            self.gl.disable_vertex_attrib_array(position_loc);
            self.gl.disable_vertex_attrib_array(tex_coord_loc);
            self.gl.bind_buffer(glow::ARRAY_BUFFER, None);
            self.gl.bind_texture(glow::TEXTURE_2D, None);
            self.gl.use_program(None);
        }
    }

    pub fn clear_textures(&mut self) {
        unsafe {
            for (_, state) in self.textures.drain() {
                self.gl.delete_texture(state.texture);
            }
        }
    }
}

fn scene_vertices(positions: &[[f32; 2]; 4]) -> [f32; 16] {
    let tex_coords = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
    let mut vertices = [0.0; 16];
    for (index, (position, tex_coord)) in positions.iter().zip(tex_coords).enumerate() {
        let offset = index * 4;
        vertices[offset..offset + 2].copy_from_slice(position);
        vertices[offset + 2..offset + 4].copy_from_slice(&tex_coord);
    }
    vertices
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
    use super::scene_vertices;

    #[test]
    fn interleaves_scene_positions_with_quad_texture_coordinates() {
        let positions = [[-1.0, -0.5], [0.5, -0.5], [0.5, 1.0], [-1.0, 1.0]];
        assert_eq!(
            scene_vertices(&positions),
            [
                -1.0, -0.5, 0.0, 0.0, 0.5, -0.5, 1.0, 0.0, 0.5, 1.0, 1.0, 1.0, -1.0, 1.0, 0.0, 1.0,
            ]
        );
    }
}
