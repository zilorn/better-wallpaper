# GTK desktop client

`crates/better-wallpaper-gtk` provides the default `better-wallpaper-desktop`
command using Rust and GTK 4.8+. It calls the existing daemon API directly and
does not embed a browser or require built web assets. The daemon retains playback,
configuration persistence, the library index, media authorization and the sole
system tray. No rendering or backend behavior changes are required.

GTK is the desktop client for both default and `--workspace` builds.
The browser management UI and its Bun/TypeScript build have been removed.
The daemon retains HTTP configuration/media APIs for GTK and Plasma, including
web wallpaper asset routes. Its root and unknown routes return JSON `404` errors.
The tray launches the sibling GTK binary; if absent, it logs an installation
error. Packaging cleans old management HTML assets during upgrades.

## Run

```sh
cargo run -p better-wallpaper-daemon -- --backend headless
cargo run -p better-wallpaper-gtk
# Use a particular daemon without starting the installed systemd service:
cargo run -p better-wallpaper-gtk -- --url http://127.0.0.1:43129
# Simplified Chinese, regardless of the desktop locale:
cargo run -p better-wallpaper-gtk -- --language zh-CN
```

Endpoint discovery uses the daemon's runtime file and falls back to
`http://127.0.0.1:43129`. Only HTTP origins on `127.0.0.1` are accepted; credentials,
paths, queries, fragments, redirects and proxy forwarding are rejected. Without
`--url`, a failed connection requests a nonblocking systemd user-service start
and polls readiness for ten seconds. Failures are visible and retryable.

GTK Application uses the desktop session bus to focus the existing window on
repeat launches. A second launch keeps the first window's endpoint and language. Closing the
client leaves daemon playback and the tray running. No new tray is created.

## Language support / 语言支持

The native interface supports English and Simplified Chinese. It reads the first
nonempty `LC_ALL`, `LC_MESSAGES`, then `LANG`; Chinese (`zh`) locales select
Simplified Chinese, and other locales fall back to English. `--language en` or
`--language zh-CN` overrides detection for that process. Close the existing window
before launching with a different override. Built-in labels, dropdown display
values, notices, dialogs and error summaries are translated; protocol/configuration
values stay unchanged. Service logs, daemon-provided details, file paths and
wallpaper-authored names/properties retain their original text. System file-picker
chrome follows the desktop's own GTK/portal locale. Chinese glyphs require an
installed CJK font, such as Noto Sans CJK.

原生界面支持英文和简体中文，默认跟随系统语言。可使用 `--language zh-CN` 强制中文，
或使用 `--language en` 强制英文。更改启动语言前请先关闭已有窗口。日志、文件路径和
壁纸作者提供的名称、属性说明保持原文；语言切换不会修改壁纸配置。

## Native views

- **Wallpapers:** search by name/path, filter by type, load cards in batches of
  twelve as you scroll, inspect scene compatibility details, and preview
  videos in a muted GTK media window. Choosing an entry prepares a draft;
  **Save changes** applies it. Scene changes reset property overrides.
- **Playback:** file picker, wallpaper type, fill mode, frame limit, mute/loop,
  scene quality, mouse/parallax/audio controls, and editable project properties.
  Scene reports retain supported features, unsupported features and warnings.
  Reserved particle and script execution controls are not offered.
- **Displays:** add named outputs, enable/disable them, or remove overrides.
- **Library folders:** add folders with a native picker or edit one path per
  line; saving updates the daemon index and refreshes the library.
- **Settings:** backend preference, decoding and height cap, log level, startup
  restoration and live connection diagnostics. Backend saves explicitly report
  that a service restart is needed; reload requests do not claim playback success.
- **Logs:** automatic refresh while visible, freeze refresh, and text filtering.

Network requests and service startup run outside GTK's main thread. Status polls
every two seconds with no overlapping status requests. A failed status disables
saving/playback controls while keeping drafts; recovery at the same endpoint is
automatic. Reloading settings while edits are pending asks before discarding them.
Library requests are bounded to one scan at a time. The complete library metadata
is fetched once per refresh; cards are created twelve at a time near the bottom
of the list. Thumbnails load only within 300 pixels of the visible area, with at
most four requests in flight, including requests from a previous search/refresh.
Scrolling back reuses loaded cards and textures. Leaving Wallpapers pauses new
thumbnail requests. Search, type filters and refresh reset the list to the top;
generation checks discard stale thumbnail results.

## Configuration writes

The client retains the complete JSON document, including unknown fields. A save
fetches a fresh document and merges only user-edited fields. Unrelated changes
from the tray or other API clients survive. Same-field conflicts stop the save with an error;
the user can keep editing or reload. Output arrays and library path arrays are
treated as single fields. This is best-effort conflict detection: the existing
API does not offer a revision token or atomic compare-and-swap, so another writer
can still race between GET and PUT. Validation, atomic disk writes and playback
reload remain the daemon's responsibility.

## Validation

```sh
cargo clippy
cargo fmt --all -- --check
cargo test -p better-wallpaper-gtk
dbus-run-session -- xvfb-run -a env GDK_BACKEND=x11 GSK_RENDERER=cairo \
  cargo test -p better-wallpaper-gtk -- --ignored --test-threads=1
# Repeat the same native workflows with Chinese labels and notices:
dbus-run-session -- xvfb-run -a env GDK_BACKEND=x11 GSK_RENDERER=cairo \
  BETTER_WALLPAPER_GTK_TEST_LANGUAGE=zh-CN \
  cargo test -p better-wallpaper-gtk -- --ignored --test-threads=1
./packaging/verify.sh
```

The display test uses an isolated HTTP fixture, drives native controls, and
checks connection, pause/resume, scroll loading, deferred thumbnails, filtering,
search beyond loaded cards, hidden-page refresh, thumbnail concurrency and stale
responses across filters, scene selection, property
reset, typed properties, conditional visibility, hidden drafts, configuration
preservation, conflicts, backend restart notices, logs,
offline recovery and controller cleanup. Set `BETTER_WALLPAPER_GTK_SCREENSHOTS`
to an output directory to capture its rendered windows. The ordinary unit tests
do not need a display. These checks do not prove actual niri/Plasma playback.

Video preview requires a GTK media backend (normally GStreamer) and suitable
codecs. Missing support is reported inside the preview window. Scene properties
use native toggles, decimal numeric controls, dropdowns, color choosers, file
pickers and text entries. Simple `key.value ==/!= literal` conditions update
visibility immediately; hiding a field retains its draft and saved overrides.
All property values remain strings. Unknown types and invalid typed values stay
editable as text, and unknown combo values remain selectable. Reset clears
overrides without writing defaults until edited. Existing backend limits still
apply; compatibility tiers are metadata analysis, not playback guarantees.

GTK APIs follow the [gtk-rs main-loop guidance](https://gtk-rs.org/gtk4-rs/stable/latest/book/main_event_loop.html):
blocking I/O runs on GIO workers and only the main thread updates widgets.
