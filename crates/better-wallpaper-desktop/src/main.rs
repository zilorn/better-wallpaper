use std::{
    fs::File,
    io::Read,
    path::PathBuf,
    process::Command,
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use clap::Parser;
use tauri::{Manager, State, Url, WebviewUrl, WebviewWindow, WebviewWindowBuilder};
use tracing::{info, warn};

const DEFAULT_ENDPOINT: &str = "http://127.0.0.1:43129";

#[derive(Parser)]
#[command(version, about = "Better Wallpaper desktop management client")]
struct Cli {
    /// Connect to a specific local daemon without starting the installed service.
    #[arg(long, value_parser = validate_endpoint)]
    url: Option<Url>,
}

#[derive(Clone)]
struct Connection {
    explicit_url: Option<Url>,
    origin: Arc<Mutex<Option<Url>>>,
}

fn validate_endpoint(value: &str) -> Result<Url> {
    let url = Url::parse(value.trim()).context("invalid management URL")?;
    if url.scheme() != "http"
        || url.host_str() != Some("127.0.0.1")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
        || url.port_or_known_default() == Some(0)
    {
        bail!("management URL must be an HTTP origin on 127.0.0.1");
    }
    Ok(url)
}

fn discover_endpoint() -> Result<Url> {
    let directory = if let Some(runtime) = std::env::var_os("XDG_RUNTIME_DIR") {
        PathBuf::from(runtime)
    } else {
        PathBuf::from(std::env::var_os("HOME").context("HOME is not set")?)
            .join(".better-wallpaper")
    };
    let path = directory.join("better-wallpaper/endpoint");
    match File::open(&path) {
        Ok(file) => {
            let mut value = String::new();
            file.take(2049).read_to_string(&mut value)?;
            if value.len() > 2048 {
                bail!("management endpoint discovery file is too large");
            }
            validate_endpoint(&value)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            validate_endpoint(DEFAULT_ENDPOINT)
        }
        Err(error) => Err(error).context("failed to read management endpoint discovery file"),
    }
}

fn check_service(client: &reqwest::blocking::Client, url: &Url) -> Result<()> {
    let response = client
        .get(url.join("api/v1/status")?)
        .send()?
        .error_for_status()?;
    let mut body = Vec::new();
    response.take(65537).read_to_end(&mut body)?;
    if body.len() > 65536 {
        bail!("management status response is too large");
    }
    let status: serde_json::Value = serde_json::from_slice(&body)?;
    if status["api_version"] != 1 || !status["playback"].is_object() {
        bail!("endpoint does not provide the Better Wallpaper v1 API");
    }
    // Avoid opening a blank/404 window when the web UI has not been built.
    let response = client.get(url.clone()).send()?.error_for_status()?;
    if !response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.starts_with("text/html"))
    {
        bail!("management Web UI is unavailable; build and install the web assets");
    }
    Ok(())
}

fn connect_service(explicit_url: Option<Url>) -> Result<Url> {
    let client = reqwest::blocking::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(1))
        .build()?;
    let url = explicit_url.clone().map_or_else(discover_endpoint, Ok)?;
    match check_service(&client, &url) {
        Ok(()) => return Ok(url),
        Err(error) if explicit_url.is_some() => return Err(error),
        Err(error) => {
            warn!(%error, "management service unavailable; requesting user service start")
        }
    }
    let output = Command::new("systemctl")
        .args([
            "--user",
            "--no-ask-password",
            "--no-block",
            "start",
            "better-wallpaper.service",
        ])
        .output()
        .context("failed to start Better Wallpaper user service")?;
    if !output.status.success() {
        bail!(
            "failed to start Better Wallpaper user service: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let url = discover_endpoint()?;
        match check_service(&client, &url) {
            Ok(()) => return Ok(url),
            Err(error) if Instant::now() >= deadline => {
                return Err(error)
                    .context("management service did not become ready within 10 seconds");
            }
            Err(_) => thread::sleep(Duration::from_millis(250)),
        }
    }
}

#[tauri::command]
async fn connect(window: WebviewWindow, state: State<'_, Connection>) -> Result<(), String> {
    let explicit_url = state.explicit_url.clone();
    let result = tauri::async_runtime::spawn_blocking(move || connect_service(explicit_url))
        .await
        .map_err(|error| error.to_string())?;
    let url = result.map_err(|error| {
        warn!(%error, "desktop client could not connect to management service");
        format!("{error:#}")
    })?;
    *state.origin.lock().map_err(|error| error.to_string())? = Some(url.clone());
    window
        .navigate(url.clone())
        .map_err(|error| error.to_string())?;
    info!(%url, "desktop client connected to management service");
    Ok(())
}

fn main() -> Result<()> {
    // Configure WebKit before Tauri or any worker threads start. NVIDIA's GBM
    // path can fail to allocate the webview buffers even when API requests work.
    // Preserve explicit user overrides and leave other GPU stacks unchanged.
    let webkit_workaround = cfg!(target_os = "linux")
        && std::path::Path::new("/sys/module/nvidia/version").is_file()
        && std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none();
    if webkit_workaround {
        // SAFETY: main is still single-threaded; no GTK, WebKit or async runtime
        // has been initialized, and no concurrent environment readers exist.
        unsafe { std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1") };
    }
    tracing_subscriber::fmt::init();
    if webkit_workaround {
        info!("enabled WebKit DMA-BUF compatibility workaround for NVIDIA");
    }
    let cli = Cli::parse();
    let connection = Connection {
        explicit_url: cli.url,
        origin: Arc::new(Mutex::new(None)),
    };
    let origin = Arc::clone(&connection.origin);
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            if let Some(window) = app.get_webview_window("main")
                && let Err(error) = window.unminimize().and_then(|()| window.set_focus())
            {
                warn!(%error, "failed to focus existing desktop client");
            }
        }))
        .manage(connection)
        .invoke_handler(tauri::generate_handler![connect])
        .setup(move |app| {
            WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
                .title("Better Wallpaper")
                .inner_size(1200.0, 800.0)
                .min_inner_size(760.0, 540.0)
                .on_navigation(move |url| {
                    let allowed = is_bootstrap_url(url)
                        || origin.lock().is_ok_and(|value| {
                            value
                                .as_ref()
                                .is_some_and(|base| url.origin() == base.origin())
                        });
                    if !allowed {
                        warn!(%url, "blocked desktop navigation outside management origin");
                    }
                    allowed
                })
                .build()?;
            info!("desktop management window started");
            Ok(())
        })
        .run(tauri::generate_context!())
        .context("failed to run desktop client")
}

fn is_bootstrap_url(url: &Url) -> bool {
    (url.scheme() == "tauri" && url.host_str() == Some("localhost"))
        || (url.scheme() == "http" && url.host_str() == Some("tauri.localhost"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{BufRead, BufReader, Write},
        net::TcpListener,
    };

    #[test]
    fn discovery_accepts_only_local_http_origins() {
        assert!(validate_endpoint("http://127.0.0.1:43129\n").is_ok());
        for value in [
            "https://127.0.0.1:43129",
            "http://localhost:43129",
            "http://example.com",
            "http://127.0.0.1:0",
            "http://user@127.0.0.1",
            "http://127.0.0.1/api",
            "http://127.0.0.1?url=evil",
            "http://127.0.0.1#evil",
        ] {
            assert!(validate_endpoint(value).is_err(), "{value}");
        }
    }

    #[test]
    fn unrelated_service_and_redirect_are_rejected() {
        for (response, accepted) in [
            (
                "200 OK\r\nContent-Type: application/json\r\n\r\n{\"api_version\":1,\"playback\":{}}",
                true,
            ),
            ("200 OK\r\n\r\n{\"api_version\":2,\"playback\":{}}", false),
            ("302 Found\r\nLocation: http://example.com\r\n\r\n", false),
        ] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let url =
                validate_endpoint(&format!("http://{}", listener.local_addr().unwrap())).unwrap();
            let server = thread::spawn(move || {
                for index in 0..if accepted { 2 } else { 1 } {
                    let (mut socket, _) = listener.accept().unwrap();
                    socket
                        .set_read_timeout(Some(Duration::from_secs(2)))
                        .unwrap();
                    let mut request = Vec::new();
                    let mut reader = BufReader::new(&mut socket);
                    while !request.ends_with(b"\r\n\r\n") {
                        assert!(reader.read_until(b'\n', &mut request).unwrap() > 0);
                        assert!(request.len() <= 4096);
                    }
                    let body = if index == 0 {
                        response
                    } else {
                        "200 OK\r\nContent-Type: text/html\r\n\r\n<!doctype html>"
                    };
                    write!(socket, "HTTP/1.1 {body}").unwrap();
                }
            });
            let client = reqwest::blocking::Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(Duration::from_secs(2))
                .build()
                .unwrap();
            assert_eq!(check_service(&client, &url).is_ok(), accepted);
            server.join().unwrap();
        }
    }
}
