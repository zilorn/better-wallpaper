# niri Improvements

## Output Hotplug — Implementation Gap / Desktop Verification Needed

Evidence: `OutputHandler` in `crates/better-wallpaper-wayland/src/niri.rs` currently
logs additions, changes, and removals. README flags hotplug verification as pending.

Completion: create/update/release matching background surfaces; verify output
connect/disconnect, resolution changes, and configured output selection in niri.
Inspect daemon multi-output orchestration before changing the ownership model.

## Compositor Reconnect — Verification Pending

Evidence: README flags reconnection as unverified. Inspect the current Wayland
event loop and playback supervisor before assuming recovery is absent.

Completion: demonstrate bounded recovery after compositor restart, safe resource
cleanup, and restored playback without leaked surfaces or stalled frame waits.
