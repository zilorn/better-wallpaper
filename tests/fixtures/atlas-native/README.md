# Native atlas acceptance probes

These are independently authored, opaque RGBA test images and scene packages.
They contain no Wallpaper Engine code, shaders, artwork or redistributed runtime.
`generate.py` derives five affine/multi-image probes from the owned
`../scene-behavior` packages, adds explicit `SPRITESHEET: 1`, fixed camera settings
and no-padding models, and emits constant-frame and animated variants. Four
additional probes exercise clamp, nearest filtering and packed authored mips.

`captures/*.png` are **original-program window captures**, not renderer output or
analytical reference generation. Wallpaper Engine 2.8.42 (build 23967692) opened
each package in a fresh Wine 11.20 process on an isolated Xvfb desktop. Executable,
input and PNG identities, viewport and quantitative gates are in `manifest.json`.
Each native reference uses a one-frame TEXS2 sequence, so there is no unknown
animation phase. The animated counterparts retain two 0.5-second frames and are
compared against the corresponding constant-frame captures at explicit scene times.

| Samples | Coverage |
| --- | --- |
| `multi-image0/1`, `multi-image-animated` | Independent 2×2 + 1×1 mip chain and 1×2 base image; image selection |
| `rotated0/1`, `flipped-x0/1`, `flipped-y0/1`, `skewed0/1`, animated counterparts | Signed bases, quarter turn, both reflections, non-axis-aligned UVs; repeat addressing |
| `clamped`, `nearest` | Same rotated color quadrants with clamp-linear / clamp-point sampling |
| `packed-mips`, `packed-point` | 64×32 packed texture, complete seven-level authored mip chain, 32×32 frame reduced to a 16×16 output square; deliberately different base/mip colors |

All outputs are 128×128, with no scaling, color conversion or edited reference
pixels after capture. Inputs disable camera parallax/shake/fade/bloom, fix zoom to
1, and use no pointer, random, property or audio operators. Native RGB is compared
to RGBA with alpha fixed at 255. Gates registered before capture: local SSIM ≥
0.995, RGB MSE ≤1 (equivalent RGBA MSE ≤0.75), maximum byte-channel error ≤2.
Changing these gates or generating native references from the implementation
would invalidate the comparison.

## Reproduce

Check inputs without modifying them:

```sh
python3 tests/fixtures/atlas-native/generate.py
```

`--write` regenerates only the 38 owned input files. It never regenerates captures.
The ordinary Rust test verifies hashes of these inputs and all 14 PNG references.

For an independent native recapture, prepare an isolated working directory outside
the repository with `app/wallpaper64.exe`, a copy of the installation's `bin/`,
and access to its `assets/` and `ui/` directories. Put a **copy** of the working
Steam compatibility prefix in `pfx/` (including `system.reg`). Use the exact
executable identity in the manifest. Keep the original installation and running
prefix separate: the script stops only the Wine server of this isolated prefix.
Requires Wine, Xvfb, libX11 and ImageMagick's `import` command.

```sh
python3 tests/fixtures/atlas-native/capture.py --work /absolute/isolated-directory
```

The script copies fixtures into a new `recapture/` directory, launches a new
native process for each sample, verifies the client is exactly 128×128, hides the
separate native setup window, then captures the named client. Native shader caches
are written beside the temporary input copies. It refuses to overwrite an existing
recapture directory or use a different executable. It does not copy results back
into the repository. Compare decoded RGB pixels with the committed references;
PNG metadata/timestamps need not be byte-identical. The checked-in script was run
against all fourteen samples and its decoded pixels matched the retained captures.

GPU validation (ordinary test runs intentionally ignore graphics cases):

```sh
cargo test -p gpu-renderer --test scene_golden -- --ignored --test-threads=1 --nocapture
```

Select Mesa explicitly for software verification on a system where NVIDIA is the
default EGL vendor; `LIBGL_ALWAYS_SOFTWARE=1` alone does not select Mesa:

```sh
__EGL_VENDOR_LIBRARY_FILENAMES=/usr/share/glvnd/egl_vendor.d/50_mesa.json \
LIBGL_ALWAYS_SOFTWARE=1 cargo test -p gpu-renderer --test scene_golden \
  -- --ignored --test-threads=1 --nocapture
```

The native animated multi-image window visibly alternated both authored colors.
Native CLI `pause` did **not** freeze that Wine window; no native pause or precise
clock equivalence is claimed. Controlled-clock GPU tests independently verify
frame boundaries, loop, repeated paused time, rewind, clear/reload and fresh EGL
contexts. See the [validation record](../../../docs/validation/a1-native-atlases.md)
for results and desktop boundaries.
