#!/bin/sh
set -eu

provider_dir=${WOPR_POT_PROVIDER_DIR:-/opt/bgutil-provider/server}
provider_port=${WOPR_POT_PROVIDER_PORT:-4416}
provider_home=${WOPR_POT_PROVIDER_HOME:-/var/lib/bkgrnd/provider-home}
provider_deno_dir=${WOPR_POT_PROVIDER_DENO_DIR:-/var/cache/bkgrnd/deno}
provider_runtime_path=${WOPR_POT_PROVIDER_PATH:-/usr/local/bin:/usr/bin:/bin}
provider_token_ttl=${TOKEN_TTL:-6}
provider_base_url=${WOPR_POT_PROVIDER_URL:-http://127.0.0.1:${provider_port}}
provider_ping_url=${provider_base_url%/}/ping
provider_restart_delay=${WOPR_POT_PROVIDER_RESTART_DELAY:-2}

provider_supervisor_pid=
server_pid=

mkdir -p "$provider_home" "$provider_deno_dir"

run_provider() {
  cd "$provider_dir/node_modules"
  exec env -i \
    PATH="$provider_runtime_path" \
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
}

supervise_provider() {
  provider_child_pid=
  provider_stopping=0

  # Invoked indirectly by the signal trap below.
  # shellcheck disable=SC2329
  stop_provider() {
    provider_stopping=1
    if [ -n "$provider_child_pid" ] && kill -0 "$provider_child_pid" 2>/dev/null; then
      kill -TERM "$provider_child_pid" 2>/dev/null || true
    fi
  }

  trap 'stop_provider' INT TERM HUP

  while [ "$provider_stopping" -eq 0 ]; do
    run_provider &
    provider_child_pid=$!
    provider_status=0
    wait "$provider_child_pid" || provider_status=$?
    provider_child_pid=

    if [ "$provider_stopping" -ne 0 ]; then
      break
    fi

    echo "POT provider exited with status ${provider_status}; restarting POT provider" >&2
    sleep "$provider_restart_delay" || true
  done
}

stop_children() {
  if [ -n "$server_pid" ] && kill -0 "$server_pid" 2>/dev/null; then
    kill -TERM "$server_pid" 2>/dev/null || true
  fi
  if [ -n "$provider_supervisor_pid" ] && kill -0 "$provider_supervisor_pid" 2>/dev/null; then
    kill -TERM "$provider_supervisor_pid" 2>/dev/null || true
  fi
  if [ -n "$server_pid" ]; then
    wait "$server_pid" 2>/dev/null || true
  fi
  if [ -n "$provider_supervisor_pid" ]; then
    wait "$provider_supervisor_pid" 2>/dev/null || true
  fi
}

# Invoked indirectly by the PID 1 signal trap below.
# shellcheck disable=SC2329
handle_shutdown() {
  trap - INT TERM HUP
  stop_children
  exit 143
}

trap 'handle_shutdown' INT TERM HUP

supervise_provider &
provider_supervisor_pid=$!

attempt=0
provider_ready=0
while [ "$attempt" -lt 30 ]; do
  if curl -fsS "$provider_ping_url" >/dev/null; then
    provider_ready=1
    break
  fi
  if ! kill -0 "$provider_supervisor_pid" 2>/dev/null; then
    echo "POT provider supervisor exited before the provider became ready" >&2
    stop_children
    exit 1
  fi
  attempt=$((attempt + 1))
  sleep 1
done

if [ "$provider_ready" -ne 1 ]; then
  echo "POT provider did not become ready at ${provider_ping_url}" >&2
  stop_children
  exit 1
fi

bkgrnd_server "$@" &
server_pid=$!
server_status=0
wait "$server_pid" || server_status=$?
server_pid=

stop_children
exit "$server_status"
