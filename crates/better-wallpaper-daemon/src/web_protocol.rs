//! Read-only, leased desktop data for private web wallpaper origins.
use std::{
    collections::HashMap,
    io::Read,
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use better_wallpaper_core::config::WebConfig;
use better_wallpaper_renderer::SceneAudioSpectrum;
use serde_json::{Value, json};
use tracing::{info, warn};
use zbus::{
    blocking::{Connection, Proxy, connection::Builder},
    zvariant::OwnedValue,
};

use crate::desktop_audio::SpectrumAnalyzer;

const LEASE: Duration = Duration::from_millis(500);
const PERIOD: Duration = Duration::from_millis(33);

#[derive(Clone, Copy, Default)]
pub struct WebOptions {
    pub audio: bool,
    pub media: bool,
}
impl From<&WebConfig> for WebOptions {
    fn from(config: &WebConfig) -> Self {
        Self {
            audio: config.audio_processing,
            media: config.media_integration,
        }
    }
}

struct Shared {
    stop: bool,
    last_request: Option<Instant>,
    frame: Value,
}

pub struct WebProtocol {
    shared: Arc<Mutex<Shared>>,
    worker: Option<JoinHandle<()>>,
}

fn empty_media() -> Value {
    json!({"available":false,"properties":{"title":"","artist":"","subTitle":"","albumTitle":"","albumArtist":"","genres":"","contentType":"music"},"playback":"stopped","timeline":{"position":0.0,"duration":0.0}})
}
fn empty_frame(options: WebOptions) -> Value {
    json!({"audio":null,"audio_enabled":options.audio,"media_enabled":options.media,"media":empty_media()})
}
impl WebProtocol {
    pub fn start(options: WebOptions) -> Result<Self> {
        let shared = Arc::new(Mutex::new(Shared {
            stop: false,
            last_request: None,
            frame: empty_frame(options),
        }));
        let state = Arc::clone(&shared);
        let worker = thread::Builder::new().name("web-desktop-data".into()).spawn(move || {
            let mut capture: Option<StereoCapture> = None;
            let mut bus: Option<Connection> = None;
            let mut audio_attempt = Instant::now() - Duration::from_secs(5);
            let mut media_poll = Instant::now() - Duration::from_secs(1);
            let mut media = empty_media();
            let mut bus_failed = false;
            loop {
                let active = {
                    let Ok(state) = state.lock() else { break };
                    if state.stop { break; }
                    state.last_request.is_some_and(|last| last.elapsed() < LEASE)
                };
                if !active {
                    capture.take();
                    bus.take();
                    media = empty_media();
                    audio_attempt = Instant::now() - Duration::from_secs(5);
                    media_poll = Instant::now() - Duration::from_secs(1);
                    if let Ok(mut state) = state.lock() { state.frame = empty_frame(options); }
                } else {
                    if options.audio && capture.is_none() && audio_attempt.elapsed() >= Duration::from_secs(5) {
                        audio_attempt = Instant::now();
                        match StereoCapture::start() {
                            Ok(value) => capture = Some(value),
                            Err(error) => warn!(%error, "web stereo monitor unavailable; retrying in five seconds"),
                        }
                    }
                    if capture.as_mut().is_some_and(|capture| capture.stopped()) { capture.take(); }
                    if options.media && media_poll.elapsed() >= Duration::from_millis(500) {
                        media_poll = Instant::now();
                        if bus.is_none() {
                            bus = Builder::session().and_then(|builder| builder.max_queued(8).method_timeout(Duration::from_millis(100)).build()).ok();
                            if bus.is_none() && !bus_failed { warn!("web MPRIS session bus unavailable"); }
                            bus_failed = bus.is_none();
                        }
                        media = match bus.as_ref().map(read_media) {
                            Some(Ok(value)) => value,
                            Some(Err(error)) => { warn!(%error, "web MPRIS query failed"); bus.take(); empty_media() },
                            None => empty_media(),
                        };
                    }
                    let audio = capture.as_ref().and_then(StereoCapture::levels);
                    if let Ok(mut state) = state.lock() {
                        state.frame = json!({"audio":audio,"audio_enabled":options.audio,"media_enabled":options.media,"media":media});
                    }
                }
                thread::sleep(PERIOD);
            }
            // Capture drop kills/reaps parec and joins the reader before this worker exits.
            info!("web desktop data sources released");
        })?;
        Ok(Self {
            shared,
            worker: Some(worker),
        })
    }
    pub fn frame(&mut self) -> Value {
        let mut state = self.shared.lock().expect("web protocol lock");
        state.last_request = Some(Instant::now());
        state.frame.clone()
    }
}
impl Drop for WebProtocol {
    fn drop(&mut self) {
        if let Ok(mut state) = self.shared.lock() {
            state.stop = true;
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

type LatestSpectrum = Arc<Mutex<Option<(Instant, Vec<f32>)>>>;

struct StereoCapture {
    child: Child,
    worker: Option<JoinHandle<()>>,
    latest: LatestSpectrum,
}
impl StereoCapture {
    fn start() -> Result<Self> {
        let mut child = Command::new("parec")
            .args([
                "--record",
                "--device=@DEFAULT_MONITOR@",
                "--client-name=better-wallpaper",
                "--stream-name=Web wallpaper stereo spectrum",
                "--format=float32le",
                "--rate=48000",
                "--channels=2",
                "--channel-map=front-left,front-right",
                "--latency-msec=20",
                "--raw",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .context("cannot start parec")?;
        let mut stdout = child.stdout.take().context("parec stdout unavailable")?;
        let latest = Arc::new(Mutex::new(None));
        let output = Arc::clone(&latest);
        let worker = match thread::Builder::new()
            .name("web-stereo-spectrum".into())
            .spawn(move || {
                let mut analyzers = [SpectrumAnalyzer::default(), SpectrumAnalyzer::default()];
                let spectra = [SceneAudioSpectrum::default(), SceneAudioSpectrum::default()];
                let mut bytes = [0u8; 8192];
                let mut partial = Vec::with_capacity(8199);
                while let Ok(count) = stdout.read(&mut bytes) {
                    if count == 0 {
                        break;
                    }
                    partial.extend_from_slice(&bytes[..count]);
                    let complete = partial.len() / 8 * 8;
                    for pair in partial[..complete].chunks_exact(8) {
                        let mut updated = false;
                        for channel in 0..2 {
                            let sample = f32::from_le_bytes(
                                pair[channel * 4..channel * 4 + 4].try_into().unwrap(),
                            );
                            updated = analyzers[channel].push_sample(sample, &spectra[channel]);
                        }
                        if updated {
                            let levels = (0..128).map(|i| spectra[i / 64].get(i % 64)).collect();
                            if let Ok(mut output) = output.lock() {
                                *output = Some((Instant::now(), levels));
                            }
                        }
                    }
                    partial.drain(..complete);
                }
            }) {
            Ok(worker) => worker,
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error.into());
            }
        };
        info!("web stereo desktop monitor started");
        Ok(Self {
            child,
            worker: Some(worker),
            latest,
        })
    }
    fn stopped(&mut self) -> bool {
        match self.child.try_wait() {
            Ok(None) => false,
            Ok(Some(status)) => {
                warn!(%status, "web desktop monitor exited; retrying in five seconds");
                true
            }
            Err(error) => {
                warn!(%error, "cannot inspect web desktop monitor");
                true
            }
        }
    }
    fn levels(&self) -> Option<Vec<f32>> {
        self.latest
            .lock()
            .ok()?
            .as_ref()
            .filter(|(updated, _)| updated.elapsed() < LEASE)
            .map(|(_, levels)| levels.clone())
    }
}
impl Drop for StereoCapture {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        info!("web stereo desktop monitor released");
    }
}

const PLAYER: &str = "org.mpris.MediaPlayer2.Player";
fn read_media(bus: &Connection) -> Result<Value> {
    let dbus = Proxy::new(
        bus,
        "org.freedesktop.DBus",
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
    )?;
    let names: Vec<String> = dbus.call("ListNames", &())?;
    let mut names: Vec<_> = names
        .into_iter()
        .filter(|name| name.starts_with("org.mpris.MediaPlayer2."))
        .collect();
    names.sort();
    let mut selected = None;
    let mut priority = 0;
    // Bound work even if many players advertise on the session bus. Playing > paused > stopped;
    // lexicographic name breaks ties, independently of D-Bus enumeration order.
    for name in names.into_iter().take(4) {
        let proxy = Proxy::new(
            bus,
            name.as_str(),
            "/org/mpris/MediaPlayer2",
            "org.freedesktop.DBus.Properties",
        )?;
        let Ok(mut props) = proxy.call::<_, _, HashMap<String, OwnedValue>>("GetAll", &(PLAYER,))
        else {
            continue;
        };
        let status = props
            .get("PlaybackStatus")
            .and_then(|v| <&str>::try_from(v).ok())
            .unwrap_or("Stopped");
        let rank = match status {
            "Playing" => 3,
            "Paused" => 2,
            _ => 1,
        };
        if rank <= priority {
            continue;
        }
        let status = status.to_owned();
        let position = props
            .get("Position")
            .and_then(|v| i64::try_from(v).ok())
            .unwrap_or(0);
        let metadata = props
            .remove("Metadata")
            .and_then(|v| HashMap::<String, OwnedValue>::try_from(v).ok())
            .unwrap_or_default();
        let length = metadata
            .get("mpris:length")
            .and_then(|v| i64::try_from(v).ok())
            .unwrap_or(0);
        let text = |key: &str| -> String {
            let Some(value) = metadata.get(key) else {
                return String::new();
            };
            let value = <&str>::try_from(value)
                .map(str::to_owned)
                .ok()
                .or_else(|| {
                    value
                        .try_clone()
                        .ok()
                        .and_then(|v| Vec::<String>::try_from(v).ok())
                        .map(|v| v.join(", "))
                })
                .unwrap_or_default();
            value.chars().take(4096).collect()
        };
        priority = rank;
        selected = Some(json!({"available":true,"properties":{
            "title":text("xesam:title"),"artist":text("xesam:artist"),"subTitle":"",
            "albumTitle":text("xesam:album"),"albumArtist":text("xesam:albumArtist"),"genres":text("xesam:genre"),"contentType":"music"},
            "playback":status.to_ascii_lowercase(),"timeline":{"position":position.max(0) as f64 / 1_000_000.0,"duration":length.max(0) as f64 / 1_000_000.0}}));
    }
    Ok(selected.unwrap_or_else(empty_media))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn opt_out_has_no_sources_and_drop_is_bounded() {
        let start = Instant::now();
        let mut protocol = WebProtocol::start(WebOptions::default()).unwrap();
        let frame = protocol.frame();
        assert!(frame["audio"].is_null());
        assert_eq!(frame["media"]["available"], false);
        assert_eq!(frame["media_enabled"], false);
        drop(protocol);
        assert!(start.elapsed() < Duration::from_secs(1));
    }
}
