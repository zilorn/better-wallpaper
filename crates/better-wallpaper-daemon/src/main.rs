use std::{path::PathBuf, time::Duration};

use anyhow::{Context, Result};
use better_wallpaper_core::{
    BackendKind, ConfigStore,
    desktop::{ProcessEnvironment, detect_desktop, select_backend},
};
use better_wallpaper_daemon::{playback, server};
use better_wallpaper_renderer::NvidiaVulkanContext;
use clap::{Parser, ValueEnum};
use tracing::{error, info, warn};
use tracing_subscriber::EnvFilter;

#[derive(Debug, Parser)]
#[command(version, about = "Linux 视频壁纸服务")]
struct Cli {
    #[arg(long)]
    config: Option<PathBuf>,
    #[arg(long, value_enum)]
    backend: Option<CliBackend>,
    #[arg(long)]
    no_ui: bool,
    #[arg(long, default_value = "info")]
    log_level: String,
    /// niri 启动时要求 NVIDIA Vulkan/DMA-BUF 可用，否则退出
    #[arg(long)]
    require_nvidia: bool,
    /// 运行指定秒数后正常退出；用于 headless 稳定性测试
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
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_new(&cli.log_level).context("无效的 --log-level")?)
        .with_target(true)
        .init();
    std::panic::set_hook(Box::new(|panic| error!(%panic, "进程发生 panic")));

    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .context("HOME 未设置")?;
    let path = cli.config.unwrap_or(ConfigStore::default_path()?);
    let config = ConfigStore::new(path.clone(), home.clone()).load_or_create()?;
    let detection = detect_desktop(&ProcessEnvironment);
    let backend = select_backend(
        cli.backend.map(Into::into),
        config.general.backend,
        detection.kind,
    );

    info!(config = %path.display(), desktop = ?detection.kind, evidence = %detection.evidence, candidates = ?detection.candidates, backend = ?backend, ui_enabled = !cli.no_ui, "启动配置已解析");
    let _nvidia_renderer = if backend == BackendKind::Niri {
        match NvidiaVulkanContext::new() {
            Ok(context) => Some(context),
            Err(error) if cli.require_nvidia => {
                return Err(error).context("NVIDIA GPU 渲染为必需项，但初始化失败");
            }
            Err(error) => {
                warn!(%error, "NVIDIA Vulkan/DMA-BUF 不可用，将使用 CPU wl_shm 渲染路径");
                None
            }
        }
    } else {
        info!(?backend, "当前后端不初始化 NVIDIA niri 渲染器");
        None
    };
    if !cli.no_ui {
        let playback_config = config.clone();
        let playback_control = playback::PlaybackControl::default();
        let worker_control = playback_control.clone();
        std::thread::Builder::new()
            .name("wallpaper-playback".into())
            .spawn(move || {
                worker_control.set_running(true);
                let result = run_playback(
                    backend,
                    playback_config,
                    cli.run_for_seconds,
                    worker_control.clone(),
                );
                worker_control.set_running(false);
                if let Err(error) = result {
                    error!(%error, "壁纸播放线程退出");
                }
            })
            .context("创建壁纸播放线程失败")?;
        let web_root = std::env::var_os("BETTER_WALLPAPER_WEB_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("web/dist"));
        return server::serve(
            "127.0.0.1:17321",
            web_root,
            server::ApiState::new(
                config,
                ConfigStore::new(path, home),
                detection,
                backend,
                playback_control,
            ),
        );
    }
    run_playback(
        backend,
        config,
        cli.run_for_seconds,
        playback::PlaybackControl::default(),
    )
}

fn run_playback(
    backend: BackendKind,
    config: better_wallpaper_core::AppConfig,
    run_for_seconds: Option<u64>,
    control: playback::PlaybackControl,
) -> Result<()> {
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
                        Duration::from_secs(seconds),
                    )?;
                } else {
                    playback::run_headless_controlled(
                        video_path,
                        config.wallpaper.loop_playback,
                        hardware,
                        control,
                    )?;
                }
            } else {
                info!("未配置 wallpaper.path，headless 后端保持空闲");
            }
        } else {
            info!("restore_on_start=false，headless 后端不自动播放");
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
                    matches!(
                        config.decode.hardware,
                        better_wallpaper_core::config::HardwareDecode::Auto
                    ),
                    config.wallpaper.fill_mode,
                    &output_names,
                    control,
                )?;
            } else {
                info!("未配置 wallpaper.path，niri 后端保持空闲");
            }
        } else {
            info!("restore_on_start=false，niri 后端不自动播放");
        }
    } else {
        warn!(backend = ?backend, "桌面后端尚未实现；本次仅完成配置与后端选择");
    }
    Ok(())
}
