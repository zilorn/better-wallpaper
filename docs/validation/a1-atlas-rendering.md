# A1 shared atlas rendering validation

Date: 2026-10-18. Implementation commit: `3152a70`. This batch connects the bounded atlas model to main scene
material resources and shared GPU rendering. Multiple images with independent
mips, rotation, reflection and finite in-bounds skew now render. This is verified
synthetic behavior, not completed original-program or desktop parity.

## Resulting behavior

`TexAtlas::sprite_animation` supplies one admission rule for resource resolution
and package-aware compatibility analysis. It keeps every frame's image index,
duration and full affine basis. It rejects the whole sequence when any frame has
a nonpositive/nonfinite duration, a missing image, an out-of-bounds corner,
basis length below one texel or a determinant magnitude below 0.0001. An animated
or multi-image texture requires a nonempty sequence. One valid frame is a fixed
transformed crop; at least two identify sprite animation for L2 metadata.
`SceneMetadata.unsupported_textures` carries concrete parse/layout errors into
the compatibility report, rather than silently omitting invalid frames.

The renderer retains image zero in `Scene2dDraw.texture` and later images in
`additional_textures`. Repeated material references share decoded bytes. Main
material TEX data has a 512 MiB aggregate retained-payload budget per resolution,
in addition to 64 images, 32 mips/image, 10,000 frames and 128 MiB per atlas.
This aggregate excludes direct media, fonts and secondary/fallback textures;
temporary decoding buffers and GPU conversion also consume memory separately.

The GPU cache holds an entire image group per path. A failed image upload deletes
earlier images from that attempt and admits no partial group. The existing
256-texture cap now counts images individually and returns a capacity error
instead of evicting an arbitrary image from active content. Clear/drop releases
all images. The supplied scene clock selects the image and UV together, including
after context replacement. Quad and puppet coordinates and logical effect
displacements use the full two-column basis.

Sampling uses a half-texel inset along each authored basis. This preserves the
old rectangular convention and makes a one-texel axis sample its center. Native
filtering, packed-atlas minification and boundary equivalence are still unknown;
this policy is explicitly an implementation convention. Atlas-backed effect
masks and authored system-texture fallbacks still use the legacy single-image
adapter. No Plasma scene consumer or desktop lifecycle behavior was added.

`scene-inspect --atlas --json` now distinguishes the legacy `single_image_layout`
result from shared `sprite_layout` admission. These are CPU checks, not successful
GPU upload. Single video metadata remains unverified in this report; volume
textures and multi-video layouts remain unsupported.

## Reproducible samples and checks

The [behavior matrix](../../tests/fixtures/scene-behavior/README.md) now renders
all 13 existing packages in 50 ordered steps. The original 26 project/package
artifacts and ten baseline PNGs are byte-for-byte unchanged. Seven new independent
analytical PNGs cover the two atlas image colors, identity, rotation, skew and
both flips. Their generator never reads renderer output. The 0.995 SSIM, 1.0 RGBA
MSE and 2-byte maximum error gates were retained without relaxation.

Each new atlas sample runs at t=0, 0.499999, 0.5, 0.5 again, 1, then 0.5 after
context recreation. Repeated time checks pause without introducing wall time;
the final step also rewinds the clock. The multi-image sample verifies different
image dimensions and mip counts. Legacy adapter rejection is checked separately.

A second GPU test adds five comparisons: retry after the second image fails to
upload, both frames after the global image limit is reached, clear/reload, and a
reverse logical quarter-turn effect that cancels the atlas rotation. This last
case exercises the fragment shader's affine displacement, not just vertex UVs.
Ordinary tests check shared image storage, complete-sequence rejection, metadata
diagnostics, finite/bounds rules, one-frame crops and rectangular inset retention.

| Command | Result |
| --- | --- |
| `cargo test --workspace` | 234 passed, 0 failed; 2 explicit GPU cases ignored in this invocation |
| `LIBGL_ALWAYS_SOFTWARE=1 cargo test -p gpu-renderer --test scene_golden -- --ignored --test-threads=1 --nocapture` | Both GPU cases passed; 55 comparisons passed |
| `cargo clippy` | Passed |
| `cargo clippy -p better-wallpaper-scene-format -p better-wallpaper-renderer -p gpu-renderer --all-targets -- -D warnings` | All changed crates passed |
| `cargo fmt --all -- --check` | Passed |
| `python3 tests/fixtures/scene-behavior/generate.py` | 27 generated input/manifest files reproduced exactly |
| `python3 crates/gpu-renderer/tests/golden/generate_atlas_references.py` | Seven analytical PNGs reproduced exactly |
| `git diff --check`, changed Markdown links and unchanged baseline artifact checks | Passed |

The full suite used local GPU and temporary socket/symlink access required by
existing tests. GPU readback used surfaceless EGL/GLES on llvmpipe (LLVM 23.1.1,
256 bits). All original 20 comparisons remained exact. Across all 55, minimum
SSIM was 0.9990804491739541, maximum MSE 0.171875 and maximum channel error 1.
Transient logs are `/tmp/a1-render-workspace.log`, `/tmp/a1-render-gpu.log` and
`/tmp/a1-render-changed-clippy.log`; this record retains results independently.

An additional strict Clippy run over every workspace test target encountered the
pre-existing `items_after_test_module` lint in
`better-wallpaper-daemon/src/web_wallpaper.rs:258` (a later `run_niri` function).
That unrelated file was not changed. Required ordinary Clippy and strict checks
for all three changed crates passed; the broader strict run is not claimed green.

## Remaining acceptance

No new native runtime evidence was collected. The
[static atlas report](../reverse-engineering/wallpaper-engine-atlases.md) retains
the binary identity, reader/update/binding/shader observations and incomplete
uniform-upload mapping. Original project acceptance, deterministic screenshots,
packed-edge/filtering parity, vendor GPUs and actual niri sessions remain pending.
The matrix still labels every native reference `pending`. Plasma scenes still
lack a rendering consumer. These gaps keep the overall A1 work package open.
