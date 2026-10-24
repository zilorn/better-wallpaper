# A0 audit JSON syntax coverage

Date: 2026-10-24. Change: `fix: discover references in commented audit JSON`.
This closes the recorded syntax gap in the bounded asset inventory. It does not
change runtime scene/material parsing or establish native-engine acceptance.

## Behavior and regression evidence

The audit first parses strict JSON. On failure it accepts only `//` line comments,
non-nested `/* ... */` block comments and trailing commas before closing arrays or
objects. Normalization replaces those bytes with spaces, retaining newlines and
all string bytes. The existing strict parser validates the resulting document.
Original bytes still determine hashes, sizes and archive offsets. Strict documents
retain `json_syntax_valid`; extended documents use `json_extended_syntax_valid`.

Input remains capped at 2 MiB before allocating the temporary copy; UTF-8, nesting,
reference-count, string and pointer limits remain enforced. No single-quoted
strings, unquoted keys, nonfinite numbers, nested block comments or token joining
are supported. Unterminated comments and malformed documents produce `invalid_json`
and no partial references. Error line and byte-column positions refer to input.

The new loose/package regression failed on `audit.complete` before implementation
and passes after it. Five new tests cover source-byte identity, reference pointers,
string escaping, Unicode/URLs, malformed tokens, comment boundaries, recursion
limits, error positions and refusal to emit partial references. Existing budget
tests now exercise extended JSON and an oversized comment. No runtime parser or
GPU code changed.

## Local audit replay

Re-run the exact source selection in [the baseline](a0-asset-inventory.md#输入边界与复现)
with the current `asset-audit`. Two independent runs returned **0** and produced
byte-identical stdout (SHA-256
`803cc6c6763b648681aa94453d0cb7e0e47a1c8d08d83548282abc1a81d69a34`).
All 2,593 source/path pairs, sizes and hashes match the historical manifest;
only the 32 previously rejected JSON records changed. All old reference records
are preserved exactly, and the new candidates come from these 32 documents.

| Measure | Baseline | Updated audit |
| --- | ---: | ---: |
| Inventory records | 2,593 | 2,593 |
| Strict JSON successes | 2,025 | 2,025 |
| Extended JSON successes | 0 | 32 |
| JSON failures | 32 | 0 |
| Candidate references | 3,858 | 4,160 |
| Candidates found in scope | 2,149 | 2,264 |
| Candidates not found in scope | 1,657 | 1,844 |
| Non-file/unsafe candidates | 52 | 52 |
| `complete` / exit code | false / 2 | true / 0 |

Traversal is unchanged: 5,619 directory entries, 154,287,219 input bytes, 1,537
excluded loose files, no notices. TEX results remain 459 validated 2D payloads
and 28 explicitly unsupported volume textures.

The [machine-readable delta](a0-json-extensions-delta.json) preserves all 32 changed
records and 302 added candidates, source hashes, baseline/output hashes and summary
counts. The [historical manifest](a0-assets.json) is intentionally unchanged.
These two files retain the audit evidence without duplicating unaffected records
or including proprietary JSON bodies/assets.

`complete: true` applies only to the selected inventory and documented literal
reference fields. The additional 187 `not_found_in_scope` candidates are not proof
of missing installed files. Dynamic/script paths, other fields, excluded loose
shader/media files and native lookup priority remain outside this audit contract.

## Validation

- `cargo clippy`: passed.
- `cargo clippy -p better-wallpaper-scene-format --bin asset-audit --tests -- -D warnings`: passed.
- `cargo fmt --all -- --check` and `git diff --check`: passed.
- `cargo test -p better-wallpaper-scene-format`: 112 passed, including 13 audit tests.
- `cargo test --workspace`: 244 passed, 0 failed, 6 existing GPU cases ignored.
  The first restricted run hit `Operation not permitted` in the existing daemon
  symlink-boundary test. The rerun with local filesystem access passed. Final
  limit-assertion changes were followed by the full format-crate suite above.
- CLI probes: valid extensions exit 0; an additional malformed document exits 2,
  preserves the report and contributes no partial references.
- Full local audit replay: both runs exit 0, identical output, all input identities
  and old references preserved.

GPU tests were not explicitly rerun because this change affects only inventory
syntax inspection. Original-engine capture, desktop playback and long-running
soak/fuzz checks remain pending; A0 and the adaptation plan are not complete.
