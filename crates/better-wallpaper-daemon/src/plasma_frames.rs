use std::{
    fs::{self, OpenOptions},
    io::{ErrorKind, Write},
    os::unix::net::{UnixListener, UnixStream},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU32, AtomicU64, Ordering},
    time::{Duration, Instant},
};

use anyhow::{Context, Result, ensure};
use better_wallpaper_core::{AppConfig, DecodedFrame};
use memmap2::{MmapMut, MmapOptions};
use sha1::{Digest, Sha1};
use tracing::{debug, info};

const MAGIC: &[u8; 8] = b"BWFRAME1";
const HEADER_SIZE: usize = 64;
const SLOT_COUNT: usize = 3;
const SEQUENCE_OFFSET: usize = 32;
const ACTIVE_SLOT_OFFSET: usize = 40;
const PTS_OFFSET: usize = 44;
const PTS_DEN_OFFSET: usize = 52;

pub fn frame_path(config: &AppConfig) -> PathBuf {
    let digest = Sha1::digest(serde_json::to_vec(config).unwrap_or_default());
    let revision = digest[..8]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join(format!("better-wallpaper/plasma-frames-{revision}.bin"))
}

pub struct PlasmaFramePublisher {
    path: PathBuf,
    mapping: MmapMut,
    frame_bytes: usize,
    next_slot: usize,
    sequence: u64,
    stats_started: Instant,
    stats_frames: u64,
    stats_copy_time: Duration,
    notification_path: PathBuf,
    notification_listener: UnixListener,
    notification_clients: Vec<UnixStream>,
}

impl PlasmaFramePublisher {
    pub fn create(
        frame_path: &Path,
        video_path: &Path,
        width: u32,
        height: u32,
        stride: usize,
    ) -> Result<Self> {
        let frame_bytes = stride
            .checked_mul(height as usize)
            .context("Plasma frame size overflow")?;
        ensure!(
            stride >= width as usize * 4,
            "Plasma RGBA stride is invalid"
        );
        let total = HEADER_SIZE
            .checked_add(frame_bytes * SLOT_COUNT)
            .context("Plasma shared frame buffer size overflow")?;
        if let Some(parent) = frame_path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!(
                    "failed to create Plasma frame directory {}",
                    parent.display()
                )
            })?;
        }
        let file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .read(true)
            .write(true)
            .open(frame_path)
            .with_context(|| {
                format!(
                    "failed to create Plasma frame buffer {}",
                    frame_path.display()
                )
            })?;
        file.set_len(total as u64)
            .context("failed to size Plasma frame buffer")?;
        let mut mapping = unsafe { MmapOptions::new().map_mut(&file) }
            .context("failed to map Plasma frame buffer")?;
        mapping[..8].copy_from_slice(MAGIC);
        mapping[8..12].copy_from_slice(&width.to_le_bytes());
        mapping[12..16].copy_from_slice(&height.to_le_bytes());
        mapping[16..20].copy_from_slice(&(stride as u32).to_le_bytes());
        mapping[20..24].copy_from_slice(&(SLOT_COUNT as u32).to_le_bytes());
        mapping[24..32].copy_from_slice(&(frame_bytes as u64).to_le_bytes());
        mapping[SEQUENCE_OFFSET..SEQUENCE_OFFSET + 8].fill(0);
        mapping[ACTIVE_SLOT_OFFSET..ACTIVE_SLOT_OFFSET + 4].fill(0);
        mapping[PTS_OFFSET..PTS_DEN_OFFSET + 4].fill(0);
        mapping
            .flush()
            .context("failed to initialize Plasma frame buffer")?;

        // Write video path metadata file so the KDE plugin can find the audio source.
        let video_meta_path = PathBuf::from(format!("{}.video", frame_path.display()));
        if let Err(error) = fs::write(&video_meta_path, video_path.display().to_string()) {
            debug!(
                path = %video_meta_path.display(),
                %error,
                "failed to write Plasma video path metadata"
            );
        }

        let notification_path = notification_path(frame_path);
        if let Err(error) = fs::remove_file(&notification_path)
            && error.kind() != ErrorKind::NotFound
        {
            return Err(error).context("failed to remove stale Plasma notification socket");
        }
        let notification_listener = UnixListener::bind(&notification_path).with_context(|| {
            format!(
                "failed to create Plasma notification socket {}",
                notification_path.display()
            )
        })?;
        notification_listener
            .set_nonblocking(true)
            .context("failed to configure Plasma notification socket")?;
        info!(path = %frame_path.display(), notification_path = %notification_path.display(), width, height, stride, slots = SLOT_COUNT, "Plasma shared frame buffer created");
        Ok(Self {
            path: frame_path.to_path_buf(),
            mapping,
            frame_bytes,
            next_slot: 0,
            sequence: 0,
            stats_started: Instant::now(),
            stats_frames: 0,
            stats_copy_time: Duration::ZERO,
            notification_path,
            notification_listener,
            notification_clients: Vec::new(),
        })
    }

    pub fn publish(&mut self, frame: &DecodedFrame) -> Result<()> {
        ensure!(
            frame.pixels.len() >= self.frame_bytes,
            "decoded Plasma frame is truncated"
        );
        let start = HEADER_SIZE + self.next_slot * self.frame_bytes;
        let copy_started = Instant::now();
        self.mapping[start..start + self.frame_bytes]
            .copy_from_slice(&frame.pixels[..self.frame_bytes]);
        self.stats_copy_time += copy_started.elapsed();
        self.stats_frames += 1;
        self.sequence = self.sequence.wrapping_add(1).max(1);
        unsafe {
            (&*(self.mapping.as_ptr().add(PTS_OFFSET) as *const AtomicU64))
                .store(frame.pts as u64, Ordering::Relaxed);
            (&*(self.mapping.as_ptr().add(PTS_DEN_OFFSET) as *const AtomicU32))
                .store(frame.time_base.denominator as u32, Ordering::Relaxed);
            (&*(self.mapping.as_ptr().add(ACTIVE_SLOT_OFFSET) as *const AtomicU32))
                .store(self.next_slot as u32, Ordering::Relaxed);
            (&*(self.mapping.as_ptr().add(SEQUENCE_OFFSET) as *const AtomicU64))
                .store(self.sequence, Ordering::Release);
        }
        self.next_slot = (self.next_slot + 1) % SLOT_COUNT;
        self.notify_consumers();
        if self.stats_started.elapsed() >= Duration::from_secs(1) {
            let frames = self.stats_frames.max(1) as f64;
            info!(
                frames = self.stats_frames,
                copy_avg_ms = self.stats_copy_time.as_secs_f64() * 1000.0 / frames,
                frame_megabytes = self.frame_bytes as f64 / (1024.0 * 1024.0),
                "Plasma shared frame publishing performance"
            );
            self.stats_started = Instant::now();
            self.stats_frames = 0;
            self.stats_copy_time = Duration::ZERO;
        }
        debug!(sequence = self.sequence, "Plasma frame published");
        Ok(())
    }

    fn notify_consumers(&mut self) {
        loop {
            match self.notification_listener.accept() {
                Ok((stream, _)) => {
                    if let Err(error) = stream.set_nonblocking(true) {
                        debug!(%error, "Plasma frame notification client rejected");
                    } else {
                        self.notification_clients.push(stream);
                        debug!(
                            clients = self.notification_clients.len(),
                            "Plasma frame notification client connected"
                        );
                    }
                }
                Err(error) if error.kind() == ErrorKind::WouldBlock => break,
                Err(error) => {
                    debug!(%error, "Failed to accept Plasma frame notification client");
                    break;
                }
            }
        }
        self.notification_clients
            .retain_mut(|client| match client.write(&[1]) {
                Ok(_) => true,
                Err(error) if error.kind() == ErrorKind::WouldBlock => true,
                Err(error) => {
                    debug!(%error, "Plasma frame notification client disconnected");
                    false
                }
            });
    }
}

fn notification_path(frame_path: &Path) -> PathBuf {
    PathBuf::from(format!("{}.notify", frame_path.display()))
}

impl Drop for PlasmaFramePublisher {
    fn drop(&mut self) {
        info!(path = %self.path.display(), sequence = self.sequence, "Plasma shared frame publisher stopped");
        if let Err(error) = fs::remove_file(&self.path) {
            debug!(path = %self.path.display(), %error, "Plasma shared frame file cleanup deferred");
        }
        if let Err(error) = fs::remove_file(&self.notification_path) {
            debug!(path = %self.notification_path.display(), %error, "Plasma notification socket cleanup deferred");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use better_wallpaper_core::{ColorInfo, PixelFormat, Rational};

    #[test]
    fn publishes_complete_frames_before_advancing_sequence() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("frames.bin");
        let mut publisher =
            PlasmaFramePublisher::create(&path, Path::new("/dev/null"), 2, 1, 8).unwrap();
        let frame = DecodedFrame {
            pixels: vec![1, 2, 3, 4, 5, 6, 7, 8],
            format: PixelFormat::Rgba,
            width: 2,
            height: 1,
            stride: 8,
            pts: 42,
            time_base: Rational {
                numerator: 1,
                denominator: 1000,
            },
            color: ColorInfo::default(),
        };

        publisher.publish(&frame).unwrap();

        assert_eq!(&publisher.mapping[..8], MAGIC);
        assert_eq!(read_u64(&publisher.mapping, SEQUENCE_OFFSET), 1);
        assert_eq!(read_u32(&publisher.mapping, ACTIVE_SLOT_OFFSET), 0);
        assert_eq!(read_u64(&publisher.mapping, PTS_OFFSET), 42);
        assert_eq!(read_u32(&publisher.mapping, PTS_DEN_OFFSET), 1000);
        assert_eq!(
            &publisher.mapping[HEADER_SIZE..HEADER_SIZE + 8],
            &frame.pixels
        );
    }

    fn read_u64(data: &[u8], offset: usize) -> u64 {
        u64::from_le_bytes(data[offset..offset + 8].try_into().unwrap())
    }

    fn read_u32(data: &[u8], offset: usize) -> u32 {
        u32::from_le_bytes(data[offset..offset + 4].try_into().unwrap())
    }
}

