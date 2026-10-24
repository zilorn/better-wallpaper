# A1 native atlas uniform and sampler closure

Investigation: 2026-10-24. This closes the uniform-binding and original-program
acceptance questions in the [earlier atlas report](wallpaper-engine-atlases.md).
Target remains Wallpaper Engine 2.8.42, Steam build 23967692, `wallpaper64.exe`
SHA-256 `40e2ce021e9352324fadb3b8f72b8ba2a7ee95b71cc571d5b9f84be75cd993b0`.

REA restored the local snapshot using Hopper. The ignored local export
`target/rea/a1-uniform-evidence.json` contains 24 records, 1,353,921 bytes,
SHA-256 `b8bc3f7ad5d29d5dca67f1b7c21188f140111a38f4d4e869c41fe6acbbf40804`.
The saved snapshot `target/rea/a1-uniform-snapshot.json` is 1,491,553 bytes,
SHA-256 `87b8bfe62f5ff3536cdb9a4dd2869c7e400e5bf9880d2f055ca626901d0ee213`.
Decompiler text and commercial binaries remain local. The following describes
observed machine operations; recovered pseudocode types are not treated as source.

## Complete data path

The earlier reader normalizes pixel origin/bases by the selected image's dimensions.
Its name-registration evidence maps `g_Texture0Rotation` to ID `0x2a` and
`g_Texture0Translation` to `0x34`. This investigation connects those IDs to the
frame-update storage and mapped shader constant buffer:

| Evidence ID | Address | Observation |
| --- | --- | --- |
| `ev_58735bf0b35ba16c52f9daed45c80f771b390c63cdaf4b265021d812ff46d54f` | `0x14015f0d0` | Frame update writes the four basis floats to engine `+0x1cc + unit*16`, origin to `+0x26c + unit*8`, and image index to texture `+0x98`. |
| `ev_4a4869bcceb673f3efba9e77086cb20390f3a842f561ba730270e3b39c6de72c` | `0x1400d83b0` | Uniform dispatcher loads the 16-bit uniform ID and output offset, then dispatches through index and jump tables. |
| `ev_ad17a55e08f9f94647d974cb43c0ebab878a105264a33f524eb9b61545fed751` | `0x1400daad6` | IDs `0x2a…0x33` select case `0x20`; `0x34…0x3d` select case `0x21`. |
| `ev_10498bb265496b1c693f3dfc269869441efda143b82bfee53a339c6e6554f619` | `0x1400da984` | Jump entries 32/33 point to RVAs `0xd979d` / `0xd97b7`. |
| `ev_813c366a149d0f3d3e47b50c8b89e97be999c5c7f8f01a87c21d263295f4deb8` | `0x1400d979d` | Basis loads from engine `+ID*16-0xd4`; translation loads from `+ID*8+0xcc`. For IDs 42/52 these are exactly `+0x1cc` / `+0x26c`. The ID increments preserve the corresponding per-unit strides. |
| `ev_148729e3231f07288f2b56c203349a77753ec363bbcc5848b3d5757e894f2a0b` | `0x1400da8ea` | Writes the basis vector to the mapped destination at the uniform's recorded offset; translation case writes its eight bytes directly. |
| `ev_a4d60d5822b7650556197a56e0cd261ef7ce06b66081388fa9c138dc562e809a` | `0x1400d7440` | Caller maps the buffer, passes its data pointer to `0x1400d8300` at `0x1400d7519`, unmaps, and binds the constant buffer for shader stages. Interface slots `+0x70/+0x78/+0x38/+0x80` match D3D11 Map/Unmap/VSSetConstantBuffers/PSSetConstantBuffers. |
| `ev_95e1fe33dcba290200d057c32be5d70ff188f77da7e7b063479876362dfd49df` | `0x140099980` | Sampler construction selects point filtering from TEX flag bit 0; address mode is WRAP (1) or CLAMP (3), selected by bit 1. |

The D3D11 slot interpretation was cross-checked against the locally installed Wine
`d3d11.h` interface ordering. The shipped `genericimage4.vert` source identity and
`O + u*H + v*V` expression remain those recorded in the earlier report. Its
`SPRITESHEET` branch consumes the uploaded basis/origin without a half-texel inset.
Texture binding separately reads the frame's image index (earlier evidence at
`0x1400ee8b0`). Together these establish reader → frame update → named uniform
mapping → mapped constant buffer → shader UV consumption, plus image selection.

## Runtime corroboration and implementation boundary

Fourteen original-program captures of owned probes corroborate rotation, both
reflections, non-axis-aligned bases, independent image/mip selection, repeat versus
clamp, linear versus point filtering and packed authored-mip minification. The
[capture protocol and hashes](../../tests/fixtures/atlas-native/README.md) separate
native images from analytical legacy references. Native acceptance required an
explicit `SPRITESHEET: 1` material; omitting it produced a static full texture.
Implicit native camera defaults also changed apparent scale, so probes explicitly
fix zoom and disable unrelated camera effects before comparison.

Better Wallpaper now parses `SPRITESHEET` (case-insensitively) as optional 0/1.
Explicit 1 selects native authored UVs, TEX point/clamp flags and authored complete
mips. Explicit 0 disables frame animation in rendering and package compatibility
metadata. Missing combo preserves the established inferred-animation path and
its old inset/crop-isolation references. This fallback is a compatibility policy,
not a claim about native materials lacking the combo.

The native path admits base-only or complete contiguous mip chains. Partial or
noncontiguous multi-level chains produce a concrete shared admission error because
the GLES2 path cannot limit the accessible mip range without inventing levels.
Bounds, positive durations, valid indices, all-frame validation, decode budgets
and group upload rollback remain in force. Native evidence is for this bounded
2D material subset; particles, LUTs, effect-mask atlases, general shaders, unusual
compression/alpha/color-space combinations and Plasma scene playback are not
established by these probes. See [results](../validation/a1-native-atlases.md).
