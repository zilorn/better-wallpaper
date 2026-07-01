use crate::egl::EglRenderer;
use anyhow::{Context, Result, anyhow, bail};
use better_wallpaper_core::{DecodedFrame, config::FillMode};
use better_wallpaper_renderer::Scene2dAssets;
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    delegate_compositor, delegate_layer, delegate_output, delegate_registry, delegate_shm,
    output::{OutputHandler, OutputState},
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    shell::{
        WaylandSurface,
        wlr_layer::{
            Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
            LayerSurfaceConfigure,
        },
    },
    shm::{Shm, ShmHandler, slot::SlotPool},
};
use std::time::{Duration, Instant};
use tracing::{debug, info, warn};
use wayland_client::{
    Connection, EventQueue, Proxy, QueueHandle,
    globals::registry_queue_init,
    protocol::{wl_output, wl_shm, wl_surface},
};

pub struct NiriBackend {
    // 必须先于 Wayland connection/surface 释放。
    egl: Option<EglRenderer>,
    connection: Connection,
    event_queue: EventQueue<NiriState>,
    state: NiriState,
    layer: LayerSurface,
    pool: SlotPool,
    output_name: String,
    scale_plan: Option<ScalePlan>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct PresentMetrics {
    pub frame_callback_wait: Duration,
    pub buffer_allocate: Duration,
    pub scale: Duration,
    pub submit: Duration,
}

impl NiriBackend {
    pub fn connect(target_output: Option<&str>) -> Result<Self> {
        let connection =
            Connection::connect_to_env().context("failed to connect to Wayland compositor")?;
        let (globals, mut event_queue) =
            registry_queue_init(&connection).context("failed to read Wayland globals")?;
        let qh = event_queue.handle();
        let compositor = CompositorState::bind(&globals, &qh)
            .context("compositor did not provide wl_compositor")?;
        let layer_shell = LayerShell::bind(&globals, &qh)
            .context("compositor did not provide wlr layer-shell")?;
        let shm = Shm::bind(&globals, &qh).context("compositor did not provide wl_shm")?;
        let mut state = NiriState {
            registry_state: RegistryState::new(&globals),
            output_state: OutputState::new(&globals, &qh),
            shm,
            configured_size: None,
            closed: false,
            frame_ready: true,
        };
        event_queue
            .roundtrip(&mut state)
            .context("failed to enumerate Wayland outputs")?;

        let outputs: Vec<_> = state
            .output_state
            .outputs()
            .filter_map(|output| state.output_state.info(&output).map(|info| (output, info)))
            .collect();
        for (_, output) in &outputs {
            info!(
                id = output.id,
                name = ?output.name,
                model = %output.model,
                logical_size = ?output.logical_size,
                scale = output.scale_factor,
                "discovered Wayland output"
            );
        }
        let (output, output_info) = outputs
            .into_iter()
            .find(|(_, info)| {
                target_output.is_none_or(|target| info.name.as_deref() == Some(target))
            })
            .ok_or_else(|| match target_output {
                Some(name) => anyhow!("configured Wayland output {name} not found"),
                None => anyhow!("compositor reported no available outputs"),
            })?;
        let output_name = output_info
            .name
            .unwrap_or_else(|| format!("output-{}", output_info.id));

        let surface = compositor.create_surface(&qh);
        let layer = layer_shell.create_layer_surface(
            &qh,
            surface,
            Layer::Background,
            Some("better-wallpaper"),
            Some(&output),
        );
        layer.set_anchor(Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT);
        layer.set_exclusive_zone(-1);
        layer.set_keyboard_interactivity(KeyboardInteractivity::None);
        layer.set_size(0, 0);
        layer.commit();

        while state.configured_size.is_none() && !state.closed {
            event_queue
                .blocking_dispatch(&mut state)
                .context("failed to wait for layer surface configure")?;
        }
        if state.closed {
            bail!("layer surface closed by compositor before first configure");
        }
        let (width, height) = state
            .configured_size
            .expect("configure state already checked");
        let pool = SlotPool::new(width as usize * height as usize * 4, &state.shm)
            .context("failed to create wl_shm buffer pool")?;
        let egl = match unsafe {
            EglRenderer::new(
                connection.backend().display_ptr().cast(),
                layer.wl_surface().id().as_ptr().cast(),
                width,
                height,
            )
        } {
            Ok(renderer) => Some(renderer),
            Err(error) => {
                warn!(output=%output_name,%error,"EGL GPU 初始化失败，回退 wl_shm");
                None
            }
        };
        info!(output = %output_name, width, height, gpu=egl.is_some(), "niri background layer ready");

        Ok(Self {
            egl,
            connection,
            event_queue,
            state,
            layer,
            pool,
            output_name,
            scale_plan: None,
        })
    }

    pub fn output_name(&self) -> &str {
        &self.output_name
    }

    pub fn size(&self) -> (u32, u32) {
        self.state.configured_size.unwrap_or((1, 1))
    }
    pub fn load_scene_assets(&mut self, assets: Scene2dAssets) -> anyhow::Result<()> {
        let egl = self.egl.as_mut().ok_or_else(|| {
            anyhow::anyhow!("EGL not available, cannot load scene textures")
        })?;
        egl.set_scene_assets(assets)?;
        Ok(())
    }

    pub fn present_scene(&mut self) -> anyhow::Result<()> {
        self.dispatch_pending()?;
        let (width, height) = self.size();
        let egl = self.egl.as_mut().ok_or_else(|| {
            anyhow::anyhow!("EGL GPU backend unavailable, scene rendering not supported")
        })?;
        self.state.frame_ready = false;
        self.layer
            .wl_surface()
            .frame(&self.event_queue.handle(), self.layer.wl_surface().clone());
        egl.render_scene(width, height)?;
        Ok(())
    }


    pub fn present(&mut self, frame: &DecodedFrame, fill_mode: FillMode) -> Result<PresentMetrics> {
        self.dispatch_pending()?;
        let wait_started = Instant::now();
        while !self.state.frame_ready && !self.state.closed {
            self.event_queue
                .blocking_dispatch(&mut self.state)
                .context("failed to wait for compositor frame callback")?;
        }
        if self.state.closed {
            bail!("layer surface closed by compositor");
        }
        let frame_callback_wait = wait_started.elapsed();
        let (width, height) = self.size();
        if let Some(egl) = &mut self.egl {
            let submit_started = Instant::now();
            self.state.frame_ready = false;
            self.layer
                .wl_surface()
                .frame(&self.event_queue.handle(), self.layer.wl_surface().clone());
            match egl.render(frame, fill_mode, width, height) {
                Ok(()) => {
                    return Ok(PresentMetrics {
                        frame_callback_wait,
                        submit: submit_started.elapsed(),
                        ..Default::default()
                    });
                }
                Err(error) => {
                    if frame.cuda.is_some() {
                        return Err(error)
                            .context("CUDA OpenGL interop failed for a hardware-decoded frame");
                    }
                    warn!(output=%self.output_name,%error,"EGL GPU 呈现失败，回退 wl_shm");
                    self.egl = None;
                    self.state.frame_ready = true;
                }
            }
        }
        let stride = width.checked_mul(4).context("output stride overflow")?;
        let allocate_started = Instant::now();
        let (buffer, canvas) = self
            .pool
            .create_buffer(
                width as i32,
                height as i32,
                stride as i32,
                wl_shm::Format::Abgr8888,
            )
            .context("failed to allocate wl_shm frame buffer")?;
        let buffer_allocate = allocate_started.elapsed();
        let scale_started = Instant::now();
        let plan = self.scale_plan.get_or_insert_with(|| {
            ScalePlan::new(frame.width, frame.height, width, height, fill_mode)
        });
        if !plan.matches(frame.width, frame.height, width, height, fill_mode) {
            info!(
                output = %self.output_name,
                source_size = ?(frame.width, frame.height),
                target_size = ?(width, height),
                ?fill_mode,
                "video scale parameters changed, rebuilding sample table"
            );
            *plan = ScalePlan::new(frame.width, frame.height, width, height, fill_mode);
        }
        scale_rgba_with_plan(frame, canvas, plan);
        let scale = scale_started.elapsed();
        let submit_started = Instant::now();
        self.layer
            .wl_surface()
            .damage_buffer(0, 0, width as i32, height as i32);
        buffer
            .attach_to(self.layer.wl_surface())
            .context("wl_shm buffer still in use, cannot submit")?;
        self.state.frame_ready = false;
        self.layer
            .wl_surface()
            .frame(&self.event_queue.handle(), self.layer.wl_surface().clone());
        self.layer.commit();
        self.connection
            .flush()
            .context("failed to flush Wayland requests")?;
        let submit = submit_started.elapsed();
        debug!(output = %self.output_name, width, height, pts = frame.pts, "submitted wl_shm video frame");
        Ok(PresentMetrics {
            frame_callback_wait,
            buffer_allocate,
            scale,
            submit,
        })
    }

    pub fn dispatch_pending(&mut self) -> Result<()> {
        self.event_queue
            .dispatch_pending(&mut self.state)
            .context("failed to process Wayland events")?;
        Ok(())
    }
}

#[derive(Debug)]
struct ScalePlan {
    source_width: u32,
    source_height: u32,
    target_width: u32,
    target_height: u32,
    mode: FillMode,
    visible_left: usize,
    visible_top: usize,
    visible_right: usize,
    visible_bottom: usize,
    source_columns: Vec<usize>,
    source_rows: Vec<usize>,
    direct_copy: bool,
}

impl ScalePlan {
    fn new(source_width: u32, source_height: u32, width: u32, height: u32, mode: FillMode) -> Self {
        if source_width == 0 || source_height == 0 || width == 0 || height == 0 {
            return Self {
                source_width,
                source_height,
                target_width: width,
                target_height: height,
                mode,
                visible_left: 0,
                visible_top: 0,
                visible_right: 0,
                visible_bottom: 0,
                source_columns: Vec::new(),
                source_rows: Vec::new(),
                direct_copy: false,
            };
        }
        let source_ratio = source_width as f64 / source_height as f64;
        let target_ratio = width as f64 / height as f64;
        let (draw_width, draw_height) = match mode {
            FillMode::Stretch => (width, height),
            FillMode::Contain if source_ratio > target_ratio => {
                (width, (width as f64 / source_ratio).round() as u32)
            }
            FillMode::Contain => ((height as f64 * source_ratio).round() as u32, height),
            FillMode::Cover if source_ratio > target_ratio => {
                ((height as f64 * source_ratio).round() as u32, height)
            }
            FillMode::Cover => (width, (width as f64 / source_ratio).round() as u32),
        };
        let offset_x = (width as i64 - draw_width as i64) / 2;
        let offset_y = (height as i64 - draw_height as i64) / 2;
        let visible_left = offset_x.max(0) as usize;
        let visible_top = offset_y.max(0) as usize;
        let visible_right = (offset_x + draw_width as i64).min(width as i64).max(0) as usize;
        let visible_bottom = (offset_y + draw_height as i64).min(height as i64).max(0) as usize;
        let source_columns = (visible_left..visible_right)
            .map(|target_x| {
                let draw_x = target_x as i64 - offset_x;
                draw_x as usize * source_width as usize / draw_width as usize * 4
            })
            .collect();
        let source_rows = (visible_top..visible_bottom)
            .map(|target_y| {
                let draw_y = target_y as i64 - offset_y;
                draw_y as usize * source_height as usize / draw_height as usize
            })
            .collect();
        Self {
            source_width,
            source_height,
            target_width: width,
            target_height: height,
            mode,
            visible_left,
            visible_top,
            visible_right,
            visible_bottom,
            source_columns,
            source_rows,
            direct_copy: draw_width == source_width
                && draw_height == source_height
                && offset_x == 0
                && offset_y == 0
                && width == source_width
                && height == source_height,
        }
    }

    fn matches(&self, sw: u32, sh: u32, tw: u32, th: u32, mode: FillMode) -> bool {
        (
            self.source_width,
            self.source_height,
            self.target_width,
            self.target_height,
            self.mode,
        ) == (sw, sh, tw, th, mode)
    }
}

#[cfg(test)]
fn scale_rgba(frame: &DecodedFrame, target: &mut [u8], width: u32, height: u32, mode: FillMode) {
    let plan = ScalePlan::new(frame.width, frame.height, width, height, mode);
    scale_rgba_with_plan(frame, target, &plan);
}

fn scale_rgba_with_plan(frame: &DecodedFrame, target: &mut [u8], plan: &ScalePlan) {
    let width = plan.target_width;
    let height = plan.target_height;
    if frame.width == 0 || frame.height == 0 || width == 0 || height == 0 {
        target.fill(0);
        warn!(
            source_width = frame.width,
            source_height = frame.height,
            target_width = width,
            target_height = height,
            "skipping frame scale with invalid dimensions"
        );
        return;
    }
    // 最常见的同分辨率场景无需逐像素采样。FFmpeg 的 RGBA 行可能带 padding，
    // 因此仅在 stride 紧凑时整块复制，否则按行复制有效像素。
    if plan.direct_copy {
        let row_bytes = width as usize * 4;
        if frame.stride == row_bytes {
            target.copy_from_slice(&frame.pixels[..target.len()]);
        } else {
            for row in 0..height as usize {
                let source = &frame.pixels[row * frame.stride..row * frame.stride + row_bytes];
                let destination = &mut target[row * row_bytes..(row + 1) * row_bytes];
                destination.copy_from_slice(source);
            }
        }
        return;
    }
    if plan.visible_left > 0
        || plan.visible_top > 0
        || plan.visible_right < width as usize
        || plan.visible_bottom < height as usize
    {
        target.fill(0);
    }

    let target_stride = width as usize * 4;
    for (target_y, source_y) in
        (plan.visible_top..plan.visible_bottom).zip(plan.source_rows.iter().copied())
    {
        let source_row = &frame.pixels[source_y * frame.stride..];
        let target_row = &mut target[target_y * target_stride..(target_y + 1) * target_stride];
        for (column, source_x) in plan.source_columns.iter().copied().enumerate() {
            let destination = (plan.visible_left + column) * 4;
            // RGBA 像素按一个 u32 搬运，避免热循环中为每个像素创建两个切片。
            // 行首和像素偏移均为 4 的倍数，但使用 unaligned 保持对 Vec<u8> 对齐无假设。
            unsafe {
                let pixel =
                    std::ptr::read_unaligned(source_row.as_ptr().add(source_x).cast::<u32>());
                std::ptr::write_unaligned(
                    target_row.as_mut_ptr().add(destination).cast::<u32>(),
                    pixel,
                );
            }
        }
    }
}

struct NiriState {
    registry_state: RegistryState,
    output_state: OutputState,
    shm: Shm,
    configured_size: Option<(u32, u32)>,
    closed: bool,
    frame_ready: bool,
}

impl CompositorHandler for NiriState {
    fn scale_factor_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        new_factor: i32,
    ) {
        info!(new_factor, "layer surface scale changed");
    }

    fn transform_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        new_transform: wl_output::Transform,
    ) {
        info!(?new_transform, "layer surface transform changed");
    }

    fn frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: u32) {
        self.frame_ready = true;
    }

    fn surface_enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }

    fn surface_leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }
}

impl OutputHandler for NiriState {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }

    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {
        info!("Wayland output connected");
    }

    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {
        info!("Wayland output config changed");
    }

    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {
        warn!("Wayland output removed");
    }
}

impl LayerShellHandler for NiriState {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &LayerSurface) {
        self.closed = true;
        warn!("niri layer surface closed");
    }

    fn configure(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &LayerSurface,
        configure: LayerSurfaceConfigure,
        serial: u32,
    ) {
        let (width, height) = configure.new_size;
        if width > 0 && height > 0 {
            self.configured_size = Some((width, height));
            info!(width, height, serial, "received layer surface configure");
        }
    }
}

impl ShmHandler for NiriState {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

delegate_compositor!(NiriState);
delegate_output!(NiriState);
delegate_shm!(NiriState);
delegate_layer!(NiriState);
delegate_registry!(NiriState);

impl ProvidesRegistryState for NiriState {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }

    registry_handlers![OutputState];
}

#[cfg(test)]
mod tests {
    use better_wallpaper_core::{ColorInfo, PixelFormat, Rational};

    use super::*;

    const BLACK: [u8; 4] = [0, 0, 0, 0];

    fn frame(width: u32, height: u32) -> DecodedFrame {
        let mut pixels = Vec::with_capacity(width as usize * height as usize * 4);
        for y in 0..height {
            for x in 0..width {
                pixels.extend_from_slice(&[x as u8 + 1, y as u8 + 1, 7, 255]);
            }
        }
        DecodedFrame {
            pixels,
            cuda: None,
            format: PixelFormat::Rgba,
            width,
            height,
            stride: width as usize * 4,
            pts: 0,
            time_base: Rational {
                numerator: 1,
                denominator: 1,
            },
            color: ColorInfo::default(),
        }
    }

    fn pixel(canvas: &[u8], width: u32, x: u32, y: u32) -> [u8; 4] {
        let offset = (y as usize * width as usize + x as usize) * 4;
        canvas[offset..offset + 4].try_into().unwrap()
    }

    #[test]
    fn stretch_fills_the_entire_target() {
        let mut canvas = vec![0; 4 * 4 * 4];
        scale_rgba(&frame(2, 2), &mut canvas, 4, 4, FillMode::Stretch);

        assert_eq!(pixel(&canvas, 4, 0, 0), [1, 1, 7, 255]);
        assert_eq!(pixel(&canvas, 4, 3, 3), [2, 2, 7, 255]);
        assert!(!canvas.chunks_exact(4).any(|value| value == BLACK));
    }

    #[test]
    fn contain_letterboxes_without_cropping() {
        let mut canvas = vec![255; 4 * 4 * 4];
        scale_rgba(&frame(4, 2), &mut canvas, 4, 4, FillMode::Contain);

        assert_eq!(pixel(&canvas, 4, 0, 0), BLACK);
        assert_eq!(pixel(&canvas, 4, 0, 1), [1, 1, 7, 255]);
        assert_eq!(pixel(&canvas, 4, 3, 2), [4, 2, 7, 255]);
        assert_eq!(pixel(&canvas, 4, 3, 3), BLACK);
    }

    #[test]
    fn cover_crops_the_long_axis() {
        let mut canvas = vec![0; 2 * 2 * 4];
        scale_rgba(&frame(4, 2), &mut canvas, 2, 2, FillMode::Cover);

        assert_eq!(pixel(&canvas, 2, 0, 0), [2, 1, 7, 255]);
        assert_eq!(pixel(&canvas, 2, 1, 1), [3, 2, 7, 255]);
    }

    #[test]
    fn invalid_dimensions_leave_a_black_canvas() {
        let mut invalid = frame(0, 0);
        invalid.stride = 0;
        let mut canvas = vec![255; 2 * 2 * 4];

        scale_rgba(&invalid, &mut canvas, 2, 2, FillMode::Cover);

        assert!(canvas.chunks_exact(4).all(|value| value == BLACK));
    }
}
