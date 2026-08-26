#!/bin/sh
set -eu

provider_dir=/opt/bgutil-provider/server
provider_port=4416
provider_home=/tmp/bkgrnd-provider-home
provider_deno_dir=/opt/bgutil-provider/deno-cache
provider_token_ttl=${TOKEN_TTL:-6}

mkdir -p "$provider_home" "$provider_deno_dir"

(
  cd "$provider_dir/node_modules"
  exec env -i \
    PATH=/usr/local/bin:/usr/bin:/bin \
    HOME="$provider_home" \
    XDG_CACHE_HOME="$provider_home/.cache" \
    DENO_DIR="$provider_deno_dir" \
    TOKEN_TTL="$provider_token_ttl" \
    deno run \
    --allow-env \
    --allow-net \
    --allow-ffi=. \
    --allow-read=. \
    ../src/main.ts \
    --port "$provider_port"
) &
provider_pid=$!

attempt=0
while [ "$attempt" -lt 30 ]; do
  if curl -fsS "http://127.0.0.1:${provider_port}/ping" >/dev/null; then
    exec bkgrnd_server "$@"
  fi
  if ! kill -0 "$provider_pid" 2>/dev/null; then
    echo "POT provider exited before becoming ready" >&2
    exit 1
  fi
  attempt=$((attempt + 1))
  sleep 1
done

echo "POT provider did not become ready on 127.0.0.1:${provider_port}" >&2
kill "$provider_pid" 2>/dev/null || true
exit 1
