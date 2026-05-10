# WOPR Server (bkgrnd)

Minimal backend for:
- Playlist sync (`playlists.yaml`)
- Stable-ish streaming endpoint that proxies a fresh `yt-dlp`-resolved audio URL

## Run

Requirements on WOPR:
- `yt-dlp` available in `PATH`

Environment:
- `WOPR_BIND` (default `0.0.0.0:808`)
- `WOPR_DATA_DIR` (default `./data`)
- `WOPR_BEARER_TOKEN` (optional; if set, clients must send `Authorization: Bearer <token>`)

Commands:
- `cargo run --release`

## systemd (optional)

This repo includes a template unit file: `wopr-server/bkgrnd-wopr.service`.

Suggested setup on WOPR:
- Create a user: `sudo useradd --system --home /var/lib/bkgrnd --shell /usr/sbin/nologin bkgrnd`
- Put code at: `/opt/bkgrnd` (so binary ends up at `/opt/bkgrnd/wopr-server/target/release/wopr_server`)
- Create data dir: `sudo mkdir -p /var/lib/bkgrnd && sudo chown bkgrnd:bkgrnd /var/lib/bkgrnd`
- Install unit: `sudo cp /opt/bkgrnd/wopr-server/bkgrnd-wopr.service /etc/systemd/system/bkgrnd-wopr.service`
- `sudo systemctl daemon-reload && sudo systemctl enable --now bkgrnd-wopr`

## API

- `GET /api/v1/health` → `ok`
- `GET /api/v1/playlists` → YAML playlist document
- `PUT /api/v1/playlists` → stores YAML playlist document
- `GET /api/v1/playlists.json` → JSON playlist document
- `PUT /api/v1/playlists.json` → stores JSON playlist document
- `GET /api/v1/search?q=...` → JSON search results
- `GET /api/v1/stream?url=<youtube-url>` → proxies audio bytes

Notes:
- `/api/v1/stream` supports basic HTTP `Range` passthrough for iOS seeking/reconnects; no caching.
