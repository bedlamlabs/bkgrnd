#!/usr/bin/env bash
set -euo pipefail

mode="${1:---all}"
case "$mode" in
  --all|--ranges|--resolver) ;;
  *)
    echo "usage: $0 [--all|--ranges|--resolver]" >&2
    exit 2
    ;;
esac

config_file="${BKGRND_CONFIG_FILE:-$HOME/.bkgrnd/config.yaml}"
base_url="${BKGRND_BASE_URL:-}"
auth_token="${BKGRND_WOPR_TOKEN:-}"

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

for dependency in curl jq; do
  if ! command -v "$dependency" >/dev/null 2>&1; then
    echo "$dependency is required" >&2
    exit 2
  fi
done

work_dir="$(mktemp -d "${TMPDIR:-/tmp}/bkgrnd-hpx-verify.XXXXXX")"
trap 'rm -rf "$work_dir"' EXIT

primary_video_id="${BKGRND_VERIFY_PRIMARY_VIDEO_ID:-QhMs-t7EhXY}"
saved_video_ids="${BKGRND_VERIFY_VIDEO_IDS:-QhMs-t7EhXY CUg_GK6TvGE Dg0IjOzopYU FTcE_EuUjM8 J9aEIYkiOtQ}"
live_video_id="${BKGRND_VERIFY_LIVE_VIDEO_ID:-Lcdi9O2XB4E}"
chunk_size=1048576

request_stream_range() {
  local video_id="$1"
  local range="$2"
  local output_file="$3"
  local header_file="$4"
  local source_url="https://www.youtube.com/watch?v=${video_id}"

  curl --silent --show-error \
    --connect-timeout 15 --max-time 120 \
    --output "$output_file" --dump-header "$header_file" \
    --write-out '%{http_code}' \
    --header "Authorization: Bearer ${auth_token}" \
    --header "Range: ${range}" \
    --get "${base_url}/api/v1/stream" \
    --data-urlencode "url=${source_url}" \
    --data-urlencode "proxy=true"
}

verify_exact_range() {
  local video_id="$1"
  local start="$2"
  local end="$3"
  local label="$4"
  local body_file="$work_dir/${label}.body"
  local header_file="$work_dir/${label}.headers"
  local status bytes content_range

  status="$(request_stream_range "$video_id" "bytes=${start}-${end}" "$body_file" "$header_file")"
  bytes="$(wc -c < "$body_file" | tr -d ' ')"
  content_range="$(awk 'BEGIN { IGNORECASE=1 } /^content-range:/ { sub(/\r$/, ""); print $2; exit }' "$header_file")"

  if [[ "$status" != "206" ]]; then
    echo "FAIL: ${video_id} ${start}-${end} returned HTTP ${status}" >&2
    return 1
  fi
  if [[ "$bytes" -ne $((end - start + 1)) ]]; then
    echo "FAIL: ${video_id} ${start}-${end} returned ${bytes} bytes" >&2
    return 1
  fi
  if [[ "$content_range" != "bytes" ]]; then
    echo "FAIL: ${video_id} ${start}-${end} omitted Content-Range" >&2
    return 1
  fi
  if ! grep -Eqi "^content-range:[[:space:]]*bytes[[:space:]]+${start}-${end}/[0-9]+" "$header_file"; then
    echo "FAIL: ${video_id} ${start}-${end} returned the wrong Content-Range" >&2
    return 1
  fi
}

verify_open_ended_range() {
  local video_id="$1"
  local label="$2"
  local body_file="$work_dir/${label}.body"
  local header_file="$work_dir/${label}.headers"
  local status bytes

  status="$(request_stream_range "$video_id" "bytes=0-" "$body_file" "$header_file")"
  bytes="$(wc -c < "$body_file" | tr -d ' ')"
  if [[ "$status" != "206" || "$bytes" -le 0 || "$bytes" -gt "$chunk_size" ]]; then
    echo "FAIL: ${video_id} open-ended range returned HTTP ${status}, ${bytes} bytes" >&2
    return 1
  fi
  if ! grep -Eqi '^content-range:[[:space:]]*bytes[[:space:]]+0-[0-9]+/[0-9]+' "$header_file"; then
    echo "FAIL: ${video_id} open-ended range omitted a valid Content-Range" >&2
    return 1
  fi
}

verify_ranges() {
  local video_id index start end

  # Exercise the open-ended range shape used when iOS starts remote playback.
  verify_open_ended_range "$primary_video_id" "primary-open"

  # Four contiguous MiB prove that the direct HPX stream survives well beyond
  # the historical ~40-second cutoff without reusing a synthetic fixture.
  for index in 0 1 2 3; do
    start=$((index * chunk_size))
    end=$((start + chunk_size - 1))
    verify_exact_range "$primary_video_id" "$start" "$end" "primary-${index}"
  done

  # Recheck every ordinary saved stream that reproduced the production failure.
  index=0
  for video_id in $saved_video_ids; do
    verify_exact_range "$video_id" 0 $((chunk_size - 1)) "saved-${index}"
    index=$((index + 1))
  done

  # Preserve the existing live-HLS behavior as a control.
  local live_body="$work_dir/live.body"
  local live_status
  live_status="$(curl --silent --show-error \
    --connect-timeout 15 --max-time 120 \
    --output "$live_body" --write-out '%{http_code}' \
    --header "Authorization: Bearer ${auth_token}" \
    --get "${base_url}/api/v1/stream" \
    --data-urlencode "url=https://www.youtube.com/watch?v=${live_video_id}" \
    --data-urlencode "proxy=true")"
  if [[ "$live_status" != "200" ]] || ! grep -q '^#EXTM3U' "$live_body"; then
    echo "FAIL: live HLS control returned HTTP ${live_status} without an M3U8 manifest" >&2
    return 1
  fi

  echo "PASS: HPX served open-ended and contiguous remote ranges for the real saved streams; live HLS remained playable"
}

verify_resolver() {
  local nonce source_url response_file status source
  nonce="$(date +%s)-$$"
  source_url="https://www.youtube.com/watch?v=${primary_video_id}&bkgrnd_verify=${nonce}"
  response_file="$work_dir/resolve.json"
  status="$(curl --silent --show-error \
    --connect-timeout 15 --max-time 180 \
    --output "$response_file" --write-out '%{http_code}' \
    --header "Authorization: Bearer ${auth_token}" \
    --get "${base_url}/api/v1/resolve" \
    --data-urlencode "url=${source_url}")"

  if [[ "$status" != "200" ]]; then
    echo "FAIL: HPX resolver returned HTTP ${status}" >&2
    return 1
  fi
  source="$(jq -r '.source // empty' "$response_file")"
  case "$source" in
    pot-provider|web-embedded|legacy-android-vr|legacy-default) ;;
    *)
      echo "FAIL: HPX resolver reported unexpected source '${source:-missing}'" >&2
      return 1
      ;;
  esac
  if ! jq -e '.streamUrl | type == "string" and startswith("https://")' "$response_file" >/dev/null; then
    echo "FAIL: HPX resolver did not return a validated HTTPS media URL" >&2
    return 1
  fi

  echo "PASS: HPX resolved a freshly probed real stream via validated ${source} fallback chain"
}

case "$mode" in
  --ranges) verify_ranges ;;
  --resolver) verify_resolver ;;
  --all)
    verify_resolver
    verify_ranges
    ;;
esac
