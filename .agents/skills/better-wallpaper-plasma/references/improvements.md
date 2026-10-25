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

## Web Properties and Data Protocol — A5 Consumer Verified

Plasma now consumes the shared Qt web component on an independent read-only
origin, with typed project overrides, stereo audio and read-only MPRIS callbacks.
The explicit harness loads the actual main.qml in a Qt layer window under niri;
it verifies metadata/timeline, channel separation, pause/resume, disable/unload,
restored source registration and capture teardown. That test exposed and fixed
an empty-URL case that left timers in the previous document running.

Completion of broader desktop verification still requires a real plasmashell
containment, activities, multiple outputs, physical hotplug and session recovery.
Artwork extraction and directory import are outside the A5 bounded protocol.
See [A5 acceptance](../../../../docs/validation/a5-web-protocol.md).
