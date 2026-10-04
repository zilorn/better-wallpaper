use std::{
    ffi::OsString,
    path::PathBuf,
    sync::{Arc, RwLock},
    time::Duration,
};

use anyhow::{Context, Result};
use better_wallpaper_core::{
    BackendKind, ConfigStore, PlaybackControl, WallpaperType,
    desktop::{ProcessEnvironment, detect_desktop, select_backend},
};
use better_wallpaper_daemon::{
    LogLevelController, LogStore, desktop_audio::DesktopAudioCapture, playback,
    scene_audio::PreparedSceneAudio, server,
};
use better_wallpaper_kde::run_kde_controlled;
use better_wallpaper_renderer::{
    NvidiaVulkanContext, Scene2dOptions, build_scene_2d_plan, resolve_scene_2d_assets,
};
use better_wallpaper_scene_format::PkgReader;
use clap::{Parser, ValueEnum};
use tracing::{debug, error, info, warn};
use tracing_subscriber::{Layer, filter::filter_fn, layer::SubscriberExt};

const MANAGEMENT_BIND: &str = "127.0.0.1:43129";

#[derive(Debug, Parser)]
#[command(version, about = "Linux video wallpaper service")]
struct Cli {
    #[arg(long)]
    config: Option<PathBuf>,
    #[arg(long, value_enum)]
    backend: Option<CliBackend>,
    #[arg(long)]
    no_ui: bool,
    /// Override the configured log level for this process
    #[arg(long, value_parser = ["info", "debug"])]
    log_level: Option<String>,
    /// Require NVIDIA Vulkan/DMA-BUF to be available when starting niri, otherwise exit
    #[arg(long)]
    require_nvidia: bool,
    /// Exit normally after the specified seconds; used for headless stability tests
    #[arg(long, value_name = "SECONDS", value_parser = clap::value_parser!(u64).range(1..))]
    run_for_seconds: Option<u64>,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum CliBackend {
    Auto,
    Niri,
    Kde,
    Headless,
}

impl From<CliBackend> for BackendKind {
    fn from(value: CliBackend) -> Self {
        match value {
            CliBackend::Auto => Self::Auto,
            CliBackend::Niri => Self::Niri,
            CliBackend::Kde => Self::Kde,
            CliBackend::Headless => Self::Headless,
        }
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .context("HOME not set")?;
    let path = cli.config.clone().unwrap_or(ConfigStore::default_path()?);
    let mut config = ConfigStore::new(path.clone(), home.clone()).load_or_create()?;
    if let Some(level) = cli.log_level.as_deref() {
        config.general.log_level = level.to_owned();
    }
    let log_level = LogLevelController::new(&config.general.log_level);
    let log_store = LogStore::new(2000);
    let log_filter = log_level.clone();
    tracing::subscriber::set_global_default(
        tracing_subscriber::registry().with(
            tracing_subscriber::fmt::layer()
                .with_writer(log_store.clone())
                .with_target(true)
                .with_filter(filter_fn(move |metadata| log_filter.enabled(metadata))),
        ),
    )
    .context("failed to initialize logging")?;
    std::panic::set_hook(Box::new(|panic| error!(%panic, "process panicked")));
    let detection = detect_desktop(&ProcessEnvironment);
    let backend = select_backend(
        cli.backend.map(Into::into),
        config.general.backend,
        detection.kind,
    );

    info!(config = %path.display(), desktop = ?detection.kind, evidence = %detection.evidence, candidates = ?detection.candidates, backend = ?backend, ui_enabled = !cli.no_ui, "startup config parsed");
    let _nvidia_renderer = if backend == BackendKind::Niri {
        match NvidiaVulkanContext::new() {
            Ok(context) => Some(context),
            Err(error) if cli.require_nvidia => {
                return Err(error)
                    .context("NVIDIA GPU rendering is required but initialization failed");
            }
            Err(error) => {
                warn!(%error, "NVIDIA Vulkan/DMA-BUF unavailable, falling back to CPU wl_shm rendering");
                None
            }
        }
    } else {
        info!(
            ?backend,
            "current backend does not initialize NVIDIA niri renderer"
        );
        None
    };
    if !cli.no_ui {
        let shared_config = Arc::new(RwLock::new(config));
        let playback_config = Arc::clone(&shared_config);
        let playback_control = PlaybackControl::default();
        let worker_control = playback_control.clone();
        std::thread::Builder::new()
            .name("wallpaper-playback".into())
            .spawn(move || {
                if let Err(error) = run_playback_supervisor(
                    backend,
                    playback_config,
                    cli.run_for_seconds,
                    worker_control,
                    true,
                ) {
                    error!(%error, "playback supervisor exited");
                }
            })
            .context("failed to create wallpaper playback thread")?;
        let web_root = resolve_web_root(std::env::var_os("BETTER_WALLPAPER_WEB_ROOT"));
        return server::serve(
            MANAGEMENT_BIND,
            web_root,
            server::ApiState::new(
                shared_config,
                ConfigStore::new(path, home.clone()),
                detection,
                backend,
                playback_control,
                home,
                log_store,
                log_level,
            ),
        );
    }
    run_playback_supervisor(
        backend,
        Arc::new(RwLock::new(config)),
        cli.run_for_seconds,
        PlaybackControl::default(),
        false,
    )
}

const PLAYBACK_RETRY_INITIAL: Duration = Duration::from_millis(250);
const PLAYBACK_RETRY_MAX: Duration = Duration::from_secs(5);
const PLAYBACK_RETRY_RESET: Duration = Duration::from_secs(30);

fn run_playback_supervisor(
    backend: BackendKind,
    config: Arc<RwLock<better_wallpaper_core::AppConfig>>,
    run_for_seconds: Option<u64>,
    control: PlaybackControl,
    idle_after_success: bool,
) -> Result<()> {
    supervise_playback(
        backend,
        config,
        run_for_seconds,
        control,
        idle_after_success,
        run_playback,
    )
}

fn supervise_playback(
    backend: BackendKind,
    config: Arc<RwLock<better_wallpaper_core::AppConfig>>,
    run_for_seconds: Option<u64>,
    control: PlaybackControl,
    idle_after_success: bool,
    mut run: impl FnMut(
        BackendKind,
        better_wallpaper_core::AppConfig,
        Option<u64>,
        PlaybackControl,
    ) -> Result<()>,
) -> Result<()> {
    let mut retry_delay = PLAYBACK_RETRY_INITIAL;
    loop {
        if control.is_cancelled() {
            if control.take_reload_request() {
                retry_delay = PLAYBACK_RETRY_INITIAL;
            } else {
                return Ok(());
            }
        }
        let current = config
            .read()
            .map_err(|_| anyhow::anyhow!("config lock poisoned, playback supervisor exiting"))?
            .clone();
        control.set_running(true);
        let started = std::time::Instant::now();
        let result = run(backend, current, run_for_seconds, control.clone());
        control.set_running(false);
        if let Err(error) = &result {
            error!(error = %format!("{error:#}"), "wallpaper playback pipeline exited");
        }
        if control.take_reload_request() {
            retry_delay = PLAYBACK_RETRY_INITIAL;
            info!("rebuilding playback pipeline with latest config");
            continue;
        }
        if control.is_cancelled() {
            return Ok(());
        }
        if backend == BackendKind::Niri && result.is_err() {
            if started.elapsed() >= PLAYBACK_RETRY_RESET {
                retry_delay = PLAYBACK_RETRY_INITIAL;
            }
            warn!(
                delay_ms = retry_delay.as_millis(),
                "retrying niri playback after pipeline failure"
            );
            wait_for_playback_control(&control, Some(retry_delay));
            retry_delay = (retry_delay * 2).min(PLAYBACK_RETRY_MAX);
            continue;
        }
        if !idle_after_success {
            return result;
        }
        // Natural completion and empty/disabled configs stay idle until an update.
        wait_for_playback_control(&control, None);
    }
}

fn wait_for_playback_control(control: &PlaybackControl, timeout: Option<Duration>) {
    let started = std::time::Instant::now();
    while !control.is_cancelled() {
        let delay = match timeout {
            Some(timeout) => match timeout.checked_sub(started.elapsed()) {
                Some(remaining) if !remaining.is_zero() => remaining.min(Duration::from_millis(20)),
                _ => return,
            },
            None => Duration::from_millis(20),
        };
        std::thread::sleep(delay);
    }
}

fn resolve_web_root(configured: Option<OsString>) -> PathBuf {
    if let Some(path) = configured {
        return PathBuf::from(path);
    }
    let development = PathBuf::from("web/dist");
    if development.is_dir() {
        return development;
    }
    user_web_root(std::env::var_os("XDG_DATA_HOME"), std::env::var_os("HOME"))
}

fn user_web_root(xdg_data_home: Option<OsString>, home: Option<OsString>) -> PathBuf {
    xdg_data_home
        .map(PathBuf::from)
        .or_else(|| home.map(|home| PathBuf::from(home).join(".local/share")))
        .unwrap_or_else(|| PathBuf::from(".local/share"))
        .join("better-wallpaper/web")
}

fn run_playback(
    backend: BackendKind,
    config: better_wallpaper_core::AppConfig,
    run_for_seconds: Option<u64>,
    control: PlaybackControl,
) -> Result<()> {
    let max_height = config.decode.max_height;
    if config.wallpaper.wallpaper_type == WallpaperType::Scene {
        let Some(project_dir) = config.wallpaper.path.as_deref() else {
            info!("scene wallpaper path not configured, backend idle");
            return Ok(());
        };
        let scene_config = config.scene.clone().unwrap_or_default();
        info!(
            mouse = scene_config.mouse,
            parallax = scene_config.parallax,
            particle_limit = scene_config.particle_limit,
            "scene mouse interaction, parallax and particle limits are not implemented; saved values are ignored"
        );
        let prepared = prepare_scene(project_dir, &scene_config)?;
        info!(
            ?backend,
            draw_count = prepared.plan.quads.len(),
            resolved_draw_count = prepared.resolved_draw_count,
            skipped_nodes = prepared.plan.skipped_nodes,
            background_tracks = prepared.audio.track_count(),
            "scene wallpaper validated with shared 2D render semantics"
        );
        if let Some(error) = prepared.asset_error.as_deref() {
            warn!(?backend, %error, "scene assets are not ready for GPU submission");
            if backend == BackendKind::Kde {
                run_kde_controlled(control.clone());
            }
            return Ok(());
        }
        let assets = prepared.assets.expect("assets were validated above");
        match backend {
            BackendKind::Niri => {
                let output_names: Vec<_> = config
                    .outputs
                    .iter()
                    .filter(|o| o.enabled)
                    .map(|o| o.name.clone())
                    .collect();
                run_niri_scene(
                    assets,
                    prepared.audio,
                    !config.wallpaper.muted,
                    &output_names,
                    &scene_config,
                    control.clone(),
                )?;
            }
            BackendKind::Kde => {
                warn!("KDE Plasma scene rendering is not yet implemented through the GPU path");
                run_kde_controlled(control.clone());
            }
            BackendKind::Headless => {
                info!(draw_count = assets.draws.len(), "headless scene validated");
            }
            _ => {
                warn!(?backend, "scene rendering is not supported on this backend");
            }
        }
        // A scene project directory must never fall through into the video decoder.
        // Returning also drops all package/CPU/GPU resources before the supervisor
        // consumes a pending hot-reload request and constructs the replacement.
        return Ok(());
    }
    if config.wallpaper.wallpaper_type == WallpaperType::Web && backend != BackendKind::Kde {
        warn!(
            ?backend,
            "web wallpaper rendering is currently available through the Plasma wallpaper plugin only"
        );
        return Ok(());
    }
    if backend == BackendKind::Headless {
        if config.general.restore_on_start {
            if let Some(video_path) = config.wallpaper.path {
                let hardware = matches!(
                    config.decode.hardware,
                    better_wallpaper_core::config::HardwareDecode::Auto
                );
                if let Some(seconds) = run_for_seconds {
                    playback::run_headless_for(
                        video_path,
                        config.wallpaper.loop_playback,
                        hardware,
                        max_height,
                        Duration::from_secs(seconds),
                    )?;
                } else {
                    playback::run_headless_controlled(
                        video_path,
                        config.wallpaper.loop_playback,
                        hardware,
                        max_height,
                        control,
                    )?;
                }
            } else {
                info!("wallpaper.path not configured, headless backend idle");
            }
        } else {
            info!("restore_on_start=false, headless backend will not auto-play");
        }
    } else if backend == BackendKind::Niri {
        if config.general.restore_on_start {
            if let Some(video_path) = config.wallpaper.path.clone() {
                let output_names: Vec<_> = config
                    .outputs
                    .iter()
                    .filter(|output| output.enabled)
                    .map(|output| output.name.clone())
                    .collect();
                playback::run_niri_controlled(
                    video_path,
                    config.wallpaper.loop_playback,
                    !config.wallpaper.muted,
                    matches!(
                        config.decode.hardware,
                        better_wallpaper_core::config::HardwareDecode::Auto
                    ),
                    max_height,
                    config.wallpaper.fill_mode,
                    &output_names,
                    control,
                )?;
            } else {
                info!("wallpaper.path not configured, niri backend idle");
            }
        } else {
            info!("restore_on_start=false, niri backend will not auto-play");
        }
    } else if backend == BackendKind::Kde {
        info!("KDE media rendering is owned by the Plasma plugin");
        run_kde_controlled(control);
    } else {
        warn!(backend = ?backend, "desktop backend is unavailable");
    }
    Ok(())
}

struct PreparedScene {
    plan: better_wallpaper_renderer::Scene2dPlan,
    resolved_draw_count: usize,
    assets: Option<better_wallpaper_renderer::Scene2dAssets>,
    asset_error: Option<String>,
    audio: PreparedSceneAudio,
}

fn run_niri_scene(
    mut assets: better_wallpaper_renderer::Scene2dAssets,
    scene_audio: PreparedSceneAudio,
    play_background_audio: bool,
    output_names: &[String],
    scene_config: &better_wallpaper_core::SceneConfig,
    control: PlaybackControl,
) -> Result<()> {
    use std::time::Duration;

    let audio_response_draws = assets
        .draws
        .iter()
        .filter(|draw| draw.quad.audio_response.is_some())
        .count();
    let spectrum = Arc::new(better_wallpaper_renderer::SceneAudioSpectrum::default());
    let desktop_audio_capture = if scene_config.audio_processing && audio_response_draws > 0 {
        match DesktopAudioCapture::start(Arc::clone(&spectrum)) {
            Ok(capture) => {
                assets.audio_spectrum = Some(Arc::clone(&spectrum));
                Some(capture)
            }
            Err(error) => {
                warn!(%error, "desktop audio response unavailable; rendering without spectrum data");
                None
            }
        }
    } else {
        info!(
            enabled = scene_config.audio_processing,
            audio_response_draws, "desktop audio response capture not started"
        );
        None
    };

    let mut backends = if output_names.is_empty() {
        vec![
            better_wallpaper_wayland::NiriBackend::connect(None)
                .context("failed to auto-select niri output for scene")?,
        ]
    } else {
        output_names
            .iter()
            .map(|name| {
                better_wallpaper_wayland::NiriBackend::connect(Some(name))
                    .with_context(|| format!("failed to initialize niri output {name} for scene"))
            })
            .collect::<Result<Vec<_>>>()?
    };

    for backend in &mut backends {
        backend.load_scene_assets(assets.clone()).with_context(|| {
            format!(
                "failed to load scene assets for niri output {}",
                backend.output_name()
            )
        })?;
    }
    let mut scene_videos = better_wallpaper_daemon::scene_video::SceneVideoRuntime::start(
        &assets,
        scene_config.quality,
    )?;
    let mut background_audio = scene_audio.start(play_background_audio);
    info!(
        output_count = backends.len(),
        draw_count = assets.draws.len(),
        animated_draw_count = assets
            .draws
            .iter()
            .filter(|draw| {
                draw.animation.is_some()
                    || draw.quad.scroll.is_some()
                    || !draw.quad.water_waves.is_empty()
                    || draw.quad.water_flow.is_some()
                    || !draw.quad.shakes.is_empty()
                    || !draw.quad.pulses.is_empty()
                    || draw.quad.spin.is_some()
                    || draw.quad.iris.is_some()
                    || !draw.quad.foliage_sway.is_empty()
                    || draw.quad.shine.is_some()
            })
            .count(),
        water_wave_effects = assets
            .draws
            .iter()
            .map(|draw| draw.quad.water_waves.len())
            .sum::<usize>(),
        shake_effects = assets
            .draws
            .iter()
            .map(|draw| draw.quad.shakes.len())
            .sum::<usize>(),
        pulse_effects = assets
            .draws
            .iter()
            .map(|draw| draw.quad.pulses.len())
            .sum::<usize>(),
        spin_effects = assets
            .draws
            .iter()
            .filter(|draw| draw.quad.spin.is_some())
            .count(),
        "scene assets uploaded to niri GPU"
    );

    let fps_limit = match scene_config.quality {
        better_wallpaper_core::SceneQuality::Low => 30_u64,
        better_wallpaper_core::SceneQuality::Medium => 45_u64,
        better_wallpaper_core::SceneQuality::High => 60_u64,
    };
    info!(
        ?scene_config.quality,
        fps_limit,
        desktop_audio_capture = desktop_audio_capture.is_some(),
        audio_response_draws,
        background_tracks = background_audio.active_track_count(),
        property_override_count = scene_config.properties.len(),
        "scene runtime configuration applied"
    );
    let frame_interval = Duration::from_secs_f64(1.0 / fps_limit as f64);
    let mut scene_elapsed = Duration::ZERO;
    let mut previous_tick = std::time::Instant::now();
    let mut was_paused = false;
    let mut stats_started = std::time::Instant::now();
    let mut first_scene_frame_presented = false;
    let mut stats_frames = 0_u64;
    let mut stats_work = Duration::ZERO;
    let mut stats_max_work = Duration::ZERO;
    let mut stats_over_budget = 0_u64;
    while !control.is_cancelled() {
        let frame_start = std::time::Instant::now();
        let delta = frame_start.saturating_duration_since(previous_tick);
        previous_tick = frame_start;
        for backend in &mut backends {
            backend.dispatch_pending()?;
        }
        let outputs_available = backends
            .iter()
            .any(better_wallpaper_wayland::NiriBackend::has_output);
        let paused = control.is_paused();
        if let Some(capture) = &desktop_audio_capture {
            capture.set_paused(paused);
        }
        if !paused && outputs_available {
            scene_elapsed = scene_elapsed.saturating_add(delta);
        }
        if paused != was_paused {
            info!(
                paused,
                elapsed_seconds = scene_elapsed.as_secs_f64(),
                "scene clock pause state changed"
            );
            was_paused = paused;
        }
        scene_videos.update(scene_elapsed)?;
        let mut submitted = false;
        for backend in &mut backends {
            submitted |= backend
                .present_scene(scene_elapsed.as_secs_f64())
                .with_context(|| {
                    format!(
                        "failed to present scene frame to niri output {}",
                        backend.output_name()
                    )
                })?;
        }
        // Missing outputs do not prevent healthy outputs from starting audio.
        first_scene_frame_presented |= submitted;
        background_audio.set_paused(paused || !outputs_available || !first_scene_frame_presented);
        let work_elapsed = frame_start.elapsed();
        stats_frames += 1;
        stats_work = stats_work.saturating_add(work_elapsed);
        stats_max_work = stats_max_work.max(work_elapsed);
        stats_over_budget += u64::from(work_elapsed > frame_interval);
        let stats_elapsed = stats_started.elapsed();
        if stats_elapsed >= Duration::from_secs(1) {
            let average_work_us = stats_work.as_micros() as f64 / stats_frames.max(1) as f64;
            let spectrum_peak = (0..64).map(|bin| spectrum.get(bin)).fold(0.0_f32, f32::max);
            let audio_metrics = desktop_audio_capture
                .as_ref()
                .map(DesktopAudioCapture::take_metrics)
                .unwrap_or_default();
            debug!(
                frames = stats_frames,
                actual_fps = stats_frames as f64 / stats_elapsed.as_secs_f64(),
                target_fps = fps_limit,
                average_work_us,
                max_work_us = stats_max_work.as_micros() as u64,
                over_budget_frames = stats_over_budget,
                output_count = backends.len(),
                paused,
                desktop_capture = desktop_audio_capture.is_some(),
                captured_audio_bytes = audio_metrics.captured_bytes,
                spectrum_updates = audio_metrics.spectrum_updates,
                spectrum_update_hz =
                    audio_metrics.spectrum_updates as f64 / stats_elapsed.as_secs_f64(),
                spectrum_peak,
                scene_elapsed_seconds = scene_elapsed.as_secs_f64(),
                "scene realtime frame statistics"
            );
            stats_started = std::time::Instant::now();
            stats_frames = 0;
            stats_work = Duration::ZERO;
            stats_max_work = Duration::ZERO;
            stats_over_budget = 0;
        }
        let elapsed = frame_start.elapsed();
        if elapsed < frame_interval {
            std::thread::sleep(frame_interval - elapsed);
        }
    }
    drop(scene_videos);
    drop(background_audio);
    drop(scene_audio);
    drop(backends);
    drop(assets);
    info!("niri scene rendering stopped and GPU/CPU resources released");
    Ok(())
}
fn prepare_scene(
    project_dir: &std::path::Path,
    scene_config: &better_wallpaper_core::SceneConfig,
) -> Result<PreparedScene> {
    if !project_dir.is_dir() {
        anyhow::bail!(
            "scene wallpaper source is not a project directory: {}",
            project_dir.display()
        );
    }
    let package_path = project_dir.join("scene.pkg");
    let package_bytes = std::fs::read(&package_path)
        .with_context(|| format!("failed to read scene package: {}", package_path.display()))?;
    let package = PkgReader::parse(package_bytes)
        .with_context(|| format!("failed to parse scene package: {}", package_path.display()))?;
    let scene_entry = package
        .find("scene.json")
        .context("scene package does not contain scene.json")?;
    let scene_json = package
        .read_entry_string(scene_entry)
        .context("failed to read scene.json")?;
    let graph = better_wallpaper_scene_format::parse_scene_graph_with_properties(
        &scene_json,
        &scene_config.properties,
    )
    .context("failed to parse scene graph")?;
    let audio = PreparedSceneAudio::prepare(&package, &graph);
    let plan = build_scene_2d_plan(
        &graph,
        Scene2dOptions {
            viewport_width: 1920,
            viewport_height: 1080,
        },
    )
    .context("failed to build shared scene draw plan")?;
    let (resolved_draw_count, asset_error, assets) =
        match resolve_scene_2d_assets(&package, plan.clone()) {
            Ok(assets) => (assets.draws.len(), None, Some(assets)),
            Err(error) => (0, Some(error.to_string()), None),
        };
    Ok(PreparedScene {
        plan,
        resolved_draw_count,
        asset_error,
        assets,
        audio,
    })
}

#[cfg(test)]
mod tests {
    use std::{ffi::OsString, path::PathBuf};

    use super::user_web_root;

    #[test]
    fn niri_failure_retries_without_reload_in_no_ui_mode() {
        let config = std::sync::Arc::new(std::sync::RwLock::new(
            better_wallpaper_core::AppConfig::default(),
        ));
        let control = better_wallpaper_core::PlaybackControl::default();
        let mut attempts = 0;
        super::supervise_playback(
            better_wallpaper_core::BackendKind::Niri,
            config,
            None,
            control.clone(),
            false,
            |_, _, _, worker| {
                assert!(worker.is_running());
                attempts += 1;
                if attempts < 3 {
                    anyhow::bail!("compositor unavailable");
                }
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(attempts, 3);
        assert!(!control.is_running());
        assert!(!control.is_cancelled());
    }

    #[test]
    fn natural_completion_stays_idle_until_config_reload() {
        let config = std::sync::Arc::new(std::sync::RwLock::new(
            better_wallpaper_core::AppConfig::default(),
        ));
        let control = better_wallpaper_core::PlaybackControl::default();
        let worker_control = control.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            super::supervise_playback(
                better_wallpaper_core::BackendKind::Niri,
                config,
                None,
                worker_control,
                true,
                |_, _, _, _| {
                    tx.send(()).unwrap();
                    Ok(())
                },
            )
        });
        rx.recv_timeout(std::time::Duration::from_secs(2)).unwrap();
        assert!(
            rx.recv_timeout(std::time::Duration::from_millis(100))
                .is_err()
        );
        control.request_reload();
        rx.recv_timeout(std::time::Duration::from_secs(2)).unwrap();
        control.cancel();
        worker.join().unwrap().unwrap();
    }

    #[test]
    fn config_update_interrupts_retry_and_uses_latest_config() {
        let config = std::sync::Arc::new(std::sync::RwLock::new(
            better_wallpaper_core::AppConfig::default(),
        ));
        config.write().unwrap().wallpaper.muted = false;
        let control = better_wallpaper_core::PlaybackControl::default();
        let worker_config = std::sync::Arc::clone(&config);
        let worker_control = control.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            super::supervise_playback(
                better_wallpaper_core::BackendKind::Niri,
                worker_config,
                None,
                worker_control,
                false,
                |_, current, _, _| {
                    tx.send(current.wallpaper.muted).unwrap();
                    if !current.wallpaper.muted {
                        anyhow::bail!("connection lost");
                    }
                    Ok(())
                },
            )
        });
        assert!(!rx.recv_timeout(std::time::Duration::from_secs(2)).unwrap());
        config.write().unwrap().wallpaper.muted = true;
        control.request_reload();
        assert!(rx.recv_timeout(std::time::Duration::from_secs(2)).unwrap());
        worker.join().unwrap().unwrap();
        assert!(!control.is_cancelled());
    }

    #[test]
    fn cancellation_interrupts_backoff_without_another_attempt() {
        let config = std::sync::Arc::new(std::sync::RwLock::new(
            better_wallpaper_core::AppConfig::default(),
        ));
        let control = better_wallpaper_core::PlaybackControl::default();
        let worker_control = control.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            super::supervise_playback(
                better_wallpaper_core::BackendKind::Niri,
                config,
                None,
                worker_control,
                false,
                |_, _, _, _| {
                    tx.send(()).unwrap();
                    anyhow::bail!("connection lost")
                },
            )
        });
        rx.recv_timeout(std::time::Duration::from_secs(2)).unwrap();
        control.cancel();
        worker.join().unwrap().unwrap();
        assert!(rx.try_recv().is_err());
        assert!(!control.is_running());
    }

    #[test]
    fn defaults_web_root_to_user_data_directory() {
        assert_eq!(
            user_web_root(None, Some(OsString::from("/home/user"))),
            PathBuf::from("/home/user/.local/share/better-wallpaper/web")
        );
        assert_eq!(
            user_web_root(
                Some(OsString::from("/home/user/.data")),
                Some(OsString::from("/ignored"))
            ),
            PathBuf::from("/home/user/.data/better-wallpaper/web")
        );
    }
}
