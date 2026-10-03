# Plasma Improvements

## Scene Rendering — Capability Gap

Evidence: `server.rs::plasma_config_response` sets `scene_rendering_available` to
false; active `main.qml` shows a placeholder. Shared scene draw planning exists.

Completion: implement plugin scene submission with shared scene semantics, verify
rendering and reload/resource cleanup on Plasma, then update capability reporting
and UI together. Compilation alone does not establish desktop rendering support.

## Playback Path Documentation — Alignment Candidate

Evidence: README describes direct local Rust/FFmpeg playback, while current
`main.qml` uses Qt `MediaPlayer` with the daemon media URL. The native
`SharedFrameItem` module exists but is not instantiated by that QML file.

Completion: establish the intended active path for the requested change, verify
its installed QML/plugin consumers, and align documentation and diagnostics with
actual behavior. Do not switch playback engines merely to reconcile wording.
