//! GPU 渲染基础设施。公开接口不暴露 FFmpeg 类型，保证解码与呈现可独立演进。

mod cache;
mod nvidia;
mod scene2d;

pub use cache::{GpuCacheError, GpuResourceCache, GpuResourceKey, MAX_GPU_CACHE_BUDGET_BYTES};
pub use nvidia::{NvidiaDeviceInfo, NvidiaVulkanContext, NvidiaVulkanError};
pub use scene2d::{
    Mat3, Scene2dAssets, Scene2dDraw, Scene2dError, Scene2dMesh, Scene2dOptions, Scene2dPlan,
    Scene2dQuad, SceneAudioResponse, SceneAudioSpectrum, SceneFontRender, SpriteAnimation,
    SpriteFrame, build_scene_2d_plan, font_text_texture, resolve_scene_2d_assets,
};
