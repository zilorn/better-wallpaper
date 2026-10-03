# niri Backend Interfaces

Source: `crates/better-wallpaper-wayland/src/niri.rs` and `egl.rs`; public export:
`crates/better-wallpaper-wayland/src/lib.rs`. Daemon setup and backend selection
live in `crates/better-wallpaper-daemon/src/main.rs`.

| Interface | Contract |
| --- | --- |
| `NiriBackend::connect(target_output: Option<&str>)` | Connects to Wayland and creates a background layer surface for a selected output. Returns `Result<Self>`; output targeting and multi-output orchestration belong to the caller. |
| `output_name()` / `size()` | Reports selected output name and configured size; size falls back to `(1, 1)` before configuration. |
| `present(&DecodedFrame, FillMode)` | Dispatches events, waits for compositor frame readiness, and presents a video frame. Returns `PresentMetrics` covering callback wait, buffer allocation, scaling, and submission. |
| `load_scene_assets(Scene2dAssets)` | Requires available EGL; fails rather than providing software scene rendering when EGL is unavailable. |
| `present_scene(elapsed_seconds)` | Requires EGL, waits for a compositor frame callback, renders the scene, commits, and flushes. A closed layer surface is an error. |
| `dispatch_pending()` | Processes queued Wayland events and propagates failures. |

Video has GPU and `wl_shm` paths; do not assume all scene or CUDA paths have the
same fallback. Startup probes NVIDIA Vulkan/DMA-BUF capability; `--require-nvidia`
turns probe failure into startup failure. That probe alone does not prove end-to-end
zero-copy frame presentation.

Preserve frame callback pacing, configured output selection, resize behavior, and
buffer lifetime. EGL resources must be released before their Wayland connection
and surface. For shared draw/texture changes, also read
[rendering interfaces](../../better-wallpaper-rendering/references/api.md).

Smoke command: `cargo run --release -p better-wallpaper-daemon -- --backend niri --no-ui`.
Actual output/hotplug/GPU checks require a real niri session; a headless run cannot
verify them. Use bounded English diagnostic logs as specified in `AGENTS.md`.
