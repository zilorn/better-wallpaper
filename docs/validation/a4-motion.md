# A4 motion, parallax and integration acceptance

Date: 2026-10-25. This completes A4's bounded common-effect/motion work alongside
[Tint acceptance](a4-tint.md), without claiming general shader or SceneScript
compatibility. The [owned matrix](../../tests/fixtures/motion-native/README.md)
and [manifest](../../tests/fixtures/motion-native/manifest.json) fix all inputs,
native identities, pointer coordinates and comparison gates.

## Native evidence and corrections

Reference target: Wallpaper Engine 2.8.42/build 23967692, x86-64
`wallpaper64.exe`, SHA-256
`40e2ce021e9352324fadb3b8f72b8ba2a7ee95b71cc571d5b9f84be75cd993b0`.
REA/Hopper profile digest:
`32c01d9334c1fa8dee7c09790f43820490bcf0022af7efe157da365c2ab35983`.
Static observations are distinguished from the independently repeated native
captures; neither is reconstructed original C++ source.

| Evidence | Observation |
| --- | --- |
| `ev_2fdd2269999d99c493c70b378808f51f5c1c0b8d84633115c406d0bc90756596` | Parallax setting strings at `0x140488ae0`, `0x140489100`, `0x14048e950`, `0x14048e968`. |
| `ev_c61786a00bc5cfc315dde7012d435dc1509b5b42b8277bdcaf8fcc4ce2f486c9` | Registration `0x140199780`: amount/delay/influence offsets `0x334/0x338/0x33c`. Its large pseudocode is registration evidence, not an execution trace. |
| `ev_e8116c0f1496856026c48de7faea1bac9faeeab7fef480d3eb4156fe1c045398` | `0x1401e0530` registers origin, scale, angles and layer depth. |
| `ev_ebb06459d0245613fdd2e8826e452195df5faa22b3b02f8b7362a5a4efa89bb6` | Camera update `0x1401891a0`, especially `0x140189c0d`–`0x140189c79`: positive delay selects capped per-frame interpolation. |
| `ev_6dc337c3d8c7fd0df9970da598263cd42585fc783f32f32ab6fcc4657edfd866` / `ev_498e84064d646d31d9def05195fee48a3c62a64200ccae01523b3d6d3a76ce30` | Constants at `0x140492830` and `0x140492868` are IEEE floats 3 and 10. |

`ev_5ccd156857ce073b95fe37eff3da97f0cf066dfd16fe4959f9bf932e53de747f`
records the constructor bytes at `0x140186fa5`–`0x140186fc5`, which initialize amount
0.5, delay 0.1 and mouse influence 0.5. The omitted-parameter capture agrees with
the explicit 0.5×0.5 operation. Earlier runtime defaults (0.1 and 1) were corrected.

The measured displacement is `-pointer * amount * influence * depth` times half
the authored projection on each axis, followed by the same cover/zoom transform
as the layer. On the 128-pixel square, pointer X=96, amount=0.25 and depth=1 move
the marker eight pixels left. A 256×128 authored projection moves it sixteen
pixels left on the same square client. The previous implementation omitted this
aspect factor. Negative depth reverses motion; zoom scales it; zero amount or
influence and axis-zero depth suppress the corresponding movement.

Delay zero applies input immediately. For positive delay, the native instruction
sequence yields `alpha = min(1, dt * 10 * (1 - delay / 3))`, then moves the current
position toward the target by alpha. The former exponential time-constant model
was incorrect. The admitted delay range is 0–3, avoiding negative native steps;
3 holds position. Repeated paused scene times never consume pointer movement,
rewind resets state, and asset clear/reload resets smoothing. This native per-frame
recurrence depends on cadence; it does not promise frame-rate independence.

Eighteen owned captures establish additive transform channels for both observed
values of editor `relative`, empty-axis fallback, and absolute scalar alpha/zoom.
For example saved scale 1 plus final channel 2 gives scale 3, not 2. The new shared
motion plan evaluates root-to-leaf matrices on scene time, before cover, zoom and
parallax. Zero/negative scales remain ordinary finite 2D transforms. Parent order,
frozen/rewound time, loops and property-bound saved values have CPU regressions.

## Bounded contracts and reporting

Position/scale channels are linear autonomous XY offsets; Z channels must be
empty/zero. Each channel has at most 1,024 finite keys, unique frames within the
authored length, fps in (0,1000], length in (0,1e9] and offset magnitude ≤1e6.
Motion plans share authored transforms and cap animated ancestry at 64 nodes;
non-finite composed vertices fail explicitly. Scalar frame arithmetic now uses
f64 scene time, preserving fractional loop positions after long uptime.

Unsupported curves, scripts, event/start-paused schedules, wrap-loop behavior,
angles/size/depth channels retain the saved transform and node-specific reports.
Puppet and procedural audio-spectrum motion consumers remain excluded; runtime
logs/skipped counts identify those cases. Mixed scenes keep supported features
and limitations together. Invalid scene IR errors are now visible in library/GTK
compatibility reports. Two old reporting fixtures that omitted required effect
passes were corrected to supply valid inputs; their assertions were retained.
Disabled effects no longer grant a tier or an unsupported-operation warning.

The selected local seven transform timelines still require scripts, Bezier or
scheduling. No claim is made that these whole projects now play faithfully.
The bounded Tint operation still excludes linear masks, mode 20, general stacks,
HDR and custom shaders. Bloom/camera shake remain reported as ignored.

## Verification

- All 18 native endpoint images independently repeat with identical RGB. The
  smoothing trace matches four visible per-update centers within 0.5 pixel.
- Both NVIDIA RTX 3060 Ti and Mesa llvmpipe pass the full eleven explicit GPU
  cases, including all previous atlas/LUT/video comparisons. New A4 motion checks
  make 230 endpoint/trajectory comparisons per device, all pixel-exact; Tint has
  184 comparisons per device, maximum channel error one, including fractional
  alpha, zero/disable, frozen time, missing-mask failure/recovery and reload.
- The live niri test makes 108 accepted submissions for six Tint/motion/parallax
  layouts on DP-1 at 1920×1080, scale 1, normal transform, with six backend/context
  replacements. It changes no saved configuration or service state.
- Required Clippy, targeted all-target strict Clippy, niri-test strict Clippy,
  formatting, generated inputs and the hash-preserving 30-package survey pass.
  Final workspace test totals are recorded in the adaptation plan.

The initial restricted workspace run hit two existing file-boundary permission
failures; the complete run with local access passed. The new runtime-error report
also exposed an invalid existing reporting fixture; its missing pass was fixed.
No test was skipped or threshold relaxed to hide these results. Full GPU suites
passed before the final additive Z/default guards; final targeted A4 suites and
workspace checks were repeated after those changes.

The niri probe verifies presentation/context ownership, not compositor pixel
readback or real pointer focus over other windows. Standard Wayland supplies only
wallpaper-surface input; global tracking remains unavailable. Multi-output
hotplug, compositor restart, other GPUs, Plasma scenes and the A9 soak/fuzz gates
remain separate work.

Reproduction commands:

```sh
cargo clippy
cargo fmt --all -- --check
cargo test --workspace
cargo clippy -p better-wallpaper-scene-format -p better-wallpaper-renderer -p gpu-renderer --all-targets -- -D warnings
python3 tests/fixtures/motion-native/generate.py
python3 tests/fixtures/tint-native/generate.py
python3 tests/fixtures/tint-native/survey.py --root /absolute/steamapps/workshop/content/431960
cargo test -p gpu-renderer --test scene_golden -- --ignored --test-threads=1 --nocapture
BETTER_WALLPAPER_NIRI_TEST_OUTPUT=DP-1 cargo test -p better-wallpaper-daemon --test effects_motion_niri -- --ignored --nocapture
```

Select Mesa explicitly with `__EGL_VENDOR_LIBRARY_FILENAMES=/usr/share/glvnd/egl_vendor.d/50_mesa.json`
and `LIBGL_ALWAYS_SOFTWARE=1`, or the NVIDIA vendor JSON for that device. Use the
printed renderer identity as evidence. Local logs are `/tmp/a4-*.log`; native
captures/packages are under `/tmp/a4-native/`, and ignored REA exports/snapshots
are under `target/rea/a4-motion-*` and `target/rea/a4-acceptance-*`. No commercial resource body or executable is
stored in the repository. Original analytical PNGs and prior native references
were not changed.
