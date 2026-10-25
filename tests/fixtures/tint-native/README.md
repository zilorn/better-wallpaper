# A4 owned Tint probes

`python3 tests/fixtures/tint-native/generate.py` checks all 24 owned inputs;
`--write` regenerates them without changing references. No commercial texture,
shader source or executable is committed. `manifest.json` pins inputs, captures
and the Wallpaper Engine 2.8.42 executable/fragment identities.

All projects use a 128×128 orthographic canvas, camera zoom 1, no bloom,
camera shake, parallax or random input. Properties use their serialized defaults;
pointer position has no effect. The source is an eight-level RGB grid. Masks use
red (not alpha), logical size 96×64 and storage 128×64, with conspicuous padding.

The supported matrix covers baseline, Tint/default mode 30, zero/partial/full
strength, multiply 2, screen 7, normal 0, source alpha, point/clamp mask, disabled
effect and a single linear alpha timeline (0→1 in one second). Native captures
occur after the single timeline completes; it differs from the static Tint by
at most one byte. GPU comparisons use times 2, 2, 8, 2, reload and new contexts.
The preselected gate is SSIM ≥0.995, RGBA MSE ≤1, maximum channel error ≤2.

`mask-linear` is a retained **negative** probe: native linear-filtered boundaries
differ by up to 5 bytes (MSE 0.059327) from GLES bilinear sampling. The gate was
not relaxed. This layout is explicitly rejected. Only static, base-only RGBA8,
point/clamp masks are admitted in A4's bounded Tint operation.

To repeat native capture, provision a new directory containing `app/` copied
from the matching installed engine and a dedicated working Wine `pfx/`, as in
the [LUT capture protocol](../lut-native/README.md). Run:

```sh
python3 tests/fixtures/tint-native/capture.py --work /absolute/isolated-directory
```

The script refuses an existing `recapture/`, checks identities, injects the
unmodified installed Tint vertex/fragment into temporary copies of the owned
packages, and runs Wine on a private 640×480 Xvfb display. Window captures come
from a 128×128 client after four seconds; all Wine processes and Xvfb are joined.
The installed application and the user's service/configuration are not changed.
The 12 original-program images were independently repeated with identical RGB.
Do not use Better Wallpaper output to create these references.

Admission tests also cover property overrides, missing/unsafe masks, unverified
shader overrides, extra passes/targets, mixed unsupported nodes and invalid
parameters. Neither these fixtures nor the local occurrence survey establish
whole-project playback or Plasma scene support.
