# Rendering Improvements

## Mouse, Parallax and Particle Settings — Not Implemented

Evidence: `SceneConfig` in `better-wallpaper-core/src/config.rs` preserves `mouse`,
`parallax`, and `particle_limit`, but `run_niri_scene` only consumes quality and
audio processing, while `prepare_scene` consumes property overrides.
`NiriBackend::present_scene` passes
elapsed time to the GPU renderer without pointer state or a particle budget.
The daemon now reports the reserved values as ignored, and Web controls are
disabled and marked unimplemented.

Completion: implement each setting's runtime consumer with regression evidence;
verify pointer-driven interaction/parallax in a real desktop session and enforce
particle budgets on actual particle draws. Enable each control and update logs
only when its corresponding behavior works.

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
