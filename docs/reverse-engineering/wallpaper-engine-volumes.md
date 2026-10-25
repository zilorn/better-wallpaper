# A2 volume TEX layout evidence

Parser-stage investigation: 2026-10-24. The later bounded GPU/effect consumption
and native captures are recorded in [LUT evidence](wallpaper-engine-luts.md).
The statements below describe this parser-stage investigation.

Investigation: 2026-10-24. Scope: single-image RGBA8 volume parsing for inspection.
No GPU upload, LUT interpolation, color transfer or color-grading effect behavior
is established by this investigation.

## Target identity and evidence

Wallpaper Engine 2.8.42, Steam build 23967692, x86-64 `wallpaper64.exe`:
5,360,112 bytes, SHA-256
`40e2ce021e9352324fadb3b8f72b8ba2a7ee95b71cc571d5b9f84be75cd993b0`.
Installation: `~/.local/share/Steam/steamapps/common/wallpaper_engine`.
REA restored the earlier atlas snapshot through Hopper, profile digest
`32c01d9334c1fa8dee7c09790f43820490bcf0022af7efe157da365c2ab35983`.
The native target was not executed. Evidence was exported and the session closed.

Ignored local exports (not distributed application/decompiler content):

| File | Bytes | SHA-256 |
| --- | ---: | --- |
| `target/rea/a2-volume-evidence.json` | 2,745,535; 26 records including retained earlier evidence | `fdaf64dad23868700d79c67686a370a90b21aee284922d4a461b1cd7ec82b0e3` |
| `target/rea/a2-volume-snapshot.json` | 4,269,991 | `4ba3a97dce6efb81d5c2ddcc7a1d0fe19b9e2335a01209cd6b5de2099ac086fe` |

| Evidence | Location | Observed operations |
| --- | --- | --- |
| `ev_49e9b6347993edc488fbbd89b73ee5b3d0c4df07fead3b95b10b67e6cdd47341` (retained) | TEXI reader `0x14015c760`, especially `0x14015c855`–`0x14015c8c1` | After format, flags, storage width/height and source width/height, flag 0x40 enables an additional depth read. A final TEXI0001 word follows depth; its meaning remains opaque here. |
| `ev_63996aa9475382a5676e2d1cc7872244f95903e3373f728109d70f8de0ecb5ea` | TEXB dossier `0x14015c8d0`; `0x14015d319`–`0x14015d3e6` | Mip width and height precede the conditional depth. Native checks include width/height ≤8192, depth ≤128 and bounded width×height×depth×4 arithmetic. |
| Same dossier | `0x14015dd64`–`0x14015ddcf` | The embedded-image branch multiplies mip height by depth, allocates width×height×depth×4 bytes, and passes width and flattened height to the decoder interface. No per-slice operation occurs in this call sequence. |

Assembly corroborates the sequential TEXB count/codec/version reads already
recorded in the [texture investigation](wallpaper-engine-textures.md). Hopper's
reconstructed pseudocode is not original source; its large expanded control flow
is not used to infer LUT sampling. Hopper's function-body and indirect-call
coverage limitations remain. This closes the observed reader layout, not the
complete downstream volume upload/consumer chain.

## Independent local file observations

The 28 direct `assets/materials/lut/*.tex` files all have:

- TEXI format 0, flags `0x42`, storage 32×32, source dimensions 1024×32, depth 32.
- A nonzero opaque final TEXI word, followed by `TEXB0004` at byte 50.
- One image, codec 13 (PNG), non-video marker 0 and one mip.
- Mip dimensions 32×32×32, compression 0, decoded-size field 0, encoded length,
  then a PNG beginning at byte 99. Each PNG is 32×1024.

`neutral.tex` retains its earlier SHA-256
`967ea45d38c40eda283d440bd80b914587c779ec6fbe1981e761d2443a68fa3b`.
Independent Pillow decoding and the new Rust parser produce identical hashes for
all 28 PNG payloads. For the neutral sample, all 32,768 voxels match
`(round(255*x/31), round(255*y/31), round(255*z/31), 255)` at linear index
`((z*32+y)*32+x)*4`. This directly establishes the file's x/y/z byte ordering;
it does not establish how an effect maps input colors onto normalized coordinates.
Every input size/hash matches the prior A0 inventory. The
[selected manifest](../validation/a2-volumes.json) stores individual raw/decoded
hashes and results without redistributing commercial assets.

## Implemented boundary and remaining evidence

`TexVolume` is separate from `TexAtlas`, `TexTexture` and the renderer's 2D input.
The bounded subset accepts one nonanimated, nonvideo image with raw ARGB8888 or
PNG payloads; TEXB1–4 raw and TEXB3/4 PNG headers and optional LZ4 are covered.
PNG volume mips use width×(height*depth); bytes remain x/y/z ordered. Consecutive
mip dimensions halve each axis with floor division and clamp to one. This is a
conservative admission rule for the supported subset; the local assets provide
only base-level PNGs, so other valid native mip layouts may still be rejected.

BCn/other raw formats, other embedded codecs, multiple images, animation/video,
noncontiguous mips and trailing blocks return explicit errors. Preserve raw ARGB
bytes and canonical PNG RGBA separately. Do not infer a color transfer function.
No 2D playback admission, compatibility level or effect whitelist was changed.

Remaining A2: verify the effect's coordinate convention, interpolation, channel
and color-space behavior with original-program captures; choose bounded GPU
3D/slice storage and capability handling; connect parameter/resource binding;
verify identity, nonidentity and edge sampling before claiming LUT playback.
