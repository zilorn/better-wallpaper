//! Web projects are served on a separate loopback origin, without management APIs.
use std::{
    collections::BTreeMap,
    fs::File,
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
                if descriptor.metadata()?.len() > 1024 * 1024 {
                    bail!("web descriptor exceeds 1 MiB");
                }
                let project: Value =
                    serde_json::from_reader(File::open(descriptor)?.take(1024 * 1024 + 1))?;
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
                if properties.len() > 256 {
                    bail!("web project exceeds 256 properties");
                }
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

    /// Resolve typed overrides without granting access outside the project.
    pub fn resolved_properties(
        &self,
        overrides: &BTreeMap<String, String>,
        prefix: &str,
    ) -> Result<Value> {
        let mut properties = self.properties.clone();
        for (key, value) in overrides {
            let property = properties
                .get_mut(key)
                .with_context(|| format!("unknown web property: {key}"))?;
            let parsed = match property["type"].as_str().unwrap_or("") {
                "bool" => Value::Bool(
                    value
                        .parse()
                        .context("web checkbox must be true or false")?,
                ),
                "slider" => {
                    let number: f64 = value
                        .parse()
                        .context("web slider must be a finite number")?;
                    let min = property["min"].as_f64().unwrap_or(0.0);
                    let max = property["max"].as_f64().unwrap_or(100.0);
                    if !number.is_finite()
                        || number < min
                        || number > max
                        || (!property["fraction"].as_bool().unwrap_or(false)
                            && number.fract() != 0.0)
                    {
                        bail!("web slider {key} is outside its range");
                    }
                    json!(number)
                }
                "combo" => {
                    let options = property["options"]
                        .as_array()
                        .context("web combo has no options")?;
                    let option = options
                        .iter()
                        .find(|option| scalar_text(&option["value"]).as_deref() == Some(value))
                        .context("invalid web combo selection")?;
                    option["value"].clone()
                }
                "color" => {
                    let components: Vec<_> =
                        value.split_whitespace().map(str::parse::<f64>).collect();
                    if components.len() != 3
                        || !components.iter().all(|v| {
                            v.as_ref()
                                .is_ok_and(|v| v.is_finite() && (0.0..=1.0).contains(v))
                        })
                    {
                        bail!("web color must contain three values in 0..=1");
                    }
                    json!(value)
                }
                "textinput" | "file" => json!(value),
                other => bail!("web property type {other} is not editable"),
            };
            property["value"] = parsed;
        }
        for (_, property) in properties
            .as_object_mut()
            .context("invalid web properties")?
        {
            if property["type"] == "combo"
                && let Some(label) = property["options"]
                    .as_array()
                    .and_then(|options| {
                        options
                            .iter()
                            .find(|option| option["value"] == property["value"])
                    })
                    .and_then(|option| option.get("label"))
            {
                property["text"] = label.clone();
            }
            if property["type"] == "file" {
                let value = property["value"].as_str().unwrap_or("");
                if value.is_empty() {
                    continue;
                }
                let relative = Path::new(value);
                let canonical = self
                    .root
                    .join(relative)
                    .canonicalize()
                    .context("web file property is unavailable")?;
                if !canonical.starts_with(&self.root) || !canonical.is_file() {
                    bail!("web file property is outside its project");
                }
                let relative = canonical.strip_prefix(&self.root)?;
                property["value"] = json!(format!(
                    "{prefix}{}",
                    encode_asset_path(&relative.to_string_lossy())
                ));
            }
        }
        Ok(properties)
    }

    pub fn configured_properties(&self, config: &AppConfig, prefix: &str) -> Result<Value> {
        let empty = BTreeMap::new();
        let overrides = config
            .wallpaper
            .path
            .as_ref()
            .and_then(|path| config.web.properties.get(path.to_string_lossy().as_ref()))
            .unwrap_or(&empty);
        self.resolved_properties(overrides, prefix)
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

fn scalar_text(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => Some(value.clone()),
        Value::Number(_) | Value::Bool(_) => Some(value.to_string()),
        _ => None,
    }
}

/// Reuse the native editor schema while preserving numeric/bool combo types at runtime.
/// Conversion here affects editor strings only, never authored callback values.
pub(crate) fn editable_properties(
    descriptor: &str,
) -> Result<Vec<better_wallpaper_scene_format::UserProperty>> {
    let mut descriptor: Value = serde_json::from_str(descriptor)?;
    if let Some(properties) = descriptor
        .pointer_mut("/general/properties")
        .and_then(Value::as_object_mut)
    {
        for property in properties.values_mut() {
            if property["type"] == "combo" {
                if let Some(value) = scalar_text(&property["value"]) {
                    property["value"] = json!(value);
                }
                if let Some(options) = property["options"].as_array_mut() {
                    for option in options {
                        if let Some(value) = scalar_text(&option["value"]) {
                            option["value"] = json!(value);
                        }
                    }
                }
            }
        }
    }
    Ok(
        better_wallpaper_scene_format::parse_project_properties(&descriptor.to_string())?
            .into_iter()
            .map(|(_, property)| property)
            .filter(|property| {
                matches!(
                    property.prop_type,
                    better_wallpaper_scene_format::scene::PropertyType::Slider { .. }
                        | better_wallpaper_scene_format::scene::PropertyType::Bool
                        | better_wallpaper_scene_format::scene::PropertyType::Combo { .. }
                        | better_wallpaper_scene_format::scene::PropertyType::Color
                        | better_wallpaper_scene_format::scene::PropertyType::File
                        | better_wallpaper_scene_format::scene::PropertyType::TextInput
                )
            })
            .collect(),
    )
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
    pub protocol_url: String,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl AssetServer {
    pub fn start(project: WebProject) -> Result<Self> {
        Self::start_with_options(project, Default::default())
    }

    pub fn start_with_options(
        project: WebProject,
        options: crate::web_protocol::WebOptions,
    ) -> Result<Self> {
        let server =
            Server::http("127.0.0.1:0").map_err(|error| anyhow::anyhow!(error.to_string()))?;
        let origin = format!("http://{}", server.server_addr());
        let mut entropy = [0u8; 16];
        File::open("/dev/urandom")?.read_exact(&mut entropy)?;
        let token: String = entropy.iter().map(|byte| format!("{byte:02x}")).collect();
        let protocol_path = format!("/__better_wallpaper_frame/{token}");
        let protocol_url = format!("{origin}{protocol_path}");
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = Arc::clone(&stop);
        let mut protocol = crate::web_protocol::WebProtocol::start(options)?;
        let worker = thread::Builder::new()
            .name("web-wallpaper-assets".into())
            .spawn(move || {
                while !stopped.load(Ordering::Acquire) {
                    match server.recv_timeout(Duration::from_millis(50)) {
                        Ok(Some(request)) => {
                            let url = request.url().split('?').next().unwrap_or("/");
                            let response = if *request.method() == Method::Get {
                                if url == protocol_path {
                                    crate::server::json_response(StatusCode(200), &protocol.frame())
                                        .with_header(header("Cache-Control", "no-store"))
                                        .boxed()
                                } else {
                                    project.asset_response(&request, url.trim_start_matches('/'))
                                }
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
            protocol_url,
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
    let assets = AssetServer::start_with_options(
        project.clone(),
        crate::web_protocol::WebOptions::from(&config.web),
    )?;
    let properties = project.configured_properties(config, &format!("{}/", assets.origin))?;
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
        "protocolUrl":assets.protocol_url,
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, net::TcpStream, os::unix::fs::symlink};

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
    fn numeric_and_boolean_combo_values_keep_their_authored_types() {
        let descriptor = r#"{"general":{"properties":{"mode":{"type":"combo","value":0,"options":[{"label":"Zero","value":0},{"label":"One","value":1},{"label":"Yes","value":true}]}}}}"#;
        let editors = editable_properties(descriptor).unwrap();
        assert_eq!(serde_json::to_value(&editors).unwrap()[0]["value"], "0");
        let descriptor: Value = serde_json::from_str(descriptor).unwrap();
        let project = WebProject {
            root: PathBuf::from("/unused"),
            entry: PathBuf::from("/unused/index.html"),
            properties: descriptor["general"]["properties"].clone(),
        };
        assert_eq!(
            project.resolved_properties(&BTreeMap::new(), "/").unwrap()["mode"]["text"],
            "Zero"
        );
        for (wire, expected, label) in [("1", json!(1), "One"), ("true", json!(true), "Yes")] {
            let properties = project
                .resolved_properties(&BTreeMap::from([("mode".into(), wire.into())]), "/")
                .unwrap();
            assert_eq!(properties["mode"]["value"], expected);
            assert_eq!(properties["mode"]["text"], label);
        }
    }

    #[test]
    fn typed_web_overrides_reset_and_files_stay_in_the_project() {
        let fixture =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/web-protocol");
        let project = WebProject::load(&fixture).unwrap();
        let defaults = project
            .resolved_properties(&BTreeMap::new(), "http://127.0.0.1:1/")
            .unwrap();
        let overrides = BTreeMap::from([
            ("enabled".into(), "false".into()),
            ("speed".into(), "0.25".into()),
            ("mode".into(), "fast".into()),
            ("caption".into(), "".into()),
            ("tint".into(), "0.25 0.5 1".into()),
        ]);
        let effective = project
            .resolved_properties(&overrides, "http://127.0.0.1:1/")
            .unwrap();
        assert_eq!(effective["enabled"]["value"], false);
        assert_eq!(effective["speed"]["value"], 0.25);
        assert_eq!(effective["mode"]["text"], "Fast");
        assert_eq!(effective["caption"]["value"], "");
        assert_eq!(
            effective["image"]["value"],
            "http://127.0.0.1:1/pages/probe.svg"
        );
        assert_eq!(
            project
                .resolved_properties(&BTreeMap::new(), "http://127.0.0.1:1/")
                .unwrap(),
            defaults
        );
        for (key, value) in [
            ("enabled", "yes"),
            ("speed", "NaN"),
            ("speed", "11"),
            ("mode", "unknown"),
            ("tint", "1 2 3"),
            ("image", "/etc/passwd"),
            ("image", "../Cargo.toml"),
            ("missing", "value"),
        ] {
            assert!(
                project
                    .resolved_properties(&BTreeMap::from([(key.into(), value.into())]), "/")
                    .is_err(),
                "{key}={value}"
            );
        }
        let temporary = tempfile::tempdir().unwrap();
        let store = better_wallpaper_core::ConfigStore::new(
            temporary.path().join("config.toml"),
            temporary.path().into(),
        );
        let mut config = AppConfig::default();
        config.wallpaper.path = Some(project.entry.clone());
        config
            .web
            .properties
            .insert(project.entry.to_string_lossy().into_owned(), overrides);
        store.save(&config).unwrap();
        assert_eq!(
            project
                .configured_properties(&store.load_or_create().unwrap(), "http://127.0.0.1:1/")
                .unwrap(),
            effective
        );
        config.wallpaper.path = Some(fixture);
        assert_eq!(
            project
                .configured_properties(&config, "http://127.0.0.1:1/")
                .unwrap(),
            defaults,
            "a different configured project cannot inherit edits"
        );
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
        for forbidden in [
            "/%2e%2e/secret",
            "/escape",
            "/api/v1/config",
            "/__better_wallpaper_frame",
        ] {
            assert!(request(&server, "GET", forbidden, "").starts_with("HTTP/1.1 404"));
        }
        assert!(request(&server, "GET", "/%ZZ", "").starts_with("HTTP/1.1 400"));
        assert!(request(&server, "PUT", "/api/v1/config", "").starts_with("HTTP/1.1 405"));
        let capability = server.protocol_url.strip_prefix(&server.origin).unwrap();
        let frame = request(&server, "GET", capability, "");
        assert!(frame.starts_with("HTTP/1.1 200"));
        assert!(frame.contains("\"audio_enabled\":false"));
        assert_eq!(capability.rsplit('/').next().unwrap().len(), 32);
        let address = server
            .origin
            .strip_prefix("http://")
            .unwrap()
            .parse()
            .unwrap();
        drop(server);
        // tiny_http signals its detached accept thread in Drop but does not join it.
        // Require eventual closure with a deadline; an immediate connect races shutdown.
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            match TcpStream::connect_timeout(&address, Duration::from_millis(50)) {
                Err(error) if error.kind() == std::io::ErrorKind::ConnectionRefused => break,
                Err(error) => panic!("unexpected asset listener shutdown error: {error}"),
                Ok(stream) => drop(stream),
            }
            assert!(
                Instant::now() < deadline,
                "asset listener did not close within two seconds"
            );
            thread::sleep(Duration::from_millis(10));
        }
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
