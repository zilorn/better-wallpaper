---
name: better-wallpaper-api
description: Maintain Better Wallpaper daemon HTTP/WebSocket APIs, configuration persistence, playback controls, library access, and SolidJS consumers. Use for management-plane changes; Plasma-specific integration belongs to better-wallpaper-plasma.
---

# Management API and Configuration

Follow repository `AGENTS.md`, including the prohibition on subagents. Source
paths in references are relative to the repository root.

Read [references/api.md](references/api.md) for this category's contracts and
[references/improvements.md](references/improvements.md) when addressing known
gaps or choosing scoped improvements. Backlog items do not authorize unrelated work.

## Workflow

Inspect `server.rs`, `main.rs`, `core/src/config.rs`, and `web/src/index.tsx`
before changing routes or configuration. Preserve v1 migration, v2 round-tripping,
atomic writes, media path boundaries, and safe playback rebuild. Configuration PUT
replaces a document; saved configuration does not prove a runtime backend switch.
For changes to Plasma routes, also load `better-wallpaper-plasma`.

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
