# Triage: 2 issues

| # | Sev | Domains | Category | URL / Route | Issue | File(s) |
|---|---|---|---|---|---|---|
| 1 | P0 | api | API | `/api/v1/stream?proxy=true` | Ordinary YouTube audio returns HTTP 502 for the 8 MiB ranges used by remote phone playback, while live HLS streams remain playable. | `server/src/main.rs` |
| 2 | P0 | api, deployment | API / Runtime | HPX yt-dlp resolver | The patched POT provider and `web_embedded` fallback were absent from HPX; remote streaming still used legacy `android_vr` and default extraction, leaving saved ordinary streams with unusable direct URLs. | `server/src/main.rs`, `Dockerfile`, `server/container-entrypoint.sh` |

Issue dependency analysis: Issue 2 depends on Issue 1's bounded proxy ranges for end-to-end playback verification.

## Acceptance Criteria

- [api] The HPX proxy serves the initial and subsequent byte ranges for a real ordinary YouTube video without HTTP 502, including an open-ended range matching iOS/Safari behavior.
- [api] Remote progressive playback continues beyond the former 40-second cutoff while live HLS playback remains intact.
- [api] HPX tries the pinned PR #243 POT provider first, then `web_embedded`, then legacy extraction for streams that do not support embedding.
- [deployment] The provider is pinned, checksum-verified, loopback-only, healthy before the Rust server starts, and present in the deployed container.

<!-- hosaka:domains api,deployment -->
