# Triage: HPX resolver runtime survivability

| # | Sev | Domains | Category | URL / Route | Issue | File(s) |
|---|---|---|---|---|---|---|
| 1 | P0 | api, infra | Runtime reliability | `/api/v1/health` and remote stream resolver | The 512 MiB HPX container OOM-killed the POT-provider daemon while concurrent yt-dlp/Deno resolvers ran. The provider never restarted and Rust health remained green, causing source-dependent remote playback failures. | `server/container-entrypoint.sh`, `server/src/main.rs`, `Dockerfile`, HPX runtime configuration |

## Acceptance Criteria

- [api] Public HPX health is non-success while the required provider is unavailable and returns success after recovery.
- [infra] The provider automatically restarts after process death without requiring a container restart.
- [infra] Resolver concurrency and HPX memory headroom prevent the observed cgroup OOM under production stress.
- [api] Tokyo Night Drift, Sunday Morning Jazz, and Night Work each serve phone-equivalent remote ranges in repeated ordered runs after stress.

<!-- hosaka:domains api,infra -->
