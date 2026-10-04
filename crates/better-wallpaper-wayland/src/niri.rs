use crate::egl::EglRenderer;
use anyhow::{Context, Result};
use better_wallpaper_core::{DecodedFrame, config::FillMode};
use better_wallpaper_renderer::Scene2dAssets;
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    delegate_compositor, delegate_layer, delegate_output, delegate_pointer, delegate_registry,
    delegate_seat, delegate_shm,
    output::{OutputHandler, OutputState},
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    seat::{
        Capability, SeatHandler, SeatState,
        pointer::{PointerEvent, PointerEventKind, PointerHandler},
    },
    shell::{
        WaylandSurface,
        wlr_layer::{
            Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
            LayerSurfaceConfigure,
        },
    },
    shm::{Shm, ShmHandler, slot::SlotPool},
};
use std::{
    io::ErrorKind,
    time::{Duration, Instant},
};
use tracing::{debug, info, warn};
use wayland_client::{
    Connection, EventQueue, Proxy, QueueHandle,
    globals::registry_queue_init,
    protocol::{wl_output, wl_pointer, wl_seat, wl_shm, wl_surface},
};

pub struct NiriBackend {
    // Drop EGL and surface resources before the connection and event queue.
    surface: Option<OutputSurface>,
    connection: Connection,
    event_queue: EventQueue<NiriState>,
    state: NiriState,
    compositor: CompositorState,
    layer_shell: LayerShell,
    target_output: Option<String>,
    output_name: String,
    scene_assets: Option<Scene2dAssets>,
    next_surface_attempt: Instant,
}

struct OutputSurface {
    // EGL must be released before its Wayland surface.
    egl: Option<EglRenderer>,
    layer: LayerSurface,
    pool: Option<SlotPool>,
    output: wl_output::WlOutput,
    scale_plan: Option<ScalePlan>,
}

const SURFACE_RETRY_DELAY: Duration = Duration::from_secs(1);

// Explicit targets never fall back to another output. Auto-selection keeps its
// current output until it disappears, avoiding surface churn on unrelated hotplug.
fn select_output<T: PartialEq>(
    outputs: &[(T, String)],
    target: Option<&str>,
    current: Option<&T>,
) -> Option<usize> {
    if let Some(target) = target {
        outputs.iter().position(|(_, name)| name == target)
    } else {
        current
            .and_then(|current| outputs.iter().position(|(output, _)| output == current))
            .or_else(|| (!outputs.is_empty()).then_some(0))
    }
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
        Self::connect_with(connection, target_output)
    }

    fn connect_with(connection: Connection, target_output: Option<&str>) -> Result<Self> {
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
            seat_state: SeatState::new(&globals, &qh),
            pointers: Vec::new(),
            pointer_position: None,
            mouse_enabled: false,
            configured_size: None,
            closed: false,
            frame_ready: true,
            surface: None,
            outputs_changed: true,
            needs_redraw: true,
        };
        event_queue
            .roundtrip(&mut state)
            .context("failed to enumerate Wayland outputs")?;

        let mut backend = Self {
            surface: None,
            connection,
            event_queue,
            state,
            compositor,
            layer_shell,
            target_output: target_output.map(str::to_owned),
            output_name: target_output.unwrap_or("auto").to_owned(),
            scene_assets: None,
            next_surface_attempt: Instant::now(),
        };
        backend.reconcile_output()?;
        Ok(backend)
    }

    fn release_surface(&mut self) {
        if self.surface.take().is_some() {
            info!(output = %self.output_name, "released niri background surface");
        }
        self.state.surface = None;
        self.state.pointer_position = None;
        self.state.configured_size = None;
        self.state.closed = false;
        self.state.frame_ready = true;
    }

    fn reconcile_output(&mut self) -> Result<()> {
        let now = Instant::now();
        if self.state.closed {
            self.release_surface();
            self.next_surface_attempt = now + SURFACE_RETRY_DELAY;
        }
        if self.state.outputs_changed || self.surface.is_none() {
            let outputs: Vec<_> = self
                .state
                .output_state
                .outputs()
                .filter_map(|output| {
                    self.state.output_state.info(&output).map(|info| {
                        let name = info.name.unwrap_or_else(|| format!("output-{}", info.id));
                        (output, name)
                    })
                })
                .collect();
            let selected = select_output(
                &outputs,
                self.target_output.as_deref(),
                self.surface.as_ref().map(|surface| &surface.output),
            );
            match selected.map(|index| &outputs[index]) {
                Some((output, name)) => {
                    if self
                        .surface
                        .as_ref()
                        .is_some_and(|surface| surface.output != *output)
                    {
                        self.release_surface();
                        self.next_surface_attempt = now;
                    }
                    if self.surface.is_none() && now >= self.next_surface_attempt {
                        let qh = self.event_queue.handle();
                        let surface = self.compositor.create_surface(&qh);
                        let layer = self.layer_shell.create_layer_surface(
                            &qh,
                            surface,
                            Layer::Background,
                            Some("better-wallpaper"),
                            Some(output),
                        );
                        layer.set_anchor(
                            Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT,
                        );
                        layer.set_exclusive_zone(-1);
                        layer.set_keyboard_interactivity(KeyboardInteractivity::None);
                        layer.set_size(0, 0);
                        self.state.surface = Some(layer.wl_surface().clone());
                        layer.commit();
                        self.output_name = name.clone();
                        self.surface = Some(OutputSurface {
                            egl: None,
                            layer,
                            pool: None,
                            output: output.clone(),
                            scale_plan: None,
                        });
                        info!(output = %name, "created niri background surface, waiting for configure");
                    }
                }
                None => {
                    if self.surface.is_some() || self.state.outputs_changed {
                        self.release_surface();
                        info!(target = ?self.target_output, "waiting for a matching Wayland output");
                    }
                    self.next_surface_attempt = now;
                }
            }
            self.state.outputs_changed = false;
        }
        if let Some((width, height)) = self.state.configured_size
            && let Some(surface) = &mut self.surface
            && surface.pool.is_none()
        {
            surface.pool = Some(
                SlotPool::new(width as usize * height as usize * 4, &self.state.shm)
                    .context("failed to create wl_shm buffer pool")?,
            );
            surface.egl = match unsafe {
                EglRenderer::new(
                    self.connection.backend().display_ptr().cast(),
                    surface.layer.wl_surface().id().as_ptr().cast(),
                    width,
                    height,
                )
            } {
                Ok(renderer) => Some(renderer),
                Err(error) => {
                    warn!(output = %self.output_name, %error, "EGL initialization failed, falling back to wl_shm");
                    None
                }
            };
            if let Some(assets) = &self.scene_assets {
                surface
                    .egl
                    .as_mut()
                    .context("EGL unavailable, scene rendering requires GPU support")?
                    .set_scene_assets(assets.clone())?;
            }
            info!(output = %self.output_name, width, height, gpu = surface.egl.is_some(), "niri background layer ready");
        }
        Ok(())
    }

    /// Whether a configured surface can accept a frame without blocking.
    pub fn is_ready(&self) -> bool {
        self.surface.is_some()
            && self.state.configured_size.is_some()
            && self.state.frame_ready
            && !self.state.closed
    }

    pub fn has_output(&self) -> bool {
        self.surface.is_some() && self.state.configured_size.is_some() && !self.state.closed
    }

    pub fn needs_redraw(&self) -> bool {
        self.state.needs_redraw && self.is_ready()
    }

    pub fn output_name(&self) -> &str {
        &self.output_name
    }

    pub fn size(&self) -> (u32, u32) {
        self.state.configured_size.unwrap_or((1, 1))
    }
    pub fn load_scene_assets(&mut self, assets: Scene2dAssets) -> Result<()> {
        self.state.mouse_enabled = assets.parallax.is_some();
        self.state.pointer_position = None;
        if let Some(surface) = &mut self.surface
            && surface.pool.is_some()
        {
            surface
                .egl
                .as_mut()
                .context("EGL not available, cannot load scene textures")?
                .set_scene_assets(assets.clone())?;
        }
        // Keep CPU assets so a replacement surface can upload into its new GL context.
        self.scene_assets = Some(assets);
        Ok(())
    }

    /// Returns false when the output is absent, configuring, or awaiting a callback.
    pub fn present_scene(&mut self, elapsed_seconds: f64) -> Result<bool> {
        self.dispatch_pending()?;
        if !self.is_ready() {
            return Ok(false);
        }
        let (width, height) = self.size();
        let surface = self.surface.as_mut().expect("ready surface");
        let egl = surface
            .egl
            .as_mut()
            .context("EGL GPU backend unavailable, scene rendering not supported")?;
        self.state.needs_redraw = false;
        self.state.frame_ready = false;
        surface.layer.wl_surface().frame(
            &self.event_queue.handle(),
            surface.layer.wl_surface().clone(),
        );
        let pointer = if self.state.mouse_enabled {
            self.state
                .pointer_position
                .and_then(|position| normalized_pointer(position, (width, height)))
        } else {
            None
        };
        egl.render_scene(width, height, elapsed_seconds, pointer)?;
        surface.layer.commit();
        self.connection
            .flush()
            .context("failed to flush Wayland scene frame")?;
        Ok(true)
    }

    /// Returns None when the output cannot currently accept a frame.
    pub fn present(
        &mut self,
        frame: &DecodedFrame,
        fill_mode: FillMode,
    ) -> Result<Option<PresentMetrics>> {
        self.dispatch_pending()?;
        if !self.is_ready() {
            return Ok(None);
        }
        let (width, height) = self.size();
        let surface = self.surface.as_mut().expect("ready surface");
        if let Some(egl) = &mut surface.egl {
            let submit_started = Instant::now();
            self.state.needs_redraw = false;
            self.state.frame_ready = false;
            surface.layer.wl_surface().frame(
                &self.event_queue.handle(),
                surface.layer.wl_surface().clone(),
            );
            match egl.render(frame, fill_mode, width, height) {
                Ok(()) => {
                    self.connection
                        .flush()
                        .context("failed to flush Wayland GPU frame")?;
                    return Ok(Some(PresentMetrics {
                        submit: submit_started.elapsed(),
                        ..Default::default()
                    }));
                }
                Err(error) => {
                    if frame.cuda.is_some() {
                        return Err(error)
                            .context("CUDA OpenGL interop failed for a hardware-decoded frame");
                    }
                    warn!(output = %self.output_name, %error, "EGL presentation failed, falling back to wl_shm");
                    surface.egl = None;
                    self.state.frame_ready = true;
                }
            }
        }
        let stride = width.checked_mul(4).context("output stride overflow")?;
        let allocate_started = Instant::now();
        let (buffer, canvas) = surface
            .pool
            .as_mut()
            .expect("configured buffer pool")
            .create_buffer(
                width as i32,
                height as i32,
                stride as i32,
                wl_shm::Format::Abgr8888,
            )
            .context("failed to allocate wl_shm frame buffer")?;
        let buffer_allocate = allocate_started.elapsed();
        let scale_started = Instant::now();
        let plan = surface.scale_plan.get_or_insert_with(|| {
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
        surface
            .layer
            .wl_surface()
            .damage_buffer(0, 0, width as i32, height as i32);
        buffer
            .attach_to(surface.layer.wl_surface())
            .context("wl_shm buffer still in use, cannot submit")?;
        self.state.needs_redraw = false;
        self.state.frame_ready = false;
        surface.layer.wl_surface().frame(
            &self.event_queue.handle(),
            surface.layer.wl_surface().clone(),
        );
        surface.layer.commit();
        self.connection
            .flush()
            .context("failed to flush Wayland requests")?;
        let submit = submit_started.elapsed();
        debug!(output = %self.output_name, width, height, pts = frame.pts, "submitted wl_shm video frame");
        Ok(Some(PresentMetrics {
            buffer_allocate,
            scale,
            submit,
            ..Default::default()
        }))
    }

    /// Read the socket as well as dispatching queued events, including while paused.
    /// Wayland reports EOF/protocol failures through read/dispatch/flush errors;
    /// there is no separate connection_closed callback in wayland-client.
    pub fn dispatch_pending(&mut self) -> Result<()> {
        self.event_queue
            .dispatch_pending(&mut self.state)
            .context("failed to process Wayland events")?;
        if let Some(guard) = self.connection.prepare_read() {
            match guard.read() {
                Ok(_) => {}
                Err(wayland_client::backend::WaylandError::Io(error))
                    if error.kind() == ErrorKind::WouldBlock => {}
                Err(error) => return Err(error).context("Wayland compositor connection lost"),
            }
        }
        self.event_queue
            .dispatch_pending(&mut self.state)
            .context("failed to process Wayland socket events")?;
        self.reconcile_output()?;
        self.connection
            .flush()
            .context("failed to flush Wayland lifecycle requests")?;
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

fn normalized_pointer((x, y): (f64, f64), (width, height): (u32, u32)) -> Option<[f32; 2]> {
    if width == 0 || height == 0 || !x.is_finite() || !y.is_finite() {
        return None;
    }
    Some([
        (2.0 * x / f64::from(width) - 1.0).clamp(-1.0, 1.0) as f32,
        (1.0 - 2.0 * y / f64::from(height)).clamp(-1.0, 1.0) as f32,
    ])
}

struct NiriState {
    registry_state: RegistryState,
    output_state: OutputState,
    shm: Shm,
    seat_state: SeatState,
    pointers: Vec<(wl_seat::WlSeat, wl_pointer::WlPointer)>,
    pointer_position: Option<(f64, f64)>,
    mouse_enabled: bool,
    configured_size: Option<(u32, u32)>,
    closed: bool,
    frame_ready: bool,
    surface: Option<wl_surface::WlSurface>,
    outputs_changed: bool,
    needs_redraw: bool,
}

impl SeatHandler for NiriState {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }
    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
    fn new_capability(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Pointer
            && !self.pointers.iter().any(|(known, _)| known == &seat)
        {
            match self.seat_state.get_pointer(qh, &seat) {
                Ok(pointer) => {
                    self.pointers.push((seat, pointer));
                    info!("Wayland pointer available for wallpaper-surface parallax");
                }
                Err(error) => warn!(%error, "failed to bind Wayland scene pointer"),
            }
        }
    }
    fn remove_capability(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Pointer
            && let Some(index) = self.pointers.iter().position(|(known, _)| known == &seat)
        {
            let (_, pointer) = self.pointers.swap_remove(index);
            if pointer.version() >= 3 {
                pointer.release();
            }
            self.pointer_position = None;
            info!("Wayland scene pointer removed");
        }
    }
    fn remove_seat(&mut self, conn: &Connection, qh: &QueueHandle<Self>, seat: wl_seat::WlSeat) {
        self.remove_capability(conn, qh, seat, Capability::Pointer);
    }
}

impl NiriState {
    fn handle_pointer_event(&mut self, event: &PointerEvent) {
        if !self.mouse_enabled || self.surface.as_ref() != Some(&event.surface) {
            return;
        }
        match event.kind {
            PointerEventKind::Enter { .. } | PointerEventKind::Motion { .. } => {
                self.pointer_position = Some(event.position)
            }
            PointerEventKind::Leave { .. } => self.pointer_position = None,
            _ => return,
        }
        self.needs_redraw = true;
    }
}

impl PointerHandler for NiriState {
    fn pointer_frame(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_pointer::WlPointer,
        events: &[PointerEvent],
    ) {
        for event in events {
            self.handle_pointer_event(event);
        }
    }
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

    fn frame(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        surface: &wl_surface::WlSurface,
        _: u32,
    ) {
        if self.surface.as_ref() == Some(surface) {
            self.frame_ready = true;
        }
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
        self.outputs_changed = true;
        info!("Wayland output connected");
    }

    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {
        self.outputs_changed = true;
        info!("Wayland output config changed");
    }

    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {
        self.outputs_changed = true;
        warn!("Wayland output removed");
    }
}

impl LayerShellHandler for NiriState {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, layer: &LayerSurface) {
        if self.surface.as_ref() == Some(layer.wl_surface()) {
            self.closed = true;
            warn!("niri layer surface closed, scheduling recreation");
        }
    }

    fn configure(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        layer: &LayerSurface,
        configure: LayerSurfaceConfigure,
        serial: u32,
    ) {
        self.configure_surface(layer, configure.new_size, serial);
    }
}

impl NiriState {
    fn configure_surface(
        &mut self,
        layer: &LayerSurface,
        (width, height): (u32, u32),
        serial: u32,
    ) {
        if self.surface.as_ref() == Some(layer.wl_surface()) && width > 0 && height > 0 {
            self.configured_size = Some((width, height));
            self.needs_redraw = true;
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
delegate_seat!(NiriState);
delegate_pointer!(NiriState);

impl ProvidesRegistryState for NiriState {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }

    registry_handlers![OutputState, SeatState];
}

#[cfg(test)]
mod tests {
    use better_wallpaper_core::{ColorInfo, PixelFormat, Rational};

    use super::*;

    const BLACK: [u8; 4] = [0, 0, 0, 0];

    pub(super) fn frame(width: u32, height: u32) -> DecodedFrame {
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

#[cfg(test)]
#[path = "niri_tests.rs"]
mod lifecycle_tests;
