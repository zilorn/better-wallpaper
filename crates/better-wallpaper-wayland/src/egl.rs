use anyhow::{Result, bail};
use better_wallpaper_core::{DecodedFrame, config::FillMode};
use glow::HasContext;
use gpu_renderer::GpuRenderer;
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
    renderer: GpuRenderer,
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
            .ok_or_else(|| anyhow::anyhow!("eglGetDisplay failed"))?;
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
            .ok_or_else(|| anyhow::anyhow!("no EGL ES2 window config"))?;
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
            bail!("wl_egl_window_create failed");
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
        let version_string = unsafe { gl.get_parameter_string(glow::VERSION) };
        let es3 = version_string.contains("OpenGL ES 3");

        let renderer = GpuRenderer::new(gl, width, height)
            .map_err(|msg| anyhow::anyhow!("gpu renderer: {msg}"))?;

        info!(egl_version=%format!("{}.{}",version.0,version.1), gl_version=%version_string, es3, width, height, "EGL GPU renderer initialised");

        Ok(Self {
            egl,
            display,
            surface,
            context,
            window,
            renderer,
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
        if self.output_size != (width, height) {
            unsafe {
                wl_egl_window_resize(self.window, width as i32, height as i32, 0, 0);
            }
            self.renderer.resize(width, height);
            self.output_size = (width, height);
        }

        let upload_started = Instant::now();
        self.renderer
            .upload_frame(
                &frame.pixels,
                frame.width,
                frame.height,
                frame.stride as u32,
            )
            .map_err(|msg| anyhow::anyhow!("gpu upload: {msg}"))?;
        self.perf_upload += upload_started.elapsed();

        self.renderer.draw(width, height, mode);

        let swap_started = Instant::now();
        self.egl.swap_buffers(self.display, self.surface)?;
        self.perf_swap += swap_started.elapsed();

        self.perf_frames += 1;
        if self.perf_started.elapsed() >= Duration::from_secs(1) {
            let frames = self.perf_frames.max(1) as f64;
            info!(
                frames = self.perf_frames,
                upload_avg_ms = self.perf_upload.as_secs_f64() * 1000.0 / frames,
                swap_avg_ms = self.perf_swap.as_secs_f64() * 1000.0 / frames,
                "niri GPU upload+swap performance"
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
            let _ = self.egl.make_current(self.display, None, None, None);
            let _ = self.egl.destroy_context(self.display, self.context);
            let _ = self.egl.destroy_surface(self.display, self.surface);
            wl_egl_window_destroy(self.window);
            let _ = self.egl.terminate(self.display);
        }
        debug!("EGL GPU renderer released");
    }
}
