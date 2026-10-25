# A3 TEXB4 video and editor metadata acceptance

Date: 2026-10-25. Implementation commit is recorded in the
[A3 adaptation-plan entry](../wallpaper-engine-adaptation-plan.md#a3texb4-视频与元数据).
Native target identities, addresses and Evidence IDs are in the
[TEXB4 investigation](../reverse-engineering/wallpaper-engine-texb4.md).

## Correctness and scope

TEXB4's extra word is an editor-record count, independently of video flag `0x20`.
The parser reads all bounded metadata before the images, retains the three opaque
words and JSON text, and consumes the empty per-mip variant count after each main
payload. It no longer invents an MP4 FreeImage codec or interprets one metadata
record as video. Static TEXB4 remains on the normal pixel/mip path. The CLI reports
metadata, video detection, payload byte counts and SHA-256 separately from decoding.

The pre-fix parser rejected an independently authored two-record, one-pixel static
TEXB4 with `Atlas mip count 0 is outside 1..=32`. The corrected parser and renderer
now preserve its pixel `[23,67,119,255]`, report two metadata records and do not
award video compatibility. The old synthetic video package is corrected by 13
bytes to the verified field layout; the media and all reference PNGs are unchanged.

The bounded metadata contract admits empty strings or strict JSON objects with
at most 256 records, 64 KiB/string and 1 MiB serialized metadata. Nonempty mip
variant groups and volume editor metadata fail explicitly; conditional property
or script evaluation remains outside this consumer. Invalid UTF-8/JSON/NUL
termination, counts, dimensions, lengths, LZ4, budgets and truncated inputs fail.
Both TEXB4 adapters enforce mip-count, nonempty payload, exact LZ4 length,
pixel-size and retained-byte checks. A new regression first demonstrated the
legacy adapter accepting malformed static TEXB4 mips; it now rejects them.
No claims of native editor-condition rendering or native video clock parity follow
from retaining these records.

## Samples and repeatability

[Owned fixtures](../../tests/fixtures/texb4/README.md) cover static/video inputs
and two metadata records. Unit cases additionally cover zero/one records, all
TEXB1–4 video layouts, LZ4 and static TEXB3/4 PNG mips. Every prefix of the small
owned files is tested in both parser entry points. Count/string/aggregate-byte
limits and decoded-size/compression-ratio failures have explicit regressions.
Resource and compatibility tests resolve the committed static/video metadata
fixtures through package model/material references.

```sh
python tests/fixtures/texb4/generate.py
python tests/fixtures/scene-behavior/generate.py
cargo test -p better-wallpaper-scene-format -p better-wallpaper-renderer
cargo test -p better-wallpaper-daemon scene_video -- --nocapture
```

[The real-sample manifest](a3-texb4-samples.json) records 11 local video TEX
members found among 554 TEX entries in 30 workshop package indexes. Six are
TEXB3 and five TEXB4, all with flags `0x22`, raw codec and one media mip. Nine
fit the existing 64 MiB TEX mip ceiling: their parsed bytes match independent
extraction exactly, ffprobe recognizes H.264 MP4, and the daemon decodes their
first frame at 1280×720 with 3,686,400 retained RGBA bytes. One TEXB3 and one
TEXB4 exceed the ceiling and are explicitly rejected. Commercial bytes remain
local; only identities and probe results are committed.

Recheck the recorded members using an explicit installation root:

```sh
cargo build -p better-wallpaper-scene-format --bin scene-inspect
python tests/fixtures/texb4/verify_local.py \
  --steamapps /absolute/path/to/Steam/steamapps \
  --scene-inspect "$PWD/target/debug/scene-inspect"
BW_LOCAL_VIDEO_TEX=/absolute/path/to/extracted-video.tex \
  cargo test -p better-wallpaper-daemon --lib \
  local_video_tex_decodes_and_releases_runtime_resources -- --ignored --nocapture
```

The final command was executed once for each of the nine admitted local TEX
files. Paths must be absolute because Cargo runs the test from its crate folder.
A first invocation with a relative path failed to find its input; the absolute
path rerun and all nine subsequent runs passed. The optional local test remains
ignored in an ordinary checkout, which does not contain commercial samples.

The loose-texture audit still reports `total=440 validated_2d_payloads=412
parse_errors=28 payload_errors=0`; all 28 exclusions are the unchanged volume
boundary of this legacy 2D adapter. The 50 TEXB4 entries in the workshop index
scan all have metadata count zero. Nonzero metadata regression inputs are owned
synthetic records corroborated by native reader assembly, not claims of actual
native editor project acceptance.

## Clock, bounds and cleanup

The committed embedded-video package drives the actual daemon decoder/runtime.
Controlled times 0, .499, .5, .999, 1, 1.499, 1.5, 1.999, 2 and 4.5 seconds check
all four colors, frame boundaries and repeated loops. Waiting for a future queued
PTS ensures assertions observe the newest due frame, including after a clock
jump. Frozen times preserve visible frame generations. Three fresh scene loads
restart at red and verify shared decoding for duplicated draws/outputs.

Tests verify mode-0600 private files, cancellation/join with full queues, file
removal, weak-reference release of encoded media and retained RGBA after dropping
runtime/assets, eight decoders for nine distinct references, and rollback if a
second extraction fails. Corrupt bytes and a network-referencing HLS playlist
fail through the package-only decoder and still remove extracted files.

Production limits and decoding policy are preserved: eight decoders, two queued
frames each, 128 MiB runtime inputs (TEX media additionally has its 64 MiB mip
ceiling), 64 MiB RGBA frames, 720/1080/2160 height caps, file-only MOV/Matroska/AVI
and disabled MOV external references. Runtime file creation is factored behind a
private factory solely to test cleanup in isolated directories. No video-texture
audio is mixed; background scene audio remains the separate audio consumer.

## Validation results and limits

- Required `cargo clippy`, format check and the changed format/renderer crates'
  all-target strict Clippy pass.
- Full `cargo test --workspace`: 286 passed, zero failed, 13 default-ignored
  desktop/GPU/local-media cases. The restricted run first failed two existing
  filesystem-boundary tests (`server::tests::api_and_wallpaper_assets_work_without_management_html`
  and `web_wallpaper::tests::web_projects_keep_nested_paths_defaults_and_asset_boundaries`)
  with permission errors. The complete run with local filesystem/socket access
  passes; the tests were not skipped or relaxed.
- All nine explicit offscreen GPU cases pass on each actual renderer:
  NVIDIA GeForce RTX 3060 Ti and Mesa llvmpipe (LLVM 23.1.1). Existing native
  atlas/LUT captures and analytical PNGs are unchanged. The corrected video
  package still passes frame upload, storage resize and context replacement.
- All nine admitted real videos pass the explicit local-media decode/cleanup
  test. The two larger samples retain explicit size-limit failures.
- Both owned fixture generators reproduce their committed inputs byte for byte;
  real sample re-verification matches identities, parsed media hashes and ffprobe
  results. Documentation links are checked separately.

Ordinary tests leave the nine GPU cases, two niri probes, GTK display probe and
new local-media probe ignored. The GPU and local-media checks were explicitly
executed; live niri/Plasma display checks were not repeated for this parser/runtime
change. No new original-program video screenshots/precise timing comparison,
conditional editor consumer, audio mixing, hotplug/reconnect or long soak is
claimed. These remain separate extensions/A9 gates and are not parser acceptance.
