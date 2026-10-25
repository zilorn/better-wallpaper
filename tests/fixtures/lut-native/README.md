# Owned LUT sampling probes

The eleven packages contain independently authored color patches, volume TEX
payloads, JSON, and a small pass-through vertex adapter. No Wallpaper Engine
fragment shader, commercial artwork, executable or library is redistributed.
`generate.py` uses only Python's standard library and the existing owned fixture
generators. Default execution verifies all 22 inputs; `--write` regenerates them.
It never generates or overwrites native captures or their manifest.

`captures/*.png` are unedited 128×128 RGB window captures from Wallpaper Engine
2.8.42, build 23967692, running in an isolated Wine 11.20/Xvfb environment. The
stock `ccsimple.frag` is resolved from that installation. Its normal vertex shader
expects a fullscreen application pass; the owned package vertex adapter applies
the native model/view/projection transform to make the **same unmodified native
fragment shader** usable on a controlled image-layer effect. This establishes
fragment sampling and binding behavior, not the app's global user-filter UI or
arbitrary effect vertices. Black exploratory captures without this adapter were
rejected and are not references.

All scenes disable bloom, camera fade/shake/parallax, fix camera zoom and use
static inputs with no properties, random operators, audio or pointer data. The
128×128 source is sampled at 1:1 and contains 256 RGB patches, including 0, 1, 4,
8, 127, 128, 254 and 255. Each patch occupies 8×8 pixels. Controls and captures
are identified in `manifest.json`, along with executable/input/reference hashes
and gates fixed before capture: local SSIM ≥0.995, RGB MSE ≤1, maximum byte error
≤2. RGBA comparison with opaque alpha uses the equivalent MSE gate of 0.75.

| Probe | Purpose |
| --- | --- |
| baseline / bypass | No effect versus LUT strength zero; captured RGB is identical |
| neutral | 32³ x/y/z ramp, linear clamp |
| invert / mixed / nonlinear | Channel permutation/inversion, 0.375 blend and nonlinear RGB lookup |
| raw | Raw TEXB1 volume with exactly the same voxels as the PNG invert probe |
| nearest / repeat | Point sampling and linear periodic boundaries on all axes |
| alpha | Alpha 128 input with translucent layer blending; LUT keeps input alpha |
| noncubic | Independent 4×3×2 axes and slice interpolation |

The native neutral cube is **not an exact mathematical identity**: normalized
sampling maps `c` to `c*N - 0.5`, while this common cube stores `round(255*i/(N-1))`.
It differs from the ungraded control by at most four byte values. Tests retain
and match this measured native response; they do not detect/replace neutral cubes
or introduce a half-texel color-coordinate correction. The zero-strength control
does preserve the image exactly. See the [A2 evidence](../../../docs/reverse-engineering/wallpaper-engine-luts.md).

## Reproduce

```sh
python3 tests/fixtures/lut-native/generate.py
cargo test -p gpu-renderer --test scene_golden lut_ -- --include-ignored --nocapture
```

For native recapture, follow the isolated installation/prefix preparation in
[the atlas protocol](../atlas-native/README.md). Keep the original application
and prefix separate; the script stops only its isolated Wine server. Requires
Wine, Xvfb, libX11, and ImageMagick `import`.

```sh
python3 tests/fixtures/lut-native/capture.py --work /absolute/isolated-directory
```

The script verifies executable identity, copies probes into a new `recapture/`,
opens a fresh native process per probe, checks the 128×128 client geometry and
captures its client pixels. Native shader caches remain beside temporary copies.
It refuses existing output directories and never copies images into the repository.
Compare decoded RGB with retained captures: PNG timestamp/metadata identity is
not required. All eleven captures were independently repeated with identical RGB.

Run the full GPU suite with the default device, then explicitly select Mesa:

```sh
cargo test -p gpu-renderer --test scene_golden -- --ignored --test-threads=1 --nocapture
__EGL_VENDOR_LIBRARY_FILENAMES=/usr/share/glvnd/egl_vendor.d/50_mesa.json \
LIBGL_ALWAYS_SOFTWARE=1 cargo test -p gpu-renderer --test scene_golden \
  -- --ignored --test-threads=1 --nocapture
```

Setting `BETTER_WALLPAPER_LUT_INSTALLATION` to the selected local installation
also verifies each of the 28 input hashes in `docs/validation/a2-volumes.json`,
parses it through LUT admission and uploads/draws it in both fresh contexts.
Those are resource/submission checks, not 28 additional native pixel references.
The GPU harness additionally verifies frozen/rewound time, clear/reload, context
replacement, invalid-upload retry, required-LUT failure and the combined 256
image/LUT residency ceiling. Source scaling, advanced post-process stacks, HDR,
COL, animated/property-bound LUT parameters and Plasma scene playback are outside
this bounded acceptance matrix.
