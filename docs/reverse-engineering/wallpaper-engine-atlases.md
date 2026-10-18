# Wallpaper Engine atlas model and consumer evidence

Investigation date: 2026-10-18. This A1 batch implements bounded container
inspection, not advanced atlas playback. It extends the
[TEX reader investigation](wallpaper-engine-textures.md) with static consumer
evidence. No original-program screenshots or sample-acceptance results were
obtained; native visual validation remains pending.

This records the first implementation batch (`d012f50`). Subsequent shared
scene/GPU atlas support and its limits are documented in the
[rendering validation](../validation/a1-atlas-rendering.md). The static native
evidence and its unresolved uniform-upload/sample-acceptance boundaries below
remain unchanged; descriptions of the old playback path are historical.

## Target and retained evidence

The local installation is Wallpaper Engine 2.8.42, Steam build 23967692.
`wallpaper64.exe` SHA-256:
`40e2ce021e9352324fadb3b8f72b8ba2a7ee95b71cc571d5b9f84be75cd993b0`.
REA restored the prior snapshot and used the installed Hopper provider with
profile digest `32c01d9334c1fa8dee7c09790f43820490bcf0022af7efe157da365c2ab35983`.
The 41-record export is retained locally at `target/rea/a1-atlas-evidence.json`
(3,360,785 bytes; SHA-256
`b5c2bc7fd748912a6a2502b65b75215e9f0aee01ae5cefcb2076099e9f4dfe14`).
That ignored export contains decompiler output and is not distributed. Repository
fixtures contain independently authored data, not commercial assets.

The addresses below are virtual addresses in that exact executable. Assembly
corroborates the observations; decompiler types and indirect-call names are not
treated as recovered source.

| Evidence ID | Location | Observation |
| --- | --- | --- |
| `ev_fe2b69e9a4114e18aa3ae31c5d0304e2992586e42d82797eb6433ac2a700310c` | TEXS reader `0x14015e1d0` | The image index selects an independent mip list. The first mip's dimensions normalize the three X and three Y components, respectively (`0x14015e4a2`–`0x14015e4eb`). |
| `ev_529961c6151887820793a644bad7985e4986fc3eac146d81ee64c903fb91f81a` | Frame update `0x14015f0d0` | A selected 32-byte frame has duration at +0, image index at +4, origin at +8/+12, and four basis components at +16…+28. The update copies the bases and origin to per-texture-unit engine storage (`0x14015f281`–`0x14015f2da`) and the image index to texture-object +0x98 (`0x14015f2e8`). |
| `ev_e6b03f93836543ac5918f51dab1a71d50fd78154a1eadcb8bb5aedfbf4182582` | Texture binding `0x1400ee8b0` | The binder reads object +0x98, indexes the image list at object +0x140 with a 24-byte stride, and passes the selected entry's +16 slot to the graphics-interface call (`0x1400ee8f0`–`0x1400ee913`). This corroborates image selection beyond the file reader. |
| `ev_3b95ee66a31b03aff529f84b60dd54b0aa78d86fa29dfc93a2cb6952c2d36fd5` | Uniform-name initialization `0x140002860` | Registers `g_Texture0Rotation` with identifier 0x2a and `g_Texture0Translation` with identifier 0x34. The complete mapping from the update buffers to uniform uploads has not been traced. |
| `ev_d4d3cd5f09fd1f96e828aec829d9117790a046475a222c36e56439480c7c12d4` | String search | Locates both uniform names and the `SPRITESHEET` feature string; names alone are supporting evidence, not proof of runtime binding. |

Ordinary local reads of shipped shader files provide a separate source of
evidence. Paths are relative to the selected installation:

| File | SHA-256 | Relevant lines |
| --- | --- | --- |
| `assets/shaders/genericimage4.vert` | `7cc66038ea106b213ae2b9103d0b3c6646e3b4d08f9821afbb24aa32f6186672` | 6–7 declare the uniforms; 176–180 apply their components under `SPRITESHEET`. |
| `assets/shaders/passthrough.vert` | `ff6cd2bb166ce427c73058f3f77f338442d83fab9883f302e31bbffd81fa5317` | 22 applies the same affine expression. |

In independent notation, their expression is `UV = O + u*H + v*V`, where `O`
is a two-component translation and `H`, `V` are the two columns of the four-
component basis. Combining the shader expression, reader normalization and frame
update is evidence for retaining all six authored components and the image
index. The incomplete uniform-upload mapping remains an inference boundary.
No half-texel inset, filtering policy, effect-coordinate behavior, or acceptance
of the synthetic rotated/skewed projects is inferred from this expression.

## Implemented model and inspection contract

`TexAtlas` retains `Vec<TexAtlasImage>`, each with its own mip chain.
`TexAtlasFrame` retains image index, duration, origin, horizontal basis and
vertical basis in authored pixel coordinates. TEXS1 signed integers are converted
to floats; TEXS2/3 floats retain their values. TEXS3 logical frame dimensions are
preserved separately. No normalization or sampling inset is applied by parsing.

`TexAtlas::parse` and `parse_with_budget` enforce:

- 1–64 images, 1–32 mips per image, at most 10,000 frames, and valid image indices.
- Header and validated pixel dimensions in 1–16,384; the existing 64 MiB per-mip
  ceiling and 512:1 LZ4 ratio limit; exact decoded LZ4 length.
- At most 128 MiB of retained payload across all images. A caller may lower this
  limit. One mip's temporary encoded/decoded buffers can coexist before the
  retained-byte check; this is not a 128 MiB total process-memory guarantee.
- Finite coordinates and finite nonnegative durations. Zero duration is retained
  for particle data; parsing does not establish timed animation or particles.
- Complete declared TEXS records and known versions; unknown compression,
  truncated records, empty mips and mismatched pixel sizes fail explicitly.

`to_texture_images` returns independently size-validated images. The single-image
case retains the existing TEXI padded-storage fallback. With multiple images,
each image uses its own base mip dimensions; it cannot borrow another image's
global storage dimensions to accept mismatched payload bytes. GPU format support
and useful mip progression are separate consumer concerns.

The model deliberately preserves finite off-axis, negative and degenerate bases;
it does not certify valid draw geometry, in-range UVs or edge sampling. Volume
textures still fail. A single video payload retains the existing parser boundary,
is reported as unverified, and cannot be converted to pixel images. Multiple
video images fail. Native TEXB4 metadata work remains in A3.

`TexTexture::parse` still applies the existing single-image playback gate. Its
shared parser retains legacy optional-animation behavior and budgets; stricter
atlas inspection is opt-in. `TexAtlas::try_into_single_texture` rejects multiple
images, off-axis bases and negative-direction bases with the existing messages.
No scene resource resolver, compatibility tier, GPU upload or shader is changed.

For an unpacked texture:

```sh
cargo run -p better-wallpaper-scene-format --bin scene-inspect -- /path/atlas.tex --atlas --json
```

The option caps file input at 128 MiB and reports each image/mip, full frame
components, `payload_status`, `single_image_layout` and
`native_visual_validation: pending`. `validated_2d_images` means pixel sizes
passed. `single_image_layout.accepted` only means the legacy layout conversion
passed; neither field promises supported GPU formats or successful playback.
The default inspection command continues to reject advanced layouts.

## Regression scope and remaining work

Eight new unit tests cover independent image/mip bytes, the five committed A1
probes, all twelve TEXB1–4/TEXS1–3 combinations, signed bases, total budgets,
count/index limits, truncation at every byte of a complete two-image record,
all nonfinite frame fields, negative/zero durations, LZ4 length, pixel-size errors,
and preserved video/padding boundaries. Two command-line tests check structured
reporting, legacy rejection, truncated input and wrong input kind.

The [sample matrix](../../tests/fixtures/scene-behavior/README.md) still treats
the five advanced layouts as playback failures, while the new model tests prove
their structure can be represented. The original packages, matrix and analytical
PNG expectations are unchanged. Full command results are recorded in the
[A1 validation record](../validation/a1-atlas-model.md).

Next: trace the remaining uniform upload and native sample acceptance; implement
bounded asset/GPU image selection and affine UV submission; then add positive
cross-image, rotation/flip, edge, loop, pause and context-recreation regressions.
Native captures, vendor-GPU and actual niri/Plasma verification remain separate.
