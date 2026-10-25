//! Package video layers scheduled by the shared scene clock.
use std::{
    collections::HashSet,
    io::Write,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{TryRecvError, TrySendError},
    },
    thread::JoinHandle,
    time::Duration,
};

use anyhow::{Context, Result};
use better_wallpaper_core::{
    DecodeOptions, DecodedFrame, Rational, SceneQuality, VideoDecoder, VideoError,
};
use better_wallpaper_ffmpeg::{FfmpegDecoder, FrameQueueReceiver, frame_queue};
use better_wallpaper_renderer::{Scene2dAssets, SceneVideoFrame, SceneVideoTexture};
use tempfile::{Builder, NamedTempFile};
use tracing::{info, warn};

const MAX_VIDEO_TEXTURES: usize = 8;
const MAX_VIDEO_BYTES: usize = 128 * 1024 * 1024;
const MAX_FRAME_BYTES: usize = 64 * 1024 * 1024;

struct VideoWorker {
    resource: String,
    texture: SceneVideoTexture,
    receiver: FrameQueueReceiver,
    pending: Option<DecodedFrame>,
    cancel: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    generation: u64,
    finished: bool,
}

impl Drop for VideoWorker {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[derive(Default)]
pub struct SceneVideoRuntime {
    workers: Vec<VideoWorker>,
}

impl SceneVideoRuntime {
    pub fn start(assets: &Scene2dAssets, quality: SceneQuality) -> Result<Self> {
        Self::start_with_files(assets, quality, || {
            Builder::new()
                .prefix("better-wallpaper-scene-video-")
                .suffix(".media")
                .tempfile()
        })
    }

    fn start_with_files(
        assets: &Scene2dAssets,
        quality: SceneQuality,
        mut create_file: impl FnMut() -> std::io::Result<NamedTempFile>,
    ) -> Result<Self> {
        let mut runtime = Self::default();
        let mut seen = HashSet::new();
        for draw in &assets.draws {
            let Some(texture) = &draw.video else { continue };
            if !seen.insert(draw.texture_path.clone()) {
                continue;
            }
            if runtime.workers.len() >= MAX_VIDEO_TEXTURES
                || texture.encoded.len() > MAX_VIDEO_BYTES
            {
                warn!(resource = %draw.texture_path, "scene video resource limit exceeded; skipping video layer");
                continue;
            }
            let mut file = create_file().context("failed to create private scene video file")?;
            file.write_all(&texture.encoded)
                .context("failed to extract scene video payload")?;
            let (sender, receiver) = frame_queue(2);
            let cancel = Arc::new(AtomicBool::new(false));
            let worker_cancel = Arc::clone(&cancel);
            let resource = draw.texture_path.clone();
            let worker_resource = resource.clone();
            let max_height = match quality {
                SceneQuality::Low => 720,
                SceneQuality::Medium => 1080,
                SceneQuality::High => 2160,
            };
            let thread = std::thread::Builder::new().name("scene-video".into()).spawn(move || {
                let decode = || -> Result<()> {
                    let mut decoder = FfmpegDecoder::for_package_media();
                    let media = decoder.open(file.path(), DecodeOptions { hardware: false, max_height })?;
                    anyhow::ensure!(media.width <= 8192 && media.height <= 8192, "scene video dimensions exceed safety limit");
                    let frame_interval = Duration::from_secs_f64(1.0 / media.frame_rate.unwrap_or(30.0).clamp(1.0, 240.0));
                    let mut loop_start = Duration::ZERO;
                    let mut first_pts = None;
                    let mut last_time: Option<Duration> = None;
                    let mut loop_has_frames = false;
                    while !worker_cancel.load(Ordering::Acquire) {
                        let mut frame = match decoder.next_frame() {
                            Ok(frame) => frame,
                            Err(VideoError::EndOfStream) if loop_has_frames => {
                                // Use the last frame's end, not wall time: pause and
                                // delayed rendering cannot introduce loop clock drift.
                                loop_start = loop_start.saturating_add(media.duration.unwrap_or_default()
                                    .max(last_time.unwrap_or_default().saturating_sub(loop_start) + frame_interval));
                                first_pts = None; loop_has_frames = false;
                                decoder.seek_start()?;
                                continue;
                            }
                            Err(error) => return Err(error.into()),
                        };
                        anyhow::ensure!(frame.cuda.is_none() && frame.width > 0 && frame.height > 0
                            && frame.stride == frame.width as usize * 4
                            && frame.pixels.len() == frame.stride * frame.height as usize
                            && frame.pixels.len() <= MAX_FRAME_BYTES, "invalid or oversized scene video frame");
                        let pts = frame.presentation_time();
                        let origin = *first_pts.get_or_insert(pts);
                        let time = loop_start.saturating_add(pts.saturating_sub(origin));
                        let time = last_time.map_or(time, |last: Duration| time.max(last + Duration::from_micros(1)));
                        last_time = Some(time); loop_has_frames = true;
                        frame.pts = time.as_micros().min(i64::MAX as u128) as i64;
                        frame.time_base = Rational { numerator: 1, denominator: 1_000_000 };
                        loop {
                            if worker_cancel.load(Ordering::Acquire) { return Ok(()); }
                            match sender.try_send(frame) {
                                Ok(()) => break,
                                Err(TrySendError::Full(returned)) => {
                                    frame = returned;
                                    std::thread::sleep(Duration::from_millis(5));
                                }
                                Err(TrySendError::Disconnected(_)) => return Ok(()),
                            }
                        }
                    }
                    Ok(())
                };
                if let Err(error) = decode() {
                    warn!(resource = %worker_resource, %error, "scene video decoder stopped; retaining last valid frame");
                }
                // The private file stays alive until FFmpeg and the worker exit.
                drop(file);
            }).context("failed to start scene video decoder")?;
            info!(%resource, max_height, "scene video decoder started with bounded queue");
            runtime.workers.push(VideoWorker {
                resource,
                texture: texture.clone(),
                receiver,
                pending: None,
                cancel,
                thread: Some(thread),
                generation: 0,
                finished: false,
            });
        }
        Ok(runtime)
    }

    /// Keep only the newest due frame. Future frames wait and stale frames are
    /// released immediately. A frozen scene clock therefore freezes every video.
    pub fn update(&mut self, elapsed: Duration) -> Result<()> {
        for worker in &mut self.workers {
            let mut newest = None;
            // Bound work even if a decoder keeps refilling the queue during a
            // large clock jump. Further catch-up happens on subsequent ticks.
            for _ in 0..32 {
                if worker.pending.is_none() {
                    match worker.receiver.try_recv() {
                        Ok(frame) => worker.pending = Some(frame),
                        Err(TryRecvError::Empty) => break,
                        Err(TryRecvError::Disconnected) => {
                            if !worker.finished {
                                info!(resource = %worker.resource, "scene video frame queue closed");
                                worker.finished = true;
                            }
                            break;
                        }
                    }
                }
                if worker
                    .pending
                    .as_ref()
                    .is_some_and(|frame| frame.presentation_time() > elapsed)
                {
                    break;
                }
                newest = worker.pending.take();
            }
            if let Some(frame) = newest {
                worker.generation += 1;
                *worker
                    .texture
                    .frame
                    .write()
                    .map_err(|_| anyhow::anyhow!("scene video frame lock poisoned"))? =
                    Some(SceneVideoFrame {
                        rgba: Arc::from(frame.pixels),
                        width: frame.width,
                        height: frame.height,
                        generation: worker.generation,
                    });
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use better_wallpaper_renderer::{Scene2dOptions, build_scene_2d_plan, resolve_scene_2d_assets};
    use better_wallpaper_scene_format::{PkgReader, parse_scene_graph};

    fn assets() -> Scene2dAssets {
        let media = include_bytes!("../../../tests/fixtures/scene-video.mp4");
        let string = |s: &str| {
            let mut b = (s.len() as u32).to_le_bytes().to_vec();
            b.extend_from_slice(s.as_bytes());
            b
        };
        let mut pkg = string("PKGV0001");
        pkg.extend(1_u32.to_le_bytes());
        pkg.extend(string("videos/colors.mp4"));
        pkg.extend(0_u32.to_le_bytes());
        pkg.extend((media.len() as u32).to_le_bytes());
        pkg.extend(media);
        let pkg = PkgReader::parse(pkg).unwrap();
        let graph = parse_scene_graph(r#"{"objects":[{"id":1,"video":"videos/colors.mp4","size":"16 16"},{"id":2,"video":"videos/colors.mp4","size":"16 16"}]}"#).unwrap();
        let plan = build_scene_2d_plan(
            &graph,
            Scene2dOptions {
                viewport_width: 16,
                viewport_height: 16,
            },
        )
        .unwrap();
        resolve_scene_2d_assets(&pkg, plan).unwrap()
    }

    fn wait_for_color(
        runtime: &mut SceneVideoRuntime,
        texture: &SceneVideoTexture,
        elapsed: Duration,
        color: [u8; 4],
    ) -> u64 {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            runtime.update(elapsed).unwrap();
            if let Some(frame) = texture.frame.read().unwrap().as_ref()
                && frame.rgba[..4] == color
                && runtime.workers.iter().all(|worker| {
                    worker.finished
                        || worker
                            .pending
                            .as_ref()
                            .is_some_and(|pending| pending.presentation_time() > elapsed)
                })
            {
                return frame.generation;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "video did not reach expected color {color:?}"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn rejects_playlists_and_corrupt_media_without_blocking_teardown() {
        for payload in [
            &b"#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:1\n#EXTINF:1,\nhttps://example.com/media.ts\n"[..],
            &include_bytes!("../../../tests/fixtures/corrupt-video.bin")[..],
        ] {
            let mut assets = embedded_assets();
            assets.draws[0].video.as_mut().unwrap().encoded = Arc::from(payload);
            let directory = tempfile::tempdir().unwrap();
            let mut runtime = SceneVideoRuntime::start_with_files(&assets, SceneQuality::Low, || {
                Builder::new().tempfile_in(directory.path())
            }).unwrap();
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            while !runtime.workers[0].finished {
                runtime.update(Duration::ZERO).unwrap();
                assert!(std::time::Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(5));
            }
            assert!(assets.draws[0].video.as_ref().unwrap().frame.read().unwrap().is_none());
            drop(runtime);
            assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
        }
    }

    #[test]
    fn decodes_by_scene_pts_freezes_loops_and_shares_outputs() {
        let assets = assets();
        let a = assets.draws[0].video.as_ref().unwrap();
        let b = assets.draws[1].video.as_ref().unwrap();
        assert!(Arc::ptr_eq(&a.frame, &b.frame));
        let mut runtime = SceneVideoRuntime::start(&assets, SceneQuality::High).unwrap();
        assert_eq!(runtime.workers.len(), 1);
        let generation = wait_for_color(&mut runtime, a, Duration::ZERO, [255, 0, 0, 255]);
        // A decoder can fill its queue during pause; the visible frame is frozen.
        std::thread::sleep(Duration::from_millis(50));
        runtime.update(Duration::ZERO).unwrap();
        assert_eq!(
            a.frame.read().unwrap().as_ref().unwrap().generation,
            generation
        );
        wait_for_color(
            &mut runtime,
            a,
            Duration::from_millis(500),
            [0, 255, 0, 255],
        );
        wait_for_color(
            &mut runtime,
            a,
            Duration::from_millis(1500),
            [255, 255, 0, 255],
        );
        wait_for_color(&mut runtime, a, Duration::from_secs(2), [255, 0, 0, 255]);
        assert_eq!(
            b.frame.read().unwrap().as_ref().unwrap().rgba[..4],
            [255, 0, 0, 255]
        );
        let cancel = Arc::clone(&runtime.workers[0].cancel);
        let stopped = std::time::Instant::now();
        drop(runtime); // Must join even with a full queue.
        assert!(cancel.load(Ordering::Acquire));
        assert!(stopped.elapsed() < Duration::from_secs(1));
    }

    fn embedded_assets() -> Scene2dAssets {
        let pkg = PkgReader::parse(
            include_bytes!("../../../tests/fixtures/scene-behavior/video/scene.pkg").to_vec(),
        )
        .unwrap();
        let scene = pkg.read_entry(pkg.find("scene.json").unwrap());
        let graph = parse_scene_graph(std::str::from_utf8(scene).unwrap()).unwrap();
        let plan = build_scene_2d_plan(
            &graph,
            Scene2dOptions {
                viewport_width: 32,
                viewport_height: 32,
            },
        )
        .unwrap();
        resolve_scene_2d_assets(&pkg, plan).unwrap()
    }

    #[test]
    fn texb4_clock_pause_loop_reload_release_files_workers_and_frames() {
        use std::os::unix::fs::PermissionsExt;
        let directory = tempfile::tempdir().unwrap();
        for _reload in 0..3 {
            let mut assets = embedded_assets();
            assets.draws.push(assets.draws[0].clone());
            let a = assets.draws[0].video.as_ref().unwrap();
            let b = assets.draws[1].video.as_ref().unwrap();
            assert!(Arc::ptr_eq(&a.frame, &b.frame));
            let encoded = Arc::downgrade(&a.encoded);
            let mut paths = Vec::new();
            let mut runtime =
                SceneVideoRuntime::start_with_files(&assets, SceneQuality::High, || {
                    let file = Builder::new().tempfile_in(directory.path())?;
                    paths.push(file.path().to_path_buf());
                    Ok(file)
                })
                .unwrap();
            assert_eq!(runtime.workers.len(), 1);
            assert_eq!(paths.len(), 1);
            assert_eq!(
                std::fs::metadata(&paths[0]).unwrap().permissions().mode() & 0o777,
                0o600
            );
            for (millis, color) in [
                (0, [255, 0, 0, 255]),
                (499, [255, 0, 0, 255]),
                (500, [0, 255, 0, 255]),
                (999, [0, 255, 0, 255]),
                (1000, [0, 0, 255, 255]),
                (1499, [0, 0, 255, 255]),
                (1500, [255, 255, 0, 255]),
                (1999, [255, 255, 0, 255]),
                (2000, [255, 0, 0, 255]),
                (4500, [0, 255, 0, 255]),
            ] {
                let time = Duration::from_millis(millis);
                let generation = wait_for_color(&mut runtime, a, time, color);
                std::thread::sleep(Duration::from_millis(15));
                runtime.update(time).unwrap();
                assert_eq!(
                    a.frame.read().unwrap().as_ref().unwrap().generation,
                    generation
                );
                assert_eq!(b.frame.read().unwrap().as_ref().unwrap().rgba[..4], color);
            }
            let frame = Arc::downgrade(&a.frame.read().unwrap().as_ref().unwrap().rgba);
            let cancel = Arc::clone(&runtime.workers[0].cancel);
            let stopped = std::time::Instant::now();
            drop(runtime);
            assert!(stopped.elapsed() < Duration::from_secs(1));
            assert!(cancel.load(Ordering::Acquire));
            assert!(!paths[0].exists());
            drop(assets);
            assert!(encoded.upgrade().is_none());
            assert!(frame.upgrade().is_none());
            assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
        }
    }

    #[test]
    fn texb4_decoder_limit_and_partial_start_failure_release_private_files() {
        let directory = tempfile::tempdir().unwrap();
        let mut assets = embedded_assets();
        let template = assets.draws[0].clone();
        assets.draws.clear();
        for index in 0..=MAX_VIDEO_TEXTURES {
            let mut draw = template.clone();
            draw.texture_path = format!("materials/video-{index}.tex");
            draw.video.as_mut().unwrap().frame = Arc::new(std::sync::RwLock::new(None));
            assets.draws.push(draw);
        }
        let mut calls = 0;
        let runtime = SceneVideoRuntime::start_with_files(&assets, SceneQuality::Low, || {
            calls += 1;
            Builder::new().tempfile_in(directory.path())
        })
        .unwrap();
        assert_eq!(runtime.workers.len(), MAX_VIDEO_TEXTURES);
        assert_eq!(calls, MAX_VIDEO_TEXTURES);
        let cancellations: Vec<_> = runtime.workers.iter().map(|w| w.cancel.clone()).collect();
        drop(runtime);
        assert!(cancellations.iter().all(|c| c.load(Ordering::Acquire)));
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
        calls = 0;
        let failure = SceneVideoRuntime::start_with_files(&assets, SceneQuality::Low, || {
            calls += 1;
            if calls == 2 {
                return Err(std::io::Error::other("injected extraction failure"));
            }
            Builder::new().tempfile_in(directory.path())
        });
        assert!(failure.is_err());
        assert_eq!(calls, 2);
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
    }

    #[test]
    #[ignore = "requires an explicit legally available local video TEX; set BW_LOCAL_VIDEO_TEX"]
    fn local_video_tex_decodes_and_releases_runtime_resources() {
        let path = std::env::var_os("BW_LOCAL_VIDEO_TEX").expect("set BW_LOCAL_VIDEO_TEX");
        assert!(std::fs::metadata(&path).unwrap().len() <= 128 * 1024 * 1024);
        let bytes = std::fs::read(&path).unwrap();
        let atlas = better_wallpaper_scene_format::TexAtlas::parse(&bytes).unwrap();
        assert!(atlas.is_video);
        let mut assets = embedded_assets();
        let video = assets.draws[0].video.as_mut().unwrap();
        video.encoded = atlas.images[0].mipmaps[0].data.clone();
        let directory = tempfile::tempdir().unwrap();
        let mut runtime = SceneVideoRuntime::start_with_files(&assets, SceneQuality::Low, || {
            Builder::new().tempfile_in(directory.path())
        })
        .unwrap();
        let texture = assets.draws[0].video.as_ref().unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        loop {
            runtime.update(Duration::ZERO).unwrap();
            if let Some(frame) = texture.frame.read().unwrap().as_ref() {
                assert!(frame.height <= 720);
                assert_eq!(
                    frame.rgba.len(),
                    frame.width as usize * frame.height as usize * 4
                );
                println!(
                    "local TEXB{}: {}x{} -> {}x{}, retained {} RGBA bytes",
                    atlas.container_version,
                    atlas.width,
                    atlas.height,
                    frame.width,
                    frame.height,
                    frame.rgba.len()
                );
                break;
            }
            assert!(!runtime.workers[0].finished, "local media decoder failed");
            assert!(
                std::time::Instant::now() < deadline,
                "local media decode timed out"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        drop(runtime);
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
    }
}
