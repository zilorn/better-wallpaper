---
name: better-wallpaper-niri
description: Maintain Better Wallpaper niri/Wayland layer surfaces, output selection, frame pacing, EGL integration, hotplug, and compositor recovery. Use for niri backend changes or troubleshooting.
---

# niri and Wayland Integration

Follow repository `AGENTS.md`, including the prohibition on subagents. Source
paths in references are relative to the repository root.

Read [references/api.md](references/api.md) for this category's contracts and
[references/improvements.md](references/improvements.md) when addressing known
gaps or choosing scoped improvements. Backlog items do not authorize unrelated work.

## Workflow

Trace `NiriBackend` lifecycle and daemon output orchestration together. Preserve
frame callbacks, buffer lifetime, output selection, and EGL teardown order.
Distinguish video fallback from scene EGL requirements. Load the rendering skill
for shared texture/draw changes and the API skill for configuration changes.

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
