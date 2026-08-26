# Requirements Contract — remote phone streaming range regression

Approved: 2026-08-26 (continuation of the approved remote-streaming turbo fix)

## What we are fixing

| # | Feature | Acceptance criteria (user-observable) |
|---|---|---|
| 1 | Reliable HPX progressive-audio proxy | Ordinary YouTube streams start on the phone and continue past the current immediate/40-second failure, while live HLS streams remain playable. |
| 2 | HPX POT and fallback resolver | Remote streaming uses the pinned PR #243 POT provider first, then `web_embedded`, then legacy extraction so non-embeddable ordinary videos still have a playable path. |

## Verification

- Reproduce production HTTP 502 with a real ordinary video and the range shape used by iOS/Safari.
- Add a failing server test for safe bounded upstream ranges, then make it pass.
- Add failing strategy-order and container-provider tests, then make them pass.
- Deploy HPX through the repository workflow.
- Verify production open-ended and sequential ranges across saved ordinary streams, then soak a real ordinary remote stream beyond the historical cutoff.

## Stop conditions

- No Mac playback changes.
- No Discord work.
- No unrelated UI or infrastructure changes.
