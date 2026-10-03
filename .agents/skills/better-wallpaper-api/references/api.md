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

## Configuration and Playback

| Method and route | Contract |
| --- | --- |
| `GET /api/v1/status` | Returns `api_version`, `desktop`, `evidence`, `candidates`, `backend`, `playback`, and `plasma_instances`. Playback contains `running`, `paused`, `cancelled`; instances contain `output`, `last_seen_ms`. |
| `GET /api/v1/config` | Returns normalized `AppConfig`. |
| `PUT /api/v1/config` | Validates and atomically saves an `AppConfig` JSON document, reloads shared state and log filtering, and requests playback rebuild. Success: `saved: true`, `restart_required: false`, `reload_requested: true`, normalized `config`. |
| `POST /api/v1/playback/pause` | No body required; returns playback state. `409` if cancelled or no playback task is running. |
| `POST /api/v1/playback/resume` | Same contract as pause, setting `paused` to false. |
| `GET /api/v1/logs` | Returns `{"lines":[...]}` from the in-memory log store (currently 2,000 lines). |
| `GET /api/v1/ws` | WebSocket upgrade; text JSON `{"type":"status","data":<status>}` on changes or every 15 seconds, polling every 250 ms. Invalid upgrade returns `400`. |

PUT replaces the config document; it is not a field patch. Start from GET and
preserve unrelated settings. Serde defaults apply to omitted fields. Invalid JSON
or validation/save failure returns `400`; bodies over 1 MiB return `413`.
Persistence/reload request success does not prove playback has restarted.
The backend is selected at daemon startup; the response's `restart_required`
field currently does not reliably describe live backend changes.

Config sections: `version`, `general`, `wallpaper`, optional `scene`, `library`,
`decode`, `outputs`. Key values and limits:

- Backends: `auto`, `niri`, `kde`, `headless`; wallpaper types: `video`, `web`, `scene`.
- Fill modes: `cover`, `contain`, `stretch`; `wallpaper.fps_limit`: 1–240.
- `general.log_level`: `info` or `debug`; scene quality: `low`, `medium`, `high`.
- `scene.particle_limit`: 0–100,000; `scene.properties`: at most 256 string entries,
  keys 1–128 bytes, values at most 4,096 bytes.
- `scene.mouse`, `scene.parallax`, and `scene.particle_limit` are reserved fields:
  validation and persistence preserve their values, but no runtime behavior uses
  them. The Web UI shows disabled controls marked unimplemented and preserves
  stored values on save. Scene playback logs explicitly report these values as
  ignored, separate from applied runtime settings.
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
| `GET /api/v1/wallpaper/web/<asset>` | Serves the current Wallpaper Engine web project's assets; empty suffix serves its entry file. Canonical files must stay inside the project root; inaccessible assets return `404`. |

Percent-encode paths and output names in query parameters. Library scanning uses
`library.paths` (default `~/Videos`) and discovered Steam Workshop roots for app
431960. Scanning is bounded by 1,000 entries and depth 4; check `truncated`.
Symlinks in configured library traversal are skipped. Scene compatibility contains
`level`, `level_name`, `supported_features`, `unsupported_features`, `warnings`.
The level is the highest supported feature tier identified in the scene (L0–L3
currently), with limitations listed separately even for higher tiers. Package
analysis includes referenced sprite textures. It does not promise validated
assets, successful playback on the selected backend, or full engine parity.
Library cards show supported-feature counts/details as well as limits and warnings;
the optional UI field permits responses from older daemons without support lists.

Thumbnail cache: `$XDG_CACHE_HOME/better-wallpaper/thumbnails`, falling back to
`~/.cache/better-wallpaper/thumbnails`. Cache keys include source path, size, and
modification time.
