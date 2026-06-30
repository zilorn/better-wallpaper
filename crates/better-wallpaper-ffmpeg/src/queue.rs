use std::sync::mpsc::{Receiver, SyncSender, sync_channel};

use better_wallpaper_core::DecodedFrame;

pub type FrameQueueSender = SyncSender<DecodedFrame>;
pub type FrameQueueReceiver = Receiver<DecodedFrame>;

/// 创建有界帧队列。容量至少为 1，解码速度超过呈现速度时发送端会受到背压。
pub fn frame_queue(capacity: usize) -> (FrameQueueSender, FrameQueueReceiver) {
    assert!(capacity > 0, "帧队列容量必须大于 0");
    sync_channel(capacity)
}

#[cfg(test)]
mod tests {
    use super::*;
    use better_wallpaper_core::{ColorInfo, PixelFormat, Rational};
    use std::sync::mpsc::TrySendError;

    fn frame(pts: i64) -> DecodedFrame {
        DecodedFrame {
            pixels: vec![0; 4],
            cuda: None,
            format: PixelFormat::Rgba,
            width: 1,
            height: 1,
            stride: 4,
            pts,
            time_base: Rational {
                numerator: 1,
                denominator: 60,
            },
            color: ColorInfo::default(),
        }
    }

    #[test]
    fn bounded_queue_applies_backpressure() {
        let (sender, receiver) = frame_queue(1);
        sender.try_send(frame(0)).unwrap();
        assert!(matches!(
            sender.try_send(frame(1)),
            Err(TrySendError::Full(_))
        ));
        assert_eq!(receiver.recv().unwrap().pts, 0);
        sender.try_send(frame(1)).unwrap();
    }
}
