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
pub mod material;
pub mod model;
pub mod pkg;
pub mod puppet;
pub mod resource;
pub mod scene;
pub mod texture;

pub use error::{FormatError, PkgError, SceneParseError, TexError};
pub use ir::{
    DynamicScaleKind, DynamicTextKind, FoliageSwayEffect, IrisEffect, PulseEffect, SceneCamera,
    SceneGraph, SceneNode, SceneNodeKind, SceneText, SceneTransform, ScrollEffect, ShakeEffect,
    ShineEffect, SpinEffect, UnsupportedFeature, Vec2, Vec3, WaterFlowEffect, WaterWaveEffect,
    parse_scene_graph, parse_scene_graph_with_properties,
};
pub use material::{BlendMode, MaterialDefinition, MaterialPass, parse_material_definition};
pub use model::{ModelDefinition, parse_model_definition};
pub use pkg::{PkgEntry, PkgReader};
pub use puppet::{
    PuppetAnimation, PuppetBone, PuppetError, PuppetModel, PuppetTransform, PuppetVertex,
};
pub use resource::{
    MAX_RESOURCE_CACHE_BYTES, MaterialManifest, MaterialResource, ModelManifest, ModelResource,
    ResourceCache, ResourceCacheError, ResourceCacheKey, ResourceDescriptor, ResourceKind,
    ResourceManifest, ResourceReference, ResourceValidationReport, resolve_texture_path,
};
pub use scene::{
    CompatibilityLevel, CompatibilityReport, PropertyType, PropertyValue, SceneMetadata,
    SceneProject, UserProperty, analyse_scene, compute_compatibility, parse_project_properties,
};
pub use texture::{
    AnimationFrame, FreeImageFormat, Mipmap, TexFormat, TexTexture, TextureAlphaMode,
    TextureColorSpace, TextureImage, TextureMipLevel,
};
