#!/usr/bin/env bash
set -euo pipefail

project_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)"
player="$project_dir/src-tauri/src/player.rs"

bash -c 'rg -q "wait_for_startup_readiness" "$1" && rg -q "startup_readiness_rejects_non_idle_session_without_position" "$1" && rg -q "startup_readiness_rejects_non_idle_session_at_zero_position" "$1"' AT-1 "$player"
bash -c '! rg -q "recovery\\.clone\\(\\)\\.filter\\(\\|context\\| context\\.is_live\\)" "$1"' AT-2 "$player"
bash -c 'rg -q "fn non_live_abnormal_exit_uses_next_strategy" "$1" && rg -q "fn startup_readiness_rejects_idle_session" "$1"' AT-3 "$player"

cargo test --manifest-path "$project_dir/src-tauri/Cargo.toml"

echo "AT-3 PASS: real remote playback regression guards are green"
