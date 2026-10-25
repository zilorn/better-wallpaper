# A7 SceneScript feasibility and bounded scope

Date: 2026-10-25. This is the scope/feasibility deliverable preceding the A7
implementation. The [survey](a7-script-survey.json) preserves scene/member/script
identities without retaining commercial script bodies. Reproduce it with:

```sh
python3 tests/fixtures/scenescript-native/survey.py --root /absolute/workshop/content/431960
```

The selected 30 local packages contain 900 script wrappers. Candidate member
occurrences include `engine.registerAudioBuffers` (203), `engine.frametime`
(219), `engine.userProperties` (37), `engine.runtime` (25), and
`thisScene.getLayer` (13). These are per-script regex candidates, including dead
code/comments; they are neither executed-call counts nor complete semantic
dependencies. The survey inspects only bounded scene.json members, not imported
JS modules, and cannot establish project compatibility or playback success.

The language includes exported lifecycle functions, mutable state, vectors,
script-property builders, audio, media, cursor interactions, shared schedulers
and timers. Implementing all of this requires a substantially broader host API.
The selected first runtime instead admits deterministic straight-line numeric
`update(value)` code on alpha and ordinary XY origin/scale transforms. Explicit
semicolons, local scalar let/const declarations, assignments, arithmetic,
selected Math functions and Vec3 results form the grammar. The compiler must
consume the complete source; recognizing a substring never authorizes execution.

Numeric slider defaults/overrides, scene runtime seconds, own static numeric
layer members and literal-name lookups of unique static layers are the initial
host subset. Dynamic object reads, foreign writes, text/effect/particle scripts,
imports, input, dates, audio, shared state, callbacks, other exports, timers and
arbitrary JavaScript remain excluded. Project properties change through the
existing saved-config/reload path; there is no independent script-owned setting.

The reference documentation describes update events and host interfaces:
[update](https://docs.wallpaperengine.io/en/scene/scenescript/reference/event/update.html),
[IEngine](https://docs.wallpaperengine.io/en/scene/scenescript/reference/class/IEngine.html),
[ILayer](https://docs.wallpaperengine.io/en/scene/scenescript/reference/class/ILayer.html),
[IScene](https://docs.wallpaperengine.io/en/scene/scenescript/reference/class/IScene.html).
An owned native probe establishes that reading/modifying the previous bound
value accumulates across frames. That stateful form is excluded from this
first runtime: absolute assignments must precede reads of changed components.
Unassigned components may pass through with `return value`. Captures and the
implementation's acceptance record must preserve this distinction.

## Runtime decision and budgets

A full JavaScript engine is a candidate for a later stateful extension. QuickJS
offers [memory and interrupt limits](https://docs.rs/rquickjs/latest/rquickjs/struct.Runtime.html),
but an engine alone does not reproduce Wallpaper Engine's scheduler/object API.
A helper process adds IPC, failure handling and cross-output synchronization;
an embedded engine adds allocator, stack, interrupt and teardown obligations.
For this explicitly finite subset, a Rust numeric instruction evaluator is
feasible without an additional runtime/dependency or native host bindings.
This is a restricted SceneScript subset, never general JavaScript compatibility.

Planned hard admission/execution limits:

| Resource | Bound |
| --- | --- |
| Source per binding / per scene | 8 KiB / 64 KiB |
| Script wrappers per scene | 64; crossing a scene cap disables all bindings |
| Tokens / expression depth | 512 / 32 |
| Statements / scalar locals | 32 / 16 |
| Numeric instructions | 256 per binding evaluation, no backward jumps/calls |
| Per-evaluation storage | Fixed 256-number stack plus 19-number registers |
| Results | Finite, magnitude at most 1e6; transform Z must stay unchanged |

The operation bound replaces an uninterruptible wall-time timeout. Loops,
recursion, heap allocation, dynamic code, async jobs and external I/O have no
instruction or host entry point. Bounded source compilation occurs at scene
load. At render time, shared per-clock caches avoid re-executing parent scripts
for every descendant/output. Cache contention must never wait for script work.
Arithmetic/type/capability faults retain the authored property, disable only the
affected binding, and log once until reload. No script-owned threads, handles,
callbacks or GPU objects exist. A fresh plan resets faults; pause freezes the
scene clock; rewind deterministically recomputes absolute expressions.

## Implementation acceptance

The next commit must connect the existing opt-in `scene.script_enabled` setting
to the actual scene consumer, preserve false as default, and keep L4 disabled.
Library reports must distinguish admitted bindings from concrete exclusions and
make the opt-in/backend boundary visible. GPU/native probes must cover absolute
transforms, alpha, numeric properties and static object access; negative probes
must cover previous-value state, non-finite math, unavailable APIs and budgets.
Pause, rewind, reload, multiple cloned consumers and context replacement need
regressions. Native time-invariant captures cannot establish exact native clock
equivalence; time-dependent paths need separately labelled analytical checks.

Plasma has no scene consumer. Multi-output desktop hotplug/restart and A9 soak
gates remain outside A7. Completing this scope does not promise that any entire
surveyed real project becomes fully supported.
