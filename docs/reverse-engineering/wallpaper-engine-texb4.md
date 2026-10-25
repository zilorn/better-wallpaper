# TEXB4 editor metadata and embedded video

Investigated 2026-10-25 against the local Wallpaper Engine 2.8.42 installation
(Steam build 23967692). This report supersedes the earlier unverified TEXB4
video interpretation in the original texture investigation. It describes the
container reader and a bounded runtime consumer, not recovered source code or
complete editor/SceneScript compatibility.

## Identity and evidence

`wallpaper64.exe`: 5,360,112 bytes, SHA-256
`40e2ce021e9352324fadb3b8f72b8ba2a7ee95b71cc571d5b9f84be75cd993b0`.
REA used Hopper with profile digest
`32c01d9334c1fa8dee7c09790f43820490bcf0022af7efe157da365c2ab35983`.
The reader snapshot and complete evidence remain local, outside Git:
`target/rea/a3-reader-snapshot.json` and `target/rea/a3-texb4-evidence.json`.
The latter includes earlier observations imported from the A2 snapshot.

| Evidence | Location | Observation |
| --- | --- | --- |
| `ev_765c28b458e0680cbffec250d0b47bbb844bb70595125aa01966f6ce18ae8b86` | Batch decompile, `0x14015c8d0` | Resolves the TEXB reader; shows a counted editor-record loop preceding the image/mip loops. Large reconstructed control flow is checked against assembly below. |
| `ev_539893108dc437efd90bead989d00bf3440408b9eae1c3d768243c808accb639` | Reader assembly, `0x14015c8d0` | Image count at `0x14015c932`; codec at `0x14015c96e`; TEXB4 count at `0x14015c998`. `0x14015c9d4` branches on zero, and `0x14015d1b3`–`0x14015d1cc` increments/compares the record index. |
| Same assembly | `0x14015ca25`, `0x14015ca4f`, `0x14015ca74`, `0x14015ca93`–`0x14015cab9` | Reads three four-byte fields, then scans a NUL-terminated string. JSON condition handling is separate from the stream reads. |
| Same assembly | `0x14015d2b3`, `0x14015d323`, `0x14015d34e`, `0x14015d44c`, `0x14015d477`, `0x14015d4cb`–`0x14015d503` | Reads mip count, width/height, compression, decoded length, encoded length, then bounds/advances over the encoded payload. |
| Same assembly | `0x14015d53d`–`0x14015d572` | With nonzero editor count, additional per-mip variant groups follow the main payload. They are not fields preceding mip dimensions. |
| Same assembly | `0x14015d20f`; `0x14015dd64`–`0x14015de4c` | Flag `0x20` is tested independently of metadata count. Codec `-1` retains the payload instead of sending it through the FreeImage image decoder. |

The same assembly Evidence ID was already retained by the earlier investigation;
this pass examines previously unreported branches. Hopper's reconstructed types,
large pseudocode and incomplete indirect-call coverage are not original source.
No interpretation of the three editor integer fields is needed for parsing: the
model retains them in file order without inventing their authoring meaning.

## Corrected layout

All words below are little-endian `u32` unless identified otherwise. Markers
include their terminating NUL.

| Block | Fields |
| --- | --- |
| TEXB1 | Image count, image records |
| TEXB2 | Image count, image records with compression/decoded length |
| TEXB3 | Image count, FreeImage codec (`-1` for raw), image records |
| TEXB4 | Image count, FreeImage codec, **editor record count**, all editor records, image records |
| Editor record | Three opaque words, bounded NUL-terminated UTF-8 JSON string |
| Image record | Mip count, then mip records |
| Mip record | Width, height; TEXB2+ compression and signed decoded length; encoded length; exactly that many payload bytes |
| TEXB4 mip with editor records | Main mip record, followed by variant-group count and any variant groups |

Video is identified by TEXI flag `0x20`, independently of TEXB version and editor
count. Real selected files use flags `0x22`, codec `-1`, one image and one mip.
Their uncompressed payload begins at byte 87 for TEXB3 or 91 for TEXB4 with no
editor records. These offsets follow the verified fields; the parser does not
search for `ftyp` or guess an offset from a video signature.

The old implementation treated editor count `1` as an MP4 boolean, read two
words/string/one word inside each mip, and discarded the string. A valid static
TEXB4 with metadata could therefore be misclassified as video or rejected, and
real flag-marked TEXB4 with zero metadata had no faithful regression fixture.
The former synthetic video package encoded that old parser convention. It has
been corrected; its analytical output PNGs were not changed.

## Independent file observations

[Selected sample identities](../validation/a3-texb4-samples.json) record package
member offsets, sizes, SHA-256, dimensions, payload offsets and hashes. Reading
30 local workshop PKG indexes examined 554 TEX entries, finding six TEXB3 videos
and five TEXB4 videos. Fifty TEXB4 entries were present overall; none of those
fifty had editor records. These are bounded local observations, not prevalence
statistics or complete dependency/playability coverage.

Nine videos fit the existing 64 MiB TEX mip ceiling. Their parser-reported payload
hashes match independent byte extraction, and ffprobe recognizes each extracted
payload as H.264 MP4. All nine decode through the package-only FFmpeg runtime at
1280×720 and release their private extracted files on shutdown. The other two
videos (one TEXB3, one TEXB4) exceed 64 MiB and fail explicitly before allocation;
limits were not raised to admit these samples. Commercial media stays local and
is not distributed in this repository.

The 440 loose installed textures still produce 412 validated 2D payloads and 28
volume exclusions, with zero invalid 2D payloads. Owned fixtures separately cover
zero/one/two editor records, static/video data, PNG static mips, compression and
version differences. They establish parser behavior, not native editor rendering.

## Admission and unresolved behavior

`TexAtlas` and the legacy `TexTexture` retain `editor_metadata`. Empty strings and
JSON objects are admitted; malformed UTF-8/JSON, missing NUL, more than 256 records,
strings over 64 KiB or more than 1 MiB total serialized metadata fail. JSON is
bounded by serde_json's recursion limit and is never executed. Nonzero conditional
mip-variant groups return an explicit unsupported error; choosing variants from
properties/scripts is not implemented. Volume editor metadata remains explicitly
unsupported. No editor-condition evaluation is implied by metadata retention.

Video admission requires a nonanimated single image/single payload and raw codec,
valid dimensions, nonempty media, exact LZ4 length and no trailing bytes. The
64 MiB encoded/decoded mip cap, compression-ratio cap and caller byte budget
apply. Static TEXB4 uses the ordinary image decoder, not the video path. Both parser
adapters enforce bounded mip counts, exact LZ4 lengths, pixel-size validation
and a 128 MiB retained-byte ceiling for TEXB4.

Scene video uses the existing shared pause-aware scene clock and package-only
FFmpeg restrictions. No native clock/screenshot equivalence, native editor
condition equivalence, Plasma scene consumer or video-texture audio mixing is
claimed. Desktop hotplug/reconnect and eight-hour soak remain A9 work. See the
[A3 acceptance record](../validation/a3-texb4.md) for tests and reproduction.
