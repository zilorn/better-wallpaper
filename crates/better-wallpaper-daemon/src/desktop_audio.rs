use std::{
    io::Read,
    process::{Child, Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
};

use anyhow::{Context, Result};
use better_wallpaper_renderer::SceneAudioSpectrum;
use tracing::{info, warn};

const SAMPLE_RATE: f32 = 48_000.0;
const CHANNELS: usize = 2;
const WINDOW_SIZE: usize = 1024;
const SPECTRUM_BINS: usize = 64;

/// Captures the default desktop-output monitor exposed by PipeWire/PulseAudio.
/// Killing `parec` closes stdout and releases the blocking reader during drop.
pub struct DesktopAudioCapture {
    child: Child,
    worker: Option<JoinHandle<()>>,
    paused: Arc<AtomicBool>,
}

impl DesktopAudioCapture {
    pub fn start(spectrum: Arc<SceneAudioSpectrum>) -> Result<Self> {
        let mut child = Command::new("parec")
            .args([
                "--record",
                "--device=@DEFAULT_MONITOR@",
                "--client-name=better-wallpaper",
                "--stream-name=Desktop audio response",
                "--format=float32le",
                "--rate=48000",
                "--channels=2",
                "--raw",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .context("failed to start parec desktop monitor capture")?;
        let mut stdout = child
            .stdout
            .take()
            .context("parec did not expose captured audio on stdout")?;
        let paused = Arc::new(AtomicBool::new(false));
        let worker_paused = Arc::clone(&paused);
        let worker = thread::Builder::new()
            .name("desktop-audio-spectrum".into())
            .spawn(move || {
                let mut analyzer = SpectrumAnalyzer::default();
                let mut bytes = [0_u8; 8192];
                let mut partial = Vec::with_capacity(8192 + 3);
                loop {
                    match stdout.read(&mut bytes) {
                        Ok(0) => break,
                        Ok(read) => {
                            partial.extend_from_slice(&bytes[..read]);
                            let complete = partial.len() / 4 * 4;
                            if !worker_paused.load(Ordering::Acquire) {
                                for sample in partial[..complete].chunks_exact(4) {
                                    analyzer.push_sample(
                                        f32::from_le_bytes(sample.try_into().expect("four bytes")),
                                        &spectrum,
                                    );
                                }
                            }
                            partial.drain(..complete);
                        }
                        Err(error) => {
                            warn!(%error, "desktop audio monitor read failed");
                            break;
                        }
                    }
                }
                info!("desktop audio monitor reader stopped");
            })
            .context("failed to create desktop audio spectrum thread")?;
        info!("desktop audio monitor capture started");
        Ok(Self {
            child,
            worker: Some(worker),
            paused,
        })
    }

    pub fn set_paused(&self, paused: bool) {
        self.paused.store(paused, Ordering::Release);
    }
}

impl Drop for DesktopAudioCapture {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        info!("desktop audio monitor capture released");
    }
}

struct SpectrumAnalyzer {
    window: Vec<f32>,
    hann: Vec<f32>,
    channel_sum: f32,
    channel_index: usize,
    smoothed: [f32; SPECTRUM_BINS],
}

impl Default for SpectrumAnalyzer {
    fn default() -> Self {
        Self {
            window: Vec::with_capacity(WINDOW_SIZE),
            hann: (0..WINDOW_SIZE)
                .map(|index| {
                    0.5 - 0.5
                        * (std::f32::consts::TAU * index as f32 / (WINDOW_SIZE - 1) as f32).cos()
                })
                .collect(),
            channel_sum: 0.0,
            channel_index: 0,
            smoothed: [0.0; SPECTRUM_BINS],
        }
    }
}

impl SpectrumAnalyzer {
    fn push_sample(&mut self, sample: f32, spectrum: &SceneAudioSpectrum) {
        self.channel_sum += if sample.is_finite() { sample } else { 0.0 };
        self.channel_index += 1;
        if self.channel_index < CHANNELS {
            return;
        }
        self.window.push(self.channel_sum / CHANNELS as f32);
        self.channel_sum = 0.0;
        self.channel_index = 0;
        if self.window.len() == WINDOW_SIZE {
            self.analyze(spectrum);
            self.window.clear();
        }
    }

    fn analyze(&mut self, spectrum: &SceneAudioSpectrum) {
        const MIN_HZ: f32 = 50.0;
        const MAX_HZ: f32 = 16_000.0;
        let frequency_ratio = MAX_HZ / MIN_HZ;
        for bin in 0..SPECTRUM_BINS {
            let fraction = bin as f32 / (SPECTRUM_BINS - 1) as f32;
            let frequency = MIN_HZ * frequency_ratio.powf(fraction);
            let omega = std::f32::consts::TAU * frequency / SAMPLE_RATE;
            let coefficient = 2.0 * omega.cos();
            let mut previous = 0.0_f32;
            let mut before_previous = 0.0_f32;
            for (sample, hann) in self.window.iter().zip(&self.hann) {
                let current = sample * hann + coefficient * previous - before_previous;
                before_previous = previous;
                previous = current;
            }
            let power = (previous * previous + before_previous * before_previous
                - coefficient * previous * before_previous)
                .max(0.0);
            let magnitude = power.sqrt() / WINDOW_SIZE as f32;
            let normalized = (magnitude * 12.0).sqrt().clamp(0.0, 1.0);
            let attack = if normalized > self.smoothed[bin] {
                0.45
            } else {
                0.18
            };
            self.smoothed[bin] += (normalized - self.smoothed[bin]) * attack;
        }
        spectrum.set(&self.smoothed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spectrum_places_a_tone_in_the_expected_logarithmic_band() {
        let spectrum = SceneAudioSpectrum::default();
        let mut analyzer = SpectrumAnalyzer::default();
        for frame in 0..WINDOW_SIZE {
            let sample = (std::f32::consts::TAU * 1000.0 * frame as f32 / SAMPLE_RATE).sin() * 0.5;
            analyzer.push_sample(sample, &spectrum);
            analyzer.push_sample(sample, &spectrum);
        }
        let peak = (0..SPECTRUM_BINS)
            .max_by(|left, right| spectrum.get(*left).total_cmp(&spectrum.get(*right)))
            .unwrap();
        assert!(
            (29..=37).contains(&peak),
            "unexpected 1 kHz peak bin: {peak}"
        );
        assert!(spectrum.get(peak) > spectrum.get(0));
    }
}
