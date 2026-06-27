use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::{Arc, RwLock},
    time::Duration,
};

use crate::playback::PlaybackControl;
use anyhow::Result;
use better_wallpaper_core::{AppConfig, BackendKind, ConfigStore, DesktopDetection};
use serde::Serialize;
use tiny_http::{Header, Method, Request, Response, Server, StatusCode};
use tracing::{info, warn};

const MAX_REQUEST_BYTES: usize = 1024 * 1024;

#[derive(Clone)]
pub struct ApiState {
    config: Arc<RwLock<AppConfig>>,
    store: ConfigStore,
    detection: DesktopDetection,
    backend: BackendKind,
    playback: PlaybackControl,
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
    ) -> Self {
        Self {
            config: Arc::new(RwLock::new(config)),
            store,
            detection,
            backend,
            playback,
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
    let url = request.url().split('?').next().unwrap_or("/").to_owned();
    info!(method = %method, %url, "处理管理接口请求");
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
