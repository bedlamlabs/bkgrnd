#!/usr/bin/env bash
set -euo pipefail

config_file="${BKGRND_CONFIG_FILE:-$HOME/.bkgrnd/config.yaml}"
base_url="${BKGRND_BASE_URL:-}"
auth_token="${BKGRND_WOPR_TOKEN:-}"
hpx_host="${HPX_HOST:-hpx-remote}"
container_name="${HPX_BKGRND_CONTAINER:-bkgrnd-hpx}"
ssh_bin="${SSH_BIN:-/usr/bin/ssh}"
minimum_memory_bytes="${BKGRND_MIN_MEMORY_BYTES:-1073741824}"
sequence_rounds="${BKGRND_SEQUENCE_ROUNDS:-2}"
stress_requests="${BKGRND_STRESS_REQUESTS:-3}"
chunk_size=1048576

if [[ -z "$base_url" && -r "$config_file" ]]; then
  base_url="$(awk '$1 == "woprBaseUrl:" { print $2; exit }' "$config_file")"
fi
if [[ -z "$auth_token" && -r "$config_file" ]]; then
  auth_token="$(awk '$1 == "woprToken:" { print $2; exit }' "$config_file")"
fi
base_url="${base_url:-https://bkgrnd.bedl.am}"
base_url="${base_url%/}"

if [[ -z "$auth_token" ]]; then
  echo "BKGRND_WOPR_TOKEN is required (or configure woprToken in $config_file)" >&2
  exit 2
fi

for dependency in curl jq "$ssh_bin"; do
  if [[ "$dependency" == */* ]]; then
    if [[ ! -x "$dependency" ]]; then
      echo "$dependency is required" >&2
      exit 2
    fi
  elif ! command -v "$dependency" >/dev/null 2>&1; then
    echo "$dependency is required" >&2
    exit 2
  fi
done

for integer in "$minimum_memory_bytes" "$sequence_rounds" "$stress_requests"; do
  if [[ ! "$integer" =~ ^[1-9][0-9]*$ ]]; then
    echo "verification counts and memory threshold must be positive integers" >&2
    exit 2
  fi
done

work_dir="$(mktemp -d "${TMPDIR:-/tmp}/bkgrnd-hpx-runtime.XXXXXX")"
trap 'rm -rf "$work_dir"' EXIT

case "$auth_token" in
  *$'\n'*|*$'\r'*)
    echo "BKGRND_WOPR_TOKEN must not contain a newline" >&2
    exit 2
    ;;
esac
auth_token_escaped="${auth_token//\\/\\\\}"
auth_token_escaped="${auth_token_escaped//\"/\\\"}"
auth_config_file="$work_dir/auth.curlrc"
(umask 077 && printf 'header = "Authorization: Bearer %s"\n' "$auth_token_escaped" > "$auth_config_file")
unset auth_token auth_token_escaped

remote_runtime() {
  local action="$1"
  "$ssh_bin" "$hpx_host" bash -s -- "$container_name" "$action" <<'REMOTE'
set -euo pipefail
container_name="$1"
action="$2"

provider_process() {
  mode="$1"
  docker exec -i "$container_name" python3 - "$mode" <<'PROVIDER_SCAN'
import os
import signal
import sys

mode = sys.argv[1]
if mode not in {"identify", "terminate"}:
    raise SystemExit("unsupported provider process action")

scanner_pid = os.getpid()
matches = []
for proc_entry in os.scandir("/proc"):
    if not proc_entry.name.isdecimal():
        continue
    candidate_pid = int(proc_entry.name)
    if candidate_pid == scanner_pid:
        continue
    proc_dir = proc_entry.path
    try:
        if os.path.basename(os.readlink(f"{proc_dir}/exe")) != "deno":
            continue
        with open(f"{proc_dir}/cmdline", "rb") as cmdline_file:
            argv = cmdline_file.read().split(b"\0")
    except (FileNotFoundError, PermissionError, ProcessLookupError):
        continue
    has_port = any(
        argv[index : index + 2] == [b"--port", b"4416"]
        for index in range(len(argv) - 1)
    )
    if b"run" in argv and b"../src/main.ts" in argv and has_port:
        matches.append(candidate_pid)

if len(matches) != 1:
    raise SystemExit(f"expected exactly one POT provider, found {len(matches)}")

provider_pid = matches[0]
if mode == "terminate":
    os.kill(provider_pid, signal.SIGTERM)
print(provider_pid)
PROVIDER_SCAN
}

provider_pid() {
  provider_process identify
}

cgroup_metric() {
  metric="$1"
  container_pid=$(docker inspect "$container_name" --format '{{.State.Pid}}')
  cgroup_path=$(awk -F: '$1 == "0" { print $3; exit }' "/proc/${container_pid}/cgroup")
  cat "/sys/fs/cgroup${cgroup_path}/${metric}"
}

case "$action" in
  snapshot)
    docker inspect "$container_name" --format '{{.Id}}|{{.State.StartedAt}}|{{.RestartCount}}|{{.State.OOMKilled}}|{{.State.Status}}|{{if .State.Health}}{{.State.Health.Status}}{{else}}missing{{end}}|{{.HostConfig.Memory}}'
    ;;
  provider-pid)
    provider_pid
    ;;
  provider-ping)
    docker exec "$container_name" curl -fsS http://127.0.0.1:4416/ping >/dev/null
    ;;
  terminate-provider)
    old_pid=$(provider_process terminate)
    case "$old_pid" in
      ''|*[!0-9]*) echo "could not identify provider PID" >&2; exit 1 ;;
    esac
    provider_went_down=0
    for _attempt in $(seq 1 40); do
      if ! docker exec "$container_name" curl -fsS http://127.0.0.1:4416/ping >/dev/null 2>&1; then
        provider_went_down=1
        break
      fi
      sleep 0.05
    done
    if [ "$provider_went_down" -ne 1 ]; then
      echo "provider did not stop after targeted termination" >&2
      exit 1
    fi
    health_status=000
    for _attempt in $(seq 1 30); do
      health_status=$(docker exec "$container_name" curl -sS -o /dev/null -w '%{http_code}' http://127.0.0.1:808/api/v1/health)
      [ "$health_status" = "503" ] && break
      sleep 0.05
    done
    printf '%s|%s\n' "$old_pid" "$health_status"
    ;;
  memory-peak)
    cgroup_metric memory.peak
    ;;
  oom-kills)
    cgroup_metric memory.events | awk '$1 == "oom_kill" { print $2; found=1 } END { if (!found) print 0 }'
    ;;
  *)
    echo "unsupported runtime inspection action" >&2
    exit 2
    ;;
esac
REMOTE
}

wait_for_public_health() {
  local expected="$1"
  local attempts="${2:-60}"
  local status="000"
  for ((attempt = 1; attempt <= attempts; attempt += 1)); do
    status="$(curl --silent --show-error --connect-timeout 5 --max-time 8 \
      --output /dev/null --write-out '%{http_code}' "${base_url}/api/v1/health" || true)"
    if [[ "$status" == "$expected" ]]; then
      return 0
    fi
    sleep 1
  done
  echo "FAIL: public health remained HTTP ${status}; expected ${expected}" >&2
  return 1
}

verify_provider_restart() {
  local termination old_pid unavailable_status new_pid
  termination="$(remote_runtime terminate-provider)"
  IFS='|' read -r old_pid unavailable_status <<<"$termination"
  if [[ ! "$old_pid" =~ ^[0-9]+$ || "$unavailable_status" != "503" ]]; then
    echo "FAIL: provider termination did not make dependency-aware health return 503" >&2
    return 1
  fi

  new_pid=""
  for ((attempt = 1; attempt <= 60; attempt += 1)); do
    new_pid="$(remote_runtime provider-pid 2>/dev/null || true)"
    if [[ "$new_pid" =~ ^[0-9]+$ && "$new_pid" != "$old_pid" ]] && remote_runtime provider-ping >/dev/null 2>&1; then
      break
    fi
    new_pid=""
    sleep 1
  done
  if [[ -z "$new_pid" ]]; then
    echo "FAIL: POT provider did not restart within 60 seconds" >&2
    return 1
  fi
  wait_for_public_health 200
  echo "PASS: provider process restarted and dependency-aware health recovered"
}

source_url() {
  local video_id="$1"
  printf 'https://www.youtube.com/watch?v=%s' "$video_id"
}

request_range() {
  local video_id="$1"
  local range="$2"
  local body_file="$3"
  local header_file="$4"
  curl --silent --show-error --connect-timeout 15 --max-time 180 \
    --config "$auth_config_file" \
    --output "$body_file" --dump-header "$header_file" --write-out '%{http_code}' \
    --header "Range: ${range}" \
    --get "${base_url}/api/v1/stream" \
    --data-urlencode "url=$(source_url "$video_id")" \
    --data-urlencode "proxy=true"
}

verify_progressive_range() {
  local video_id="$1"
  local range="$2"
  local expected_start="$3"
  local expected_end="$4"
  local label="$5"
  local body_file="$work_dir/${label}.body"
  local header_file="$work_dir/${label}.headers"
  local status bytes expected_bytes
  status="$(request_range "$video_id" "$range" "$body_file" "$header_file")"
  bytes="$(wc -c < "$body_file" | tr -d ' ')"
  if [[ "$status" != "206" || "$bytes" -le 0 || "$bytes" -gt "$chunk_size" ]]; then
    echo "FAIL: ${label} returned HTTP ${status}, ${bytes} bytes" >&2
    return 1
  fi
  if [[ -n "$expected_end" ]]; then
    expected_bytes=$((expected_end - expected_start + 1))
    if [[ "$bytes" -ne "$expected_bytes" ]]; then
      echo "FAIL: ${label} returned ${bytes} bytes; expected ${expected_bytes}" >&2
      return 1
    fi
  fi
  if [[ -n "$expected_end" ]] && ! grep -Eqi "^content-range:[[:space:]]*bytes[[:space:]]+${expected_start}-${expected_end}/[0-9]+" "$header_file"; then
    echo "FAIL: ${label} returned the wrong explicit Content-Range" >&2
    return 1
  fi
  if [[ -z "$expected_end" ]] && ! grep -Eqi "^content-range:[[:space:]]*bytes[[:space:]]+${expected_start}-[0-9]+/[0-9]+" "$header_file"; then
    echo "FAIL: ${label} returned an invalid Content-Range" >&2
    return 1
  fi
}

verify_progressive_stream() {
  local video_id="$1"
  local label="$2"
  verify_progressive_range "$video_id" 'bytes=0-' 0 '' "${label}-open"
  verify_progressive_range "$video_id" "bytes=${chunk_size}-$((2 * chunk_size - 1))" "$chunk_size" "$((2 * chunk_size - 1))" "${label}-resume-1"
  verify_progressive_range "$video_id" "bytes=$((2 * chunk_size))-$((3 * chunk_size - 1))" "$((2 * chunk_size))" "$((3 * chunk_size - 1))" "${label}-resume-2"
}

verify_live_stream() {
  local video_id="$1"
  local label="$2"
  local manifest_file="$work_dir/${label}.m3u8"
  local segment_file="$work_dir/${label}.segment"
  local status media_uri media_url media_status media_bytes
  status="$(curl --silent --show-error --connect-timeout 15 --max-time 180 \
    --config "$auth_config_file" \
    --output "$manifest_file" --write-out '%{http_code}' \
    --get "${base_url}/api/v1/stream" \
    --data-urlencode "url=$(source_url "$video_id")" \
    --data-urlencode "proxy=true")"
  if [[ "$status" != "200" ]] || ! grep -q '^#EXTM3U' "$manifest_file"; then
    echo "FAIL: ${label} did not return an HLS manifest (HTTP ${status})" >&2
    return 1
  fi
  media_uri="$(awk 'NF && $0 !~ /^#/ { sub(/\r$/, ""); print; exit }' "$manifest_file")"
  if [[ -z "$media_uri" ]]; then
    echo "FAIL: ${label} HLS manifest did not contain a media URI" >&2
    return 1
  fi
  case "$media_uri" in
    /api/v1/stream-segment\?*) media_url="${base_url}${media_uri}" ;;
    *)
      echo "FAIL: ${label} HLS manifest contained a non-relay media URI" >&2
      return 1
      ;;
  esac
  media_status="$(curl --silent --show-error --connect-timeout 15 --max-time 180 \
    --config "$auth_config_file" \
    --output "$segment_file" --write-out '%{http_code}' \
    "$media_url")"
  media_bytes="$(wc -c < "$segment_file" | tr -d ' ')"
  if [[ "$media_status" != "200" && "$media_status" != "206" ]] || [[ "$media_bytes" -le 0 ]]; then
    echo "FAIL: ${label} HLS media fetch returned HTTP ${media_status}, ${media_bytes} bytes" >&2
    return 1
  fi
}

run_resolver_stress() {
  local -a video_ids=(Lcdi9O2XB4E VHpLQYtjikQ FTcE_EuUjM8)
  local -a pids=()
  local request_index video_id nonce response_file status_file status
  nonce="$(date +%s)-$$"

  for ((request_index = 0; request_index < stress_requests; request_index += 1)); do
    video_id="${video_ids[$((request_index % ${#video_ids[@]}))]}"
    response_file="$work_dir/stress-${request_index}.json"
    status_file="$work_dir/stress-${request_index}.status"
    (
      curl --silent --show-error --connect-timeout 15 --max-time 300 \
        --config "$auth_config_file" \
        --output "$response_file" --write-out '%{http_code}' \
        --get "${base_url}/api/v1/resolve" \
        --data-urlencode "url=$(source_url "$video_id")&bkgrnd_stress=${nonce}-${request_index}" \
        >"$status_file"
    ) &
    pids+=("$!")
  done

  for request_index in "${!pids[@]}"; do
    if ! wait "${pids[$request_index]}"; then
      echo "FAIL: resolver stress request ${request_index} did not complete" >&2
      return 1
    fi
    status="$(<"$work_dir/stress-${request_index}.status")"
    if [[ "$status" != "200" ]] || ! jq -e '.streamUrl | type == "string" and startswith("https://")' "$work_dir/stress-${request_index}.json" >/dev/null; then
      echo "FAIL: resolver stress request ${request_index} returned HTTP ${status}" >&2
      return 1
    fi
  done

  remote_runtime provider-ping >/dev/null
  echo "PASS: ${stress_requests} concurrent fresh resolver requests completed with the provider alive"
}

verify_phone_sequence() {
  local round
  for ((round = 1; round <= sequence_rounds; round += 1)); do
    verify_live_stream Lcdi9O2XB4E "round-${round}-tokyo"
    verify_progressive_stream VHpLQYtjikQ "round-${round}-sunday"
    verify_progressive_stream FTcE_EuUjM8 "round-${round}-night-work"
  done
  echo "PASS: Tokyo -> Sunday Jazz -> Night Work completed ${sequence_rounds} ordered phone-equivalent rounds"
}

read_snapshot() {
  local snapshot="$1"
  IFS='|' read -r snapshot_id snapshot_started snapshot_restarts snapshot_oom snapshot_status snapshot_health snapshot_memory <<<"$snapshot"
}

baseline_snapshot="$(remote_runtime snapshot)"
read_snapshot "$baseline_snapshot"
baseline_id="$snapshot_id"
baseline_started="$snapshot_started"
baseline_restarts="$snapshot_restarts"
baseline_oom_kills="$(remote_runtime oom-kills)"

if [[ "$snapshot_status" != "running" || "$snapshot_health" != "healthy" || "$snapshot_oom" != "false" ]]; then
  echo "FAIL: bkgrnd container did not start in a healthy, non-OOM state" >&2
  exit 1
fi
if [[ ! "$snapshot_memory" =~ ^[0-9]+$ || "$snapshot_memory" -lt "$minimum_memory_bytes" ]]; then
  echo "FAIL: bkgrnd memory limit is ${snapshot_memory:-unknown} bytes; require at least ${minimum_memory_bytes}" >&2
  exit 1
fi
if [[ "$baseline_oom_kills" != "0" ]]; then
  echo "FAIL: bkgrnd cgroup already records ${baseline_oom_kills} OOM kill(s)" >&2
  exit 1
fi
remote_runtime provider-ping >/dev/null
wait_for_public_health 200

verify_provider_restart
run_resolver_stress
verify_phone_sequence

final_snapshot="$(remote_runtime snapshot)"
read_snapshot "$final_snapshot"
final_oom_kills="$(remote_runtime oom-kills)"
memory_peak="$(remote_runtime memory-peak)"

if [[ "$snapshot_id" != "$baseline_id" || "$snapshot_started" != "$baseline_started" || "$snapshot_restarts" != "$baseline_restarts" ]]; then
  echo "FAIL: bkgrnd container restarted during resilience verification" >&2
  exit 1
fi
if [[ "$snapshot_status" != "running" || "$snapshot_health" != "healthy" || "$snapshot_oom" != "false" ]]; then
  echo "FAIL: bkgrnd container ended unhealthy or OOM-marked" >&2
  exit 1
fi
if [[ "$final_oom_kills" != "$baseline_oom_kills" ]]; then
  echo "FAIL: cgroup OOM kills changed from ${baseline_oom_kills} to ${final_oom_kills}" >&2
  exit 1
fi
if [[ ! "$memory_peak" =~ ^[0-9]+$ || "$memory_peak" -ge $((snapshot_memory * 9 / 10)) ]]; then
  echo "FAIL: peak memory ${memory_peak:-unknown} bytes left less than 10% cgroup headroom" >&2
  exit 1
fi
remote_runtime provider-ping >/dev/null
wait_for_public_health 200

echo "PASS: HPX provider restart, truthful health, resolver stress, ordered phone playback, and memory headroom are verified"
