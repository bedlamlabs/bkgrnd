#!/usr/bin/env bash
set -euo pipefail

entrypoint="server/container-entrypoint.sh"
server_source="server/src/main.rs"
dockerfile="Dockerfile"

bash -c 'test "$1" = "AT-3" && test "$2" = "/api/v1/health"' _ "AT-3" "/api/v1/health"
grep -q 'restarting POT provider' "$entrypoint"
grep -q 'provider_health' "$server_source"
grep -q 'SERVICE_UNAVAILABLE' "$server_source"
grep -Eq 'YTDLP_MAX_CONCURRENCY:[[:space:]]*usize[[:space:]]*=[[:space:]]*[12];' "$server_source"
grep -q 'WOPR_POT_PROVIDER_URL=http://127.0.0.1:4416' "$dockerfile"
