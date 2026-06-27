use anyhow::{Context, Result, anyhow, bail};
use better_wallpaper_core::{DecodedFrame, config::FillMode};
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
    Connection, EventQueue, QueueHandle,
    globals::registry_queue_init,
    protocol::{wl_output, wl_shm, wl_surface},
};

pub struct NiriBackend {
    connection: Connection,
    event_queue: EventQueue<NiriState>,
    state: NiriState,
    layer: LayerSurface,
    pool: SlotPool,
    output_name: String,
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
        let connection = Connection::connect_to_env().context("连接 Wayland compositor 失败")?;
        let (globals, mut event_queue) =
            registry_queue_init(&connection).context("读取 Wayland globals 失败")?;
        let qh = event_queue.handle();
        let compositor =
            CompositorState::bind(&globals, &qh).context("compositor 未提供 wl_compositor")?;
        let layer_shell =
            LayerShell::bind(&globals, &qh).context("compositor 未提供 wlr layer-shell")?;
        let shm = Shm::bind(&globals, &qh).context("compositor 未提供 wl_shm")?;
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
            .context("枚举 Wayland 输出失败")?;

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
                "发现 Wayland 输出"
            );
        }
        let (output, output_info) = outputs
            .into_iter()
            .find(|(_, info)| {
                target_output.is_none_or(|target| info.name.as_deref() == Some(target))
            })
            .ok_or_else(|| match target_output {
                Some(name) => anyhow!("找不到配置的 Wayland 输出 {name}"),
                None => anyhow!("compositor 未报告可用输出"),
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
                .context("等待 layer surface configure 失败")?;
        }
        if state.closed {
            bail!("layer surface 在首次 configure 前被 compositor 关闭");
        }
        let (width, height) = state.configured_size.expect("configure 状态已检查");
        let pool = SlotPool::new(width as usize * height as usize * 4, &state.shm)
            .context("创建 wl_shm buffer pool 失败")?;
        info!(output = %output_name, width, height, "niri background layer 已就绪");

        Ok(Self {
            connection,
            event_queue,
            state,
            layer,
            pool,
            output_name,
        })
    }

    pub fn output_name(&self) -> &str {
        &self.output_name
    }

    pub fn size(&self) -> (u32, u32) {
        self.state.configured_size.unwrap_or((1, 1))
    }

    pub fn present(&mut self, frame: &DecodedFrame, fill_mode: FillMode) -> Result<PresentMetrics> {
        self.dispatch_pending()?;
        let wait_started = Instant::now();
        while !self.state.frame_ready && !self.state.closed {
            self.event_queue
                .blocking_dispatch(&mut self.state)
                .context("等待 compositor frame callback 失败")?;
        }
        if self.state.closed {
            bail!("layer surface 已被 compositor 关闭");
        }
        let frame_callback_wait = wait_started.elapsed();
        let (width, height) = self.size();
        let stride = width.checked_mul(4).context("输出 stride 溢出")?;
        let allocate_started = Instant::now();
        let (buffer, canvas) = self
            .pool
            .create_buffer(
                width as i32,
                height as i32,
                stride as i32,
                wl_shm::Format::Abgr8888,
            )
            .context("分配 wl_shm frame buffer 失败")?;
        let buffer_allocate = allocate_started.elapsed();
        let scale_started = Instant::now();
        scale_rgba(frame, canvas, width, height, fill_mode);
        let scale = scale_started.elapsed();
        let submit_started = Instant::now();
        self.layer
            .wl_surface()
            .damage_buffer(0, 0, width as i32, height as i32);
        buffer
            .attach_to(self.layer.wl_surface())
            .context("wl_shm buffer 仍在使用，无法提交")?;
        self.state.frame_ready = false;
        self.layer
            .wl_surface()
            .frame(&self.event_queue.handle(), self.layer.wl_surface().clone());
        self.layer.commit();
        self.connection.flush().context("刷新 Wayland 请求失败")?;
        let submit = submit_started.elapsed();
        debug!(output = %self.output_name, width, height, pts = frame.pts, "已提交 wl_shm 视频帧");
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
            .context("处理 Wayland 事件失败")?;
        Ok(())
    }
}

fn scale_rgba(frame: &DecodedFrame, target: &mut [u8], width: u32, height: u32, mode: FillMode) {
    if frame.width == 0 || frame.height == 0 || width == 0 || height == 0 {
        target.fill(0);
        warn!(
            source_width = frame.width,
            source_height = frame.height,
            target_width = width,
            target_height = height,
            "跳过无效尺寸的视频帧缩放"
        );
        return;
    }
    let source_ratio = frame.width as f64 / frame.height as f64;
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

    // 最常见的同分辨率场景无需逐像素采样。FFmpeg 的 RGBA 行可能带 padding，
    // 因此仅在 stride 紧凑时整块复制，否则按行复制有效像素。
    if draw_width == frame.width
        && draw_height == frame.height
        && offset_x == 0
        && offset_y == 0
        && width == frame.width
        && height == frame.height
    {
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
    let visible_left = offset_x.max(0) as usize;
    let visible_top = offset_y.max(0) as usize;
    let visible_right = (offset_x + draw_width as i64).min(width as i64).max(0) as usize;
    let visible_bottom = (offset_y + draw_height as i64).min(height as i64).max(0) as usize;

    if visible_left > 0
        || visible_top > 0
        || visible_right < width as usize
        || visible_bottom < height as usize
    {
        target.fill(0);
    }

    // 横向采样位置只依赖输出列，预计算后避免在数百万像素的内层循环中做除法。
    let source_columns: Vec<_> = (visible_left..visible_right)
        .map(|target_x| {
            let draw_x = target_x as i64 - offset_x;
            draw_x as usize * frame.width as usize / draw_width as usize * 4
        })
        .collect();
    let target_stride = width as usize * 4;
    for target_y in visible_top..visible_bottom {
        let draw_y = target_y as i64 - offset_y;
        let source_y = draw_y as usize * frame.height as usize / draw_height as usize;
        let source_row = &frame.pixels[source_y * frame.stride..];
        let target_row = &mut target[target_y * target_stride..(target_y + 1) * target_stride];
        for (column, source_x) in source_columns.iter().copied().enumerate() {
            let destination = (visible_left + column) * 4;
            target_row[destination..destination + 4]
                .copy_from_slice(&source_row[source_x..source_x + 4]);
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
        info!(new_factor, "layer surface scale 已变化");
    }

    fn transform_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        new_transform: wl_output::Transform,
    ) {
        info!(?new_transform, "layer surface transform 已变化");
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
        info!("Wayland 输出已连接");
    }

    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {
        info!("Wayland 输出配置已变化");
    }

    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {
        warn!("Wayland 输出已移除");
    }
}

impl LayerShellHandler for NiriState {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &LayerSurface) {
        self.closed = true;
        warn!("niri layer surface 已关闭");
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
            info!(width, height, serial, "收到 layer surface configure");
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
