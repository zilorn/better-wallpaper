# Management API Improvements

## Restricted SceneScript Configuration — Connected

`scene.script_enabled` now gates a finite arithmetic runtime on niri, defaulting
false. Existing config persistence/reload updates declared numeric slider values
and rebuilds programs, with bounded canonical descriptor loading and regressions
for defaults, overrides, disabled execution, text/numeric type separation,
oversized descriptors and escaping symlinks. Unsupported APIs remain explicit;
there is no new route, script-owned persistence, GTK toggle, L4 or Plasma scene
consumer. See [A7 acceptance](../../../../docs/validation/a7-scenescript.md).

## Configuration Write Ordering — Resolved

`update_config` used to save and reload the file before acquiring the shared
configuration write lock. Parallel PUTs and tray writes could interleave those
steps, leaving disk and runtime settings different. A poisoned lock also allowed
a PUT to change the file before returning `500`. The API now holds the same lock
as the tray across save, reload, shared-state/log-filter updates and the playback
reload request. Regressions reproduce both the concurrent mismatch and the
failed-request file mutation. This orders daemon writes; client GET/PUT conflict
protection still requires the revision mechanism described below.

## Invalid Media Range Responses — Resolved

`media_response` previously treated a rejected Range header like an absent header
and sent the entire file with `200`. Both library and current-wallpaper media now
return the documented `416` with the file length in `Content-Range`. Regression
coverage checks full responses, bounded/open-ended/clamped ranges, malformed and
multiple ranges, unsatisfiable ranges, and empty files.

## Relative Configuration File Saves — Resolved

`ConfigStore::save` previously treated the empty parent of a bare filename as a
directory path. Saving `--config config.toml` could replace the file and then fail
when opening the empty path for directory synchronization. It now uses `.` for
that parent, preserving atomic replacement and synchronization. A regression
test saves and reloads a bare relative filename twice without changing the
process working directory.

## Native GTK Desktop and Property Editors — Implemented

`better-wallpaper-gtk` provides the installed desktop command with native widgets;
the old Tauri client and browser management UI have been removed. The GTK client preserves full configuration documents, merges
edits against a fresh GET and detects same-field
conflicts, with regression coverage for unrelated concurrent changes. The API
still has no revision/conditional-write mechanism; completion of atomic conflict
protection requires a daemon-supported revision checked by PUT plus concurrent
writer tests. A client-only GET/PUT merge cannot close that race.

`properties.rs` supplies native bool/slider/combo/color/file/text editors while
preserving string-valued overrides. Conditional visibility updates immediately
and never drops hidden drafts. Unknown types/invalid values retain text editing;
unknown combo values remain selectable. Display regression fixtures verify typed
edits, fractional values, conditional hiding, refresh, save/reset, preservation of
unusual overrides and controller cleanup. Existing backend limitations remain.

## Browser Management UI Removal — Resolved

The GTK client now covers the former browser property editors. `web/`, the
Bun/TypeScript build, static HTML serving, web-root discovery and browser tray
fallback have been removed. Configuration, media, WebSocket and Plasma API
contracts remain. HTTP regression coverage verifies the API and web wallpaper
assets alongside `404` responses for former UI paths. Packaging no longer needs
management HTML; staging verification seeds stale assets and checks their removal
on upgrade, while retaining the independent Qt web wallpaper renderer.

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

A follow-up regression reproduces moving a watched directory and recreating its
old pathname before the next request: inotify continued following the moved inode,
so the replacement's later changes could be missed. The monitor now retires old
bindings for the entire moved subtree before rebinding paths. Tests cover both
local videos and Workshop projects, including nested additions/deletions and
updates to the moved tree, while checking unaffected caches remain reusable.

## Backend Reload Reporting — Resolved

`server.rs::update_config` compares the saved backend preference, resolving
`auto` with startup desktop detection, against the running backend. It returns
`restart_required: true` while they differ, including on repeated saves. Status
continues to report the running backend. The GTK UI displays a restart prompt
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
