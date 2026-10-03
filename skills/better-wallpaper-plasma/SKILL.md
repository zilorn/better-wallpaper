---
name: better-wallpaper-plasma
description: Maintain Better Wallpaper Plasma QML playback, native frame plugin/C ABI, output synchronization, heartbeats, and plugin packaging. Use for Plasma backend behavior or troubleshooting.
---

# KDE Plasma Integration

Follow repository `AGENTS.md`, including the prohibition on subagents. Source
paths in references are relative to the repository root.

Read [references/api.md](references/api.md) for this category's contracts and
[references/improvements.md](references/improvements.md) when addressing known
gaps or choosing scoped improvements. Backlog items do not authorize unrelated work.

## Workflow

Inspect active `main.qml` before choosing a playback path; the native frame module
exists but is not currently instantiated there. Keep config polling, visibility,
audio, output names, and heartbeat expiry consistent. Preserve C ABI ownership and
GL thread/context lifetime. Load rendering for scene work and API for changes to
shared configuration or management routes.

## Maintenance and Verification

Update these references in the same change whenever the category's interfaces,
behavior, limitations, or improvement status change. Update other affected skills
for cross-category contracts. Record demonstrated gaps with source/reproduction
evidence and observable completion criteria; revise or remove resolved items.
Implementation is authoritative when older plans disagree.

Run `cargo clippy` plus relevant checks. For web changes run `bun run check` and
`bun run build` in `web/`; use feature-specific Rust tests for changed behavior.
Desktop/GPU verification requires the relevant real session. Report checks that
could not run accurately rather than inferring success from compilation.
