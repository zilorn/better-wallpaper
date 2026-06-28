use std::{
    fs::{self, OpenOptions},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

use anyhow::{Context, Result};
use better_wallpaper_ffmpeg::FfmpegAudioDecoder;
use better_wallpaper_core::VideoError;
use memmap2::{MmapMut, MmapOptions};
use tracing::{debug, info};

const AUDIO_MAGIC: &[u8; 4] = b"BWAD";
const AUDIO_HEADER_SIZE: usize = 64;
const AUDIO_BUFFER_SAMPLES: usize = 48_000;

pub fn audio_path(frame_path: &Path) -> PathBuf {
    PathBuf::from(format!("{}.audio", frame_path.display()))
}

pub struct PlasmaAudioPublisher {
    mapping: MmapMut,
    write_pos: u64,
    buffer_samples: usize,
    channels: usize,
    path: PathBuf,
}

impl PlasmaAudioPublisher {
    pub fn create(
        frame_path: &Path,
        sample_rate: u32,
        channels: u32,
    ) -> Result<Self> {
        let path = audio_path(frame_path);
        let buffer_samples = AUDIO_BUFFER_SAMPLES;
        let channels = channels as usize;
        let data_bytes = buffer_samples * channels * std::mem::size_of::<f32>();
        let total = AUDIO_HEADER_SIZE + data_bytes;

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!(
                    "failed to create Plasma audio directory {}",
                    parent.display()
                )
            })?;
        }

        let file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .read(true)
            .write(true)
            .open(&path)
            .with_context(|| {
                format!("failed to create Plasma audio buffer {}", path.display())
            })?;
        file.set_len(total as u64)
            .context("failed to size Plasma audio buffer")?;
        let mut mapping = unsafe { MmapOptions::new().map_mut(&file) }
            .context("failed to map Plasma audio buffer")?;

        mapping[0..4].copy_from_slice(AUDIO_MAGIC);
        mapping[4..8].copy_from_slice(&sample_rate.to_le_bytes());
        mapping[8..12].copy_from_slice(&(channels as u32).to_le_bytes());
        mapping[12..16].copy_from_slice(&(buffer_samples as u32).to_le_bytes());
        mapping[16..32].fill(0); // write_pos = 0, read_pos = 0
        mapping
            .flush()
            .context("failed to initialize Plasma audio buffer")?;

        info!(
            path = %path.display(),
            sample_rate,
            channels,
            buffer_samples,
            "Plasma audio ring buffer created"
        );

        Ok(Self {
            mapping,
            write_pos: 0,
            buffer_samples,
            channels,
            path,
        })
    }

    pub fn write_samples(&mut self, samples: &[f32]) -> usize {
        if samples.is_empty() || self.channels == 0 {
            return 0;
        }
        let to_write = samples.len() / self.channels;
        let to_write = to_write.min(self.buffer_samples);
        if to_write == 0 {
            return 0;
        }

        let offset = self.write_pos as usize % self.buffer_samples;
        let byte_offset = AUDIO_HEADER_SIZE + offset * self.channels * 4;
        let copy_bytes = to_write * self.channels * 4;
        let samples_to_copy = &samples[..to_write * self.channels];

        // SAFETY: f32 and u8 have compatible alignment; [f32] reinterpreted as [u8].
        let src_bytes: &[u8] = unsafe {
            std::slice::from_raw_parts(
                samples_to_copy.as_ptr().cast::<u8>(),
                copy_bytes,
            )
        };

        let first_chunk = copy_bytes.min(self.mapping.len() - byte_offset);
        self.mapping[byte_offset..byte_offset + first_chunk]
            .copy_from_slice(&src_bytes[..first_chunk]);

        if first_chunk < copy_bytes {
            let remaining = copy_bytes - first_chunk;
            self.mapping[AUDIO_HEADER_SIZE..AUDIO_HEADER_SIZE + remaining]
                .copy_from_slice(&src_bytes[first_chunk..]);
        }

        self.write_pos = self.write_pos.wrapping_add(to_write as u64);
        unsafe {
            (&*(self.mapping.as_ptr().add(16) as *const AtomicU64))
                .store(self.write_pos, Ordering::Release);
        }
        to_write
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for PlasmaAudioPublisher {
    fn drop(&mut self) {
        info!(
            path = %self.path.display(),
            writes = self.write_pos,
            "Plasma audio publisher stopped"
        );
        if let Err(error) = fs::remove_file(&self.path) {
            debug!(
                path = %self.path.display(),
                %error,
                "Plasma audio file cleanup deferred"
            );
        }
    }
}

/// Decode audio from a video file and publish to a PlasmaAudioPublisher.
/// Runs in its own thread.
pub fn run_audio_decode(
    video_path: PathBuf,
    frame_path: PathBuf,
    loop_playback: bool,
    control: crate::playback::PlaybackControl,
) -> Result<()> {
    let (mut decoder, info) = FfmpegAudioDecoder::open(&video_path)
        .with_context(|| format!("failed to open audio for {}", video_path.display()))?;

    let mut publisher = PlasmaAudioPublisher::create(
        &frame_path,
        info.sample_rate,
        info.channels as u32,
    )?;

    info!(
        sample_rate = info.sample_rate,
        channels = info.channels,
        "Plasma audio decode+publish started"
    );

    while !control.is_cancelled() {
        if control.is_paused() {
            std::thread::sleep(Duration::from_millis(20));
            continue;
        }
        match decoder.next_samples() {
            Ok(samples) => {
                publisher.write_samples(&samples);
            }
            Err(VideoError::EndOfStream) if loop_playback => {
                decoder
                    .seek_start()
                    .context("Plasma audio loop seek failed")?;
                info!("Plasma audio decoder looped");
                continue;
            }
            Err(VideoError::EndOfStream) => break,
            Err(error) => {
                tracing::warn!(%error, "Plasma audio decode error, skipping");
                std::thread::sleep(Duration::from_millis(5));
            }
        }
    }

    info!("Plasma audio decode+publish ended");
    Ok(())
}

