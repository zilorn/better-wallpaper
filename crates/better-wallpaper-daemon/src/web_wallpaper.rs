//! Web projects are served on a separate loopback origin, without management APIs.
use std::{
    fs::{self, File},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use crate::server::{asset_content_type, error_response, header, parse_byte_range, percent_decode};
use anyhow::{Context, Result, bail};
use better_wallpaper_core::{AppConfig, PlaybackControl};
use serde_json::{Value, json};
use tiny_http::{Method, Request, Response, ResponseBox, Server, StatusCode};
use tracing::{info, warn};

#[derive(Clone, Debug)]
pub struct WebProject {
    pub root: PathBuf,
    pub entry: PathBuf,
    pub properties: Value,
}

impl WebProject {
    pub fn load(path: &Path) -> Result<Self> {
        let path = path
            .canonicalize()
            .context("web wallpaper entry does not exist")?;
        let (entry, mut directory) = if path.is_dir() {
            (None, path.clone())
        } else {
            (
                Some(path.clone()),
                path.parent()
                    .context("web entry has no directory")?
                    .to_path_buf(),
            )
        };
        loop {
            let descriptor = directory.join("project.json");
            if descriptor.is_file() {
                let project: Value = serde_json::from_slice(&fs::read(descriptor)?)?;
                if !project["type"]
                    .as_str()
                    .is_some_and(|kind| kind.eq_ignore_ascii_case("web"))
                {
                    bail!("wallpaper project is not web content");
                }
                let relative = Path::new(
                    project["file"]
                        .as_str()
                        .context("web project has no entry file")?,
                );
                if relative.is_absolute() {
                    bail!("web project entry must be relative");
                }
                let configured = directory.join(relative).canonicalize()?;
                if !configured.starts_with(&directory) || !configured.is_file() {
                    bail!("web project entry is outside its root or is not a file");
                }
                if entry.as_ref().is_some_and(|entry| *entry != configured) {
                    bail!("configured web entry does not match project.json");
                }
                let properties = project
                    .pointer("/general/properties")
                    .and_then(Value::as_object)
                    .map(|properties| {
                        properties
                            .iter()
                            .filter(|(_, property)| property.get("value").is_some())
                            .map(|(name, property)| (name.clone(), property.clone()))
                            .collect::<serde_json::Map<_, _>>()
                    })
                    .unwrap_or_default();
                return Ok(Self {
                    root: directory,
                    entry: configured,
                    properties: Value::Object(properties),
                });
            }
            if !directory.pop() {
                break;
            }
        }
        let entry = entry.context("web project directory has no project.json")?;
        if !entry.is_file()
            || !entry.extension().is_some_and(|ext| {
                ext.eq_ignore_ascii_case("html") || ext.eq_ignore_ascii_case("htm")
            })
        {
            bail!("standalone web wallpaper must be an HTML file");
        }
        Ok(Self {
            root: entry.parent().unwrap().to_path_buf(),
            entry,
            properties: json!({}),
        })
    }

    pub fn entry_url(&self, prefix: &str) -> String {
        let relative = self
            .entry
            .strip_prefix(&self.root)
            .expect("validated web entry");
        format!("{prefix}{}", encode_asset_path(&relative.to_string_lossy()))
    }

    pub fn asset_response(&self, request: &Request, encoded_path: &str) -> ResponseBox {
        let Some(relative) = decode_asset_path(encoded_path) else {
            return error_response(StatusCode(400), "invalid web asset path encoding").boxed();
        };
        let candidate = if relative.is_empty() {
            self.entry.clone()
        } else {
            self.root.join(relative)
        };
        let canonical = match candidate.canonicalize() {
            Ok(path) if path.starts_with(&self.root) && path.is_file() => path,
            _ => return error_response(StatusCode(404), "web asset is unavailable").boxed(),
        };
        let mut file = match File::open(&canonical) {
            Ok(file) => file,
            Err(_) => return error_response(StatusCode(404), "cannot open web asset").boxed(),
        };
        let length = match file.metadata() {
            Ok(metadata) => metadata.len(),
            Err(_) => return error_response(StatusCode(500), "cannot read web asset").boxed(),
        };
        let mut headers = vec![
            header("Content-Type", asset_content_type(&canonical)),
            header("Accept-Ranges", "bytes"),
            header("Cache-Control", "no-store"),
            header("X-Content-Type-Options", "nosniff"),
        ];
        if let Some(range) = request
            .headers()
            .iter()
            .find(|header| header.field.equiv("Range"))
        {
            let Some((start, end)) = parse_byte_range(range.value.as_str(), length) else {
                return error_response(StatusCode(416), "invalid web asset byte range")
                    .with_header(header("Content-Range", &format!("bytes */{length}")))
                    .boxed();
            };
            if file.seek(SeekFrom::Start(start)).is_err() {
                return error_response(StatusCode(500), "cannot seek web asset").boxed();
            }
            headers.push(header(
                "Content-Range",
                &format!("bytes {start}-{end}/{length}"),
            ));
            return Response::new(
                StatusCode(206),
                headers,
                file.take(end - start + 1),
                usize::try_from(end - start + 1).ok(),
                None,
            )
            .boxed();
        }
        Response::new(
            StatusCode(200),
            headers,
            file,
            usize::try_from(length).ok(),
            None,
        )
        .boxed()
    }
}

fn encode_asset_path(path: &str) -> String {
    let mut encoded = String::new();
    for byte in path.bytes() {
        if byte.is_ascii_alphanumeric() || b"/-._~".contains(&byte) {
            encoded.push(char::from(byte));
        } else {
            use std::fmt::Write;
            let _ = write!(encoded, "%{byte:02X}");
        }
    }
    encoded
}

fn decode_asset_path(path: &str) -> Option<String> {
    // '+' is literal in URL paths, unlike query/form encoding.
    percent_decode(&path.replace('+', "%2B"))
}

pub struct AssetServer {
    pub origin: String,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl AssetServer {
    pub fn start(project: WebProject) -> Result<Self> {
        let server =
            Server::http("127.0.0.1:0").map_err(|error| anyhow::anyhow!(error.to_string()))?;
        let origin = format!("http://{}", server.server_addr());
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = Arc::clone(&stop);
        let worker = thread::Builder::new()
            .name("web-wallpaper-assets".into())
            .spawn(move || {
                while !stopped.load(Ordering::Acquire) {
                    match server.recv_timeout(Duration::from_millis(50)) {
                        Ok(Some(request)) => {
                            let url = request.url().split('?').next().unwrap_or("/");
                            let response = if *request.method() == Method::Get {
                                project.asset_response(&request, url.trim_start_matches('/'))
                            } else {
                                Response::empty(StatusCode(405)).boxed()
                            };
                            if let Err(error) = request.respond(response) {
                                warn!(%error, "failed to send web wallpaper asset");
                            }
                        }
                        Ok(None) => (),
                        Err(error) => {
                            warn!(%error, "web asset server failed");
                            break;
                        }
                    }
                }
            })?;
        Ok(Self {
            origin,
            stop,
            worker: Some(worker),
        })
    }
}

impl Drop for AssetServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

struct WebProcess(Child);
impl Drop for WebProcess {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{net::TcpStream, os::unix::fs::symlink};

    fn request(server: &AssetServer, method: &str, path: &str, extra: &str) -> String {
        let mut stream =
            TcpStream::connect(server.origin.strip_prefix("http://").unwrap()).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        write!(
            stream,
            "{method} {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n{extra}\r\n"
        )
        .unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        response
    }

    #[test]
    fn web_projects_keep_nested_paths_defaults_and_asset_boundaries() {
        let root = tempfile::tempdir().unwrap();
        let project_dir = root.path().join("project");
        fs::create_dir_all(project_dir.join("pages")).unwrap();
        fs::write(
            project_dir.join("pages/main +你好.html"),
            "<html>hello</html>",
        )
        .unwrap();
        fs::write(project_dir.join("pages/script.js"), "0123456789").unwrap();
        fs::write(root.path().join("secret"), "secret").unwrap();
        symlink(root.path().join("secret"), project_dir.join("escape")).unwrap();
        fs::write(project_dir.join("project.json"), r#"{"type":"Web","file":"pages/main +你好.html","general":{"properties":{"color":{"type":"color","value":"1 0 0"},"heading":{"type":"text"}}}}"#).unwrap();
        let project = WebProject::load(&project_dir).unwrap();
        assert_eq!(project.properties["color"]["value"], "1 0 0");
        assert!(project.properties.get("heading").is_none());
        let url = project.entry_url("/");
        assert_eq!(url, "/pages/main%20%2B%E4%BD%A0%E5%A5%BD.html");
        assert_eq!(WebProject::load(&project.entry).unwrap().root, project.root);
        let server = AssetServer::start(project).unwrap();
        assert!(request(&server, "GET", &url, "").contains("<html>hello</html>"));
        assert!(
            request(&server, "GET", "/pages/main%20+%E4%BD%A0%E5%A5%BD.html", "")
                .starts_with("HTTP/1.1 200")
        );
        let partial = request(&server, "GET", "/pages/script.js", "Range: bytes=2-5\r\n");
        assert!(partial.starts_with("HTTP/1.1 206"));
        assert!(partial.ends_with("2345"));
        assert!(
            request(&server, "GET", "/pages/script.js", "Range: bytes=90-\r\n")
                .starts_with("HTTP/1.1 416")
        );
        for forbidden in ["/%2e%2e/secret", "/escape", "/api/v1/config"] {
            assert!(request(&server, "GET", forbidden, "").starts_with("HTTP/1.1 404"));
        }
        assert!(request(&server, "GET", "/%ZZ", "").starts_with("HTTP/1.1 400"));
        assert!(request(&server, "PUT", "/api/v1/config", "").starts_with("HTTP/1.1 405"));
        let address = server.origin.strip_prefix("http://").unwrap().to_owned();
        drop(server);
        assert!(TcpStream::connect(address).is_err());
    }

    #[test]
    fn web_projects_reject_unsafe_entries_and_accept_standalone_html() {
        let root = tempfile::tempdir().unwrap();
        let project = root.path().join("project");
        fs::create_dir(&project).unwrap();
        let entry = root.path().join("main.html");
        fs::write(&entry, "html").unwrap();
        assert!(WebProject::load(&entry).is_ok());
        for file in ["../main.html", entry.to_str().unwrap(), "missing.html"] {
            fs::write(
                project.join("project.json"),
                json!({"type":"web","file":file}).to_string(),
            )
            .unwrap();
            assert!(WebProject::load(&project).is_err());
        }
        fs::write(project.join("main.html"), "html").unwrap();
        fs::write(project.join("other.html"), "html").unwrap();
        fs::write(
            project.join("project.json"),
            r#"{"type":"web","file":"main.html"}"#,
        )
        .unwrap();
        assert!(WebProject::load(&project.join("other.html")).is_err());
        fs::write(
            project.join("project.json"),
            r#"{"type":"scene","file":"main.html"}"#,
        )
        .unwrap();
        assert!(WebProject::load(&project).is_err());
    }
}

pub fn run_niri(
    config: &AppConfig,
    duration: Option<Duration>,
    control: PlaybackControl,
) -> Result<()> {
    if !config.general.restore_on_start {
        info!("web wallpaper automatic playback disabled");
        return Ok(());
    }
    let Some(path) = config.wallpaper.path.as_deref() else {
        info!("web wallpaper path not configured");
        return Ok(());
    };
    if !config.outputs.is_empty() && !config.outputs.iter().any(|output| output.enabled) {
        info!("all web wallpaper outputs are disabled");
        return Ok(());
    }
    let project = WebProject::load(path)?;
    let relative_url = project.entry_url("/");
    let properties = project.properties.clone();
    let assets = AssetServer::start(project)?;
    let executable = std::env::var_os("BETTER_WALLPAPER_WEB_PLAYER")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::current_exe()
                .unwrap_or_default()
                .with_file_name("better-wallpaper-web")
        });
    let mut process = WebProcess(Command::new(&executable).stdin(Stdio::piped()).spawn()
        .with_context(|| format!("cannot start web wallpaper renderer {}; build/install the Qt web helper or set BETTER_WALLPAPER_WEB_PLAYER", executable.display()))?);
    let mut input = process
        .0
        .stdin
        .take()
        .context("web renderer stdin unavailable")?;
    let mut paused = control.is_paused();
    writeln!(
        input,
        "{}",
        json!({"url":format!("{}{relative_url}",assets.origin), "properties":properties,
        "outputs":config.outputs.iter().filter(|output| output.enabled).map(|output| &output.name).collect::<Vec<_>>(),
        "muted":config.wallpaper.muted,"paused":paused,"fps":config.wallpaper.fps_limit})
    )?;
    info!("niri web wallpaper renderer started");
    let started = Instant::now();
    while !control.is_cancelled() && !duration.is_some_and(|limit| started.elapsed() >= limit) {
        if let Some(status) = process.0.try_wait()? {
            bail!("web wallpaper renderer exited: {status}");
        }
        if control.is_paused() != paused {
            paused = control.is_paused();
            writeln!(input, "{}", json!({"paused":paused}))?;
            info!(paused, "web wallpaper pause state synchronized");
        }
        thread::sleep(Duration::from_millis(20));
    }
    // Terminate Chromium and its surfaces before removing the private asset origin.
    drop(input);
    let deadline = Instant::now() + Duration::from_secs(2);
    while process.0.try_wait()?.is_none() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(20));
    }
    drop(process);
    info!("niri web wallpaper renderer stopped");
    Ok(())
}
