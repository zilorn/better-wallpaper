# A1 bounded atlas model validation

Date: 2026-10-18. This is the first A1 implementation batch: independent image/mip
chains, full frame bases and opt-in structural inspection. Advanced scene/GPU
playback and native screenshot parity remain incomplete. The
[atlas investigation](../reverse-engineering/wallpaper-engine-atlases.md) records
binary/shader identities, static evidence, model contracts and remaining unknowns.

## Inputs and observed behavior

All five A0 candidate layouts now parse through `TexAtlas`; conversion to the
legacy playback type still rejects them. The multi-image fixture retains image 0
with 2×2 and 1×1 mips, image 1 with a 1×2 mip, and frames referencing images 0
and 1. Rotation, skew and both flips retain their original signed basis vectors.
The input packages, matrix and independent PNGs were not modified.

New tests check all twelve TEXB1–4/TEXS1–3 combinations, independent pixel bytes,
exact total-budget boundaries, count/index limits, byte-by-byte truncation,
nonfinite fields, duration rules, LZ4 output length and mismatched pixel sizes.
The CLI regression checks `--atlas --json`, payload status, explicit legacy
layout rejection and `native_visual_validation: pending`. Existing padding,
video and single-image playback behavior remain covered.

## Verification

Run from the repository root:

| Command | Result |
| --- | --- |
| `cargo test -p better-wallpaper-scene-format` | 104 passed: 94 library, 8 audit and 2 CLI tests; no failures |
| `cargo clippy -p better-wallpaper-scene-format --all-targets -- -D warnings` | Passed |
| `cargo test --workspace` | 230 passed, 0 failed, 1 explicit GPU case ignored in the ordinary invocation |
| `LIBGL_ALWAYS_SOFTWARE=1 cargo test -p gpu-renderer --test scene_golden -- --ignored --test-threads=1 --nocapture` | Explicit GPU case passed separately; 20 comparisons all SSIM=1, MSE=0, maximum channel error=0 |
| `python3 tests/fixtures/scene-behavior/generate.py` | All 27 generated files reproduced exactly |
| `cargo clippy` | Passed without warnings |
| `cargo fmt --all -- --check` | Passed |
| `git diff --check` and changed document-link checks | Passed |

The full suite ran with access to local GPU devices and temporary sockets/symlinks
required by existing tests. The explicit GPU run used surfaceless EGL/GLES with
llvmpipe (LLVM 23.1.1, 256 bits). Transient logs were retained at
`/tmp/better-wallpaper-a1-{format,workspace,gpu}.log`; the counts above do not
depend on keeping those files. Synthetic comparisons show the parser refactor
preserved the existing output. They do not validate the new advanced layouts on
the GPU, the original program, vendor GPUs, or actual niri/Plasma desktops.

## Follow-up acceptance

The new inspection path is deliberately stricter than legacy playback parsing
for complete animation blocks, finite values, image/mip counts, LZ4 length and
retained bytes. Its success certifies only the documented structural/payload
checks. Off-axis and negative bases are represented but not rendered; zero-
duration frames and video metadata remain outside timed sprite guarantees.

Next A1 work must connect bounded image resources and affine UVs to the shared
renderer, retain concrete unsupported-layout reporting, and add positive tests
for cross-image selection, rotation/flips, edge sampling, loop/pause and context
recreation. Original sample acceptance and deterministic captures are still
pending. The adaptation plan keeps those items open.
