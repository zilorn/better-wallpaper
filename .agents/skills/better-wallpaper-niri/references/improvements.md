# niri Improvements

## Output Hotplug — Implemented / Desktop Verification Needed

Evidence: `OutputHandler` marks output changes for `NiriBackend::reconcile_output`,
which creates matching layers, retains healthy layers, and releases removed layers.
Missing configured outputs remain dormant; auto-selection moves on removal.
`crates/better-wallpaper-wayland/src/niri_tests.rs` exercises additions, removals,
replug, auto-selection, layer closure/backoff, and socket EOF using an isolated
Wayland protocol peer. The peer leaves layers unconfigured; resize/stale-event
tests exercise the configuration state handler directly without initializing EGL.

Completion: verify real multi-output connect/disconnect, resolution changes,
configured selection, paused repaint, and video/scene EGL resource teardown in niri.

## Compositor Reconnect — Implemented / Desktop Verification Needed

Evidence: socket errors propagate from nonblocking event pumping to the daemon
supervisor, which retries niri failures with 250 ms–5 s exponential backoff in both
UI and `--no-ui` modes. Closed layers are recreated on a surviving connection.
Tests in `main.rs` cover retries without reload, natural-completion idle behavior,
latest-config reload during backoff, and cancellation. Video decoder cleanup is
guarded and tested in `playback.rs`. wayland-client exposes connection failures
through errors rather than a `connection_closed` handler.

Completion: demonstrate bounded recovery after compositor restart, safe resource
cleanup, and restored playback without leaked surfaces or stalled frame waits.

## Basic Parallax Input — Implemented / Desktop Verification Needed

`SeatState`, `SeatHandler` and `PointerHandler` bind pointer-capable seats and
forward current-surface enter/motion/leave. The isolated peer regression verifies
disabled input, normalization, leave and stale-surface rejection after removal.
Completion: verify real pointer focus/cursor behavior, multiple seats, multi-output
motion and resize in niri. Only exposed wallpaper-surface input is available;
other windows' global coordinates need a supported compositor-specific source.
