# bkgrnd Mobile Follow-Up

Date: 2026-06-15

## Current State

- Live PWA: `https://bkgrnd.bedl.am/`
- Deployment host: `hpx@hpx.lan`
- Runtime: Docker Compose under `/etc/dokploy/compose/wopr-apps/code`
- Live container: `bkgrnd-hpx`
- HPX is the permanent/future host; WOPR is legacy and discontinued for this app.
- Public app is serving cache-busted `v=17` web assets.
- HPX container includes Deno and sets `WOPR_YTDLP_JS_RUNTIMES=deno:/usr/local/bin/deno`.

## App Store Connect Blocker

The App Store Connect API key is still missing. Needed values/files:

- `.p8` private key file
- Key ID
- Issuer ID
- Apple Team ID
- Confirmation that the iOS bundle ID is correct for the intended App Store/TestFlight app

The API key supports automated upload/TestFlight workflows. It does not by itself solve local device install if the developer account remains disabled.

## Tomorrow Checklist

1. iPhone PWA cache reset
   - Delete the existing Home Screen bookmark.
   - Open `https://bkgrnd.bedl.am/` in Safari.
   - Add it back to Home Screen.
   - Confirm UI says `Recent` and `Remote`.

2. Real phone playback QA
   - `Recent` should play locally on the phone through the HPX stream endpoint.
   - `Remote` should start/control MBP playback.
   - Search should return YouTube results.
   - Pasting a YouTube URL should start playback.
   - Now Playing and mini-player should remain usable on mobile viewport.

3. Streaming startup QA
   - Compare a known YouTube URL cold start vs repeat start.
   - Confirm repeat start uses cache or starts materially faster.
   - Watch HPX logs for `resolved stream url via ... in ...ms`.
   - Confirm no missing-JS-runtime warnings after the Deno container rebuild.

4. Spotify playlist reconstruction test
   - Paste a Spotify playlist into the menubar app.
   - Verify it enumerates tracks, searches YouTube, creates a YouTube-backed queue, and starts playback.
   - Repeat from the PWA if the deployed HPX/server path is expected to support Spotify URLs.
   - If HPX Spotify fails, check deployed environment for Spotify API credentials.

5. Native iOS app parity
   - Verify Swift app builds after the committed changes.
   - Once signing is available, test on-device:
     - background audio
     - lock screen metadata
     - Control Center commands
     - `Recent` / `Remote` / `Search` parity with PWA
   - Dynamic Island / Live Activity support is still a separate implementation item.

6. Distribution path
   - If Apple Developer access is restored, configure App Store Connect API key and TestFlight upload.
   - If account access remains blocked, keep PWA as the primary mobile app path.

## Rollback Notes

HPX source backup from the deploy:

- `/etc/dokploy/compose/wopr-apps/code/bkgrnd.bak.20260614T205759Z`

The deployed container data volume was not replaced:

- `/etc/dokploy/compose/wopr-apps/data/bkgrnd:/data`
