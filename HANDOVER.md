# bkgrnd Stopgap (WOPR + iPhone PWA) Handover

## Goal

Temporary iPhone-accessible `bkgrnd` using a PWA served from the WOPR box, backed by the existing WOPR streaming API, and keeping recent plays synced from this Mac to WOPR.

## Live URL

- `https://bkgrnd.bedl.am/`

## Server Topology (WOPR)

- SSH host alias: `ssh wopr` (macOS machine running Caddy)
- Caddy binary: `/usr/local/opt/caddy/bin/caddy`
- Caddy config: `/usr/local/etc/Caddyfile`
  - Routes `/api/v1/*` to `127.0.0.1:18081` (Rust `bkgrnd_server`)
  - Serves the web app at `/`
- Rust server deployed on WOPR:
  - Binary: `/Users/dev/bkgrnd-wopr/bkgrnd_server`
  - Web assets: `/Users/dev/bkgrnd-wopr/web/`
  - Data dir: `/Users/dev/bkgrnd-wopr/data/`
  - Logs: `/Users/dev/bkgrnd-wopr/bkgrnd_server.log`
  - Listen address: `127.0.0.1:18081` (check with `lsof -iTCP:18081 -sTCP:LISTEN`)
- Dependencies installed on WOPR:
  - `yt-dlp`: `/Users/dev/local/bin/yt-dlp`
  - `deno`: `/Users/dev/local/bin/deno`

## Server Rust API (`bkgrnd_server`, axum)

### Existing endpoints

- `GET /api/v1/health`
- `GET/PUT /api/v1/playlists.json`
  - Persisted as YAML at `data_dir/playlists.yaml`
- `GET /api/v1/search?q=...`
- `GET /api/v1/stream?url=<youtube-url>`
  - Proxies upstream bytes; supports Range passthrough (client seeks/reconnects)

### Added endpoints

- `GET/PUT /api/v1/history.json`
  - Stored as JSON at `data_dir/history.json`

### Auth behavior

- If `WOPR_BEARER_TOKEN` is set, requests require:
  - `Authorization: Bearer <token>`, or
  - `?token=<token>` query param (added as a stopgap for `<audio>` playback and browser clients)

## Critical Streaming Fix (matches local desktop behavior)

Local desktop backend (`src-tauri/src/ytdlp.rs`) tries `bestaudio` then falls back to `best`.

WOPR server was updated to:

- Prefer iOS-friendly formats first (to avoid flaky `audio/webm` in iOS Safari):
  1. `bestaudio[ext=m4a]`
  2. `bestaudio[acodec^=mp4a]`
  3. `bestaudio`
  4. `best`

### JS runtime support for `yt-dlp`

YouTube extraction on WOPR required a JS runtime.

- Env var: `WOPR_YTDLP_JS_RUNTIMES="deno:/Users/dev/local/bin/deno"`
- WOPR process `PATH` must include `/Users/dev/local/bin` so it can find `yt-dlp` and `deno`.

### Cookies support (not required in the final working path)

- Optional env var: `WOPR_YTDLP_COOKIES=/path/to/cookies.txt`
- If present, `yt-dlp` is invoked with `--cookies <path>`

## Web App (PWA)

- Source (repo): `server/web/`
- Deployed copy (WOPR): `/Users/dev/bkgrnd-wopr/web/`
- Must work at the site root:
  - Uses relative asset URLs (`styles.css`, `app.js`, `manifest.webmanifest`, `assets/...`)
  - Registers service worker relatively (`sw.js`)

### Features

- Search or paste URL
  - Uses `/api/v1/search`
  - Plays via `<audio>` using `/api/v1/stream`
- Playlists
  - Loads `/api/v1/playlists.json`
  - Renders playlists and items; plays items
- Recent
  - Loads `/api/v1/history.json` (server-backed)

### iOS UI hang fix

Safari can keep `await audio.play()` pending while buffering. The UI was updated to:

- Not await `audio.play()`
- Show `Buffering...` via `waiting/stalled` events
- Clear message on `playing`

## Sync From This Mac -> WOPR

### Local source data

- `~/.bkgrnd/history.json` exists locally
- No local `~/.bkgrnd/playlists.yaml` existed; server playlists are derived as "Recent" from history.

### Sync script

- `scripts/sync-wopr-data.sh`
  - `PUT` local `history.json` -> `https://bkgrnd.bedl.am/api/v1/history.json`
  - Derives playlist doc ("Recent") from history and `PUT`s -> `.../api/v1/playlists.json`

### Auto-sync (launchd)

- Installed: `~/Library/LaunchAgents/com.bkgrnd.wopr-sync.plist`
- Runs every 300s and on load
- Logs:
  - `/tmp/bkgrnd-wopr-sync.out.log`
  - `/tmp/bkgrnd-wopr-sync.err.log`

## Known Gotchas

- iOS Safari caching/service worker can keep old JS. Use a hard refresh / private tab if behavior seems stale.
- Caddy reload on WOPR required `sudo` previously due to a root-owned Caddy internal PKI directory; plan changes accordingly.
- Some YouTube URLs may still fail due to bot checks; cookies support exists but was not used in the final working path.

## Quick Verification Checklist

- `curl https://bkgrnd.bedl.am/api/v1/health` -> `ok`
- `curl -I "https://bkgrnd.bedl.am/api/v1/stream?url=<...>"` -> `200/206` and content-type `audio/mp4` or `application/vnd.apple.mpegurl`
- On WOPR: `tail /Users/dev/bkgrnd-wopr/bkgrnd_server.log` for `yt-dlp` errors
- On this Mac: `tail /tmp/bkgrnd-wopr-sync.err.log`

## Key Files Changed In Repo

- `server/src/main.rs`
- `server/Cargo.toml`
- `server/web/index.html`
- `server/web/app.js`
- `server/web/styles.css`
- `server/web/manifest.webmanifest`
- `server/web/sw.js`
- `scripts/sync-wopr-data.sh`
- `scripts/com.bkgrnd.wopr-sync.plist`
