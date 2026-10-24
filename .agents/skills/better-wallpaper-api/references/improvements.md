# Management API Improvements

## Native GTK Desktop and Property Editors — Implemented

`better-wallpaper-gtk` provides the installed desktop command with native widgets;
the old Tauri client has been removed, while the web UI remains available for
browser access. The GTK client preserves full configuration documents, merges
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
