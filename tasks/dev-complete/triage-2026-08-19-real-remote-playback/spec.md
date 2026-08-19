# Triage: real remote YouTube playback

## Reproduction

A production remote command for a real non-live video from the user's history was accepted and exposed `isPlaying: true`, while media position remained at zero. The implementation currently equates an existing mpv process with successful playback and only starts stall/abnormal-exit recovery when yt-dlp labels the item live. The previous verifier counted one hard-coded live URL and therefore did not exercise ordinary remote-controlled videos.

## Issues

| # | Severity | Domains | Classification | Issue |
|---|----------|---------|----------------|-------|
| 1 | P0 | remote-control, playback | regression | Remote startup reports success before media is demonstrably playable, so a dead first resolver session prevents startup fallback. |
| 2 | P0 | playback, YouTube | regression | Stall and abnormal-exit recovery are restricted to live streams, leaving ordinary videos stuck or stopped. |

Issue dependency analysis: Issue 1's truthful startup state is required before Issue 2's recovery can be verified reliably. Execution order: 1 → 2.

## Acceptance Criteria

- [desktop] A remote non-live YouTube command must advance media position before startup is accepted; a resolver that only creates an idle mpv process must fall through automatically.
- [desktop] Stall monitoring and abnormal-exit fallback apply to all direct YouTube sessions, while still respecting pause, shuffle, session-generation, and bounded-strategy guards.
- [remote-control] Production verification must use the real command queue and three distinct URLs from the user's history: an ordinary video, a Mix URL, and a live stream. The old hard-coded verifier URL cannot satisfy this matrix.
- [desktop] Verification audio is muted and refuses to commandeer active user playback without explicit owner opt-in.

<!-- hosaka:domains desktop,remote-control,playback,youtube -->
