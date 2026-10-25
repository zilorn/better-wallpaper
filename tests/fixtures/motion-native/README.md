# A4 owned motion and parallax probes

`python3 tests/fixtures/motion-native/generate.py` verifies 36 owned inputs;
`--write` regenerates inputs without changing captures. `manifest.json` pins all
inputs, 18 native screenshots and one native smoothing trace. There is no random
input, script execution, audio, bloom or camera shake. Canvas/client size is
128×128 except the explicitly authored 256×128 projection in `parallax-aspect`.
The plain source is a solid 16×16 RGBA marker on black.

Native screenshots come from Wallpaper Engine 2.8.42/build 23967692, with the
executable identity in the manifest. All 18 were independently recaptured with
identical RGB. Saved transform values and channels are deliberately different:

- `origin`: saved origin zero, channels (48,64)→(80,48), editor-relative false.
- `relative`: saved (64,64), channels (0,0)→(16,-16), editor-relative true.
- `scale`: saved (1,1), channels (1,1)→(2,0.5), producing final (3,1.5).
- `scale-relative`: saved (1,1), channels (0,0)→(1,-0.5), producing (2,0.5).
- `layer-alpha` and `zoom` disambiguate absolute scalar channels from additive
  transform channels; final alpha is 0.25 and zoom 0.5.
- Parallax probes isolate pointer center/right/up, negative/axis-zero depth,
  amount, influence, disabled amount, zoom, aspect, smoothing and omitted defaults.

Single animations use 30 fps, length 30 frames. Endpoint screenshots are taken
after completion; they do not establish exact synchronization with the native
scene clock. GPU tests separately compare fixed analytical position trajectories
at 0, 0.25, 0.5, 0.5, 0.75, 1, 0 seconds, plus reloads and context replacement.
Their gate is SSIM ≥0.995, RGBA MSE ≤1, max channel error ≤2.

Parallax pointer coordinates are recorded per project in the manifest, with
Y-down window pixels converted to Y-up normalized values for GLES. The smooth
probe centers the pointer, waits two seconds, steps X from 64 to 96 and reads the
marker center repeatedly for 1.5 seconds. `trajectory.json` brackets XGetImage
readbacks with monotonic timestamps; these are not native frame timestamps.
The window presents at approximately 15 Hz. Its four visible position updates
match the instruction-derived per-frame recurrence within 0.5 pixel. No precise
wall-clock parity across Wine and niri is claimed.

Provision copied `app/` and a dedicated `pfx/` as for the
[Tint capture protocol](../tint-native/README.md), then run:

```sh
python3 tests/fixtures/motion-native/capture.py --work /absolute/isolated-directory
```

`--only NAME` can be repeated for bounded investigations. Captures require a new
`recapture/` destination, a private Xvfb display and the hash-matched executable.
Pointer warps target only that virtual display. The user's real desktop pointer,
Wallpaper Engine installation and Better Wallpaper service/settings are unchanged.
The script joins Wine and Xvfb; no executable or commercial shader is committed.

The admitted transform subset is autonomous linear XY position/scale offsets
with empty/zero Z channels. Curves, wrap-loop interpolation, event scheduling,
scripts, angle/size/depth timelines and puppet/spectrum mesh motion retain
explicit limitations. The seven real local transform timelines in the survey
depend on these excluded script/Bezier/scheduling forms and remain unsupported.
