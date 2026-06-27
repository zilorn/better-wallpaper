//! niri/wlroots layer-shell 后端，首版使用 wl_shm 提交 CPU RGBA 帧。

mod egl;
mod niri;

pub use niri::NiriBackend;
