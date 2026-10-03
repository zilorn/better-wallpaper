# Rendering Improvements

## Scene Fidelity — Sample-Driven Candidate

Evidence: `crates/better-wallpaper-renderer/src/scene2d.rs` reports skipped draws
and unsupported procedural generators; `crates/gpu-renderer/src/scene_renderer.rs`
reports unsupported texture formats. Older effect/rendering gaps in `plan.md` and
`progress.txt` may already be resolved.

Completion: select a currently reproducible mismatch, identify the missing
semantic, implement it with regression or golden evidence, and update compatibility
reporting. Scene configuration switches alone are not proof of runtime support.

## Parser Fuzzing — Verification Pending

Evidence: `progress.txt` records outstanding 24-hour runs; targets and instructions
are in `crates/better-wallpaper-scene-format/fuzz/`.

Completion: run the documented targets for the planned duration, record actual
results and reproducible failures, and address failures within task scope. Unit
tests or compilation do not fulfill the long-running verification criterion.
