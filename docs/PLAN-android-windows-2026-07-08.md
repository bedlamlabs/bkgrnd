# bkgrnd — Android + Windows Expansion Plan (2026-07-08)

## Scope (locked 2026-07-08)
Two deliverables:
1. **Windows desktop app** — standalone port of the Tauri menubar (system-tray + popover), bundled yt-dlp + mpv, no relay needed. Built locally on Geoffrey's Windows machine.
2. **Native Android app, distributed as a sideloaded APK** — phone-only, streams through the relay. Audio reaches the car over **Bluetooth**.

**Explicitly out of scope (deferred):** **Android Auto** in-car projection. It's
the only piece that requires **publishing to Google Play + passing Google's
review**, and a sideloaded APK cannot project to a real head unit. The Android
app is architected (Media3 `MediaSession`) so Auto can be added later as a flip,
but no `MediaLibraryService` / Play Store work now. (iPhone users already have
CarPlay in any modern car; Android Auto only matters for Android-phone users, and
Bluetooth covers the car audio in the meantime.)

---

Original three-item framing (for reference): (1) Android Auto — deferred,
(2) native Android APK — in scope (sideloaded), (3) Windows desktop — in scope.

## Where things stand today
- **iOS** — native app + **CarPlay** (shipped 2026-07-08) + call/SMS auto-resume. Streams through the relay (bearer token).
- **Android** — **PWA only** (per-user login via cookie session; streams through the relay).
- **Desktop** — **macOS menubar** (Tauri v2). Confirmed **standalone for playback**: resolves with bundled **yt-dlp**, plays with bundled **mpv**, playlists/history stored **locally**. The relay (`wopr_sync`) is used only for the optional "Remote-control target" feature; playback needs no relay.
- **Relay** — Rust/axum on hpx: proxied `/api/v1/stream`, per-user auth (`/api/v1/pwa/*`), playlists/history/search/spotify.

---

## Workstream A — Native Android app (APK) + Android Auto  (items 1 & 2)

### Clarification: CarPlay vs Android Auto
CarPlay is iOS-only and already shipped. The Android in-car equivalent is
**Android Auto**. There is no "CarPlay on Android" — the APK delivers Android
Auto. **Android Auto cannot use the PWA**; it requires a *native* app, so items
1 and 2 collapse into one native Android build.

### What to build
- **Native Android app** (Kotlin + Jetpack Compose) with parity to the PWA/iOS: per-user **login**, browse library, search, play, bookmark, Now Playing.
- **Auth** — reuse the relay's per-user endpoints (`/api/v1/pwa/login`). Native clients can hold the session cookie, but cleaner is a small server tweak to **return the session token in the login response body** so the app sends it as a bearer/cookie explicitly. (Only server change in this workstream.)
- **Playback** — **Media3 / ExoPlayer** streaming `/api/v1/stream?proxy=true` from the relay; background playback + **MediaSession** for lock-screen/notification controls.
- **Android Auto** — a Media3 **`MediaLibraryService`** exposing the library as a browsable tree + the MediaSession. Android Auto renders the browse UI + Now Playing automatically (the analog of the iOS `CarPlayController`).
- **Spotify** (optional v1) — reuse the relay's `/api/v1/spotify/queue` conversion.

### Distribution & testing (in scope)
- **Sideloaded APK**, phone-only. Install directly; audio to the car over Bluetooth.
- No Google Play / no Auto review in this scope.

### Deferred (phase 2, not now)
- `MediaLibraryService` + Android Auto browse tree, DHU testing, Play Store listing + Google Auto review. Build the app so this is a later flip, not a rewrite.

### Effort / risk
- Largest piece — a new native client from scratch (Kotlin/Compose + Media3).
- Relay work is minimal (auth tweak; everything else reused).

### Milestones (v1, phone-only)
1. App skeleton + login + library browse + ExoPlayer playback (PWA core parity).
2. MediaSession + notification/lock-screen transport controls (also sets up Auto later).
3. Search, bookmark, video-id dedup parity, polish.
4. Signed release APK for sideloading.

---

## Workstream B — Windows desktop app  (item 3)

### "Windows has no menu bar" — resolved
Windows has a **system tray** (notification area), which is the direct menu-bar
equivalent. The current app already uses Tauri v2 `tray-icon` +
`tauri-plugin-positioner`, so the Windows variant should be a **system-tray app
with a popover window** — the same UX as the Mac menubar. **Not** a floating
widget, and it doesn't need to become a full standalone window. (The macOS-only
`ActivationPolicy::Accessory` call is gated per-platform.)

### Standalone + credentials (your sidebar question)
The desktop app is **standalone** — it does its own resolving and playback and
does **not** depend on the relay to play:
- **Plain YouTube:** **no login / no credentials** needed — bundled yt-dlp resolves public videos.
- **Spotify playlists/albums:** needs **Spotify Web API client id/secret** (bundle them to enable Spotify, or ship YouTube-only; YouTube works either way).
- **Relay token:** only for the optional "Remote-control target" sync — **omit/disable** it for a standalone install on someone else's machine.

So on another user's Windows box: install it and YouTube playback just works, no accounts required. (Contrast: the **Android APK streams through the relay**; the **Windows desktop app is standalone**.)

### What to build
- Add a **Windows target** to the existing Tauri v2 app (logic is already cross-platform).
- **Bundle Windows binaries:** `yt-dlp.exe` + `mpv.exe` (+ its DLLs). Today only Mac-ARM binaries are vendored (`src-tauri/binaries/`); add per-platform sidecars.
- **Windows platform work:** tray click/positioning behavior, gate the macOS-only activation policy, taskbar/startup/close-to-tray behavior.
- **Build pipeline:** builds run **locally on Geoffrey's Windows machine** (decided 2026-07-08) — no CI runner needed. `tauri build` on Windows produces the `.exe`/MSI/NSIS installer.
- Optional: a `standalone` config flag that strips the relay/Remote sync from this build.
- Optional: **Windows code signing** to avoid SmartScreen warnings.

### Effort / risk
- Moderate — a port, not a rewrite. Risk concentrates in Windows binaries (mpv + DLLs), tray/window quirks, and standing up a Windows build environment.

---

## Sequencing & shared notes
- **Relay:** essentially unchanged. One optional tweak: token-in-login-response for the native Android app. Android streams via the relay; Windows is standalone.
- **Suggested order:** Windows first is the faster standalone win (it's a port of working code); Android is the larger build but unlocks in-car Android. Pick based on which user need is more urgent.

## Decisions
- **Windows build env:** ✅ **local Windows machine** (2026-07-08) — no CI runner.
- **Scope:** ✅ Windows standalone desktop + sideloaded native Android APK (phone-only). Android Auto deferred to phase 2.
- **Android auth:** per-user login like the PWA (assumed — confirm).
- **Spotify:** ✅ **YouTube-only for v1** (2026-07-08) — no Spotify on either new client to start; keeps the Windows install credential-free. Spotify deferred.
- **Priority:** ✅ **Windows first** (2026-07-08), then the Android APK.

### Fully locked — ready to build when you are.
First up when we start: **Workstream B (Windows desktop)** — port the Tauri app,
vendor `yt-dlp.exe` + `mpv.exe`, wire the Windows tray/popover, YouTube-only,
build locally on your Windows machine.
