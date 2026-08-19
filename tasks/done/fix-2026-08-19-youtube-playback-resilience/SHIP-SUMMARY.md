# YouTube playback resilience hotfix

## Type
hotfix

## What
YouTube streams now start through a resilient resolver chain and live playback automatically recovers when media authorization expires or progress stalls.

## Why
Some streams failed immediately, while others stopped after roughly 40 seconds because later live-media segments were rejected.

## Changes
- Added ordered Proof-of-Origin, embedded, and legacy playback fallback.
- Added bounded live-stream stall detection and automatic recovery that preserves current user intent.
- Installed a pinned, loopback-only provider and a guarded, muted production playback verifier.
- Added automated desktop playback coverage and macOS CI.

## Verification

| What Was Changed | Method | Evidence |
|------------------|--------|----------|
| Ordered startup fallback in the installed Mac app | API | [Real YouTube playback entered the playing state after exercising the complete resolver chain](evidence/production-verification.json) |
| Automatic recovery beyond the former cutoff | API | [A forced stall recovered and an unforced live stream advanced for 95 seconds](evidence/production-verification.json) |
