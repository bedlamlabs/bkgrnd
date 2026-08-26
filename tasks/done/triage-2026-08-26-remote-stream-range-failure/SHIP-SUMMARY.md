# Remote phone streaming reliability

## Type
triage

## What
Remote phone playback now serves real YouTube audio continuously through HPX, including reconnecting range requests and validated resolver fallbacks.

## Why
Ordinary remote streams either failed immediately or stopped near 40 seconds while live streams continued to work.

## Changes
- Bounded progressive streaming ranges to sizes accepted by current upstream media servers.
- Added POT, embedded, and legacy resolver strategies with full media-candidate validation.
- Installed and isolated the patched POT provider in the HPX runtime.
- Added direct production checks for saved ordinary streams and live HLS.

## Verification

| What Was Verified | Method | Evidence |
|-------------------|--------|----------|
| HPX returned the iPhone-style open range, four contiguous audio ranges, every saved failing stream, and the live-HLS manifest. | API | evidence/production-verification.json |
| A fresh production stream resolved through the ordered validated fallback and returned playable HTTPS media. | API | evidence/acceptance-test-production.json |
