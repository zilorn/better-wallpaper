# Plasma Improvements

## Scene Rendering — Capability Gap

Evidence: `server.rs::plasma_config_response` sets `scene_rendering_available` to
false; active `main.qml` shows a placeholder. Shared scene draw planning exists.

Completion: implement plugin scene submission with shared scene semantics, verify
rendering and reload/resource cleanup on Plasma, then update capability reporting
and UI together. Compilation alone does not establish desktop rendering support.

## Playback Path Documentation — Resolved

Evidence: `main.qml` uses Qt `MediaPlayer` with the daemon media URL. README,
packaging documentation, and QML logs now describe that HTTP/Qt Multimedia path.
The unused native frame module, Rust C ABI, native library build outputs, and
systemd prerequisite were removed. Packaging verification simulates a legacy
installation and asserts that upgrading removes the obsolete module files.

Future playback-engine changes require an actual QML consumer and desktop
verification, with audio, visibility, and lifecycle behavior preserved.

## Web Project Defaults and Pause — Implemented / Desktop Verification Needed

Evidence: Plasma config returns nested entry URLs and project defaults; QML calls
Wallpaper Engine property listeners after page load and freezes a hidden browser
behind a retained pause screenshot. Empty registration stubs avoid missing global
functions for audio/media callbacks but do not provide their data.
Completion: verify real Plasma page load, pause/resume, visibility and type changes
with nested-entry projects and audio. QML lint alone does not prove playback.
