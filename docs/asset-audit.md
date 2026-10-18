# Bounded asset inventory

`asset-audit` inventories explicitly selected local sources without extracting
archives, executing Wallpaper Engine, opening a GPU context, or fetching URLs.
It emits JSON on stdout. Source names are logical labels; keep the command or a
source-path mapping with each report, because absolute input roots are intentionally
omitted from the JSON. Every successfully read file/member has a relative path,
byte count and SHA-256. A PKG container record has an empty path; its members have
their package paths and absolute byte offsets within that hashed container.

```sh
cargo run -p better-wallpaper-scene-format --bin asset-audit -- \
  --shared builtin=/path/to/wallpaper_engine/assets \
  --root projects=/path/to/wallpaper_engine/projects \
  --pkg chosen=/path/to/chosen/scene.pkg \
  --pkg synthetic=tests/fixtures/dynamic-scene/scene.pkg > inventory.json
```

Each argument is repeatable and uses a unique `NAME=PATH`. At least one source is
required, at most 32 may be supplied. Source names and file paths are sorted so
unchanged inputs and limits produce identical reports. Sources are inspected in
name order, which also determines budget allocation. Loose trees select only TEX
and JSON, case-insensitively; other loose files are counted as excluded. A PKG
must be explicitly selected: every accepted member is hashed, including opaque
media, models and shaders. PKGs discovered under loose roots are not opened.

Use a quiescent tree. Symlink entries, symlink roots and non-regular input files
are not followed. Final file opens also use `O_NOFOLLOW` and `O_NONBLOCK`; inode,
size and timestamp checks detect common concurrent replacements/writes. This is
an offline inventory, not a transactionally consistent filesystem snapshot or a
sandbox for a hostile process concurrently changing ancestor directories.

## Bounds and exit codes

| Option | Default | Hard maximum |
| --- | ---: | ---: |
| `--max-walk-entries` | 20,000 | 100,000 |
| `--max-records` | 10,000 | 100,000 |
| `--max-depth` | 16 | 64 |
| `--max-file-bytes` | 128 MiB | 256 MiB |
| `--max-total-bytes` | 1 GiB | 16 GiB |
| `--max-decoded-bytes` | 128 MiB per TEX | 256 MiB per TEX |
| `--max-references` | 100,000 | 1,000,000 |

Byte-valued CLI options take integer bytes. JSON parsing additionally caps each
document at 2 MiB, reference strings/keys at 512 bytes and JSON pointers at 4 KiB.
Existing PKG entry/count/aggregate limits and TEX per-mip/ratio/frame limits still
apply. A directory that would exceed the remaining walk budget is skipped whole,
rather than selecting a nondeterministic `readdir` prefix. Notices are capped at
256 (or the smaller record budget); `complete: false` always preserves the fact
that coverage was incomplete even when later diagnostics are suppressed.

Only one file/package and one decoded texture are processed at a time; package
members borrow the already counted archive bytes. `max_decoded_bytes` limits
retained decoded mip data through `TexTexture::parse_with_budget`. The ordinary
`TexTexture::parse` contract is unchanged. An additional mip's encoded/decoded
buffers can temporarily coexist, each under existing per-mip limits. These are
input/work/output-data budgets, not an exact process RSS or execution-time limit.
The changed-file check may read one byte past the initial stat size to detect
growth; those bytes count against the read accounting.

- Exit 0: selected inventory and known JSON reference inspection completed.
  TEX rejection is an inventory result and does not alone cause exit 2.
- Exit 2: valid JSON report, but coverage is incomplete (for example a traversal
  limit, unread file, rejected PKG, invalid/oversized JSON or reference limit).
- Other nonzero: invocation or output failure; do not assume a complete report.

`complete` never means a scene is playable. A fully inventoried volume texture
can have `complete: true` and `failure_class: unsupported_volume`.

## Formats and reference evidence

TEX inspection reuses the production parser and pixel-size validator. Reports
separate `validated_2d`, `parsed_video_unverified`, `parse_error` and `payload_error`.
They retain dimensions, TEXB version, pixel format, flags, mip/frame counts and
decoded byte count when parsing succeeds. Rejected files retain hashes and error
classes; a recognized TEXV/TEXI header also retains its raw format ID and flags.
Classification covers volume layouts, multiple images, off-axis frames, unknown
versions/formats, limits, malformed data and invalid pixel payloads. Passing a
pixel-size check is not GPU upload, media decoding or visual validation.

JSON inspection is strict syntax checking plus **candidate** reference discovery,
not complete scene/material/script semantic validation. It records literal strings
under `image`, `material`, `model`, `puppet`, `particle`, `sound`, `video`, `file`,
`textures` and `dependencies`, including array items and their JSON pointers.
Texture names use the existing `resolve_texture_path` rule. Parent traversal,
absolute paths, URLs, empty values and `_rt_` render targets are not filesystem
candidates. No reference causes additional filesystem reads or extraction.

Candidates are matched against the same source's inventory and explicitly shared
sources, never unrelated selected packages. In a loose tree, the nearest
inventoried `project.json` or `scene.json` establishes a candidate project root.
All matching paths are retained without claiming runtime lookup priority.
`not_found_in_scope` means absent from this selected inventory, not globally
missing. Loose PNG/audio/shader files, dynamic/property/script-generated paths,
unlisted JSON fields, object-style texture maps, TEX-embedded editor JSON and
references inside rejected JSON remain outside this reference-discovery subset.
Comments and trailing commas are reported as strict JSON failures, without
asserting that Wallpaper Engine rejects those files.

The manifest contains metadata, hashes and selected path strings only. Do not
add proprietary resource bytes or full JSON documents to the repository.

## Regression checks

```sh
cargo test -p better-wallpaper-scene-format
cargo clippy
cargo fmt --all -- --check
```

Synthetic tests cover hash/offset identity, the scene→model→material→texture
chain, shared roots, isolation between packages, unsafe paths, deterministic
output, malformed and unsupported TEX, cumulative mip budgets, traversal/read/
record/reference/JSON limits, symlinks and rejected archives.
