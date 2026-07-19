use std::{
    io::Read,
    process::{Child, Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread::{self, JoinHandle},
};

use anyhow::{Context, Result};
use better_wallpaper_renderer::SceneAudioSpectrum;
use tracing::{info, warn};

const SAMPLE_RATE: f32 = 48_000.0;
const CHANNELS: usize = 1;
const WINDOW_SIZE: usize = 1024;
const HOP_SIZE: usize = WINDOW_SIZE / 2;
const SPECTRUM_BINS: usize = 64;
const PAREC_ARGS: [&str; 10] = [
    "--record",
    "--device=@DEFAULT_MONITOR@",
    "--client-name=better-wallpaper",
    "--stream-name=Desktop audio response",
    "--format=float32le",
    "--rate=48000",
    "--channels=1",
    "--channel-map=mono",
    "--latency-msec=20",
    "--raw",
];

/// Captures the default desktop-output monitor exposed by PipeWire/PulseAudio.
/// Killing `parec` closes stdout and releases the blocking reader during drop.
pub struct DesktopAudioCapture {
    child: Child,
    worker: Option<JoinHandle<()>>,
    paused: Arc<AtomicBool>,
    metrics: Arc<CaptureMetrics>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct DesktopAudioMetrics {
    pub captured_bytes: u64,
    pub spectrum_updates: u64,
}

#[derive(Debug, Default)]
struct CaptureMetrics {
    captured_bytes: AtomicU64,
    spectrum_updates: AtomicU64,
}

impl DesktopAudioCapture {
    pub fn start(spectrum: Arc<SceneAudioSpectrum>) -> Result<Self> {
        let mut child = Command::new("parec")
            .args(PAREC_ARGS)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("failed to start parec desktop monitor capture")?;
        let mut stdout = child
            .stdout
            .take()
            .context("parec did not expose captured audio on stdout")?;
        let mut stderr = child
            .stderr
            .take()
            .context("parec did not expose diagnostics on stderr")?;
        let paused = Arc::new(AtomicBool::new(false));
        let worker_paused = Arc::clone(&paused);
        let metrics = Arc::new(CaptureMetrics::default());
        let worker_metrics = Arc::clone(&metrics);
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
                            worker_metrics
                                .captured_bytes
                                .fetch_add(read as u64, Ordering::Relaxed);
                            partial.extend_from_slice(&bytes[..read]);
                            let complete = partial.len() / 4 * 4;
                            if !worker_paused.load(Ordering::Acquire) {
                                for sample in partial[..complete].chunks_exact(4) {
                                    if analyzer.push_sample(
                                        f32::from_le_bytes(sample.try_into().expect("four bytes")),
                                        &spectrum,
                                    ) {
                                        worker_metrics
                                            .spectrum_updates
                                            .fetch_add(1, Ordering::Relaxed);
                                    }
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
                let mut diagnostic = String::new();
                if stderr.read_to_string(&mut diagnostic).is_ok() && !diagnostic.trim().is_empty() {
                    warn!(
                        detail = diagnostic.trim(),
                        "desktop audio monitor process stopped"
                    );
                }
                info!("desktop audio monitor reader stopped");
            })
            .context("failed to create desktop audio spectrum thread")?;
        info!("desktop audio monitor capture started");
        Ok(Self {
            child,
            worker: Some(worker),
            paused,
            metrics,
        })
    }

    pub fn set_paused(&self, paused: bool) {
        self.paused.store(paused, Ordering::Release);
    }

    pub fn take_metrics(&self) -> DesktopAudioMetrics {
        DesktopAudioMetrics {
            captured_bytes: self.metrics.captured_bytes.swap(0, Ordering::Relaxed),
            spectrum_updates: self.metrics.spectrum_updates.swap(0, Ordering::Relaxed),
        }
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
    coefficients: [f32; SPECTRUM_BINS],
    channel_sum: f32,
    channel_index: usize,
    smoothed: [f32; SPECTRUM_BINS],
}

impl Default for SpectrumAnalyzer {
    fn default() -> Self {
        const MIN_HZ: f32 = 50.0;
        const MAX_HZ: f32 = 16_000.0;
        let frequency_ratio = MAX_HZ / MIN_HZ;
        Self {
            window: Vec::with_capacity(WINDOW_SIZE),
            hann: (0..WINDOW_SIZE)
                .map(|index| {
                    0.5 - 0.5
                        * (std::f32::consts::TAU * index as f32 / (WINDOW_SIZE - 1) as f32).cos()
                })
                .collect(),
            coefficients: std::array::from_fn(|bin| {
                let fraction = bin as f32 / (SPECTRUM_BINS - 1) as f32;
                let frequency = MIN_HZ * frequency_ratio.powf(fraction);
                2.0 * (std::f32::consts::TAU * frequency / SAMPLE_RATE).cos()
            }),
            channel_sum: 0.0,
            channel_index: 0,
            smoothed: [0.0; SPECTRUM_BINS],
        }
    }
}

impl SpectrumAnalyzer {
    fn push_sample(&mut self, sample: f32, spectrum: &SceneAudioSpectrum) -> bool {
        self.channel_sum += if sample.is_finite() { sample } else { 0.0 };
        self.channel_index += 1;
        if self.channel_index < CHANNELS {
            return false;
        }
        self.window.push(self.channel_sum / CHANNELS as f32);
        self.channel_sum = 0.0;
        self.channel_index = 0;
        if self.window.len() == WINDOW_SIZE {
            self.analyze(spectrum);
            self.window.copy_within(HOP_SIZE.., 0);
            self.window.truncate(WINDOW_SIZE - HOP_SIZE);
            return true;
        }
        false
    }

    fn analyze(&mut self, spectrum: &SceneAudioSpectrum) {
        for bin in 0..SPECTRUM_BINS {
            let coefficient = self.coefficients[bin];
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
                0.65
            } else {
                0.22
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
        }
        assert_eq!(analyzer.window.len(), HOP_SIZE);
        let peak = (0..SPECTRUM_BINS)
            .max_by(|left, right| spectrum.get(*left).total_cmp(&spectrum.get(*right)))
            .unwrap();
        assert!(
            (29..=37).contains(&peak),
            "unexpected 1 kHz peak bin: {peak}"
        );
        assert!(spectrum.get(peak) > spectrum.get(0));
    }

    #[test]
    fn spectrum_publishes_after_one_window_then_each_half_window() {
        let spectrum = SceneAudioSpectrum::default();
        let mut analyzer = SpectrumAnalyzer::default();
        for _ in 0..WINDOW_SIZE - 1 {
            assert!(!analyzer.push_sample(0.1, &spectrum));
        }
        assert!(analyzer.push_sample(0.1, &spectrum));
        for _ in 0..HOP_SIZE - 1 {
            assert!(!analyzer.push_sample(0.1, &spectrum));
        }
        assert!(analyzer.push_sample(0.1, &spectrum));
    }

    #[test]
    fn capture_requests_the_default_monitor_with_low_latency() {
        assert!(PAREC_ARGS.contains(&"--device=@DEFAULT_MONITOR@"));
        assert!(PAREC_ARGS.contains(&"--channels=1"));
        assert!(PAREC_ARGS.contains(&"--latency-msec=20"));
    }
}
