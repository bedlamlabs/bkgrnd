# bkgrnd Code Review — 2026-07-02

Scope: pre-upgrade review of (1) the macOS menubar app (`src-tauri/` + `src/`),
(2) the relay server on HPX (`server/src/main.rs`, Docker on Dokploy), and
(3) the remote web app at bkgrnd.bedl.am (`server/web/`), ahead of the iOS
conversion and 2x mockups. Focus areas per Geoffrey: stream speed (local +
remote), stability, and the Spotify → YouTube conversion path.

Repo state reviewed at commit `694f5b9`.

---

## 1. Spotify → YouTube conversion (untested path) — VERDICT

The code path is real and mostly sound (`src-tauri/src/spotify.rs` →
`ytdlp::search_first_music` per track → queue), but **it will not work as
shipped for most launches**, for four reasons:

### S1 (blocker) — Credentials arrive via env vars, but GUI apps don't get your shell env
`access_token()` reads `BKGRND_SPOTIFY_CLIENT_ID/SECRET/ACCESS_TOKEN` from
`std::env::var`. A `.app` launched from Finder/menubar inherits launchd's
environment, not `.zshrc`. Unless you launch bkgrnd from a terminal, the
conversion always fails with the credentials error. Meanwhile `config.rs`
already has a `~/.bkgrnd/config.yaml` mechanism (used for `wopr_base_url` /
`wopr_token`) — Spotify creds should move there (fields + settings UI), with
env as override only.

### S2 (blocker for playlists) — One failed track search aborts the whole conversion
In `spotify::enumerate`, `ytdlp::search_first_music(...).await?` propagates
any single yt-dlp failure. Converting a 75-track playlist forks 75 sequential
yt-dlp searches; one transient failure (rate limit, network blip) errors the
entire conversion. Should be skip-and-continue, with a summary ("68 of 75
matched").

### S3 (speed) — Fully sequential, blocking, before any audio starts
75 tracks × ~1.5–3 s per `ytsearch1` = **2–4 minutes of "Connecting..."**
before the first note. Fix: resolve track 1, start playback immediately, then
fill the rest of the queue in the background with bounded concurrency (3–4
parallel sidecars). Also cache the client-credentials token (currently
re-fetched per conversion — minor).

### S4 (architecture gap) — Conversion exists ONLY in the Mac app
The relay server has no Spotify support at all. On the web app / future iOS
app: "remote" scope with a Spotify URL goes to `/api/v1/resolve` → yt-dlp,
which does not handle Spotify → hard fail. It only works in "local" scope by
relaying the URL to the Mac. **For the iOS app the conversion must be ported
into the relay server** (same enumerate/search logic; it's plain reqwest +
yt-dlp, no Tauri dependency — good candidate for a shared crate between
`src-tauri` and `server`).

Also note: Spotify's client-credentials flow only reads *public* playlists,
and Spotify has restricted several endpoints for new API apps (Nov 2024
changes). Verify the registered app can hit `/v1/playlists/{id}` at all —
that's the first thing to smoke-test once S1 is fixed.

---

## 2. Stability findings

### A1 (high, local) — Watcher race can kill playlist auto-advance or misreport state
`player.rs` spawns a polling watcher per play: single-track play spawns a
"reaper" (play(), ~line 174) and `play_queue_item` spawns an "auto-advance"
watcher (~line 230). Neither is tied to the session it was spawned for — they
just poll `s.session`. Sequence: play single track → play a playlist before
the old reaper exits → old reaper `try_wait()`s the *new* session's child,
reaps its exit, sets `session = None`, and swallows the exit the auto-advance
watcher needed. Result: playlist stops mid-queue / tray stuck. Fix: add a
generation counter to `PlayerState`, stamp each session, and have watchers
exit when the generation changes. (Better long-term: subscribe to mpv IPC
`end-file` events instead of polling `try_wait` at 500 ms.)

### A2 (high, remote) — Stale cached stream URLs are never evicted on failure
Server caches resolved googlevideo URLs up to 90 min
(`cache_ttl_for_stream_url`). If the URL dies early (YouTube invalidates,
IP/UA mismatch, 403), `stream_audio` returns 502 but leaves the dead URL in
`stream_cache`, so every retry for up to 90 minutes re-serves the corpse. The
phone shows "reconnect" forever with no recovery. Fix: on upstream non-2xx/206
in `stream_audio`, evict the cache entry (and optionally re-resolve once
inline).

### A3 (high, remote/iOS) — Redirect mode hands out IP-locked URLs; the iOS client uses it
`/api/v1/stream` without `proxy=true` issues a 302 to the raw googlevideo URL.
Those URLs are typically bound to the resolver's IP (HPX), so any off-network
client following the redirect gets 403. The PWA always sends `proxy=true`, but
`ios/bkgrnd/Sources/WOPRClient.swift#streamURL` builds the URL **without
`proxy=true` and without the token query param** — iOS playback via AVPlayer
will 302 to a URL it can't fetch. Fix in WOPRClient (add `proxy=true` +
token), and consider making proxy the server default.

### A4 (medium, remote) — Unbounded yt-dlp process spawn on the relay
`/api/v1/search`, `/resolve`, `/prewarm` each fork yt-dlp with no global
concurrency cap. The PWA's prewarm queue is client-side serialized, but N
clients (or a misbehaving one) can fork dozens of yt-dlp+deno processes on
HPX (each is CPU/RAM heavy). Given HPX's history of resource exhaustion
outages, add a `tokio::sync::Semaphore` (e.g. 3–4 permits) around all yt-dlp
invocations.

### A5 (medium, remote) — In-memory caches grow without bound
`stream_cache`, `stream_failures` (1 h TTL entries), and prewarm sets are
never pruned — expired entries stay in the HashMaps forever. Slow leak on a
long-lived container. Sweep expired entries opportunistically on insert.

### A6 (medium, local) — Single shared mpv IPC socket path
`/tmp/bkgrnd-mpv-ipc-$USER` is constant. Two app instances (or a crashed mpv
plus a new one) fight over the socket; `pause()`/`get_paused()` talk to
whichever mpv owns the path. `stop_stale_mpv` runs only at startup. Use a
per-PID socket owned by the session, stored in `MpvSession`.

### A7 (low) — `get_status` lock gap
`player::get_status` drops the state lock, does 3 mpv IPC round-trips (each
with a 3 s timeout), re-acquires. Called every 1.5 s by the menubar UI *and*
every 2 s by `wopr_sync` — when mpv is wedged, status calls stack up to ~9 s
each and the UI goes "offline" spuriously. Batch the three property reads into
one IPC connection, and consider a single cached status refreshed by one
poller.

### A8 (low) — Two competing sync writers
The launchd job `scripts/sync-wopr-data.sh` (every 300 s) and the in-app
`wopr_sync::sync_once` both PUT playlists/history with last-write-wins on
hand-rolled timestamps. If the launchd agent is still loaded, retire it — the
app loop supersedes it. (Also: `parse_ts`/`chrono_now` are hand-rolled in 3
places; one `time`-crate helper would remove ~150 lines.)

---

## 3. Speed findings

### P1 (remote) — Server resolve doesn't use the fast player client
The Mac app pins `--extractor-args youtube:player_client=android_vr`
(ytdlp.rs, noted as "markedly faster per resolve") with fallback to default.
The server's `resolve_direct_url` doesn't — every cold remote resolve pays the
multi-client extraction cost, which is most of the 5–15 s "resolving" the
phone shows. Port the fast-client-with-fallback loop to the server. This is
the single biggest remote-speed win.

### P2 (remote) — Search is uncached and slow
`/api/v1/search` runs `ytsearch10:<q> music` (full flat-playlist dump) per
keystroke-submit, 10–30 s worst case, no cache. Add: fast player client, a
small LRU keyed on query (5-min TTL), and consider `--flat-playlist
--print`-based extraction instead of full JSON dump.

### P3 (remote) — First-byte latency: resolve + proxy are serialized with playback
The prewarm endpoint helps, but `playRemote` resolves, then sets `audio.src`,
then iOS issues its first Range request, which re-enters `resolve` (cache hit)
then opens the upstream connection. Consider having `/resolve` also prefetch
the first ~1 MB into a short-lived buffer so the first Range request is served
instantly. For the iOS app, keep the same prewarm-on-render pattern the PWA
uses (WOPRClient currently has no prewarm call).

### P4 (local) — Spotify conversion latency (see S3) and playlist start
`enumerate_playlist` on large YouTube playlists also blocks start; same
pattern applies — start item 1, enumerate the rest in the background.

### P5 (web) — Full grid re-render every 3 s
`refreshLocalStatus` runs `renderGrid()` + `renderMiniPlayer()` every 3 s,
rebuilding the entire mix grid DOM (24 cards + images) even when nothing
changed. Diff on a fingerprint (or only re-render the status strip/dot) —
matters on older iPhones. Same file: media session has play/pause handlers
but never sets `navigator.mediaSession.metadata`, so the iOS lock screen shows
nothing — cheap, high-visibility win while the PWA remains the stopgap.

### P6 (remote control) — Command relay latency is up to ~4 s
Phone → server queue → Mac polls every 2 s → status published on next tick.
Feels laggy for pause/next taps. Cheap: drop poll interval to 1 s with jitter.
Right: long-poll `commands/next` (hold up to 25 s) — no protocol change on the
phone side, sub-second command pickup, and *less* idle traffic than 2 s
polling. Do this before the iOS app bakes in expectations.

---

## 4. Security / correctness notes (brief)

- Token-in-query-string (`?token=`) is a documented stopgap; it leaks into
  Caddy/Dokploy logs. Fine for personal use; for the iOS app use the
  Authorization header everywhere (AVPlayer supports custom headers via
  `AVURLAsset` options) and consider dropping query-token support after.
- `auth_ok` compares tokens non-constant-time. Low risk; `subtle` crate is a
  one-liner if you care.
- `put_history_json` / `put_playlists` accept unbounded bodies → disk fill.
  Add axum `DefaultBodyLimit` (e.g. 2 MB).
- Live-stream HLS via proxy: manifests are proxied verbatim, but their
  absolute googlevideo segment URLs are fetched directly by the client and
  may be IP-locked like A3. Untested remotely — verify a live stream from
  cellular before assuming it works; a real fix is manifest rewriting to
  route segments through the proxy.
- `history.rs` migration renames `~/.play` → `~/.bkgrnd` silently; harmless
  but delete once migration window has passed.

---

## 5. Testing & CI

- Only tests in the repo: 3 URL-parse tests in `spotify.rs`. No server tests,
  no JS tests, no `cargo test` / build workflow (only `ios-ipa.yml`).
- Highest-value additions given the upgrade plan:
  1. Server: `parse_range_start_end`, `cache_ttl_for_stream_url`,
     `classify_ytdlp_error`, `extract_youtube_id` are pure — trivial to test.
  2. Spotify conversion: extract the "tracks → search queries → queue"
     mapping behind a trait so it's testable without yt-dlp.
  3. CI: `cargo build --locked` + `cargo test` for `server/` and `src-tauri/`
     on push (they currently only get compiled when someone deploys).

---

## 6. iOS conversion readiness (context for the mockup work)

A SwiftUI skeleton already exists (`ios/bkgrnd/Sources`, ~1,090 lines:
WOPRClient, AudioPlayer, Root/Search/Recent/NowPlaying/Settings views) plus
`ios-ipa.yml` CI and v2–v4 mockups in `mockups/`. Gaps to close, in order:

1. Fix A3 (proxy + auth in `WOPRClient.streamURL`) — playback is broken
   without it.
2. Port Spotify conversion server-side (S4) so iOS gets it for free.
3. Long-poll command channel (P6) for snappy remote control of the Mac.
4. Server-side queue semantics: the relay has no notion of a queue — the PWA
   and iOS app can only play single items. If the iOS app should auto-advance
   playlists remotely, either the client owns the queue (simplest: iOS
   AVQueuePlayer + per-item resolve) or the server grows queue endpoints.
5. Media session metadata / Now Playing info (lock screen + AirPods controls)
   on both PWA (stopgap) and iOS (`MPNowPlayingInfoCenter`).

---

## Suggested fix order

| # | Item | Component | Effort | Payoff |
|---|------|-----------|--------|--------|
| 1 | P1 fast player client on server resolve | relay | S | biggest remote-speed win |
| 2 | A2 evict dead cached stream URLs | relay | S | fixes "reconnect forever" |
| 3 | A3 iOS/redirect proxy+token | relay+ios | S | unblocks iOS playback |
| 4 | S1 Spotify creds → config.yaml + UI | menubar | S | makes conversion usable at all |
| 5 | S2/S3 skip-failures + play-first-resolve-rest | menubar | M | conversion works and starts fast |
| 6 | A1 session-generation watchers | menubar | M | playlist stability |
| 7 | A4 yt-dlp semaphore + A5 cache sweep | relay | S | HPX stability |
| 8 | P6 long-poll commands | relay+menubar | M | snappy remote |
| 9 | S4 Spotify conversion server-side | relay | M | iOS/PWA parity |
| 10 | P5 grid diff + media-session metadata | web | S | stopgap polish |
