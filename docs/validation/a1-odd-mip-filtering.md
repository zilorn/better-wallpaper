# A1 generated odd-sized mip filtering

Date: 2026-10-24. This fixes one reproducible sampling defect in generated RGBA
mips; it does not complete A1 or establish Wallpaper Engine visual parity.

## Reproduction and change

`downsample_rgba8` previously divided the source into whole-texel bins. For a
5×1 texture with a central RGBA texel (250, 150, 100, 200) and zero elsewhere,
5→2 produced `(0, 0, 0, 0)` and `(83, 50, 33, 66)`. The regression was run before
the fix and failed with those bytes. Equal 2.5-texel footprints must instead
share the center texel and produce `(50, 30, 20, 40)` on both sides.

The implementation now weights source contributions by exact integer footprint
overlap in each axis, using 64-bit sums. Channel division still truncates as
before. Unit coverage includes horizontal/vertical 5→2, four-channel 5×5→2×2→1×1,
and the existing even-sized and 3→1 checks. Authored mip chains, compressed
uploads, texture admission and the GL capability gates are unchanged.

The new explicit GPU case replaces a committed sample's base image with an
opaque black 5×5 texture whose center is RGB (250, 150, 100). Production upload,
draw and 1×1 readback must produce exact RGBA (10, 6, 4, 255), matching 1/25
coverage. A second upload supplies a solid authored 2×2 mip; the readback must
match its (80, 40, 20, 255) color. Expectations are analytical constants, not
captured renderer output. No fixture packages or reference PNGs changed.

## Validation

- `cargo clippy` and `cargo fmt --all -- --check`: passed.
- `cargo clippy -p gpu-renderer --all-targets -- -D warnings`: passed.
- `cargo test -p gpu-renderer --lib`: 13 passed.
- `cargo test --workspace`: 237 passed, three explicit GPU cases ignored in an
  isolated copy containing only this change, with local filesystem permissions. The first
  restricted run failed at an existing web-project symlink test with
  `Operation not permitted`; the permitted rerun passed that test too.
- `LIBGL_ALWAYS_SOFTWARE=1 cargo test -p gpu-renderer --test scene_golden -- --ignored --test-threads=1 --nocapture`:
  passed on llvmpipe (LLVM 23.1.1). The 55 existing PNG comparisons passed and
  the two new exact byte assertions passed. An unrelated concurrently added
  test was also present in this workspace run and is excluded from this change.
  A separate copy exported from the staged index passed Clippy, formatting,
  all 237 ordinary tests and all three GPU cases without concurrent edits.
  That permitted GPU run reported NVIDIA GeForce RTX 3060 Ti/PCIe/SSE2 despite
  `LIBGL_ALWAYS_SOFTWARE=1`; record the actual renderer, not the requested one.
  Its 55 PNG comparisons and two exact assertions passed as well.
- `git diff --check`: passed.

Native filtering, packed-atlas edge bleed, color-space/alpha-aware filtering,
real niri output and broader vendor GPU behavior remain unverified. Integer RGBA
quantization still applies at each generated level; exact mean preservation
for arbitrary images across an entire mip chain is not claimed.
