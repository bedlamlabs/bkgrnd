#!/usr/bin/env bash
set -euo pipefail

project_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
app_source="$project_dir/src-tauri/target/release/bundle/macos/bkgrnd.app"
app_target="/Applications/bkgrnd.app"
backup_dir="$HOME/.bkgrnd/backups"

# POT is part of the install/update path, not an undocumented machine
# prerequisite. The installer is commit- and checksum-pinned.
bash "$project_dir/scripts/install-bgutil-provider.sh"

(
  cd "$project_dir"
  npm run tauri build -- --bundles app
)

osascript -e 'tell application id "com.bedlamlabs.bkgrnd" to quit' >/dev/null 2>&1 || true
for _ in {1..20}; do
  pgrep -f '/Applications/bkgrnd.app/Contents/MacOS/bkgrnd' >/dev/null || break
  sleep 0.25
done
running_pid="$(pgrep -f '/Applications/bkgrnd.app/Contents/MacOS/bkgrnd' || true)"
if [[ -n "$running_pid" ]]; then
  kill -TERM $running_pid
  for _ in {1..40}; do
    pgrep -f '/Applications/bkgrnd.app/Contents/MacOS/bkgrnd' >/dev/null || break
    sleep 0.25
  done
fi
if pgrep -f '/Applications/bkgrnd.app/Contents/MacOS/bkgrnd' >/dev/null; then
  echo "Previous bkgrnd process did not exit; refusing to replace a running app bundle" >&2
  exit 1
fi

mkdir -p "$backup_dir"
if [[ -d "$app_target" ]]; then
  backup_path="$backup_dir/bkgrnd-$(date -u +%Y%m%dT%H%M%SZ).app"
  mv "$app_target" "$backup_path"
  echo "Previous app retained at $backup_path"
fi

/usr/bin/ditto "$app_source" "$app_target"
open -a "$app_target"
for _ in {1..40}; do
  new_pid="$(pgrep -f '/Applications/bkgrnd.app/Contents/MacOS/bkgrnd' | head -1 || true)"
  if [[ -n "$new_pid" ]]; then
    sleep 1
    if kill -0 "$new_pid" 2>/dev/null; then
      echo "bkgrnd is installed and running"
      exit 0
    fi
  fi
  sleep 0.25
done

echo "bkgrnd did not start after installation" >&2
exit 1
