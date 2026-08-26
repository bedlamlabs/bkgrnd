# Triage: HPX resolver runtime survivability

## Type
triage

## What
- Remote phone streams now keep working through resolver-provider failure and sustained multi-stream load instead of silently stopping.

## Why
- HPX memory pressure killed the required provider while health continued reporting success, leaving remote playback broken until the container was restarted.

## Changes
- The provider is supervised and automatically restarted, health reflects provider availability, resolver work is bounded, and HPX has measured memory headroom.
- Provider dependencies are readable by the non-root runtime while remaining root-owned and non-writable.
- Production acceptance covers provider death, recovery, resolver concurrency, repeated real-stream ranges, container stability, and memory use.

## Verification

| What Was Verified | Method | Result |
|-------------------|--------|--------|
| Remote health and repeated phone-equivalent playback recover after the provider is stopped, while the HPX container remains stable within memory limits. | API | [Production health returned 200; provider outage and recovery were observed; three concurrent resolves and two Tokyo Night Drift → Sunday Morning Jazz → Night Work rounds completed with no OOM or restart drift.](evidence/production-verification.json) |
