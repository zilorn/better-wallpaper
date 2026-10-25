# A5: bounded web wallpaper properties and desktop data

Date: 2026-10-25. Implementation commit is recorded in the adaptation plan.
This package implements and tests a documented protocol subset on both consumers.
It does not establish full Wallpaper Engine compatibility or full Plasma desktop
lifecycle acceptance; those boundaries remain explicit below.

## Protocol evidence and owned inputs

The primary public references, checked on the validation date, are Wallpaper
Engine's [user properties](https://docs.wallpaperengine.io/en/web/customization/properties.html),
[property listener](https://docs.wallpaperengine.io/en/web/api/propertylistener.html),
[audio visualization](https://docs.wallpaperengine.io/en/web/audio/visualizer.html),
[media integration](https://docs.wallpaperengine.io/en/web/audio/media.html), and the
[MPRIS Player interface](https://specifications.freedesktop.org/mpris/latest/Player_Interface.html).
No commercial project assets, native executable, or copied implementation is committed.
No original-program web capture was made; tests assert documented shapes and this
implementation's declared scheduling, not measured native audio-number parity.

The owned [project](../../tests/fixtures/web-protocol/project.json) has six editable
properties: checkbox, fractional slider, string combo, color, text input and file.
Its nested HTML entry registers listeners immediately, reports order and received
values, and loads an owned 64x64 SVG through the delivered file URL. The canvas
has a fixed 400x240 internal size. There is no random input or pointer dependency.
A separate regression checks numeric and boolean combo options. The companion
[manifest](a5-web-inputs.json) records fixture hashes and fixed protocol/audio inputs.

The [recorded results](a5-web-results.json) preserve the final environment and
per-consumer observations.

The [JavaScript contract check](../../native/web-wallpaper/tests/bridge.mjs) runs the
same injected bridge in isolated document contexts. It checks pre-load registration,
properties/general/pause/data order, typed booleans and fractional numbers, changed
property deltas, change-only media events, 128 audio values, playback constants,
pause/resume, listener replacement, null unregistration, exceptions, pagehide
cleanup and a fresh document. Registration stores one callback per category.
A later registration replays the most recent media event while active; it does
not manufacture an audio sample. Author callback exceptions do not stop delivery
to other categories. Reload destroys the old document and listener registry.

## Saved properties and file boundaries

Configuration remains version 2. `web` defaults are backward compatible with v1
migration and older v2 documents:

```toml
[web]
audio_processing = false
media_integration = false

[web.properties."/absolute/project/pages/index.html"]
enabled = "false"
speed = "0.25"
caption = "Saved caption"
image = "pages/probe.svg"
```

The configured absolute path is the project-map key. Directory and HTML aliases
are deliberately separate keys. Switching projects retains each map without
applying its values to another key. Missing overrides use authored defaults;
an empty string is an explicit value; clearing the selected map restores defaults.
Limits are 32 maps, 256 overrides total, 4096-byte absolute project keys,
1..128-byte property keys and 4096-byte values. Web descriptors are limited to
1 MiB and 256 valued properties. Unsupported metadata can remain in authored
defaults, but its presence does not authorize an editor or an asset import API.

The existing config PUT validates selected overrides before atomic persistence
and playback rebuild. It rejects unknown property keys, unsupported editor types,
invalid booleans, non-finite/out-of-range sliders, fractional values on integer
sliders, nonexistent combo options and invalid three-component 0..1 colors.
Persistent values stay strings; callback values retain booleans, numbers or the
selected option's authored scalar type. Combo `.text` uses the selected label.
GTK uses the existing typed editor schema, including conditions, and preserves
scene settings when editing/resetting web properties. English and Chinese display
regressions cover web edit, save, reload and reset.

File defaults and edits resolve to existing canonical files within the project
root. Relative paths and absolute paths within that root are accepted; escapes,
external files and symlink escapes are rejected. Pages receive percent-encoded
absolute HTTP asset URLs. No file:// permission is added. Projects that prepend
`file:///` themselves need adaptation; arbitrary external-file imports and
directory/random-directory/fetchall protocols remain unsupported. Descriptor and
asset paths retain the existing canonical project boundary and range handling.

Rust regressions save and reload ConfigStore from disk, check exact callback values,
reset and independent project keys, reject invalid config without changing the file,
verify library editor metadata, share Plasma sessions across outputs, and retire
an old private origin when saved settings request a rebuild.

## Audio, media and lifecycle

Both backends load pages from a private ephemeral loopback origin with GET-only
assets and read-only protocol data, independent of management. The host receives
a random 128-bit capability URL for data polling; it is not passed into page JS.
The unqualified protocol path returns 404. Management refuses requests carrying
browser Origin or Sec-Fetch-Mode headers, including simple cross-origin playback
POSTs and browser WebSocket access. Native GTK and QML remain consumers of the
management API. The former management asset route remains for native requests;
current browser pages use the private origin.

The shared WebSurface polls every 33 ms with at most one request outstanding.
Pause/visibility/unload abort outstanding requests and stop polling. A 500 ms
lease controls the data sources; multiple outputs share a session. Last-consumer
removal stops capture and releases the bus connection after expiry. Session drop
joins the provider and kills/reaps/joins the capture child and reader. An idle
niri helper intentionally remains available for output reconnection; it owns no
active capture once the last lease expires. All helper resources stop on stdin
EOF, with the daemon's existing bounded termination fallback.

Audio opt-in starts `parec` on the default output monitor, float32le stereo at
48 kHz. It never opens the microphone. Each channel has an independent 1024-sample
Hann window, 512-sample hop, 64 logarithmic bands from 50 Hz to 16 kHz and attack/
release smoothing inherited from the scene analyzer. Published finite values are
clamped to 0..1 and ordered left bins 0..63 then right bins 64..127. There is no
mono duplication. Polling yields roughly 30 callbacks/s when available, without
claiming a native FFT transfer function or exact timing. Unavailable/stale capture
is null and causes no fabricated audio callback. Failed capture retries after
five seconds. Pause/resume and view restoration restart capture without that
failure backoff. Wallpaper sound mute is independent of desktop-data opt-in.

Media opt-in uses zbus to read the session bus; no playerctl or player-control
method is used. Every 500 ms it sorts MPRIS names and queries at most the first
four, preferring playing over paused over stopped, with lexical tie breaking.
Each method has a 100 ms timeout and the connection queue is limited to eight.
Title, artist, album, album artist and genres map to the documented property
fields. Unsupported subtitle is empty and contentType is `music`; each text
field is truncated to 4096 Unicode scalars. Position/duration convert microseconds
to nonnegative seconds. Playback constants are internally consistent stopped=0,
playing=1, paused=2; native undocumented numeric equivalence is not asserted.

Media status reports the opt-in setting. Missing/disappearing bus/player sources
clear text, playback and timeline rather than retaining stale track data.
`betterWallpaperStatus` separately exposes audioAvailable/mediaAvailable, and
`betterWallpaperCapabilities` identifies this subset. Media events are change-only
with initial state delivery. Artwork extraction/colors are unsupported: thumbnail
listeners receive an empty thumbnail and fallback colors during media dispatch,
with mediaThumbnail capability false. Directory imports are also explicitly false.

A live removal test exposed a Qt empty-URL trap: an empty url can leave the old
document running, and an earlier pause screenshot callback can be invalidated by
the source transition. WebSurface now stops the old load and explicitly navigates
to about:blank when cleared, ignores stale blank-load completion, and resets its
request/document generation on navigation. It avoids creating an initial blank
navigation and defers/coalesces source changes; a late blank completion restarts
the selected source rather than canceling its first load. Re-adding an output uses fresh
registration and full property initialization. This fix is exercised through
actual Plasma main.qml, not inferred from source or lint.

## Reproduction and results

Run from the repository root. The live probe needs a niri session, PulseAudio
client tools against PipeWire/PulseAudio, dbus-python/PyGObject, and the Qt helper.
It creates an owned null sink, moves only its own capture stream to that monitor,
and plays an owned stereo tone into that sink. It never changes the default sink,
saved wallpaper, running user service, or installed system files.

```sh
cargo clippy
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
cargo test --workspace
node native/web-wallpaper/tests/bridge.mjs
cmake -S native/web-wallpaper -B target/web-wallpaper -DCMAKE_BUILD_TYPE=Release
cmake --build target/web-wallpaper --parallel 2
qmllint kde/org.better-wallpaper/contents/ui/main.qml \
  kde/org.better-wallpaper/contents/ui/WebSurface.qml \
  native/web-wallpaper/WebWallpaper.qml
cargo build -p better-wallpaper-daemon --example web_protocol_probe
python3 native/web-wallpaper/tests/smoke.py target/web-wallpaper/better-wallpaper-web
dbus-run-session -- /usr/bin/python3 native/web-wallpaper/tests/protocol.py
dbus-run-session -- xvfb-run -a env GDK_BACKEND=x11 GSK_RENDERER=cairo \
  cargo test -p better-wallpaper-gtk -- --ignored --test-threads=1
dbus-run-session -- xvfb-run -a env GDK_BACKEND=x11 GSK_RENDERER=cairo \
  BETTER_WALLPAPER_GTK_TEST_LANGUAGE=zh-CN \
  cargo test -p better-wallpaper-gtk -- --ignored --test-threads=1
./packaging/verify.sh
```

Rust invocations also used `--offline` where recorded by the command runner;
packaging runs its normal locked release build. Workspace: **302 passed, zero
failed, 16 default ignored**. Both GTK display runs separately passed their one
explicit test. Required Clippy, strict all-target Clippy and formatting passed.
The helper CMake build, QML lint, JavaScript contract check and staged release
installation/dependency/uninstallation validation passed. Python probe syntax was
also checked. No shared GPU-renderer behavior changed, so the pre-existing ignored
scene/GPU/niri tests were not separately rerun for A5.

The live harness separately ran the helper's native niri view and the **actual
Plasma main.qml** in a Qt layer window on niri DP-1 (1920x1080, scale 1). Both
received false/0.25/Saved caption overrides, the configured FPS=37, the owned file
asset, MPRIS title/artist, 7.25-second position and 123-second duration, and changed
track/playback data. Independent 440 Hz left and 2000 Hz right monitor samples
produced peak bins **24 and 40**, with representative peaks approximately 0.243
and 0.159. These values came through parec and the independent analyzers, not
an injected JS array. Pause froze page timers/callbacks; resume restored delivery.
Output disable/removal released capture, restore created fresh registration and
full properties, and process exit released capture and the private origin.
MPRIS disappearance was checked on the Plasma consumer and cleared availability
and title. Exact callback counts/timing vary; assertions test bounded transitions.

Initial validation found the expected old relative-URL assertion and strict-lint
issues; those fixtures/code were updated to the new absolute-private-origin
contract. The extended live test failed before the explicit document-unload fix
and passed after it. Local desktop audio/bus/Wayland checks require host access;
restricted access returns permission errors. Isolated session-bus tests emitted
portal/accessibility activation warnings without failing protocol assertions.

The Plasma run is an isolated consumer test, **not a plasmashell containment or
full Plasma session**. Actual Plasma activities, session recovery, multiple
physical outputs/hotplug, compositor restart and long-duration stability stay in
A9. The niri removal test changes the helper's selected-output list; it does not
physically unplug a monitor. Native web visual/audio numerical parity, artwork,
directory imports and broader project compatibility remain unverified or excluded.
