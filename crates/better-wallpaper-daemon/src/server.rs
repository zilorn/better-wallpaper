use std::{
    collections::HashMap,
    fs::{self, File},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    process::Command,
    sync::{Arc, Mutex, RwLock},
    thread,
    time::{Duration, Instant},
};

use crate::{LogLevelController, LogStore, tray::WallpaperTray};
use anyhow::Result;
use base64::{Engine, engine::general_purpose::STANDARD};
use better_wallpaper_core::{
    AppConfig, BackendKind, ConfigStore, DesktopDetection, PlaybackControl,
    config::{FillMode, WallpaperType},
    desktop::select_backend,
};
use better_wallpaper_scene_format::{
    CompatibilityLevel, PkgReader, UserProperty, analyse_scene_with_package, compute_compatibility,
    parse_project_properties,
};
use ksni::blocking::TrayMethods;
use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};
use tiny_http::{Header, Method, Request, Response, ResponseBox, Server, StatusCode};
use tracing::{info, warn};

mod library_index;
use library_index::LibraryIndex;

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
    thumbnail_generation: Arc<Mutex<()>>,
    library_index: Arc<Mutex<LibraryIndex>>,
    pub log_store: LogStore,
    log_level: LogLevelController,
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
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        config: Arc<RwLock<AppConfig>>,
        store: ConfigStore,
        detection: DesktopDetection,
        backend: BackendKind,
        playback: PlaybackControl,
        home: PathBuf,
        log_store: LogStore,
        log_level: LogLevelController,
    ) -> Self {
        Self {
            config,
            store,
            detection,
            backend,
            playback,
            home,
            plasma_instances: Arc::new(Mutex::new(HashMap::new())),
            thumbnail_generation: Arc::new(Mutex::new(())),
            library_index: Arc::new(Mutex::new(LibraryIndex::new())),
            log_store,
            log_level,
        }
    }
}

pub fn serve(bind: &str, state: ApiState) -> Result<()> {
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
    info!(bind = %address, %ui_url, "local management service started");
    loop {
        match server.recv_timeout(Duration::from_secs(1)) {
            Ok(Some(request)) => {
                let request_state = state.clone();
                if let Err(error) = thread::Builder::new()
                    .name("management-request".into())
                    .spawn(move || handle_request(request, &request_state))
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

fn handle_request(mut request: Request, state: &ApiState) {
    let method = request.method().clone();
    let request_url = request.url().to_owned();
    let url = request_url.split('?').next().unwrap_or("/").to_owned();
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
    if method == Method::Get && url == "/api/v1/library/thumbnail" {
        let response = library_thumbnail_response(&request_url, state);
        if let Err(error) = request.respond(response) {
            warn!(%error, %url, "failed to send library thumbnail response");
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
    if method == Method::Get && url.starts_with("/api/v1/wallpaper/web/") {
        let response = wallpaper_web_response(&request, &url, state);
        if let Err(error) = request.respond(response) {
            warn!(%error, %url, "failed to send web wallpaper asset response");
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
    if method == Method::Get && url == "/api/v1/logs" {
        let response = json_response(
            StatusCode(200),
            &serde_json::json!({ "lines": state.log_store.lines() }),
        );
        if let Err(error) = request.respond(response) {
            warn!(%error, %url, "failed to send logs response");
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
    media_path: Option<PathBuf>,
    wallpaper_type: WallpaperType,
    web_url: Option<String>,
    web_properties: serde_json::Value,
    fill_mode: FillMode,
    muted: bool,
    paused: bool,
    loop_playback: bool,
    revision: u64,
    scene_rendering_available: bool,
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
    let web_project = (config.wallpaper.wallpaper_type == WallpaperType::Web)
        .then(|| {
            config
                .wallpaper
                .path
                .as_deref()
                .and_then(|path| crate::web_wallpaper::WebProject::load(path).ok())
        })
        .flatten();
    json_response(
        StatusCode(200),
        &PlasmaConfigPayload {
            api_version: 1,
            output,
            enabled,
            media_url: "/api/v1/wallpaper/media",
            media_path: config.wallpaper.path.clone(),
            wallpaper_type: config.wallpaper.wallpaper_type,
            web_url: web_project
                .as_ref()
                .map(|project| project.entry_url("/api/v1/wallpaper/web/")),
            web_properties: web_project
                .map(|project| project.properties)
                .unwrap_or_else(|| serde_json::json!({})),
            fill_mode: config.wallpaper.fill_mode,
            muted: config.wallpaper.muted,
            paused: state.playback.is_paused(),
            loop_playback: config.wallpaper.loop_playback,
            revision: config_revision(&config),
            scene_rendering_available: false,
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

#[derive(Debug, Clone, Serialize)]
struct LibraryEntry {
    name: String,
    path: PathBuf,
    engine_mode: bool,
    wallpaper_type: WallpaperType,
    preview_path: Option<PathBuf>,
    size_bytes: u64,
    modified_unix_seconds: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    scene_compatibility: Option<CompatibilityPayload>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    scene_properties: Vec<UserProperty>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct CompatibilityPayload {
    level: u32,
    level_name: String,
    supported_features: Vec<String>,
    unsupported_features: Vec<String>,
    warnings: Vec<String>,
}

#[derive(Serialize)]
struct LibraryPayload {
    entries: Vec<LibraryEntry>,
    roots: Vec<PathBuf>,
    engine_roots: Vec<PathBuf>,
    truncated: bool,
    #[serde(skip)]
    by_path: HashMap<PathBuf, usize>,
}

impl LibraryPayload {
    fn entry(&self, path: &Path) -> Option<&LibraryEntry> {
        self.by_path.get(path).map(|index| &self.entries[*index])
    }
}

fn library_response(state: &ApiState) -> Response<std::io::Cursor<Vec<u8>>> {
    let snapshot = library_snapshot(state);
    json_response(StatusCode(200), &*snapshot)
}

fn library_roots(state: &ApiState) -> Vec<PathBuf> {
    match state.config.read() {
        Ok(config) => config.library.paths.clone(),
        Err(_) => {
            warn!("Config lock poisoned while reading library paths");
            Vec::new()
        }
    }
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

fn library_thumbnail_response(request_url: &str, state: &ApiState) -> ResponseBox {
    let Some(encoded_path) = query_parameter(request_url, "path") else {
        return error_response(StatusCode(400), "missing media path parameter").boxed();
    };
    let Some(path) = percent_decode(encoded_path).map(PathBuf::from) else {
        return error_response(StatusCode(400), "invalid media path parameter encoding").boxed();
    };
    let snapshot = library_snapshot(state);
    let requested = snapshot.entry(&path);
    if requested.as_ref().is_some_and(|entry| {
        entry.wallpaper_type == WallpaperType::Web && entry.preview_path.is_none()
    }) {
        return error_response(StatusCode(404), "web wallpaper has no preview image").boxed();
    }
    let thumbnail_path = requested
        .as_ref()
        .and_then(|entry| entry.preview_path.as_ref())
        .unwrap_or(&path);
    let canonical = match thumbnail_path.canonicalize() {
        Ok(path) => path,
        Err(error) => {
            warn!(path = %path.display(), %error, "thumbnail source does not exist");
            return error_response(StatusCode(404), "media file does not exist").boxed();
        }
    };
    let allowed = requested.is_some() && canonical == *thumbnail_path;
    if !allowed {
        warn!(path = %canonical.display(), "refusing to create thumbnail outside library");
        return error_response(StatusCode(403), "media file is not in the library").boxed();
    }
    let metadata = match canonical.metadata() {
        Ok(metadata) => metadata,
        Err(error) => {
            warn!(path = %canonical.display(), %error, "failed to read thumbnail source metadata");
            return error_response(StatusCode(500), "cannot read media metadata").boxed();
        }
    };
    if requested
        .as_ref()
        .is_some_and(|entry| entry.wallpaper_type == WallpaperType::Web)
    {
        return match File::open(&canonical) {
            Ok(file) => Response::from_file(file)
                .with_header(header("Content-Type", image_content_type(&canonical)))
                .with_header(header("Cache-Control", "public, max-age=3600"))
                .boxed(),
            Err(error) => {
                warn!(path = %canonical.display(), %error, "failed to open web wallpaper preview");
                error_response(StatusCode(404), "cannot open preview image").boxed()
            }
        };
    }
    let mut hasher = Sha1::new();
    hasher.update(canonical.to_string_lossy().as_bytes());
    hasher.update(metadata.len().to_le_bytes());
    if let Ok(modified) = metadata.modified().and_then(|time| {
        time.duration_since(std::time::UNIX_EPOCH)
            .map_err(std::io::Error::other)
    }) {
        hasher.update(modified.as_nanos().to_le_bytes());
    }
    let cache_dir = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| state.home.join(".cache"))
        .join("better-wallpaper/thumbnails");
    let cache_path = cache_dir.join(format!("{:x}.jpg", hasher.finalize()));

    let generation_guard = match state.thumbnail_generation.lock() {
        Ok(guard) => guard,
        Err(_) => return error_response(StatusCode(500), "thumbnail lock poisoned").boxed(),
    };
    if !cache_path.is_file() {
        if let Err(error) = fs::create_dir_all(&cache_dir) {
            warn!(path = %cache_dir.display(), %error, "failed to create thumbnail cache directory");
            return error_response(StatusCode(500), "cannot create thumbnail cache").boxed();
        }
        let temporary = cache_path.with_extension("tmp.jpg");
        info!(source = %canonical.display(), target = %cache_path.display(), "generating library thumbnail");
        let result = Command::new("ffmpeg")
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-nostdin",
                "-y",
                "-ss",
                "1",
                "-i",
            ])
            .arg(&canonical)
            .args([
                "-frames:v",
                "1",
                "-vf",
                "scale=480:270:force_original_aspect_ratio=increase,crop=480:270",
                "-q:v",
                "6",
            ])
            .arg(&temporary)
            .status();
        match result {
            Ok(status) if status.success() && temporary.is_file() => {
                if let Err(error) = fs::rename(&temporary, &cache_path) {
                    let _ = fs::remove_file(&temporary);
                    warn!(%error, "failed to commit generated thumbnail");
                    return error_response(StatusCode(500), "cannot store thumbnail").boxed();
                }
            }
            Ok(status) => {
                let _ = fs::remove_file(&temporary);
                warn!(source = %canonical.display(), ?status, "FFmpeg failed to generate thumbnail");
                return error_response(StatusCode(422), "cannot generate video thumbnail").boxed();
            }
            Err(error) => {
                warn!(%error, "failed to start FFmpeg thumbnail generator");
                return error_response(StatusCode(500), "FFmpeg is unavailable").boxed();
            }
        }
    }
    drop(generation_guard);
    match File::open(&cache_path) {
        Ok(file) => Response::from_file(file)
            .with_header(header("Content-Type", "image/jpeg"))
            .with_header(header(
                "Cache-Control",
                "public, max-age=31536000, immutable",
            ))
            .boxed(),
        Err(error) => {
            warn!(path = %cache_path.display(), %error, "failed to open cached thumbnail");
            error_response(StatusCode(500), "cannot open thumbnail").boxed()
        }
    }
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

fn wallpaper_web_response(request: &Request, url: &str, state: &ApiState) -> ResponseBox {
    let entry = match state.config.read() {
        Ok(config) if config.wallpaper.wallpaper_type == WallpaperType::Web => {
            config.wallpaper.path.clone()
        }
        Ok(_) => {
            return error_response(StatusCode(404), "current wallpaper is not web content").boxed();
        }
        Err(_) => return error_response(StatusCode(500), "config lock poisoned").boxed(),
    };
    let Some(entry) = entry else {
        return error_response(StatusCode(404), "no current wallpaper configured").boxed();
    };
    let project = match crate::web_wallpaper::WebProject::load(&entry) {
        Ok(project) => project,
        Err(error) => {
            warn!(%error, "cannot resolve web wallpaper project");
            return error_response(StatusCode(404), "web wallpaper project is unavailable").boxed();
        }
    };
    project.asset_response(
        request,
        url.strip_prefix("/api/v1/wallpaper/web/")
            .unwrap_or_default(),
    )
}

#[cfg(test)]
fn wallpaper_engine_web_root(entry: &Path) -> Option<PathBuf> {
    crate::web_wallpaper::WebProject::load(entry)
        .ok()
        .map(|project| project.root)
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
    let allowed = !require_library_entry || library_snapshot(state).entry(&canonical).is_some();
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
    if let Some(range) = request
        .headers()
        .iter()
        .find(|header| header.field.equiv("Range"))
    {
        let Some((start, end)) = parse_byte_range(range.value.as_str(), length) else {
            return error_response(StatusCode(416), "invalid media byte range")
                .with_header(header("Content-Range", &format!("bytes */{length}")))
                .boxed();
        };
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

pub(crate) fn percent_decode(value: &str) -> Option<String> {
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

pub(crate) fn parse_byte_range(value: &str, length: u64) -> Option<(u64, u64)> {
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

pub(crate) fn header(name: &str, value: &str) -> Header {
    Header::from_bytes(name, value).expect("valid HTTP response header")
}

fn library_snapshot(state: &ApiState) -> Arc<LibraryPayload> {
    let roots = library_roots(state);
    let mut index = state.library_index.lock().unwrap_or_else(|error| {
        warn!("Library index lock poisoned; rebuilding index");
        let mut index = error.into_inner();
        *index = LibraryIndex::new();
        state.library_index.clear_poison();
        index
    });
    index.refresh(roots, &state.home)
}

#[cfg(test)]
fn scan_library(roots: &[PathBuf], engine_roots: &[PathBuf]) -> (Vec<LibraryEntry>, bool) {
    let snapshot = LibraryIndex::new().scan(roots.to_vec(), engine_roots.to_vec());
    // Existing scanner contract tests inspect the serialized public entry fields.
    (snapshot.entries.clone(), snapshot.truncated)
}

fn wallpaper_engine_candidates(home: &Path) -> Vec<PathBuf> {
    let steam_roots = [
        home.join(".local/share/Steam"),
        home.join(".steam/steam"),
        home.join(".var/app/com.valvesoftware.Steam/.local/share/Steam"),
    ];
    let mut libraries = steam_roots.to_vec();
    for steam_root in steam_roots {
        let library_folders = steam_root.join("steamapps/libraryfolders.vdf");
        let Ok(contents) = fs::read_to_string(&library_folders) else {
            continue;
        };
        for line in contents.lines() {
            let fields = line.split('"').collect::<Vec<_>>();
            if fields.get(1).is_some_and(|field| field.trim() == "path")
                && let Some(path) = fields.get(3).filter(|path| !path.is_empty())
            {
                libraries.push(PathBuf::from(path.replace("\\\\", "\\")));
            }
        }
    }
    let mut roots = libraries
        .into_iter()
        .map(|library| library.join("steamapps/workshop/content/431960"))
        .collect::<Vec<_>>();
    roots.sort();
    roots.dedup();
    roots
}

#[derive(Deserialize)]
struct WallpaperEngineProject {
    #[serde(rename = "type")]
    kind: String,
    file: Option<PathBuf>,
    title: Option<String>,
    preview: Option<PathBuf>,
}

/// Analyse a scene.pkg for compatibility information.
/// Returns None if the pkg is missing or unreadable.
fn analyse_scene_pkg(project_dir: &Path) -> Option<CompatibilityPayload> {
    let pkg_path = project_dir.join("scene.pkg");
    let data = fs::read(&pkg_path).ok()?;
    let pkg = PkgReader::parse(data).ok()?;

    // Try to find and parse scene.json from the package
    let scene_entry = pkg.find("scene.json")?;
    let scene_json = pkg.read_entry_string(scene_entry).ok()?;
    let meta = analyse_scene_with_package(&scene_json, &pkg).ok()?;
    let compat = compute_compatibility(&meta);

    Some(CompatibilityPayload {
        level: compat.level as u32,
        level_name: match compat.level {
            CompatibilityLevel::L0 => "L0".into(),
            CompatibilityLevel::L1 => "L1".into(),
            CompatibilityLevel::L2 => "L2".into(),
            CompatibilityLevel::L3 => "L3".into(),
            CompatibilityLevel::L4 => "L4".into(),
        },
        supported_features: compat.supported_features,
        unsupported_features: compat.unsupported_features,
        warnings: compat.warnings,
    })
}

fn scan_wallpaper_engine_project(project_directory: &Path) -> Option<LibraryEntry> {
    let descriptor = project_directory.join("project.json");
    let descriptor_json = match fs::read_to_string(&descriptor) {
        Ok(json) => json,
        Err(error) => {
            warn!(path = %descriptor.display(), %error, "skipping unreadable Wallpaper Engine project");
            return None;
        }
    };
    let project: WallpaperEngineProject = match serde_json::from_str(&descriptor_json) {
        Ok(project) => project,
        Err(error) => {
            warn!(path = %descriptor.display(), %error, "skipping invalid Wallpaper Engine project");
            return None;
        }
    };
    let wallpaper_type = match project.kind.to_ascii_lowercase().as_str() {
        "video" => WallpaperType::Video,
        "web" => WallpaperType::Web,
        "scene" => WallpaperType::Scene,
        _ => {
            info!(path = %descriptor.display(), kind = %project.kind, "skipping unsupported Wallpaper Engine project type");
            return None;
        }
    };
    let mut scene_properties = if wallpaper_type == WallpaperType::Scene {
        match parse_project_properties(&descriptor_json) {
            Ok(properties) => properties
                .into_iter()
                .map(|(_, property)| property)
                .collect(),
            Err(error) => {
                warn!(path = %descriptor.display(), %error, "failed to parse scene user properties");
                Vec::new()
            }
        }
    } else {
        Vec::new()
    };
    scene_properties.sort_by(|left, right| {
        left.order
            .unwrap_or(u32::MAX)
            .cmp(&right.order.unwrap_or(u32::MAX))
            .then_with(|| left.key.cmp(&right.key))
    });

    let (media_path, preview_path, scene_compatibility) = match wallpaper_type {
        WallpaperType::Scene => {
            // Scene wallpapers: the entry path is the project directory itself.
            let canonical_project = match project_directory.canonicalize() {
                Ok(path) => path,
                Err(_) => return None,
            };
            let preview = project.preview.and_then(|relative| {
                if relative.is_absolute() {
                    return None;
                }
                project_directory
                    .join(relative)
                    .canonicalize()
                    .ok()
                    .filter(|path| path.starts_with(&canonical_project) && path.is_file())
            });
            // Analyse scene.pkg for compatibility
            let compatibility = analyse_scene_pkg(&canonical_project);
            (canonical_project, preview, compatibility)
        }
        _ => {
            let Some(relative_file) = project.file.filter(|path| !path.is_absolute()) else {
                warn!(path = %descriptor.display(), "skipping Wallpaper Engine project without a safe entry path");
                return None;
            };
            let media_path = project_directory.join(relative_file);
            let canonical_project = match project_directory.canonicalize() {
                Ok(path) => path,
                Err(_) => return None,
            };
            let media_path = match media_path.canonicalize() {
                Ok(path) if path.starts_with(&canonical_project) => path,
                _ => {
                    warn!(path = %media_path.display(), "skipping unsafe Wallpaper Engine entry path");
                    return None;
                }
            };
            let _ = match media_path.metadata() {
                Ok(md) if md.is_file() => md,
                Ok(_) => return None,
                Err(error) => {
                    warn!(path = %media_path.display(), %error, "skipping missing Wallpaper Engine entry file");
                    return None;
                }
            };
            let preview = project.preview.and_then(|relative| {
                if relative.is_absolute() {
                    return None;
                }
                project_directory
                    .join(relative)
                    .canonicalize()
                    .ok()
                    .filter(|path| path.starts_with(&canonical_project) && path.is_file())
            });
            (media_path, preview, None)
        }
    };

    let metadata = match media_path.metadata() {
        Ok(metadata) => metadata,
        Err(error) => {
            warn!(path = %media_path.display(), %error, "skipping unreadable project metadata");
            return None;
        }
    };
    Some(LibraryEntry {
        name: project
            .title
            .filter(|title| !title.trim().is_empty())
            .unwrap_or_else(|| {
                project_directory
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned()
            }),
        path: media_path,
        engine_mode: true,
        wallpaper_type,
        preview_path,
        size_bytes: metadata.len(),
        modified_unix_seconds: metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|duration| duration.as_secs()),
        scene_compatibility,
        scene_properties,
    })
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

fn image_content_type(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("png") => "image/png",
        Some("webp") => "image/webp",
        Some("gif") => "image/gif",
        _ => "image/jpeg",
    }
}

pub(crate) fn asset_content_type(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("html" | "htm") => "text/html; charset=utf-8",
        Some("js" | "mjs") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("json") => "application/json",
        Some("png" | "jpg" | "jpeg" | "webp" | "gif") => image_content_type(path),
        Some("svg") => "image/svg+xml",
        Some("mp3") => "audio/mpeg",
        Some("ogg") => "audio/ogg",
        Some("wav") => "audio/wav",
        Some("mp4" | "webm") => video_content_type(path),
        Some("woff") => "font/woff",
        Some("woff2") => "font/woff2",
        Some("ttf") => "font/ttf",
        Some("wasm") => "application/wasm",
        Some("ico") => "image/x-icon",
        _ => "application/octet-stream",
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
    // Serialize persistence and runtime updates with other PUTs and tray writes.
    // Saving before taking this lock can leave disk and shared state out of sync.
    let mut current = match state.config.write() {
        Ok(current) => current,
        Err(_) => return error_response(StatusCode(500), "config lock poisoned"),
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
    *current = config.clone();
    state.log_level.set(&config.general.log_level);
    // Compare against the running backend, not the previous saved config: a
    // second save must keep reporting a pending switch until the daemon restarts.
    // Resolve auto using startup detection. A CLI override must be removed or
    // adjusted on restart for the saved preference to take effect.
    let configured_backend = select_backend(None, config.general.backend, state.detection.kind);
    let restart_required = configured_backend != state.backend;
    state.playback.request_reload();
    drop(current);
    info!(
        running_backend = ?state.backend,
        ?configured_backend,
        restart_required,
        "management API config update complete, playback pipeline rebuild requested"
    );
    json_response(
        StatusCode(200),
        &serde_json::json!({
            "saved": true,
            "restart_required": restart_required,
            "reload_requested": true,
            "config": config,
        }),
    )
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

pub(crate) fn error_response(
    status: StatusCode,
    message: &str,
) -> Response<std::io::Cursor<Vec<u8>>> {
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
        wallpaper_engine_web_root, websocket_accept, write_websocket_text,
    };
    use better_wallpaper_core::{AppConfig, WallpaperType, config::OutputConfig};

    fn config_test_state(directory: &std::path::Path) -> super::ApiState {
        use super::*;
        use better_wallpaper_core::desktop::DesktopKind;

        ApiState::new(
            Arc::new(RwLock::new(AppConfig::default())),
            ConfigStore::new(directory.join("config.toml"), directory.into()),
            DesktopDetection {
                kind: DesktopKind::Niri,
                evidence: "test desktop".into(),
                candidates: vec!["niri".into()],
            },
            BackendKind::Niri,
            PlaybackControl::default(),
            directory.into(),
            LogStore::new(10),
            LogLevelController::new("info"),
        )
    }

    #[test]
    fn api_and_wallpaper_assets_work_without_management_html() {
        use super::*;
        use better_wallpaper_core::desktop::DesktopKind;
        use std::net::TcpStream;

        let directory = tempfile::tempdir().unwrap();
        let project = directory.path().join("wallpaper");
        fs::create_dir(&project).unwrap();
        fs::write(
            project.join("project.json"),
            r#"{"type":"web","file":"main.html"}"#,
        )
        .unwrap();
        fs::write(project.join("main.html"), "<html>wallpaper-content</html>").unwrap();
        let mut config = AppConfig::default();
        config.wallpaper.wallpaper_type = WallpaperType::Web;
        config.wallpaper.path = Some(project);
        let state = ApiState::new(
            Arc::new(RwLock::new(config)),
            ConfigStore::new(
                directory.path().join("config.toml"),
                directory.path().into(),
            ),
            DesktopDetection {
                kind: DesktopKind::Niri,
                evidence: "test".into(),
                candidates: vec![],
            },
            BackendKind::Niri,
            PlaybackControl::default(),
            directory.path().into(),
            LogStore::new(10),
            LogLevelController::new("info"),
        );
        let cases = [
            ("/", "404", "endpoint not found"),
            ("/index.html", "404", "endpoint not found"),
            ("/assets/old.js", "404", "endpoint not found"),
            ("/api/v1/unknown", "404", "endpoint not found"),
            ("/api/v1/status", "200", "api_version"),
            ("/api/v1/config", "200", "wallpaper_type"),
            ("/api/v1/plasma/config?output=DP-1", "200", "web_url"),
            ("/api/v1/wallpaper/web/", "200", "wallpaper-content"),
        ];
        let server = Server::http("127.0.0.1:0").unwrap();
        let address = server.server_addr().to_ip().unwrap();
        thread::scope(|scope| {
            scope.spawn(|| {
                for _ in &cases {
                    let request = server
                        .recv_timeout(Duration::from_secs(5))
                        .unwrap()
                        .unwrap();
                    handle_request(request, &state);
                }
            });
            for (path, status, body) in cases {
                let mut stream = TcpStream::connect(address).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                write!(
                    stream,
                    "GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
                )
                .unwrap();
                let mut response = String::new();
                stream.read_to_string(&mut response).unwrap();
                assert!(
                    response.starts_with(&format!("HTTP/1.1 {status}")),
                    "{path}: {response}"
                );
                assert!(response.contains(body), "{path}: {response}");
            }
        });
    }

    #[test]
    fn config_put_reports_pending_backend_switch_until_reverted() {
        use super::*;

        let directory = tempfile::tempdir().unwrap();
        let state = config_test_state(directory.path());

        let put = |body: &'static str, expected_backend, expected_restart| {
            let mut request = tiny_http::TestRequest::new()
                .with_method(Method::Put)
                .with_path("/api/v1/config")
                .with_body(body)
                .into();
            let response = update_config(&mut request, &state);
            assert_eq!(response.status_code(), StatusCode(200));
            let result: serde_json::Value =
                serde_json::from_reader(response.into_reader()).unwrap();
            assert_eq!(result["saved"], true);
            assert_eq!(result["restart_required"], expected_restart);
            assert_eq!(result["reload_requested"], true);
            assert_eq!(
                result["config"]["general"]["backend"],
                serde_json::to_value(expected_backend).unwrap()
            );
            assert_eq!(
                state.store.load_or_create().unwrap().general.backend,
                expected_backend
            );
            assert_eq!(
                state.config.read().unwrap().general.backend,
                expected_backend
            );
            assert!(state.playback.take_reload_request());
            assert_eq!(status_payload(&state).backend, BackendKind::Niri);
        };
        put(
            r#"{"general":{"backend":"auto"}}"#,
            BackendKind::Auto,
            false,
        );
        put(
            r#"{"general":{"backend":"niri"}}"#,
            BackendKind::Niri,
            false,
        );
        put(r#"{"general":{"backend":"kde"}}"#, BackendKind::Kde, true);
        put(
            r#"{"general":{"backend":"kde","log_level":"debug"}}"#,
            BackendKind::Kde,
            true,
        );
        put(
            r#"{"general":{"backend":"headless"}}"#,
            BackendKind::Headless,
            true,
        );
        put(
            r#"{"general":{"backend":"auto"}}"#,
            BackendKind::Auto,
            false,
        );

        // Simulate a CLI override selecting a backend different from detection.
        let state = ApiState {
            backend: BackendKind::Headless,
            ..state
        };
        let mut request = tiny_http::TestRequest::new()
            .with_method(Method::Put)
            .with_body(r#"{"general":{"backend":"auto"}}"#)
            .into();
        let response = update_config(&mut request, &state);
        assert_eq!(response.status_code(), StatusCode(200));
        let result: serde_json::Value = serde_json::from_reader(response.into_reader()).unwrap();
        assert_eq!(result["restart_required"], true);
        assert_eq!(status_payload(&state).backend, BackendKind::Headless);
    }

    #[test]
    fn config_put_does_not_save_when_shared_state_is_unavailable() {
        use super::*;

        let directory = tempfile::tempdir().unwrap();
        let state = config_test_state(directory.path());
        state.store.save(&AppConfig::default()).unwrap();
        let original = fs::read(directory.path().join("config.toml")).unwrap();
        let config = Arc::clone(&state.config);
        assert!(
            thread::spawn(move || {
                let _guard = config.write().unwrap();
                panic!("poison config lock to simulate an unavailable shared state");
            })
            .join()
            .is_err()
        );

        let mut request = tiny_http::TestRequest::new()
            .with_method(Method::Put)
            .with_body(r#"{"wallpaper":{"fps_limit":30}}"#)
            .into();
        assert_eq!(
            update_config(&mut request, &state).status_code(),
            StatusCode(500)
        );
        assert_eq!(
            fs::read(directory.path().join("config.toml")).unwrap(),
            original
        );
        assert!(!state.playback.take_reload_request());
    }

    #[test]
    fn concurrent_config_puts_and_tray_writes_keep_disk_and_runtime_in_sync() {
        use super::*;
        use std::sync::Barrier;

        let directory = tempfile::tempdir().unwrap();
        let state = config_test_state(directory.path());
        state.store.save(&AppConfig::default()).unwrap();
        *state.config.write().unwrap() = state.store.load_or_create().unwrap();
        let barrier = Barrier::new(8);
        thread::scope(|scope| {
            for index in 0..8 {
                let state = &state;
                let barrier = &barrier;
                scope.spawn(move || {
                    barrier.wait();
                    for _ in 0..10 {
                        if index % 2 == 0 {
                            let mut request = tiny_http::TestRequest::new()
                                .with_method(Method::Put)
                                .with_body(r#"{"wallpaper":{"fps_limit":30},"general":{"log_level":"debug"}}"#)
                                .into();
                            assert_eq!(update_config(&mut request, state).status_code(), StatusCode(200));
                        } else {
                            // The tray holds this same write lock while persisting mute.
                            let mut current = state.config.write().unwrap();
                            current.wallpaper.muted = !current.wallpaper.muted;
                            state.store.save(&current).unwrap();
                            state.playback.request_reload();
                        }
                        let current = state.config.read().unwrap();
                        assert_eq!(state.store.load_or_create().unwrap(), *current);
                    }
                });
            }
        });
        assert_eq!(
            state.store.load_or_create().unwrap(),
            *state.config.read().unwrap()
        );
        assert!(state.playback.take_reload_request());
    }

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

        let (entries, truncated) = scan_library(&[directory.path().to_path_buf()], &[]);

        assert!(!truncated);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].name, "clip.webm");
        assert_eq!(entries[1].name, "wallpaper.MP4");
        assert!(entries.iter().all(|entry| !entry.engine_mode));
    }

    #[test]
    fn library_serializes_supported_scene_tiers_and_remaining_limits() {
        let directory = tempfile::tempdir().unwrap();
        for (name, scene, expected_level) in [
            ("audio", r#"{"objects":[{"sound":"sounds/bg.ogg"}]}"#, 2),
            (
                "effects",
                r#"{"general":{"bloom":true},"objects":[{"image":"models/bg.json","effects":[{"file":"effects/shake/effect.json","passes":[{"constantshadervalues":{}}]}]},{"particle":"particles/snow.json"}]}"#,
                3,
            ),
        ] {
            let project = directory.path().join(name);
            fs::create_dir(&project).unwrap();
            fs::write(
                project.join("project.json"),
                format!(r#"{{"type":"scene","file":"scene.pkg","title":"{name}"}}"#),
            )
            .unwrap();
            let mut package = Vec::new();
            package.extend(8u32.to_le_bytes());
            package.extend(b"PKGV0018");
            package.extend(1u32.to_le_bytes());
            package.extend(10u32.to_le_bytes());
            package.extend(b"scene.json");
            package.extend(0u32.to_le_bytes());
            package.extend((scene.len() as u32).to_le_bytes());
            package.extend(scene.as_bytes());
            fs::write(project.join("scene.pkg"), package).unwrap();
            let report = super::analyse_scene_pkg(&project).unwrap();
            assert_eq!(report.level, expected_level);
            assert_eq!(report.level_name, format!("L{expected_level}"));
        }
        let (entries, truncated) = scan_library(&[], &[directory.path().to_path_buf()]);
        assert!(!truncated);
        assert_eq!(entries.len(), 2);
        let json = serde_json::to_value(&entries).unwrap();
        assert_eq!(json[0]["scene_compatibility"]["level"], 2);
        assert_eq!(json[1]["scene_compatibility"]["level"], 3);
        for entry in json.as_array().unwrap() {
            assert!(
                !entry["scene_compatibility"]["supported_features"]
                    .as_array()
                    .unwrap()
                    .is_empty()
            );
        }
        let effects = &json[1]["scene_compatibility"];
        assert_eq!(effects["unsupported_features"].as_array().unwrap().len(), 1);
        assert_eq!(effects["warnings"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn scans_wallpaper_engine_projects_from_project_json_only() {
        let directory = tempfile::tempdir().unwrap();
        for (id, kind, file) in [
            ("1", "video", "movie.mp4"),
            ("2", "Web", "index.html"),
            ("3", "scene", "scene.pkg"),
        ] {
            let project = directory.path().join(id);
            fs::create_dir(&project).unwrap();
            fs::write(project.join(file), b"content").unwrap();
            fs::write(
                project.join("project.json"),
                format!(r#"{{"type":"{kind}","file":"{file}","title":"Project {id}","preview":"preview.jpg"}}"#),
            )
            .unwrap();
            fs::write(project.join("preview.jpg"), b"preview").unwrap();
        }
        fs::write(directory.path().join("orphan.mp4"), b"video").unwrap();

        let (entries, truncated) = scan_library(&[], &[directory.path().to_path_buf()]);

        assert!(!truncated);
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].name, "Project 1");
        assert_eq!(entries[0].path, directory.path().join("1/movie.mp4"));
        assert!(entries[0].engine_mode);
        assert_eq!(entries[0].wallpaper_type, WallpaperType::Video);
        assert_eq!(entries[1].name, "Project 2");
        assert_eq!(entries[1].wallpaper_type, WallpaperType::Web);
        assert_eq!(
            entries[1].preview_path,
            Some(directory.path().join("2/preview.jpg"))
        );
        assert_eq!(entries[1].path, directory.path().join("2/index.html"));
        // Scene type: path is the project directory, scene_compatibility is None (invalid pkg)
        assert_eq!(entries[2].name, "Project 3");
        assert_eq!(entries[2].wallpaper_type, WallpaperType::Scene);
        assert!(entries[2].scene_compatibility.is_none());
        assert!(entries[2].path.is_dir());
        assert_eq!(
            wallpaper_engine_web_root(&entries[1].path),
            directory.path().join("2").canonicalize().ok()
        );
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
    fn media_ranges_reject_invalid_requests_without_sending_the_full_file() {
        use super::*;
        use better_wallpaper_core::desktop::DesktopKind;

        let directory = tempfile::tempdir().unwrap();
        let media = directory.path().join("video.mp4");
        fs::write(&media, b"0123456789").unwrap();
        let mut config = AppConfig::default();
        config.wallpaper.path = Some(media.clone());
        config.library.paths = vec![directory.path().into()];
        let state = ApiState::new(
            Arc::new(RwLock::new(config)),
            ConfigStore::new(
                directory.path().join("config.toml"),
                directory.path().into(),
            ),
            DesktopDetection {
                kind: DesktopKind::Unknown,
                evidence: "test".into(),
                candidates: vec![],
            },
            BackendKind::Headless,
            PlaybackControl::default(),
            directory.path().into(),
            LogStore::new(10),
            LogLevelController::new("info"),
        );
        for require_library_entry in [true, false] {
            for (range, status, content_range, expected_body) in [
                (None, 200, None, Some("0123456789")),
                (Some("bytes=2-5"), 206, Some("bytes 2-5/10"), Some("2345")),
                (Some("bytes=7-"), 206, Some("bytes 7-9/10"), Some("789")),
                (Some("bytes=8-99"), 206, Some("bytes 8-9/10"), Some("89")),
                (Some("bytes=10-"), 416, Some("bytes */10"), None),
                (Some("bytes=5-2"), 416, Some("bytes */10"), None),
                (Some("bytes=0-1,4-5"), 416, Some("bytes */10"), None),
                (Some("bytes=bad"), 416, Some("bytes */10"), None),
            ] {
                let mut request = tiny_http::TestRequest::new();
                if let Some(range) = range {
                    request = request.with_header(header("Range", range));
                }
                let response = media_response(
                    &request.into(),
                    media.clone(),
                    &state,
                    require_library_entry,
                );
                assert_eq!(response.status_code(), StatusCode(status), "{range:?}");
                assert_eq!(
                    response
                        .headers()
                        .iter()
                        .find(|header| header.field.equiv("Content-Range"))
                        .map(|header| header.value.as_str()),
                    content_range,
                    "{range:?}"
                );
                let mut body = String::new();
                response.into_reader().read_to_string(&mut body).unwrap();
                if let Some(expected) = expected_body {
                    assert_eq!(body, expected);
                } else {
                    assert!(
                        serde_json::from_str::<serde_json::Value>(&body).unwrap()["error"]
                            .is_string()
                    );
                }
            }
        }
        fs::write(&media, b"").unwrap();
        let request = tiny_http::TestRequest::new().with_header(header("Range", "bytes=0-"));
        let response = wallpaper_media_response(&request.into(), &state);
        assert_eq!(response.status_code(), StatusCode(416));
        assert!(
            response
                .headers()
                .iter()
                .any(|header| header.field.equiv("Content-Range") && header.value == "bytes */0")
        );
    }

    #[test]
    fn plasma_web_config_preserves_nested_entry_and_default_properties() {
        use super::*;
        use better_wallpaper_core::desktop::DesktopKind;
        let directory = tempfile::tempdir().unwrap();
        fs::create_dir(directory.path().join("pages")).unwrap();
        fs::write(directory.path().join("pages/main.html"), "html").unwrap();
        fs::write(directory.path().join("project.json"), r#"{"type":"Web","file":"pages/main.html","general":{"properties":{"speed":{"type":"slider","value":2}}}}"#).unwrap();
        let mut config = AppConfig::default();
        config.wallpaper.wallpaper_type = WallpaperType::Web;
        config.wallpaper.path = Some(directory.path().join("pages/main.html"));
        let control = PlaybackControl::default();
        control.set_paused(true);
        let state = ApiState::new(
            Arc::new(RwLock::new(config)),
            ConfigStore::new(
                directory.path().join("config.toml"),
                directory.path().into(),
            ),
            DesktopDetection {
                kind: DesktopKind::Kde,
                evidence: "test".into(),
                candidates: vec![],
            },
            BackendKind::Kde,
            control,
            directory.path().into(),
            LogStore::new(10),
            LogLevelController::new("info"),
        );
        let response = plasma_config_response("/api/v1/plasma/config?output=DP-1", &state);
        assert_eq!(response.status_code(), StatusCode(200));
        let config: serde_json::Value = serde_json::from_reader(response.into_reader()).unwrap();
        assert_eq!(config["web_url"], "/api/v1/wallpaper/web/pages/main.html");
        assert_eq!(config["web_properties"]["speed"]["value"], 2);
        assert_eq!(config["paused"], true);
        assert_eq!(config["muted"], true);
        assert_eq!(config["wallpaper_type"], "web");
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
