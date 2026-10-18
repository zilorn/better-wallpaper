# Rendering Improvements

## Bounded Resource Audit — Implemented / Reference Coverage Explicit

`asset-audit` inventories named loose TEX/JSON roots and selected PKGs with hashes,
format and failure categories, absolute member offsets and bounded candidate
reference discovery. Tests cover transitive field references, shared fallback,
package isolation, malformed assets, symlinks, deterministic output and budgets.
The audit calls `TexTexture::parse_with_budget` to cap retained decoded mip bytes.
It also exposed stale TEXS frame bases in `tests/fixtures/dynamic-scene/scene.pkg`;
the package is regenerated from the current generator and a regression now reads
the checked-in bytes to verify atlas indices, rectangles and timing.

Local inventory found JSON with comments/trailing commas (for example builtin
`effects/blend/preview/effects/blend/effect.json` and
`effects/fluidsimulation/effect.json`). Strict JSON parsing records those failures
and incomplete reference coverage. It does not imply the native engine rejects
them. Complete broader reference discovery only after defining and testing the
additional syntax/fields and lookup rules against samples; do not silently treat
unknown dependencies as resolved or absent from the installation.
The retained [A0 inventory report](../../../../docs/validation/a0-asset-inventory.md)
records 2,593 assets/container entries and 3,858 candidate references from two
identical scans. Its machine-readable manifest preserves 32 strict-JSON failures
and incomplete reference coverage; 459 size-validated TEX payloads are not a
wallpaper playback success count.

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
SSIM plus RGBA MSE/max error. Twenty comparisons passed on llvmpipe with SSIM=1
and zero channel error, including TEXB0004 video updates/context replacement and
basic pointer parallax and all TEXS1/2/3 rectangular sprite versions. The harness exposed and now covers a translucent alpha
compositing bug. The ordinary suite tests the comparison gate; GPU cases require
an explicit command documented in the rendering interfaces.

The A0 behavior matrix now commits 13 standalone synthetic projects, their
SHA-256 identities and fixed inputs. The same eight renderable packages drive
all 20 comparisons, while five complete A1 candidate layouts exercise explicit
texture rejection/skipped draws. Regeneration is checked separately from
analytical PNG generation. See `tests/fixtures/scene-behavior/README.md` for
per-sample timing, properties, unused random seeds, pointer inputs, native-capture
protocol and uncovered work packages. Native acceptance of these synthetic
projects remains unverified; Wine/Proton availability alone does not establish it.

Completion: add legal authored L1/L2 samples with independently captured Wallpaper
Engine reference screenshots, fixed time/property/pointer inputs and recorded
thresholds; run on supported vendor GPUs and real niri sessions. Synthetic exact
matches do not complete the reference-screenshot fidelity or eight-hour soak exit
criteria in `plan.md`. Fonts, broader effects and authoring-format variation still
need reference sample coverage.

## Native TEX Reader Compatibility — Implemented / Advanced Layouts Pending

Evidence: `docs/reverse-engineering/wallpaper-engine-textures.md` records the
Wallpaper Engine 2.8.42 binary identity, REA Evidence IDs, assembly corroboration,
and exact local sample hashes. Corrected TEXB1 image count, TEXB3 field order,
TEXS NUL/version dispatch, image indices and six-component frame bases. Of 440
local loose textures, validated 2D payloads increased from 370 to 412; the remaining
28 volume/LUT textures now produce explicit unsupported errors. Synthetic tests
cover every TEXB1–4/TEXS1–3 combination and malformed records. GPU goldens cover
all three sprite versions without changing reference pixels.

Completion for remaining layouts: represent multiple atlas images and rotated
frame bases explicitly, add bounded 3D LUT parsing/upload, and verify reference
output before advertising support. The current parser rejects these layouts;
negative-direction horizontal/vertical bases also fail explicitly. A0 sample
construction exposed that flipped frames previously parsed successfully, then
were discarded by `sprite_animation`, silently leaving a static full atlas.
Regression coverage rejects both axes in TEXS1/2/3; flip rendering remains pending.
TEXB4 video/editor metadata still needs separate native/sample corroboration.
Particle atlases do not establish particle simulation support. Local pixel-size
validation and synthetic goldens do not establish Wallpaper Engine visual parity.
