# Management API Improvements

## Incremental Library Index — Resolved

Previously `library_response`, `library_thumbnail_response` and `media_response`
each called the full scanner, including descriptor parsing and scene package
analysis. `server/library_index.rs` now shares cached directory listings, project
metadata and a path lookup map across these routes. inotify invalidates changed
directories/projects on the next request; unchanged requests reuse the snapshot.
Watch failure/overflow has a logged scanning/rebuild fallback. The cache lasts
until daemon exit; the UI refresh button requests an incremental update.

Regression tests verify snapshot reuse, directory read/project parse counts,
file writes/additions/deletions/moves, root additions/removals/recreation, newly
discovered Steam libraries, aliases and nested project dependencies, depth/entry
limits, fallback freshness, shared list/thumbnail/media requests and rejection of
cached paths replaced with symlinks outside the library.

## Backend Reload Reporting — Resolved

`server.rs::update_config` compares the saved backend preference, resolving
`auto` with startup desktop detection, against the running backend. It returns
`restart_required: true` while they differ, including on repeated saves. Status
continues to report the running backend. The Web UI displays a restart prompt
and explains that backend changes require a service restart.

Regression coverage exercises actual configuration PUT handling, persistence,
playback reload requests, repeated saves, reverting a pending switch, automatic
selection, and a running backend selected by a CLI override. CLI overrides must
be removed or adjusted on restart for a conflicting preference to take effect.
Atomic persistence and playback rebuild behavior remain unchanged.

## Web Project Discovery and Relative Assets — Resolved

The scanner now accepts `Web` as well as `web` (and mixed-case video/scene types).
`WebProject` validates descriptor entries and their canonical asset root, preserves
nested entry URLs and literal plus signs, and serves fonts/WASM/range media.
Regression coverage exercises uppercase Web discovery, UTF-8 nested assets,
properties, invalid ranges, traversal, symlinks and private-origin teardown.
The UI keeps web type while editing paths and exposes explicit type selection.
