//! KDE Plasma 后端适配。真实渲染由 Plasma 壁纸插件持有，本 crate 提供 daemon 与
//! 插件之间的控制平面 glue（配置/状态/生命周期）。

use std::{thread, time::Duration};

use better_wallpaper_core::PlaybackControl;
use tracing::info;

const CONTROL_POLL_INTERVAL: Duration = Duration::from_millis(20);

/// Keep the daemon-side Plasma control plane alive while Plasma owns media rendering.
/// The loop exits on either process cancellation or a configuration reload request.
pub fn run_kde_controlled(control: PlaybackControl) {
    info!("Plasma playback control plane started without a configured video");
    while !control.is_cancelled() {
        thread::sleep(CONTROL_POLL_INTERVAL);
    }
    info!("Plasma playback control plane stopped");
}

#[cfg(test)]
mod tests {
    use std::{thread, time::Duration};

    use super::*;

    #[test]
    fn kde_control_plane_waits_for_reload() {
        let control = PlaybackControl::default();
        let worker_control = control.clone();
        let worker = thread::spawn(move || run_kde_controlled(worker_control));

        thread::sleep(Duration::from_millis(50));
        assert!(!worker.is_finished());

        control.request_reload();
        worker.join().unwrap();
        assert!(control.take_reload_request());
    }
}
