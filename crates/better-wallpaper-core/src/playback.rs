use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use tracing::info;

/// 可跨线程复制的播放控制句柄。解码和消费端都定期检查取消状态，避免队列满时无法退出。
#[derive(Clone, Debug, Default)]
pub struct PlaybackControl {
    paused: Arc<AtomicBool>,
    cancelled: Arc<AtomicBool>,
    reload_requested: Arc<AtomicBool>,
    running: Arc<AtomicBool>,
}

impl PlaybackControl {
    pub fn set_paused(&self, paused: bool) {
        self.paused.store(paused, Ordering::Release);
        info!(paused, "playback pause state updated");
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
        info!("playback cancel request received");
    }

    pub fn request_reload(&self) {
        self.reload_requested.store(true, Ordering::Release);
        info!("playback config reload request received");
    }

    pub fn take_reload_request(&self) -> bool {
        self.reload_requested.swap(false, Ordering::AcqRel)
    }

    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::Acquire)
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire) || self.reload_requested.load(Ordering::Acquire)
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::Acquire)
    }

    pub fn set_running(&self, running: bool) {
        self.running.store(running, Ordering::Release);
        info!(running, "playback running state updated");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn playback_control_tracks_pause_and_cancel() {
        let control = PlaybackControl::default();
        let worker = control.clone();

        control.set_paused(true);
        assert!(worker.is_paused());
        control.set_paused(false);
        assert!(!worker.is_paused());

        control.cancel();
        assert!(worker.is_cancelled());

        control.set_running(true);
        assert!(worker.is_running());
    }

    #[test]
    fn playback_control_consumes_reload_request_once() {
        let control = PlaybackControl::default();

        assert!(!control.take_reload_request());
        control.request_reload();
        assert!(control.is_cancelled());
        assert!(control.take_reload_request());
        assert!(!control.take_reload_request());
        assert!(!control.is_cancelled());
    }
}
