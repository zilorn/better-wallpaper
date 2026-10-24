use crate::i18n::{localized, tr};
use std::{
    fs::File,
    io::Read,
    path::PathBuf,
    process::Command,
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use reqwest::{Method, Url, blocking::Client};
use serde::Deserialize;
use serde_json::{Value, json};
use tracing::{info, warn};

const DEFAULT_ENDPOINT: &str = "http://127.0.0.1:43129";
const MAX_JSON: u64 = 8 * 1024 * 1024;

#[derive(Clone)]
pub struct Api {
    pub url: Url,
    client: Client,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Status {
    pub api_version: u32,
    pub desktop: String,
    pub backend: String,
    pub evidence: String,
    pub candidates: Vec<String>,
    pub playback: Playback,
    pub plasma_instances: Vec<Instance>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Playback {
    pub running: bool,
    pub paused: bool,
    pub cancelled: bool,
}

impl Playback {
    pub fn label(&self) -> &'static str {
        if self.cancelled {
            tr("Stopped")
        } else if !self.running {
            tr("Idle")
        } else if self.paused {
            tr("Paused")
        } else {
            tr("Playing")
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct Instance {
    pub output: String,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct Library {
    pub entries: Vec<Entry>,
    pub roots: Vec<String>,
    pub engine_roots: Vec<String>,
    pub truncated: bool,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Entry {
    pub name: String,
    pub path: String,
    pub wallpaper_type: String,
    pub engine_mode: bool,
    pub size_bytes: u64,
    pub scene_compatibility: Option<Value>,
    #[serde(default)]
    pub scene_properties: Vec<Value>,
}

pub struct Saved {
    pub config: Value,
    pub restart_required: bool,
    pub reload_requested: bool,
}

pub fn validate_endpoint(value: &str) -> Result<Url> {
    let url = Url::parse(value.trim()).context(tr("Invalid management address"))?;
    if url.scheme() != "http"
        || url.host_str() != Some("127.0.0.1")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
        || url.port_or_known_default() == Some(0)
    {
        bail!(tr(
            "The management address must be an HTTP origin on 127.0.0.1"
        ));
    }
    Ok(url)
}

fn discover_endpoint() -> Result<Url> {
    let directory = match std::env::var_os("XDG_RUNTIME_DIR") {
        Some(runtime) => PathBuf::from(runtime),
        None => PathBuf::from(std::env::var_os("HOME").context(tr("HOME is not set"))?)
            .join(".better-wallpaper"),
    };
    let path = directory.join("better-wallpaper/endpoint");
    match File::open(path) {
        Ok(file) => {
            let mut value = String::new();
            file.take(2049).read_to_string(&mut value)?;
            if value.len() > 2048 {
                bail!(tr("Management discovery file is too large"));
            }
            validate_endpoint(&value)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            validate_endpoint(DEFAULT_ENDPOINT)
        }
        Err(error) => Err(error).context(tr("Cannot read management discovery file")),
    }
}

impl Api {
    fn new(url: Url) -> Result<Self> {
        Ok(Self {
            url,
            client: Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(Duration::from_secs(1))
                .timeout(Duration::from_secs(15))
                .build()?,
        })
    }

    pub fn connect(explicit: Option<Url>) -> Result<(Self, Status, Value)> {
        let api = Self::new(explicit.clone().map_or_else(discover_endpoint, Ok)?)?;
        match api.status() {
            Ok(status) => {
                let config = api.config()?;
                return Ok((api, status, config));
            }
            Err(error) if explicit.is_some() => return Err(error),
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
            .context(tr("Cannot start Better Wallpaper user service"))?;
        if !output.status.success() {
            bail!(localized!(
                "Cannot start Better Wallpaper service: {}",
                "无法启动 Better Wallpaper 服务：{}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let api = Self::new(discover_endpoint()?)?;
            match api.status() {
                Ok(status) => {
                    let config = api.config()?;
                    return Ok((api, status, config));
                }
                Err(error) if Instant::now() >= deadline => {
                    return Err(error)
                        .context(tr("Service did not become ready within 10 seconds"));
                }
                Err(_) => thread::sleep(Duration::from_millis(250)),
            }
        }
    }

    fn request(&self, method: Method, path: &str, body: Option<&Value>) -> Result<Value> {
        let mut request = self.client.request(method, self.url.join(path)?);
        if path == "api/v1/status" {
            request = request.timeout(Duration::from_secs(1));
        }
        if let Some(body) = body {
            request = request.json(body);
        }
        let response = request
            .send()
            .context(tr("Cannot reach the wallpaper service"))?;
        let status = response.status();
        let mut bytes = Vec::new();
        response.take(MAX_JSON + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_JSON {
            bail!(tr("Service response is too large"));
        }
        let value: Value =
            serde_json::from_slice(&bytes).context(tr("Invalid service response"))?;
        if !status.is_success() {
            bail!(localized!(
                "Service rejected the request: {}",
                "服务拒绝了请求：{}",
                value["error"].as_str().unwrap_or(status.as_str())
            ));
        }
        Ok(value)
    }

    pub fn status(&self) -> Result<Status> {
        let status: Status =
            serde_json::from_value(self.request(Method::GET, "api/v1/status", None)?)?;
        if status.api_version != 1 {
            bail!(tr(
                "This service does not provide the Better Wallpaper v1 API"
            ));
        }
        Ok(status)
    }

    pub fn config(&self) -> Result<Value> {
        let config = self.request(Method::GET, "api/v1/config", None)?;
        // Validate the expected shape while retaining unknown fields for round trips.
        let _: better_wallpaper_core::config::AppConfig = serde_json::from_value(config.clone())?;
        if config["version"] != 2 || !config["wallpaper"].is_object() {
            bail!(tr("Unsupported configuration document"));
        }
        Ok(config)
    }

    pub fn save(&self, baseline: &Value, draft: &Value) -> Result<Saved> {
        let mut current = self.config()?;
        merge_changes(baseline, draft, &mut current, "config")?;
        let result = self.request(Method::PUT, "api/v1/config", Some(&current))?;
        if result["saved"] != true || !result["config"].is_object() {
            bail!(tr("Service did not confirm the save"));
        }
        info!("desktop configuration saved; playback reload requested");
        Ok(Saved {
            config: result["config"].clone(),
            restart_required: result["restart_required"] == true,
            reload_requested: result["reload_requested"] == true,
        })
    }

    pub fn library(&self) -> Result<Library> {
        Ok(serde_json::from_value(self.request(
            Method::GET,
            "api/v1/library",
            None,
        )?)?)
    }

    pub fn pause(&self, paused: bool) -> Result<Status> {
        self.request(
            Method::POST,
            if paused {
                "api/v1/playback/pause"
            } else {
                "api/v1/playback/resume"
            },
            None,
        )?;
        info!(paused, "desktop playback control applied");
        self.status()
    }

    pub fn logs(&self) -> Result<String> {
        let value = self.request(Method::GET, "api/v1/logs", None)?;
        let lines: Vec<String> = serde_json::from_value(value["lines"].clone())?;
        Ok(lines.join("\n"))
    }

    pub fn thumbnail(&self, path: &str) -> Result<Vec<u8>> {
        let mut url = self.url.join("api/v1/library/thumbnail")?;
        url.query_pairs_mut().append_pair("path", path);
        let response = self
            .client
            .get(url)
            .timeout(Duration::from_secs(3))
            .send()?
            .error_for_status()?;
        let mut bytes = Vec::new();
        response.take(4 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
        if bytes.len() > 4 * 1024 * 1024 {
            bail!(tr("Thumbnail is too large"));
        }
        Ok(bytes)
    }
}

// Apply only edited fields to a fresh GET, preserving concurrent tray/browser
// changes. Detect same-field conflicts rather than silently overwriting them.
pub fn merge_changes(base: &Value, draft: &Value, current: &mut Value, path: &str) -> Result<()> {
    if base == draft {
        return Ok(());
    }
    if let (Some(base), Some(draft), Some(current)) =
        (base.as_object(), draft.as_object(), current.as_object_mut())
    {
        for key in base
            .keys()
            .chain(draft.keys())
            .collect::<std::collections::BTreeSet<_>>()
        {
            let old = base.get(key).unwrap_or(&Value::Null);
            let new = draft.get(key).unwrap_or(&Value::Null);
            if old == new {
                continue;
            }
            let slot = current.entry(key.clone()).or_insert(Value::Null);
            merge_changes(old, new, slot, &format!("{path}.{key}"))?;
            if !draft.contains_key(key) {
                current.remove(key);
            }
        }
    } else {
        if current != base && current != draft {
            bail!(localized!(
                "{path} changed in another client. Reload settings before saving again.",
                "{path} 已在其他客户端中更改。请重新加载设置后再保存。"
            ));
        }
        *current = draft.clone();
    }
    Ok(())
}

pub fn select_wallpaper(config: &mut Value, entry: &Entry) {
    let changed = config["wallpaper"]["path"].as_str() != Some(&entry.path);
    config["wallpaper"]["path"] = json!(entry.path);
    config["wallpaper"]["wallpaper_type"] = json!(entry.wallpaper_type);
    config["wallpaper"]["engine_mode"] = json!(entry.engine_mode);
    if changed && entry.wallpaper_type == "scene" {
        if !config["scene"].is_object() {
            config["scene"] =
                serde_json::to_value(better_wallpaper_core::config::SceneConfig::default())
                    .expect("serializable scene defaults");
        }
        config["scene"]["properties"] = json!({});
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_loopback_origins_are_accepted() {
        assert!(validate_endpoint("http://127.0.0.1:43129\n").is_ok());
        for value in [
            "https://127.0.0.1",
            "http://localhost",
            "http://example.com",
            "http://127.0.0.1:0",
            "http://user@127.0.0.1",
            "http://127.0.0.1/path",
            "http://127.0.0.1?query",
            "http://127.0.0.1#fragment",
        ] {
            assert!(validate_endpoint(value).is_err(), "{value}");
        }
    }

    #[test]
    fn edits_preserve_concurrent_and_unknown_fields_and_detect_conflicts() {
        let base = json!({"wallpaper":{"muted":true,"fps_limit":60},"future":{"value":1}});
        let mut draft = base.clone();
        draft["wallpaper"]["fps_limit"] = json!(30);
        let mut current = base.clone();
        current["wallpaper"]["muted"] = json!(false);
        current["future"]["value"] = json!(2);
        merge_changes(&base, &draft, &mut current, "config").unwrap();
        assert_eq!(
            current,
            json!({"wallpaper":{"muted":false,"fps_limit":30},"future":{"value":2}})
        );
        current["wallpaper"]["fps_limit"] = json!(120);
        assert!(merge_changes(&base, &draft, &mut current, "config").is_err());
    }

    #[test]
    fn scene_selection_resets_only_properties_for_a_different_scene() {
        let mut config = json!({"wallpaper":{"path":"old"},"scene":{"quality":"low","properties":{"foo":"bar"}}});
        let entry: Entry = serde_json::from_value(json!({"name":tr("Scene"), "path":"new", "wallpaper_type":"scene", "engine_mode":true, "size_bytes":0})).unwrap();
        select_wallpaper(&mut config, &entry);
        assert_eq!(config["scene"], json!({"quality":"low","properties":{}}));
        config["scene"]["properties"] = json!({"foo":"kept"});
        select_wallpaper(&mut config, &entry);
        assert_eq!(config["scene"]["properties"]["foo"], "kept");
    }

    #[test]
    fn api_needs_no_html_and_rejects_redirects_and_errors() {
        for (code, body, accepted) in [
            (
                200,
                json!({"api_version":1,"desktop":"test","backend":"headless","evidence":"test","candidates":[],"playback":{"running":false,"paused":false,"cancelled":false},"plasma_instances":[]}),
                true,
            ),
            (302, json!({"error":"redirect denied"}), false),
            (409, json!({"error":"No running playback"}), false),
            (200, json!({"api_version":2}), false),
        ] {
            let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
            let api =
                Api::new(validate_endpoint(&format!("http://{}", server.server_addr())).unwrap())
                    .unwrap();
            let worker = thread::spawn(move || {
                let request = server
                    .recv_timeout(Duration::from_secs(3))
                    .unwrap()
                    .unwrap();
                assert_eq!(request.url(), "/api/v1/status");
                request
                    .respond(
                        tiny_http::Response::from_string(body.to_string()).with_status_code(code),
                    )
                    .unwrap();
            });
            assert_eq!(api.status().is_ok(), accepted);
            worker.join().unwrap();
        }
    }
}
