use std::{
    path::{Path, PathBuf},
    sync::mpsc,
    thread,
    time::Duration,
};

use better_wallpaper_core::PlaybackControl;
use better_wallpaper_daemon::playback::{run_headless_controlled, run_headless_for};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name)
}

#[test]
fn paused_playback_can_be_cancelled_without_deadlock() {
    let control = PlaybackControl::default();
    control.set_paused(true);
    let worker_control = control.clone();
    let (result_tx, result_rx) = mpsc::channel();

    thread::spawn(move || {
        let result = run_headless_controlled(fixture("h264.mp4"), true, false, 0, worker_control);
        let _ = result_tx.send(result);
    });

    thread::sleep(Duration::from_millis(50));
    control.cancel();
    result_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("cancel should cause timely exit")
        .expect("cancel should end playback normally");
}

#[test]
fn deadline_stops_looping_playback_and_reports_stats() {
    let stats = run_headless_for(
        fixture("h264.mp4"),
        true,
        false,
        0,
        Duration::from_millis(650),
    )
    .expect("deadline should end looped playback normally");

    assert!(stats.presented >= 5);
    assert!(stats.loops >= 1);
}
