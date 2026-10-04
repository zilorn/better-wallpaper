use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::{
        Arc, RwLock,
        atomic::{AtomicU32, Ordering},
    },
};

use better_wallpaper_scene_format::{
    BlendMode, DynamicScaleKind, DynamicTextKind, MaterialManifest, ModelManifest, PkgReader,
    PuppetModel, ScalarAnimation, SceneGraph, SceneNodeKind, SceneText, TexFormat, TexTexture,
    TextureAlphaMode, TextureColorSpace, TextureImage, TextureMipLevel, resolve_texture_path,
};
use thiserror::Error;
use tracing::{debug, warn};

#[derive(Debug, Clone)]
pub struct SceneFontRender {
    pub font: Arc<fontdue::Font>,
    pub source: String,
    pub fallback: Option<Arc<fontdue::Font>>,
    pub fallback_source: Option<String>,
    pub pixel_size: f32,
    pub color: better_wallpaper_scene_format::Vec3,
    pub dynamic: Option<DynamicTextKind>,
}

/// Column-major affine 2D matrix. Points are multiplied as `matrix * [x, y, 1]`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Mat3(pub [f32; 9]);

impl Mat3 {
    pub const IDENTITY: Self = Self([1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]);

    pub fn transform_point(self, point: [f32; 2]) -> [f32; 2] {
        let m = self.0;
        [
            m[0] * point[0] + m[3] * point[1] + m[6],
            m[1] * point[0] + m[4] * point[1] + m[7],
        ]
    }

    fn translation(x: f32, y: f32) -> Self {
        Self([1.0, 0.0, 0.0, 0.0, 1.0, 0.0, x, y, 1.0])
    }

    fn scale(x: f32, y: f32) -> Self {
        Self([x, 0.0, 0.0, 0.0, y, 0.0, 0.0, 0.0, 1.0])
    }

    fn rotation(radians: f32) -> Self {
        let (sin, cos) = radians.sin_cos();
        Self([cos, sin, 0.0, -sin, cos, 0.0, 0.0, 0.0, 1.0])
    }
}

impl std::ops::Mul for Mat3 {
    type Output = Self;

    fn mul(self, rhs: Self) -> Self::Output {
        let mut out = [0.0; 9];
        for column in 0..3 {
            for row in 0..3 {
                out[column * 3 + row] = (0..3)
                    .map(|index| self.0[index * 3 + row] * rhs.0[column * 3 + index])
                    .sum();
            }
        }
        Self(out)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Scene2dOptions {
    pub viewport_width: u32,
    pub viewport_height: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Scene2dQuad {
    pub node_id: String,
    pub resource: String,
    pub text: Option<SceneText>,
    pub video: bool,
    pub parallax_depth: [f32; 2],
    pub dynamic_scale: Option<DynamicScaleKind>,
    pub audio_response: Option<SceneAudioResponse>,
    pub layout_transform: Mat3,
    /// Quad corners in NDC, ordered top-left, top-right, bottom-right, bottom-left.
    pub vertices: [[f32; 2]; 4],
    pub opacity: f32,
    pub opacity_animation: Option<ScalarAnimation>,
    pub texture_override: Option<String>,
    pub backdrop_fade: bool,
    pub scroll: Option<better_wallpaper_scene_format::ScrollEffect>,
    pub water_waves: Vec<better_wallpaper_scene_format::WaterWaveEffect>,
    pub water_flow: Option<better_wallpaper_scene_format::WaterFlowEffect>,
    pub shakes: Vec<better_wallpaper_scene_format::ShakeEffect>,
    pub pulses: Vec<better_wallpaper_scene_format::PulseEffect>,
    pub spin: Option<better_wallpaper_scene_format::SpinEffect>,
    pub iris: Option<better_wallpaper_scene_format::IrisEffect>,
    pub foliage_sway: Vec<better_wallpaper_scene_format::FoliageSwayEffect>,
    pub shine: Option<better_wallpaper_scene_format::ShineEffect>,
}

/// Smooth pointer motion on the scene clock: equal paused timestamps produce
/// equal offsets, and leaving a surface transitions back to the centered view.
#[derive(Debug, Default)]
pub struct SceneParallaxState {
    position: [f32; 2],
    previous_time: Option<f64>,
}

impl SceneParallaxState {
    pub fn update(
        &mut self,
        settings: Option<better_wallpaper_scene_format::SceneParallax>,
        elapsed_seconds: f64,
        pointer: Option<[f32; 2]>,
    ) -> [f32; 2] {
        let elapsed_seconds = if elapsed_seconds.is_finite() {
            elapsed_seconds.max(0.0)
        } else {
            0.0
        };
        let previous = self.previous_time.replace(elapsed_seconds);
        let Some(settings) = settings else {
            self.position = [0.0; 2];
            return self.position;
        };
        if previous.is_some_and(|time| elapsed_seconds < time) {
            self.position = [0.0; 2];
        }
        let delta = previous.map_or(0.0, |time| (elapsed_seconds - time).max(0.0));
        let target = pointer
            .filter(|p| p.iter().all(|v| v.is_finite()))
            .unwrap_or([0.0; 2])
            .map(|v| v.clamp(-1.0, 1.0));
        let alpha = if previous.is_some() && delta == 0.0 {
            0.0
        } else if settings.delay <= f32::EPSILON {
            1.0
        } else {
            1.0 - (-delta / f64::from(settings.delay)).exp() as f32
        };
        for (position, target) in self.position.iter_mut().zip(target) {
            *position += (target - *position) * alpha;
        }
        self.position
    }
}

pub fn scene_parallax_offset(
    settings: Option<better_wallpaper_scene_format::SceneParallax>,
    pointer: [f32; 2],
    depth: [f32; 2],
) -> [f32; 2] {
    settings.map_or([0.0; 2], |settings| {
        std::array::from_fn(|axis| {
            (-pointer[axis] * settings.amount * settings.mouse_influence * depth[axis])
                .clamp(-2.0, 2.0)
        })
    })
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SceneAudioResponse {
    pub bin: usize,
    pub height_scale: f32,
}

#[derive(Debug)]
pub struct SceneAudioSpectrum {
    bins: [AtomicU32; 64],
}

impl Default for SceneAudioSpectrum {
    fn default() -> Self {
        Self {
            bins: std::array::from_fn(|_| AtomicU32::new(0.0_f32.to_bits())),
        }
    }
}

impl SceneAudioSpectrum {
    pub fn set(&self, values: &[f32; 64]) {
        for (bin, value) in self.bins.iter().zip(values) {
            bin.store(value.clamp(0.0, 1.0).to_bits(), Ordering::Release);
        }
    }

    pub fn get(&self, index: usize) -> f32 {
        self.bins
            .get(index)
            .map_or(0.0, |value| f32::from_bits(value.load(Ordering::Acquire)))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Scene2dPlan {
    pub quads: Vec<Scene2dQuad>,
    pub skipped_nodes: usize,
    pub projection_size: [f32; 2],
    pub layout_viewport: [u32; 2],
    pub camera_zoom: f32,
    pub camera_zoom_animation: Option<ScalarAnimation>,
    pub parallax: Option<better_wallpaper_scene_format::SceneParallax>,
}

/// A draw whose model/material/texture dependency chain has been fully resolved.
/// This is the shared CPU-side submission boundary used by niri and Plasma.
#[derive(Debug, Clone)]
pub struct Scene2dDraw {
    pub quad: Scene2dQuad,
    pub blend_mode: BlendMode,
    pub texture_path: String,
    pub texture: TextureImage,
    pub video: Option<SceneVideoTexture>,
    pub text_render: Option<SceneFontRender>,
    /// Logical image bounds within a potentially padded GPU texture.
    pub uv: [f32; 4],
    pub animation: Option<SpriteAnimation>,
    pub mesh: Option<Scene2dMesh>,
    pub water_wave_masks: Vec<Option<Scene2dEffectTexture>>,
    pub water_wave_normals: Vec<Option<Scene2dEffectTexture>>,
    pub water_flow_mask: Option<Scene2dEffectTexture>,
    pub water_flow_phase: Option<Scene2dEffectTexture>,
    pub iris_mask: Option<Scene2dEffectTexture>,
    pub foliage_masks: Vec<Option<Scene2dEffectTexture>>,
    pub shine_mask: Option<Scene2dEffectTexture>,
    pub shake_maps: Vec<Option<Scene2dEffectTexture>>,
    pub pulse_masks: Vec<Option<Scene2dEffectTexture>>,
}

/// Immutable encoded media plus a CPU frame shared by every output/context.
#[derive(Debug, Clone)]
pub struct SceneVideoTexture {
    pub encoded: Arc<[u8]>,
    pub frame: Arc<RwLock<Option<SceneVideoFrame>>>,
}

#[derive(Debug, Clone)]
pub struct SceneVideoFrame {
    pub rgba: Arc<[u8]>,
    pub width: u32,
    pub height: u32,
    pub generation: u64,
}

#[derive(Debug, Clone)]
pub struct Scene2dMesh {
    /// NDC positions for every authored animation frame.
    pub frames: Vec<Vec<[f32; 2]>>,
    /// Logical, unpadded texture coordinates shared by all frames.
    pub uv: Vec<[f32; 2]>,
    pub indices: Vec<u16>,
    pub fps: f32,
}

impl Scene2dMesh {
    pub fn positions_at(&self, elapsed_seconds: f64) -> &[[f32; 2]] {
        let index = if self.frames.len() <= 1 || self.fps <= 0.0 {
            0
        } else {
            ((elapsed_seconds * f64::from(self.fps)).floor() as usize) % self.frames.len()
        };
        self.frames.get(index).map_or(&[], Vec::as_slice)
    }
}

#[derive(Debug, Clone)]
pub struct Scene2dEffectTexture {
    pub path: String,
    pub texture: TextureImage,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SpriteFrame {
    /// Normalized top-left and bottom-right texture coordinates.
    pub uv: [f32; 4],
    pub duration_seconds: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SpriteAnimation {
    pub frames: Vec<SpriteFrame>,
    pub duration_seconds: f32,
}

impl SpriteAnimation {
    pub fn uv_at(&self, elapsed_seconds: f64) -> [f32; 4] {
        if self.frames.is_empty() || self.duration_seconds <= 0.0 {
            return [0.0, 0.0, 1.0, 1.0];
        }
        let mut cursor = elapsed_seconds.rem_euclid(f64::from(self.duration_seconds)) as f32;
        for frame in &self.frames {
            if cursor < frame.duration_seconds {
                return frame.uv;
            }
            cursor -= frame.duration_seconds;
        }
        self.frames
            .last()
            .map_or([0.0, 0.0, 1.0, 1.0], |frame| frame.uv)
    }
}

#[derive(Debug, Clone)]
pub struct Scene2dAssets {
    pub draws: Vec<Scene2dDraw>,
    pub skipped_nodes: usize,
    /// Source canvas used to rebuild the cover transform for the real output.
    pub projection_size: [f32; 2],
    /// Viewport used when the backend-independent quad plan was built.
    pub layout_viewport: [u32; 2],
    pub camera_zoom: f32,
    pub camera_zoom_animation: Option<ScalarAnimation>,
    pub parallax: Option<better_wallpaper_scene_format::SceneParallax>,
    pub audio_spectrum: Option<Arc<SceneAudioSpectrum>>,
}

#[derive(Debug, Error, PartialEq)]
pub enum Scene2dError {
    #[error("scene viewport must be non-zero")]
    EmptyViewport,
    #[error("scene projection must contain finite positive dimensions")]
    InvalidProjection,
    #[error("duplicate scene node id: {0}")]
    DuplicateNodeId(String),
    #[error("scene node {node} references missing parent {parent}")]
    MissingParent { node: String, parent: String },
    #[error("scene node parent cycle contains: {0}")]
    ParentCycle(String),
    #[error("scene node {0} has an invalid opacity")]
    InvalidOpacity(String),
    #[error("scene image node {node} references unknown model {model}")]
    MissingModel { node: String, model: String },
    #[error("scene model {model} references unknown material {material}")]
    MissingMaterial { model: String, material: String },
    #[error("scene material {0} has no render pass")]
    MissingMaterialPass(String),
    #[error("scene material {0} uses an unsupported blend mode")]
    UnsupportedBlendMode(String),
    #[error("scene material {0} does not reference exactly one texture")]
    UnsupportedTextureCount(String),
    #[error("scene texture is missing from package: {0}")]
    MissingTexture(String),
    #[error("scene texture {path} is invalid: {detail}")]
    InvalidTexture { path: String, detail: String },
    #[error("scene puppet model is missing from package: {0}")]
    MissingPuppet(String),
    #[error("scene puppet model {path} is invalid: {detail}")]
    InvalidPuppet { path: String, detail: String },
    #[error("scene asset manifest is invalid: {0}")]
    InvalidAssetManifest(String),
    #[error("scene font {font} could not be loaded: {detail}")]
    InvalidFont { font: String, detail: String },
}

/// Resolves a validated draw plan into immutable texture payloads without
/// creating GPU objects. Desktop backends can upload this data in their own GL
/// context while retaining identical scene semantics.
pub fn resolve_scene_2d_assets(
    package: &PkgReader,
    plan: Scene2dPlan,
) -> Result<Scene2dAssets, Scene2dError> {
    let models = ModelManifest::from_package(package)
        .map_err(|error| Scene2dError::InvalidAssetManifest(error.to_string()))?;
    let materials = MaterialManifest::from_package(package, &models)
        .map_err(|error| Scene2dError::InvalidAssetManifest(error.to_string()))?;
    let model_by_path = models
        .models
        .iter()
        .map(|model| (model.path.as_str(), model.clone()))
        .collect::<HashMap<_, _>>();
    let material_by_path = materials
        .materials
        .iter()
        .map(|material| (material.path.as_str(), material.clone()))
        .collect::<HashMap<_, _>>();

    let mut videos = HashMap::<String, SceneVideoTexture>::new();
    let mut draws = Vec::with_capacity(plan.quads.len());
    let mut skipped_nodes = plan.skipped_nodes;
    let projection_size = plan.projection_size;
    let layout_viewport = plan.layout_viewport;
    // Text point sizes are authored for the output/design viewport, while the
    // node transform below operates in project-canvas coordinates. Convert the
    // requested size back into canvas pixels before projection so a 2160p
    // scene does not make text half-sized on a 1080p output.
    let text_resolution_scale = (projection_size[1] / layout_viewport[1] as f32).clamp(0.25, 8.0);
    for quad in plan.quads {
        let resolved = (|| {
            if quad.video {
                let entry = package
                    .find(&quad.resource)
                    .ok_or_else(|| Scene2dError::MissingTexture(quad.resource.clone()))?;
                let video = videos
                    .entry(quad.resource.clone())
                    .or_insert_with(|| SceneVideoTexture {
                        encoded: Arc::from(package.read_entry(entry)),
                        frame: Arc::new(RwLock::new(None)),
                    })
                    .clone();
                let mut draw = procedural_draw(quad, solid_color_texture([0, 0, 0, 0]), None);
                draw.video = Some(video);
                return Ok(draw);
            }
            if let Some(text) = &quad.text {
                let initial = match text.dynamic {
                    Some(DynamicTextKind::Clock) => "00:00",
                    Some(DynamicTextKind::Date) => "0000 | 00 | 00 | 星期日",
                    Some(DynamicTextKind::Media) => "",
                    None => &text.value,
                };
                let font_render =
                    resolve_scene_font(package, text, initial, text_resolution_scale)?;
                let texture = font_text_texture(initial, &font_render);
                let mut text_quad = quad.clone();
                let level = texture
                    .levels
                    .first()
                    .expect("font renderer always creates a base level");
                text_quad.vertices = quad_vertices(
                    text_quad.layout_transform,
                    level.width as f32,
                    level.height as f32,
                );
                debug!(
                    node = %quad.node_id,
                    dynamic = ?text.dynamic,
                    requested_font = %text.font,
                    selected_font = %font_render.source,
                    fallback_font = ?font_render.fallback_source,
                    point_size = text.point_size,
                    resolution_scale = text_resolution_scale,
                    raster_pixel_size = font_render.pixel_size,
                    rendered_width = level.width,
                    rendered_height = level.height,
                    "Prepared scene text with a font file renderer"
                );
                return Ok(procedural_draw(text_quad, texture, Some(font_render)));
            }
            if quad.resource == "models/util/solidlayer.json" {
                debug!(node = %quad.node_id, "Prepared built-in solid scene layer");
                return Ok(procedural_draw(
                    quad.clone(),
                    solid_color_texture([255, 255, 255, 255]),
                    None,
                ));
            }
            if quad.backdrop_fade {
                debug!(node = %quad.node_id, "Prepared animated black scene entrance overlay");
                return Ok(procedural_draw(
                    quad.clone(),
                    solid_color_texture([0, 0, 0, 255]),
                    None,
                ));
            }
            if let Some(texture_name) = quad.texture_override.as_deref() {
                let texture_path = resolve_texture_path("", texture_name);
                let entry = package
                    .find(&texture_path)
                    .ok_or_else(|| Scene2dError::MissingTexture(texture_path.clone()))?;
                let parsed = TexTexture::parse(package.read_entry(entry)).map_err(|error| {
                    Scene2dError::InvalidTexture {
                        path: texture_path.clone(),
                        detail: error.to_string(),
                    }
                })?;
                let texture =
                    parsed
                        .to_texture_image()
                        .map_err(|error| Scene2dError::InvalidTexture {
                            path: texture_path.clone(),
                            detail: error.to_string(),
                        })?;
                let level = texture
                    .levels
                    .first()
                    .ok_or_else(|| Scene2dError::InvalidTexture {
                        path: texture_path.clone(),
                        detail: "texture has no uploadable base level".into(),
                    })?;
                debug!(
                    node = %quad.node_id,
                    texture = %texture_path,
                    width = level.width,
                    height = level.height,
                    "Prepared authored fallback for unavailable system scene texture"
                );
                return Ok(Scene2dDraw {
                    quad,
                    blend_mode: BlendMode::Translucent,
                    texture_path,
                    texture,
                    video: None,
                    text_render: None,
                    uv: [0.0, 0.0, 1.0, 1.0],
                    animation: None,
                    mesh: None,
                    water_wave_masks: Vec::new(),
                    water_wave_normals: Vec::new(),
                    water_flow_mask: None,
                    water_flow_phase: None,
                    iris_mask: None,
                    foliage_masks: Vec::new(),
                    shine_mask: None,
                    shake_maps: Vec::new(),
                    pulse_masks: Vec::new(),
                });
            }
            let model = model_by_path.get(quad.resource.as_str()).ok_or_else(|| {
                Scene2dError::MissingModel {
                    node: quad.node_id.clone(),
                    model: quad.resource.clone(),
                }
            })?;
            let material_path = &model.definition.material;
            let material = material_by_path
                .get(material_path.as_str())
                .ok_or_else(|| Scene2dError::MissingMaterial {
                    model: model.path.clone(),
                    material: material_path.clone(),
                })?;
            let pass = material
                .definition
                .passes
                .first()
                .ok_or_else(|| Scene2dError::MissingMaterialPass(material.path.clone()))?;
            if matches!(pass.blend_mode, BlendMode::Unknown(_)) {
                return Err(Scene2dError::UnsupportedBlendMode(material.path.clone()));
            }
            let [texture_name] = pass.textures.as_slice() else {
                return Err(Scene2dError::UnsupportedTextureCount(material.path.clone()));
            };
            let texture_path = resolve_texture_path(&material.path, texture_name);
            let entry = package
                .find(&texture_path)
                .ok_or_else(|| Scene2dError::MissingTexture(texture_path.clone()))?;
            let bytes = package.read_entry(entry);
            // Direct material video references and video-bearing TEX containers
            // enter the same submission path; media is never uploaded as pixels.
            let parsed = if better_wallpaper_scene_format::is_video_resource(&texture_path) {
                None
            } else {
                Some(
                    TexTexture::parse(bytes).map_err(|error| Scene2dError::InvalidTexture {
                        path: texture_path.clone(),
                        detail: error.to_string(),
                    })?,
                )
            };
            let encoded_video = if parsed.as_ref().is_some_and(|tex| tex.is_video) {
                Some(
                    parsed
                        .as_ref()
                        .unwrap()
                        .mipmaps
                        .first()
                        .ok_or_else(|| Scene2dError::InvalidTexture {
                            path: texture_path.clone(),
                            detail: "video TEX has no media payload".into(),
                        })?
                        .data
                        .clone(),
                )
            } else if parsed.is_none() {
                Some(Arc::from(bytes))
            } else {
                None
            };
            let video = encoded_video.map(|encoded| {
                videos
                    .entry(texture_path.clone())
                    .or_insert_with(|| SceneVideoTexture {
                        encoded,
                        frame: Arc::new(RwLock::new(None)),
                    })
                    .clone()
            });
            let (texture, uv, animation, mesh) = if let Some(parsed) =
                parsed.as_ref().filter(|tex| !tex.is_video)
            {
                debug!(
                    texture = %texture_path,
                    format = ?parsed.format,
                    embedded_format = ?parsed.free_image_format,
                    mip_levels = parsed.mipmaps.len(),
                    animated_frames = parsed.frames.len(),
                    is_video = parsed.is_video,
                    "Decoded scene texture"
                );
                let texture =
                    parsed
                        .to_texture_image()
                        .map_err(|error| Scene2dError::InvalidTexture {
                            path: texture_path.clone(),
                            detail: error.to_string(),
                        })?;
                let base_level =
                    texture
                        .levels
                        .first()
                        .ok_or_else(|| Scene2dError::InvalidTexture {
                            path: texture_path.clone(),
                            detail: "texture has no uploadable base level".into(),
                        })?;
                let upload_width = base_level.width as f32;
                let upload_height = base_level.height as f32;
                let uv = [
                    0.0,
                    0.0,
                    parsed.width as f32 / upload_width,
                    parsed.height as f32 / upload_height,
                ];
                let animation = sprite_animation(parsed, upload_width, upload_height);
                let mesh = model
                    .definition
                    .puppet
                    .as_deref()
                    .map(|path| load_puppet_mesh(package, path, &quad, parsed.width, parsed.height))
                    .transpose()?;
                (texture, uv, animation, mesh)
            } else {
                (
                    solid_color_texture([0, 0, 0, 0]),
                    [0.0, 0.0, 1.0, 1.0],
                    None,
                    None,
                )
            };
            let water_wave_masks = quad
                .water_waves
                .iter()
                .map(|effect| {
                    effect
                        .mask
                        .as_deref()
                        .map(|path| load_effect_texture(package, path))
                        .transpose()
                })
                .collect::<Result<Vec<_>, _>>()?;
            let water_flow_mask = quad
                .water_flow
                .as_ref()
                .and_then(|effect| effect.mask.as_deref())
                .map(|path| load_effect_texture(package, path))
                .transpose()?;
            let water_wave_normals = quad
                .water_waves
                .iter()
                .map(|effect| {
                    effect
                        .normal
                        .as_deref()
                        .map(|path| load_effect_texture(package, path))
                        .transpose()
                })
                .collect::<Result<Vec<_>, _>>()?;
            if let Some((wave, ripple)) = quad
                .water_waves
                .first()
                .and_then(|wave| wave.ripple.map(|ripple| (wave, ripple)))
            {
                debug!(
                    node = %quad.node_id,
                    mask = ?wave.mask,
                    normal = ?wave.normal,
                    direction = wave.direction,
                    scale = wave.scale,
                    strength = wave.strength,
                    animation_speed = ripple.animation_speed,
                    scroll_speed = ripple.scroll_speed,
                    ratio = ripple.ratio,
                    "Prepared normal-map water ripple effect"
                );
            }
            let water_flow_phase = quad
                .water_flow
                .as_ref()
                .and_then(|effect| effect.phase.as_deref())
                .map(|path| load_effect_texture(package, path))
                .transpose()?;
            let iris_mask = quad
                .iris
                .as_ref()
                .and_then(|effect| effect.mask.as_deref())
                .map(|path| load_effect_texture(package, path))
                .transpose()?;
            let foliage_masks = quad
                .foliage_sway
                .iter()
                .map(|effect| {
                    effect
                        .mask
                        .as_deref()
                        .map(|path| load_effect_texture(package, path))
                        .transpose()
                })
                .collect::<Result<Vec<_>, _>>()?;
            let shine_mask = quad
                .shine
                .as_ref()
                .and_then(|effect| effect.mask.as_deref())
                .map(|path| load_effect_texture(package, path))
                .transpose()?;
            let shake_maps = quad
                .shakes
                .iter()
                .map(|effect| {
                    effect
                        .direction_map
                        .as_deref()
                        .map(|path| load_effect_texture(package, path))
                        .transpose()
                })
                .collect::<Result<Vec<_>, _>>()?;
            let pulse_masks = quad
                .pulses
                .iter()
                .map(|effect| {
                    effect
                        .mask
                        .as_deref()
                        .map(|path| load_effect_texture(package, path))
                        .transpose()
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok(Scene2dDraw {
                quad,
                blend_mode: pass.blend_mode.clone(),
                texture_path,
                texture,
                video,
                text_render: None,
                uv,
                animation,
                mesh,
                water_wave_masks,
                water_wave_normals,
                water_flow_mask,
                water_flow_phase,
                iris_mask,
                foliage_masks,
                shine_mask,
                shake_maps,
                pulse_masks,
            })
        })();
        match resolved {
            Ok(draw) => draws.push(draw),
            Err(error) => {
                skipped_nodes += 1;
                warn!(%error, "Skipping unsupported scene image node");
            }
        }
    }
    Ok(Scene2dAssets {
        draws,
        skipped_nodes,
        projection_size,
        layout_viewport,
        camera_zoom: plan.camera_zoom,
        camera_zoom_animation: plan.camera_zoom_animation,
        parallax: plan.parallax,
        audio_spectrum: None,
    })
}

fn procedural_draw(
    quad: Scene2dQuad,
    texture: TextureImage,
    text_render: Option<SceneFontRender>,
) -> Scene2dDraw {
    let texture_path = quad.resource.clone();
    Scene2dDraw {
        quad,
        blend_mode: BlendMode::Translucent,
        texture_path,
        texture,
        video: None,
        text_render,
        uv: [0.0, 0.0, 1.0, 1.0],
        animation: None,
        mesh: None,
        water_wave_masks: Vec::new(),
        water_wave_normals: Vec::new(),
        water_flow_mask: None,
        water_flow_phase: None,
        iris_mask: None,
        foliage_masks: Vec::new(),
        shine_mask: None,
        shake_maps: Vec::new(),
        pulse_masks: Vec::new(),
    }
}

fn solid_color_texture(color: [u8; 4]) -> TextureImage {
    TextureImage {
        format: TexFormat::RGBA8888,
        color_space: TextureColorSpace::Srgb,
        alpha_mode: TextureAlphaMode::Straight,
        levels: vec![TextureMipLevel {
            width: 1,
            height: 1,
            data: Arc::from(color),
        }],
    }
}

fn resolve_scene_font(
    package: &PkgReader,
    text: &SceneText,
    label: &str,
    resolution_scale: f32,
) -> Result<SceneFontRender, Scene2dError> {
    let pixel_size = (text.point_size * 2.0 * resolution_scale).clamp(8.0, 512.0);
    let settings = |collection_index| fontdue::FontSettings {
        collection_index,
        scale: pixel_size,
        ..fontdue::FontSettings::default()
    };
    if let Some(entry) = package.find(&text.font) {
        let font = fontdue::Font::from_bytes(package.read_entry(entry), settings(0)).map_err(
            |detail| Scene2dError::InvalidFont {
                font: text.font.clone(),
                detail: detail.into(),
            },
        )?;
        return Ok(SceneFontRender {
            font: Arc::new(font),
            source: format!("package:{}", text.font),
            fallback: load_cjk_fallback(label, pixel_size),
            fallback_source: cjk_fallback_source(label),
            pixel_size,
            color: text.color,
            dynamic: text.dynamic,
        });
    }

    let candidates = system_font_candidates(&text.font, label);
    let mut errors = Vec::new();
    for (path, collection_index) in candidates {
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) => {
                errors.push(format!("{}: {error}", path.display()));
                continue;
            }
        };
        match fontdue::Font::from_bytes(bytes, settings(collection_index)) {
            Ok(font) => {
                if text.font.contains('/') || text.font.contains('\\') {
                    if path.to_string_lossy().contains("wallpaper_engine/assets") {
                        debug!(
                            requested_font = %text.font,
                            asset_font = %path.display(),
                            "Resolved missing package font from Wallpaper Engine assets"
                        );
                    } else {
                        warn!(
                            requested_font = %text.font,
                            fallback_font = %path.display(),
                            "Scene font resource is missing; using a system font file"
                        );
                    }
                }
                return Ok(SceneFontRender {
                    font: Arc::new(font),
                    source: path.display().to_string(),
                    fallback: load_cjk_fallback(label, pixel_size),
                    fallback_source: cjk_fallback_source(label),
                    pixel_size,
                    color: text.color,
                    dynamic: text.dynamic,
                });
            }
            Err(error) => errors.push(format!("{}: {error}", path.display())),
        }
    }
    Err(Scene2dError::InvalidFont {
        font: text.font.clone(),
        detail: errors.join("; "),
    })
}

fn system_font_candidates(requested: &str, label: &str) -> Vec<(PathBuf, u32)> {
    let needs_cjk = label
        .chars()
        .any(|character| matches!(character as u32, 0x2E80..=0x9FFF | 0xF900..=0xFAFF));
    let mut candidates = Vec::new();
    if !requested.starts_with("systemfont_")
        && let Some(user_home) = std::env::var_os("HOME").map(PathBuf::from)
    {
        for steam_root in [
            user_home.join(".local/share/Steam"),
            user_home.join(".steam/steam"),
            user_home.join(".var/app/com.valvesoftware.Steam/.local/share/Steam"),
        ] {
            candidates.push((
                steam_root
                    .join("steamapps/common/wallpaper_engine/assets")
                    .join(requested),
                0,
            ));
        }
    }
    if needs_cjk {
        candidates.push((
            PathBuf::from("/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc"),
            2,
        ));
        candidates.push((
            PathBuf::from("/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc"),
            2,
        ));
    }
    let requested_lower = requested.to_ascii_lowercase();
    if requested_lower.contains("arial") || requested_lower.contains("sans") {
        candidates.push((
            PathBuf::from("/usr/share/fonts/liberation/LiberationSans-Regular.ttf"),
            0,
        ));
    }
    if requested_lower.contains("mono") || requested_lower.contains("monofur") {
        candidates.push((
            PathBuf::from("/usr/share/fonts/noto/NotoSansMono-Regular.ttf"),
            0,
        ));
    }
    candidates.extend([
        (
            PathBuf::from("/usr/share/fonts/noto/NotoSans-Regular.ttf"),
            0,
        ),
        (
            PathBuf::from("/usr/share/fonts/Adwaita/AdwaitaSans-Regular.ttf"),
            0,
        ),
    ]);
    candidates
}

fn cjk_fallback_path(label: &str) -> Option<(PathBuf, u32)> {
    label
        .chars()
        .any(|character| matches!(character as u32, 0x2E80..=0x9FFF | 0xF900..=0xFAFF))
        .then(|| {
            (
                PathBuf::from("/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc"),
                2,
            )
        })
        .filter(|(path, _)| path.is_file())
}

fn load_cjk_fallback(label: &str, pixel_size: f32) -> Option<Arc<fontdue::Font>> {
    let (path, collection_index) = cjk_fallback_path(label)?;
    let bytes = std::fs::read(path).ok()?;
    fontdue::Font::from_bytes(
        bytes,
        fontdue::FontSettings {
            collection_index,
            scale: pixel_size.max(8.0),
            ..fontdue::FontSettings::default()
        },
    )
    .ok()
    .map(Arc::new)
}

fn cjk_fallback_source(label: &str) -> Option<String> {
    cjk_fallback_path(label).map(|(path, _)| path.display().to_string())
}

/// Rasterizes text using the selected TTF/TTC font and its real line metrics.
pub fn font_text_texture(text: &str, render: &SceneFontRender) -> TextureImage {
    let characters = text.chars().take(256).collect::<Vec<_>>();
    let mut glyphs = Vec::with_capacity(characters.len());
    let mut pen_x = 0.0_f32;
    let mut previous: Option<(char, bool)> = None;
    let line = render.font.horizontal_line_metrics(render.pixel_size);
    let mut top = line.map_or(0.0, |metrics| -metrics.ascent);
    let mut bottom = line.map_or(render.pixel_size, |metrics| -metrics.descent);
    if let Some(fallback_line) = render
        .fallback
        .as_ref()
        .and_then(|font| font.horizontal_line_metrics(render.pixel_size))
    {
        top = top.min(-fallback_line.ascent);
        bottom = bottom.max(-fallback_line.descent);
    }
    let mut left = 0.0_f32;
    for character in characters {
        let use_fallback = render.font.lookup_glyph_index(character) == 0
            && render
                .fallback
                .as_ref()
                .is_some_and(|font| font.lookup_glyph_index(character) != 0);
        let font = if use_fallback {
            render.fallback.as_deref().unwrap_or(&render.font)
        } else {
            &render.font
        };
        if let Some((previous, previous_fallback)) = previous
            && previous_fallback == use_fallback
        {
            pen_x += font
                .horizontal_kern(previous, character, render.pixel_size)
                .unwrap_or(0.0);
        }
        let (metrics, bitmap) = font.rasterize(character, render.pixel_size);
        let glyph_left = pen_x + metrics.xmin as f32;
        let glyph_top = -(metrics.ymin as f32 + metrics.height as f32);
        left = left.min(glyph_left);
        top = top.min(glyph_top);
        bottom = bottom.max(-metrics.ymin as f32);
        glyphs.push((glyph_left, glyph_top, metrics, bitmap));
        pen_x += metrics.advance_width;
        previous = Some((character, use_fallback));
    }
    let right = glyphs.iter().fold(pen_x, |right, (x, _, metrics, _)| {
        right.max(*x + metrics.width as f32)
    });
    let width = (right.ceil() - left.floor()).max(1.0) as usize;
    let height = (bottom.ceil() - top.floor()).max(1.0) as usize;
    let origin_x = -left.floor();
    let origin_y = -top.floor();
    let color = [
        (render.color.x.clamp(0.0, 1.0) * 255.0).round() as u8,
        (render.color.y.clamp(0.0, 1.0) * 255.0).round() as u8,
        (render.color.z.clamp(0.0, 1.0) * 255.0).round() as u8,
    ];
    let mut pixels = vec![0_u8; width * height * 4];
    for (glyph_x, glyph_y, metrics, bitmap) in glyphs {
        let start_x = (origin_x + glyph_x).round() as isize;
        let start_y = (origin_y + glyph_y).round() as isize;
        for y in 0..metrics.height {
            for x in 0..metrics.width {
                let target_x = start_x + x as isize;
                let target_y = start_y + y as isize;
                if target_x < 0
                    || target_y < 0
                    || target_x >= width as isize
                    || target_y >= height as isize
                {
                    continue;
                }
                let alpha = bitmap[y * metrics.width + x];
                let offset = (target_y as usize * width + target_x as usize) * 4;
                pixels[offset..offset + 3].copy_from_slice(&color);
                pixels[offset + 3] = alpha;
            }
        }
    }
    TextureImage {
        format: TexFormat::RGBA8888,
        color_space: TextureColorSpace::Srgb,
        alpha_mode: TextureAlphaMode::Straight,
        levels: vec![TextureMipLevel {
            width: width as u32,
            height: height as u32,
            data: Arc::from(pixels),
        }],
    }
}

fn quad_vertices(transform: Mat3, width: f32, height: f32) -> [[f32; 2]; 4] {
    let half_x = width * 0.5;
    let half_y = height * 0.5;
    [
        transform.transform_point([-half_x, half_y]),
        transform.transform_point([half_x, half_y]),
        transform.transform_point([half_x, -half_y]),
        transform.transform_point([-half_x, -half_y]),
    ]
}

fn load_puppet_mesh(
    package: &PkgReader,
    path: &str,
    quad: &Scene2dQuad,
    logical_width: u32,
    logical_height: u32,
) -> Result<Scene2dMesh, Scene2dError> {
    let entry = package
        .find(path)
        .ok_or_else(|| Scene2dError::MissingPuppet(path.to_owned()))?;
    let model = PuppetModel::parse(package.read_entry(entry)).map_err(|error| {
        Scene2dError::InvalidPuppet {
            path: path.to_owned(),
            detail: error.to_string(),
        }
    })?;
    let animation = model
        .animations
        .first()
        .ok_or_else(|| Scene2dError::InvalidPuppet {
            path: path.to_owned(),
            detail: "puppet model has no animation pose".into(),
        })?;
    let mut frames = Vec::with_capacity(animation.frame_count);
    for frame in 0..animation.frame_count {
        let positions = model.skinned_positions(animation, frame).map_err(|error| {
            Scene2dError::InvalidPuppet {
                path: path.to_owned(),
                detail: error.to_string(),
            }
        })?;
        frames.push(
            positions
                .into_iter()
                .map(|position| {
                    puppet_position_to_ndc(
                        quad.vertices,
                        position,
                        logical_width as f32,
                        logical_height as f32,
                    )
                })
                .collect(),
        );
    }
    debug!(
        puppet = path,
        vertices = model.vertices.len(),
        indices = model.indices.len(),
        bones = model.bones.len(),
        animation_id = animation.id,
        animation = animation.name,
        frames = animation.frame_count,
        fps = animation.fps,
        "Resolved animated puppet mesh"
    );
    Ok(Scene2dMesh {
        frames,
        uv: model.vertices.iter().map(|vertex| vertex.uv).collect(),
        indices: model.indices,
        fps: animation.fps,
    })
}

fn puppet_position_to_ndc(
    quad: [[f32; 2]; 4],
    position: [f32; 3],
    width: f32,
    height: f32,
) -> [f32; 2] {
    let x = position[0] / width + 0.5;
    // MDL geometry is Y-up while scene image bounds are Y-down.
    let y = 0.5 - position[1] / height;
    [
        quad[0][0] + (quad[1][0] - quad[0][0]) * x + (quad[3][0] - quad[0][0]) * y,
        quad[0][1] + (quad[1][1] - quad[0][1]) * x + (quad[3][1] - quad[0][1]) * y,
    ]
}

fn load_effect_texture(
    package: &PkgReader,
    logical_name: &str,
) -> Result<Scene2dEffectTexture, Scene2dError> {
    let path = resolve_texture_path("", logical_name);
    let entry = package
        .find(&path)
        .ok_or_else(|| Scene2dError::MissingTexture(path.clone()))?;
    let parsed = TexTexture::parse(package.read_entry(entry)).map_err(|error| {
        Scene2dError::InvalidTexture {
            path: path.clone(),
            detail: error.to_string(),
        }
    })?;
    let texture = parsed
        .to_texture_image()
        .map_err(|error| Scene2dError::InvalidTexture {
            path: path.clone(),
            detail: error.to_string(),
        })?;
    Ok(Scene2dEffectTexture { path, texture })
}

fn sprite_animation(
    texture: &TexTexture,
    upload_width: f32,
    upload_height: f32,
) -> Option<SpriteAnimation> {
    let width = upload_width;
    let height = upload_height;
    if width <= 0.0 || height <= 0.0 || texture.frames.len() < 2 {
        return None;
    }
    let frames = texture
        .frames
        .iter()
        .filter_map(|frame| {
            if !frame.frametime.is_finite()
                || frame.frametime <= 0.0
                || !frame.x.is_finite()
                || !frame.y.is_finite()
                || !frame.width.is_finite()
                || !frame.height.is_finite()
                || frame.x < 0.0
                || frame.y < 0.0
                || frame.width <= 0.0
                || frame.height <= 0.0
                || frame.x + frame.width > width
                || frame.y + frame.height > height
            {
                return None;
            }
            Some(SpriteFrame {
                uv: [
                    (frame.x + 0.5) / width,
                    (frame.y + 0.5) / height,
                    (frame.x + frame.width - 0.5) / width,
                    (frame.y + frame.height - 0.5) / height,
                ],
                duration_seconds: frame.frametime,
            })
        })
        .collect::<Vec<_>>();
    if frames.len() < 2 {
        return None;
    }
    let duration_seconds = frames
        .iter()
        .map(|frame| frame.duration_seconds)
        .sum::<f32>();
    if !duration_seconds.is_finite() || duration_seconds <= 0.0 {
        return None;
    }
    Some(SpriteAnimation {
        frames,
        duration_seconds,
    })
}

/// Builds a deterministic backend-independent list of 2D draws from validated scene IR.
pub fn build_scene_2d_plan(
    graph: &SceneGraph,
    options: Scene2dOptions,
) -> Result<Scene2dPlan, Scene2dError> {
    if options.viewport_width == 0 || options.viewport_height == 0 {
        return Err(Scene2dError::EmptyViewport);
    }
    let mut indexes = HashMap::with_capacity(graph.nodes.len());
    for (index, node) in graph.nodes.iter().enumerate() {
        if indexes.insert(node.id.as_str(), index).is_some() {
            return Err(Scene2dError::DuplicateNodeId(node.id.clone()));
        }
    }
    for node in &graph.nodes {
        if let Some(parent) = node.parent.as_deref()
            && !indexes.contains_key(parent)
        {
            return Err(Scene2dError::MissingParent {
                node: node.id.clone(),
                parent: parent.to_owned(),
            });
        }
    }

    let projection = graph
        .camera
        .projection_size
        .unwrap_or(better_wallpaper_scene_format::Vec2 {
            x: options.viewport_width as f32,
            y: options.viewport_height as f32,
        });
    if !projection.x.is_finite()
        || !projection.y.is_finite()
        || projection.x <= 0.0
        || projection.y <= 0.0
    {
        return Err(Scene2dError::InvalidProjection);
    }
    // The serialized camera position belongs to the editor viewport. Runtime
    // 2D layers are authored in project-canvas coordinates and full-screen
    // layers consistently use projection/2 as their origin. Applying the
    // editor pan here shifts otherwise exact full-screen layers and exposes
    // clear-color borders.
    let editor_camera = graph
        .camera
        .center
        .unwrap_or(better_wallpaper_scene_format::Vec3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        });
    let center_x = projection.x * 0.5;
    let center_y = projection.y * 0.5;
    let viewport_width = options.viewport_width as f32;
    let viewport_height = options.viewport_height as f32;
    // Preserve project-canvas proportions on every output. `cover` matches
    // wallpaper semantics: the scene fills the output and only the excess on
    // the long axis is cropped. Scaling X and Y independently would distort
    // authored layer positions on outputs whose aspect ratio differs from the
    // scene projection.
    let output_scale = (viewport_width / projection.x).max(viewport_height / projection.y);
    let view = Mat3::scale(
        2.0 * output_scale / viewport_width,
        2.0 * output_scale / viewport_height,
    ) * Mat3::translation(-center_x, -center_y);
    debug!(
        projection_width = projection.x,
        projection_height = projection.y,
        viewport_width,
        viewport_height,
        output_scale,
        editor_camera_x = editor_camera.x,
        editor_camera_y = editor_camera.y,
        view_center_x = center_x,
        view_center_y = center_y,
        camera_zoom = graph.camera.zoom,
        camera_zoom_animated = graph.camera.zoom_animation.is_some(),
        animated_opacity_nodes = graph
            .nodes
            .iter()
            .filter(|node| node.transform.opacity_animation.is_some())
            .count(),
        fallback_instance_textures = graph
            .nodes
            .iter()
            .filter(|node| node.texture_override.is_some())
            .count(),
        backdrop_fade_nodes = graph
            .nodes
            .iter()
            .filter(|node| node.backdrop_fade.is_some())
            .count(),
        "Building aspect-preserving scene layout in project canvas coordinates; editor camera pan ignored"
    );

    let mut worlds = vec![None; graph.nodes.len()];
    let mut visible = vec![None; graph.nodes.len()];
    let mut visiting = HashSet::new();
    for index in 0..graph.nodes.len() {
        resolve_node(
            index,
            graph,
            &indexes,
            &mut worlds,
            &mut visible,
            &mut visiting,
        )?;
    }

    let mut quads = Vec::new();
    let mut skipped_nodes = 0;
    for (index, node) in graph.nodes.iter().enumerate() {
        let (resource, text) = match &node.kind {
            SceneNodeKind::Image(resource) | SceneNodeKind::Video(resource) => {
                (resource.clone(), None)
            }
            SceneNodeKind::Text(text) => (
                format!("__better_wallpaper/text/{}", node.id),
                Some(text.clone()),
            ),
            _ => {
                skipped_nodes += 1;
                continue;
            }
        };
        if !visible[index].unwrap_or(false) {
            continue;
        }
        if resource == "models/util/solidlayer.json"
            && let Some(effect) = node
                .effects
                .iter()
                .find(|effect| unsupported_solid_generator(effect))
        {
            // A solid layer is commonly just the input canvas for a procedural
            // effect. Drawing that input without the generator turns the
            // intended transparent result into a conspicuous opaque rectangle.
            skipped_nodes += 1;
            warn!(
                node = %node.id,
                node_name = %node.name,
                %effect,
                "Skipping procedural solid layer because its pixel-generating effect is unsupported"
            );
            continue;
        }
        let Some(size) = node.transform.size else {
            skipped_nodes += 1;
            continue;
        };
        let opacity = node.transform.opacity.unwrap_or(1.0);
        if !opacity.is_finite() || !(0.0..=1.0).contains(&opacity) {
            return Err(Scene2dError::InvalidOpacity(node.id.clone()));
        }
        if let Some(visualizer) = node.audio_visualizer {
            let origin = node
                .transform
                .origin
                .unwrap_or(better_wallpaper_scene_format::Vec3 {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                });
            let rotation = node.transform.angles.map_or(0.0, |angles| angles.z);
            let parent_world = node
                .parent
                .as_deref()
                .map(|parent| worlds[indexes[parent]].unwrap_or(Mat3::IDENTITY))
                .unwrap_or(Mat3::IDENTITY);
            for bin in 0..visualizer.bins {
                let local =
                    Mat3::translation(origin.x + visualizer.spacing * (bin + 1) as f32, origin.y)
                        * Mat3::rotation(rotation)
                        * Mat3::scale(visualizer.bar_width, 1.0);
                let transform = view * parent_world * local;
                quads.push(Scene2dQuad {
                    node_id: format!("{}-audio-{bin}", node.id),
                    resource: resource.clone(),
                    text: None,
                    video: matches!(node.kind, SceneNodeKind::Video(_)),
                    parallax_depth: node
                        .parallax_depth
                        .map_or([0.0, 0.0], |depth| [depth.x, depth.y]),
                    dynamic_scale: None,
                    audio_response: Some(SceneAudioResponse {
                        bin,
                        height_scale: visualizer.height_scale,
                    }),
                    layout_transform: transform,
                    vertices: quad_vertices(transform, size.x, size.y),
                    opacity,
                    opacity_animation: node.transform.opacity_animation.clone(),
                    texture_override: node.texture_override.clone(),
                    backdrop_fade: node.backdrop_fade.is_some(),
                    scroll: node.scroll,
                    water_waves: node.water_waves.clone(),
                    water_flow: node.water_flow.clone(),
                    shakes: node.shakes.clone(),
                    pulses: node.pulses.clone(),
                    spin: node.spin,
                    iris: node.iris.clone(),
                    foliage_sway: node.foliage_sway.clone(),
                    shine: node.shine.clone(),
                });
            }
            continue;
        }
        let transform = view * worlds[index].unwrap_or(Mat3::IDENTITY);
        quads.push(Scene2dQuad {
            node_id: node.id.clone(),
            resource,
            text,
            video: matches!(node.kind, SceneNodeKind::Video(_)),
            parallax_depth: node
                .parallax_depth
                .map_or([0.0, 0.0], |depth| [depth.x, depth.y]),
            dynamic_scale: node.dynamic_scale,
            audio_response: None,
            layout_transform: transform,
            vertices: quad_vertices(transform, size.x, size.y),
            opacity,
            opacity_animation: node
                .backdrop_fade
                .clone()
                .or_else(|| node.transform.opacity_animation.clone()),
            texture_override: node.texture_override.clone(),
            backdrop_fade: node.backdrop_fade.is_some(),
            scroll: node.scroll,
            water_waves: node.water_waves.clone(),
            water_flow: node.water_flow.clone(),
            shakes: node.shakes.clone(),
            pulses: node.pulses.clone(),
            spin: node.spin,
            iris: node.iris.clone(),
            foliage_sway: node.foliage_sway.clone(),
            shine: node.shine.clone(),
        });
    }
    Ok(Scene2dPlan {
        quads,
        skipped_nodes,
        projection_size: [projection.x, projection.y],
        layout_viewport: [options.viewport_width, options.viewport_height],
        camera_zoom: graph.camera.zoom,
        camera_zoom_animation: graph.camera.zoom_animation.clone(),
        parallax: graph.camera.parallax,
    })
}

fn unsupported_solid_generator(effect: &str) -> bool {
    let effect = effect.to_ascii_lowercase();
    (effect.contains("audio") && (effect.contains("bar") || effect.contains("visualizer")))
        || effect.contains("procedural_noise")
}

fn resolve_node(
    index: usize,
    graph: &SceneGraph,
    indexes: &HashMap<&str, usize>,
    worlds: &mut [Option<Mat3>],
    visible: &mut [Option<bool>],
    visiting: &mut HashSet<usize>,
) -> Result<(), Scene2dError> {
    if worlds[index].is_some() {
        return Ok(());
    }
    if !visiting.insert(index) {
        return Err(Scene2dError::ParentCycle(graph.nodes[index].id.clone()));
    }
    let node = &graph.nodes[index];
    let origin = node
        .transform
        .origin
        .unwrap_or(better_wallpaper_scene_format::Vec3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        });
    let scale = node
        .transform
        .scale
        .unwrap_or(better_wallpaper_scene_format::Vec3 {
            x: 1.0,
            y: 1.0,
            z: 1.0,
        });
    let rotation = node.transform.angles.map_or(0.0, |angles| angles.z);
    let local = Mat3::translation(origin.x, origin.y)
        * Mat3::rotation(rotation)
        * Mat3::scale(scale.x, scale.y);
    let (world, inherited_visibility) = if let Some(parent) = node.parent.as_deref() {
        let parent_index = indexes[parent];
        resolve_node(parent_index, graph, indexes, worlds, visible, visiting)?;
        (
            worlds[parent_index].unwrap_or(Mat3::IDENTITY) * local,
            visible[parent_index].unwrap_or(false),
        )
    } else {
        (local, true)
    };
    worlds[index] = Some(world);
    visible[index] = Some(inherited_visibility && node.visible);
    visiting.remove(&index);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use better_wallpaper_scene_format::parse_scene_graph;

    #[test]
    fn parallax_is_frame_rate_independent_frozen_by_pause_and_axis_bounded() {
        let settings = better_wallpaper_scene_format::SceneParallax {
            amount: 0.25,
            delay: 0.2,
            mouse_influence: 0.5,
        };
        let mut slow = SceneParallaxState::default();
        let mut fast = SceneParallaxState::default();
        slow.update(Some(settings), 0.0, Some([1.0, -1.0]));
        fast.update(Some(settings), 0.0, Some([1.0, -1.0]));
        let expected = slow.update(Some(settings), 1.0, Some([1.0, -1.0]));
        let mut actual = [0.0; 2];
        for frame in 1..=60 {
            actual = fast.update(Some(settings), f64::from(frame) / 60.0, Some([1.0, -1.0]));
        }
        assert!((actual[0] - expected[0]).abs() < 0.00001);
        assert_eq!(fast.update(Some(settings), 1.0, Some([-1.0, 1.0])), actual);
        let offset = scene_parallax_offset(Some(settings), actual, [2.0, 0.0]);
        assert!(offset[0] < -0.24);
        assert_eq!(offset[1], 0.0);
        assert_eq!(scene_parallax_offset(None, actual, [1.0, 1.0]), [0.0, 0.0]);
        assert_eq!(fast.update(None, 1.0, None), [0.0, 0.0]);
        fast.update(Some(settings), 2.0, Some([1.0, 1.0]));
        let recentered = fast.update(Some(settings), 3.0, None);
        assert!(recentered.iter().all(|v| v.abs() < 0.01));
        assert_eq!(
            fast.update(Some(settings), 0.0, Some([1.0, 1.0])),
            [0.0, 0.0]
        );
    }

    #[test]
    fn carries_authored_parallax_to_the_shared_draw_plan() {
        let plan = plan(r#"{"general":{"cameraparallax":true,"cameraparallaxamount":0.25},"objects":[{"image":"models/a.json","size":"10 10","parallaxDepth":"1 0"},{"image":"models/b.json","size":"10 10","parallaxDepth":"0 2"}]}"#).unwrap();
        assert_eq!(plan.parallax.unwrap().amount, 0.25);
        assert_eq!(plan.quads[0].parallax_depth, [1.0, 0.0]);
        assert_eq!(plan.quads[1].parallax_depth, [0.0, 2.0]);
        let meta = better_wallpaper_scene_format::analyse_scene(r#"{"general":{"cameraparallax":true},"objects":[{"image":"models/a.json","parallaxDepth":"1 0"}]}"#).unwrap();
        let report = better_wallpaper_scene_format::compute_compatibility(&meta);
        assert_eq!(
            report.level,
            better_wallpaper_scene_format::CompatibilityLevel::L2
        );
        assert!(
            report
                .warnings
                .iter()
                .all(|warning| !warning.contains("ignored"))
        );
    }

    #[test]
    fn sprite_animation_uses_frame_durations_and_loops() {
        let animation = SpriteAnimation {
            frames: vec![
                SpriteFrame {
                    uv: [0.0, 0.0, 0.5, 1.0],
                    duration_seconds: 0.1,
                },
                SpriteFrame {
                    uv: [0.5, 0.0, 1.0, 1.0],
                    duration_seconds: 0.2,
                },
            ],
            duration_seconds: 0.3,
        };
        assert_eq!(animation.uv_at(0.05), [0.0, 0.0, 0.5, 1.0]);
        assert_eq!(animation.uv_at(0.15), [0.5, 0.0, 1.0, 1.0]);
        assert_eq!(animation.uv_at(0.35), [0.0, 0.0, 0.5, 1.0]);
    }

    #[test]
    fn sprite_animation_insets_uvs_to_prevent_adjacent_frame_bleeding() {
        let texture = TexTexture {
            format: better_wallpaper_scene_format::TexFormat::ARGB8888,
            width: 4,
            height: 2,
            texture_width: 4,
            texture_height: 2,
            flags: 4,
            mipmaps: Vec::new(),
            frames: vec![
                better_wallpaper_scene_format::AnimationFrame {
                    frame_number: 0,
                    frametime: 0.5,
                    x: 0.0,
                    y: 0.0,
                    width: 2.0,
                    height: 2.0,
                },
                better_wallpaper_scene_format::AnimationFrame {
                    frame_number: 1,
                    frametime: 0.5,
                    x: 2.0,
                    y: 0.0,
                    width: 2.0,
                    height: 2.0,
                },
            ],
            is_animated: true,
            container_version: 2,
            free_image_format: None,
            is_video: false,
            spritesheet_cols: 2,
            spritesheet_rows: 1,
            spritesheet_frames: 2,
            spritesheet_duration: 1.0,
        };
        let animation = sprite_animation(&texture, 4.0, 2.0).unwrap();
        assert_eq!(animation.uv_at(0.25), [0.125, 0.25, 0.375, 0.75]);
        assert_eq!(animation.uv_at(0.75), [0.625, 0.25, 0.875, 0.75]);
    }

    fn plan_at(
        json: &str,
        viewport_width: u32,
        viewport_height: u32,
    ) -> Result<Scene2dPlan, Scene2dError> {
        build_scene_2d_plan(
            &parse_scene_graph(json).unwrap(),
            Scene2dOptions {
                viewport_width,
                viewport_height,
            },
        )
    }

    fn plan(json: &str) -> Result<Scene2dPlan, Scene2dError> {
        plan_at(json, 100, 100)
    }

    fn sized_string(value: &str) -> Vec<u8> {
        let mut bytes = (value.len() as u32).to_le_bytes().to_vec();
        bytes.extend_from_slice(value.as_bytes());
        bytes
    }

    fn package(entries: &[(&str, Vec<u8>)]) -> PkgReader {
        let mut bytes = sized_string("PKGV0001");
        bytes.extend_from_slice(&(entries.len() as u32).to_le_bytes());
        let mut offset = 0u32;
        for (name, data) in entries {
            bytes.extend_from_slice(&sized_string(name));
            bytes.extend_from_slice(&offset.to_le_bytes());
            bytes.extend_from_slice(&(data.len() as u32).to_le_bytes());
            offset += data.len() as u32;
        }
        for (_, data) in entries {
            bytes.extend_from_slice(data);
        }
        PkgReader::parse(bytes).unwrap()
    }

    fn rgba_tex(width: u32, height: u32) -> Vec<u8> {
        rgba_tex_with_storage(width, height, width, height)
    }

    fn rgba_tex_with_storage(
        width: u32,
        height: u32,
        storage_width: u32,
        storage_height: u32,
    ) -> Vec<u8> {
        let mut bytes = b"TEXV0005\0TEXI0001\0".to_vec();
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        for value in [storage_width, storage_height, width, height, 0] {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        bytes.extend_from_slice(b"TEXB0003\0");
        for value in [
            13u32,
            1,
            1,
            width,
            height,
            0,
            0,
            storage_width * storage_height * 4,
        ] {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        bytes.resize(
            bytes.len() + (storage_width * storage_height * 4) as usize,
            255,
        );
        bytes
    }

    #[test]
    fn builds_centered_quad_in_source_order() {
        let result = plan(
            r#"{
            "general":{"orthogonalprojection":{"width":100,"height":100}},
            "camera":{"center":"0 0 -1"},
            "objects":[
                {"id":1,"image":"models/a.json","origin":"25 50 0","size":"50 100"},
                {"id":2,"image":"models/b.json","origin":"75 50 0","size":"50 100","alpha":0.5}
            ]
        }"#,
        )
        .unwrap();
        assert_eq!(result.quads[0].vertices[0], [-1.0, 1.0]);
        assert_eq!(result.quads[0].vertices[2], [0.0, -1.0]);
        assert_eq!(result.quads[1].resource, "models/b.json");
        assert_eq!(result.quads[1].opacity, 0.5);
    }

    #[test]
    fn expands_a_recognized_audio_visualizer_into_spectrum_bars() {
        let result = plan(
            r#"{
                "general":{"orthogonalprojection":{"width":100,"height":100}},
                "objects":[{
                    "id":"spectrum",
                    "image":"models/bar.json",
                    "origin":"10 20 0",
                    "size":"4 4",
                    "visible":{
                        "script":"let audioData=engine.registerAudioBuffers(64); audioData.average[i]; thisScene.createLayer('models/bar.json');",
                        "scriptproperties":{"barWidth":5,"scaleY":50,"originX":2},
                        "value":true
                    }
                }]
            }"#,
        )
        .unwrap();
        assert_eq!(result.quads.len(), 64);
        assert_eq!(result.quads[0].audio_response.unwrap().bin, 0);
        assert_eq!(result.quads[63].audio_response.unwrap().bin, 63);
        assert_eq!(result.quads[0].audio_response.unwrap().height_scale, 50.0);
        let first_center = (result.quads[0].vertices[0][0] + result.quads[0].vertices[1][0]) * 0.5;
        let second_center = (result.quads[1].vertices[0][0] + result.quads[1].vertices[1][0]) * 0.5;
        assert!((second_center - first_center - 0.04).abs() < 0.0001);
    }

    #[test]
    fn resolves_text_and_builtin_solid_layers_without_package_assets() {
        let plan = plan(
            r#"{
                "general":{"orthogonalprojection":{"width":200,"height":100}},
                "objects":[
                    {"id":1,"text":{"script":"new Date().getHours(); new Date().getMinutes();","value":"12:34"},"color":"0.5 0.75 1","origin":"100 30 0","size":"80 20"},
                    {"id":2,"image":"models/util/solidlayer.json","origin":"100 70 0","size":"100 2","scale":{"script":"new Date().getSeconds() / 60","value":"1 1 1"}}
                ]
            }"#,
        )
        .unwrap();
        assert_eq!(plan.quads.len(), 2);
        assert_eq!(
            plan.quads[1].dynamic_scale,
            Some(DynamicScaleKind::ClockSecondX)
        );

        let assets = resolve_scene_2d_assets(&package(&[]), plan).unwrap();
        assert_eq!(assets.draws.len(), 2);
        assert_eq!(
            assets.draws[0]
                .text_render
                .as_ref()
                .and_then(|render| render.dynamic),
            Some(DynamicTextKind::Clock)
        );
        assert!(assets.draws[0].texture.levels[0].height >= 32);
        assert_eq!(assets.draws[1].texture.levels[0].data.as_ref(), &[255; 4]);
    }

    #[test]
    fn suppresses_unsupported_pixel_generators_instead_of_drawing_their_solid_input() {
        let result = plan(
            r#"{
                "general":{"orthogonalprojection":{"width":200,"height":100}},
                "objects":[{
                    "id":"sound-bars",
                    "name":"Sound bars",
                    "image":"models/util/solidlayer.json",
                    "origin":"100 50 0",
                    "size":"40 30",
                    "effects":[{
                        "file":"effects/workshop/2084198056/Simple_Audio_Bars/effect.json"
                    }]
                }]
            }"#,
        )
        .unwrap();

        assert!(result.quads.is_empty());
        assert_eq!(result.skipped_nodes, 1);
    }

    #[test]
    fn font_text_texture_is_bounded_and_preserves_color() {
        let text = SceneText {
            value: "1".repeat(300),
            dynamic: None,
            color: better_wallpaper_scene_format::Vec3 {
                x: 0.25,
                y: 0.5,
                z: 1.0,
            },
            font: "systemfont_arial".into(),
            point_size: 32.0,
        };
        let package = package(&[]);
        let render = resolve_scene_font(&package, &text, &text.value, 1.0).unwrap();
        let texture = font_text_texture(&text.value, &render);
        let bounded = font_text_texture(&"1".repeat(256), &render);
        let level = &texture.levels[0];
        assert_eq!(level.width, bounded.levels[0].width);
        assert!(
            level
                .data
                .chunks_exact(4)
                .any(|pixel| pixel[0..3] == [64, 128, 255] && pixel[3] > 0)
        );
    }

    #[test]
    fn text_size_is_stable_across_scene_canvas_resolutions() {
        fn resolved_text_height(scene_width: u32, scene_height: u32) -> f32 {
            let json = format!(
                r#"{{
                    "general":{{"orthogonalprojection":{{"width":{scene_width},"height":{scene_height}}}}},
                    "objects":[{{
                        "id":"clock",
                        "font":"systemfont_arial",
                        "pointsize":32,
                        "text":{{"value":"12:34"}},
                        "origin":"{} {} 0",
                        "size":"100 40"
                    }}]
                }}"#,
                scene_width / 2,
                scene_height / 2
            );
            let plan = plan_at(&json, 1920, 1080).unwrap();
            let assets = resolve_scene_2d_assets(&package(&[]), plan).unwrap();
            let vertices = assets.draws[0].quad.vertices;
            (vertices[0][1] - vertices[3][1]).abs()
        }

        let full_hd = resolved_text_height(1920, 1080);
        let ultra_hd = resolved_text_height(3840, 2160);
        assert!(
            (full_hd - ultra_hd).abs() < 0.002,
            "text height changed across canvas resolutions: {full_hd} vs {ultra_hd}"
        );
    }

    #[test]
    fn ignores_serialized_editor_camera_pan_for_runtime_layout() {
        let result = plan_at(
            r#"{
                "general":{"orthogonalprojection":{"width":3840,"height":2160}},
                "camera":{"center":"85.98602 -93.05257 -1"},
                "objects":[{
                    "id":"background",
                    "image":"models/background.json",
                    "origin":"1920 1080 0",
                    "size":"3840 2160"
                }]
            }"#,
            1920,
            1080,
        )
        .unwrap();

        assert!((result.quads[0].vertices[0][0] + 1.0).abs() < 0.0001);
        assert!((result.quads[0].vertices[0][1] - 1.0).abs() < 0.0001);
        assert!((result.quads[0].vertices[2][0] - 1.0).abs() < 0.0001);
        assert!((result.quads[0].vertices[2][1] + 1.0).abs() < 0.0001);
    }

    #[test]
    fn preserves_projection_aspect_ratio_and_crops_to_fill_output() {
        let result = plan_at(
            r#"{
                "general":{"orthogonalprojection":{"width":200,"height":100}},
                "objects":[{
                    "id":"background",
                    "image":"models/background.json",
                    "origin":"100 50 0",
                    "size":"200 100"
                }]
            }"#,
            100,
            100,
        )
        .unwrap();

        // A 2:1 scene covers a 1:1 output without stretching, so its sides are
        // cropped while its full height remains visible.
        assert!((result.quads[0].vertices[0][0] + 2.0).abs() < 0.0001);
        assert!((result.quads[0].vertices[0][1] - 1.0).abs() < 0.0001);
        assert!((result.quads[0].vertices[2][0] - 2.0).abs() < 0.0001);
        assert!((result.quads[0].vertices[2][1] + 1.0).abs() < 0.0001);
    }

    #[test]
    fn interprets_scene_angles_as_radians() {
        let result = plan(
            r#"{
                "general":{"orthogonalprojection":{"width":100,"height":100}},
                "objects":[{
                    "id":"rotated",
                    "image":"models/a.json",
                    "origin":"50 50 0",
                    "size":"20 10",
                    "angles":"0 0 1.57079632679"
                }]
            }"#,
        )
        .unwrap();

        let top_left = result.quads[0].vertices[0];
        assert!((top_left[0] + 0.1).abs() < 0.0001);
        assert!((top_left[1] + 0.2).abs() < 0.0001);
    }

    #[test]
    fn maps_scene_y_from_bottom_to_top() {
        let result = plan(
            r#"{
                "general":{"orthogonalprojection":{"width":100,"height":100}},
                "objects":[{"image":"models/a.json","origin":"50 90 0","size":"10 10"}]
            }"#,
        )
        .unwrap();
        let center_y = (result.quads[0].vertices[0][1] + result.quads[0].vertices[2][1]) * 0.5;
        assert!((center_y - 0.8).abs() < 0.0001);
    }

    #[test]
    fn composes_parent_transform_and_visibility() {
        let result = plan(r#"{
            "general":{"orthogonalprojection":{"width":100,"height":100}},
            "objects":[
                {"id":"parent","container":true,"origin":"50 50 0","scale":"2 2 1","visible":false},
                {"id":"child","parent":"parent","image":"models/a.json","origin":"10 0 0","size":"10 10"}
            ]
        }"#).unwrap();
        assert!(result.quads.is_empty());
    }

    #[test]
    fn rejects_invalid_hierarchy() {
        let duplicate = plan(r#"{"objects":[{"id":1},{"id":1}]}"#).unwrap_err();
        assert_eq!(duplicate, Scene2dError::DuplicateNodeId("1".into()));

        let missing = plan(r#"{"objects":[{"id":1,"parent":2}]}"#).unwrap_err();
        assert!(matches!(missing, Scene2dError::MissingParent { .. }));

        let cycle = plan(r#"{"objects":[{"id":1,"parent":2},{"id":2,"parent":1}]}"#).unwrap_err();
        assert!(matches!(cycle, Scene2dError::ParentCycle(_)));
    }

    #[test]
    fn rejects_invalid_projection_and_opacity() {
        assert_eq!(
            plan(r#"{"general":{"orthogonalprojection":{"width":0,"height":100}}}"#).unwrap_err(),
            Scene2dError::InvalidProjection
        );
        assert!(matches!(
            plan(r#"{"objects":[{"id":1,"image":"a","size":"1 1","alpha":2}]}"#),
            Err(Scene2dError::InvalidOpacity(_))
        ));
    }

    #[test]
    fn resolves_direct_and_embedded_video_without_static_pixel_conversion() {
        let mut tex = b"TEXV0005\0TEXI0001\0".to_vec();
        for value in [0_u32, 32, 16, 16, 16, 16, 0] {
            tex.extend(value.to_le_bytes());
        }
        tex.extend(b"TEXB0002\0");
        for value in [1_u32, 1, 16, 16, 0, 5, 5] {
            tex.extend(value.to_le_bytes());
        }
        tex.extend(b"media");
        let package = package(&[
            ("videos/a.mp4", b"direct".to_vec()),
            (
                "models/bg.json",
                br#"{"material":"materials/bg.json"}"#.to_vec(),
            ),
            (
                "materials/bg.json",
                br#"{"passes":[{"shader":"genericimage4","textures":["bg"]}]}"#.to_vec(),
            ),
            ("materials/bg.tex", tex),
        ]);
        let plan = plan(r#"{"objects":[{"id":1,"video":"videos/a.mp4","size":"16 16"},{"id":2,"image":"models/bg.json","size":"16 16"}]}"#).unwrap();
        let assets = resolve_scene_2d_assets(&package, plan).unwrap();
        assert_eq!(assets.skipped_nodes, 0);
        assert_eq!(assets.draws.len(), 2);
        assert_eq!(&*assets.draws[0].video.as_ref().unwrap().encoded, b"direct");
        assert_eq!(&*assets.draws[1].video.as_ref().unwrap().encoded, b"media");
        assert_eq!(assets.draws[1].uv, [0.0, 0.0, 1.0, 1.0]);
        let meta = better_wallpaper_scene_format::analyse_scene_with_package(
            r#"{"objects":[{"image":"models/bg.json"}]}"#,
            &package,
        )
        .unwrap();
        assert!(meta.has_video_texture);
        assert_eq!(
            better_wallpaper_scene_format::compute_compatibility(&meta).level,
            better_wallpaper_scene_format::CompatibilityLevel::L2
        );
    }

    #[test]
    fn resolves_shared_model_material_texture_submission() {
        let package = package(&[
            (
                "models/bg.json",
                br#"{"material":"materials/bg.json"}"#.to_vec(),
            ),
            (
                "materials/bg.json",
                br#"{"passes":[{"blending":"translucent","shader":"genericimage4","textures":["bg"]}]}"#.to_vec(),
            ),
            ("materials/bg.tex", rgba_tex(2, 2)),
        ]);
        let plan =
            plan(r#"{"objects":[{"id":"bg","image":"models/bg.json","size":"100 100"}]}"#).unwrap();

        let assets = resolve_scene_2d_assets(&package, plan).unwrap();
        assert_eq!(assets.draws.len(), 1);
        assert_eq!(assets.draws[0].texture_path, "materials/bg.tex");
        assert_eq!(assets.draws[0].blend_mode, BlendMode::Translucent);
        assert_eq!(assets.draws[0].texture.levels[0].data.len(), 16);
    }

    #[test]
    fn derives_uv_bounds_from_the_actual_uploaded_texture_size() {
        let package = package(&[
            (
                "models/bg.json",
                br#"{"material":"materials/bg.json"}"#.to_vec(),
            ),
            (
                "materials/bg.json",
                br#"{"passes":[{"blending":"translucent","shader":"genericimage4","textures":["bg"]}]}"#.to_vec(),
            ),
            ("materials/bg.tex", rgba_tex_with_storage(2, 2, 4, 4)),
        ]);
        let plan =
            plan(r#"{"objects":[{"id":"bg","image":"models/bg.json","size":"100 100"}]}"#).unwrap();

        let assets = resolve_scene_2d_assets(&package, plan).unwrap();
        assert_eq!(
            (
                assets.draws[0].texture.levels[0].width,
                assets.draws[0].texture.levels[0].height
            ),
            (4, 4)
        );
        assert_eq!(assets.draws[0].uv, [0.0, 0.0, 0.5, 0.5]);
    }

    #[test]
    fn resolves_water_effect_masks_as_shared_draw_assets() {
        let package = package(&[
            (
                "models/water.json",
                br#"{"material":"materials/water.json"}"#.to_vec(),
            ),
            (
                "materials/water.json",
                br#"{"passes":[{"blending":"translucent","shader":"genericimage4","textures":["water"]}]}"#.to_vec(),
            ),
            ("materials/water.tex", rgba_tex(2, 2)),
            ("materials/masks/wave.tex", rgba_tex(2, 2)),
            ("materials/masks/flow.tex", rgba_tex(2, 2)),
            ("materials/effects/normal.tex", rgba_tex(2, 2)),
            ("materials/effects/phase.tex", rgba_tex(2, 2)),
        ]);
        let plan = plan(
            r#"{"objects":[{"id":"water","image":"models/water.json","size":"100 100","effects":[{"file":"effects/waterripple/effect.json","passes":[{"constantshadervalues":{"scale":1,"ripplestrength":0.1},"textures":[null,"masks/wave","effects/normal"]}]},{"file":"effects/waterflow/effect.json","passes":[{"constantshadervalues":{"phasescale":1,"speed":1,"strength":1},"textures":[null,"masks/flow","effects/phase"]}]}]}]}"#,
        )
        .unwrap();

        let assets = resolve_scene_2d_assets(&package, plan).unwrap();
        let draw = &assets.draws[0];
        assert_eq!(
            draw.water_wave_masks[0].as_ref().unwrap().path,
            "materials/masks/wave.tex"
        );
        assert_eq!(
            draw.water_flow_mask.as_ref().unwrap().path,
            "materials/masks/flow.tex"
        );
        assert_eq!(
            draw.water_wave_normals[0].as_ref().unwrap().path,
            "materials/effects/normal.tex"
        );
        assert_eq!(
            draw.water_flow_phase.as_ref().unwrap().path,
            "materials/effects/phase.tex"
        );
    }

    #[test]
    fn resolves_iris_foliage_and_shine_masks_as_shared_draw_assets() {
        let package = package(&[
            (
                "models/animated.json",
                br#"{"material":"materials/animated.json"}"#.to_vec(),
            ),
            (
                "materials/animated.json",
                br#"{"passes":[{"blending":"translucent","shader":"genericimage4","textures":["animated"]}]}"#.to_vec(),
            ),
            ("materials/animated.tex", rgba_tex(2, 2)),
            ("materials/masks/iris.tex", rgba_tex(2, 2)),
            ("materials/masks/foliage-a.tex", rgba_tex(2, 2)),
            ("materials/masks/foliage-b.tex", rgba_tex(2, 2)),
            ("materials/masks/shine.tex", rgba_tex(2, 2)),
        ]);
        let plan = plan(
            r#"{"objects":[{"id":"animated","image":"models/animated.json","size":"100 100","effects":[
                {"file":"effects/iris/effect.json","passes":[{"constantshadervalues":{"speed":1,"scale":"1 1"},"textures":[null,"masks/iris"]}]},
                {"file":"effects/foliagesway/effect.json","passes":[{"constantshadervalues":{"speeduv":2,"strength":0.5},"textures":[null,"masks/foliage-a"]}]},
                {"file":"effects/foliagesway/effect.json","passes":[{"constantshadervalues":{"speeduv":3,"strength":0.4},"textures":[null,"masks/foliage-b"]}]},
                {"file":"effects/shine/effect.json","passes":[{"constantshadervalues":{},"textures":[null,"masks/shine"]},{"constantshadervalues":{"rayintensity":0.2,"raylength":0.4}}]}
            ]}]}"#,
        )
        .unwrap();

        let assets = resolve_scene_2d_assets(&package, plan).unwrap();
        let draw = &assets.draws[0];
        assert_eq!(
            draw.iris_mask.as_ref().unwrap().path,
            "materials/masks/iris.tex"
        );
        assert_eq!(draw.foliage_masks.len(), 2);
        assert_eq!(
            draw.foliage_masks[1].as_ref().unwrap().path,
            "materials/masks/foliage-b.tex"
        );
        assert_eq!(
            draw.shine_mask.as_ref().unwrap().path,
            "materials/masks/shine.tex"
        );
    }

    #[test]
    fn skips_an_unsupported_draw_without_discarding_valid_draws() {
        let package = package(&[
            (
                "models/good.json",
                br#"{"material":"materials/good.json"}"#.to_vec(),
            ),
            (
                "models/unsupported.json",
                br#"{"material":"materials/unsupported.json"}"#.to_vec(),
            ),
            (
                "materials/good.json",
                br#"{"passes":[{"blending":"translucent","shader":"genericimage4","textures":["good"]}]}"#.to_vec(),
            ),
            (
                "materials/unsupported.json",
                br#"{"passes":[{"blending":"unimplemented","shader":"genericimage4","textures":["bad"]}]}"#.to_vec(),
            ),
            ("materials/good.tex", rgba_tex(1, 1)),
        ]);
        let plan = plan(
            r#"{"objects":[
                {"id":"good","image":"models/good.json","size":"10 10"},
                {"id":"unsupported","image":"models/unsupported.json","size":"10 10"}
            ]}"#,
        )
        .unwrap();

        let assets = resolve_scene_2d_assets(&package, plan).unwrap();
        assert_eq!(assets.draws.len(), 1);
        assert_eq!(assets.draws[0].quad.node_id, "good");
        assert_eq!(assets.skipped_nodes, 1);
    }

    #[test]
    fn skips_builtin_post_processing_layers_instead_of_covering_real_images() {
        let package = package(&[
            (
                "models/bg.json",
                br#"{"material":"materials/bg.json"}"#.to_vec(),
            ),
            (
                "materials/bg.json",
                br#"{"passes":[{"blending":"translucent","shader":"genericimage4","textures":["bg"]}]}"#.to_vec(),
            ),
            ("materials/bg.tex", rgba_tex(1, 1)),
        ]);
        let plan = plan(
            r#"{"objects":[
                {"id":"bg","image":"models/bg.json","size":"100 100"},
                {"id":"fx","image":"models/util/projectlayer.json","size":"100 100"}
            ]}"#,
        )
        .unwrap();

        let assets = resolve_scene_2d_assets(&package, plan).unwrap();
        assert_eq!(assets.draws.len(), 1);
        assert_eq!(assets.draws[0].quad.node_id, "bg");
        assert_eq!(assets.skipped_nodes, 1);
    }

    #[test]
    fn resolves_an_instance_texture_without_the_builtin_system_material() {
        let package = package(&[("materials/album-fallback.tex", rgba_tex(2, 2))]);
        let plan = plan(
            r#"{"objects":[{"id":"album","image":"models/system-instance.json","size":"16 16","instance":{"textures":["album-fallback"],"usertextures":[{"name":"$mediaThumbnail","type":"system"}]}}]}"#,
        )
        .unwrap();

        let assets = resolve_scene_2d_assets(&package, plan).unwrap();
        assert_eq!(assets.draws.len(), 1);
        assert_eq!(assets.draws[0].texture_path, "materials/album-fallback.tex");
    }

    #[test]
    fn resolves_a_color_grading_entrance_as_an_animated_black_overlay() {
        let plan = plan(
            r#"{"objects":[{"id":"intro","image":"models/util/composelayer.json","size":"100 100","effects":[{"file":"effects/workshop/example/color_grading/effect.json","passes":[{"constantshadervalues":{"Brightness":{"animation":{"c0":[{"frame":0,"value":-1},{"frame":30,"value":0}],"options":{"fps":30,"length":30,"mode":"single"}},"value":0}}}]}]}]}"#,
        )
        .unwrap();

        let assets = resolve_scene_2d_assets(&package(&[]), plan).unwrap();
        assert_eq!(assets.draws.len(), 1);
        assert_eq!(
            assets.draws[0].texture.levels[0].data.as_ref(),
            &[0, 0, 0, 255]
        );
        let fade = assets.draws[0].quad.opacity_animation.as_ref().unwrap();
        assert_eq!(fade.value_at(0.0), 1.0);
        assert_eq!(fade.value_at(1.0), 0.0);
    }
}
