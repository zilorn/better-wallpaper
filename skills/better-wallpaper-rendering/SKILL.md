---
name: better-wallpaper-rendering
description: Maintain Better Wallpaper scene parsing, shared draw plans, textures, GPU submission, and resource caches. Use for rendering or format fidelity work; desktop output lifecycle belongs to the niri or Plasma skill.
---

# Shared Rendering and Scene Formats

Follow repository `AGENTS.md`, including the prohibition on subagents. Source
paths in references are relative to the repository root.

Read [references/api.md](references/api.md) for this category's contracts and
[references/improvements.md](references/improvements.md) when addressing known
gaps or choosing scoped improvements. Backlog items do not authorize unrelated work.

## Workflow

Trace parsing through shared scene plans/assets into GPU submission. Keep format
semantics shared across backends, preserve unsupported-feature reporting, and
verify texture channels, dimensions, mips, UVs, and resource release. Use legal
local samples and existing fixtures for private formats. Load the niri or Plasma
skill when changes affect presentation or desktop-owned GL contexts.

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
