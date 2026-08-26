# Regression Post-Mortem — HPX provider death silently degraded remote streaming

## What broke

The HPX POT provider was OOM-killed and never restarted. Public health remained green, so streams that needed a generated token failed while unrelated resolver paths could still work.

## Root cause

The container entrypoint performed provider readiness only at startup and then left no supervisor responsible for the child. Four concurrent memory-heavy resolutions ran beside the provider within a 512 MiB cgroup, and the Rust health endpoint did not check the required provider.

## Why tests missed it

The prior production matrix proved successful resolution and media bytes. It did not kill the provider, inspect cgroup OOM counters, validate memory headroom, or replay the exact user sequence after concurrent fresh resolution.

## How it occurred

Two transient yt-dlp/Deno processes ran alongside the resident Deno provider, exhausted the cgroup, and caused the kernel to kill the provider. Rust survived and continued reporting healthy, hiding the partial outage.

## Fix applied

The provider is supervised and restarted, health fails closed while it is unavailable, resolver concurrency is serialized, speculative prewarming is limited, and production verification requires a 1 GiB memory floor plus recovery and playback stress.

## Process gap

The previous acceptance plan measured successful output but did not validate dependency lifecycle or the runtime resource envelope.

## Prevention

- [ ] Require `scripts/verify-hpx-runtime-resilience.sh`—including provider fault injection, OOM/headroom checks, and the ordered Tokyo → Sunday Jazz → Night Work matrix—for every HPX production deployment.
