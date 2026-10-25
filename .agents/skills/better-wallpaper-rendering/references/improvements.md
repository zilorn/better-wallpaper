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
`effects/fluidsimulation/effect.json`). Audit-only normalization now accepts line
comments, non-nested block comments and trailing commas, while preserving strings,
original byte identities, error locations and resource limits. Strict/extended
success statuses remain distinct; malformed documents emit no partial references.
Two scans of the same 2,593 input identities resolve all 32 earlier syntax failures
and add 302 candidate references (115 found, 187 outside inventory scope).
See the [JSON validation delta](../../../../docs/validation/a0-json-extensions.md).
Runtime parsers are unchanged. Complete broader reference discovery only after
testing additional fields and lookup rules against samples; do not silently treat
unknown dependencies as resolved or absent from the installation.
The retained [A0 inventory report](../../../../docs/validation/a0-asset-inventory.md)
records 2,593 assets/container entries and the historical 3,858 candidate references.
Its machine-readable manifest preserves the original failures; the separate delta
records 4,160 candidates and complete coverage of the selected field subset.
Neither completeness nor 459 size-validated TEX payloads is a playback success count.

## Basic Mouse Parallax — Implemented / Desktop Verification Needed

Evidence: scene IR parses bounded camera parameters and per-axis layer depth;
`SceneParallaxState` and `scene_parallax_offset` share scene-clock smoothing and
translation. `SceneGpuRenderer::draw_scene_with_pointer` applies the displacement.
The niri seat/pointer handlers forward only current wallpaper-surface events and
clear pointer state on leave/removal. The daemon gates it with `scene.mouse` and
`scene.parallax`; the GTK UI enables these controls with a niri limitation label.
Regression tests cover the native per-frame recurrence, pause, recentering, depth axes,
invalid parameters, disabled input and stale surfaces after hotplug.

Completion: verify actual pointer-driven playback in niri with multiple outputs,
resize and compositor recovery, and compare authored sample motion. Standard
Wayland does not provide other windows' global pointer positions; a supported
compositor input source is required before claiming desktop-wide mouse tracking.
Depth-map parallax and arbitrary mouse scripts remain unsupported.

## Bounded 2D Particles — Implemented; Broader Operators Excluded

A6 implements strict box emission, fixed lifetime, initialized XY motion/size/color
and static sprite submission. The shared scene budget is enforced and exposed in
GTK. Admission and package-aware compatibility use the same checks and preserve
specific limitations for mixed scenes. See `docs/validation/a6-particle-subset.md`
and `docs/validation/a6-particles.md` for evidence and validation.

Variable lifetimes, gravity/drag, alphafade, sphere emitters, atlas/random-frame
animation, trails, events/children and scripts remain excluded. Native RNG stream
identity and general maxcount scheduling are not reproduced. Extend an operator
only with an owned fixed-input sample, native semantic evidence, bounded CPU/GPU
behavior and visible rejection of its unverified variants. Plasma still needs A8.

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

The behavior matrix commits 13 standalone synthetic projects, their SHA-256
identities and fixed inputs. A1 now renders all thirteen packages in 50 ordered
steps, with seven new analytical references for independent images, rotation,
flips and skew. The original 20 comparisons remain exact; a separate GPU case
adds five comparisons for upload rollback, image limits, reload and effect
composition. All 55 passed on llvmpipe (maximum byte error 1); see the
[A1 rendering record](../../../../docs/validation/a1-atlas-rendering.md).
The five advanced layouts still exercise explicit legacy-adapter rejection.
Regeneration is checked separately from
analytical PNG generation. See `tests/fixtures/scene-behavior/README.md` for
per-sample timing, properties, unused random seeds, pointer inputs, native-capture
protocol and uncovered work packages. These older missing-combo projects are analytical regressions. Explicit-sprite
variants now have native acceptance and fourteen original-program captures in
`tests/fixtures/atlas-native`; camera and material inputs are fixed separately.

A1 now supplies owned atlas references, preselected gates, 134 comparisons on
each of NVIDIA and llvmpipe, and real niri submissions; see the
[native acceptance record](../../../../docs/validation/a1-native-atlases.md).
Completion of broader fidelity: add native fonts/effects/authoring-format samples
and other vendor GPUs. Neither these atlas comparisons nor the desktop submission
probe complete the eight-hour soak exit criterion in `plan.md`.

## Native TEX Reader and 2D Atlases — A1 Bounded Subset Validated

Fractional frame timing now accumulates the loop period in `f64`, matching the
scene clock. Previously 10,000 stored 0.1-second durations produced a period
97.121835 ms too short and skipped most of the last frame. CPU regressions cover
uniform/mixed durations and repeated loops; 24 analytical GPU comparisons cover
image selection, pause, rewind, clear/reload and context replacement. This is a
local timing correction, not native-engine parity or a wall-clock soak result.
See [validation](../../../../docs/validation/a1-atlas-timing.md).

Packed rectangular sprite frames now avoid whole-image mip sampling, which can
mix neighboring frames during minification. Coverage classification preserves
mips for full-image frames and restores filtering across frame/effect bindings.
`tests/support/atlas_sampling.rs` in the GPU crate checks solid-color crops at
32/8/4/1 pixels, rotations/flips, independent authored mips, reload and context
replacement. This remains the inferred legacy path. Explicit sprite materials now use
native authored UVs and sampler/mip data, verified against original-program
captures including packed-frame minification; arbitrary skewed-edge isolation is
not promised by native affine sampling.

Generated odd-sized mip filtering now weights fractional texel coverage instead
of partitioning the source into unequal whole-texel bins. A 5×1 center impulse
previously produced asymmetric 0/83 red values instead of 50/50; regressions cover
both axes, all RGBA channels, the 5×5 chain and GPU minification with authored-mip
precedence. This fixes a local sampling defect; native and packed-edge parity
remain pending. See [validation](../../../../docs/validation/a1-odd-mip-filtering.md).

Evidence: `docs/reverse-engineering/wallpaper-engine-textures.md` records the
Wallpaper Engine 2.8.42 binary identity, REA Evidence IDs, assembly corroboration,
and exact local sample hashes. Corrected TEXB1 image count, TEXB3 field order,
TEXS NUL/version dispatch, image indices and six-component frame bases. Of 440
local loose textures, validated 2D payloads increased from 370 to 412; the remaining
28 volume/LUT textures now produce explicit unsupported errors. Synthetic tests
cover every TEXB1–4/TEXS1–3 combination and malformed records. GPU goldens cover
all three sprite versions without changing reference pixels.

The A1 `TexAtlas` model represents independent image/mip chains,
frame image indices and full signed affine bases with count/byte/finite-value
bounds. Its unit and CLI tests cover the five A0 layouts and retain legacy-adapter
rejection. Main scene material textures now use shared sprite admission and
complete per-path GPU image groups. Rotation, flips and finite in-bounds skew
reach affine vertex/effect UV submission, with whole-sequence failure reporting,
512 MiB retained main-TEX budget and a 256 resident GPU-image cap. Golden tests
cover selection, loop/pause, clear/reload and context replacement. The
[atlas report](../../../../docs/reverse-engineering/wallpaper-engine-atlases.md)
records native frame-update/image-binding assembly and shipped shader evidence;
the follow-up [uniform/sampler report](../../../../docs/reverse-engineering/wallpaper-engine-atlas-uniforms.md)
closes the mapped-buffer/stage-binding chain and native sample acceptance.

A1 completion: fourteen original-program captures cover independent images/mips,
rotation, both flips, skew, repeat/clamp, point/linear and packed authored-mip
minification. Both NVIDIA and Mesa pass 134 comparisons plus prior regressions;
real niri completes 90 submissions with reload and five context replacements.
Explicit `SPRITESHEET` 0/1 is respected by rendering and compatibility analysis;
missing-combo inference retains old results. Native partial/noncontiguous mip
chains have a concrete admission error. See the [acceptance audit](../../../../docs/validation/a1-native-atlases.md).

Remaining A1 extensions: broaden effect/alpha/color-space and vendor coverage,
implement partial native mip sampling without inventing levels, and test desktop
hotplug/compositor restart separately. Effect masks and authored system-texture
fallbacks retain the legacy single-image adapter. Volume parsing is now separate;
LUT upload/effect consumption remains pending. A0 sample
construction exposed that flipped frames previously parsed successfully, then
were discarded by `sprite_animation`, silently leaving a static full atlas.
Legacy regression coverage still rejects both axes in TEXS1/2/3; the new main
material path renders the validated affine sequence instead.
TEXB4 video/editor metadata now has the bounded A3 reader/sample evidence below.
Particle atlases do not establish particle simulation support. Local pixel-size
validation and synthetic goldens do not establish Wallpaper Engine visual parity.

## Volume TEX / 3D LUT — Bounded A2 Subset Complete

The earlier TexVolume reader remains independently inspectable, with bounded
raw ARGB8888/PNG and LZ4 decoding, mip/depth/byte checks and explicit exclusions.
All 28 selected local PNG LUTs retain A0 input identities and independently checked
voxel hashes. ColorLut now admits a stricter base-only 2–128-axis, 8 MiB LDR subset
for package-verified ccsimple effects; raw volumes do not become 2D image textures.

Native target/shader evidence and eleven owned captures establish normalized
coordinate, byte-RGB, interpolation, channel, alpha and parameter semantics.
The normal neutral 32³ cube is not a mathematical identity under native edge
sampling (max bias 4); strength zero is exact and native neutral output is matched
exactly. Explicit high-precision samplers fix the measured repeat-boundary error.
Both NVIDIA and llvmpipe pass the full nine GPU cases, 176 native LUT comparisons
each plus failure/recovery/residency checks, and upload/draw all 28 selected LUTs.
Live niri covers 90 submissions and five context replacements. See
[evidence](../../../../docs/reverse-engineering/wallpaper-engine-luts.md) and
[acceptance](../../../../docs/validation/a2-lut-rendering.md).

Broader work remains: app-global filter settings, COL/HDR, animated/property-bound
parameters, general effect stacks and minification/resampling parity, other volume
formats/codecs/mips and Plasma consumption. They require separate native probes
and concrete admission/visual/lifecycle criteria. The completed bounded A2 scope
does not imply arbitrary shaders, full effect parity or A9 soak/release completion.

## TEXB4 Video and Editor Metadata — A3 Bounded Contract Validated

Native assembly identifies the TEXB4 extra word as an editor-record count;
TEXI flag 0x20 identifies video independently. The parser retains bounded records
before images and consumes empty per-mip variant tables after payloads. It no
longer invents an MP4 codec or treats metadata count one as video. Nonempty
conditional variants and volume editor metadata are explicit exclusions.

Real local observations cover six TEXB3/five TEXB4 videos: nine exact payloads
decode at 720p, while two exceed the preserved 64 MiB TEX mip ceiling. The 440
loose TEX inventory retains 412 valid 2D payloads and 28 volume exclusions. Owned
metadata/static/video samples, all truncation prefixes, length/count/byte budgets,
LZ4/version differences and PNG mip regressions cover parser boundaries.

Daemon checks cover shared scene times, pause/loops, three reloads, decoder
sharing/limit, partial startup rollback, private extracted-file cleanup and byte
ownership release. Existing file-only FFmpeg container/protocol restrictions and
64 MiB RGBA-frame/128 MiB runtime-input limits are preserved. Both available
renderers pass all nine explicit GPU cases, including corrected video inputs,
storage resize and context replacement. See [evidence](../../../../docs/reverse-engineering/wallpaper-engine-texb4.md)
and [validation](../../../../docs/validation/a3-texb4.md).

Remaining extensions require their own samples/consumer: conditional editor mip
selection, volume metadata, larger media budgets, native video screenshot/clock
equivalence, video-texture audio, Plasma scene playback and A9 desktop/soak gates.
No arbitrary editor/property/script execution is implied by retaining metadata.

## A4 Tint — Bounded Native Operation Validated

The selected 30-package occurrence survey finds 83 authored Tint instances
(31 default-enabled after bound visibility), all unmasked. One visible
package-verified LDR pass now supports modes 0/2/7/30, properties, linear alpha
animation and bounded point/clamp red masks. Twelve owned original-program
captures repeat exactly; final 184 comparisons on NVIDIA and llvmpipe pass with
max error one, including fractional alpha and missing-mask recovery.
Linear-mask edges differ by up to five bytes and remain explicitly rejected;
no threshold was relaxed. Other blend modes, stacks, HDR, curves/scripts and
resampling/minification need separate evidence. See [A4 Tint](../../../../docs/validation/a4-tint.md);
this does not imply complete effect fidelity, Plasma scenes or an A9 soak.

## A4 Motion and Parallax — Bounded Native Contract Complete

Owned native probes establish additive XY origin/scale channels (including saved
values and empty axes), absolute scalar channels, parallax sign/amount/influence,
native defaults, projection/cover/zoom units and the capped per-frame smoothing
formula. Shared motion plans now apply autonomous linear XY timelines on scene
time with hierarchy/finite-value bounds. Unsupported curves, scripts, schedules,
wrap loops, other channels and mesh consumers retain concrete diagnostics.
Eighteen native images repeat exactly; both renderers pass 230 new motion
comparisons, the prior eleven GPU cases and 108 single-output niri submissions.
See [A4 acceptance](../../../../docs/validation/a4-motion.md).

Remaining extensions need separate evidence: native fractional-clock capture,
linear-mask resampling, broader effect ordering/blend modes/HDR, Bezier/event
animation, mesh motion and actual multi-output pointer/lifecycle validation.
The selected real script/Bezier timelines remain excluded; A7/A8/A9 are unchanged.
