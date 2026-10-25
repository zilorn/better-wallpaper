# Wallpaper Engine texture investigation

Investigation date: 2026-10-18. Scope: TEX container and rectangular sprite
compatibility, using the locally installed application and its assets. This is
static reverse engineering, not recovery of the original source or a claim of
complete Wallpaper Engine compatibility.

Follow-up: [A3 TEXB4 investigation](wallpaper-engine-texb4.md) corrects the
previously unverified video/editor interpretation: TEXB4 adds an editor-record
count, while TEXI flag `0x20` identifies video. It records native-reader evidence,
real TEXB3/4 video payload checks and bounded metadata/runtime validation. The
remaining-question statements below describe this original investigation's date.

## Target and retained evidence

The selected installation was `~/.local/share/Steam/steamapps/common/wallpaper_engine`.
`version.json` reports 2.8.42; Steam's app 431960 manifest reports build 23967692.
The x86-64 PE `wallpaper64.exe` is 5,360,112 bytes, SHA-256
`40e2ce021e9352324fadb3b8f72b8ba2a7ee95b71cc571d5b9f84be75cd993b0`.

REA used the installed Hopper provider. Profile digest:
`32c01d9334c1fa8dee7c09790f43820490bcf0022af7efe157da365c2ab35983`.
The complete 11-record evidence bundle and analysis snapshot were exported locally
before closing the native session:

- `target/rea/wallpaper-engine-2.8.42-evidence.json`
- `target/rea/wallpaper-engine-2.8.42-snapshot.json`

These ignored files contain decompiler output and are not distributed with the
repository. The report retains identities and interpretations; regression fixtures
use independently generated pixels. No target application execution was needed.

| Evidence | Operation / location | Observation |
| --- | --- | --- |
| `ev_c1e8e6e57b559c9a693f9058210e683aa65442e4cae9c25500a4fb01fb49f272` | Function dossier, `0x14015e580` | Dispatches TEXI, TEXB and TEXS blocks to the readers below. |
| `ev_5a46399dfeabd00b1edddf171b54919327560b8e944c9621e0d7847aac71dc80` | Pseudocode, `0x14015c8d0` | Reads image count for block version >=1, then a codec for >=3, then an additional field for >=4. |
| `ev_539893108dc437efd90bead989d00bf3440408b9eae1c3d768243c808accb639` | Assembly, `0x14015c8d0` | Corroborates the version comparisons and sequential four-byte reads at `0x14015c910`, `0x14015c94e`, `0x14015c97c`. |
| `ev_ef18606fd7e2c1a228b6ca6344ea50456654db93876c1f93bf2e23e399704589` | Pseudocode, `0x14015e1d0` | Reads the frame count, two dimensions for TEXS >=3, and image index, duration and six coordinate components per frame. |
| `ev_fe2b69e9a4114e18aa3ae31c5d0304e2992586e42d82797eb6433ac2a700310c` | Assembly, `0x14015e1d0` | TEXS1 converts signed integers; later versions read floats. Components 1/3/5 are divided by texture width, 2/4/6 by height (`0x14015e4d6`–`0x14015e4eb`). The first field indexes an image (`0x14015e4a2`–`0x14015e4ba`). |
| `ev_49e9b6347993edc488fbbd89b73ee5b3d0c4df07fead3b95b10b67e6cdd47341` | Assembly, `0x14015c760` | Tests flag 0x40 and consumes an extra TEXI dimension (`0x14015c855`–`0x14015c885`). The TEXB reader also conditionally reads mip depth. |

Hopper pseudocode has reconstructed types/control flow and is not original source.
Some expressions in the large TEXB pseudocode are visibly misleading; the findings
above were checked against assembly and file bytes. Hopper does not expose full
function body ranges through its public API, and indirect-call coverage remains
limited. Static reads do not establish runtime behavior or visual equivalence.

## Findings and resulting changes

1. **TEXB headers — answered.** TEXB0001 also stores image count before mip count.
   The old parser omitted it and shifted every following field. TEXB0003 stores
   image count before FreeImage codec; the opposite ordering existed in our
   synthetic tests, not in the inspected native reader. Removed the ambiguous
   codec/count heuristic and corrected those tests. Raw data uses codec -1.
2. **TEXS version dispatch — answered.** The old parser compared a nine-byte string
   including its NUL terminator against eight-character markers. Consequently,
   TEXS1 integers were read as floats and TEXS3 dimensions were read as frame data.
   Version dispatch now uses the same NUL-stripping reader as other TEX blocks.
3. **Rectangular sprite records — answered for the supported subset.** The six
   components are interpreted as origin `(x,y)` and two basis vectors. Native
   component normalization plus rectangular local samples support mapping
   `(x,y,width,0,0,height)` to the existing rectangle model. The old interpretation
   `(x,y,width,height,ignored,ignored)` lost height. The first field selects the
   atlas image, not the frame's ordinal. Tests now use image zero for both frames.
4. **Unrepresented layouts — explicitly bounded.** The current renderer holds one
   2D image/mip chain and axis-aligned frame rectangles. Multiple images, volume
   textures, off-axis frame bases and missing image references now return clear
   errors instead of merging images into mip levels or silently misdrawing them.
   Unknown TEXS versions, oversized frame counts and truncated records fail.

Zero-duration particle atlas entries remain data, not timed sprite animation.
Particle system playback is still unsupported. The renderer's existing frame
validation, duration handling, UV inset, and pause-aware scene clock are retained.

## Local sample observations

All paths below are relative to the selected installation. These are file-byte
observations made with ordinary local readers, separate from REA Evidence records.

| Sample | SHA-256 | Reproduction |
| --- | --- | --- |
| `assets/materials/util/noflow.tex` | `1b1b65601c240b50948ddfac7c1be3807646995a202c84f0fb9075cc35561e6e` | TEXB1, one 32×32 RGBA mip; previously decoded only 32 bytes and failed upload-size validation. |
| `assets/materials/util/white.tex` | `ac8a6c0726bf5744e816300278cf42bbe660ea885088e77bbbfb136725a26bdd` | TEXB1, four mips; previously misread as one invalid mip. |
| `assets/materials/particle/animals/fish1.tex` | `dbb3aa9ecd0ac7f21eef0e2f1849a322c11c28a1225fc580d039ff65f01caeea` | TEXS3 at byte 998164, 16 frames. First record: image 0, duration 0.0625, components `(0,0,256,0,0,256)`. |
| `assets/materials/particle/bubbles/bubble3.tex` | `5b09546c9a78825a6e879a4b5e53aca3433c549114b359802f71a7bb06e20b6a` | TEXS3 at byte 394427, 64 frames. First record: image 0, duration 0.015625, components `(0,0,128,0,0,128)`. |
| `assets/materials/lut/neutral.tex` | `967ea45d38c40eda283d440bd80b914587c779ec6fbe1981e761d2443a68fa3b` | Flags 0x42, extra depth 32. Explicitly unsupported volume texture. |

A deterministic scan of 440 loose `.tex` files under the installation's assets
and projects produced:

| Result | Before | After |
| --- | ---: | ---: |
| Validated 2D pixel payloads | 370 | 412 |
| Parsed but invalid pixel payloads | 42 | 0 |
| Parse errors | 28 | 28 |

All 28 remaining errors are volume/LUT textures, now identified explicitly.
The scan does not cover textures inside PKG files, other installations, actual
GPU format support, or whether the owning scene's effects/particles can play.

Reproduce the current inventory with an explicit local path:

```sh
cargo run -p better-wallpaper-scene-format --example texture-audit -- /path/to/wallpaper_engine
```

The tool does not follow symlinks or start the application. It prints per-file
parser/payload results and totals; those errors are inventory results rather than
a failed command. The before/after logs are retained under `target/rea/` locally.

## Validation and remaining questions

- The new native-layout regression failed before the fix with an invalid mip size.
  The corrected test covers all twelve TEXB1–4 / TEXS1–3 combinations, exact pixels,
  frame coordinates, frame image indices and total duration.
- Malformed-layout tests cover count/codec collisions, volume flags, unsupported
  animation versions, excessive frame counts, missing images, non-rectangular
  bases and truncation at every byte within frame records.
- The offscreen GLES golden test exercises all three sprite versions at start,
  frame transition and loop boundary. All twenty comparisons passed on llvmpipe:
  SSIM 1.0, MSE 0, maximum channel error 0. References were not regenerated.
- `cargo clippy`, formatting and the scene-format, renderer, GPU, Wayland and KDE
  tests passed. The ordinary suite keeps one explicit GPU test ignored; the
  GPU check was also run explicitly.
- Full workspace testing first encountered a sandbox prohibition on an existing
  symlink-boundary test; rerunning outside the sandbox passed that test. Mock
  Wayland socket tests also required an unsandboxed run and then passed. It then
  failed `hardware_request_decodes_or_falls_back_to_software` in the unchanged
  FFmpeg crate: CPU pixel length 0, expected 9216. That crate depends only on the
  unchanged core crate, not the modified scene parser. Remaining package suites
  were run separately. The full workspace suite is therefore not claimed green.

Reproduce the focused checks:

```sh
cargo clippy
cargo fmt --all -- --check
cargo test -p better-wallpaper-scene-format -p better-wallpaper-renderer -p gpu-renderer
LIBGL_ALWAYS_SOFTWARE=1 cargo test -p gpu-renderer --test scene_golden -- --ignored --test-threads=1 --nocapture
```

**Partially answered:** runtime visual parity. The synthetic GPU tests establish
regression behavior, but no Wallpaper Engine reference capture, real niri session,
vendor-GPU comparison, or long soak was performed.

**Unresolved:** multi-image/rotated atlas rendering, 3D LUT upload, and complete
TEXB4 video/editor metadata semantics. Complete these with separate asset shapes,
explicit renderer representation, bounded allocation tests and reference output.
SceneScript, arbitrary effects, particle simulation and other native subsystems
were outside this focused investigation and remain separate work.
