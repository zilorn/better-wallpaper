# A7 restricted SceneScript acceptance

Date: 2026-10-25. The [scope/feasibility record](a7-scenescript-scope.md), delivered
in `e4996b6`, fixes the bounded subset and resource/lifecycle contracts. The
[survey](a7-script-survey.json) records 900 script bindings across 30 local
projects; candidate member counts include comments/dead code and do not measure
executions, supported projects or playback success.

## Implemented boundary

The niri scene path now honors `scene.script_enabled` (default false) with a
finite Rust numeric evaluator. It compiles an entire `update(value)` export,
with explicit semicolons, scalar let/const locals, absolute numeric assignments,
arithmetic, selected Math functions and Vec3 results. Alpha and ordinary XY
origin/scale are the consumers. Root-to-leaf motion and alpha reach the shared
GPU submission path rather than stopping at metadata/configuration.

`engine.runtime` uses the daemon's elapsed scene clock. Declared numeric slider
defaults are read from project.json and overlaid with existing configuration
values; numeric-looking text/combo properties are not silently converted. The
descriptor must remain inside the canonical project root and fit 1 MiB/256
properties. Saving configuration uses the existing atomic persistence and
playback rebuild, so edited properties reach a newly compiled plan. No new API
route or GTK toggle was added; GTK continues preserving the existing field.

Selected static numeric own-layer members and literal-name `thisScene.getLayer`
reads are available. Ambiguous names, dynamic source layers, foreign writes,
combined timelines, container alpha, puppet motion, text/effect/particle scripts,
other lifecycle exports, mutable previous values, shared state, timers, input,
audio/media APIs and arbitrary JavaScript remain explicit capability limits.
Library reports retain admitted bindings alongside limitations, require opt-in,
and never award L4. Metadata without project numeric values retains an unresolved
property limitation; it is not a full runtime asset/property validation.

Limits are active: 64 wrappers and 64 KiB source per scene, 8 KiB/512 tokens per
binding, 32 expression levels/statements, 16 locals and 256 numeric instructions.
The operation ceiling replaces a wall-time timeout: there is no loop, recursion,
heap-object allocation, dynamic code, filesystem/network host call or async job.
A fixed numeric stack/register set serves evaluation. Non-finite arithmetic,
results above 1e6 or changed transform Z disable just that binding, log its
identity once, and retain its saved property until reload. Clone-shared fault
flags and nonblocking clock caches prevent output/descendant duplication of
normal script work. Paused time freezes results; rewind recomputes absolute
expressions. There are no script worker threads, callbacks or GPU handles.

## Independent reference evidence

Reference program: Wallpaper Engine 2.8.42/build 23967692, `wallpaper64.exe`
SHA-256 `40e2ce021e9352324fadb3b8f72b8ba2a7ee95b71cc571d5b9f84be75cd993b0`.
No binary reverse engineering was needed for this work: public host
documentation and owned input/output probes establish the selected semantics.
The [fixture manifest](../../tests/fixtures/scenescript-native/manifest.json)
pins inputs, executable identity and eight native screenshots. Capture runs use
a copied app/prefix and private Xvfb display, without changing the saved user
configuration or live service. Every native capture repeated with identical RGB.

Seven positive images cover absolute origin/scale, alpha Math operations,
own/named-layer reads, a numeric slider default and time-invariant runtime access.
The eighth probe repeatedly adds to the previous vector: the marker leaves the
canvas. This disproved the initial saved-value-per-frame assumption. The final
compiler rejects that stateful form; its black native screenshot is negative
evidence, never the expected output for a supported/fallback draw. Expressions
must assign changed components absolutely before reading them. Unchanged
components can pass through `return value`.

Time-dependent motion is compared with independently calculated coordinates at
0, 0.25, 0.5, repeated 0.5, 1 and rewound 0 seconds. These analytical checks are
not original-program clock captures. The original runtime rolls over and Wine
frame timing was not synchronized; no exact fractional/long-uptime native clock
equivalence is claimed.

## Validation

- The full workspace tests and required Clippy pass; formatting and strict
  all-target checks for format/renderer/GPU plus the niri integration target pass.
  The final totals are recorded in the adaptation plan.
- Both actual GPUs, NVIDIA RTX 3060 Ti and Mesa llvmpipe, pass all twelve explicit
  GPU cases, including the eleven prior atlas/LUT/Tint/motion suites. A7 adds
  184 native/analytical/lifecycle comparisons per device. Seven positive native
  references use SSIM >=0.995, RGBA MSE <=1 and max channel error <=2; analytical
  integer motion and disabled/fault fallbacks use exact RGBA. No threshold or
  existing reference was changed.
- CPU tests cover grammar/precedence, unsupported APIs, recursion/loop/dynamic
  code rejection, const/ASI/increment semantics, budgets, malformed serialized
  instructions, non-finite math, type/capability faults, mixed reporting without
  L4, numeric defaults/overrides, descriptor size/root boundaries, shared-output
  fault isolation, parent order, pause, rewind and fresh-plan recovery.
- Live niri on DP-1 (1920x1080, scale 1, normal transform) completed 90 submissions
  across origin, scale, property, motion and fault layouts, fresh-plan reloads,
  frozen/rewound time and five backend/context replacements. This is presentation
  and lifecycle evidence, not compositor pixel readback or physical color parity.
- Input regeneration and the bounded local survey reproduce their saved data.
  All eight original-program references repeat with identical RGB.

The first unrestricted parallel dependency build was killed; limiting Cargo to
two jobs completed validation. Reference-environment preparation initially
followed Wine drive symlinks and exhausted a temporary quota. That copy was
stopped and completely removed, then recreated with symlinks preserved. Neither
failure was hidden by skipping tests or relaxing assertions.

Reproduce from this worktree/repository root:

```sh
cargo clippy -j 2 --offline
cargo fmt --all -- --check
cargo test -j 2 --workspace --offline
cargo clippy -j 2 -p better-wallpaper-scene-format -p better-wallpaper-renderer -p gpu-renderer --all-targets --offline -- -D warnings
cargo clippy -j 2 -p better-wallpaper-daemon --test scenescript_niri --offline -- -D warnings
python3 tests/fixtures/scenescript-native/generate.py
python3 tests/fixtures/scenescript-native/survey.py --root /absolute/workshop/content/431960
__EGL_VENDOR_LIBRARY_FILENAMES=/usr/share/glvnd/egl_vendor.d/50_mesa.json LIBGL_ALWAYS_SOFTWARE=1 cargo test -j 2 -p gpu-renderer --test scene_golden --offline -- --ignored --test-threads=1 --nocapture
__EGL_VENDOR_LIBRARY_FILENAMES=/usr/share/glvnd/egl_vendor.d/10_nvidia.json cargo test -j 2 -p gpu-renderer --test scene_golden --offline -- --ignored --test-threads=1 --nocapture
BETTER_WALLPAPER_NIRI_TEST_OUTPUT=DP-1 cargo test -j 2 -p better-wallpaper-daemon --test scenescript_niri --offline -- --ignored --nocapture
```

The printed renderer identity is authoritative. Logs are `/tmp/a7-*.log`; the
independent capture directories are `/tmp/a7-native/recapture` and `repeat`.
Broader stateful SceneScript, callbacks/objects, exact native clocks, Plasma
scenes, multiple desktop outputs/hotplug/restart and A9 soak/fuzz remain separate
work. No entire surveyed commercial project is declared newly faithful.
