# A6 bounded 2D particle subset

Design and evidence, 2026-10-25. This specification precedes implementation.
The acceptance results will be recorded separately. Plasma still has no scene
consumer. This is a limited native-format adapter, not general particle parity.

## Admission and simulation

Admit one `boxrandom` emitter, four explicit initializers (`lifetimerandom`,
`sizerandom`, `velocityrandom`, `colorrandom`), one force-free `movement`
operator and one default `sprite` renderer. Lifetime must be constant; position
and velocity are XY only. Support either a continuous rate or one initial burst,
with nonnegative start time and finite optional emission duration. Reject unknown
fields, operators, nonzero flags, control-point offsets, instance overrides,
scripts, parented/animated particle transforms, effects and nonplanar transforms.
No gravity, drag, trails, child systems, events, turbulence or collisions.

Use one package-owned static 2D texture, one `genericparticle` pass and explicit
translucent/additive blending. Reject video, volumes, animation/frame tables,
extra textures/passes, shader overrides and nondefault shader combinations.
Frame selection is the complete static image (frame zero); animated and random
atlas frame selection remain unsupported. Native sprite width is the size;
height is width times texture height/width. Color modulates sampled RGBA.

Continuous births have monotonically numbered identities. Evaluate their age
and constant-velocity position directly from the shared f64 scene clock; do
not integrate wall time or replay missed frames. Births begin after one rate
interval; burst births occur at start time. Death excludes age >= lifetime.
Use a fixed, documented counter hash of node identity, birth identity and random
channel. It is an application policy, not an emulation of the native RNG stream.
Equal scene times reproduce particles; rewind/reload needs no historical state.

The configured scene limit (0–100,000, default 10,000) is shared by all systems
in authored order. Also honor native `maxcount`. Reserve a fixed capacity from
each system's maximum concurrent births; zero disables particles. When reduced,
retain a fixed prefix of birth residues in the natural occupancy cycle. A birth
that was dropped never appears later in its lifetime. This stable thinning is
an explicit degradation policy, not native maxcount scheduling. CPU sampling and
GPU vertex buffers must remain bounded independently of scene uptime.

## Evidence boundary

Reference: installed Wallpaper Engine 2.8.42/build 23967692, `wallpaper64.exe`
SHA-256 `40e2ce021e9352324fadb3b8f72b8ba2a7ee95b71cc571d5b9f84be75cd993b0`.
REA Hopper profile `32c01d9334c1fa8dee7c09790f43820490bcf0022af7efe157da365c2ab35983`.

| Evidence | Observation |
| --- | --- |
| `ev_0841629f09fb3cee63849e88dfc8930cc0038b74185abc190b09343c4a9d2e06` | Native initializer/emitter names; string inventory alone does not prove their operation. |
| `ev_3b0a308cd9e931828278fd54dc852e215a64553e2b2af8c261aa0e9cbd3e5a7c` | Loader containing `0x1401c6390`, `0x1401c7149`, `0x1401c7320`, `0x1401c74cf`, `0x1401c803f`: box ranges and scalar/vector min/max/exponent records; material, starttime, maxcount and randomframe mode. Large decompilation is static evidence, not recovered source. |
| `ev_74c9015427e75ba57c6de17d1afb5799358aa38c5df1b0979ce09265c7c0caec` | Velocity defaults/exponent registration at `0x1401bac50`. Explicit ranges avoid dependence on defaults. |
| `ev_379791ccadeba77f7210b8b3110dae3bcbcc85a9b49476918ef886ab6c14375a` | Movement registration at `0x1401bc9a0`: zero gravity, zero drag and flags. |

Installed `assets/scenes/particleelementpreviews/*` provide schema examples;
`assets/shaders/common_particles.h::ComputeParticlePosition` establishes width,
texture aspect and UV orientation. `genericparticle.frag` multiplies sampled
RGBA by particle color. Only independently captured native zero-range probes
can establish visual agreement. Random sequence identity and precise native
emission clock boundaries are not established by these observations.
Complete local REA evidence is exported to `/tmp/a6-particles-evidence.json`;
commercial assets and full decompilation are excluded from the repository.

## Fixed sample and acceptance matrix

Owned fixture generation must fix viewport 128×128, node IDs, random seed policy,
time and all ranges. The matrix covers:

| Case | Required checks |
| --- | --- |
| Burst/static, non-square texture | Width/aspect, UV orientation, static frame zero, color modulation |
| Additive/translucent overlap | Analytical pixels and independent native stable captures |
| Continuous, finite duration, delayed start | Birth/death boundaries and fixed time snapshots |
| Random box/velocity/size/color | Repeatability, bounds, seed/node separation and cadence independence |
| Zero/tiny/full budgets and multiple systems | Global cap, stable thinning, no late resurrection and bounded work |
| Pause/resume, rewind, long uptime | Frozen bytes, no wall-time catch-up, constant bounded work |
| Unknown operators/fields, atlas/video/material variants | Shared admission rejection with specific compatibility/log reasons |
| Reload, failed upload and context replacement | CPU/GPU ownership released; fixed snapshots reproduce |

Run required Clippy, formatting, relevant admission/simulation tests, full workspace
tests and explicit offscreen GPU checks. Record real desktop results separately;
offscreen rendering does not prove compositor presentation or Plasma support.
