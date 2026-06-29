use std::{
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
        match ui_open_command(&self.ui_url).spawn() {
            Ok(_) => info!(
                url = self.ui_url,
                "tray requested isolated management UI launch"
            ),
            Err(error) => warn!(%error, url = self.ui_url, "failed to open management UI"),
        }
    }

    fn restart_backend(&self) {
        self.playback.request_reload();
        info!("tray requested backend restart");
    }
}

fn ui_open_command(url: &str) -> Command {
    let mut command = Command::new("xdg-open");
    command
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    for name in GRAPHICS_ENV_OVERRIDES {
        command.env_remove(name);
    }
    command
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browser_launch_does_not_inherit_graphics_overrides() {
        let command = ui_open_command("http://127.0.0.1:1234");
        assert_eq!(command.get_program(), "xdg-open");
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            vec![std::ffi::OsStr::new("http://127.0.0.1:1234")]
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
                label: "Open UI".into(),
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
