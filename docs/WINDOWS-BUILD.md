# bkgrnd — Windows desktop build guide

Standalone Windows variant of the menubar app (Workstream B). Runs from the
**system tray** (Windows' menu-bar equivalent) with a popover window. Resolves
with bundled **yt-dlp.exe**, plays with bundled **mpv.exe** — **no relay, no
accounts, YouTube-only** for v1.

## What's already done (cross-platform, committed)
The Rust is now platform-aware and the macOS build is unchanged:
- **mpv IPC** — `src-tauri/src/mpv.rs`: Unix socket on macOS, **named pipe**
  (`\\.\pipe\bkgrnd-mpv-ipc-…`) on Windows, via `IpcStream`/`connect_ipc`. mpv
  binary is `mpv.exe`; the `DYLD_LIBRARY_PATH` dylib env is macOS-only (Windows
  DLLs sit next to `mpv.exe`). `stop_stale_mpv` (a /tmp sweep) is Unix-only.
- **yt-dlp sidecar** — `src-tauri/src/ytdlp.rs`: resolves `yt-dlp.exe` on Windows.
- **Tray/activation** — `ActivationPolicy::Accessory` is already `#[cfg(target_os = "macos")]`; the tray-icon + positioner plugins support Windows.

## Steps on the Windows machine

### 1. Toolchain
- Rust + **MSVC** toolchain (`rustup default stable-x86_64-pc-windows-msvc`).
- Node (for the front-end) + Tauri CLI (already a devDependency: `npx tauri`).
- **WebView2 runtime** (preinstalled on Win11; the installer can bootstrap it).

### 2. Vendor the Windows binaries (into `src-tauri/binaries/`)
- **yt-dlp:** download the official `yt-dlp.exe` from
  https://github.com/yt-dlp/yt-dlp/releases and save it as the Tauri sidecar name:
  `src-tauri/binaries/yt-dlp-x86_64-pc-windows-msvc.exe`
- **mpv:** get a Windows mpv build (mpv.exe + its DLLs) from https://mpv.io/installation/
  (e.g. the shinchiro/sourceforge builds). Put `mpv.exe` **and all its DLLs** in:
  `src-tauri/binaries/mpv-bundle-windows/`
  (These are large and platform-specific — do NOT commit them to git; keep them
  local or add via a release-asset step. See `.gitignore`.)

### 3. Point the bundle at the Windows mpv folder
Add **`src-tauri/tauri.windows.conf.json`** (Tauri v2 deep-merges platform config):
```json
{
  "bundle": {
    "resources": { "binaries/mpv-bundle-windows/*": "mpv-bundle/" },
    "windows": { "webviewInstallMode": { "type": "downloadBootstrapper" } }
  }
}
```
Note: the base `tauri.conf.json` still lists the macOS `mpv-bundle` resource —
if the Windows build tries to include it, move the macOS `resources` block into a
`tauri.macos.conf.json` and drop it from the base so each platform gets only its
own mpv. (Verify the macOS build still bundles correctly after that move.)

### 4. Build
```
npm install
npx tauri build
```
Produces an NSIS installer + MSI under `src-tauri/target/release/bundle/`.

### 5. First-run checks
- Tray icon appears; clicking it shows the popover.
- Search a YouTube URL → resolves (yt-dlp.exe) → plays (mpv.exe) with transport controls.
- No login prompts; no relay calls.

## Known Windows-specific things to watch
- **mpv named-pipe IPC** is new code — first place to check if playback controls
  don't respond (verify the pipe name + that mpv accepts `--input-ipc-server` with it).
- **Tray positioning** — `Position::TrayBottomCenter` behaves differently on
  Windows; may need a Windows-specific position.
- **Close/minimize to tray** and run-at-startup are Windows conventions to wire.
- **Relay/Remote sync** (`wopr_sync`) — optional; leave the `wopr_*` config empty
  for a pure standalone install, or gate it out with a build flag.
- **Code signing** (optional) avoids SmartScreen warnings on other users' machines.

## Out of scope (v1)
Spotify (needs API creds), the relay/Remote feature, and any auto-update.
