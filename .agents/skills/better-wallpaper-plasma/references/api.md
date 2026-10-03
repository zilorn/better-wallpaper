# Plasma Integration Contracts

Source: `crates/better-wallpaper-daemon/src/server.rs`,
`kde/org.better-wallpaper/contents/ui/main.qml`, and `packaging/`.

## Synchronization API


| Method and route | Contract |
| --- | --- |
| `GET /api/v1/plasma/config?output=<encoded-name>` | Returns `api_version`, `output`, `enabled`, `media_url`, `media_path`, `wallpaper_type`, `web_url`, `fill_mode`, `muted`, `paused`, `loop_playback`, `revision`, `scene_rendering_available`. |
| `POST /api/v1/plasma/heartbeat` | Body `{"output":"screen-name"}` with nonblank name; success `{"accepted":true}`. Invalid body: `400`; oversized/unreadable body: `413`. |

With no configured outputs, all Plasma outputs are enabled. Otherwise names must
match enabled entries. Instances expire after 10 seconds without a heartbeat.
`revision` hashes configuration; pause state is separate from revision changes.

## Active QML Playback

The current `main.qml` uses Qt Multimedia `MediaPlayer` / `VideoOutput` with
`daemonUrl + media_url`, and QtWebEngine for web wallpaper assets. It refreshes
config every two seconds while visible and sends a heartbeat after a successful
refresh. Hidden instances pause video and stop the refresh timer; becoming visible
refreshes configuration again. The daemon-side KDE crate owns control-plane
lifecycle, not the Plasma scene graph.

The daemon serves video file bytes over HTTP (including range requests); Qt
Multimedia decodes video/audio in plasmashell. The QML package needs Qt Quick,
Qt Multimedia, and Qt WebEngine runtime modules. It has no native frame plugin,
Rust C ABI, or CMake build step. Shared GPU rendering remains a Rust library used
by the niri backend; it does not establish a Plasma rendering path.

The daemon currently reports `scene_rendering_available: false`, and active QML
shows a scene placeholder. Shared scene assets alone do not establish Plasma
scene submission. For that work, read
[rendering interfaces](../../better-wallpaper-rendering/references/api.md).

Installation remains in `packaging/install.sh`, which copies the QML package and
removes the obsolete installed `BetterWallpaper/qmldir` and native plugin binary
on upgrade. The systemd unit checks the QML entry, not a native shared library.
Package verification is `./packaging/verify.sh`; it checks cleanup of a simulated
legacy module as well as QML lint, installation, and uninstallation.
