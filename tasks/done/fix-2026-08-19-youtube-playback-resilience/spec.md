# YouTube playback resilience hotfix

## Reproduction

Current YouTube live HLS URLs start normally, then newly requested media segments begin returning HTTP 403 after roughly 35–40 seconds. Some videos fail during initial extraction. The failure reproduces independently of remote-phone control: local playback also stalls. `web_embedded` produces usable later segments for the reproduced live streams, while the current `android_vr` or default path does not.

## Fix

Use an explicit ordered resolver strategy: patched bgutil Proof-of-Origin first, `web_embedded` second, and the existing legacy extraction last. Carry the chosen strategy into the player session. Monitor live playback progress; when an unpaused session stops advancing long enough to represent a real stall, re-resolve the original URL with the next strategy and replace the session while preserving the user's playback intent.

The recovery loop must be bounded, ignore paused playback, avoid fighting newer user commands, and expose enough diagnostics to identify which strategy failed.

## Verification

Unit-test resolver order and the stall state machine. Run the complete desktop Rust suite. Build and install the Mac app, command it through its real local control API, and verify a real live stream keeps advancing for at least 90 seconds.

## Implementation surface

- `src-tauri/src/ytdlp.rs`: ordered POT, `web_embedded`, and legacy resolution for direct and Spotify-backed YouTube playback.
- `src-tauri/src/player.rs` and `src-tauri/src/mpv.rs`: live progress monitoring, bounded recovery, pause/session ownership guards, and recovery metadata refresh.
- `scripts/install-bgutil-provider.sh` and `scripts/install-macos.sh`: pinned loopback-only provider installation and safe app replacement.
- `scripts/verify-youtube-playback.sh`: installed-app startup fallback, deterministic stall recovery, real provider-use proof, and an unforced 95-second live soak.

The executable Rust suite covers strategy order, legacy attempts, missing-URL diagnostics, mpv-start fallback, stalled/paused/exhausted recovery, stale-session suppression, abnormal exits, shuffle precedence, and recovered metadata. The task acceptance harness runs those tests directly; production acceptance drives the installed app through its control API.
