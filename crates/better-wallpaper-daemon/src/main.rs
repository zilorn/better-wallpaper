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
use better_wallpaper_daemon::{LogStore, playback, server};
use better_wallpaper_kde::run_kde_controlled;
use better_wallpaper_renderer::{NvidiaVulkanContext, Scene2dOptions, build_scene_2d_plan};
use better_wallpaper_scene_format::{PkgReader, parse_scene_graph};
use clap::{Parser, ValueEnum};
use tracing::{error, info, warn};
use tracing_subscriber::EnvFilter;

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
    #[arg(long, default_value = "info")]
    log_level: String,
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
    let log_store = LogStore::new(2000);
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_new(&cli.log_level).context("invalid --log-level")?)
        .with_writer(log_store.clone())
        .with_target(true)
        .init();
    std::panic::set_hook(Box::new(|panic| error!(%panic, "process panicked")));

    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .context("HOME not set")?;
    let path = cli.config.unwrap_or(ConfigStore::default_path()?);
    let config = ConfigStore::new(path.clone(), home.clone()).load_or_create()?;
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
                run_playback_supervisor(
                    backend,
                    playback_config,
                    cli.run_for_seconds,
                    worker_control,
                );
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
            ),
        );
    }
    run_playback(
        backend,
        config,
        cli.run_for_seconds,
        PlaybackControl::default(),
    )
}

fn run_playback_supervisor(
    backend: BackendKind,
    config: Arc<RwLock<better_wallpaper_core::AppConfig>>,
    run_for_seconds: Option<u64>,
    control: PlaybackControl,
) {
    loop {
        let current = match config.read() {
            Ok(config) => config.clone(),
            Err(_) => {
                error!("config lock poisoned, playback supervisor exiting");
                return;
            }
        };
        control.set_running(true);
        let result = run_playback(backend, current, run_for_seconds, control.clone());
        control.set_running(false);
        if let Err(error) = result {
            error!(%error, "wallpaper playback pipeline exited");
        }
        if control.take_reload_request() {
            info!("rebuilding playback pipeline with latest config");
            continue;
        }
        // Keep the supervisor alive when playback ends naturally or the current config is empty,
        // so it can respond to later UI config updates.
        while !control.is_cancelled() {
            std::thread::sleep(Duration::from_millis(100));
        }
        if control.take_reload_request() {
            info!("idle playback supervisor received config update, rebuilding playback pipeline");
            continue;
        }
        return;
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
        let plan = prepare_scene(project_dir)?;
        info!(
            ?backend,
            draw_count = plan.quads.len(),
            skipped_nodes = plan.skipped_nodes,
            "scene wallpaper validated with shared 2D render semantics"
        );
        warn!(
            ?backend,
            "scene texture submission is not available yet; keeping the current desktop surface unchanged"
        );
        if backend == BackendKind::Kde {
            run_kde_controlled(control);
        }
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

fn prepare_scene(project_dir: &std::path::Path) -> Result<better_wallpaper_renderer::Scene2dPlan> {
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
    let graph = parse_scene_graph(&scene_json).context("failed to parse scene graph")?;
    build_scene_2d_plan(
        &graph,
        Scene2dOptions {
            viewport_width: 1920,
            viewport_height: 1080,
        },
    )
    .context("failed to build shared scene draw plan")
}

#[cfg(test)]
mod tests {
    use std::{ffi::OsString, path::PathBuf};

    use super::user_web_root;

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
