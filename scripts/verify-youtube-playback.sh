#!/usr/bin/env bash
set -euo pipefail

mode="soak"
soak_seconds=95
case "${1:-}" in
  --startup-only)
    mode="startup"
    ;;
  --soak-seconds)
    soak_seconds="${2:?--soak-seconds requires a value}"
    ;;
  "") ;;
  *)
    echo "Usage: $0 [--startup-only | --soak-seconds N]" >&2
    exit 2
    ;;
esac

config_path="${BKGRND_CONFIG_PATH:-$HOME/.bkgrnd/config.yaml}"
live_url="${BKGRND_LIVE_TEST_URL:-https://www.youtube.com/watch?v=Lcdi9O2XB4E}"
base_url="$(awk -F': *' '$1=="woprBaseUrl" {sub(/^[^:]*:[[:space:]]*/, ""); gsub(/^"|"$/, ""); print; exit}' "$config_path")"
token="$(awk -F': *' '$1=="woprToken" {sub(/^[^:]*:[[:space:]]*/, ""); gsub(/^"|"$/, ""); print; exit}' "$config_path")"
test -n "$base_url" -a -n "$token"
api="${base_url%/}/api/v1"
curl -fsS http://127.0.0.1:4416/ping | jq -e '.version == "1.3.1"' >/dev/null
curl -fsS -X POST http://127.0.0.1:4416/invalidate_caches >/dev/null
app_bin="/Applications/bkgrnd.app/Contents/MacOS/bkgrnd"
strategy_file="$HOME/.bkgrnd/last-resolver-strategy"
verification_log="$HOME/.bkgrnd/logs/playback-verification-app.log"
test -x "$app_bin"

# This verifier restarts the installed app several times. Never commandeer an
# active listening session unless the caller deliberately opts in.
if pgrep -f "$app_bin" >/dev/null; then
  current_state="$(curl -fsS --max-time 5 -H "Authorization: Bearer $token" "$api/local/status" || true)"
  currently_playing="$(jq -r '.status.isPlaying // false' <<<"$current_state" 2>/dev/null || echo false)"
  if [[ "$currently_playing" == true && "${BKGRND_ALLOW_DISRUPTIVE_PLAYBACK_TEST:-}" != 1 ]]; then
    echo "Refusing to restart bkgrnd during active playback. Set BKGRND_ALLOW_DISRUPTIVE_PLAYBACK_TEST=1 to opt in." >&2
    exit 2
  fi
fi

stop_app() {
  osascript -e 'tell application id "com.bedlamlabs.bkgrnd" to quit' >/dev/null 2>&1 || true
  for _ in {1..40}; do
    pgrep -f '/Applications/bkgrnd.app/Contents/MacOS/bkgrnd' >/dev/null || return 0
    sleep 0.25
  done
  running_pids="$(pgrep -f '/Applications/bkgrnd.app/Contents/MacOS/bkgrnd' || true)"
  [[ -z "$running_pids" ]] || kill -TERM $running_pids
  for _ in {1..40}; do
    pgrep -f '/Applications/bkgrnd.app/Contents/MacOS/bkgrnd' >/dev/null || return 0
    sleep 0.25
  done
  echo "Timed out waiting for the previous bkgrnd process to exit" >&2
  return 1
}

start_app() {
  local fail_strategies="${1:-}"
  local force_stall_strategy="${2:-}"
  stop_app
  env \
    BKGRND_VERIFY_FAIL_STRATEGIES="$fail_strategies" \
    BKGRND_VERIFY_FREEZE_STALL_STRATEGY="$force_stall_strategy" \
    BKGRND_VERIFY_MUTE_AUDIO=1 \
    "$app_bin" >>"$verification_log" 2>&1 &
  new_pid=$!
  for _ in {1..40}; do
    if kill -0 "$new_pid" 2>/dev/null \
      && curl -fsS --max-time 3 -H "Authorization: Bearer $token" "$api/local/status" >/dev/null; then
      sleep 1
      kill -0 "$new_pid" 2>/dev/null && return 0
    fi
    sleep 0.25
  done
  echo "New bkgrnd verification process failed to become ready (pid $new_pid)" >&2
  return 1
}

restore_normal_app() {
  trap - EXIT
  stop_app
  open -a /Applications/bkgrnd.app >/dev/null 2>&1 || true
}
trap restore_normal_app EXIT

read_marker() {
  marker_ms=0
  resolver_strategy=""
  marker_url=""
  if [[ -f "$strategy_file" ]]; then
    IFS=$'\t' read -r marker_ms resolver_strategy marker_url < "$strategy_file" || true
  fi
}

assert_pot_provider_used() {
  curl -fsS http://127.0.0.1:4416/minter_cache \
    | jq -e 'type == "array" and length > 0' >/dev/null
}

read_status() {
  local response
  for _ in {1..10}; do
    if response="$(curl -fsS --max-time 5 -H "Authorization: Bearer $token" "$api/local/status" 2>/dev/null)"; then
      printf '%s\n' "$response"
      return 0
    fi
    sleep 1
  done
  echo "Timed out reading bkgrnd control status" >&2
  return 1
}

submit_play_command() {
  local payload="$1"
  local http_code
  for _ in {1..15}; do
    http_code="$(curl -sS --max-time 5 -o /dev/null -w '%{http_code}' -X POST \
      -H "Authorization: Bearer $token" \
      -H 'content-type: application/json' \
      --data "$payload" \
      "$api/local/commands" || echo 000)"
    case "$http_code" in
      2*) return 0 ;;
      409) sleep 1 ;;
      *) echo "Playback command failed with HTTP $http_code" >&2; return 1 ;;
    esac
  done
  echo "Timed out waiting for the previous playback command to clear" >&2
  return 1
}

submit_and_wait_for_strategy() {
  local expected_strategy="$1"
  command_started_ms="$(perl -MTime::HiRes=time -e 'printf "%.0f\n", time * 1000')"
  payload="$(jq -n --arg url "$live_url" '{action:"play",url:$url,title:"bkgrnd live playback verification"}')"
  submit_play_command "$payload"

  start_position=""
  for _ in {1..30}; do
    player_state="$(read_status)"
    playing="$(jq -r '.status.isPlaying // false' <<<"$player_state")"
    source_url="$(jq -r '.status.sourceUrl // empty' <<<"$player_state")"
    start_position="$(jq -r '.status.position // empty' <<<"$player_state")"
    read_marker
    if [[ "$playing" == true && "$source_url" == "$live_url" && -n "$start_position" \
      && "$resolver_strategy" == "$expected_strategy" && "$marker_url" == "$live_url" \
      && "$marker_ms" =~ ^[0-9]+$ ]] && (( marker_ms >= command_started_ms )); then
      return 0
    fi
    sleep 2
  done
  echo "Timed out waiting for $expected_strategy playback" >&2
  return 1
}

if [[ "$mode" == "startup" ]]; then
  start_app "" ""
  submit_and_wait_for_strategy "pot-provider"
  assert_pot_provider_used
  start_app "pot-provider" ""
  submit_and_wait_for_strategy "web-embedded"
  start_app "pot-provider,web-embedded" ""
  submit_and_wait_for_strategy "legacy"
  echo "AT-1 PASS: installed app proved POT -> web_embedded -> legacy startup fallback"
  exit 0
fi

# First prove that the real stall detector can replace a failed POT session.
start_app "" "pot-provider"
submit_and_wait_for_strategy "pot-provider"
assert_pot_provider_used
initial_position="$start_position"
initial_marker_ms="$marker_ms"

for _ in {1..20}; do
  read_marker
  if [[ "$resolver_strategy" == "web-embedded" && "$marker_url" == "$live_url" \
    && "$marker_ms" =~ ^[0-9]+$ ]] && (( marker_ms > initial_marker_ms )); then
    break
  fi
  sleep 2
done
test "$resolver_strategy" = "web-embedded"
test "$marker_url" = "$live_url"
recovery_position=""
for _ in {1..15}; do
  recovery_state="$(read_status)"
  recovery_playing="$(jq -r '.status.isPlaying // false' <<<"$recovery_state")"
  recovery_source_url="$(jq -r '.status.sourceUrl // empty' <<<"$recovery_state")"
  recovery_position="$(jq -r '.status.position // empty' <<<"$recovery_state")"
  if [[ "$recovery_playing" == true && "$recovery_source_url" == "$live_url" \
    && -n "$recovery_position" ]]; then
    break
  fi
  sleep 1
done
test -n "$recovery_position"

# Playback position restarts when recovery replaces mpv. Accumulate progress
# within each session instead of treating a legitimate strategy switch as a
# regression, while requiring the app to remain playing for the full soak.
sample_seconds=5
sample_count=$((soak_seconds / sample_seconds))
accumulated_progress=0
last_position="$recovery_position"
last_marker_ms="$marker_ms"
strategies_seen="web-embedded"
for ((sample = 0; sample < sample_count; sample++)); do
  sleep "$sample_seconds"
  sample_state="$(read_status)"
  sample_playing="$(jq -r '.status.isPlaying // false' <<<"$sample_state")"
  sample_position="$(jq -r '.status.position // empty' <<<"$sample_state")"
  test "$sample_playing" = true
  test -n "$sample_position"
  read_marker
  test "$marker_url" = "$live_url"

  if [[ "$marker_ms" != "$last_marker_ms" ]]; then
    strategies_seen="$strategies_seen,$resolver_strategy"
    last_marker_ms="$marker_ms"
    last_position="$sample_position"
    continue
  fi

  delta="$(awk -v previous="$last_position" -v current="$sample_position" 'BEGIN { d=current-previous; print (d > 0 ? d : 0) }')"
  accumulated_progress="$(awk -v total="$accumulated_progress" -v delta="$delta" 'BEGIN { print total+delta }')"
  last_position="$sample_position"
done

minimum_progress=$((soak_seconds - 35))
awk -v actual="$accumulated_progress" -v minimum="$minimum_progress" 'BEGIN { exit !(actual >= minimum) }'

# Then remove every failure/stall injection and prove the real live stream keeps
# advancing for the full production soak. This catches genuine segment expiry
# or resolver regressions that the deterministic detector exercise cannot.
start_app "" ""
submit_and_wait_for_strategy "pot-provider"
real_start_position="$start_position"
sleep "$soak_seconds"
real_state="$(read_status)"
real_playing="$(jq -r '.status.isPlaying // false' <<<"$real_state")"
real_source_url="$(jq -r '.status.sourceUrl // empty' <<<"$real_state")"
real_end_position="$(jq -r '.status.position // empty' <<<"$real_state")"
test "$real_playing" = true
test "$real_source_url" = "$live_url"
test -n "$real_end_position"
real_progress="$(awk -v start="$real_start_position" -v end="$real_end_position" 'BEGIN { print end-start }')"
real_minimum_progress=$((soak_seconds - 20))
awk -v actual="$real_progress" -v minimum="$real_minimum_progress" 'BEGIN { exit !(actual >= minimum) }'

echo "AT-2 PASS: installed app recovered a forced POT stall through $strategies_seen, then an unforced real live stream advanced ${real_progress}s during the ${soak_seconds}s soak"
