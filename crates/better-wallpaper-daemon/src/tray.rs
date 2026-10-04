use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Arc, RwLock},
};

use better_wallpaper_core::{AppConfig, ConfigStore, PlaybackControl};
use ksni::{
    Tray,
    menu::{CheckmarkItem, StandardItem},
};
use tracing::{info, warn};

// These variables are commonly used to select the GPU stack for the wallpaper
// renderer. They must not leak into an unrelated desktop application: doing so
// can force a browser onto the same NVIDIA device and driver path as the
// compositor and wallpaper renderer.
const GRAPHICS_ENV_OVERRIDES: &[&str] = &[
    "__NV_PRIME_RENDER_OFFLOAD",
    "__NV_PRIME_RENDER_OFFLOAD_PROVIDER",
    "__GLX_VENDOR_LIBRARY_NAME",
    "__VK_LAYER_NV_optimus",
    "DRI_PRIME",
    "GBM_BACKEND",
    "LIBVA_DRIVER_NAME",
    "VDPAU_DRIVER",
    "VK_DRIVER_FILES",
    "VK_ICD_FILENAMES",
    "WLR_RENDERER",
];

pub struct WallpaperTray {
    config: Arc<RwLock<AppConfig>>,
    store: ConfigStore,
    playback: PlaybackControl,
    ui_url: String,
}

impl WallpaperTray {
    pub fn new(
        config: Arc<RwLock<AppConfig>>,
        store: ConfigStore,
        playback: PlaybackControl,
        ui_url: String,
    ) -> Self {
        Self {
            config,
            store,
            playback,
            ui_url,
        }
    }

    fn toggle_muted(&self) {
        let Ok(mut config) = self.config.write() else {
            warn!("failed to update mute state: config lock poisoned");
            return;
        };
        config.wallpaper.muted = !config.wallpaper.muted;
        if let Err(error) = self.store.save(&config) {
            config.wallpaper.muted = !config.wallpaper.muted;
            warn!(%error, "failed to persist tray mute state");
            return;
        }
        self.playback.request_reload();
        info!(muted = config.wallpaper.muted, "tray mute state updated");
    }

    fn open_ui(&self) {
        let url = self.ui_url.clone();
        // Wait off the tray thread so launch failures are visible without blocking
        // the menu. systemd-run exits after starting the service, not the browser.
        if let Err(error) = std::thread::Builder::new()
            .name("management-ui-launch".into())
            .spawn(move || match ui_open_command(&url).output() {
                Ok(output) if output.status.success() => {
                    info!(%url, "management UI opener started in independent user service");
                }
                Ok(output) => warn!(
                    %url,
                    status = %output.status,
                    stderr = %String::from_utf8_lossy(&output.stderr).trim(),
                    "failed to start management UI opener in user service"
                ),
                Err(error) => warn!(%error, %url, "failed to launch management UI opener"),
            })
        {
            warn!(%error, "failed to create management UI launch thread");
        }
    }

    fn restart_backend(&self) {
        self.playback.request_reload();
        info!("tray requested backend restart");
    }
}

fn desktop_client_path() -> Option<PathBuf> {
    let path = std::env::current_exe()
        .ok()?
        .parent()?
        .join("better-wallpaper-desktop");
    path.is_file().then_some(path)
}

fn ui_open_command(url: &str) -> Command {
    ui_open_command_with_client(url, desktop_client_path().as_deref())
}

fn ui_open_command_with_client(url: &str, desktop_client: Option<&Path>) -> Command {
    // A direct child inherits ProtectSystem, PrivateTmp and NoNewPrivileges
    // from the wallpaper service. In particular, browsers cannot write their
    // profiles or find existing browser sockets there. A transient *service*
    // (not --scope) is spawned by the user manager outside that sandbox, with
    // the desktop session environment rather than the daemon's environment.
    let mut command = Command::new("systemd-run");
    command
        .args([
            "--user",
            "--collect",
            "--quiet",
            "--service-type=exec",
            "--description=Better Wallpaper management UI",
        ])
        .arg(format!(
            "--property=UnsetEnvironment={}",
            GRAPHICS_ENV_OVERRIDES.join(" ")
        ))
        .arg("--");
    if let Some(path) = desktop_client {
        command.arg(path).arg("--url");
    } else {
        command.arg("xdg-open");
    }
    command
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    for name in GRAPHICS_ENV_OVERRIDES {
        command.env_remove(name);
    }
    command
}

impl Tray for WallpaperTray {
    const MENU_ON_ACTIVATE: bool = true;

    fn id(&self) -> String {
        "better-wallpaper".into()
    }

    fn title(&self) -> String {
        "Better Wallpaper".into()
    }

    fn icon_name(&self) -> String {
        "preferences-desktop-wallpaper".into()
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        self.open_ui();
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        let paused = self.playback.is_paused();
        let muted = self
            .config
            .read()
            .map(|config| config.wallpaper.muted)
            .unwrap_or(true);
        vec![
            StandardItem {
                label: if paused {
                    "Resume wallpaper".into()
                } else {
                    "Pause wallpaper".into()
                },
                icon_name: if paused {
                    "media-playback-start".into()
                } else {
                    "media-playback-pause".into()
                },
                activate: Box::new(|tray: &mut Self| {
                    let paused = !tray.playback.is_paused();
                    tray.playback.set_paused(paused);
                    info!(paused, "tray playback state updated");
                }),
                ..Default::default()
            }
            .into(),
            CheckmarkItem {
                label: "Mute".into(),
                checked: muted,
                activate: Box::new(|tray: &mut Self| tray.toggle_muted()),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Restart backend".into(),
                icon_name: "view-refresh".into(),
                activate: Box::new(|tray: &mut Self| tray.restart_backend()),
                ..Default::default()
            }
            .into(),
            ksni::MenuItem::Separator,
            StandardItem {
                label: "Open desktop client".into(),
                icon_name: "preferences-system".into(),
                activate: Box::new(|tray: &mut Self| tray.open_ui()),
                ..Default::default()
            }
            .into(),
        ]
    }

    fn watcher_online(&self) {
        info!("system tray watcher connected");
    }

    fn watcher_offline(&self, reason: ksni::OfflineReason) -> bool {
        warn!(?reason, "system tray watcher unavailable");
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_launch_uses_sibling_binary_and_explicit_endpoint() {
        let command = ui_open_command_with_client(
            "http://127.0.0.1:1234",
            Some(Path::new(
                "/opt/better-wallpaper/bin/better-wallpaper-desktop",
            )),
        );
        let args = command.get_args().collect::<Vec<_>>();
        assert_eq!(
            &args[args.len() - 4..],
            &[
                "--",
                "/opt/better-wallpaper/bin/better-wallpaper-desktop",
                "--url",
                "http://127.0.0.1:1234",
            ]
        );
        assert_eq!(command.get_program(), "systemd-run");
        for name in GRAPHICS_ENV_OVERRIDES {
            assert!(
                command
                    .get_envs()
                    .any(|(key, value)| key == *name && value.is_none())
            );
        }
    }

    #[test]
    fn browser_launch_uses_independent_service_without_graphics_overrides() {
        let command = ui_open_command_with_client("http://127.0.0.1:1234", None);
        assert_eq!(command.get_program(), "systemd-run");
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            vec![
                "--user",
                "--collect",
                "--quiet",
                "--service-type=exec",
                "--description=Better Wallpaper management UI",
                &format!(
                    "--property=UnsetEnvironment={}",
                    GRAPHICS_ENV_OVERRIDES.join(" ")
                ),
                "--",
                "xdg-open",
                "http://127.0.0.1:1234",
            ]
        );
        let removed = command
            .get_envs()
            .filter_map(|(name, value)| value.is_none().then_some(name))
            .collect::<Vec<_>>();
        for name in GRAPHICS_ENV_OVERRIDES {
            assert!(removed.contains(&std::ffi::OsStr::new(name)));
        }
    }
}
