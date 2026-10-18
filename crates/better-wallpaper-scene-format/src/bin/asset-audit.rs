//! Bounded, read-only asset inventory. No extraction, rendering or target execution.
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, OpenOptions},
    io::{self, Read},
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Component, Path, PathBuf},
};

use anyhow::{Context, Result, ensure};
use better_wallpaper_scene_format::{PkgReader, TexError, TexTexture, resolve_texture_path};
use clap::Parser;
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, Parser, Serialize)]
struct Limits {
    /// Maximum directory entries inspected across all loose roots.
    #[arg(long, default_value_t = 20_000, value_parser = clap::value_parser!(u32).range(1..=100_000))]
    max_walk_entries: u32,
    /// Maximum inventory records, including packages and their members.
    #[arg(long, default_value_t = 10_000, value_parser = clap::value_parser!(u32).range(1..=100_000))]
    max_records: u32,
    #[arg(long, default_value_t = 16, value_parser = clap::value_parser!(u32).range(1..=64))]
    max_depth: u32,
    /// Maximum bytes per file (including each explicitly selected PKG).
    #[arg(long, default_value_t = 134_217_728, value_parser = clap::value_parser!(u64).range(1..=268_435_456))]
    max_file_bytes: u64,
    /// Maximum filesystem bytes read; PKG members share their parent's bytes.
    #[arg(long, default_value_t = 1_073_741_824, value_parser = clap::value_parser!(u64).range(1..=17_179_869_184))]
    max_total_bytes: u64,
    /// Maximum retained decoded mip bytes per TEX (plus bounded transient buffers).
    #[arg(long, default_value_t = 134_217_728, value_parser = clap::value_parser!(u64).range(1..=268_435_456))]
    max_decoded_bytes: u64,
    #[arg(long, default_value_t = 100_000, value_parser = clap::value_parser!(u32).range(1..=1_000_000))]
    max_references: u32,
}

#[derive(Parser)]
#[command(about = "Audit loose TEX/JSON roots and explicitly selected PKGs as JSON on stdout")]
struct Args {
    /// Independent loose TEX/JSON tree, as NAME=PATH; repeatable.
    #[arg(long, value_name = "NAME=PATH")]
    root: Vec<String>,
    /// Shared loose TEX/JSON tree searched as a reference fallback; repeatable.
    #[arg(long, value_name = "NAME=PATH")]
    shared: Vec<String>,
    /// Explicit package, as NAME=PATH; packages found in loose trees are not opened.
    #[arg(long, value_name = "NAME=PATH")]
    pkg: Vec<String>,
    #[command(flatten)]
    limits: Limits,
}

#[derive(Serialize)]
struct Source {
    name: String,
    kind: &'static str,
    #[serde(skip)]
    path: PathBuf,
}

#[derive(Serialize)]
struct Record {
    source: String,
    path: String,
    size: u64,
    sha256: Option<String>,
    kind: &'static str,
    status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    failure_class: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    format: Option<Value>,
}

#[derive(Serialize)]
struct Reference {
    source: String,
    path: String,
    pointer: String,
    value: String,
    candidate: Option<String>,
    status: &'static str,
    matches: Vec<Value>,
}

#[derive(Serialize)]
struct Audit {
    schema_version: u32,
    sources: Vec<Source>,
    limits: Limits,
    /// False if traversal, reading, JSON-reference coverage or a budget was incomplete.
    complete: bool,
    walk_entries: u32,
    input_bytes: u64,
    excluded_loose_files: u64,
    notices: Vec<Value>,
    records: Vec<Record>,
    references: Vec<Reference>,
    summary: BTreeMap<String, usize>,
}

fn main() -> Result<()> {
    let audit = run(Args::parse())?;
    serde_json::to_writer(io::stdout().lock(), &audit)?;
    println!();
    // Resource failures are inventory outcomes. Incomplete coverage gets a distinct exit code.
    if !audit.complete {
        std::process::exit(2);
    }
    Ok(())
}

fn run(args: Args) -> Result<Audit> {
    let mut sources = Vec::new();
    let mut names = BTreeSet::new();
    for (kind, specs) in [
        ("root", args.root),
        ("shared", args.shared),
        ("pkg", args.pkg),
    ] {
        for spec in specs {
            let (name, path) = spec.split_once('=').context("source must be NAME=PATH")?;
            ensure!(
                !name.is_empty()
                    && name.len() <= 64
                    && name
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b)),
                "source names must contain 1–64 ASCII letters, digits, underscores or hyphens"
            );
            ensure!(
                names.insert(name.to_owned()),
                "duplicate source name: {name}"
            );
            ensure!(!path.is_empty(), "source path is empty");
            sources.push(Source {
                name: name.into(),
                kind,
                path: path.into(),
            });
        }
    }
    ensure!(
        !sources.is_empty(),
        "select at least one --root, --shared or --pkg"
    );
    ensure!(sources.len() <= 32, "at most 32 sources may be selected");
    sources.sort_by(|a, b| a.name.cmp(&b.name));
    let mut audit = Audit {
        schema_version: 1,
        sources,
        limits: args.limits,
        complete: true,
        walk_entries: 0,
        input_bytes: 0,
        excluded_loose_files: 0,
        notices: Vec::new(),
        records: Vec::new(),
        references: Vec::new(),
        summary: BTreeMap::new(),
    };
    for index in 0..audit.sources.len() {
        let path = audit.sources[index].path.clone();
        let name = audit.sources[index].name.clone();
        if audit.sources[index].kind == "pkg" {
            audit.file(&name, &path, "", true);
        } else {
            audit.walk(&name, &path, &path, 0);
        }
    }
    audit
        .records
        .sort_by(|a, b| (&a.source, &a.path).cmp(&(&b.source, &b.path)));
    audit.resolve_references();
    for record in &audit.records {
        *audit
            .summary
            .entry(format!("{}:{}", record.kind, record.status))
            .or_default() += 1;
        if let Some(class) = &record.failure_class {
            *audit.summary.entry(format!("failure:{class}")).or_default() += 1;
        }
    }
    Ok(audit)
}

impl Audit {
    fn notice(&mut self, source: &str, path: &str, class: &str) {
        self.complete = false;
        // Bound diagnostics too; repeated exhaustion does not need a notice per file.
        if self.notices.len() < self.limits.max_records.min(256) as usize {
            self.notices
                .push(json!({"source":source, "path":path, "class":class}));
        }
    }

    fn walk(&mut self, source: &str, root: &Path, path: &Path, depth: u32) {
        let relative = path
            .strip_prefix(root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        let metadata = match fs::symlink_metadata(path) {
            Ok(metadata) => metadata,
            Err(_) => {
                self.notice(source, &relative, "io_error");
                return;
            }
        };
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            self.notice(source, &relative, "not_a_directory_or_symlink");
            return;
        }
        if depth > self.limits.max_depth {
            self.notice(source, &relative, "depth_limit");
            return;
        }
        let entries = match fs::read_dir(path) {
            Ok(entries) => entries,
            Err(_) => {
                self.notice(source, &relative, "io_error");
                return;
            }
        };
        let remaining = (self.limits.max_walk_entries - self.walk_entries) as usize;
        let mut entries = match entries.take(remaining + 1).collect::<io::Result<Vec<_>>>() {
            Ok(entries) => entries,
            Err(_) => {
                self.notice(source, &relative, "io_error");
                return;
            }
        };
        if entries.len() > remaining {
            // Do not select an arbitrary readdir prefix; skip this directory as a whole.
            self.walk_entries = self.limits.max_walk_entries;
            self.notice(source, &relative, "walk_entry_limit");
            return;
        }
        self.walk_entries += entries.len() as u32;
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let path = entry.path();
            let relative = path.strip_prefix(root).unwrap();
            let Some(relative) = relative.to_str() else {
                self.notice(source, "<non-UTF8 path>", "unsupported_path");
                continue;
            };
            let file_type = match entry.file_type() {
                Ok(value) => value,
                Err(_) => {
                    self.notice(source, relative, "io_error");
                    continue;
                }
            };
            if file_type.is_symlink() {
                self.notice(source, relative, "symlink_excluded");
            } else if file_type.is_dir() {
                self.walk(source, root, &path, depth + 1);
            } else if file_type.is_file() && matches!(extension(relative).as_str(), "tex" | "json")
            {
                self.file(source, &path, relative, false);
            } else {
                self.excluded_loose_files += 1;
            }
        }
    }

    fn file(&mut self, source: &str, path: &Path, relative: &str, package: bool) {
        if self.records.len() >= self.limits.max_records as usize {
            self.notice(source, relative, "record_limit");
            return;
        }
        let mut record = Record {
            source: source.into(),
            path: relative.into(),
            size: 0,
            sha256: None,
            kind: if package {
                "pkg"
            } else if extension(relative) == "tex" {
                "tex"
            } else {
                "json"
            },
            status: "unread",
            failure_class: None,
            detail: None,
            format: None,
        };
        match self.read(path) {
            Ok(bytes) => {
                record.size = bytes.len() as u64;
                record.sha256 = Some(hash(&bytes));
                if package {
                    match PkgReader::parse(bytes) {
                        Ok(pkg) => {
                            record.status = "indexed";
                            record.format =
                                Some(json!({"version":pkg.version(), "members":pkg.len()}));
                            self.records.push(record);
                            let mut entries: Vec<_> = pkg.entries().iter().collect();
                            entries.sort_by(|a, b| a.filename.cmp(&b.filename));
                            for entry in entries {
                                if self.records.len() >= self.limits.max_records as usize {
                                    self.notice(source, "@package", "record_limit");
                                    break;
                                }
                                let mut member =
                                    self.inspect(source, &entry.filename, pkg.read_entry(entry));
                                // Offset is absolute in the hashed archive, not an extracted pathname.
                                let offset = pkg.read_entry(entry).as_ptr() as usize
                                    - pkg.raw_data().as_ptr() as usize;
                                member.format.get_or_insert_with(|| json!({}))["archive_offset"] =
                                    json!(offset);
                                self.records.push(member);
                            }
                            return;
                        }
                        Err(error) => {
                            record.status = "parse_error";
                            record.failure_class = Some("package_rejected".into());
                            record.detail = Some(error.to_string());
                            self.complete = false; // Member inventory unavailable.
                        }
                    }
                } else {
                    record = self.inspect(source, relative, &bytes);
                }
            }
            Err((class, size)) => {
                record.size = size;
                record.status = "unread";
                record.failure_class = Some(class.into());
                self.complete = false;
            }
        }
        self.records.push(record);
    }

    fn read(&mut self, path: &Path) -> std::result::Result<Vec<u8>, (&'static str, u64)> {
        let before = fs::symlink_metadata(path).map_err(|_| ("io_error", 0))?;
        if !before.is_file() {
            return Err(("non_regular_or_symlink", 0));
        }
        // O_NONBLOCK prevents a concurrent file-to-FIFO replacement from blocking the audit.
        let mut file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(path)
            .map_err(|_| ("io_error", before.len()))?;
        let metadata = file.metadata().map_err(|_| ("io_error", 0))?;
        let size = metadata.len();
        if (before.dev(), before.ino()) != (metadata.dev(), metadata.ino()) {
            return Err(("input_changed", size));
        }
        if !metadata.is_file() {
            return Err(("non_regular_or_symlink", size));
        }
        if size > self.limits.max_file_bytes {
            return Err(("file_byte_limit", size));
        }
        let remaining = self.limits.max_total_bytes.saturating_sub(self.input_bytes);
        if size > remaining {
            return Err(("total_byte_limit", size));
        }
        let mut bytes = Vec::new();
        let result = (&mut file).take(size + 1).read_to_end(&mut bytes);
        self.input_bytes += bytes.len() as u64;
        result.map_err(|_| ("io_error", size))?;
        let after = file.metadata().map_err(|_| ("io_error", size))?;
        if bytes.len() as u64 != size
            || metadata.modified().ok() != after.modified().ok()
            || (metadata.ctime(), metadata.ctime_nsec()) != (after.ctime(), after.ctime_nsec())
        {
            return Err(("input_changed", size));
        }
        Ok(bytes)
    }

    fn inspect(&mut self, source: &str, path: &str, bytes: &[u8]) -> Record {
        let ext = extension(path);
        let mut record = Record {
            source: source.into(),
            path: path.into(),
            size: bytes.len() as u64,
            sha256: Some(hash(bytes)),
            kind: "opaque",
            status: "hashed",
            failure_class: None,
            detail: None,
            format: Some(json!({"extension":ext})),
        };
        if ext == "tex" {
            record.kind = "tex";
            if bytes.starts_with(b"TEXV0005\0TEXI0001\0") && bytes.len() >= 26 {
                record.format = Some(json!({
                    "magic":"TEXV0005", "info_magic":"TEXI0001",
                    "format_id":u32::from_le_bytes(bytes[18..22].try_into().unwrap()),
                    "flags":u32::from_le_bytes(bytes[22..26].try_into().unwrap())
                }));
            }
            match TexTexture::parse_with_budget(bytes, self.limits.max_decoded_bytes) {
                Ok(texture) => {
                    record.format = Some(json!({
                        "container_version":texture.container_version, "pixel_format":format!("{:?}", texture.format),
                        "width":texture.width, "height":texture.height, "flags":texture.flags,
                        "storage_width":texture.texture_width, "storage_height":texture.texture_height,
                        "mips":texture.mipmaps.len(), "frames":texture.frames.len(),
                        "is_video":texture.is_video, "is_animated":texture.is_animated,
                        "decoded_bytes":texture.mipmaps.iter().map(|m| m.data.len() as u64).sum::<u64>()
                    }));
                    if texture.is_video {
                        record.status = "parsed_video_unverified";
                    } else if let Err(error) = texture.to_texture_image() {
                        record.status = "payload_error";
                        record.failure_class = Some(tex_failure(&error).into());
                        record.detail = Some(error.to_string());
                    } else {
                        record.status = "validated_2d";
                    }
                }
                Err(error) => {
                    record.status = "parse_error";
                    record.failure_class = Some(tex_failure(&error).into());
                    record.detail = Some(error.to_string());
                }
            }
        } else if ext == "json" {
            record.kind = "json";
            // Bound DOM and string/path processing independently of the larger TEX/PKG cap.
            if bytes.len() > 2 * 1024 * 1024 {
                record.status = "not_parsed";
                record.failure_class = Some("json_byte_limit".into());
                self.complete = false;
            } else {
                match serde_json::from_slice::<Value>(bytes) {
                    Ok(value) => {
                        record.status = "json_syntax_valid";
                        self.json_references(source, path, "", &value, None);
                    }
                    Err(error) => {
                        record.status = "parse_error";
                        record.failure_class = Some("invalid_json".into());
                        record.detail = Some(error.to_string());
                        self.complete = false;
                    }
                }
            }
        }
        record
    }

    fn json_references(
        &mut self,
        source: &str,
        path: &str,
        pointer: &str,
        value: &Value,
        key: Option<&str>,
    ) {
        if pointer.len() > 4096 {
            self.notice(source, path, "json_pointer_limit");
            return;
        }
        match value {
            Value::Object(values) => {
                for (name, value) in values {
                    if name.len() > 512 {
                        self.notice(source, path, "json_key_limit");
                        continue;
                    }
                    let escaped = name.replace('~', "~0").replace('/', "~1");
                    self.json_references(
                        source,
                        path,
                        &format!("{pointer}/{escaped}"),
                        value,
                        Some(name),
                    );
                }
            }
            Value::Array(values) => {
                for (index, value) in values.iter().enumerate() {
                    self.json_references(source, path, &format!("{pointer}/{index}"), value, key);
                }
            }
            Value::String(raw)
                if matches!(
                    key,
                    Some(
                        "image"
                            | "material"
                            | "model"
                            | "puppet"
                            | "particle"
                            | "sound"
                            | "video"
                            | "file"
                            | "textures"
                            | "dependencies"
                    )
                ) =>
            {
                if self.references.len() >= self.limits.max_references as usize {
                    self.notice(source, path, "reference_limit");
                    return;
                }
                if raw.len() > 512 {
                    self.notice(source, path, "reference_path_limit");
                    return;
                }
                let normalized = raw.replace('\\', "/");
                let candidate = safe_relative(&normalized).then(|| {
                    if key == Some("textures") {
                        resolve_texture_path(path, &normalized)
                    } else {
                        normalized
                    }
                });
                self.references.push(Reference {
                    source: source.into(),
                    path: path.into(),
                    pointer: pointer.into(),
                    value: raw.clone(),
                    status: if candidate.is_some() {
                        "not_found_in_scope"
                    } else {
                        "non_file_or_unsafe"
                    },
                    candidate,
                    matches: Vec::new(),
                });
            }
            _ => (),
        }
    }

    fn resolve_references(&mut self) {
        let index: BTreeSet<_> = self
            .records
            .iter()
            .map(|r| (r.source.clone(), r.path.clone()))
            .collect();
        let shared: Vec<_> = self
            .sources
            .iter()
            .filter(|s| s.kind == "shared")
            .map(|s| s.name.as_str())
            .collect();
        for reference in &mut self.references {
            let Some(candidate) = &reference.candidate else {
                continue;
            };
            // Loose trees may contain several projects. Find the nearest inventoried root.
            let mut directory = Path::new(&reference.path).parent().unwrap_or(Path::new(""));
            while !directory.as_os_str().is_empty() {
                if ["project.json", "scene.json"].iter().any(|name| {
                    index.contains(&(
                        reference.source.clone(),
                        directory.join(name).to_string_lossy().into_owned(),
                    ))
                }) {
                    break;
                }
                directory = directory.parent().unwrap_or(Path::new(""));
            }
            let local_path = directory.join(candidate).to_string_lossy().into_owned();
            let mut candidates = vec![(reference.source.as_str(), local_path.as_str())];
            for source in &shared {
                candidates.push((source, candidate));
            }
            for (source, path) in candidates {
                if index.contains(&(source.to_owned(), path.to_owned())) {
                    let target = json!({"source":source, "path":path});
                    if !reference.matches.contains(&target) {
                        reference.matches.push(target);
                    }
                }
            }
            if !reference.matches.is_empty() {
                reference.status = "candidate_found";
            }
        }
    }
}

fn extension(path: &str) -> String {
    Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
}

fn safe_relative(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with("_rt_")
        && !path.contains([':', '\0'])
        && Path::new(path)
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
}

fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn tex_failure(error: &TexError) -> &'static str {
    match error {
        TexError::InvalidData(message) if message.starts_with("Volume textures") => {
            "unsupported_volume"
        }
        TexError::InvalidData(message) if message.contains("multiple atlas images") => {
            "unsupported_multi_image"
        }
        TexError::InvalidData(message) if message.starts_with("Rotated or skewed") => {
            "unsupported_frame_transform"
        }
        TexError::InvalidData(message) if message.starts_with("Unsupported texture animation") => {
            "unsupported_version"
        }
        TexError::UnknownFormat(_) => "unsupported_format",
        TexError::UnsupportedContainerVersion(_) => "unsupported_version",
        TexError::MipmapTooLarge { .. } | TexError::CompressionRatioExceeded { .. } => {
            "texture_limit"
        }
        TexError::Lz4Error(_) => "decompression_error",
        TexError::InvalidMipmapDataSize { .. }
        | TexError::MissingMipmaps
        | TexError::InvalidDimensions { .. } => "invalid_pixel_payload",
        _ => "malformed_texture",
    }
}

#[cfg(test)]
#[path = "asset_audit/tests.rs"]
mod tests;
