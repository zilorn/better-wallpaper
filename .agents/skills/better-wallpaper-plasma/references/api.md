# Plasma Integration Contracts

Source: `crates/better-wallpaper-daemon/src/server.rs`,
`kde/org.better-wallpaper/contents/ui/main.qml`, `kde/frame-plugin/`, and
`crates/gpu-renderer/src/ffi.rs`.

## Synchronization API


| Method and route | Contract |
| --- | --- |
| `GET /api/v1/plasma/config?output=<encoded-name>` | Returns `api_version`, `output`, `enabled`, `media_url`, `media_path`, `wallpaper_type`, `web_url`, `fill_mode`, `muted`, `paused`, `loop_playback`, `revision`, `scene_rendering_available`. |
| `POST /api/v1/plasma/heartbeat` | Body `{"output":"screen-name"}` with nonblank name; success `{"accepted":true}`. Invalid body: `400`; oversized/unreadable body: `413`. |

With no configured outputs, all Plasma outputs are enabled. Otherwise names must
match enabled entries. Instances expire after 10 seconds without a heartbeat.
`revision` hashes configuration; pause state is separate from revision changes.

## Active QML Playback and Native Plugin Interfaces

The current `main.qml` uses Qt Multimedia `MediaPlayer` / `VideoOutput` with
`daemonUrl + media_url`, and QtWebEngine for web wallpaper assets. It refreshes
config every two seconds while visible and sends a heartbeat after a successful
refresh. Hidden instances pause video and stop the refresh timer; becoming visible
refreshes configuration again. The daemon-side KDE crate owns control-plane
lifecycle, not the Plasma scene graph.

The repository also builds a `BetterWallpaper` QML module registering
`SharedFrameItem`, implemented by `FrameItem` in `kde/frame-plugin/frameitem.h`.
Its properties are `source`, `fillMode`, `paused`, `loopPlayback`, and
`playbackPosition`. The current `main.qml` does not instantiate this type. Check
the active QML consumer before assuming local Rust decoding is the deployed path.

The native Rust C ABI provides `kde_video_decoder_open`, `next`, `seek_start`, and
`destroy`, plus `gpu_renderer_create`, `upload`, `draw`, `resize`, and `destroy`.
Decoder `next` returns 1 for a frame, 0 for end of stream, and -1 for error. Frame
bytes are decoder-owned and must be copied before the next decode or destruction.
Keep C/C++ declarations and Rust layouts in sync; create/use/destroy GPU objects
with a valid owning GL context. See `ffi.rs` for full signatures.

The daemon currently reports `scene_rendering_available: false`, and active QML
shows a scene placeholder. Shared scene assets alone do not establish Plasma
scene submission. For that work, read
[rendering interfaces](../../better-wallpaper-rendering/references/api.md).

Build plugin: `cmake -S kde/frame-plugin -B target/plasma-plugin -DCMAKE_BUILD_TYPE=Release`,
then `cmake --build target/plasma-plugin --parallel`. Installation remains in
`packaging/install.sh`; package verification is `./packaging/verify.sh`.
