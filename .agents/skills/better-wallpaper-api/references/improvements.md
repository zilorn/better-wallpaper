# Management API Improvements

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
