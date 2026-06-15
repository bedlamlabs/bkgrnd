# Weekend Pickup: Visualizer Mode and WOPR Migration

## projectM visualizer direction

Goal: keep bkgrnd's discrete menubar UI as the primary control surface, then add an optional visualizer mode that can be invoked on demand.

The visualizer should not be implemented inside a macOS WidgetKit widget. WidgetKit is good for a compact now-playing/status surface and for launching actions, but it is not the right place for a continuously running OpenGL/Metal visualizer.

Recommended product shape:

- Menubar app remains the default UI.
- Optional WidgetKit widget shows current track/artwork/status and a visualizer launch control.
- Visualizer opens as a real app-owned window.
- Visualizer window supports:
  - borderless/movable mini-player style mode
  - promotion to fullscreen
  - close/hide without stopping audio
  - preset cycling controls later

## projectM fit

projectM is a strong fit for this feature. Its core library, `libprojectM`, is a C++ MilkDrop-compatible visualizer engine. It analyzes PCM audio with FFT/beat detection and renders presets through OpenGL. The projectM docs/repo describe rendering to a dedicated OpenGL context or texture.

Useful sources:

- https://projectm-visualizer.org/
- https://github.com/projectM-visualizer/projectm

Licensing note: `libprojectM` is LGPL-2.1. If bkgrnd ships it, prefer dynamic linking and keep the integration boundary clean so users can replace the library as required by the license.

## Main technical dependency

The hard part is not opening a window. The hard part is feeding projectM PCM samples.

Current desktop playback path:

- `yt-dlp` resolves a direct YouTube media URL.
- bkgrnd passes that URL to bundled `mpv`.
- `mpv` plays with `--no-video`.

That means bkgrnd does not currently own decoded PCM audio frames. projectM needs those frames for responsive beat-synced visuals.

## Recommended Swift-era architecture

When the app moves to Swift:

- Use a Swift/AppKit or SwiftUI menubar shell.
- Keep the menubar UI as the always-available control surface.
- Move playback into an app-owned playback layer where possible.
- Expose PCM samples from the playback layer to the visualizer engine.
- Wrap `libprojectM` behind a small Swift-friendly boundary.
- Render projectM in an app-owned visualizer window, initially via OpenGL if that is the shortest path.
- Keep WidgetKit as a launcher/status/control companion only.

Potential implementation options:

- Best long-term: Swift playback layer owns decoding/playback and feeds PCM directly to `libprojectM`.
- Faster experiment: launch or embed a projectM SDL/frontend prototype and let it capture audio externally.
- Hybrid: keep `mpv` for playback initially, but investigate whether an audio filter/hook/IPC path can expose PCM. Treat this as exploratory; do not assume it will be clean.

## Suggested first spike

1. Build a tiny Swift/AppKit visualizer window prototype.
2. Link or shell out to a projectM frontend just to validate rendering, presets, fullscreen, and performance.
3. Separately spike PCM access from the future Swift playback path.
4. Only after both spikes work, wire the menubar and optional WidgetKit launch action.

## WOPR migration note

Current live WOPR deployment from `HANDOVER.md`:

- URL: `https://bkgrnd.bedl.am/`
- SSH alias: `wopr`
- Deployed directory: `/Users/dev/bkgrnd-wopr`
- Binary: `/Users/dev/bkgrnd-wopr/bkgrnd_server`
- Web assets: `/Users/dev/bkgrnd-wopr/web/`
- Data: `/Users/dev/bkgrnd-wopr/data/`

As of 2026-06-12, `/Users/dev/bkgrnd-wopr` on the WOPR host is not a git worktree. It appears to be a manually deployed runtime directory containing the binary, web assets, data, logs, and timestamped backups.

Local repo status relevant to WOPR:

- `server/Cargo.lock`, `server/Cargo.toml`, `server/README.md`, `server/bkgrnd-server.service`, and `server/src/main.rs` are tracked.
- `server/web/` is currently untracked locally.
- The local worktree has existing uncommitted WOPR changes.

Before moving WOPR to a new server, commit or otherwise preserve:

- tracked WOPR Rust changes
- `server/web/`
- deployment/service config
- current data migration plan for `/Users/dev/bkgrnd-wopr/data/`

Do not treat the current WOPR host directory as the source of truth. Make the repo source of truth first, then deploy the new server from the repo plus migrated data.
