# A0 behavior sample matrix validation

Date: 2026-10-18. Commits: `07c9d63` (flipped-frame rejection), `54a6ff2`
(standalone samples and matrix-driven regression). This completes a scoped A0
sample batch, not all planned behavior coverage or original-program parity.

## Inputs and resulting behavior

[`tests/fixtures/scene-behavior/README.md`](../../tests/fixtures/scene-behavior/README.md)
documents 13 original synthetic projects: eight renderable scenes and five A1
candidate layouts. Their 26 project/package artifacts total 51,185 bytes. Together
with the matrix, all 27 generated files reproduce byte-for-byte. The matrix SHA-256
is `863856f7e617d635c12d0d8276b8975f3138ef8f8df8fd8d981dea1a9d78c15d`.

The GPU regression now reads the same committed packages and ordered inputs
recorded in the matrix. It no longer constructs a separate in-memory copy of
scene assets. The ordinary test verifies project/package/PNG hashes, bounded
dimensions/times/pointers, declared draw counts and rejection outcomes. The
existing ten analytical PNGs and numerical comparison gates were not changed.

The five A1 probes cover independent image/mip chains, off-axis rotation/skew and
negative-direction horizontal/vertical bases. A separate wire-layout inspection
confirmed the multi-image package contains image 0 mips 2×2 and 1×1, image 1 mip
1×2, and two 0.5-second frames selecting image indices 0 and 1; it consumes the
entire TEX payload. This is a fixture-integrity check, not native UV evidence.

Sample construction exposed a previously unreported failure: negative-direction
bases passed TEX parsing, then `sprite_animation` discarded those frames and
could leave a static full atlas. The new unit regression failed before the fix.
Parsing now rejects both directions explicitly in TEXS1/2/3. Scene resolution
skips the failed image node, preserving its existing partial-scene behavior.
Flip rendering itself is still unimplemented.

## Verification

Run from the repository root:

| Command | Result |
| --- | --- |
| `cargo test -p better-wallpaper-scene-format --lib` | 86 passed; new flip regression checks both directions across all three TEXS versions |
| `python3 tests/fixtures/scene-behavior/generate.py` | 27 generated files reproduced exactly; read-only check |
| `cargo test -p gpu-renderer --test scene_golden` | 2 passed; explicit GPU case ignored in the ordinary invocation |
| `LIBGL_ALWAYS_SOFTWARE=1 cargo test -p gpu-renderer --test scene_golden -- --ignored --test-threads=1 --nocapture` | Explicit GPU case passed; all 20 comparisons SSIM=1, MSE=0, maximum channel error=0 |
| `cargo test --workspace` | 220 passed, 0 failed, 1 explicit GPU case ignored; that case passed separately above |
| `cargo clippy` | Passed |
| `cargo clippy -p gpu-renderer --tests -- -D warnings` | Passed for the changed test target |
| `cargo fmt --all -- --check` | Passed |
| `git diff --check` and changed fixture-document link checks | Passed |

GPU readback used surfaceless EGL/GLES and llvmpipe (LLVM 23.1.1, 256 bits), with
no visible desktop surface. The full suite ran with access to local GPU devices
and temporary sockets/symlinks required by existing tests. The local transient
full-suite log is `/tmp/better-wallpaper-a0-matrix-tests.log`; conclusions do not
depend on that uncommitted log being retained. This run is not new vendor-GPU
rendering or actual niri/Plasma desktop verification.

## Original capture status and remaining work

Read-only inspection found Arch Linux/Wayland, Wine 11.20, Proton Experimental,
Xvfb and an existing app 431960 compatibility prefix. No Wallpaper Engine/Wine
process matched at inspection. Original-program launch, sample acceptance and
deterministic scene-time control were not tested in this batch. Thus no native
screenshots were captured; runtime availability alone proves neither successful
execution nor inability to execute. All matrix entries remain `native_reference:
pending`.

The sample README defines fixed inputs, capture provenance, dimensions, timing
requirements and pre-registered comparison thresholds. Internal video frame
replacement/context recreation is explicitly separated from native screenshot
steps. Next work must establish original acceptance/UV consumption for the A1
candidates and turn them into positive rendering cases when implemented.

A2 LUT effects, native A3 video metadata, broader A4 effects/motion, and the
defined subsets for A5–A8 still need additional minimal samples. The A0
all-behavior and original-reference checkboxes therefore remain open.
