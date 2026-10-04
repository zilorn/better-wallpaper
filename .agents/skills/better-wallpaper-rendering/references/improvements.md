# Rendering Improvements

## Basic Mouse Parallax — Implemented / Desktop Verification Needed

Evidence: scene IR parses bounded camera parameters and per-axis layer depth;
`SceneParallaxState` and `scene_parallax_offset` share scene-clock smoothing and
translation. `SceneGpuRenderer::draw_scene_with_pointer` applies the displacement.
The niri seat/pointer handlers forward only current wallpaper-surface events and
clear pointer state on leave/removal. The daemon gates it with `scene.mouse` and
`scene.parallax`; the Web UI enables these controls with a niri limitation label.
Regression tests cover frame-rate independence, pause, recentering, depth axes,
invalid parameters, disabled input and stale surfaces after hotplug.

Completion: verify actual pointer-driven playback in niri with multiple outputs,
resize and compositor recovery, and compare authored sample motion. Standard
Wayland does not provide other windows' global pointer positions; a supported
compositor input source is required before claiming desktop-wide mouse tracking.
Depth-map parallax and arbitrary mouse scripts remain unsupported.

## Particle Settings — Unsupported by Design

`scene.particle_limit` is preserved and validated, but particle nodes are skipped.
The daemon reports it as ignored and the Web UI keeps the control disabled.
Particle rendering and runtime budget enforcement remain outside the L2 task.

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

## Golden/SSIM Regression Harness — Implemented / Reference Parity Pending

Evidence: `gpu-renderer/tests/scene_golden.rs` renders shared PKG/TEX assets in a
surfaceless GLES context and compares ten analytical PNG references using local
SSIM plus RGBA MSE/max error. Fourteen comparisons passed on llvmpipe with SSIM=1
and zero channel error, including TEXB0004 video updates/context replacement and
basic pointer parallax. The harness exposed and now covers a translucent alpha
compositing bug. The ordinary suite tests the comparison gate; GPU cases require
an explicit command documented in the rendering interfaces.

Completion: add legal authored L1/L2 samples with independently captured Wallpaper
Engine reference screenshots, fixed time/property/pointer inputs and recorded
thresholds; run on supported vendor GPUs and real niri sessions. Synthetic exact
matches do not complete the reference-screenshot fidelity or eight-hour soak exit
criteria in `plan.md`. Fonts, broader effects and authoring-format variation still
need reference sample coverage.
