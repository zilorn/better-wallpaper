# A2 native LUT consumption evidence

Investigation: 2026-10-25. This extends the
[volume-reader evidence](wallpaper-engine-volumes.md) with the LDR `ccsimple`
fragment contract and original-program sampling probes. The target is Wallpaper
Engine 2.8.42, Steam build 23967692, x86-64 `wallpaper64.exe`, 5,360,112 bytes,
SHA-256 `40e2ce021e9352324fadb3b8f72b8ba2a7ee95b71cc571d5b9f84be75cd993b0`.
The native binary was opened with REA/Hopper using the prior A2 snapshot and
profile digest `32c01d9334c1fa8dee7c09790f43820490bcf0022af7efe157da365c2ab35983`.
The session was exported and closed. Native execution occurred separately in an
isolated Wine prefix with owned packages; the original installation was unchanged.

## Static observations

| Evidence ID | Location | Observation and boundary |
| --- | --- | --- |
| `ev_518ec925057c8fd6cdcf0427df0df0261ae506e9e2e162c10f25d39769d62299` | `lutparams` string xref `0x140182e8e` | Resolves into settings-consumption function `0x140181f30`. |
| `ev_d4514695c8021f08037d72b6bfcc701ec9073f4f6fe20be6fccbf8fbddf4b3fc` | `0x140181f30`; `0x140182560`, `0x1401825d7`, `0x140182e77`–`0x140182ea3` | Reads `wcc_v` and `wcc_amt`; uses `materials/util/ccsimple.json`; loads float at object offset `0x3148` and passes its address with count 1 to the material constant setter under `lutparams`. Decompiler variables are reconstructed, not source. This is app settings evidence, not implemented API support for those settings. |
| `ev_a6e9dc54bdeedbd1b9d7e54e509c4d22ec07698c7ba8b3430d2945c546f21184` | Shader generator `0x1400f5cb0`; `Texture3D` xref `0x1400f6d43` | Emits a 3D texture declaration and sampler state. Complete upload/driver internals are not inferred from this dossier. |

Shipped file identities (not redistributed):

| File under `assets/` | SHA-256 |
| --- | --- |
| `shaders/ccsimple.frag` | `0cb10ceff0b7fe6e14576928ee09e4d2fed7fb405dc6ff38d94970687e46ed3b` |
| `shaders/ccsimple.vert` | `25d48e4e45c8f2092e87d1a29eb2aeb62e5592e1581a6efc15dd06e9790f53cd` |

The fragment declares input texture zero, volume texture one under LUT, and the
`lutparams` float defaulting to one. In the verified COL=0/HDR=0 branch it samples
the volume with the input RGB vector, mixes original and looked-up RGB using that
float and retains input alpha. There is no explicit sRGB/linear conversion or
half-texel correction in that branch. The separate HDR and HSV branches are not
admitted by this implementation.

The stock vertex stage writes clip-space positions directly, explaining the black
initial image-material/effect probes. The final owned probes supply a small MVP
vertex adapter while retaining the original shipped fragment stage. Do not label
these as unmodified app-global filter screenshots: they establish the bounded
fragment behavior, texture binding, voxel/channel order and normalized sampling.

Ignored local evidence exports:

| File | Bytes | SHA-256 |
| --- | ---: | --- |
| `target/rea/a2-lut-evidence.json` | 4,292,164 | `0078583774bb08b00e5c0b3312108b04406ac7dcf2a7b153365241f1f8b4bc4d` |
| `target/rea/a2-lut-snapshot.json` | 7,344,900 | `45afd2c4a5d0c03d4cbe3977a1e09c368ca29bd1e65f78d6f801fda9f4153f71` |

## Observed sampling

Eleven [owned probes](../../tests/fixtures/lut-native/README.md) cover a 1:1
ungraded control, strength zero, a neutral cube, channel permutation/inversion,
nonlinear RGB, partial blend, raw versus PNG, alpha, point/linear, clamp/repeat
and a noncubic 4×3×2 volume. All native captures were independently repeated with
identical RGB. Inputs, reference hashes and preselected comparison gates are
retained in the fixture manifest. No reference pixels came from Better Wallpaper.

For linear sampling on each axis the observed coordinate is `p = c*N - 0.5`;
interpolate the eight surrounding voxels after clamping or periodic addressing
their integer indices. Point sampling uses `floor(c*N)` followed by addressing.
X varies fastest, then Y and Z. Raw format-zero bytes and PNG RGBA agree in the
controlled invert probe. The nonlinear probes agree with sampling byte-normalized
RGB directly rather than adding a linear-light conversion in the verified path.
These findings do not establish physical display color management or HDR behavior.

The neutral 32³ cube stores `round(255*i/31)`, so direct normalized-edge sampling
is not an exact identity. The native neutral capture differs from its control by
at most four byte values (mean absolute RGB error 1.875). Strength zero is an
exact identity. This corrects the plan's idealized neutral criterion: acceptance
must match native finite-lattice behavior, not silently change coordinates or
replace the cube to force mathematical identity. GPU comparisons to the actual
neutral capture are exact on both tested devices.

## Implemented boundary

Package-aware admission resolves the effect JSON and its one material pass; only
the known `ccsimple` LUT-only contract with one visible effect, static strength
0–1 and texture slot one is admitted. A name or a parsable volume alone grants no
effect support. Unknown combos/constants, HDR/COL, extra passes/targets, custom
fragment code, unsafe paths and unsupported volume layouts remain diagnostics.

Base-only raw ARGB8888/PNG volumes, axes 2–128 and at most 8 MiB retained bytes
are used as unconverted RGBA8. Scene LUT bytes are shared by path and capped at
32 MiB. GPU storage is a bounded 2D stack with nearest voxel fetches and explicit
3D interpolation/addressing, requiring no optional 3D-texture extension in GLES2.
Stack dimensions must fit the actual device 2D texture limit; failure is explicit
and a required missing LUT prevents drawing. CPU inspection accepts a broader
volume subset and still does not imply standalone 2D playback.

This validates a bounded LDR LUT operation. App-global user filters, arbitrary
shader overrides, HDR, HSV/COL, animated parameter bindings, general multipass
resampling/order, broader minification/color fidelity and Plasma scenes remain
separate work. See [validation](../validation/a2-lut-rendering.md) for device,
lifecycle, local-asset and desktop results.
