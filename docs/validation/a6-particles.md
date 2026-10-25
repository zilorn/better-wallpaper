# A6 limited particle implementation and acceptance

Date: 2026-10-25. Scope was committed first in `733695d`; the
[subset and matrix](a6-particle-subset.md) record admission, evidence and intended
gates. This implements a limited 2D native-format adapter for the shared niri
scene path. It does not provide general particle or Plasma scene compatibility.
Implementation and acceptance commit: `a5320ad` (`feat: render bounded deterministic 2D particles`).

Merge integration on 2026-10-25 preserves the later A5 web protocol and A7
restricted arithmetic runtime. Both daemon preparation regressions remain;
particle admission explicitly rejects script-bearing transforms, and GTK's
English/Chinese explanation describes all three bounded capabilities. The merged
tree passed required Clippy, formatting, 331 workspace tests (20 opt-in cases
default-ignored), all 13 llvmpipe GPU cases, both GTK display workflows, fixture
reproduction and documentation-link checks.

## Implemented behavior

One boxrandom emitter, four explicit initializers (fixed lifetimerandom, linear
sizerandom/velocityrandom/colorrandom), force-free movement and a default sprite
renderer. Emission is either a burst at zero or continuous rate, optionally
finite duration. Position/velocity are XY, with zero Z. Sprite frame zero is the
whole unpadded static image; one base mip, one genericparticle material pass and
translucent/additive blending are required. TEX point/clamp sampler flags apply.

Unknown fields/operators, flags, nonzero control-point offsets, overrides,
scripts, effects, parented/animated/nonplanar particle transforms, random or
animated atlas frames, video/volume textures, shader variants, extra passes and
texture inputs fail shared admission. Rejected nodes retain specific compatibility
and English log reasons while supported scene draws continue. Package validation
can award L3 metadata; the tier does not establish playback or full-scene parity.
Non-power-of-two repeat sampling is rejected for the GLES2 contract; clamp
sampling remains available on such textures.

Samples use the daemon's f64 scene clock, not wall time or per-frame integration.
FNV-1a of the UTF-8 node ID plus SplitMix64 birth/channel counters fixes random
inputs. Pause freezes snapshots; resume uses scene time, rewind/reload reconstruct
without history. This does not reproduce the native random stream. When the
exact-integer birth range (2^52 intervals) is exceeded, sampling fails explicitly.

Capacity is bounded by native maxcount and the configured aggregate scene limit
(0–100,000, default 10,000), assigned in authored order. Fixed birth residues are
dropped under pressure; a dropped birth cannot appear later in its lifetime.
This stable thinning is a degradation policy, not native maxcount scheduling.
At most 64 systems, 100,000 concurrent candidate births per system, 8 MiB per
particle texture and 32 MiB retained scene particle texture bytes are admitted.
JSON inputs are capped at 128 KiB. Sampling work depends on reserved capacity,
not elapsed uptime; one batch contains at most 24,000,000 vertex bytes. Texture
decoding retains existing temporary-buffer limits in addition to these caps.

GTK exposes the saved particle limit, including zero/off. Configuration validation,
atomic persistence and playback rebuild behavior are unchanged. Scene preparation
applies the saved limit; controls do not enable excluded operators. GPU reload
releases batch-buffer storage and textures, and owning-context drop deletes buffers.
Repeated definitions share CPU pixel ownership; replacing contexts reuploads it.
Missing required particle GPU textures and failed uploads return explicit errors.
The recovery probe exposed acceptance of malformed raw pixels; converted RGBA
byte counts are now checked before GL submission, and failed texture objects are
deleted before a valid retry. Ordinary image failure policy remains unchanged.

## Native findings

Native target and REA identities are in the subset document. Ten owned original
program captures fix static color/alpha, additive/translucent overlap, explicit
scale, asymmetric UV orientation, rotation/nonuniform scaling, finite emission
and the negative starttime probe. No Better Wallpaper image generated a native
reference. Captured RGB values are independently checked and hashed in the
[manifest](../../tests/fixtures/particles/manifest.json).

Two corrections came from the probes:

- An authored size of 32 produces width 16 at scale one, and width 32 at scale
  two. The renderer applies this calibration before layer transforms and aspect.
- `starttime=60` does not postpone the native burst for sixty seconds. Nonzero
  start/prewarm behavior is excluded instead of implementing a guessed delay.
  The diagnostic input and capture remain in the matrix as a rejection case.

The native finite emitter at rate 10/duration 1 produces ten overlapping unit
RGB contributions after emission stops. It matches the bounded sampler. Exact
native fractional birth timing and RNG distribution/sequence identity remain
unverified; static probes and finite totals do not establish those properties.

## Validation

| Check | Result |
| --- | --- |
| `cargo clippy`, formatting and diff whitespace | Passed |
| Strict all-target Clippy for format/renderer/GPU/GTK | Passed |
| Strict daemon binary and particle niri test Clippy | Passed |
| Full workspace | 313 passed, zero failed, 18 default-ignored desktop/GPU cases |
| Full explicit GPU suite, NVIDIA RTX 3060 Ti and Mesa llvmpipe | 12 cases passed on each device |
| A6 native pixels on each device | 180 comparisons across nine positive probes; maximum channel error zero |
| CPU admission/sampling/assets | Birth/death boundaries, counter vectors, finite emission, frozen/rewound/long time, global budgets, actual 24,000,000-byte maximum staging, limits and ownership release passed |
| Native captures | Ten original-program PNGs independently recaptured; RGB bytes identical |
| Real niri DP-1 | 54 submissions, 1920×1080/scale 1/normal transform, three scene types, zero/seven budgets, reload and three backend/context replacements |
| GTK private display, English and Chinese | Both passed, including zero/seven particle-limit edits and saved configuration |
| Fixture generation, hashes and documentation links | Passed |

The broad `--all-targets -D warnings` command including the daemon library tests
encountered the existing `items_after_test_module` lint in `web_wallpaper.rs`
(`tests` at line 258 precedes `run_niri` at line 373). Required normal Clippy and
the affected daemon binary/integration target pass. No lint suppression or
unrelated daemon reordering was introduced. Earlier new-test lint findings were
fixed, and final scoped strict checks pass. GTK's private bus emitted existing
accessibility activation notices; both display workflows exited successfully.

Reproduction commands (from this worktree, with an optional shared build cache):

```sh
python3 tests/fixtures/particles/generate.py
cargo clippy
cargo fmt --all -- --check
cargo clippy -p better-wallpaper-scene-format -p better-wallpaper-renderer -p gpu-renderer -p better-wallpaper-gtk --all-targets -- -D warnings
cargo clippy -p better-wallpaper-daemon --bin better-wallpaper-daemon --test particles_niri -- -D warnings
cargo test --workspace
LIBGL_ALWAYS_SOFTWARE=1 __EGL_VENDOR_LIBRARY_FILENAMES=/usr/share/glvnd/egl_vendor.d/50_mesa.json cargo test -p gpu-renderer --test scene_golden -- --ignored --nocapture --test-threads=1
__EGL_VENDOR_LIBRARY_FILENAMES=/usr/share/glvnd/egl_vendor.d/10_nvidia.json cargo test -p gpu-renderer --test scene_golden -- --ignored --nocapture --test-threads=1
BETTER_WALLPAPER_NIRI_TEST_OUTPUT=DP-1 cargo test -p better-wallpaper-daemon --test particles_niri -- --ignored --nocapture
dbus-run-session -- xvfb-run -a env GDK_BACKEND=x11 GSK_RENDERER=cairo cargo test -p better-wallpaper-gtk -- --ignored --test-threads=1
dbus-run-session -- xvfb-run -a env GDK_BACKEND=x11 GSK_RENDERER=cairo BETTER_WALLPAPER_GTK_TEST_LANGUAGE=zh-CN cargo test -p better-wallpaper-gtk -- --ignored --test-threads=1
```

Native capture uses the existing isolated Wine/Xvfb harness, with a copied app
and prefix as described in [the atlas capture setup](../../tests/fixtures/atlas-native/README.md).
`python3 tests/fixtures/particles/capture.py /absolute/private/work` refuses to
overwrite recapture output. Use separate prefixes for simultaneous investigations.
Two early extra-probe passes shared one prefix and failed with a vanished X
window/missing client; the final probes were repeated in independent copies.
The finite PNG uses grayscale encoding; the comparison reader preserves its
original bytes/hash and expands channels for pixel comparison.

The bounded desktop check records actual submissions on one named output. It
does not read compositor pixels, verify hotplug/multiple outputs, or replace A9's
long-run and fuzz gates. Variable lifetimes, alphafade/forces, sphere emission,
trails, events/children, atlas animation, scripts and Plasma scenes remain excluded.
