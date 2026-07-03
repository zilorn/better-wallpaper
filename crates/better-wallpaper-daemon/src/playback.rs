use std::{
    path::{Path, PathBuf},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use better_wallpaper_core::{
    DecodeOptions, PlaybackControl, VideoDecoder, VideoError, config::FillMode,
};
use better_wallpaper_ffmpeg::{
    AudioInfo, FfmpegAudioDecoder, FfmpegDecoder, FrameDecision, PlaybackClock, frame_queue,
};
use better_wallpaper_wayland::NiriBackend;
use rodio::{OutputStream, OutputStreamBuilder, Sink, Source};
use tracing::{debug, info, warn};

const FRAME_QUEUE_CAPACITY: usize = 3;
const DROP_THRESHOLD: Duration = Duration::from_millis(50);
const STATS_INTERVAL: u64 = 300;
const REALTIME_STATS_INTERVAL: Duration = Duration::from_secs(1);
const CONTROL_POLL_INTERVAL: Duration = Duration::from_millis(20);

struct AudioPlayback {
    _stream: OutputStream,
    sink: Sink,
    paused: bool,
    path: PathBuf,
}

struct FfmpegAudioSource {
    decoder: FfmpegAudioDecoder,
    info: AudioInfo,
    pending: std::vec::IntoIter<f32>,
    failed: bool,
}

impl FfmpegAudioSource {
    fn open(path: &Path) -> Result<Self> {
        let (decoder, info) = FfmpegAudioDecoder::open(path)?;
        Ok(Self {
            decoder,
            info,
            pending: Vec::new().into_iter(),
            failed: false,
        })
    }
}

impl Iterator for FfmpegAudioSource {
    type Item = f32;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(sample) = self.pending.next() {
                return Some(sample);
            }
            if self.failed {
                return None;
            }
            match self.decoder.next_samples() {
                Ok(samples) => self.pending = samples.into_iter(),
                Err(VideoError::EndOfStream) => return None,
                Err(error) => {
                    warn!(%error, "niri audio decoding stopped");
                    self.failed = true;
                }
            }
        }
    }
}

impl Source for FfmpegAudioSource {
    fn current_span_len(&self) -> Option<usize> {
        None
    }

    fn channels(&self) -> u16 {
        self.info.channels
    }

    fn sample_rate(&self) -> u32 {
        self.info.sample_rate
    }

    fn total_duration(&self) -> Option<Duration> {
        None
    }
}

impl AudioPlayback {
    fn open(path: &Path) -> Result<Self> {
        let stream = OutputStreamBuilder::open_default_stream()
            .context("failed to open default audio output")?;
        let sink = Sink::connect_new(stream.mixer());
        let source = FfmpegAudioSource::open(path)
            .with_context(|| format!("failed to decode audio source {}", path.display()))?;
        sink.append(source);
        sink.pause();
        info!(path = %path.display(), "niri audio output initialized for video-controlled looping");
        Ok(Self {
            _stream: stream,
            sink,
            paused: true,
            path: path.to_path_buf(),
        })
    }

    fn set_paused(&mut self, paused: bool) {
        if self.paused == paused {
            return;
        }
        if paused {
            self.sink.pause();
        } else {
            self.sink.play();
        }
        self.paused = paused;
        info!(paused, "niri audio pause state updated");
    }

    fn restart_for_video_loop(&mut self) -> Result<()> {
        let source = FfmpegAudioSource::open(&self.path)
            .with_context(|| format!("failed to restart audio source {}", self.path.display()))?;
        self.sink.clear();
        self.sink.append(source);
        self.sink.play();
        self.paused = false;
        info!(path = %self.path.display(), "audio restarted at video loop boundary");
        Ok(())
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
        debug!(
            decoded_fps = self.decoded as f64 / seconds,
            submitted_fps = self.presented as f64 / seconds,
            late_fps = self.dropped as f64 / seconds,
            present_avg_ms = self.present_time.as_secs_f64() * 1000.0 / presented,
            callback_wait_avg_ms = self.callback_wait.as_secs_f64() * 1000.0 / presented,
            scale_avg_ms = self.scale.as_secs_f64() * 1000.0 / presented,
            buffer_avg_ms = self.buffer_allocate.as_secs_f64() * 1000.0 / presented,
            submit_avg_ms = self.submit.as_secs_f64() * 1000.0 / presented,
            output_count,
            "niri realtime playback performance"
        );
        *self = Self::new();
    }
}

/// 运行软件解码和 headless 帧消费闭环。同步队列会在消费者落后时对解码线程施加背压。
pub fn run_headless(
    path: PathBuf,
    loop_playback: bool,
    hardware: bool,
    max_height: u32,
) -> Result<()> {
    run_headless_controlled(
        path,
        loop_playback,
        hardware,
        max_height,
        PlaybackControl::default(),
    )
    .map(|_| ())
}

/// 在指定时间后通过正常取消路径结束播放，供稳定性测试和自动化验收使用。
pub fn run_headless_for(
    path: PathBuf,
    loop_playback: bool,
    hardware: bool,
    max_height: u32,
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
                    "playback deadline reached, starting graceful shutdown"
                );
                timer_control.cancel();
            }
        })
        .context("failed to create playback deadline thread")?;

    let result = run_headless_controlled(path, loop_playback, hardware, max_height, control);
    let _ = finished_tx.send(());
    timer
        .join()
        .map_err(|_| anyhow::anyhow!("playback deadline thread panicked"))?;
    result
}

/// 在 niri background layer 上按 PTS 播放视频。多个输出共享同一个解码结果。
pub fn run_niri(
    path: PathBuf,
    loop_playback: bool,
    play_audio: bool,
    hardware: bool,
    max_height: u32,
    fill_mode: FillMode,
    output_names: &[String],
) -> Result<()> {
    run_niri_controlled(
        path,
        loop_playback,
        play_audio,
        hardware,
        max_height,
        fill_mode,
        output_names,
        PlaybackControl::default(),
    )
}

/// 在 niri background layer 上播放视频，并响应来自管理接口的暂停和取消命令。
pub fn run_niri_controlled(
    path: PathBuf,
    loop_playback: bool,
    play_audio: bool,
    hardware: bool,
    max_height: u32,
    fill_mode: FillMode,
    output_names: &[String],
    control: PlaybackControl,
) -> Result<()> {
    let mut backends = if output_names.is_empty() {
        vec![
            NiriBackend::connect(None)
                .context("failed to auto-select and initialize niri output")?,
        ]
    } else {
        output_names
            .iter()
            .map(|name| {
                NiriBackend::connect(Some(name))
                    .with_context(|| format!("failed to initialize niri output {name}"))
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
            let media = match decoder.open(
                &decode_path,
                DecodeOptions {
                    hardware,
                    max_height,
                },
            ) {
                Ok(media) => media,
                Err(error) => {
                    let message =
                        format!("failed to open video {}: {error}", decode_path.display());
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
                            break Err(
                                anyhow::anyhow!(error).context("niri loop playback seek failed")
                            );
                        }
                    }
                    Err(VideoError::EndOfStream) => break Ok(()),
                    Err(error) => break Err(anyhow::anyhow!(error)),
                }
            };
            let _ = decode_result_tx.send(result);
        })
        .context("failed to create niri decode thread")?;
    let media = startup_rx
        .recv()
        .context("niri decode thread did not return media info")?
        .map_err(anyhow::Error::msg)?;
    let mut audio = if play_audio {
        match AudioPlayback::open(&path) {
            Ok(audio) => Some(audio),
            Err(error) => {
                warn!(%error, path = %path.display(), "niri audio unavailable; continuing with video only");
                None
            }
        }
    } else {
        info!(path = %path.display(), "niri audio disabled by configuration");
        None
    };
    info!(
        path = %path.display(),
        outputs = ?backends
            .iter()
            .map(|backend| (backend.output_name(), backend.size()))
            .collect::<Vec<_>>(),
        output_count = backends.len(),
        video_size = ?(media.width, media.height),
        ?fill_mode,
        "niri video playback started"
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
            info!(path = %path.display(), "niri playback responded to cancel request");
            break;
        }
        if control.is_paused() {
            if let Some(audio) = audio.as_mut() {
                audio.set_paused(true);
            }
            for backend in &mut backends {
                backend.dispatch_pending().with_context(|| {
                    format!(
                        "failed to dispatch events for niri output {} during pause",
                        backend.output_name()
                    )
                })?;
            }
            clock = None;
            thread::sleep(CONTROL_POLL_INTERVAL);
            continue;
        }
        if first_frame_presented && let Some(audio) = audio.as_mut() {
            audio.set_paused(false);
        }
        match frames_rx.recv_timeout(CONTROL_POLL_INTERVAL) {
            Ok(frame) => {
                perf.decoded += 1;
                let pts = frame.presentation_time();
                if pts < previous_pts {
                    loops += 1;
                    if let Some(clock) = clock.as_mut() {
                        clock.reset(Instant::now(), pts);
                    }
                    if let Some(audio) = audio.as_mut() {
                        audio.restart_for_video_loop()?;
                    }
                }
                if clock.is_none() {
                    clock = Some(PlaybackClock::new(Instant::now(), pts, DROP_THRESHOLD));
                }
                previous_pts = pts;
                loop {
                    let decision = clock
                        .as_ref()
                        .expect("clock initialized")
                        .decide(Instant::now(), pts);
                    match decision {
                        FrameDecision::Wait(duration) => {
                            thread::sleep(duration.min(CONTROL_POLL_INTERVAL));
                            for backend in &mut backends {
                                backend.dispatch_pending().with_context(|| {
                                    format!(
                                        "failed to dispatch events for niri output {}",
                                        backend.output_name()
                                    )
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
                                        format!(
                                            "failed to submit frame to niri output {}",
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
                            let present_elapsed = present_started.elapsed();
                            perf.present_time += present_elapsed;
                            if !first_frame_presented {
                                clock
                                    .as_mut()
                                    .expect("clock initialized")
                                    .reset(Instant::now(), pts);
                                first_frame_presented = true;
                                if let Some(audio) = audio.as_mut() {
                                    audio.set_paused(false);
                                }
                                info!(
                                    present_ms = present_elapsed.as_millis(),
                                    "first frame submitted, playback clock excludes backend initialization time"
                                );
                            } else if presented.is_multiple_of(STATS_INTERVAL) {
                                debug!(
                                    presented,
                                    dropped,
                                    present_ms = present_elapsed.as_millis(),
                                    "niri present statistics"
                                );
                            }
                            break;
                        }
                        FrameDecision::Drop => {
                            dropped += 1;
                            perf.dropped += 1;
                            debug!(
                                pts_ms = pts.as_millis(),
                                dropped, "niri video frame exceeded the video timeline tolerance"
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
        .map_err(|_| anyhow::anyhow!("niri decode thread panicked"))?;
    decode_result_rx
        .recv()
        .context("niri decode thread did not return a result")??;
    info!(path = %path.display(), output_count = backends.len(), presented, dropped, loops, "niri video playback ended");
    Ok(())
}

pub fn run_headless_controlled(
    path: PathBuf,
    loop_playback: bool,
    hardware: bool,
    max_height: u32,
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
                max_height,
                frames_tx,
                decode_control,
            );
            let _ = result_tx.send(result);
        })
        .context("failed to create video decode thread")?;

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
                info!(
                    loops = stats.loops,
                    "loop start detected, playback clock reset"
                );
            }
            Some(_) => {}
        }
        previous_pts = pts;

        loop {
            match clock
                .as_ref()
                .expect("playback clock initialized")
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
                        "headless consumer dropped stale frame"
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
                "headless playback statistics"
            );
        }
    }

    decoder_thread
        .join()
        .map_err(|_| anyhow::anyhow!("video decode thread panicked"))?;
    result_rx
        .recv()
        .context("video decode thread did not return a result")??;
    info!(path = %path.display(), presented = stats.presented, dropped = stats.dropped, loops = stats.loops, "headless playback ended");
    Ok(stats)
}

fn decode_frames(
    path: PathBuf,
    loop_playback: bool,
    hardware: bool,
    max_height: u32,
    sender: better_wallpaper_ffmpeg::FrameQueueSender,
    control: PlaybackControl,
) -> Result<()> {
    let mut decoder = FfmpegDecoder::new();
    let media = decoder
        .open(
            &path,
            DecodeOptions {
                hardware,
                max_height,
            },
        )
        .with_context(|| format!("failed to open video {}", path.display()))?;
    info!(path = %path.display(), width = media.width, height = media.height, duration_ms = ?media.duration.map(|value| value.as_millis()), frame_rate = ?media.frame_rate, queue_capacity = FRAME_QUEUE_CAPACITY, "headless decode started");

    loop {
        if control.is_cancelled() {
            info!("decode thread responded to cancel request");
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
                            info!(
                                "decode thread responded to cancel request while waiting for frame queue"
                            );
                            return Ok(());
                        }
                        thread::sleep(CONTROL_POLL_INTERVAL);
                    }
                    Err(mpsc::TrySendError::Disconnected(_)) => {
                        bail!("headless consumer stopped")
                    }
                }
            },
            Err(VideoError::EndOfStream) if loop_playback => {
                decoder.seek_start().context("loop playback seek failed")?;
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
}
