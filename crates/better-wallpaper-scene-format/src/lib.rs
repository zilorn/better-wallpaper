//! Safe parser for Wallpaper Engine scene.pkg and .tex formats.
//!
//! This crate implements secure parsing and version-independent scene data for
//! Better Wallpaper. It has no GPU, media decoding, or unrestricted filesystem access.
//!
//! # Architecture
//!
//! - [`pkg::PkgReader`] — zero-copy parser for `.pkg` archives
//! - [`texture::TexTexture`] — parser for `.tex` texture containers
//! - [`scene::analyse_scene`] — scene.json metadata extraction and compatibility analysis
//! - [`ir::SceneGraph`] — validated, version-independent scene representation
//! - [`resource::ResourceManifest`] — deterministic package resource inventory

pub mod error;
pub mod ir;
pub mod pkg;
pub mod resource;
pub mod scene;
pub mod texture;

pub use error::{FormatError, PkgError, SceneParseError, TexError};
pub use ir::{
    SceneCamera, SceneGraph, SceneNode, SceneNodeKind, SceneTransform, UnsupportedFeature, Vec2,
    Vec3, parse_scene_graph,
};
pub use pkg::{PkgEntry, PkgReader};
pub use resource::{
    MAX_RESOURCE_CACHE_BYTES, ResourceCache, ResourceCacheError, ResourceCacheKey,
    ResourceDescriptor, ResourceKind, ResourceManifest, ResourceReference,
    ResourceValidationReport,
};
pub use scene::{
    CompatibilityLevel, CompatibilityReport, PropertyType, PropertyValue, SceneMetadata,
    SceneProject, UserProperty, analyse_scene, compute_compatibility, parse_project_properties,
};
pub use texture::{
    AnimationFrame, FreeImageFormat, Mipmap, TexFormat, TexTexture, TextureAlphaMode,
    TextureColorSpace, TextureImage, TextureMipLevel,
};
