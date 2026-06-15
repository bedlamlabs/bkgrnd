# bkgrnd

A self-contained macOS menubar app for playing YouTube audio. No browser window, no ads, no dependencies to install.

![macOS](https://img.shields.io/badge/macOS-Apple%20Silicon-black)

## Install

1. Download `bkgrnd_1.0.0_aarch64.dmg` from [Releases](https://github.com/bedlamlabs/bkgrnd/releases)
2. Open the DMG and drag **bkgrnd** to Applications
3. Launch bkgrnd — it appears as a menubar icon (no dock icon)
4. On first launch: right-click the app > Open (required for unsigned apps)

## Usage

Click the menubar icon to open the popover. Paste a YouTube URL and press Enter.

- **Single videos** — extracts audio via yt-dlp, plays through mpv
- **Playlists** — enumerates tracks, auto-advances, shows queue position
- **Live streams** — works with YouTube live streams
- **Spotify playlists/albums** — uses Spotify metadata to build a YouTube-backed queue
- **History** — recent plays saved to `~/.bkgrnd/history.json`

Spotify URLs do not play Spotify streams directly. bkgrnd reads the Spotify track list, searches YouTube for each track, and plays the resulting YouTube matches. Set `BKGRND_SPOTIFY_CLIENT_ID` and `BKGRND_SPOTIFY_CLIENT_SECRET` for Spotify Web API metadata access, or provide `BKGRND_SPOTIFY_ACCESS_TOKEN`. `BKGRND_SPOTIFY_MAX_TRACKS` limits conversion size and defaults to 75.

### Controls

| Control | Action |
|---------|--------|
| Space | Pause / Resume |
| Seek buttons | Skip forward/back 10s |
| Prev / Next | Navigate playlist queue |
| Volume slider | Adjust playback volume |
| Stop | Stop playback and clear queue |
| Escape | Hide window |
| Cmd+Q | Quit |

### Tray icon states

| Icon | Meaning |
|------|---------|
| Default | Idle |
| Green | Streaming audio |
| Yellow | Paused |

## Architecture

Single Tauri 2 app (Rust + HTML/CSS/JS) with all dependencies bundled:

```
bkgrnd.app/
├── Contents/
│   ├── MacOS/
│   │   ├── bkgrnd          # Tauri app binary
│   │   └── yt-dlp          # YouTube stream extractor (sidecar)
│   └── Resources/
│       └── mpv-bundle/     # mpv + 64 dylibs (loader paths rewritten)
│           ├── mpv
│           ├── libavcodec.62.dylib
│           └── ...
```

- **yt-dlp** extracts audio stream URLs from YouTube
- **mpv** plays the audio stream, controlled via Unix socket IPC
- **No Chromium/browser** — pure stream extraction + native playback
- **~120 MB total** (yt-dlp 35 MB + mpv bundle 81 MB + app 5 MB)

## Building from source

Requires: Rust, Node.js, and Homebrew (for mpv + dylibs at build time only).

```bash
# Install build-time dependencies
brew install mpv

# Bundle mpv + all dylibs (rewrites loader paths)
./scripts/bundle-mpv.sh

# Download yt-dlp standalone binary
curl -L -o src-tauri/binaries/yt-dlp-aarch64-apple-darwin \
  https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp_macos
chmod +x src-tauri/binaries/yt-dlp-aarch64-apple-darwin

# Install JS dependencies and build
npm install
npm run build
```

The built app and DMG will be in `src-tauri/target/release/bundle/`.

## Requirements

- macOS 13+ (Ventura or later)
- Apple Silicon (aarch64)
- No runtime dependencies — everything is bundled

## License

MIT
