use anyhow::{Result, bail};
use better_wallpaper_core::{DecodedFrame, config::FillMode};
use glow::HasContext;
use khronos_egl as egl;
use std::{
    ffi::c_void,
    ptr,
    time::{Duration, Instant},
};
use tracing::{debug, info};

#[link(name = "wayland-egl")]
unsafe extern "C" {
    fn wl_egl_window_create(surface: *mut c_void, width: i32, height: i32) -> *mut c_void;
    fn wl_egl_window_destroy(window: *mut c_void);
    fn wl_egl_window_resize(window: *mut c_void, width: i32, height: i32, dx: i32, dy: i32);
}

pub(crate) struct EglRenderer {
    egl: egl::DynamicInstance<egl::EGL1_4>,
    display: egl::Display,
    surface: egl::Surface,
    context: egl::Context,
    window: *mut c_void,
    gl: glow::Context,
    program: glow::Program,
    buffer: glow::Buffer,
    texture: glow::Texture,
    upload_buffers: Vec<glow::Buffer>,
    upload_index: usize,
    texture_size: (u32, u32),
    output_size: (u32, u32),
    perf_started: Instant,
    perf_frames: u64,
    perf_upload: Duration,
    perf_swap: Duration,
}

impl EglRenderer {
    pub(crate) unsafe fn new(
        display_ptr: *mut c_void,
        surface_ptr: *mut c_void,
        width: u32,
        height: u32,
    ) -> Result<Self> {
        let egl = unsafe { egl::DynamicInstance::<egl::EGL1_4>::load_required()? };
        let display = unsafe { egl.get_display(display_ptr) }
            .ok_or_else(|| anyhow::anyhow!("eglGetDisplay 失败"))?;
        let version = egl.initialize(display)?;
        egl.bind_api(egl::OPENGL_ES_API)?;
        let attrs = [
            egl::SURFACE_TYPE,
            egl::WINDOW_BIT,
            egl::RED_SIZE,
            8,
            egl::GREEN_SIZE,
            8,
            egl::BLUE_SIZE,
            8,
            egl::ALPHA_SIZE,
            8,
            egl::RENDERABLE_TYPE,
            egl::OPENGL_ES2_BIT,
            egl::NONE,
        ];
        let config = egl
            .choose_first_config(display, &attrs)?
            .ok_or_else(|| anyhow::anyhow!("没有 EGL ES2 window config"))?;
        // 优先 ES3 以启用 PIXEL_UNPACK_BUFFER；旧驱动仍可降级 ES2 GPU 绘制。
        let context = egl
            .create_context(
                display,
                config,
                None,
                &[egl::CONTEXT_CLIENT_VERSION, 3, egl::NONE],
            )
            .or_else(|_| {
                egl.create_context(
                    display,
                    config,
                    None,
                    &[egl::CONTEXT_CLIENT_VERSION, 2, egl::NONE],
                )
            })?;
        let window = unsafe { wl_egl_window_create(surface_ptr, width as i32, height as i32) };
        if window.is_null() {
            bail!("wl_egl_window_create 失败");
        }
        let surface = unsafe {
            egl.create_window_surface(display, config, window as egl::NativeWindowType, None)
        }?;
        egl.make_current(display, Some(surface), Some(surface), Some(context))?;
        egl.swap_interval(display, 1)?;
        let gl = unsafe {
            glow::Context::from_loader_function(|name| {
                egl.get_proc_address(name)
                    .map_or(ptr::null(), |p| p as *const _)
            })
        };
        let program = unsafe { program(&gl)? };
        let buffer = unsafe { gl.create_buffer().map_err(anyhow::Error::msg)? };
        let texture = unsafe { gl.create_texture().map_err(anyhow::Error::msg)? };
        let version_string = unsafe { gl.get_parameter_string(glow::VERSION) };
        let upload_buffers = if version_string.contains("OpenGL ES 3") {
            (0..3)
                .map(|_| unsafe { gl.create_buffer().map_err(anyhow::Error::msg) })
                .collect::<Result<Vec<_>>>()?
        } else {
            Vec::new()
        };
        unsafe {
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
        }
        info!(egl_version=%format!("{}.{}",version.0,version.1), gl_version=%version_string, async_pbo=!upload_buffers.is_empty(), width,height,"Wayland EGL GPU 渲染已启用");
        Ok(Self {
            egl,
            display,
            surface,
            context,
            window,
            gl,
            program,
            buffer,
            texture,
            upload_buffers,
            upload_index: 0,
            texture_size: (0, 0),
            output_size: (width, height),
            perf_started: Instant::now(),
            perf_frames: 0,
            perf_upload: Duration::ZERO,
            perf_swap: Duration::ZERO,
        })
    }
    pub(crate) fn render(
        &mut self,
        frame: &DecodedFrame,
        mode: FillMode,
        width: u32,
        height: u32,
    ) -> Result<()> {
        if frame.stride != frame.width as usize * 4 {
            bail!("GPU 上传要求紧凑 RGBA 帧");
        }
        if self.output_size != (width, height) {
            unsafe { wl_egl_window_resize(self.window, width as i32, height as i32, 0, 0) };
            self.output_size = (width, height);
        }
        let v = vertices(frame.width, frame.height, width, height, mode);
        let upload_started = Instant::now();
        unsafe {
            self.gl.use_program(Some(self.program));
            self.gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.buffer));
            self.gl.buffer_data_u8_slice(
                glow::ARRAY_BUFFER,
                std::slice::from_raw_parts(v.as_ptr().cast(), std::mem::size_of_val(&v)),
                glow::DYNAMIC_DRAW,
            );
            for (i, off) in [(0, 0), (1, 8)] {
                self.gl.enable_vertex_attrib_array(i);
                self.gl
                    .vertex_attrib_pointer_f32(i, 2, glow::FLOAT, false, 16, off);
            }
            self.gl.bind_texture(glow::TEXTURE_2D, Some(self.texture));
            if self.texture_size != (frame.width, frame.height) {
                self.gl.bind_buffer(glow::PIXEL_UNPACK_BUFFER, None);
                self.gl.tex_image_2d(
                    glow::TEXTURE_2D,
                    0,
                    glow::RGBA as i32,
                    frame.width as i32,
                    frame.height as i32,
                    0,
                    glow::RGBA,
                    glow::UNSIGNED_BYTE,
                    None,
                );
                self.texture_size = (frame.width, frame.height);
            }
            if !self.upload_buffers.is_empty() {
                let pbo = self.upload_buffers[self.upload_index];
                self.upload_index = (self.upload_index + 1) % self.upload_buffers.len();
                self.gl.bind_buffer(glow::PIXEL_UNPACK_BUFFER, Some(pbo));
                // orphan 旧存储，避免等待仍在使用该 PBO 的上一轮 DMA。
                self.gl.buffer_data_size(
                    glow::PIXEL_UNPACK_BUFFER,
                    frame.pixels.len() as i32,
                    glow::STREAM_DRAW,
                );
                self.gl
                    .buffer_sub_data_u8_slice(glow::PIXEL_UNPACK_BUFFER, 0, &frame.pixels);
                self.gl.tex_sub_image_2d(
                    glow::TEXTURE_2D,
                    0,
                    0,
                    0,
                    frame.width as i32,
                    frame.height as i32,
                    glow::RGBA,
                    glow::UNSIGNED_BYTE,
                    glow::PixelUnpackData::BufferOffset(0),
                );
                self.gl.bind_buffer(glow::PIXEL_UNPACK_BUFFER, None);
            } else {
                self.gl.tex_sub_image_2d(
                    glow::TEXTURE_2D,
                    0,
                    0,
                    0,
                    frame.width as i32,
                    frame.height as i32,
                    glow::RGBA,
                    glow::UNSIGNED_BYTE,
                    glow::PixelUnpackData::Slice(&frame.pixels),
                );
            }
            self.gl.viewport(0, 0, width as i32, height as i32);
            self.gl.clear_color(0., 0., 0., 1.);
            self.gl.clear(glow::COLOR_BUFFER_BIT);
            self.gl.draw_arrays(glow::TRIANGLE_STRIP, 0, 4);
        }
        self.perf_upload += upload_started.elapsed();
        let swap_started = Instant::now();
        self.egl.swap_buffers(self.display, self.surface)?;
        self.perf_swap += swap_started.elapsed();
        self.perf_frames += 1;
        if self.perf_started.elapsed() >= Duration::from_secs(1) {
            let frames = self.perf_frames.max(1) as f64;
            info!(
                frames = self.perf_frames,
                async_pbo = !self.upload_buffers.is_empty(),
                upload_submit_avg_ms = self.perf_upload.as_secs_f64() * 1000.0 / frames,
                swap_avg_ms = self.perf_swap.as_secs_f64() * 1000.0 / frames,
                "EGL GPU 实时上传性能"
            );
            self.perf_started = Instant::now();
            self.perf_frames = 0;
            self.perf_upload = Duration::ZERO;
            self.perf_swap = Duration::ZERO;
        }
        Ok(())
    }
}
impl Drop for EglRenderer {
    fn drop(&mut self) {
        unsafe {
            self.gl.delete_texture(self.texture);
            for buffer in self.upload_buffers.drain(..) {
                self.gl.delete_buffer(buffer);
            }
            self.gl.delete_buffer(self.buffer);
            self.gl.delete_program(self.program);
            let _ = self.egl.make_current(self.display, None, None, None);
            let _ = self.egl.destroy_context(self.display, self.context);
            let _ = self.egl.destroy_surface(self.display, self.surface);
            wl_egl_window_destroy(self.window);
            let _ = self.egl.terminate(self.display);
        }
        debug!("EGL GPU 渲染器已释放");
    }
}

unsafe fn program(gl: &glow::Context) -> Result<glow::Program> {
    unsafe {
        let p = gl.create_program().map_err(anyhow::Error::msg)?;
        for (k, s) in [
            (
                glow::VERTEX_SHADER,
                "attribute vec2 p;attribute vec2 t;varying vec2 u;void main(){gl_Position=vec4(p,0.,1.);u=t;}",
            ),
            (
                glow::FRAGMENT_SHADER,
                "precision mediump float;varying vec2 u;uniform sampler2D v;void main(){gl_FragColor=texture2D(v,u);}",
            ),
        ] {
            let sh = gl.create_shader(k).map_err(anyhow::Error::msg)?;
            gl.shader_source(sh, s);
            gl.compile_shader(sh);
            if !gl.get_shader_compile_status(sh) {
                bail!("shader: {}", gl.get_shader_info_log(sh));
            }
            gl.attach_shader(p, sh);
            gl.delete_shader(sh);
        }
        gl.bind_attrib_location(p, 0, "p");
        gl.bind_attrib_location(p, 1, "t");
        gl.link_program(p);
        if !gl.get_program_link_status(p) {
            bail!("program: {}", gl.get_program_info_log(p));
        }
        Ok(p)
    }
}
fn vertices(sw: u32, sh: u32, tw: u32, th: u32, m: FillMode) -> [f32; 16] {
    let (sr, tr) = (sw as f32 / sh as f32, tw as f32 / th as f32);
    let (mut x, mut y, mut a, mut b, mut c, mut d) = (1., 1., 0., 0., 1., 1.);
    match m {
        FillMode::Stretch => {}
        FillMode::Contain if sr > tr => y = tr / sr,
        FillMode::Contain => x = sr / tr,
        FillMode::Cover if sr > tr => {
            a = (1. - tr / sr) / 2.;
            c = 1. - a
        }
        FillMode::Cover => {
            b = (1. - sr / tr) / 2.;
            d = 1. - b
        }
    }
    [-x, -y, a, d, x, -y, c, d, -x, y, a, b, x, y, c, b]
}
