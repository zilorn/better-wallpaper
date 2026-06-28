use std::{
    collections::{HashMap, HashSet},
    fs::{self, File},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, RwLock},
    thread,
    time::{Duration, Instant},
};

use crate::{playback::PlaybackControl, tray::WallpaperTray};
use anyhow::Result;
use base64::{Engine, engine::general_purpose::STANDARD};
use better_wallpaper_core::{
    AppConfig, BackendKind, ConfigStore, DesktopDetection, config::FillMode,
};
use ksni::blocking::TrayMethods;
use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};
use tiny_http::{Header, Method, Request, Response, ResponseBox, Server, StatusCode};
use tracing::{info, warn};

const MAX_REQUEST_BYTES: usize = 1024 * 1024;
const STATUS_POLL_INTERVAL: Duration = Duration::from_millis(250);
const STATUS_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(15);
const MAX_LIBRARY_ENTRIES: usize = 1_000;
const MAX_LIBRARY_DEPTH: usize = 4;
const PLASMA_INSTANCE_TTL: Duration = Duration::from_secs(10);

#[derive(Clone)]
pub struct ApiState {
    config: Arc<RwLock<AppConfig>>,
    store: ConfigStore,
    detection: DesktopDetection,
    backend: BackendKind,
    playback: PlaybackControl,
    home: PathBuf,
    plasma_instances: Arc<Mutex<HashMap<String, Instant>>>,
}

#[derive(Serialize)]
struct StatusPayload<'a> {
    api_version: u8,
    desktop: &'a str,
    evidence: &'a str,
    candidates: &'a [String],
    backend: BackendKind,
    playback: PlaybackPayload,
    plasma_instances: Vec<PlasmaInstancePayload>,
}

#[derive(Serialize)]
struct PlasmaInstancePayload {
    output: String,
    last_seen_ms: u128,
}

#[derive(Serialize)]
struct PlaybackPayload {
    running: bool,
    paused: bool,
    cancelled: bool,
}

impl ApiState {
    pub fn new(
        config: Arc<RwLock<AppConfig>>,
        store: ConfigStore,
        detection: DesktopDetection,
        backend: BackendKind,
        playback: PlaybackControl,
        home: PathBuf,
    ) -> Self {
        Self {
            config,
            store,
            detection,
            backend,
            playback,
            home,
            plasma_instances: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

pub fn serve(bind: &str, web_root: PathBuf, state: ApiState) -> Result<()> {
    let server = Server::http(bind).map_err(|error| anyhow::anyhow!(error.to_string()))?;
    let address = server
        .server_addr()
        .to_ip()
        .ok_or_else(|| anyhow::anyhow!("management service did not bind an IP address"))?;
    let ui_url = format!("http://127.0.0.1:{}", address.port());
    write_endpoint_file(&state.home, &ui_url)?;
    let _tray = match WallpaperTray::new(
        Arc::clone(&state.config),
        state.store.clone(),
        state.playback.clone(),
        ui_url.clone(),
    )
    .assume_sni_available(true)
    .spawn()
    {
        Ok(handle) => {
            info!("system tray service started");
            Some(handle)
        }
        Err(error) => {
            warn!(%error, "system tray service could not start");
            None
        }
    };
    info!(bind = %address, %ui_url, web_root = %web_root.display(), "local management service started");
    loop {
        match server.recv_timeout(Duration::from_secs(1)) {
            Ok(Some(request)) => {
                let request_state = state.clone();
                let request_web_root = web_root.clone();
                if let Err(error) = thread::Builder::new()
                    .name("management-request".into())
                    .spawn(move || handle_request(request, &request_web_root, &request_state))
                {
                    warn!(%error, "failed to create management request thread");
                }
            }
            Ok(None) => {}
            Err(error) => warn!(%error, "failed to receive HTTP request"),
        }
    }
}

fn write_endpoint_file(home: &Path, ui_url: &str) -> Result<()> {
    let directory = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".better-wallpaper"))
        .join("better-wallpaper");
    fs::create_dir_all(&directory)?;
    let path = directory.join("endpoint");
    fs::write(&path, format!("{ui_url}\n"))?;
    info!(path = %path.display(), %ui_url, "management endpoint discovery file updated");
    Ok(())
}

fn handle_request(mut request: Request, web_root: &Path, state: &ApiState) {
    let method = request.method().clone();
    let request_url = request.url().to_owned();
    let url = request_url.split('?').next().unwrap_or("/").to_owned();
    info!(method = %method, %url, "handling management API request");
    if method == Method::Get && url == "/api/v1/ws" {
        upgrade_websocket(request, state.clone());
        return;
    }
    if method == Method::Get && url == "/api/v1/library/media" {
        let response = library_media_response(&request, &request_url, state);
        if let Err(error) = request.respond(response) {
            warn!(%error, %url, "failed to send library media response");
        }
        return;
    }
    if method == Method::Get && url == "/api/v1/wallpaper/media" {
        let response = wallpaper_media_response(&request, state);
        if let Err(error) = request.respond(response) {
            warn!(%error, %url, "failed to send current wallpaper media response");
        }
        return;
    }
    if method == Method::Get && url == "/api/v1/plasma/config" {
        let response = plasma_config_response(&request_url, state);
        if let Err(error) = request.respond(response) {
            warn!(%error, %url, "failed to send Plasma config response");
        }
        return;
    }
    if method == Method::Post && url == "/api/v1/plasma/heartbeat" {
        let response = plasma_heartbeat_response(&mut request, state);
        if let Err(error) = request.respond(response) {
            warn!(%error, %url, "failed to send Plasma heartbeat response");
        }
        return;
    }
    let response = match (method, url.as_str()) {
        (Method::Get, "/api/v1/status") => json_response(
            StatusCode(200),
            &StatusPayload {
                api_version: 1,
                desktop: desktop_name(state.detection.kind),
                evidence: &state.detection.evidence,
                candidates: &state.detection.candidates,
                backend: state.backend,
                playback: playback_payload(&state.playback),
                plasma_instances: active_plasma_instances(state),
            },
        ),
        (Method::Get, "/api/v1/config") => match state.config.read() {
            Ok(config) => json_response(StatusCode(200), &*config),
            Err(_) => error_response(StatusCode(500), "config lock poisoned"),
        },
        (Method::Get, "/api/v1/library") => library_response(state),
        (Method::Put, "/api/v1/config") => update_config(&mut request, state),
        (Method::Post, "/api/v1/playback/pause") => set_paused(state, true),
        (Method::Post, "/api/v1/playback/resume") => set_paused(state, false),
        (Method::Get, _) => static_response(web_root, &url),
        _ => error_response(StatusCode(404), "endpoint not found"),
    };
    if let Err(error) = request.respond(response) {
        warn!(%error, %url, "failed to send HTTP response");
    }
}

#[derive(Deserialize)]
struct PlasmaHeartbeatRequest {
    output: String,
}

fn plasma_heartbeat_response(
    request: &mut Request,
    state: &ApiState,
) -> Response<std::io::Cursor<Vec<u8>>> {
    let mut body = Vec::new();
    if request
        .as_reader()
        .take((MAX_REQUEST_BYTES + 1) as u64)
        .read_to_end(&mut body)
        .is_err()
        || body.len() > MAX_REQUEST_BYTES
    {
        return error_response(StatusCode(413), "request body too large");
    }
    let heartbeat: PlasmaHeartbeatRequest =
        match serde_json::from_slice::<PlasmaHeartbeatRequest>(&body) {
            Ok(heartbeat) if !heartbeat.output.trim().is_empty() => heartbeat,
            _ => return error_response(StatusCode(400), "invalid Plasma heartbeat"),
        };
    let Ok(mut instances) = state.plasma_instances.lock() else {
        return error_response(StatusCode(500), "Plasma instance lock poisoned");
    };
    let is_new = instances
        .insert(heartbeat.output.clone(), Instant::now())
        .is_none();
    if is_new {
        info!(
            output = heartbeat.output,
            "Plasma wallpaper instance connected"
        );
    }
    json_response(StatusCode(200), &serde_json::json!({ "accepted": true }))
}

fn active_plasma_instances(state: &ApiState) -> Vec<PlasmaInstancePayload> {
    let now = Instant::now();
    let Ok(mut instances) = state.plasma_instances.lock() else {
        warn!("Plasma instance lock poisoned");
        return Vec::new();
    };
    instances.retain(|output, last_seen| {
        let active = now.duration_since(*last_seen) <= PLASMA_INSTANCE_TTL;
        if !active {
            info!(output, "Plasma wallpaper instance disconnected");
        }
        active
    });
    let mut payload: Vec<_> = instances
        .iter()
        .map(|(output, last_seen)| PlasmaInstancePayload {
            output: output.clone(),
            last_seen_ms: now.duration_since(*last_seen).as_millis(),
        })
        .collect();
    payload.sort_by(|left, right| left.output.cmp(&right.output));
    payload
}

#[derive(Serialize)]
struct PlasmaConfigPayload {
    api_version: u8,
    output: String,
    enabled: bool,
    media_url: &'static str,
    fill_mode: FillMode,
    muted: bool,
    paused: bool,
    loop_playback: bool,
    revision: u64,
}

fn plasma_config_response(
    request_url: &str,
    state: &ApiState,
) -> Response<std::io::Cursor<Vec<u8>>> {
    let output = query_parameter(request_url, "output")
        .and_then(percent_decode)
        .unwrap_or_default();
    let config = match state.config.read() {
        Ok(config) => config,
        Err(_) => return error_response(StatusCode(500), "config lock poisoned"),
    };
    let enabled = output_enabled(&config, &output);
    info!(output, enabled, "serving Plasma wallpaper configuration");
    json_response(
        StatusCode(200),
        &PlasmaConfigPayload {
            api_version: 1,
            output,
            enabled,
            media_url: "/api/v1/wallpaper/media",
            fill_mode: config.wallpaper.fill_mode,
            muted: config.wallpaper.muted,
            paused: state.playback.is_paused(),
            loop_playback: config.wallpaper.loop_playback,
            revision: config_revision(&config),
        },
    )
}

fn output_enabled(config: &AppConfig, output: &str) -> bool {
    if config.outputs.is_empty() {
        return true;
    }
    config
        .outputs
        .iter()
        .find(|candidate| candidate.name == output)
        .is_some_and(|candidate| candidate.enabled)
}

fn config_revision(config: &AppConfig) -> u64 {
    // Stable FNV-1a is sufficient as a cache-busting configuration revision.
    serde_json::to_vec(config)
        .unwrap_or_default()
        .into_iter()
        .fold(0xcbf29ce484222325, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
        })
}

#[derive(Debug, Serialize, PartialEq, Eq)]
struct LibraryEntry {
    name: String,
    path: PathBuf,
    size_bytes: u64,
    modified_unix_seconds: Option<u64>,
}

#[derive(Serialize)]
struct LibraryPayload {
    entries: Vec<LibraryEntry>,
    roots: Vec<PathBuf>,
    truncated: bool,
}

fn library_response(state: &ApiState) -> Response<std::io::Cursor<Vec<u8>>> {
    let roots = library_roots(state);
    let (entries, truncated) = scan_library(&roots);
    info!(
        roots = roots.len(),
        entries = entries.len(),
        truncated,
        "library scan complete"
    );
    json_response(
        StatusCode(200),
        &LibraryPayload {
            entries,
            roots,
            truncated,
        },
    )
}

fn library_roots(state: &ApiState) -> Vec<PathBuf> {
    let mut roots = vec![state.home.join("Videos")];
    if let Ok(config) = state.config.read()
        && let Some(parent) = config.wallpaper.path.as_deref().and_then(Path::parent)
    {
        roots.push(parent.to_path_buf());
    }
    roots.sort();
    roots.dedup();
    roots
}

fn library_media_response(request: &Request, request_url: &str, state: &ApiState) -> ResponseBox {
    let Some(encoded_path) = query_parameter(request_url, "path") else {
        return error_response(StatusCode(400), "missing media path parameter").boxed();
    };
    let Some(path) = percent_decode(encoded_path).map(PathBuf::from) else {
        return error_response(StatusCode(400), "invalid media path parameter encoding").boxed();
    };
    media_response(request, path, state, true)
}

fn wallpaper_media_response(request: &Request, state: &ApiState) -> ResponseBox {
    let path = match state.config.read() {
        Ok(config) => config.wallpaper.path.clone(),
        Err(_) => return error_response(StatusCode(500), "config lock poisoned").boxed(),
    };
    let Some(path) = path else {
        return error_response(StatusCode(404), "no current wallpaper configured").boxed();
    };
    media_response(request, path, state, false)
}

fn media_response(
    request: &Request,
    path: PathBuf,
    state: &ApiState,
    require_library_entry: bool,
) -> ResponseBox {
    let canonical = match path.canonicalize() {
        Ok(path) => path,
        Err(error) => {
            warn!(path = %path.display(), %error, "library media file does not exist");
            return error_response(StatusCode(404), "media file does not exist").boxed();
        }
    };
    let allowed = !require_library_entry
        || scan_library(&library_roots(state)).0.iter().any(|entry| {
            entry
                .path
                .canonicalize()
                .is_ok_and(|candidate| candidate == canonical)
        });
    if !allowed {
        warn!(path = %canonical.display(), "refusing to read media file outside library");
        return error_response(StatusCode(403), "media file is not in the library").boxed();
    }
    let mut file = match File::open(&canonical) {
        Ok(file) => file,
        Err(error) => {
            warn!(path = %canonical.display(), %error, "failed to open library media");
            return error_response(StatusCode(404), "cannot open media file").boxed();
        }
    };
    let length = match file.metadata() {
        Ok(metadata) => metadata.len(),
        Err(error) => {
            warn!(path = %canonical.display(), %error, "failed to read library media size");
            return error_response(StatusCode(500), "cannot read media file").boxed();
        }
    };
    let content_type = video_content_type(&canonical);
    let range = request
        .headers()
        .iter()
        .find(|header| header.field.equiv("Range"))
        .and_then(|header| parse_byte_range(header.value.as_str(), length));
    if let Some((start, end)) = range {
        if file.seek(SeekFrom::Start(start)).is_err() {
            return error_response(StatusCode(500), "cannot seek media file").boxed();
        }
        let response_length = end - start + 1;
        info!(path = %canonical.display(), start, end, "serving library media range");
        return Response::new(
            StatusCode(206),
            vec![
                header("Content-Type", content_type),
                header("Accept-Ranges", "bytes"),
                header("Content-Range", &format!("bytes {start}-{end}/{length}")),
            ],
            file.take(response_length),
            usize::try_from(response_length).ok(),
            None,
        )
        .boxed();
    }
    info!(path = %canonical.display(), length, "serving full library media");
    Response::from_file(file)
        .with_header(header("Content-Type", content_type))
        .with_header(header("Accept-Ranges", "bytes"))
        .boxed()
}

fn query_parameter<'a>(url: &'a str, name: &str) -> Option<&'a str> {
    url.split_once('?')?.1.split('&').find_map(|part| {
        let (key, value) = part.split_once('=').unwrap_or((part, ""));
        (key == name).then_some(value)
    })
}

fn percent_decode(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'%' if index + 2 < bytes.len() => {
                decoded.push(hex_value(bytes[index + 1])? << 4 | hex_value(bytes[index + 2])?);
                index += 3;
            }
            b'%' => return None,
            b'+' => {
                decoded.push(b' ');
                index += 1;
            }
            byte => {
                decoded.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8(decoded).ok()
}

const fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn parse_byte_range(value: &str, length: u64) -> Option<(u64, u64)> {
    let range = value.strip_prefix("bytes=")?;
    if range.contains(',') || length == 0 {
        return None;
    }
    let (start, end) = range.split_once('-')?;
    let start = start.parse::<u64>().ok()?;
    let end = if end.is_empty() {
        length - 1
    } else {
        end.parse::<u64>().ok()?.min(length - 1)
    };
    (start <= end && start < length).then_some((start, end))
}

fn header(name: &str, value: &str) -> Header {
    Header::from_bytes(name, value).expect("valid HTTP response header")
}

fn scan_library(roots: &[PathBuf]) -> (Vec<LibraryEntry>, bool) {
    let mut entries = Vec::new();
    let mut visited = HashSet::new();
    let mut pending: Vec<_> = roots.iter().map(|path| (path.clone(), 0)).collect();
    let mut truncated = false;
    while let Some((directory, depth)) = pending.pop() {
        let canonical = match directory.canonicalize() {
            Ok(path) if visited.insert(path.clone()) => path,
            _ => continue,
        };
        let children = match fs::read_dir(&canonical) {
            Ok(children) => children,
            Err(error) => {
                warn!(path = %canonical.display(), %error, "skipping unreadable library directory");
                continue;
            }
        };
        for child in children.flatten() {
            let path = child.path();
            if child.file_type().is_ok_and(|kind| kind.is_symlink()) {
                warn!(path = %path.display(), "skipping symlink in library");
                continue;
            }
            let metadata = match child.metadata() {
                Ok(metadata) => metadata,
                Err(error) => {
                    warn!(path = %path.display(), %error, "skipping library entry with unreadable metadata");
                    continue;
                }
            };
            if metadata.is_dir() && depth < MAX_LIBRARY_DEPTH {
                pending.push((path, depth + 1));
            } else if metadata.is_file() && is_video_path(&path) && path.to_str().is_some() {
                let modified_unix_seconds = metadata
                    .modified()
                    .ok()
                    .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|duration| duration.as_secs());
                entries.push(LibraryEntry {
                    name: child.file_name().to_string_lossy().into_owned(),
                    path,
                    size_bytes: metadata.len(),
                    modified_unix_seconds,
                });
                if entries.len() >= MAX_LIBRARY_ENTRIES {
                    truncated = true;
                    break;
                }
            }
        }
        if truncated {
            break;
        }
    }
    entries.sort_by_key(|entry| entry.name.to_lowercase());
    (entries, truncated)
}

fn is_video_path(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|extension| extension.to_str())
            .map(|extension| extension.to_ascii_lowercase())
            .as_deref(),
        Some("mp4" | "mkv" | "webm" | "mov" | "avi" | "m4v")
    )
}

fn video_content_type(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("webm") => "video/webm",
        Some("mov") => "video/quicktime",
        Some("avi") => "video/x-msvideo",
        Some("mkv") => "video/x-matroska",
        _ => "video/mp4",
    }
}

fn status_payload(state: &ApiState) -> StatusPayload<'_> {
    StatusPayload {
        api_version: 1,
        desktop: desktop_name(state.detection.kind),
        evidence: &state.detection.evidence,
        candidates: &state.detection.candidates,
        backend: state.backend,
        playback: playback_payload(&state.playback),
        plasma_instances: active_plasma_instances(state),
    }
}

fn upgrade_websocket(request: Request, state: ApiState) {
    let key = request
        .headers()
        .iter()
        .find(|header| header.field.equiv("Sec-WebSocket-Key"))
        .map(|header| header.value.as_str().to_owned());
    let is_upgrade = request.headers().iter().any(|header| {
        header.field.equiv("Upgrade") && header.value.as_str().eq_ignore_ascii_case("websocket")
    });
    let Some(key) = key.filter(|_| is_upgrade) else {
        warn!("rejecting invalid WebSocket upgrade request");
        let _ = request.respond(error_response(
            StatusCode(400),
            "invalid WebSocket upgrade request",
        ));
        return;
    };
    let response = Response::empty(StatusCode(101))
        .with_header(
            Header::from_bytes("Upgrade", "websocket").expect("valid WebSocket response header"),
        )
        .with_header(
            Header::from_bytes("Connection", "Upgrade").expect("valid WebSocket response header"),
        )
        .with_header(
            Header::from_bytes("Sec-WebSocket-Accept", websocket_accept(&key))
                .expect("valid WebSocket accept response header"),
        );
    let mut stream = request.upgrade("websocket", response);
    if let Err(error) = thread::Builder::new()
        .name("status-websocket".into())
        .spawn(move || {
            info!("WebSocket status subscription connected");
            let mut previous = Vec::new();
            let mut last_sent = Instant::now() - STATUS_HEARTBEAT_INTERVAL;
            loop {
                let payload = match serde_json::to_vec(&serde_json::json!({
                    "type": "status",
                    "data": status_payload(&state),
                })) {
                    Ok(payload) => payload,
                    Err(error) => {
                        warn!(%error, "failed to serialize WebSocket status");
                        break;
                    }
                };
                if (payload != previous || last_sent.elapsed() >= STATUS_HEARTBEAT_INTERVAL)
                    && write_websocket_text(&mut stream, &payload).is_err()
                {
                    info!("WebSocket status subscription disconnected");
                    break;
                }
                if payload != previous || last_sent.elapsed() >= STATUS_HEARTBEAT_INTERVAL {
                    previous = payload;
                    last_sent = Instant::now();
                }
                thread::sleep(STATUS_POLL_INTERVAL);
            }
        })
    {
        warn!(%error, "failed to create WebSocket status push thread");
    }
}

fn websocket_accept(key: &str) -> String {
    let digest = Sha1::digest(format!("{key}258EAFA5-E914-47DA-95CA-C5AB0DC85B11"));
    STANDARD.encode(digest)
}

fn write_websocket_text(stream: &mut impl Write, payload: &[u8]) -> std::io::Result<()> {
    let mut header = vec![0x81];
    match payload.len() {
        length @ 0..=125 => header.push(length as u8),
        length @ 126..=65535 => {
            header.push(126);
            header.extend_from_slice(&(length as u16).to_be_bytes());
        }
        length => {
            header.push(127);
            header.extend_from_slice(&(length as u64).to_be_bytes());
        }
    }
    stream.write_all(&header)?;
    stream.write_all(payload)?;
    stream.flush()
}

fn playback_payload(control: &PlaybackControl) -> PlaybackPayload {
    PlaybackPayload {
        running: control.is_running(),
        paused: control.is_paused(),
        cancelled: control.is_cancelled(),
    }
}

fn set_paused(state: &ApiState, paused: bool) -> Response<std::io::Cursor<Vec<u8>>> {
    if state.playback.is_cancelled() {
        return error_response(
            StatusCode(409),
            "playback already stopped, cannot change pause state",
        );
    }
    if !state.playback.is_running() {
        return error_response(StatusCode(409), "no playback task currently running");
    }
    state.playback.set_paused(paused);
    info!(paused, "management API applied playback control command");
    json_response(StatusCode(200), &playback_payload(&state.playback))
}

fn update_config(request: &mut Request, state: &ApiState) -> Response<std::io::Cursor<Vec<u8>>> {
    let mut body = Vec::new();
    if request
        .as_reader()
        .take((MAX_REQUEST_BYTES + 1) as u64)
        .read_to_end(&mut body)
        .is_err()
    {
        return error_response(StatusCode(400), "failed to read request body");
    }
    if body.len() > MAX_REQUEST_BYTES {
        return error_response(StatusCode(413), "request body too large");
    }
    let config: AppConfig = match serde_json::from_slice(&body) {
        Ok(config) => config,
        Err(error) => return error_response(StatusCode(400), &format!("invalid JSON: {error}")),
    };
    if let Err(error) = state.store.save(&config) {
        warn!(%error, "management API config update failed");
        return error_response(StatusCode(400), &error.to_string());
    }
    let config = match state.store.load_or_create() {
        Ok(config) => config,
        Err(error) => {
            warn!(%error, "failed to reload saved config");
            return error_response(StatusCode(500), &error.to_string());
        }
    };
    match state.config.write() {
        Ok(mut current) => *current = config,
        Err(_) => return error_response(StatusCode(500), "config lock poisoned"),
    }
    state.playback.request_reload();
    info!("management API config update complete, playback pipeline rebuild requested");
    json_response(
        StatusCode(200),
        &serde_json::json!({ "saved": true, "restart_required": false, "reload_requested": true }),
    )
}

fn static_response(web_root: &Path, url: &str) -> Response<std::io::Cursor<Vec<u8>>> {
    let relative = if url == "/" {
        "index.html"
    } else {
        url.trim_start_matches('/')
    };
    if relative.contains("..") {
        return error_response(StatusCode(400), "invalid path");
    }
    let path = web_root.join(relative);
    let path = if path.is_file() {
        path
    } else {
        web_root.join("index.html")
    };
    match fs::read(&path) {
        Ok(body) => response(StatusCode(200), content_type(&path), body),
        Err(error) => {
            warn!(path = %path.display(), %error, "failed to read Web static asset");
            error_response(StatusCode(404), "Web UI not built, run bun run build")
        }
    }
}

fn json_response<T: Serialize>(
    status: StatusCode,
    value: &T,
) -> Response<std::io::Cursor<Vec<u8>>> {
    match serde_json::to_vec(value) {
        Ok(body) => response(status, "application/json; charset=utf-8", body),
        Err(error) => error_response(
            StatusCode(500),
            &format!("failed to serialize response: {error}"),
        ),
    }
}

fn error_response(status: StatusCode, message: &str) -> Response<std::io::Cursor<Vec<u8>>> {
    json_response(status, &serde_json::json!({ "error": message }))
}

fn response(
    status: StatusCode,
    content_type: &str,
    body: Vec<u8>,
) -> Response<std::io::Cursor<Vec<u8>>> {
    Response::from_data(body)
        .with_status_code(status)
        .with_header(
            Header::from_bytes("Content-Type", content_type).expect("valid static Content-Type"),
        )
}

fn content_type(path: &Path) -> &'static str {
    match path.extension().and_then(|value| value.to_str()) {
        Some("html") => "text/html; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("js") => "text/javascript; charset=utf-8",
        Some("svg") => "image/svg+xml",
        _ => "application/octet-stream",
    }
}

fn desktop_name(kind: better_wallpaper_core::DesktopKind) -> &'static str {
    match kind {
        better_wallpaper_core::DesktopKind::Niri => "niri",
        better_wallpaper_core::DesktopKind::Kde => "kde",
        better_wallpaper_core::DesktopKind::Unknown => "unknown",
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{
        config_revision, output_enabled, parse_byte_range, percent_decode, scan_library,
        websocket_accept, write_websocket_text,
    };
    use better_wallpaper_core::{AppConfig, config::OutputConfig};

    #[test]
    fn computes_websocket_accept_from_rfc_example() {
        assert_eq!(
            websocket_accept("dGhlIHNhbXBsZSBub25jZQ=="),
            "s3pPLMBiTxaQ9kYGzzhZRbK+xOo="
        );
    }

    #[test]
    fn encodes_short_and_extended_text_frames() {
        let mut short = Vec::new();
        write_websocket_text(&mut short, b"status").unwrap();
        assert_eq!(&short[..2], &[0x81, 6]);
        assert_eq!(&short[2..], b"status");

        let mut extended = Vec::new();
        write_websocket_text(&mut extended, &[b'x'; 126]).unwrap();
        assert_eq!(&extended[..4], &[0x81, 126, 0, 126]);
        assert_eq!(extended.len(), 130);
    }

    #[test]
    fn scans_supported_videos_recursively() {
        let directory = tempfile::tempdir().unwrap();
        fs::create_dir(directory.path().join("nested")).unwrap();
        fs::write(directory.path().join("wallpaper.MP4"), b"video").unwrap();
        fs::write(directory.path().join("nested/clip.webm"), b"video2").unwrap();
        fs::write(directory.path().join("ignored.txt"), b"text").unwrap();

        let (entries, truncated) = scan_library(&[directory.path().to_path_buf()]);

        assert!(!truncated);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].name, "clip.webm");
        assert_eq!(entries[1].name, "wallpaper.MP4");
    }

    #[test]
    fn decodes_media_paths_and_parses_ranges() {
        assert_eq!(
            percent_decode("%2Fhome%2Fme%2Fa%20b.mp4").as_deref(),
            Some("/home/me/a b.mp4")
        );
        assert_eq!(percent_decode("%ZZ"), None);
        assert_eq!(parse_byte_range("bytes=10-19", 100), Some((10, 19)));
        assert_eq!(parse_byte_range("bytes=90-", 100), Some((90, 99)));
        assert_eq!(parse_byte_range("bytes=100-", 100), None);
    }

    #[test]
    fn resolves_plasma_output_enablement_and_revision() {
        let mut config = AppConfig::default();
        assert!(output_enabled(&config, "DP-1"));
        config.outputs = vec![OutputConfig {
            name: "DP-1".into(),
            enabled: true,
        }];
        assert!(output_enabled(&config, "DP-1"));
        assert!(!output_enabled(&config, "HDMI-A-1"));

        let revision = config_revision(&config);
        config.wallpaper.muted = false;
        assert_ne!(revision, config_revision(&config));
    }
}
