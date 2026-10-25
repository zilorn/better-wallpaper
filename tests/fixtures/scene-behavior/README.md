# Deterministic scene behavior samples

These 13 independently authored scene projects are inputs for the A0 adaptation
baseline and A1 atlas regressions. Each directory contains `project.json` and `scene.pkg`; the package has
the scene, models, materials and original TEX pixels. No Wallpaper Engine binary
or commercial artwork is included. Native acceptance of these original projects
has not been tested. A3 corrected the synthetic video container to the verified
native TEXB4 layout: flag `0x20`, raw codec and zero editor records. Separate
[metadata fixtures](../texb4/README.md) cover editor records without implying
original-program visual or clock equivalence.

[`matrix.json`](matrix.json) is consumed by
[`scene_golden.rs`](../../../crates/gpu-renderer/tests/scene_golden.rs). It records
package/project SHA-256 and byte counts, canvas/output dimensions, ordered times,
pointer inputs, video actions, parser outcomes and analytical PNG hashes. Every
sample has empty properties, no random operators (`random_seed: null`) and
`native_reference: pending`. The existing analytical PNGs are separate from these
input packages and are never generated from renderer output.

## Reproduction

Run from the repository root:

```sh
# Read-only check against all 27 committed generated files.
python3 tests/fixtures/scene-behavior/generate.py

# Deliberate input refresh. Does not edit analytical PNGs.
python3 tests/fixtures/scene-behavior/generate.py --write

# Ordinary hash, input-contract, asset-resolution and rejection checks.
cargo test -p gpu-renderer --test scene_golden

# Read-only check of the seven independent analytical A1 references.
python3 crates/gpu-renderer/tests/golden/generate_atlas_references.py

# Explicit GPU checks against the packages and ordered matrix inputs.
LIBGL_ALWAYS_SOFTWARE=1 cargo test -p gpu-renderer --test scene_golden -- --ignored --test-threads=1 --nocapture
```

The generator uses Python's standard library, the parent directory's original
`scene-video.mp4` and existing PNG hashes. It does not launch either application
or fetch assets. Output is fixed to this fixture directory. Hash changes are
reviewed input changes, not evidence of visual correctness.

## Renderable baseline

All scene canvases are 32×32. Default output is 32×32; `cover` uses 64×32. Each
row starts with a fresh renderer. Steps within a row share assets and state;
the first draw uses the explicit time below. Pointer coordinates are normalized,
Y-up, in [-1,1]; null means no pointer. No property overrides, audio input or
wall-clock values are used.

| Sample | Ordered fixed inputs | Analytical expectation |
| --- | --- | --- |
| `layers` | t=0 at 32×32; t=0 at 64×32; t=1 with pointer (1,1); t=1 with pointer (-1,-1) | Parent translation, radians rotation, source-over/additive layers; cover crop; separate X/Y parallax; unchanged time freezes pointer displacement |
| `opacity` | t=2, no pointer | Opacity reaches 1 at frame 60 with 30 FPS, single mode |
| `zoom` | t=2, no pointer | Camera zoom reaches 2 at frame 60 with 30 FPS, single mode |
| `sprite-v1` | t=0, 0.5, 1; no pointer | Red, green, red; TEXS1 signed integer bases |
| `sprite-v2` | t=0, 0.5, 1; no pointer | Same colors/loop; TEXS2 float bases |
| `sprite-v3` | t=0, 0.5, 1; no pointer | Same colors/loop; TEXS3 logical dimensions and float bases |
| `padding` | t=0, no pointer | Four asymmetric quadrants retain orientation; unused magenta storage is absent |
| `video` | Decode at t=0 and 0.5; replace CPU frame at t=1; recreate GL context at t=1 | Red then green; replacing 16×16 with solid green 8×8 reallocates storage; retained pixels upload into the replacement context |

The last two video steps are consumer-lifecycle probes, not source movie frames.
Native screenshot comparison must use the first two steps only; context/storage
replacement needs a separate native lifecycle experiment. The movie's actual
third frame is blue. The three layered samples have three draws and one
non-drawable parent container: `skipped_nodes: 1` counts that container, not a
missing layer. Other renderable samples have one draw and no skipped node.

These baseline rows retain 20 comparisons against ten independent PNGs. All require local 8×8
luminance SSIM >= 0.995, RGBA MSE <= 1 and maximum channel error <= 2. The test
checks that recorded gates match the comparator. Failure artifacts and reference
generation remain documented in the
[golden README](../../../crates/gpu-renderer/tests/golden/README.md).

## A1 atlas samples and legacy adapter probes

These complete synthetic layouts now render through the shared `TexAtlas` scene
path: one draw, no skipped nodes. Both frames last 0.5 seconds. Each sample uses
six ordered steps: t=0, 0.499999, 0.5, 0.5 again (fixed-time pause), 1 (loop), then
0.5 after recreating the EGL context. There is no pointer, no property override
and no random input. The legacy `TexTexture` adapter still rejects the layouts;
the matrix's `legacy_rejection` field checks those diagnostics separately.
Neither path establishes original-program sample acceptance or UV parity.

| Sample | Texture input | Legacy adapter rejection |
| --- | --- | --- |
| `multi-image` | Image 0: 2×2 plus 1×1 mip; image 1: one 1×2 mip; frames select images 0 then 1 | Multiple atlas images |
| `rotated` | Asymmetric 2×2 pixels; basis (origin 2,0; horizontal 0,2; vertical -2,0), then identity | Rotated or skewed frame |
| `skewed` | Same pixels; basis (0,0; 1,0; 1,2), then identity | Rotated or skewed frame |
| `flipped-x` | Same pixels; basis (2,0; -2,0; 0,2), then identity | Flipped frame |
| `flipped-y` | Same pixels; basis (0,2; 2,0; 0,-2), then identity | Flipped frame |

Both multi-image mip chains are fully encoded independently and normalized to
their own image dimensions. Seven new analytical PNGs describe the two uniform
image colors, identity and four affine transforms. The asymmetric 2×2 texture
uses bilinear interpolation at explicit pixel centers. A half-texel inset along
each authored basis specifies Better Wallpaper sampling; the skew sample's
one-texel horizontal axis samples its center. This is not a native edge/filtering
claim. The generator has explicit coordinate permutations for rotation/flips and
does not read renderer output.

The full matrix now contains 50 comparisons against 17 analytical PNGs. A separate
GPU case adds five comparisons for failed-upload retry, preserving resident
atlases at the image-count limit, clear/reload, and composing a reverse logical
quarter-turn effect with an atlas rotation. These five are consumer probes, not
native-capture steps. Existing packages and ten baseline PNGs are unchanged.
Format-library tests retain the legacy failures above and verify shared sprite
admission, which rejects an entire malformed sequence rather than dropping frames.
The
[atlas report](../../../docs/reverse-engineering/wallpaper-engine-atlases.md)
records static evidence; the [rendering record](../../../docs/validation/a1-atlas-rendering.md)
records current behavior, verification and remaining native/desktop work.

## Original-program capture protocol and pending coverage

No original-program screenshots are present. A read-only host check on
2026-10-18 found Arch Linux/Wayland, Wine 11.20, Proton Experimental, Xvfb and an
existing Steam app 431960 compatibility prefix. No matching Wallpaper Engine/Wine
process was observed then. This establishes tool/prefix availability only. This
work did not launch the original program, establish sample acceptance, or test
deterministic scene-time control; it does not establish that this host cannot
run the program.

For a future controlled capture:

1. Record executable hash/version, OS, exact native Windows or Wine/Proton
   runtime, GPU/driver, color settings and capture method. Match the installation
   identity in the [native report](../../../docs/reverse-engineering/wallpaper-engine-textures.md).
2. Use an isolated test display/profile. Verify project loading and shader
   resolution. Record original errors if loading fails; no screenshot is a
   reference before this check.
3. Establish scene-time control first, then replay the ordered inputs, including
   pause and pointer direction. Sleeping after launch does not establish exact
   scene time. Record frame identity and timing uncertainty at sprite boundaries.
4. Capture the content rectangle losslessly at the specified pixel dimensions,
   without scaling, cursor overlays or UI chrome. Retain raw captures, crop
   coordinates, alpha handling, timestamps and hashes. If the engine cannot
   render at this size, author a separate larger sample and independent
   expectation before capture; do not silently resize the image.
5. Pre-register the numerical gates above for these exact raster probes. State
   RGB/alpha handling explicitly; an opaque desktop capture may lack native
   alpha. Keep native references in a separate capture manifest and retain
   failure/difference images. Do not overwrite analytical PNGs, loosen gates
   after viewing results, or label Better Wallpaper output a native reference.

This covers the shared 2D golden baseline and A1 synthetic atlas rendering. A2
now has eleven owned LUT effect probes and native captures in
[`../lut-native`](../lut-native/README.md); A3 has [reader/sample evidence](../../../docs/reverse-engineering/wallpaper-engine-texb4.md)
and shared-clock lifecycle checks. A4 needs an observed
effect mismatch and broader motion coverage. A5 web callbacks, A6 particles,
A7 SceneScript and A8 Plasma consumers need their own defined subsets/fixtures.
The [decoder fixtures](../README.md) cover A0 decoding separately. These gaps
keep the all-behavior sample and original-reference plan items open.
