# Requirements Contract — fix-2026-08-19-youtube-playback-resilience: YouTube playback resilience hotfix
Approved: 2026-08-19T16:01:27Z

## What we are building
| # | Feature | Acceptance criteria (user-observable) |
|---|---------|----------------------------------------|
| 1 | Resilient YouTube stream startup | A YouTube URL that the current resolver rejects starts playback automatically by trying the patched Proof-of-Origin path first, then `web_embedded`, then the legacy resolver without requiring user intervention. |
| 2 | Automatic live-stream recovery | A live YouTube stream whose authorized media segments expire during playback resumes through the next resolver strategy automatically and continues beyond the current 40-second failure window. |

## How each feature will be verified
| # | Local proof | Production proof | Dixie routes | API probes | Data checks |
|---|-------------|------------------|--------------|------------|-------------|
| 1 | Rust resolver-order tests plus the full Tauri Cargo test suite. | Send a real YouTube URL through the installed Mac app and confirm playback enters the playing state. | N/A — native playback hotfix with no UI change. | Local control API `play` and `status`. | Status title and playback position are populated. |
| 2 | Rust stall-detector and fallback-state tests plus a real live-stream soak test. | Drive the installed Mac app through its local control API and confirm position is still advancing after at least 90 seconds. | N/A — native playback hotfix with no UI change. | Local control API `play` and repeated `status` probes. | Position advances on both sides of the historical 40-second cutoff. |

## Deploy plan
Target: the signed/local macOS `bkgrnd.app` installation plus `bedlamlabs/bkgrnd` via the repository delivery workflow. Monitor: installed-app health and playback status probes. Rollback: restore the pre-hotfix application bundle retained during installation.

## Stop conditions
No Discord integration and no unrelated server or UI work. Escalate only for failed production authentication, an unavailable deployment host, or a destructive production action outside replacing the explicitly requested application build. The hotfix is not delivered until the installed app passes the real 90-second live-stream proof.
