# A2 bounded LUT rendering acceptance

Date: 2026-10-25. Implementation and completion commits are recorded in the
[adaptation plan](../wallpaper-engine-adaptation-plan.md). This completes A2's
bounded raw RGBA8/PNG, base-only, LDR LUT operation; it does not complete A4's
general effect fidelity, A8's Plasma consumer or A9's broader release gates.

## Delivered behavior

`ColorLut::parse` admits a stricter subset of `TexVolume`: exactly one base mip,
axes 2–128, raw ARGB8888/PNG, point/linear and clamp/repeat flags only, at most
8 MiB retained bytes. Raw format-zero and PNG bytes are used as RGBA8 without a
transfer conversion. All broader volume parser limits remain in force.

`resolve_color_lut` verifies package effect/material/instance bindings rather
than trusting effect filenames. It admits one visible effect and one pass using
the known `ccsimple` LUT branch, texture slot one and static `lutparams` in 0–1.
COL/HDR, unknown combos/constants, extra passes/targets, arbitrary fragment or
vertex overrides and unsafe paths have explicit diagnostics. The owned MVP
vertex adapter used for native probes is the only admitted package vertex
override. Ordinary image-model draws use this binding; text, direct video,
procedural solids and system fallback branches retain their existing limits.
An image material containing encoded video can use the same draw-level operation;
native dynamic-video LUT fidelity is outside these static pixel probes.

Shared CPU assets deduplicate LUT pixels by package path, with a 32 MiB retained
LUT budget separate from the existing 512 MiB main-TEX budget. One bounded 8 MiB
decode may coexist while admission checks the aggregate. Per-context GPU objects
use vertically stacked slices and explicit 3D voxel interpolation/addressing.
They require no optional 3D-texture capability. Oversized stacks return the actual
2D device limit instead of resizing or discarding slices. The combined resident
image/LUT count is 256. Failed uploads remain retryable; a missing required LUT
blocks drawing. Clear/drop deletes LUTs in the owning current context.

Package-aware compatibility adds L3 effect metadata only after binding and LUT
layout admission. A parsed or unused volume alone does not raise the tier.
Malformed payloads keep the unsupported effect and concrete texture error.
Metadata still does not prove GPU/backend availability or whole-scene playback.
Standalone `scene-inspect --volume` remains structural inspection: its generic
playback/native-validation fields do not claim validated effect binding for the
arbitrary file being inspected.

## Native evidence and neutral criterion

The [evidence record](../reverse-engineering/wallpaper-engine-luts.md) retains
target/shader identities, REA IDs and limits. Eleven owned packages were captured
with the unmodified native `ccsimple` fragment shader and an owned image-layer
vertex adapter; they are not screenshots of the application's global filter UI.
Every reference was independently recaptured with identical decoded RGB.
The [fixture protocol](../../tests/fixtures/lut-native/README.md) records fixed
camera/input, all 33 input/reference hashes, and gates chosen before capture.

The original neutral 32³ cube is not an exact mathematical identity under native
normalized-edge sampling: its largest byte difference from the ungraded control
is 4, mean absolute RGB difference 1.875. The neutral capture is matched exactly
on both GPU implementations. Strength zero matches the ungraded control exactly.
The plan's neutral acceptance wording is corrected to this measured behavior;
no native gate was relaxed or reference edited to conceal the difference.

An initial linear-repeat comparison failed with maximum error 6 because default
GLES sampler precision perturbed input RGB before the steep wrap boundary.
Explicit high-precision source/LUT samplers fixed it; final repeat error is at
most 1. No new half-texel color correction or linear-light conversion was added.

## Results

| Check | Result |
| --- | --- |
| `cargo clippy`; format; `git diff --check` | Passed |
| Format, shared renderer and GPU crates, all-target strict Clippy | Passed |
| New live niri test, strict Clippy | Passed |
| Full workspace with local filesystem access | 272 passed, 0 failed, 12 default-ignored |
| Explicit GPU suite, Mesa llvmpipe LLVM 23.1.1 | All 9 passed |
| Explicit GPU suite, NVIDIA RTX 3060 Ti | All 9 passed |
| Eleven native LUT references | 176 timed/lifecycle comparisons per renderer, plus four recovery/residency pixel checks |
| Selected local assets | 28/28 exact A0/A2 input hashes, LUT admission, upload and draw in each of two fresh contexts on each renderer |
| Live niri DP-1, 1920×1080, scale 1, normal transform | Five LUT layouts, 90 successful submissions, reload and five backend/context replacements |
| Owned input regeneration | All 22 package/project files reproduce byte-for-byte |
| Independent native recapture | All 11 decoded RGB references reproduce exactly |

| Renderer | Native comparisons | Minimum SSIM | Maximum RGBA MSE | Maximum channel error |
| --- | ---: | ---: | ---: | ---: |
| llvmpipe | 176 | 0.9997939768074552 | 0.1572265625 | 1 |
| NVIDIA RTX 3060 Ti | 176 | 0.9988192065205551 | 0.1904296875 | 1 |

The twelve ordinary ignored cases comprise nine explicit GPU cases, two live niri
cases and an existing GTK display case. The nine GPU cases and new LUT niri case
were run explicitly for this change. The existing atlas niri and GTK display
cases were not rerun. Existing atlas/native captures and analytical PNGs were
unchanged, and all existing GPU cases passed on both devices.

The restricted workspace run encountered the existing daemon filesystem/symlink
permission failures; the local-access run passed. A later concurrent validation
run hit the existing `config_update_interrupts_retry_and_uses_latest_config`
channel-disconnect race. Its isolated rerun and the sequential full final run
passed; this change does not modify that supervisor/test. These environmental
and timing failures are retained separately from the final passing totals.

Local logs: `/tmp/a2-workspace-final-sequential.log`, `/tmp/a2-gpu-final-mesa.log`,
`/tmp/a2-gpu-final-nvidia.log`, `/tmp/a2-niri.log`, `/tmp/a2-clippy-final.log`,
`/tmp/a2-clippy-strict.log`, `/tmp/a2-clippy-niri.log`, plus rejected/recaptured
native experiments under `/tmp/a2-lut-native/`. Commercial resources and complete
decompiler output are not committed.

## Reproduce

```sh
cargo clippy
cargo fmt --all -- --check
cargo test --workspace
cargo clippy -p better-wallpaper-scene-format -p better-wallpaper-renderer \
  -p gpu-renderer --all-targets -- -D warnings
cargo clippy -p better-wallpaper-daemon --test lut_niri -- -D warnings
python3 tests/fixtures/lut-native/generate.py
cargo test -p gpu-renderer --test scene_golden -- --ignored --test-threads=1 --nocapture
__EGL_VENDOR_LIBRARY_FILENAMES=/usr/share/glvnd/egl_vendor.d/50_mesa.json \
  LIBGL_ALWAYS_SOFTWARE=1 cargo test -p gpu-renderer --test scene_golden \
  -- --ignored --test-threads=1 --nocapture
BETTER_WALLPAPER_NIRI_TEST_OUTPUT=DP-1 cargo test -p better-wallpaper-daemon \
  --test lut_niri -- --ignored --nocapture
```

Optional `BETTER_WALLPAPER_LUT_INSTALLATION=/absolute/selected/installation` on
either GPU command enables the 28 hash-checked local resource checks. It does
not create 28 native pixel parity claims. Use the renderer name printed by the
harness as device evidence; `LIBGL_ALWAYS_SOFTWARE=1` alone does not select Mesa.

The niri test proves accepted live submission/context lifecycle, not compositor
pixel readback, physical color accuracy, multi-output hotplug or restart behavior.
Arbitrary shader/effect stacks, app-global filter settings, source minification
and multipass resampling parity, HDR/COL, animated property parameters, other
volume codecs/mips and Plasma playback remain outside the bounded A2 subset.
Long injected scene times are not an eight-hour wall-clock soak or A9 acceptance.
