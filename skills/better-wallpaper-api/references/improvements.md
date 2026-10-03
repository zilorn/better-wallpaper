# Management API Improvements

## Backend Reload Reporting — Proposal

Evidence: `main.rs` selects the backend at startup and captures it in the playback
supervisor; `server.rs::update_config` always returns `restart_required: false`.
A successful PUT does not establish that a new backend took effect.

Completion: accurately report when a daemon restart is needed, or implement safe
backend switching. Verify configuration, status, and UI agreement after changing
backends. Preserve atomic persistence and cleanup during playback rebuild.
