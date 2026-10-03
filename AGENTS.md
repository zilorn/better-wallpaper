# AGENTS.md

## Project Features and Technology Stack

Better Wallpaper is a Linux video wallpaper application for niri and KDE Plasma 6.
It provides playback controls, persistent configuration, a wallpaper library,
thumbnails, a system tray, and a web management UI. Wallpaper Engine web and scene
projects have partial, backend-specific support; consult the relevant project skill before
changing or describing these capabilities.

- Rust workspace: daemon, configuration, FFmpeg decoding/audio, scene parsing,
  GPU rendering, Wayland/niri integration, and KDE integration.
- Bun, TypeScript, SolidJS, and Vite: web management UI.
- CMake, C++, Qt 6/QML, and Qt Multimedia: Plasma wallpaper plugin.
- systemd user service: installed daemon lifecycle.

## Common Commands

Run commands from the repository root unless indicated otherwise.

| Task | Command |
| --- | --- |
| Required Rust validation | `cargo clippy` |
| Check Rust formatting | `cargo fmt --all -- --check` |
| Run Rust tests | `cargo test --workspace` |
| Build release binaries | `cargo build --locked --release` |
| Install web dependencies (in `web/`) | `bun install --frozen-lockfile` |
| Check web types (in `web/`) | `bun run check` |
| Build web UI (in `web/`) | `bun run build` |
| Run web development server (in `web/`) | `bun run dev` |
| Run daemon with management API | `cargo run -p better-wallpaper-daemon -- --backend headless` |
| Run bounded playback smoke check | `cargo run -p better-wallpaper-daemon -- --backend headless --no-ui --run-for-seconds 30` |
| Run niri playback | `cargo run --release -p better-wallpaper-daemon -- --backend niri --no-ui` |
| Install application | `./packaging/install.sh` (`./install.sh` delegates to it) |
| Verify staged packaging | `./packaging/verify.sh` |
| Read recent service logs | `journalctl --user -u better-wallpaper.service --since "10 minutes ago" -n 200 --no-pager` |

Build the web UI before using the management page. Set
`BETTER_WALLPAPER_WEB_ROOT="$PWD/web/dist"` when launching a development daemon
to serve the current build. Rust builds require FFmpeg, ALSA, and Wayland
development libraries; plugin builds additionally need CMake and Qt 6.6+.
See `packaging/README.md` for dependencies and installation options.

## Important Notes

- Never use subagents or delegate work to other agents.
- Write useful English logs for state changes, failures, and debugging. Routine
  web route access logs are not required.
- Read runtime logs with `journalctl --user -u better-wallpaper.service`; limit
  output by time, line count, or a relevant filter. Debug mode includes DEBUG and
  higher levels; normal user mode includes INFO and higher levels. Enable debug
  with `--log-level debug` or `general.log_level = "debug"`.
- Keep installation logic in `packaging/install.sh`.
- `cargo clippy` must pass. Run additional checks appropriate to the change;
  accurately report checks that could not run and their blockers.
- Preserve configuration validation, atomic persistence, safe playback reload,
  and filesystem boundaries for media and wallpaper assets.
- Read the relevant category skill before feature/API work:
  - Rendering and scene formats: `skills/better-wallpaper-rendering/SKILL.md`.
  - niri and Wayland: `skills/better-wallpaper-niri/SKILL.md`.
  - KDE Plasma: `skills/better-wallpaper-plasma/SKILL.md`.
  - Management API, configuration, and library: `skills/better-wallpaper-api/SKILL.md`.
- Load only relevant skills; cross-category changes require each affected skill.
  Update their references in the same change whenever interfaces, behavior,
  limitations, commands, or improvement status change. Add confirmed gaps with
  evidence and completion criteria; revise resolved gaps. Implementation takes
  precedence over older plans.

## Git Commits

Use English commit messages with one of these prefixes:
`feat: <message>`, `fix: <message>`, `docs: <message>`, or `refactor: <message>`.

Keep the subject concise and describe the resulting change. An optional short
English body may explain implementation or validation; avoid long narratives.
Example: `docs: document project APIs and maintenance rules`.

After each commit, report back to the user with a table covering the commit, the
affected files, and the validation results.
