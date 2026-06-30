use std::{borrow::Cow, collections::HashSet, fmt, ops::Range, path::Path, sync::Arc};

use crate::error::PkgError;

// ── Safety limits ──────────────────────────────────────────────────────────

/// Maximum number of file entries in a .pkg
pub const MAX_PKG_FILES: u32 = 4_096;

/// Maximum filename length (bytes) per entry
pub const MAX_FILENAME_LENGTH: usize = 512;

/// Maximum total decompressed output across all entries (128 MiB)
pub const MAX_TOTAL_DATA_BYTES: u64 = 128 * 1024 * 1024;

/// Maximum single entry size (64 MiB)
pub const MAX_ENTRY_SIZE: u64 = 64 * 1024 * 1024;

// ── Data model ─────────────────────────────────────────────────────────────

/// A parsed .pkg archive. Holds a borrowed view of the backing data.
///
/// The entire archive is memory-mapped or loaded once; individual file entries
/// reference slices without copying. This is the "zero-copy index" approach
/// from the plan.
#[derive(Debug, Clone)]
pub struct PkgReader {
    /// Full archive data
    data: Arc<[u8]>,
    /// Validated package format version from the archive header.
    version: String,
    /// Parsed entries
    entries: Vec<PkgEntry>,
    /// Byte offset where file data starts (= end of the file index)
    base_offset: u64,
}

/// A single file entry inside a .pkg archive
#[derive(Debug, Clone)]
pub struct PkgEntry {
    /// Normalised filename (forward slashes, no `..`, no leading `/`)
    pub filename: String,
    /// File extension for quick filtering
    pub extension: String,
    /// Byte range within the archive data
    pub data_range: Range<u64>,
    /// Size of the file
    pub size: u64,
}

impl PkgEntry {
    /// Return the file extension (lowercased, without the dot), e.g. "json", "tex"
    pub fn ext(&self) -> &str {
        self.extension.as_str()
    }

    /// Whether the filename suggests a known scene asset type
    pub fn is_scene_asset(&self) -> bool {
        matches!(
            self.ext(),
            "json"
                | "tex"
                | "frag"
                | "vert"
                | "mdl"
                | "flac"
                | "wav"
                | "ogg"
                | "png"
                | "jpg"
                | "dds"
        )
    }
}

impl fmt::Display for PkgEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} ({} bytes, offset {})",
            self.filename, self.size, self.data_range.start
        )
    }
}

// ── Parsing ────────────────────────────────────────────────────────────────

impl PkgReader {
    /// Parse a .pkg archive from raw bytes.
    ///
    /// All bounds and safety checks are performed eagerly so the parsed
    /// `PkgReader` is safe to use without further validation.
    pub fn parse(data: impl Into<Arc<[u8]>>) -> Result<Self, PkgError> {
        let data: Arc<[u8]> = data.into();
        let archive_size = data.len() as u64;
        let mut offset: u64 = 0;

        // 1. Header: sized string (must start with "PKGV")
        let (header, header_len) = read_sized_string(&data, offset, archive_size)?;
        offset = offset
            .checked_add(header_len as u64)
            .ok_or(PkgError::OffsetOverflow {
                index: 0,
                name: "<header>".into(),
                offset: header_len as u64,
            })?;

        if !header.starts_with("PKGV") {
            return Err(PkgError::BadHeader(header));
        }

        // 2. File count
        let (file_count, _) = read_u32_le(&data, offset, archive_size)?;
        offset = offset.checked_add(4).ok_or(PkgError::OffsetOverflow {
            index: 0,
            name: "<file_count>".into(),
            offset: 4,
        })?;

        if file_count > MAX_PKG_FILES {
            return Err(PkgError::TooManyFiles {
                count: file_count,
                max: MAX_PKG_FILES,
            });
        }

        // 3. File entries
        let mut entries = Vec::with_capacity(file_count as usize);
        let mut seen_names = HashSet::new();

        for i in 0..file_count {
            let idx = i;

            // Filename (sized string)
            let (filename_raw, str_total) = read_sized_string(&data, offset, archive_size)?;

            if filename_raw.len() > MAX_FILENAME_LENGTH {
                return Err(PkgError::FilenameTooLong {
                    index: idx,
                    length: filename_raw.len(),
                    max: MAX_FILENAME_LENGTH,
                });
            }

            // Validate the filename
            let filename = validate_filename(&filename_raw, idx)?;

            offset = offset
                .checked_add(str_total as u64)
                .ok_or(PkgError::OffsetOverflow {
                    index: idx,
                    name: filename.clone(),
                    offset: str_total as u64,
                })?;

            // Entry offset (relative to base_offset)
            let (raw_offset, _) = read_u32_le(&data, offset, archive_size)?;
            offset = offset.checked_add(4).ok_or(PkgError::OffsetOverflow {
                index: idx,
                name: filename.clone(),
                offset: 4,
            })?;

            // Entry length
            let (raw_length, _) = read_u32_le(&data, offset, archive_size)?;
            offset = offset.checked_add(4).ok_or(PkgError::OffsetOverflow {
                index: idx,
                name: filename.clone(),
                offset: 4,
            })?;

            // Bounds checking with checked arithmetic
            let entry_offset = raw_offset as u64;
            let entry_length = raw_length as u64;

            if entry_length > MAX_ENTRY_SIZE {
                return Err(PkgError::EntryOutOfBounds {
                    index: idx,
                    name: filename.clone(),
                    offset: entry_offset,
                    length: entry_length,
                    archive_size,
                });
            }

            // Check duplicates
            if !seen_names.insert(filename.clone()) {
                return Err(PkgError::DuplicateFilename {
                    index: idx,
                    name: filename,
                });
            }

            entries.push(PkgEntry {
                extension: extension_from_filename(&filename),
                data_range: entry_offset..entry_offset + entry_length,
                size: entry_length,
                filename,
            });
        }

        // baseOffset = current offset (after all file entries)
        let base_offset = offset;

        // Validate all entry ranges against the archive
        let mut total = 0u64;
        for (i, entry) in entries.iter().enumerate() {
            let abs_start = base_offset.checked_add(entry.data_range.start).ok_or(
                PkgError::OffsetOverflow {
                    index: i as u32,
                    name: entry.filename.clone(),
                    offset: entry.data_range.start,
                },
            )?;
            let abs_end = abs_start
                .checked_add(entry.data_range.end - entry.data_range.start)
                .ok_or(PkgError::LengthOverflow {
                    index: i as u32,
                    name: entry.filename.clone(),
                    length: entry.data_range.end - entry.data_range.start,
                })?;

            if abs_end > archive_size {
                return Err(PkgError::EntryOutOfBounds {
                    index: i as u32,
                    name: entry.filename.clone(),
                    offset: abs_start,
                    length: entry.data_range.end - entry.data_range.start,
                    archive_size,
                });
            }

            total = total
                .checked_add(entry.size)
                .ok_or(PkgError::LengthOverflow {
                    index: i as u32,
                    name: entry.filename.clone(),
                    length: entry.size,
                })?;
        }

        if total > MAX_TOTAL_DATA_BYTES {
            return Err(PkgError::EntryOutOfBounds {
                index: entries.len() as u32,
                name: "<total>".into(),
                offset: 0,
                length: total,
                archive_size,
            });
        }

        let mut ranges: Vec<_> = entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| entry.size > 0)
            .map(|(index, entry)| (entry.data_range.clone(), index, entry.filename.clone()))
            .collect();
        ranges.sort_unstable_by_key(|(range, _, _)| range.start);
        for pair in ranges.windows(2) {
            let (previous, _, previous_name) = &pair[0];
            let (current, index, current_name) = &pair[1];
            if current.start < previous.end {
                return Err(PkgError::OverlappingEntries {
                    index: *index as u32,
                    name: current_name.clone(),
                    previous_name: previous_name.clone(),
                });
            }
        }

        Ok(Self {
            data,
            version: header,
            entries,
            base_offset,
        })
    }

    /// Number of entries in the archive
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Iterate over all entries
    pub fn entries(&self) -> &[PkgEntry] {
        &self.entries
    }

    /// Find an entry by exact filename
    pub fn find(&self, name: &str) -> Option<&PkgEntry> {
        self.entries.iter().find(|e| e.filename == name)
    }

    /// Find entries by extension
    pub fn find_by_ext(&self, ext: &str) -> Vec<&PkgEntry> {
        let ext = ext.trim_start_matches('.');
        self.entries.iter().filter(|e| e.extension == ext).collect()
    }

    /// Read the raw bytes for a single entry (zero-copy slice into archive)
    pub fn read_entry(&self, entry: &PkgEntry) -> &[u8] {
        let start = (self.base_offset + entry.data_range.start) as usize;
        let end = (self.base_offset + entry.data_range.end) as usize;
        &self.data[start..end]
    }

    /// Read an entry's content as a UTF-8 string (for JSON files)
    pub fn read_entry_string(&self, entry: &PkgEntry) -> Result<Cow<'_, str>, PkgError> {
        let bytes = self.read_entry(entry);
        Ok(String::from_utf8_lossy(bytes))
    }

    /// Package version string (the PKGV value)
    pub fn version(&self) -> &str {
        &self.version
    }

    /// The backing data — useful for test assertions
    pub fn raw_data(&self) -> &[u8] {
        &self.data
    }
}

// ── Binary reading helpers ─────────────────────────────────────────────────

/// Read a "sized string": uint32 LE length + that many bytes.
/// Returns (string, total_bytes_consumed).
fn read_sized_string(
    data: &[u8],
    offset: u64,
    archive_size: u64,
) -> Result<(String, usize), PkgError> {
    let (len, _) = read_u32_le(data, offset, archive_size)?;
    let len_usize: usize = len as usize;
    let total = 4usize
        .checked_add(len_usize)
        .ok_or(PkgError::FilenameTooLong {
            index: 0,
            length: len_usize,
            max: MAX_FILENAME_LENGTH,
        })?;

    let start = offset.checked_add(4).ok_or(PkgError::OffsetOverflow {
        index: 0,
        name: "<sized_string_len>".into(),
        offset: 4,
    })?;

    if start as usize + len_usize > data.len() {
        return Err(PkgError::UnexpectedEof {
            offset: start,
            needed: len_usize,
            available: data.len().saturating_sub(start as usize),
        });
    }

    let s = String::from_utf8_lossy(&data[start as usize..][..len_usize]).into_owned();
    Ok((s, total))
}

/// Read a uint32 little-endian from data at offset.
fn read_u32_le(data: &[u8], offset: u64, _archive_size: u64) -> Result<(u32, usize), PkgError> {
    let offset_usize: usize = offset as usize;
    if offset_usize + 4 > data.len() {
        return Err(PkgError::UnexpectedEof {
            offset,
            needed: 4,
            available: data.len().saturating_sub(offset_usize),
        });
    }
    let b = &data[offset_usize..offset_usize + 4];
    let v = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
    Ok((v, 4))
}

// ── Filename validation ────────────────────────────────────────────────────

/// Validate and normalise a filename from a .pkg entry.
///
/// Rejects:
/// - Absolute paths (starts with `/`)
/// - Parent directory traversal (`..`)
/// - NUL bytes
/// - Empty paths
/// - Windows drive letters (`C:\`, etc.)
fn validate_filename(raw: &str, index: u32) -> Result<String, PkgError> {
    if raw.is_empty() {
        return Err(PkgError::InvalidFilename {
            index,
            reason: "empty filename".into(),
        });
    }

    if raw.contains('\0') {
        return Err(PkgError::InvalidFilename {
            index,
            reason: "contains NUL byte".into(),
        });
    }

    // Normalise to forward slashes
    let normalised = raw.replace('\\', "/");

    // Check for absolute paths: leading `/` or windows drive letter
    if normalised.starts_with('/') {
        return Err(PkgError::InvalidFilename {
            index,
            reason: format!("absolute path not allowed: {raw:?}"),
        });
    }

    // Check for drive letters (e.g., C:)
    if normalised.len() >= 2
        && normalised.as_bytes()[1] == b':'
        && normalised.as_bytes()[0].is_ascii_alphabetic()
    {
        return Err(PkgError::InvalidFilename {
            index,
            reason: format!("Windows drive path not allowed: {raw:?}"),
        });
    }

    // Check for parent traversal
    for component in normalised.split('/') {
        if component == ".." {
            return Err(PkgError::InvalidFilename {
                index,
                reason: format!("path contains '..': {raw:?}"),
            });
        }
    }

    Ok(normalised)
}

fn extension_from_filename(name: &str) -> String {
    Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default()
}

// ── Serde for JSON output ──────────────────────────────────────────────────

#[derive(serde::Serialize)]
pub struct PkgInspectOutput {
    pub version: String,
    pub file_count: u32,
    pub total_size: u64,
    pub entries: Vec<PkgEntryOutput>,
}

#[derive(serde::Serialize)]
pub struct PkgEntryOutput {
    pub index: usize,
    pub filename: String,
    pub offset: u64,
    pub length: u64,
    pub extension: String,
    pub file_type_hint: String,
}

impl PkgReader {
    /// Produce a serialisable inspect output (for `scene-inspect --json`)
    pub fn inspect(&self) -> PkgInspectOutput {
        PkgInspectOutput {
            version: self.version().to_string(),
            file_count: self.entries.len() as u32,
            total_size: self.data.len() as u64,
            entries: self
                .entries
                .iter()
                .enumerate()
                .map(|(i, e)| PkgEntryOutput {
                    index: i,
                    filename: e.filename.clone(),
                    offset: e.data_range.start,
                    length: e.size,
                    extension: e.extension.clone(),
                    file_type_hint: file_type_hint(&e.extension, self.read_entry(e)),
                })
                .collect(),
        }
    }
}

fn file_type_hint(ext: &str, data: &[u8]) -> String {
    if data.len() < 4 {
        return ext.to_string();
    }
    let magic = &data[..4];
    match magic {
        b"\x89PNG" => "PNG image".into(),
        b"\xff\xd8" => "JPEG image".into(),
        b"DDS " => "DDS texture".into(),
        b"OggS" => "OGG audio".into(),
        b"RIFF" => "WAV/RIFF".into(),
        b"\x1aE\xdf\xa3" => "WebM/Matroska".into(),
        b"TEXV" => "Wallpaper Engine texture".into(),
        _ => {
            if data[0] == b'{' || data[0] == b'[' {
                "JSON".into()
            } else if ext == "frag" || ext == "vert" {
                "GLSL shader".into()
            } else if ext == "mdl" {
                "Spriter model".into()
            } else {
                ext.to_string()
            }
        }
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal valid .pkg with a single file entry.
    fn build_minimal_pkg(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut buf = Vec::new();

        // Header: sized string "PKGV0018"
        let header = b"PKGV0018";
        buf.extend((header.len() as u32).to_le_bytes());
        buf.extend(header);

        // File count
        buf.extend((files.len() as u32).to_le_bytes());

        // Track positions for fixup
        struct Fixup {
            offset_pos: usize,
            length_pos: usize,
            data_len: u32,
        }
        let mut fixups = Vec::new();

        for (name, data) in files {
            let name_bytes = name.as_bytes();
            buf.extend((name_bytes.len() as u32).to_le_bytes());
            buf.extend(name_bytes);

            let offset_pos = buf.len();
            buf.extend(0u32.to_le_bytes()); // placeholder offset
            let length_pos = buf.len();
            buf.extend(0u32.to_le_bytes()); // placeholder length

            fixups.push(Fixup {
                offset_pos,
                length_pos,
                data_len: data.len() as u32,
            });
        }

        let _base_offset = buf.len() as u32;

        // Fix up offsets and lengths
        let mut data_offset = 0u32;
        for fixup in &fixups {
            buf[fixup.offset_pos..fixup.offset_pos + 4].copy_from_slice(&data_offset.to_le_bytes());
            buf[fixup.length_pos..fixup.length_pos + 4]
                .copy_from_slice(&fixup.data_len.to_le_bytes());
            data_offset += fixup.data_len;
        }

        // Write file data
        for (_name, data) in files {
            buf.extend_from_slice(data);
        }

        buf
    }

    #[test]
    fn parse_minimal_pkg() {
        let data = build_minimal_pkg(&[("scene.json", br#"{"camera":{}}"#)]);
        let pkg = PkgReader::parse(data).unwrap();
        assert_eq!(pkg.len(), 1);
        assert_eq!(pkg.entries()[0].filename, "scene.json");
    }

    #[test]
    fn reject_absolute_path() {
        let data = build_minimal_pkg(&[("/etc/passwd", b"data")]);
        let err = PkgReader::parse(data).unwrap_err();
        assert!(matches!(err, PkgError::InvalidFilename { .. }));
    }

    #[test]
    fn reject_parent_traversal() {
        let data = build_minimal_pkg(&[("../../etc/passwd", b"data")]);
        let err = PkgReader::parse(data).unwrap_err();
        assert!(matches!(err, PkgError::InvalidFilename { .. }));
    }

    #[test]
    fn reject_duplicate_filenames() {
        let data = build_minimal_pkg(&[("scene.json", b"data1"), ("scene.json", b"data2")]);
        let err = PkgReader::parse(data).unwrap_err();
        assert!(matches!(err, PkgError::DuplicateFilename { .. }));
    }

    #[test]
    fn reject_overlapping_entries() {
        let mut data = build_minimal_pkg(&[("a.json", b"abcd"), ("b.json", b"efgh")]);
        let first_entry_size = 4 + "a.json".len() + 8;
        let second_offset_position = 4 + 8 + 4 + first_entry_size + 4 + "b.json".len();
        data[second_offset_position..second_offset_position + 4]
            .copy_from_slice(&2_u32.to_le_bytes());

        assert!(matches!(
            PkgReader::parse(data),
            Err(PkgError::OverlappingEntries { .. })
        ));
    }

    #[test]
    fn reports_full_package_version() {
        let pkg = PkgReader::parse(build_minimal_pkg(&[])).unwrap();
        assert_eq!(pkg.version(), "PKGV0018");
    }

    #[test]
    fn reject_empty_filename() {
        let data = build_minimal_pkg(&[("", b"data")]);
        let err = PkgReader::parse(data).unwrap_err();
        assert!(matches!(err, PkgError::InvalidFilename { .. }));
    }

    #[test]
    fn reject_too_many_files() {
        let mut buf = Vec::new();
        let header = b"PKGV0018";
        buf.extend((header.len() as u32).to_le_bytes());
        buf.extend(header);
        buf.extend((MAX_PKG_FILES + 1).to_le_bytes());
        let pkg = PkgReader::parse(buf);
        assert!(pkg.is_err());
    }

    #[test]
    fn find_entry_by_name() {
        let data = build_minimal_pkg(&[("scene.json", br#"{"camera":{}}"#)]);
        let pkg = PkgReader::parse(data).unwrap();
        assert!(pkg.find("scene.json").is_some());
        assert!(pkg.find("nonexistent.json").is_none());
    }

    #[test]
    fn read_entry_content() {
        let content = br#"{"camera":{}}"#;
        let data = build_minimal_pkg(&[("scene.json", content)]);
        let pkg = PkgReader::parse(data).unwrap();
        let entry = pkg.find("scene.json").unwrap();
        let bytes = pkg.read_entry(entry);
        assert_eq!(bytes, content);
        let s = pkg.read_entry_string(entry).unwrap();
        assert_eq!(s.as_ref(), r#"{"camera":{}}"#);
    }

    #[test]
    fn reject_truncated_data() {
        let data = vec![0u8; 3]; // too short for any header
        let err = PkgReader::parse(data);
        assert!(err.is_err());
    }

    #[test]
    fn windows_drive_path_rejected() {
        let data = build_minimal_pkg(&[("C:\\foo\\bar.tex", b"data")]);
        let err = PkgReader::parse(data).unwrap_err();
        assert!(matches!(err, PkgError::InvalidFilename { .. }));
    }

    #[test]
    fn backslash_normalised_to_forward_slash() {
        let data = build_minimal_pkg(&[("materials\\foo.tex", b"data")]);
        let pkg = PkgReader::parse(data).unwrap();
        assert_eq!(pkg.entries()[0].filename, "materials/foo.tex");
    }

    #[test]
    fn entry_out_of_bounds_rejected() {
        // Build a pkg where offset points past the archive data
        let mut buf = Vec::new();
        let header = b"PKGV0018";
        buf.extend((header.len() as u32).to_le_bytes());
        buf.extend(header);
        buf.extend(1u32.to_le_bytes()); // 1 file
        let name = b"scene.json";
        buf.extend((name.len() as u32).to_le_bytes());
        buf.extend(name);
        buf.extend(999_999u32.to_le_bytes()); // offset way past end
        buf.extend(100u32.to_le_bytes()); // length
        let err = PkgReader::parse(buf).unwrap_err();
        assert!(matches!(err, PkgError::EntryOutOfBounds { .. }));
    }
}
