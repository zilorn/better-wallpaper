# Feature API Contracts

Source of truth: `crates/better-wallpaper-daemon/src/server.rs`, configuration in
`crates/better-wallpaper-core/src/config.rs`, and consumers in `web/src/index.tsx`.
For Plasma-specific routes, read [Plasma API](../../better-wallpaper-plasma/references/api.md).

## Service and Discovery

The daemon binds to `127.0.0.1:43129`. `--no-ui` disables the management service.
The endpoint URL is written to `$XDG_RUNTIME_DIR/better-wallpaper/endpoint`;
without `XDG_RUNTIME_DIR`, the fallback is
`~/.better-wallpaper/better-wallpaper/endpoint`.
API version 1 and configuration version 2 are separate version numbers.
JSON errors have the form `{"error":"message"}`.

## Desktop Client and Tray

`crates/better-wallpaper-gtk` is the default native GTK 4.8+ client. It builds the
installed `better-wallpaper-desktop` command; run with
`cargo run -p better-wallpaper-gtk`. Both default and `--workspace` builds use GTK;
the old Tauri package and its GTK 3/WebKitGTK dependencies have been removed.
Packaging and the tray use the existing desktop binary name and menu entry.

The GTK app provides searchable library thumbnails loaded as you scroll, video
preview, scene reports/properties, wallpaper/playback controls, named outputs,
folder pickers, settings, diagnostics and logs. It uses API v1 directly without loading HTML.
HTTP work and service startup run on GIO workers; only GTK's main thread updates
widgets. Status is polled every two seconds without overlapping requests, and
logs refresh only while visible. The Wallpapers page initially creates twelve
cards and appends twelve more near the bottom. Thumbnails load only near the
viewport, with at most four requests in flight across searches and refreshes;
loaded cards/textures are reused while scrolling. Leaving the page pauses new
thumbnail requests. Search, type filters and refresh reset the scroll position
and discard stale thumbnail results. Library metadata still comes from the
complete bounded library response; this is client-side lazy loading. Preview
uses the authorized media endpoint and requires a GTK media backend/codecs;
errors appear in the preview window.

The client discovers the endpoint from the runtime file, falling back to
`http://127.0.0.1:43129`, and accepts only HTTP origins on `127.0.0.1`. Without
`--url`, an unavailable daemon triggers a nonblocking systemd user-service start
and up to ten seconds of readiness polling; failure shows a retry screen.
`--url <local-origin>` overrides discovery and disables service startup.
GTK disables HTTP redirects and proxies and limits response sizes.
GTK Application provides session-bus single-instance activation. Closing the client releases its controller while
daemon playback/tray continue. A second launch focuses the first endpoint.

The GTK interface supports English and Simplified Chinese, automatically selected
from `LC_ALL`, `LC_MESSAGES`, then `LANG` (first nonempty value). `--language en`
or `--language zh-CN` overrides detection. The first window retains its language
on repeated activation. Labels, option display text and client notices/errors are
localized without changing protocol values, paths, logs or author-provided text.
The display regression can run with `BETTER_WALLPAPER_GTK_TEST_LANGUAGE=zh-CN`.

GTK edits retain the full configuration JSON. Save fetches the latest document,
merges only edited fields, preserves unrelated concurrent changes and rejects
same-field conflicts. This is not atomic compare-and-swap: the API has no revision
token and a writer can still race between GET and PUT. Saving retains validation,
atomic persistence and reload semantics in the daemon. Connection loss retains
drafts and disables save/playback; reconnect to the same endpoint is automatic.
Reloading pending edits requires confirmation. Library selection is a draft until
Save changes; switching scenes clears property overrides. Backend changes report
restart-required without claiming a runtime switch.

See [GTK client](../../../../docs/gtk-desktop.md) for commands, display tests and
remaining UI limitations. Property fields currently use text entries with
metadata tooltips; conditional visibility and specialized editors remain pending.

The daemon owns the only tray, retaining pause/resume, persistent mute and
backend reload. Its launcher starts the sibling desktop binary with the current
endpoint, falling back to `xdg-open` if absent, in an independent transient user
service with graphics overrides removed. Packaging installs the GTK binary,
application-menu entry and icon.

## Configuration and Playback

| Method and route | Contract |
| --- | --- |
| `GET /api/v1/status` | Returns `api_version`, `desktop`, `evidence`, `candidates`, `backend`, `playback`, and `plasma_instances`. Playback contains `running`, `paused`, `cancelled`; instances contain `output`, `last_seen_ms`. |
| `GET /api/v1/config` | Returns normalized `AppConfig`. |
| `PUT /api/v1/config` | Validates and atomically saves an `AppConfig` JSON document, reloads shared state and log filtering, and requests playback rebuild. Success: `saved: true`, `restart_required: <boolean>`, `reload_requested: true`, normalized `config`. |
| `POST /api/v1/playback/pause` | No body required; returns playback state. `409` if cancelled or no playback task is running. |
| `POST /api/v1/playback/resume` | Same contract as pause, setting `paused` to false. |
| `GET /api/v1/logs` | Returns `{"lines":[...]}` from the in-memory log store (currently 2,000 lines). |
| `GET /api/v1/ws` | WebSocket upgrade; text JSON `{"type":"status","data":<status>}` on changes or every 15 seconds, polling every 250 ms. Invalid upgrade returns `400`. |

PUT replaces the config document; it is not a field patch. Start from GET and
preserve unrelated settings. Serde defaults apply to omitted fields. Invalid JSON
or validation/save failure returns `400`; bodies over 1 MiB return `413`.
Persistence/reload request success does not prove playback has restarted.
The backend is selected at daemon startup. `restart_required` is true when the
saved backend preference (with `auto` resolved using startup desktop detection)
differs from the running backend reported by status. Repeated saves retain this
flag until the preference matches the running backend or the service restarts.
An equivalent explicit/`auto` preference does not require a restart. CLI
`--backend` overrides must be removed or adjusted on restart for a conflicting
saved preference to take effect. Playback reloads still use the running backend.
The Web UI consumes this flag and prompts for a service restart; otherwise it
reports a reload request without claiming that playback has already restarted.

Config sections: `version`, `general`, `wallpaper`, optional `scene`, `library`,
`decode`, `outputs`. Key values and limits:

- Backends: `auto`, `niri`, `kde`, `headless`; wallpaper types: `video`, `web`, `scene`.
- Fill modes: `cover`, `contain`, `stretch`; `wallpaper.fps_limit`: 1–240.
- `general.log_level`: `info` or `debug`; scene quality: `low`, `medium`, `high`.
- `scene.particle_limit`: 0–100,000; `scene.properties`: at most 256 string entries,
  keys 1–128 bytes, values at most 4,096 bytes.
- `scene.mouse` and `scene.parallax` gate basic authored niri camera parallax.
  Both must be true, and the scene must contain enabled camera settings and
  per-layer depth. Pointer motion is available while focused on exposed wallpaper;
  moving over another window returns the scene toward center. The Web UI enables
  both controls and explains the niri-only input limitation. No arbitrary mouse
  scripts or Plasma scene consumer is implied.
- `scene.particle_limit` remains a reserved field: validation/persistence preserve
  it, but the runtime ignores it and the Web UI keeps it disabled.
- `scene.audio_processing` enables niri desktop spectrum capture for supported
  audio-response draws; capture failures fall back to rendering without spectrum
  data. `scene.script_enabled` does not execute scripts.
- Config paths expand `~` and `~/`; relative paths resolve against user home.
  Library paths are normalized, sorted, and deduplicated.
- Version 1 migrates to version 2; legacy `engine_mode` is cleared during migration.

## Library and Assets

| Method and route | Contract |
| --- | --- |
| `GET /api/v1/library` | Returns `entries`, configured `roots`, discovered `engine_roots`, `truncated`. Entries contain `name`, `path`, `engine_mode`, `wallpaper_type`, `preview_path`, `size_bytes`, `modified_unix_seconds`, optional `scene_compatibility` and nonempty `scene_properties`. |
| `GET /api/v1/library/media?path=<encoded-path>` | Requires a scanned library entry. Supports single byte ranges (`206`, `Content-Range`). Missing/invalid parameter: `400`; unavailable file: `404`; outside library: `403`; invalid range: `416`. |
| `GET /api/v1/library/thumbnail?path=<encoded-path>` | Requires a scanned entry; serves web preview images or cached FFmpeg JPEG thumbnails. Web entries without previews return `404`; FFmpeg generation failure can return `422`. |
| `GET /api/v1/wallpaper/media` | Serves the configured wallpaper path with range support; `404` when none is configured. |
| `GET /api/v1/wallpaper/web/<asset>` | Serves the current Wallpaper Engine web project's assets; empty suffix serves its entry file; explicit relative entry URLs preserve nested resource paths. Supports single byte ranges and rejects malformed asset encoding. Canonical files must stay inside the project root; inaccessible assets return `404`. |

Percent-encode paths and output names in query parameters. Library scanning uses
`library.paths` (default `~/Videos`) and discovered Steam Workshop roots for app
431960. Scanning is bounded by 1,000 entries and depth 4; check `truncated`.
Symlinks in configured library traversal are skipped. Scene compatibility contains
`level`, `level_name`, `supported_features`, `unsupported_features`, `warnings`.
The level is the highest supported feature tier identified in the scene (L0–L3
currently), with limitations listed separately even for higher tiers. Package
analysis includes referenced sprite and video textures, plus basic parallax with
the niri pointer-focus warning. It does not promise validated
assets, successful playback on the selected backend, or full engine parity.
Library cards show supported-feature counts/details as well as limits and warnings;
the optional UI field permits responses from older daemons without support lists.

Library requests, thumbnail authorization, and video/media authorization share a
daemon-lifetime in-memory index (`server/library_index.rs`). The first request
builds it; unchanged requests reuse an immutable snapshot without enumerating
directories, reading descriptors, or analysing scene packages. Linux inotify
events are drained on the next library/thumbnail/media request. Additions,
deletions, renames and writes invalidate only the affected directory listings
or Wallpaper Engine projects; unchanged projects retain their parsed metadata
and scene reports. Saving library paths adds/removes indexed roots while retaining
unaffected caches. Missing roots, new nested directories, Steam library-folder
changes and Workshop additions are watched through their existing ancestors.
When a watched directory moves, its old watch bindings and those of its
descendants are retired before scanning. Recreating the original pathname before
the next request therefore preserves change detection for both directory trees.
The UI's refresh button reads this same index; changes appear on the next request,
not through automatic page updates. Restarting the daemon rebuilds the index on
first use. Notification overflow/read failure rebuilds it; unavailable watches
fall back to scanning with a warning to avoid silently returning stale entries.
Cached media lookups use a path map and revalidate the requested canonical file;
thumbnail sources must still resolve to the indexed path before being opened.

Thumbnail cache: `$XDG_CACHE_HOME/better-wallpaper/thumbnails`, falling back to
`~/.cache/better-wallpaper/thumbnails`. Cache keys include source path, size, and
modification time.

Web project discovery accepts case-insensitive Wallpaper Engine type names.
`WebProject::load` supports a descriptor-backed entry or project directory and
standalone `.html` / `.htm` files; descriptor entries must match the configured
file and remain within their canonical project root. Asset URL paths preserve
literal `+`, percent-encode UTF-8 and keep nested entry directories. Standalone
HTML uses its containing directory as the asset boundary. niri uses an independent
random loopback port with no management routes, stopped on reload/teardown;
Plasma retains the management asset route. MIME types include fonts and WASM.
The wallpaper page offers explicit video/web selection and preserves type when
editing a path. Web selection does not imply full Wallpaper Engine parity;
default user properties are loaded, audio-spectrum/media callbacks remain stubs.
