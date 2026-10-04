# niri web wallpaper helper

Qt Quick / Qt WebEngine render the page; LayerShellQt creates background surfaces.
Build requires CMake, Qt 6.6+ Quick/WebEngineQuick and LayerShellQt for Qt 6.

From the repository root:

```sh
cmake -S native/web-wallpaper -B target/web-wallpaper -DCMAKE_BUILD_TYPE=Release
cmake --build target/web-wallpaper --parallel 2
qmllint native/web-wallpaper/WebWallpaper.qml
```

The daemon starts the helper with an owned stdin pipe. The first newline-delimited
JSON message contains `url` (private loopback origin), `properties`, `outputs`,
`muted`, `paused` and optional advisory `fps`. Subsequent messages contain `paused`.
EOF exits normally; the daemon waits and kills a stuck helper on teardown.
No script can invoke native configuration or file APIs. Chromium sandboxing is
left enabled. Packaging builds, atomically installs and removes this executable.

Run this explicit smoke check in a real niri session with an exposed background:

```sh
python3 native/web-wallpaper/tests/smoke.py
```

It briefly creates a synthetic wallpaper without modifying saved configuration.
It verifies HTTP loading, WebGL creation, animation timers, property defaults,
compatibility registration, freeze/resume and stdin-close cleanup. It does not
verify audio playback, pixel fidelity, multiple outputs or compositor recovery.
The UI retains a screenshot while hiding/freezing the web item on pause; the
screenshot result must stay alive until resume. Qt lifecycle and user-script
APIs: [page lifecycle](https://doc.qt.io/qt-6/qtwebengine-features.html#page-lifecycle-api),
[user script collections](https://doc.qt.io/qt-6/qml-qtwebengine-webenginescriptcollection.html).
