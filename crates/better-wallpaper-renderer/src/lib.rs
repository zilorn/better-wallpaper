//! GPU 渲染基础设施。公开接口不暴露 FFmpeg 类型，保证解码与呈现可独立演进。

mod cache;
mod nvidia;

pub use cache::{GpuCacheError, GpuResourceCache, GpuResourceKey, MAX_GPU_CACHE_BUDGET_BYTES};
pub use nvidia::{NvidiaDeviceInfo, NvidiaVulkanContext, NvidiaVulkanError};
