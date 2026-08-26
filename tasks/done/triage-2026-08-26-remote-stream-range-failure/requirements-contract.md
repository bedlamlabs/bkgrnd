# Requirements Contract — HPX resolver runtime survivability

Approved: 2026-08-26T19:58:18Z

## What we are building

| # | Feature | Acceptance criteria (user-observable) |
|---|---------|----------------------------------------|
| 1 | Self-healing remote-stream resolver | Remote phone playback does not silently degrade after resolver memory pressure: the POT provider restarts automatically, HPX reports unhealthy while it is unavailable, resolver concurrency stays within measured container headroom, and Tokyo Night Drift → Sunday Morning Jazz → Night Work succeeds repeatedly. |

## How each feature will be verified

| # | Local proof | Production proof | Dixie routes | API probes | Data checks |
|---|-------------|------------------|--------------|------------|-------------|
| 1 | Runtime supervision, truthful-health, and bounded-concurrency tests | Repeated three-stream phone-equivalent sequence plus concurrent resolver stress on HPX | N/A — backend runtime | GET `/api/v1/health`; authenticated `/api/v1/resolve` and `/api/v1/stream` ranges | Provider remains reachable after stress; container stays healthy without OOM/restart drift |

## Deploy plan

Target: `https://bkgrnd.bedl.am` via GitHub/HPX deployment owner. Monitor: exact-SHA CI plus HPX container/provider health. Rollback: redeploy the prior known-good runtime commit through the HPX deployment wrapper.

## Stop conditions

- No Mac playback changes.
- No Discord work.
- No unrelated UI changes.
- No Docker pruning, volume deletion, or database changes.
- Upstream YouTube refusal may trigger validated fallback, but dependency death or OOM may not remain silent.
