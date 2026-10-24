# A1 completion: native atlas rendering

Date: 2026-10-24. Implementation commit: `d5a4ac1`,
`fix: match native sprite atlas sampling`; recorded in the
[adaptation plan](../wallpaper-engine-adaptation-plan.md). This completes A1's
bounded main-material 2D atlas task. It does not complete other adaptation stages.

## Change and native evidence

The prior implementation already represented independent images/mips, signed
frame bases and bounded animation, but used inferred animation, an inset and
crop-only base sampling for all materials. Fourteen independent native captures
showed that explicit `SPRITESHEET: 1` uses authored edges, TEX repeat/clamp and
point/linear flags, and authored mips even for packed frames. Those materials now
follow that contract. Explicit 0 disables animation; a missing combo preserves
existing inferred-animation results. Partial/noncontiguous authored mip chains
have a concrete shared admission error in the native path instead of silently
inventing mip levels or discarding authored sampling data.

The [REA evidence](../reverse-engineering/wallpaper-engine-atlas-uniforms.md)
connects TEX frame decoding, per-unit update storage, registered uniform IDs,
mapped constant-buffer writes, stage binding, shader consumption and image
selection. The same executable opened every owned reference project. Native
artwork/code is not distributed. The [fixture manifest and protocol](../../tests/fixtures/atlas-native/README.md)
record executable/input/reference hashes, fixed camera/viewport, absent external
inputs, native capture instructions and preselected comparison gates.

## Acceptance audit

| A1 requirement | Current evidence |
| --- | --- |
| Multiple images, independent dimensions/mip chains, rotation/reflection/skew and actual native UV consumption | Native multi-image, rotated, flipped-X/Y and skewed packages accepted; complete static uniform path plus fourteen original-program window captures. Multi-image probe has independent 2×2→1×1 and 1×2 chains. |
| Separate image lists/mips and retain image references/2D transforms | Existing bounded `TexAtlas` model and format tests; retained unchanged except explicit native sampling admission. |
| Shared resource parsing, GPU image/UV selection, count/byte/reference limits | Existing group upload/rollback/resident-image regressions remain green. Native materials select the same validated affine frames with their authored sampler. Decode/aggregate/frame/image bounds remain unchanged. |
| Compatibility analysis agrees and unsupported layouts are concrete | Package analysis uses the material's explicit enable/disable state and the same native sequence admission as rendering. Regression covers invalid timed frames under enabled/disabled materials; incomplete native mip chains have a named limitation. |
| Cross-image switching, rotation and edge sampling | Fourteen native capture comparisons, including repeat/clamp, point/linear, both flips, skew and packed-frame authored mip minification. |
| Loop, pause and context recreation | 120 additional native-reference GPU comparisons at times 0, 0.499999, 0.5, 0.5, 1, 0.5, with clear/reload and fresh EGL contexts, across all five animated layouts. Existing fractional-duration tests also pass. Pause here means repeated frozen scene time, matching the renderer interface. |
| Previous single-atlas results remain unchanged | All six pre-existing explicit GPU tests pass with original fixture/reference files untouched; the absent-combo path retains its prior UV/filter contract. |
| Actual niri presentation | Explicit live test submits all five native animated layouts on DP-1, 1920×1080, scale 1, Normal transform. Ninety accepted presentations cover repeated frozen time, reload and five backend/context recreations. |

## Results

Native-reference gates were fixed before capture: local SSIM ≥0.995, RGB MSE ≤1,
maximum channel difference ≤2. Because alpha is opaque, the test's equivalent
RGBA MSE limit is 0.75. No reference was generated from Better Wallpaper output.

| Renderer | Native comparisons | Minimum SSIM | Maximum RGBA MSE | Maximum channel error |
| --- | ---: | ---: | ---: | ---: |
| NVIDIA GeForce RTX 3060 Ti | 134 | 0.9991226055 | 0.1704711914 | 1 |
| Mesa llvmpipe, LLVM 23.1.1, 256 bits | 134 | 0.9984054409 | 0.1518859863 | 1 |

Both devices passed all eight explicit GPU tests. Mesa was selected through its
EGL vendor file, not inferred from `LIBGL_ALWAYS_SOFTWARE=1`. Native references
used Wine 11.20 built-in D3D11/DXGI with software GL requested on Xvfb; a separate
GLX query in the same display setup identified llvmpipe / Mesa 26.2.4. All fourteen
captures were independently repeated using the committed capture script; decoded
RGB pixels matched the retained references exactly.

The regression was also run with the old inferred sampling selected temporarily:
`clamped` failed (SSIM 0.9461532463, RGBA MSE 611.9600525, maximum error 83).
The fixed source was restored before final validation. Existing PNG gates were
not relaxed, and existing reference files were not modified.

Commands run from the repository root:

```sh
cargo clippy
cargo fmt --all -- --check
cargo test --workspace
cargo clippy -p better-wallpaper-scene-format -p better-wallpaper-renderer \
  -p gpu-renderer --all-targets -- -D warnings
cargo clippy -p better-wallpaper-daemon --test atlas_niri -- -D warnings
python3 tests/fixtures/atlas-native/generate.py
cargo test -p gpu-renderer --test scene_golden -- --ignored --test-threads=1 --nocapture
__EGL_VENDOR_LIBRARY_FILENAMES=/usr/share/glvnd/egl_vendor.d/50_mesa.json \
  LIBGL_ALWAYS_SOFTWARE=1 cargo test -p gpu-renderer --test scene_golden \
  -- --ignored --test-threads=1 --nocapture
BETTER_WALLPAPER_NIRI_TEST_OUTPUT=DP-1 cargo test -p better-wallpaper-daemon \
  --test atlas_niri -- --ignored --nocapture
```

Required Clippy, formatting, targeted strict Clippy, fixture reproduction and
workspace tests pass. Workspace total: **249 passed, 0 failed, 9 ignored**; the
nine ignored tests are the eight explicitly run GPU cases and the explicitly run
live niri case. Native input/reference hashes are checked by an ordinary CPU test.
Local logs are retained under `/tmp/a1-native/` (`workspace.log`, `clippy.log`,
`clippy-strict.log`, `clippy-desktop.log`, `gpu-nvidia.log`, `gpu-mesa.log`,
`niri-final.log`, `before-fix.log`, `recapture.log`). Logs and commercial artifacts
are not committed; the protocol, capture hashes and assertions are.

## Limits kept explicit

Native CLI pause did not freeze the Wine test window, so it is not evidence of
native pause semantics or exact native clock synchronization. Its animated
multi-image scene did visibly loop through both images. The native pixel probes
avoid clock ambiguity through single-frame variants; Better Wallpaper pause,
loop and reset are tested with controlled scene times. Long injected times in
existing tests are not an eight-hour wall-clock soak.

The live desktop test proves successful backend submissions, layer/configure
handling and actual context replacement on one output. It does not assert pixel
readback from the compositor, visual visibility through other wallpaper layers,
physical display color correctness, hotplug, resize or compositor restart. Those
broader desktop/A9 checks remain separate. NVIDIA and software Mesa are two
rendering implementations, not coverage of every vendor GPU.

Effect-mask and system-fallback textures retain the legacy adapter; Plasma still
has no shared scene consumer. Arbitrary shaders/effects, particles, volume/LUTs,
partial native mip chains and broader alpha/color-space fidelity are outside the
verified atlas subset. Missing-combo inference remains an intentional compatibility
fallback. These limits are recorded in the rendering skill; no general Wallpaper
Engine parity or A9 completion is implied.
