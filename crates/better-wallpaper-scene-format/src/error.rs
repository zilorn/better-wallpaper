use thiserror::Error;

/// Errors from .pkg archive parsing
#[derive(Debug, Error)]
pub enum PkgError {
    #[error("Invalid package header: expected 'PKGV' prefix, got {0:?}")]
    BadHeader(String),

    #[error("Unexpected end of data at offset {offset}: needed {needed} bytes, got {available}")]
    UnexpectedEof {
        offset: u64,
        needed: usize,
        available: usize,
    },

    #[error("File count {count} exceeds maximum allowed ({max})")]
    TooManyFiles { count: u32, max: u32 },

    #[error("Entry {index} filename ({length} bytes) exceeds maximum ({max})")]
    FilenameTooLong {
        index: u32,
        length: usize,
        max: usize,
    },

    #[error("Entry {index} has invalid filename: {reason}")]
    InvalidFilename { index: u32, reason: String },

    #[error(
        "Entry {index} ({name}): offset {offset} + length {length} overflows archive bounds (size {archive_size})"
    )]
    EntryOutOfBounds {
        index: u32,
        name: String,
        offset: u64,
        length: u64,
        archive_size: u64,
    },

    #[error("Entry {index} ({name}): offset {offset} overflows")]
    OffsetOverflow {
        index: u32,
        name: String,
        offset: u64,
    },

    #[error("Entry {index} ({name}): length {length} overflows")]
    LengthOverflow {
        index: u32,
        name: String,
        length: u64,
    },

    #[error("Duplicate filename: {name} at entry {index}")]
    DuplicateFilename { index: u32, name: String },

    #[error("entry {index} ({name}) overlaps entry {previous_name}")]
    OverlappingEntries {
        index: u32,
        name: String,
        previous_name: String,
    },

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

/// Errors from .tex texture parsing
#[derive(Debug, Error)]
pub enum TexError {
    #[error("Bad texture magic: expected TEXV0005, got {0:?}")]
    BadMagic(String),

    #[error("Bad info block magic: expected TEXI0001, got {0:?}")]
    BadInfoBlock(String),

    #[error("Bad container block magic: got {0:?}")]
    BadContainerBlock(String),

    #[error("Unknown texture format value {0}")]
    UnknownFormat(u32),

    #[error("LZ4 decompression failed: {0}")]
    Lz4Error(String),

    #[error(
        "Texture compression ratio exceeds safety limit: {uncompressed} uncompressed bytes from {compressed} compressed bytes"
    )]
    CompressionRatioExceeded { compressed: u64, uncompressed: u64 },

    #[error("Mipmap data size {size} exceeds maximum ({max})")]
    MipmapTooLarge { size: u64, max: u64 },

    #[error("Invalid mipmap data size for level {level}: expected {expected} bytes, got {actual}")]
    InvalidMipmapDataSize {
        level: usize,
        expected: u64,
        actual: usize,
    },

    #[error("Texture has no mipmap data")]
    MissingMipmaps,

    #[error("Invalid texture dimensions at mipmap level {level}: {width}x{height}")]
    InvalidDimensions {
        level: usize,
        width: u32,
        height: u32,
    },

    #[error("Unsupported texture container version {0}")]
    UnsupportedContainerVersion(u32),

    #[error("Unexpected end of texture data")]
    UnexpectedEof,

    #[error("Invalid texture data: {0}")]
    InvalidData(String),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

/// Errors from scene.json parsing
#[derive(Debug, Error)]
pub enum SceneParseError {
    #[error("Missing required field: {0}")]
    MissingField(String),

    #[error("Invalid value for {field}: {detail}")]
    InvalidValue { field: String, detail: String },

    #[error("JSON parse error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Invalid path in scene file: {0}")]
    InvalidPath(String),

    #[error("Unsupported feature: {0}")]
    UnsupportedFeature(String),
}

/// Aggregate error type for this crate
#[derive(Debug, Error)]
pub enum FormatError {
    #[error("Package error: {0}")]
    Pkg(#[from] PkgError),

    #[error("Texture error: {0}")]
    Tex(#[from] TexError),

    #[error("Scene parse error: {0}")]
    Scene(#[from] SceneParseError),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}
