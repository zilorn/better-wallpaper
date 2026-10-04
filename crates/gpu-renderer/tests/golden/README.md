# Scene GPU golden regressions

Run from the repository root:

```sh
LIBGL_ALWAYS_SOFTWARE=1 cargo test -p gpu-renderer --test scene_golden -- --ignored --test-threads=1 --nocapture
```

The GPU test is explicitly ignored in the ordinary workspace suite because it
requires EGL 1.5, `EGL_MESA_platform_surfaceless`, GLES 2 and an installed software
renderer. Explicit runs fail on missing GPU support; they never silently skip.
No desktop display or visible surface is used. The comparison gate itself runs
in ordinary tests. Keep GPU cases on one thread so EGL display teardown is safe.

Ten committed PNG references cover parent transforms, radians/rotation, layer
order, opaque/translucent/additive blending, output cover/resize, padded TEX UVs
and orientation, opacity and camera-zoom timelines, TEX sprite frames/loops,
basic parallax axes/pause, and real embedded TEXB0004 video decoding/uploads.
Video checks also change RGBA storage size and recreate the EGL context while
retaining CPU frames, exercising the output-replacement upload contract.

References are original synthetic fixtures generated independently by
`generate_references.py` from documented geometry, source-over/additive arithmetic
and explicit texture pixels. The test builds PKG/model/material/TEX assets and
calls the production `SceneGpuRenderer`, then reads actual RGBA pixels. References
are never regenerated from renderer output. Update them deliberately with:

```sh
python3 crates/gpu-renderer/tests/golden/generate_references.py
```

The gate requires all of:

- SSIM >= 0.995, averaged over overlapping 8x8 luminance box windows, stride 4.
- Mean squared error <= 1.0 across every RGBA byte.
- Maximum absolute channel error <= 2 on the 0–255 scale.

The [SSIM formula](https://ece.uwaterloo.ca/~z70wang/publications/ssim.pdf) uses
`C1=(0.01*255)^2`, `C2=(0.03*255)^2`. This fixture metric uses uniform local windows
and population variance, not the paper's Gaussian implementation. RGBA error
gates additionally catch alpha and channel errors that luminance SSIM can miss.
Failure writes actual and amplified RGB difference PNGs under
`target/scene-golden-failures/`, and reports the metrics/path in the assertion.

On the recorded llvmpipe run all fourteen comparisons had SSIM 1.0 and zero MSE
and maximum channel error. These fixtures establish deterministic regression
coverage. They do not establish Wallpaper Engine screenshot parity, real niri
pointer/hotplug behavior, vendor GPU coverage, or the eight-hour L2 soak criterion.
