#!/usr/bin/env bash
set -euo pipefail

task_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
project_dir="$(cd "$task_dir/../../.." && pwd)"
lib_rs="$project_dir/src-tauri/src/lib.rs"
installed_mode="${1:-}"

bash -c 'test -f "$1"' AT-1 "$lib_rs"
bash -c 'test -d "$1"' AT-2 "$project_dir"

if rg -n 'window\.set_focus\(\)' "$lib_rs" >/dev/null; then
  echo "AT-1 FAIL: tray toggle still reaches Tao set_focus/activateIgnoringOtherApps" >&2
  exit 1
fi

if ! rg -n 'window\.show\(\)' "$lib_rs" >/dev/null; then
  echo "AT-2 FAIL: tray toggle no longer shows the panel" >&2
  exit 1
fi

if [[ "$installed_mode" == "--installed" ]]; then
  app_path="/Applications/bkgrnd.app"
  bundle_id="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$app_path/Contents/Info.plist")"
  ui_element="$(/usr/libexec/PlistBuddy -c 'Print :LSUIElement' "$app_path/Contents/Info.plist")"
  [[ "$bundle_id" == "com.bedlamlabs.bkgrnd" ]] || { echo "AT-2 FAIL: wrong installed bundle" >&2; exit 1; }
  [[ "$ui_element" == "true" ]] || { echo "AT-2 FAIL: installed app is not LSUIElement" >&2; exit 1; }

  pgrep -f '/Applications/bkgrnd.app/Contents/MacOS/bkgrnd' >/dev/null || open -gj -a "$app_path"
  for _ in {1..40}; do
    pgrep -f '/Applications/bkgrnd.app/Contents/MacOS/bkgrnd' >/dev/null && break
    sleep 0.25
  done
  pgrep -f '/Applications/bkgrnd.app/Contents/MacOS/bkgrnd' >/dev/null || { echo "AT-2 FAIL: installed app is not running" >&2; exit 1; }

  before="$(osascript -e 'tell application "System Events" to get name of first application process whose frontmost is true')"
  [[ "$before" != "bkgrnd" ]] || { echo "AT-1 FAIL: bkgrnd was already frontmost" >&2; exit 1; }

  osascript <<'APPLESCRIPT' >/dev/null
tell application "System Events"
  tell application process "bkgrnd"
    click menu bar item 1 of menu bar 2
  end tell
end tell
APPLESCRIPT
  sleep 1

  after="$(osascript -e 'tell application "System Events" to get name of first application process whose frontmost is true')"
  if [[ "$after" == "bkgrnd" ]]; then
    echo "AT-1 FAIL: installed bkgrnd stole frontmost status from $before" >&2
    exit 1
  fi
  echo "AT-1 PASS: installed tray interaction retained frontmost application ($before -> $after)"

  osascript <<'APPLESCRIPT' >/dev/null
tell application "System Events"
  tell application process "bkgrnd"
    click menu bar item 1 of menu bar 2
  end tell
end tell
APPLESCRIPT
  sleep 0.5
  pgrep -f '/Applications/bkgrnd.app/Contents/MacOS/bkgrnd' >/dev/null || { echo "AT-2 FAIL: app exited during panel dismissal" >&2; exit 1; }
  echo "AT-2 PASS: installed UIElement panel toggled without terminating the app"
else
  echo "AT-1 PASS: tray toggle has no forced Tao focus request"
  echo "AT-2 PASS: tray toggle still shows the panel"
fi
