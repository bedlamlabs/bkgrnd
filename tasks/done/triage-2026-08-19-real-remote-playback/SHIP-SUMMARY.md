# Triage: real remote YouTube playback

## Type
triage

## What
Remote-controlled YouTube playback now waits for real media progress, falls through unusable startup sessions, and recovers ordinary videos as well as live streams.

## Why
The earlier resilience delivery could report a non-playing ordinary video as successful and applied automatic recovery only to live media.

## Changes
- Remote startup now requires observable playback advancement.
- Resolver fallback handles startup sessions that resolve but never become usable.
- Stall and abnormal-exit recovery covers every YouTube session while preserving user intent.
- The muted production matrix uses distinct real ordinary, Mix, and live-history items and excludes the former curated verifier URL.

## Verification

| What Was Changed | Method | Evidence |
|------------------|--------|----------|
| Truthful ordinary-video startup and fallback | API | [A real remote command rejected an unusable first session, switched resolver, and advanced](evidence/production-verification.json) |
| Automatic recovery for ordinary YouTube playback | API | [A real ordinary video recovered and advanced throughout a muted 95-second soak](evidence/production-verification.json) |
| Diverse real-history playback | API | [Plain, actual Mix, and live-history items advanced with the old verifier excluded](evidence/production-verification.json) |
