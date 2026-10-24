---
name: better-wallpaper-plasma
description: Maintain Better Wallpaper Plasma QML/Qt Multimedia playback, HTTP media consumption, output synchronization, heartbeats, and QML packaging. Use for Plasma backend behavior or troubleshooting.
---

# KDE Plasma Integration

Follow repository `AGENTS.md`, including the prohibition on subagents. Source
paths in references are relative to the repository root.

Read [references/api.md](references/api.md) for this category's contracts and
[references/improvements.md](references/improvements.md) when addressing known
gaps or choosing scoped improvements. Backlog items do not authorize unrelated work.

## Workflow

Inspect active `main.qml` before choosing a playback path; Qt Multimedia consumes
the daemon's HTTP media URL. The unused native frame module and C ABI have been
removed. Keep config polling, visibility, audio, output names, and heartbeat expiry
consistent. Load rendering for scene work and API for changes to
shared configuration or management routes.

## Maintenance and Verification

Update these references in the same change whenever the category's interfaces,
behavior, limitations, or improvement status change. Update other affected skills
for cross-category contracts. Record demonstrated gaps with source/reproduction
evidence and observable completion criteria; revise or remove resolved items.
Implementation is authoritative when older plans disagree.

Run `cargo clippy` plus relevant feature-specific Rust tests. For GTK changes,
run the unit tests and display regressions documented in `docs/gtk-desktop.md`.
Management uses native GTK; web wallpaper playback still uses Qt WebEngine.
Desktop/GPU verification requires the relevant real session. Report checks that
could not run accurately rather than inferring success from compilation.
