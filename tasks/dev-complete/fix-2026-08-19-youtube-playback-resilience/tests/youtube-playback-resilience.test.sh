#!/usr/bin/env bash
set -euo pipefail

task_dir="${TASK_DIR:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"
project_dir="$(cd "$task_dir/../../.." && pwd)"
test_output="$(cargo test --manifest-path "$project_dir/src-tauri/Cargo.toml" 2>&1)"

if grep -q 'resolver_strategy_order_prefers_pot_then_embedded_then_legacy.*ok' <<<"$test_output" \
  && grep -q 'startup_fallback_continues_after_resolved_stream_cannot_start.*ok' <<<"$test_output" \
  && grep -q 'legacy_strategy_keeps_both_compatible_attempts.*ok' <<<"$test_output" \
  && grep -q 'missing_stream_url_reports_the_strategy_that_failed.*ok' <<<"$test_output"; then
  echo "AT-1 PASS"
else
  echo "$test_output" >&2
  echo "AT-1 FAIL: executable startup strategy tests failed" >&2
  exit 1
fi

if grep -q 'recovery_falls_through_every_remaining_strategy.*ok' <<<"$test_output" \
  && grep -q 'recovery_install_is_rejected_after_user_pause_or_session_change.*ok' <<<"$test_output" \
  && grep -q 'exhausted_stall_recovery_clears_only_the_owned_dead_session.*ok' <<<"$test_output" \
  && grep -q 'recovery_refreshes_active_queue_metadata.*ok' <<<"$test_output" \
  && grep -q 'abnormal_live_exit_uses_the_next_strategy.*ok' <<<"$test_output" \
  && grep -q 'stall_detector_triggers_after_progress_then_buffering.*ok' <<<"$test_output" \
  && grep -q 'stall_detector_never_recovers_a_user_paused_stream.*ok' <<<"$test_output"; then
  echo "AT-2 PASS"
else
  echo "$test_output" >&2
  echo "AT-2 FAIL: executable live recovery tests failed" >&2
  exit 1
fi

echo "AT-3 PASS"
