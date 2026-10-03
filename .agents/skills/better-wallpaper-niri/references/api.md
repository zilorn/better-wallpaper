# niri Backend Interfaces

Source: `crates/better-wallpaper-wayland/src/niri.rs` and `egl.rs`; public export:
`crates/better-wallpaper-wayland/src/lib.rs`. Daemon setup and backend selection
live in `crates/better-wallpaper-daemon/src/main.rs`.

| Interface | Contract |
| --- | --- |
| `NiriBackend::connect(target_output: Option<&str>)` | Connects to Wayland and asynchronously creates a background layer surface when the selected output is available. A missing target/no outputs is a waiting state, not an error. Each caller-owned backend targets one output; auto-selection keeps its current output until removal, then selects another available output. |
| `output_name()` / `size()` | Reports selected output name and configured size; size falls back to `(1, 1)` before configuration. |
| `is_ready()` / `has_output()` / `needs_redraw()` | Reports nonblocking frame readiness, a configured output, and pending hotplug/resize repaint respectively. Frame callbacks from old surfaces are ignored. |
| `present(&DecodedFrame, FillMode)` | Reads/dispatches events and presents only when frame-ready. Returns `Result<Option<PresentMetrics>>`; `None` means absent/configuring/callback-pending, not a successful submission. Caller polling accounts for callback wait; metrics cover allocation, scaling, and submission. |
| `load_scene_assets(Scene2dAssets)` | Retains CPU assets for replacement surfaces, uploading when EGL is ready. Scene rendering still requires EGL and fails if GPU setup/upload fails. |
| `present_scene(elapsed_seconds)` | Returns `Result<bool>`; `false` means no submission while absent/configuring/callback-pending. Ready surfaces render with EGL, commit, and flush. |
| `dispatch_pending()` | Nonblocking socket read plus queued event dispatch and output reconciliation, including while paused. Output removal releases EGL before surface resources; a closed layer is recreated after one second. Socket EOF/protocol/read/flush failures propagate to the supervisor. |

The daemon supervisor retries failed niri pipelines in UI and `--no-ui` modes,
starting at 250 ms and doubling up to five seconds. Thirty seconds of pipeline
runtime resets the delay; reload resets it immediately and reload/cancel interrupts
the wait. Natural completion and empty/disabled configurations are not retried.
UI mode remains idle for config updates; `--no-ui` exits on natural completion.
Failed video attempts stop/join their decoder without cancelling shared control.
Rebuilding a failed pipeline restarts the video/scene timeline; pause state is
preserved, and a paused replacement video pipeline decodes one frame to repaint.

Absent configured outputs do not prevent healthy outputs from playing. When all
outputs are absent, video consumption/audio and scene time/audio wait;
paused video retains its last frame to repaint a newly connected/resized output.
Scene CPU assets are reuploaded into replacement EGL contexts. Upload/render/drop
restore the owning EGL context, and frame callbacks pace submission with EGL swap
interval zero so callback waits do not block output reconciliation or cancellation.

Video has GPU and `wl_shm` paths; do not assume all scene or CUDA paths have the
same fallback. Startup probes NVIDIA Vulkan/DMA-BUF capability; `--require-nvidia`
turns probe failure into startup failure. That probe alone does not prove end-to-end
zero-copy frame presentation.

Scene surfaces do not collect pointer input or implement mouse/parallax behavior.
`scene.mouse`, `scene.parallax`, and `scene.particle_limit` are ignored reserved
settings, reported separately by the daemon and disabled in the Web UI. Their
stored values do not prove backend support; see the rendering improvements.

Preserve frame callback pacing, configured output selection, resize behavior, and
buffer lifetime. EGL resources must be released before their Wayland connection
and surface. For shared draw/texture changes, also read
[rendering interfaces](../../better-wallpaper-rendering/references/api.md).

Smoke command: `cargo run --release -p better-wallpaper-daemon -- --backend niri --no-ui`.
Actual output/hotplug/GPU checks require a real niri session; a headless run cannot
verify them. Use bounded English diagnostic logs as specified in `AGENTS.md`.
