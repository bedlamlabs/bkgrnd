#!/usr/bin/env bash
set -euo pipefail

mode="matrix"
soak_seconds=95
case "${1:-}" in
  --startup) mode="startup" ;;
  --soak-seconds)
    mode="soak"
    soak_seconds="${2:?--soak-seconds requires a value}"
    ;;
  --matrix|"") mode="matrix" ;;
  *) echo "Usage: $0 [--startup | --soak-seconds N | --matrix]" >&2; exit 2 ;;
esac

config_path="${BKGRND_CONFIG_PATH:-$HOME/.bkgrnd/config.yaml}"
history_path="${BKGRND_HISTORY_PATH:-$HOME/.bkgrnd/history.json}"
app_bin="/Applications/bkgrnd.app/Contents/MacOS/bkgrnd"
strategy_file="$HOME/.bkgrnd/last-resolver-strategy"
verification_log="$HOME/.bkgrnd/logs/remote-playback-verification-app.log"
excluded_verifier_id="${BKGRND_EXCLUDED_VERIFIER_ID:-Lcdi9O2XB4E}"

test -x "$app_bin"
test -s "$config_path"
test -s "$history_path"

base_url="$(awk -F': *' '$1=="woprBaseUrl" {sub(/^[^:]*:[[:space:]]*/, ""); gsub(/^"|"$/, ""); print; exit}' "$config_path")"
token="$(awk -F': *' '$1=="woprToken" {sub(/^[^:]*:[[:space:]]*/, ""); gsub(/^"|"$/, ""); print; exit}' "$config_path")"
test -n "$base_url" -a -n "$token"
api="${base_url%/}/api/v1"

plain_url="${BKGRND_MATRIX_PLAIN_URL:-$(jq -r --arg excluded "$excluded_verifier_id" '[.[] | select(.type == "video" and (.url | contains($excluded) | not))][0].url // empty' "$history_path")}"
mix_url="${BKGRND_MATRIX_MIX_URL:-$(jq -r --arg excluded "$excluded_verifier_id" '[.[] | select(.type == "playlist" and (.url | contains($excluded) | not) and (.url | test("[?&]list=RD[A-Za-z0-9_-]*")) and (.title | test("24/7|live stream"; "i") | not))][0].url // empty' "$history_path")}"
live_history_url="${BKGRND_MATRIX_LIVE_URL:-$(jq -r --arg excluded "$excluded_verifier_id" '[.[] | select((.url | contains($excluded) | not) and (.title | test("24/7|live stream"; "i")))][0].url // empty' "$history_path")}"

video_id() {
  local url="$1"
  if [[ "$url" =~ [\?\&]v=([A-Za-z0-9_-]{11}) ]]; then
    printf '%s' "${BASH_REMATCH[1]}"
  elif [[ "$url" =~ youtu\.be/([A-Za-z0-9_-]{11}) ]]; then
    printf '%s' "${BASH_REMATCH[1]}"
  fi
}

plain_id="$(video_id "$plain_url")"
mix_id="$(video_id "$mix_url")"
live_id="$(video_id "$live_history_url")"
live_url="https://www.youtube.com/watch?v=$live_id"

for selected in "$plain_url" "$mix_url" "$live_url"; do
  test -n "$selected"
  [[ "$selected" != *"$excluded_verifier_id"* ]]
done
[[ "$mix_url" =~ [\?\&]list=RD[A-Za-z0-9_-]* ]]
test "$plain_id" != "$mix_id"
test "$plain_id" != "$live_id"
test "$mix_id" != "$live_id"

read_status() {
  local response
  for _ in {1..8}; do
    if response="$(curl -fsS --max-time 5 -H "Authorization: Bearer $token" "$api/local/status" 2>/dev/null)"; then
      printf '%s' "$response"
      return 0
    fi
    sleep 0.5
  done
  echo "Timed out reading production remote playback status" >&2
  return 1
}

if pgrep -f "$app_bin" >/dev/null; then
  current_state="$(read_status || true)"
  currently_playing="$(jq -r '.status.isPlaying // false' <<<"$current_state" 2>/dev/null || echo false)"
  if [[ "$currently_playing" == true && "${BKGRND_ALLOW_DISRUPTIVE_PLAYBACK_TEST:-}" != 1 ]]; then
    echo "Refusing to restart bkgrnd during active playback. Set BKGRND_ALLOW_DISRUPTIVE_PLAYBACK_TEST=1 to opt in." >&2
    exit 2
  fi
fi

stop_app() {
  osascript -e 'tell application id "com.bedlamlabs.bkgrnd" to quit' >/dev/null 2>&1 || true
  for _ in {1..40}; do
    pgrep -f "$app_bin" >/dev/null || return 0
    sleep 0.25
  done
  running_pids="$(pgrep -f "$app_bin" || true)"
  [[ -z "$running_pids" ]] || kill -TERM $running_pids
  for _ in {1..40}; do
    pgrep -f "$app_bin" >/dev/null || return 0
    sleep 0.25
  done
  echo "Timed out stopping the installed bkgrnd app" >&2
  return 1
}

restore_normal_app() {
  trap - EXIT
  stop_app || true
  open -a /Applications/bkgrnd.app >/dev/null 2>&1 || true
}
trap restore_normal_app EXIT

start_muted_app() {
  local unready_strategy="${1:-}"
  local frozen_strategy="${2:-}"
  stop_app
  env \
    BKGRND_VERIFY_MUTE_AUDIO=1 \
    BKGRND_VERIFY_UNREADY_START_STRATEGY="$unready_strategy" \
    BKGRND_VERIFY_FREEZE_STALL_STRATEGY="$frozen_strategy" \
    "$app_bin" >>"$verification_log" 2>&1 &
  test_pid=$!
  for _ in {1..80}; do
    kill -0 "$test_pid" 2>/dev/null || break
    curl -fsS --max-time 3 -H "Authorization: Bearer $token" "$api/local/status" >/dev/null 2>&1 && {
      sleep 1
      kill -0 "$test_pid" 2>/dev/null && return 0
    }
    sleep 0.25
  done
  echo "Muted diagnostic app failed to become ready" >&2
  return 1
}

send_remote_play() {
  local url="$1"
  local label="$2"
  local body
  body="$(jq -nc --arg url "$url" --arg title "$label" '{action:"play",url:$url,title:$title,thumbnail:""}')"
  for _ in {1..12}; do
    code="$(curl -sS --max-time 10 -o /dev/null -w '%{http_code}' -X POST \
      -H "Authorization: Bearer $token" -H 'Content-Type: application/json' \
      --data "$body" "$api/local/commands" || true)"
    [[ "$code" == 200 ]] && return 0
    [[ "$code" == 409 ]] || { echo "Remote play command failed with HTTP $code" >&2; return 1; }
    sleep 0.5
  done
  echo "Remote play command remained busy" >&2
  return 1
}

wait_for_progress() {
  local expected_id="$1"
  local state seen_id playing position
  for _ in {1..100}; do
    state="$(read_status)"
    seen_id="$(jq -r '.status.videoId // empty' <<<"$state")"
    playing="$(jq -r '.status.isPlaying // false' <<<"$state")"
    position="$(jq -r '.status.position // 0' <<<"$state")"
    if [[ "$seen_id" == "$expected_id" && "$playing" == true ]] \
      && awk -v p="$position" 'BEGIN { exit !(p > 0.25) }'; then
      printf '%s' "$position"
      return 0
    fi
    sleep 0.5
  done
  echo "Remote item $expected_id never produced advancing playback" >&2
  return 1
}

marker_strategy_after() {
  local minimum_ms="$1"
  shift
  local marker_ms marker_strategy
  for _ in {1..80}; do
    if [[ -s "$strategy_file" ]]; then
      marker_ms="$(awk -F'\t' 'NR==1 {print $1}' "$strategy_file")"
      marker_strategy="$(awk -F'\t' 'NR==1 {print $2}' "$strategy_file")"
      if [[ "$marker_ms" =~ ^[0-9]+$ ]] && (( marker_ms >= minimum_ms )); then
        for expected in "$@"; do
          [[ "$marker_strategy" == "$expected" ]] && return 0
        done
      fi
    fi
    sleep 0.5
  done
  echo "Expected a post-POT fallback resolver marker was not observed" >&2
  return 1
}

assert_advanced_by() {
  local start="$1"
  local end="$2"
  local minimum="$3"
  if ! awk -v start="$start" -v end="$end" -v minimum="$minimum" \
    'BEGIN { exit !((end - start) >= minimum) }'; then
    echo "Playback advanced from ${start}s to ${end}s; required delta was ${minimum}s" >&2
    return 1
  fi
}

run_startup() {
  start_muted_app "pot-provider" ""
  command_ms=$(( $(date +%s) * 1000 ))
  send_remote_play "$plain_url" "remote startup history probe"
  start_position="$(wait_for_progress "$plain_id")"
  marker_strategy_after "$command_ms" "web-embedded" "legacy"
  sleep 8
  end_position="$(wait_for_progress "$plain_id")"
  assert_advanced_by "$start_position" "$end_position" 3
  echo "AT-1 PASS: a real non-live remote item rejected an unready POT session, fell through, and advanced"
}

run_soak() {
  start_muted_app "" "pot-provider"
  command_ms=$(( $(date +%s) * 1000 ))
  send_remote_play "$plain_url" "remote non-live recovery probe"
  start_position="$(wait_for_progress "$plain_id")"
  marker_strategy_after "$command_ms" "web-embedded" "legacy"
  sleep "$soak_seconds"
  end_position="$(wait_for_progress "$plain_id")"
  assert_advanced_by "$start_position" "$end_position" "$((soak_seconds - 20))"
  echo "AT-2 PASS: a real non-live remote item recovered through the next usable resolver and advanced for ${soak_seconds}s"
}

run_matrix_item() {
  local label="$1"
  local url="$2"
  local expected_id="$3"
  send_remote_play "$url" "$label"
  local start_position end_position
  start_position="$(wait_for_progress "$expected_id")"
  sleep 50
  end_position="$(wait_for_progress "$expected_id")"
  assert_advanced_by "$start_position" "$end_position" 40
}

run_matrix() {
  start_muted_app "" ""
  run_matrix_item "plain history video" "$plain_url" "$plain_id"
  run_matrix_item "history Mix URL" "$mix_url" "$mix_id"
  run_matrix_item "history live stream" "$live_url" "$live_id"
  echo "AT-3 PASS: production remote playback advanced real plain, Mix, and live history items; excluded verifier id $excluded_verifier_id"
}

case "$mode" in
  startup) run_startup ;;
  soak) run_soak ;;
  matrix) run_matrix ;;
esac
