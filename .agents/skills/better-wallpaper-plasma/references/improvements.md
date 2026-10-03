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
