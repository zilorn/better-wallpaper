use std::{
    collections::HashSet,
    fs::{self, File},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::{Arc, RwLock},
    thread,
    time::{Duration, Instant},
};

use crate::playback::PlaybackControl;
use anyhow::Result;
use base64::{Engine, engine::general_purpose::STANDARD};
use better_wallpaper_core::{AppConfig, BackendKind, ConfigStore, DesktopDetection};
use serde::Serialize;
use sha1::{Digest, Sha1};
use tiny_http::{Header, Method, Request, Response, ResponseBox, Server, StatusCode};
use tracing::{info, warn};

const MAX_REQUEST_BYTES: usize = 1024 * 1024;
const STATUS_POLL_INTERVAL: Duration = Duration::from_millis(250);
const STATUS_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(15);
const MAX_LIBRARY_ENTRIES: usize = 1_000;
const MAX_LIBRARY_DEPTH: usize = 4;

#[derive(Clone)]
pub struct ApiState {
    config: Arc<RwLock<AppConfig>>,
    store: ConfigStore,
    detection: DesktopDetection,
    backend: BackendKind,
    playback: PlaybackControl,
    home: PathBuf,
}

#[derive(Serialize)]
struct StatusPayload<'a> {
    api_version: u8,
    desktop: &'a str,
    evidence: &'a str,
    candidates: &'a [String],
    backend: BackendKind,
    playback: PlaybackPayload,
}

#[derive(Serialize)]
struct PlaybackPayload {
    running: bool,
    paused: bool,
    cancelled: bool,
}

impl ApiState {
    pub fn new(
        config: AppConfig,
        store: ConfigStore,
        detection: DesktopDetection,
        backend: BackendKind,
        playback: PlaybackControl,
        home: PathBuf,
    ) -> Self {
        Self {
            config: Arc::new(RwLock::new(config)),
            store,
            detection,
            backend,
            playback,
            home,
        }
    }
}

pub fn serve(bind: &str, web_root: PathBuf, state: ApiState) -> Result<()> {
    let server = Server::http(bind).map_err(|error| anyhow::anyhow!(error.to_string()))?;
    info!(bind, web_root = %web_root.display(), "本机管理服务已启动");
    loop {
        match server.recv_timeout(Duration::from_secs(1)) {
            Ok(Some(request)) => handle_request(request, &web_root, &state),
            Ok(None) => {}
            Err(error) => warn!(%error, "接收 HTTP 请求失败"),
        }
    }
}

fn handle_request(mut request: Request, web_root: &Path, state: &ApiState) {
    let method = request.method().clone();
    let request_url = request.url().to_owned();
    let url = request_url.split('?').next().unwrap_or("/").to_owned();
    info!(method = %method, %url, "处理管理接口请求");
    if method == Method::Get && url == "/api/v1/ws" {
        upgrade_websocket(request, state.clone());
        return;
    }
    if method == Method::Get && url == "/api/v1/library/media" {
        let response = library_media_response(&request, &request_url, state);
        if let Err(error) = request.respond(response) {
            warn!(%error, %url, "发送壁纸库媒体响应失败");
        }
        return;
    }
    if method == Method::Get && url == "/api/v1/wallpaper/media" {
        let response = wallpaper_media_response(&request, state);
        if let Err(error) = request.respond(response) {
            warn!(%error, %url, "发送当前壁纸媒体响应失败");
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
            },
        ),
        (Method::Get, "/api/v1/config") => match state.config.read() {
            Ok(config) => json_response(StatusCode(200), &*config),
            Err(_) => error_response(StatusCode(500), "配置锁已损坏"),
        },
        (Method::Get, "/api/v1/library") => library_response(state),
        (Method::Put, "/api/v1/config") => update_config(&mut request, state),
        (Method::Post, "/api/v1/playback/pause") => set_paused(state, true),
        (Method::Post, "/api/v1/playback/resume") => set_paused(state, false),
        (Method::Get, _) => static_response(web_root, &url),
        _ => error_response(StatusCode(404), "接口不存在"),
    };
    if let Err(error) = request.respond(response) {
        warn!(%error, %url, "发送 HTTP 响应失败");
    }
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
        "壁纸库扫描完成"
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
        return error_response(StatusCode(400), "缺少媒体 path 参数").boxed();
    };
    let Some(path) = percent_decode(encoded_path).map(PathBuf::from) else {
        return error_response(StatusCode(400), "媒体 path 参数编码无效").boxed();
    };
    media_response(request, path, state, true)
}

fn wallpaper_media_response(request: &Request, state: &ApiState) -> ResponseBox {
    let path = match state.config.read() {
        Ok(config) => config.wallpaper.path.clone(),
        Err(_) => return error_response(StatusCode(500), "配置锁已损坏").boxed(),
    };
    let Some(path) = path else {
        return error_response(StatusCode(404), "尚未配置当前壁纸").boxed();
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
            warn!(path = %path.display(), %error, "壁纸库媒体文件不存在");
            return error_response(StatusCode(404), "媒体文件不存在").boxed();
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
        warn!(path = %canonical.display(), "拒绝读取壁纸库范围外的媒体文件");
        return error_response(StatusCode(403), "媒体文件不在壁纸库中").boxed();
    }
    let mut file = match File::open(&canonical) {
        Ok(file) => file,
        Err(error) => {
            warn!(path = %canonical.display(), %error, "打开壁纸库媒体失败");
            return error_response(StatusCode(404), "无法打开媒体文件").boxed();
        }
    };
    let length = match file.metadata() {
        Ok(metadata) => metadata.len(),
        Err(error) => {
            warn!(path = %canonical.display(), %error, "读取壁纸库媒体大小失败");
            return error_response(StatusCode(500), "无法读取媒体文件").boxed();
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
            return error_response(StatusCode(500), "无法定位媒体文件").boxed();
        }
        let response_length = end - start + 1;
        info!(path = %canonical.display(), start, end, "传输壁纸库媒体分段");
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
    info!(path = %canonical.display(), length, "传输完整壁纸库媒体");
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
    Header::from_bytes(name, value).expect("HTTP 响应头有效")
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
                warn!(path = %canonical.display(), %error, "跳过无法读取的壁纸库目录");
                continue;
            }
        };
        for child in children.flatten() {
            let path = child.path();
            if child.file_type().is_ok_and(|kind| kind.is_symlink()) {
                warn!(path = %path.display(), "跳过壁纸库中的符号链接");
                continue;
            }
            let metadata = match child.metadata() {
                Ok(metadata) => metadata,
                Err(error) => {
                    warn!(path = %path.display(), %error, "跳过无法读取元数据的壁纸库条目");
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
        warn!("拒绝无效的 WebSocket 升级请求");
        let _ = request.respond(error_response(StatusCode(400), "无效的 WebSocket 升级请求"));
        return;
    };
    let response = Response::empty(StatusCode(101))
        .with_header(Header::from_bytes("Upgrade", "websocket").expect("WebSocket 响应头有效"))
        .with_header(Header::from_bytes("Connection", "Upgrade").expect("WebSocket 响应头有效"))
        .with_header(
            Header::from_bytes("Sec-WebSocket-Accept", websocket_accept(&key))
                .expect("WebSocket 接受响应头有效"),
        );
    let mut stream = request.upgrade("websocket", response);
    if let Err(error) = thread::Builder::new()
        .name("status-websocket".into())
        .spawn(move || {
            info!("WebSocket 状态订阅已连接");
            let mut previous = Vec::new();
            let mut last_sent = Instant::now() - STATUS_HEARTBEAT_INTERVAL;
            loop {
                let payload = match serde_json::to_vec(&serde_json::json!({
                    "type": "status",
                    "data": status_payload(&state),
                })) {
                    Ok(payload) => payload,
                    Err(error) => {
                        warn!(%error, "序列化 WebSocket 状态失败");
                        break;
                    }
                };
                if (payload != previous || last_sent.elapsed() >= STATUS_HEARTBEAT_INTERVAL)
                    && write_websocket_text(&mut stream, &payload).is_err()
                {
                    info!("WebSocket 状态订阅已断开");
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
        warn!(%error, "创建 WebSocket 状态推送线程失败");
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
        return error_response(StatusCode(409), "播放已停止，无法更改暂停状态");
    }
    if !state.playback.is_running() {
        return error_response(StatusCode(409), "当前没有正在运行的播放任务");
    }
    state.playback.set_paused(paused);
    info!(paused, "管理接口已应用播放控制命令");
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
        return error_response(StatusCode(400), "读取请求体失败");
    }
    if body.len() > MAX_REQUEST_BYTES {
        return error_response(StatusCode(413), "请求体过大");
    }
    let config: AppConfig = match serde_json::from_slice(&body) {
        Ok(config) => config,
        Err(error) => return error_response(StatusCode(400), &format!("JSON 无效: {error}")),
    };
    if let Err(error) = state.store.save(&config) {
        warn!(%error, "管理接口配置更新失败");
        return error_response(StatusCode(400), &error.to_string());
    }
    let config = match state.store.load_or_create() {
        Ok(config) => config,
        Err(error) => {
            warn!(%error, "重新读取已保存配置失败");
            return error_response(StatusCode(500), &error.to_string());
        }
    };
    match state.config.write() {
        Ok(mut current) => *current = config,
        Err(_) => return error_response(StatusCode(500), "配置锁已损坏"),
    }
    info!("管理接口配置更新完成；播放进程下次启动时生效");
    json_response(
        StatusCode(200),
        &serde_json::json!({ "saved": true, "restart_required": true }),
    )
}

fn static_response(web_root: &Path, url: &str) -> Response<std::io::Cursor<Vec<u8>>> {
    let relative = if url == "/" {
        "index.html"
    } else {
        url.trim_start_matches('/')
    };
    if relative.contains("..") {
        return error_response(StatusCode(400), "无效路径");
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
            warn!(path = %path.display(), %error, "读取 Web 静态资源失败");
            error_response(StatusCode(404), "Web UI 尚未构建，请运行 bun run build")
        }
    }
}

fn json_response<T: Serialize>(
    status: StatusCode,
    value: &T,
) -> Response<std::io::Cursor<Vec<u8>>> {
    match serde_json::to_vec(value) {
        Ok(body) => response(status, "application/json; charset=utf-8", body),
        Err(error) => error_response(StatusCode(500), &format!("序列化响应失败: {error}")),
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
            Header::from_bytes("Content-Type", content_type).expect("静态 Content-Type 有效"),
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
        parse_byte_range, percent_decode, scan_library, websocket_accept, write_websocket_text,
    };

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
}
