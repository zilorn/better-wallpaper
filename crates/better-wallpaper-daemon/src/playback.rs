use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use better_wallpaper_core::{DecodeOptions, VideoDecoder, VideoError, config::FillMode};
use better_wallpaper_ffmpeg::{FfmpegDecoder, FrameDecision, PlaybackClock, frame_queue};
use better_wallpaper_wayland::NiriBackend;
use tracing::{debug, info, warn};

const FRAME_QUEUE_CAPACITY: usize = 3;
const DROP_THRESHOLD: Duration = Duration::from_millis(100);
const STATS_INTERVAL: u64 = 300;
const REALTIME_STATS_INTERVAL: Duration = Duration::from_secs(1);
const CONTROL_POLL_INTERVAL: Duration = Duration::from_millis(20);

/// 可跨线程复制的播放控制句柄。解码和消费端都定期检查取消状态，避免队列满时无法退出。
#[derive(Clone, Debug, Default)]
pub struct PlaybackControl {
    paused: Arc<AtomicBool>,
    cancelled: Arc<AtomicBool>,
    running: Arc<AtomicBool>,
}

impl PlaybackControl {
    pub fn set_paused(&self, paused: bool) {
        self.paused.store(paused, Ordering::Release);
        info!(paused, "播放暂停状态已更新");
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
        info!("收到播放取消请求");
    }

    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::Acquire)
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::Acquire)
    }

    pub fn set_running(&self, running: bool) {
        self.running.store(running, Ordering::Release);
        info!(running, "播放运行状态已更新");
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct PlaybackStats {
    pub presented: u64,
    pub dropped: u64,
    pub loops: u64,
}

struct NiriPerformanceWindow {
    started: Instant,
    decoded: u64,
    presented: u64,
    dropped: u64,
    present_time: Duration,
    callback_wait: Duration,
    buffer_allocate: Duration,
    scale: Duration,
    submit: Duration,
}

impl NiriPerformanceWindow {
    fn new() -> Self {
        Self {
            started: Instant::now(),
            decoded: 0,
            presented: 0,
            dropped: 0,
            present_time: Duration::ZERO,
            callback_wait: Duration::ZERO,
            buffer_allocate: Duration::ZERO,
            scale: Duration::ZERO,
            submit: Duration::ZERO,
        }
    }

    fn log_if_due(&mut self, output_count: usize) {
        let elapsed = self.started.elapsed();
        if elapsed < REALTIME_STATS_INTERVAL {
            return;
        }
        let seconds = elapsed.as_secs_f64();
        let presented = self.presented.max(1) as f64;
        info!(
            decoded_fps = self.decoded as f64 / seconds,
            submitted_fps = self.presented as f64 / seconds,
            late_fps = self.dropped as f64 / seconds,
            present_avg_ms = self.present_time.as_secs_f64() * 1000.0 / presented,
            callback_wait_avg_ms = self.callback_wait.as_secs_f64() * 1000.0 / presented,
            scale_avg_ms = self.scale.as_secs_f64() * 1000.0 / presented,
            buffer_avg_ms = self.buffer_allocate.as_secs_f64() * 1000.0 / presented,
            submit_avg_ms = self.submit.as_secs_f64() * 1000.0 / presented,
            output_count,
            "niri 实时播放性能"
        );
        *self = Self::new();
    }
}

/// 运行软件解码和 headless 帧消费闭环。同步队列会在消费者落后时对解码线程施加背压。
pub fn run_headless(path: PathBuf, loop_playback: bool, hardware: bool) -> Result<()> {
    run_headless_controlled(path, loop_playback, hardware, PlaybackControl::default()).map(|_| ())
}

/// 在指定时间后通过正常取消路径结束播放，供稳定性测试和自动化验收使用。
pub fn run_headless_for(
    path: PathBuf,
    loop_playback: bool,
    hardware: bool,
    duration: Duration,
) -> Result<PlaybackStats> {
    let control = PlaybackControl::default();
    let timer_control = control.clone();
    let (finished_tx, finished_rx) = mpsc::channel();
    let timer = thread::Builder::new()
        .name("playback-deadline".into())
        .spawn(move || {
            if finished_rx.recv_timeout(duration).is_err() {
                info!(
                    duration_ms = duration.as_millis(),
                    "播放时限到达，开始正常退出"
                );
                timer_control.cancel();
            }
        })
        .context("创建播放计时线程失败")?;

    let result = run_headless_controlled(path, loop_playback, hardware, control);
    let _ = finished_tx.send(());
    timer
        .join()
        .map_err(|_| anyhow::anyhow!("播放计时线程发生 panic"))?;
    result
}

/// 在 niri background layer 上按 PTS 播放视频。多个输出共享同一个解码结果。
pub fn run_niri(
    path: PathBuf,
    loop_playback: bool,
    hardware: bool,
    fill_mode: FillMode,
    output_names: &[String],
) -> Result<()> {
    run_niri_controlled(
        path,
        loop_playback,
        hardware,
        fill_mode,
        output_names,
        PlaybackControl::default(),
    )
}

/// 在 niri background layer 上播放视频，并响应来自管理接口的暂停和取消命令。
pub fn run_niri_controlled(
    path: PathBuf,
    loop_playback: bool,
    hardware: bool,
    fill_mode: FillMode,
    output_names: &[String],
    control: PlaybackControl,
) -> Result<()> {
    let mut backends = if output_names.is_empty() {
        vec![NiriBackend::connect(None).context("自动选择并初始化 niri 输出失败")?]
    } else {
        output_names
            .iter()
            .map(|name| {
                NiriBackend::connect(Some(name))
                    .with_context(|| format!("初始化 niri 输出 {name} 失败"))
            })
            .collect::<Result<Vec<_>>>()?
    };
    let (frames_tx, frames_rx) = frame_queue(FRAME_QUEUE_CAPACITY);
    let (decode_result_tx, decode_result_rx) = mpsc::sync_channel(1);
    let (startup_tx, startup_rx) = mpsc::sync_channel(1);
    let decode_control = control.clone();
    let decode_path = path.clone();
    let decoder_thread = thread::Builder::new()
        .name("niri-video-decoder".into())
        .spawn(move || {
            let mut decoder = FfmpegDecoder::new();
            let media = match decoder.open(&decode_path, DecodeOptions { hardware }) {
                Ok(media) => media,
                Err(error) => {
                    let message = format!("打开视频 {} 失败: {error}", decode_path.display());
                    let _ = startup_tx.send(Err(message.clone()));
                    let _ = decode_result_tx.send(Err(anyhow::anyhow!(message)));
                    return;
                }
            };
            if startup_tx.send(Ok(media)).is_err() {
                return;
            }
            let result = loop {
                if decode_control.is_cancelled() {
                    break Ok(());
                }
                if decode_control.is_paused() {
                    thread::sleep(CONTROL_POLL_INTERVAL);
                    continue;
                }
                match decoder.next_frame() {
                    Ok(mut frame) => loop {
                        match frames_tx.try_send(frame) {
                            Ok(()) => break,
                            Err(mpsc::TrySendError::Full(returned)) => {
                                frame = returned;
                                if decode_control.is_cancelled() {
                                    break;
                                }
                                thread::sleep(Duration::from_millis(1));
                            }
                            Err(mpsc::TrySendError::Disconnected(_)) => break,
                        }
                    },
                    Err(VideoError::EndOfStream) if loop_playback => {
                        if let Err(error) = decoder.seek_start() {
                            break Err(anyhow::anyhow!(error).context("niri 循环播放跳转失败"));
                        }
                    }
                    Err(VideoError::EndOfStream) => break Ok(()),
                    Err(error) => break Err(anyhow::anyhow!(error)),
                }
            };
            let _ = decode_result_tx.send(result);
        })
        .context("创建 niri 解码线程失败")?;
    let media = startup_rx
        .recv()
        .context("niri 解码线程未返回媒体信息")?
        .map_err(anyhow::Error::msg)?;
    info!(
        path = %path.display(),
        outputs = ?backends
            .iter()
            .map(|backend| (backend.output_name(), backend.size()))
            .collect::<Vec<_>>(),
        output_count = backends.len(),
        video_size = ?(media.width, media.height),
        ?fill_mode,
        "niri 视频播放开始"
    );
    let mut clock: Option<PlaybackClock> = None;
    let mut previous_pts = Duration::ZERO;
    let mut presented = 0_u64;
    let mut dropped = 0_u64;
    let mut loops = 0_u64;
    let mut first_frame_presented = false;
    let mut perf = NiriPerformanceWindow::new();

    loop {
        if control.is_cancelled() {
            info!(path = %path.display(), "niri 播放已响应取消请求");
            break;
        }
        if control.is_paused() {
            for backend in &mut backends {
                backend.dispatch_pending().with_context(|| {
                    format!(
                        "暂停期间处理 niri 输出 {} 的事件失败",
                        backend.output_name()
                    )
                })?;
            }
            clock = None;
            thread::sleep(CONTROL_POLL_INTERVAL);
            continue;
        }
        match frames_rx.recv_timeout(CONTROL_POLL_INTERVAL) {
            Ok(frame) => {
                perf.decoded += 1;
                let pts = frame.presentation_time();
                match clock.as_mut() {
                    None => clock = Some(PlaybackClock::new(Instant::now(), pts, DROP_THRESHOLD)),
                    Some(clock) if pts < previous_pts => {
                        loops += 1;
                        clock.reset(Instant::now(), pts);
                    }
                    Some(_) => {}
                }
                previous_pts = pts;
                loop {
                    match clock
                        .as_ref()
                        .expect("时钟已初始化")
                        .decide(Instant::now(), pts)
                    {
                        FrameDecision::Wait(duration) => {
                            thread::sleep(duration.min(CONTROL_POLL_INTERVAL));
                            for backend in &mut backends {
                                backend.dispatch_pending().with_context(|| {
                                    format!("处理 niri 输出 {} 的事件失败", backend.output_name())
                                })?;
                            }
                            if control.is_cancelled() || control.is_paused() {
                                break;
                            }
                        }
                        FrameDecision::Present => {
                            let present_started = Instant::now();
                            for backend in &mut backends {
                                let metrics =
                                    backend.present(&frame, fill_mode).with_context(|| {
                                        format!("向 niri 输出 {} 提交帧失败", backend.output_name())
                                    })?;
                                perf.callback_wait += metrics.frame_callback_wait;
                                perf.buffer_allocate += metrics.buffer_allocate;
                                perf.scale += metrics.scale;
                                perf.submit += metrics.submit;
                            }
                            presented += 1;
                            perf.presented += 1;
                            let present_elapsed = present_started.elapsed();
                            perf.present_time += present_elapsed;
                            if !first_frame_presented {
                                clock
                                    .as_mut()
                                    .expect("时钟已初始化")
                                    .reset(Instant::now(), pts);
                                first_frame_presented = true;
                                info!(
                                    present_ms = present_elapsed.as_millis(),
                                    "首帧已提交，播放时钟已排除后端初始化耗时"
                                );
                            } else if presented.is_multiple_of(STATS_INTERVAL) {
                                debug!(
                                    presented,
                                    dropped,
                                    present_ms = present_elapsed.as_millis(),
                                    "niri 呈现统计"
                                );
                            }
                            break;
                        }
                        FrameDecision::Drop => {
                            dropped += 1;
                            perf.dropped += 1;
                            clock
                                .as_mut()
                                .expect("时钟已初始化")
                                .reset(Instant::now(), pts);
                            let present_started = Instant::now();
                            for backend in &mut backends {
                                let metrics =
                                    backend.present(&frame, fill_mode).with_context(|| {
                                        format!(
                                            "向 niri 输出 {} 提交过载恢复帧失败",
                                            backend.output_name()
                                        )
                                    })?;
                                perf.callback_wait += metrics.frame_callback_wait;
                                perf.buffer_allocate += metrics.buffer_allocate;
                                perf.scale += metrics.scale;
                                perf.submit += metrics.submit;
                            }
                            presented += 1;
                            perf.presented += 1;
                            perf.present_time += present_started.elapsed();
                            debug!(
                                pts_ms = pts.as_millis(),
                                dropped, "niri 解码过载，提交当前帧并重建时钟"
                            );
                            break;
                        }
                    }
                }
                perf.log_if_due(backends.len());
            }
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    decoder_thread
        .join()
        .map_err(|_| anyhow::anyhow!("niri 解码线程发生 panic"))?;
    decode_result_rx
        .recv()
        .context("niri 解码线程未返回结果")??;
    info!(path = %path.display(), output_count = backends.len(), presented, dropped, loops, "niri 视频播放结束");
    Ok(())
}

pub fn run_headless_controlled(
    path: PathBuf,
    loop_playback: bool,
    hardware: bool,
    control: PlaybackControl,
) -> Result<PlaybackStats> {
    let (frames_tx, frames_rx) = frame_queue(FRAME_QUEUE_CAPACITY);
    let (result_tx, result_rx) = mpsc::sync_channel(1);
    let decode_path = path.clone();
    let decode_control = control.clone();
    let decoder_thread = thread::Builder::new()
        .name("video-decoder".into())
        .spawn(move || {
            let result = decode_frames(
                decode_path,
                loop_playback,
                hardware,
                frames_tx,
                decode_control,
            );
            let _ = result_tx.send(result);
        })
        .context("创建视频解码线程失败")?;

    let mut stats = PlaybackStats::default();
    let mut clock: Option<PlaybackClock> = None;
    let mut previous_pts = Duration::ZERO;
    while !control.is_cancelled() {
        if control.is_paused() {
            thread::sleep(CONTROL_POLL_INTERVAL);
            clock = None;
            continue;
        }
        let frame = match frames_rx.recv_timeout(CONTROL_POLL_INTERVAL) {
            Ok(frame) => frame,
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        };
        let pts = frame.presentation_time();
        match clock.as_mut() {
            None => clock = Some(PlaybackClock::new(Instant::now(), pts, DROP_THRESHOLD)),
            Some(clock) if pts < previous_pts => {
                stats.loops += 1;
                clock.reset(Instant::now(), pts);
                info!(loops = stats.loops, "检测到循环起点，播放时钟已重置");
            }
            Some(_) => {}
        }
        previous_pts = pts;

        loop {
            match clock
                .as_ref()
                .expect("播放时钟已初始化")
                .decide(Instant::now(), pts)
            {
                FrameDecision::Wait(duration) => {
                    thread::sleep(duration.min(CONTROL_POLL_INTERVAL));
                    if control.is_cancelled() || control.is_paused() {
                        break;
                    }
                }
                FrameDecision::Present => {
                    stats.presented += 1;
                    break;
                }
                FrameDecision::Drop => {
                    stats.dropped += 1;
                    warn!(
                        pts_ms = pts.as_millis(),
                        dropped = stats.dropped,
                        "headless 消费端丢弃过期帧"
                    );
                    break;
                }
            }
        }
        if (stats.presented + stats.dropped).is_multiple_of(STATS_INTERVAL) {
            debug!(
                presented = stats.presented,
                dropped = stats.dropped,
                loops = stats.loops,
                "headless 播放统计"
            );
        }
    }

    decoder_thread
        .join()
        .map_err(|_| anyhow::anyhow!("视频解码线程发生 panic"))?;
    result_rx.recv().context("视频解码线程未返回结果")??;
    info!(path = %path.display(), presented = stats.presented, dropped = stats.dropped, loops = stats.loops, "headless 播放结束");
    Ok(stats)
}

fn decode_frames(
    path: PathBuf,
    loop_playback: bool,
    hardware: bool,
    sender: better_wallpaper_ffmpeg::FrameQueueSender,
    control: PlaybackControl,
) -> Result<()> {
    let mut decoder = FfmpegDecoder::new();
    let media = decoder
        .open(&path, DecodeOptions { hardware })
        .with_context(|| format!("打开视频 {} 失败", path.display()))?;
    info!(path = %path.display(), width = media.width, height = media.height, duration_ms = ?media.duration.map(|value| value.as_millis()), frame_rate = ?media.frame_rate, queue_capacity = FRAME_QUEUE_CAPACITY, "headless 解码开始");

    loop {
        if control.is_cancelled() {
            info!("解码线程已响应取消请求");
            return Ok(());
        }
        if control.is_paused() {
            thread::sleep(CONTROL_POLL_INTERVAL);
            continue;
        }
        match decoder.next_frame() {
            Ok(mut frame) => loop {
                match sender.try_send(frame) {
                    Ok(()) => break,
                    Err(mpsc::TrySendError::Full(returned)) => {
                        frame = returned;
                        if control.is_cancelled() {
                            info!("解码线程在等待帧队列时响应取消请求");
                            return Ok(());
                        }
                        thread::sleep(CONTROL_POLL_INTERVAL);
                    }
                    Err(mpsc::TrySendError::Disconnected(_)) => {
                        bail!("headless 消费端已停止")
                    }
                }
            },
            Err(VideoError::EndOfStream) if loop_playback => {
                decoder.seek_start().context("循环播放跳转失败")?;
            }
            Err(VideoError::EndOfStream) => return Ok(()),
            Err(error) => bail!(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn playback_stats_start_empty() {
        assert_eq!(
            PlaybackStats::default(),
            PlaybackStats {
                presented: 0,
                dropped: 0,
                loops: 0
            }
        );
    }

    #[test]
    fn playback_control_tracks_pause_and_cancel() {
        let control = PlaybackControl::default();
        let worker = control.clone();

        control.set_paused(true);
        assert!(worker.is_paused());
        control.set_paused(false);
        assert!(!worker.is_paused());

        control.cancel();
        assert!(worker.is_cancelled());

        control.set_running(true);
        assert!(worker.is_running());
    }
}
