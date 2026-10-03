mod cuda_gl;
mod scene_renderer;

use std::num::NonZeroU32;

use better_wallpaper_core::config::FillMode;
use glow::HasContext;
use tracing::debug;

const VERTEX_SHADER: &str =
    "attribute vec2 p;attribute vec2 t;varying vec2 u;void main(){gl_Position=vec4(p,0.,1.);u=t;}";
const FRAGMENT_SHADER: &str = "precision mediump float;varying vec2 u;uniform sampler2D v;void main(){gl_FragColor=texture2D(v,u);}";
const YUV_FRAGMENT_SHADER: &str = "precision mediump float;varying vec2 u;uniform sampler2D y_tex;uniform sampler2D uv_tex;void main(){float y=1.1643*(texture2D(y_tex,u).r-.0625);vec2 c=texture2D(uv_tex,u).rg-vec2(.5);gl_FragColor=vec4(y+1.7927*c.y,y-.2132*c.x-.5329*c.y,y+2.1124*c.x,1.);}";
const PBO_RING_SIZE: usize = 3;

pub use scene_renderer::SceneGpuRenderer;

struct GlStateGuard {
    program: Option<glow::Program>,
    array_buffer: Option<glow::Buffer>,
    texture: Option<glow::Texture>,
    active_texture: i32,
    blend_enabled: bool,
}

pub struct GpuRenderer {
    gl: glow::Context,
    program: glow::Program,
    yuv_program: glow::Program,
    vbo: glow::Buffer,
    texture: glow::Texture,
    pbo_ring: Vec<glow::Buffer>,
    pbo_index: usize,
    texture_size: (u32, u32),
    output_size: (u32, u32),
    cuda: Option<cuda_gl::CudaGl>,
    cuda_textures: Option<(glow::Texture, glow::Texture)>,
}

impl GpuRenderer {
    pub fn new(gl: glow::Context, output_width: u32, output_height: u32) -> Result<Self, String> {
        unsafe {
            let program = compile_program(&gl)?;
            let yuv_program = compile_program_with_fragment(&gl, YUV_FRAGMENT_SHADER)?;
            let vbo = gl
                .create_buffer()
                .map_err(|msg| format!("VBO allocate: {msg}"))?;
            let texture = gl
                .create_texture()
                .map_err(|msg| format!("texture allocate: {msg}"))?;
            gl.bind_texture(glow::TEXTURE_2D, Some(texture));
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_MIN_FILTER,
                glow::LINEAR as i32,
            );
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_MAG_FILTER,
                glow::LINEAR as i32,
            );
            let pbo_ring: Vec<glow::Buffer> = (0..PBO_RING_SIZE)
                .map(|_| {
                    gl.create_buffer()
                        .map_err(|msg| format!("PBO allocate: {msg}"))
                })
                .collect::<Result<_, _>>()?;
            let cuda = cuda_gl::CudaGl::new(&gl).ok();
            Ok(Self {
                gl,
                program,
                yuv_program,
                vbo,
                texture,
                pbo_ring,
                pbo_index: 0,
                texture_size: (0, 0),
                output_size: (output_width, output_height),
                cuda,
                cuda_textures: None,
            })
        }
    }

    pub fn upload_frame(
        &mut self,
        data: &[u8],
        width: u32,
        height: u32,
        stride: u32,
    ) -> Result<(), String> {
        if stride != width * 4 {
            return Err(format!(
                "gpu upload requires compact RGBA stride, got stride={stride} width={width}"
            ));
        }
        let expected = (width as usize) * (height as usize) * 4;
        if data.len() < expected {
            return Err(format!(
                "gpu upload frame data too short: {} bytes, need {expected}",
                data.len()
            ));
        }
        let guard = GlStateGuard::capture(&self.gl);
        self.cuda_textures = None;
        unsafe {
            self.gl.bind_texture(glow::TEXTURE_2D, Some(self.texture));
            if self.texture_size != (width, height) {
                self.gl.bind_buffer(glow::PIXEL_UNPACK_BUFFER, None);
                self.gl.tex_image_2d(
                    glow::TEXTURE_2D,
                    0,
                    glow::RGBA as i32,
                    width as i32,
                    height as i32,
                    0,
                    glow::RGBA,
                    glow::UNSIGNED_BYTE,
                    None,
                );
                self.texture_size = (width, height);
                debug!(width, height, "gpu texture reallocated");
            }
            let pbo = self.pbo_ring[self.pbo_index];
            self.pbo_index = (self.pbo_index + 1) % self.pbo_ring.len();
            self.gl.bind_buffer(glow::PIXEL_UNPACK_BUFFER, Some(pbo));
            self.gl.buffer_data_size(
                glow::PIXEL_UNPACK_BUFFER,
                expected as i32,
                glow::STREAM_DRAW,
            );
            self.gl
                .buffer_sub_data_u8_slice(glow::PIXEL_UNPACK_BUFFER, 0, &data[..expected]);
            self.gl.tex_sub_image_2d(
                glow::TEXTURE_2D,
                0,
                0,
                0,
                width as i32,
                height as i32,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::BufferOffset(0),
            );
            self.gl.bind_buffer(glow::PIXEL_UNPACK_BUFFER, None);
        }
        guard.restore(&self.gl);
        Ok(())
    }

    pub fn upload_cuda_frame(
        &mut self,
        frame: &better_wallpaper_core::CudaFrame,
        width: u32,
        height: u32,
    ) -> Result<(), String> {
        let cuda = self
            .cuda
            .as_mut()
            .ok_or("CUDA OpenGL interop unavailable")?;
        self.cuda_textures = Some(unsafe { cuda.upload(&self.gl, frame, width, height)? });
        self.texture_size = (width, height);
        Ok(())
    }

    pub fn draw(&self, output_width: u32, output_height: u32, fill: FillMode) {
        let vertices = fill_vertices(
            self.texture_size.0,
            self.texture_size.1,
            output_width,
            output_height,
            fill,
        );
        let guard = GlStateGuard::capture(&self.gl);
        unsafe {
            let vertex_bytes: &[u8] = std::slice::from_raw_parts(
                vertices.as_ptr().cast::<u8>(),
                vertices.len() * std::mem::size_of::<f32>(),
            );
            let program = if self.cuda_textures.is_some() {
                self.yuv_program
            } else {
                self.program
            };
            self.gl.use_program(Some(program));
            self.gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.vbo));
            self.gl
                .buffer_data_u8_slice(glow::ARRAY_BUFFER, vertex_bytes, glow::DYNAMIC_DRAW);
            for (index, offset) in [(0, 0), (1, 8)] {
                self.gl.enable_vertex_attrib_array(index);
                self.gl
                    .vertex_attrib_pointer_f32(index, 2, glow::FLOAT, false, 16, offset);
            }
            if let Some((y, uv)) = self.cuda_textures {
                self.gl.active_texture(glow::TEXTURE0);
                self.gl.bind_texture(glow::TEXTURE_2D, Some(y));
                if let Some(location) = self.gl.get_uniform_location(program, "y_tex") {
                    self.gl.uniform_1_i32(Some(&location), 0);
                }
                self.gl.active_texture(glow::TEXTURE1);
                self.gl.bind_texture(glow::TEXTURE_2D, Some(uv));
                if let Some(location) = self.gl.get_uniform_location(program, "uv_tex") {
                    self.gl.uniform_1_i32(Some(&location), 1);
                }
            } else {
                self.gl.bind_texture(glow::TEXTURE_2D, Some(self.texture));
            }
            self.gl
                .viewport(0, 0, output_width as i32, output_height as i32);
            self.gl.clear_color(0.0, 0.0, 0.0, 1.0);
            self.gl.clear(glow::COLOR_BUFFER_BIT);
            self.gl.draw_arrays(glow::TRIANGLE_STRIP, 0, 4);
        }
        guard.restore(&self.gl);
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        self.output_size = (width, height);
    }
}

impl Drop for GpuRenderer {
    fn drop(&mut self) {
        unsafe {
            if let Some(cuda) = self.cuda.as_mut() {
                cuda.destroy(&self.gl);
            }
            self.gl.delete_texture(self.texture);
            for pbo in self.pbo_ring.drain(..) {
                self.gl.delete_buffer(pbo);
            }
            self.gl.delete_buffer(self.vbo);
            self.gl.delete_program(self.program);
            self.gl.delete_program(self.yuv_program);
        }
        debug!("gpu renderer resources released");
    }
}

impl GlStateGuard {
    fn capture(gl: &glow::Context) -> Self {
        unsafe {
            let program = gl.get_parameter_i32(glow::CURRENT_PROGRAM);
            let array_buffer = gl.get_parameter_i32(glow::ARRAY_BUFFER_BINDING);
            let texture = gl.get_parameter_i32(glow::TEXTURE_BINDING_2D);
            let active_texture = gl.get_parameter_i32(glow::ACTIVE_TEXTURE);
            let blend = gl.is_enabled(glow::BLEND);
            gl.disable(glow::BLEND);
            Self {
                program: NonZeroU32::new(program as u32).map(glow::NativeProgram),
                array_buffer: NonZeroU32::new(array_buffer as u32).map(glow::NativeBuffer),
                texture: NonZeroU32::new(texture as u32).map(glow::NativeTexture),
                active_texture,
                blend_enabled: blend,
            }
        }
    }
    fn restore(self, gl: &glow::Context) {
        unsafe {
            gl.use_program(self.program);
            gl.bind_buffer(glow::ARRAY_BUFFER, self.array_buffer);
            gl.bind_texture(glow::TEXTURE_2D, self.texture);
            gl.active_texture(self.active_texture as u32);
            if self.blend_enabled {
                gl.enable(glow::BLEND);
            }
        }
    }
}

unsafe fn compile_program(gl: &glow::Context) -> Result<glow::Program, String> {
    unsafe { compile_program_with_fragment(gl, FRAGMENT_SHADER) }
}

unsafe fn compile_program_with_fragment(
    gl: &glow::Context,
    fragment: &str,
) -> Result<glow::Program, String> {
    unsafe {
        let program = gl
            .create_program()
            .map_err(|msg| format!("program create: {msg}"))?;
        let vert = gl
            .create_shader(glow::VERTEX_SHADER)
            .map_err(|msg| format!("vertex shader create: {msg}"))?;
        let frag = gl
            .create_shader(glow::FRAGMENT_SHADER)
            .map_err(|msg| format!("fragment shader create: {msg}"))?;
        for (shader, source) in [(vert, VERTEX_SHADER), (frag, fragment)] {
            gl.shader_source(shader, source);
            gl.compile_shader(shader);
            if !gl.get_shader_compile_status(shader) {
                let log = gl.get_shader_info_log(shader);
                return Err(format!("shader compile: {log}"));
            }
        }
        gl.attach_shader(program, vert);
        gl.attach_shader(program, frag);
        gl.bind_attrib_location(program, 0, "p");
        gl.bind_attrib_location(program, 1, "t");
        gl.link_program(program);
        if !gl.get_program_link_status(program) {
            let log = gl.get_program_info_log(program);
            return Err(format!("program link: {log}"));
        }
        gl.delete_shader(vert);
        gl.delete_shader(frag);
        Ok(program)
    }
}

fn fill_vertices(sw: u32, sh: u32, tw: u32, th: u32, m: FillMode) -> [f32; 16] {
    let (sr, tr) = (sw as f32 / sh as f32, tw as f32 / th as f32);
    let (mut x, mut y, mut a, mut b, mut c, mut d) = (1.0, 1.0, 0.0, 0.0, 1.0, 1.0);
    match m {
        FillMode::Stretch => {}
        FillMode::Contain if sr > tr => y = tr / sr,
        FillMode::Contain => x = sr / tr,
        FillMode::Cover if sr > tr => {
            a = (1.0 - tr / sr) / 2.0;
            c = 1.0 - a;
        }
        FillMode::Cover => {
            b = (1.0 - sr / tr) / 2.0;
            d = 1.0 - b;
        }
    }
    [-x, -y, a, d, x, -y, c, d, -x, y, a, b, x, y, c, b]
}
