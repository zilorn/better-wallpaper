use std::{
    process::Command,
    sync::{Arc, RwLock},
};

use better_wallpaper_core::{AppConfig, ConfigStore};
use ksni::{
    Tray,
    menu::{CheckmarkItem, StandardItem},
};
use tracing::{info, warn};

use crate::playback::PlaybackControl;

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
        match Command::new("xdg-open").arg(&self.ui_url).spawn() {
            Ok(_) => info!(url = self.ui_url, "tray requested management UI"),
            Err(error) => warn!(%error, url = self.ui_url, "failed to open management UI"),
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
