# Requirements Contract — real remote YouTube playback correction
Approved: 2026-08-19

## What we are building
| # | Feature | Acceptance criteria (user-observable) |
|---|---------|----------------------------------------|
| 1 | Truthful remote startup fallback | Remote playback of an ordinary YouTube video starts only after media actually advances; an unusable first resolver session automatically falls through without falsely showing successful playback. |
| 2 | Recovery for every YouTube session | Remote-controlled ordinary videos as well as live streams recover from stalls or abnormal media exits through the next resolver strategy without user intervention. |

## How each feature will be verified
| # | Local proof | Production proof | Dixie routes | API probes | Data checks |
|---|-------------|------------------|--------------|------------|-------------|
| 1 | Rust readiness and startup-fallback tests plus the full desktop suite. | Send real non-live history URLs through the production remote command queue and confirm position advances. | N/A — native playback behavior. | Production local-command and status endpoints. | Source URL, playing state, and position progression all agree. |
| 2 | Rust non-live stall and abnormal-exit recovery tests. | Run a muted 95-second matrix covering a plain video, a Mix URL, and a live history stream; do not count the prior verifier URL. | N/A — native playback behavior. | Production local-command and repeated status probes. | Position advances beyond the former cutoff and resolver strategy changes when forced to fail. |

## Deploy plan
Build and safely replace the installed Mac app, push through the repository workflow, monitor macOS CI, and verify the production remote path while the app is idle and muted. Roll back using the retained pre-install application backup.

## Stop conditions
No Discord work, no server/UI expansion, and no audible or active-session-destructive verification. Escalation is limited to unavailable production infrastructure, failed authorization, destructive authority, or a material requirement choice outside this contract.
