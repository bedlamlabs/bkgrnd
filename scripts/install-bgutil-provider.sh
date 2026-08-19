#!/usr/bin/env bash
set -euo pipefail

# Patched upstream PR #243. The commit and archive hash are pinned so an
# installer run cannot silently consume a changed pull-request branch.
provider_commit="fbe4ed47f3b63cf061f1158f18f74bcc90e54033"
provider_archive_sha256="cbc8c2e54126ec38f4c2a278b3cab685d337cadc3e7f09762116e3b28be18b5f"
provider_url="https://codeload.github.com/Brainicism/bgutil-ytdlp-pot-provider/tar.gz/$provider_commit"

bkgrnd_data_dir="${BKGRND_DATA_DIR:-$HOME/.bkgrnd}"
provider_root="$bkgrnd_data_dir/bgutil-provider/$provider_commit"
plugin_dir="$bkgrnd_data_dir/yt-dlp-plugins"
launch_agents_dir="$HOME/Library/LaunchAgents"
launch_agent="$launch_agents_dir/com.bedlamlabs.bkgrnd.bgutil-pot-provider.plist"
deno_bin="${BKGRND_DENO_BIN:-/opt/homebrew/bin/deno}"
install_tmp=""
plist_tmp=""

cleanup() {
  [[ -z "$install_tmp" ]] || rm -rf "$install_tmp"
  [[ -z "$plist_tmp" ]] || rm -f "$plist_tmp"
}
trap cleanup EXIT

if [[ ! -x "$deno_bin" ]]; then
  echo "Deno 2 is required at $deno_bin" >&2
  exit 1
fi

plugin_archive="$plugin_dir/bgutil-ytdlp-pot-provider.zip"
plugin_marker="$plugin_dir/bgutil-ytdlp-pot-provider.version"
source_ready=false
plugin_ready=false
[[ -f "$provider_root/server/src/main.ts" ]] && source_ready=true
if [[ -f "$plugin_archive" && -f "$plugin_marker" ]] \
  && unzip -tq "$plugin_archive" >/dev/null 2>&1; then
  installed_plugin_sha256="$(shasum -a 256 "$plugin_archive" | awk '{print $1}')"
  expected_plugin_marker="$provider_commit $installed_plugin_sha256"
  [[ "$(cat "$plugin_marker")" == "$expected_plugin_marker" ]] && plugin_ready=true
fi

if [[ "$source_ready" != true || "$plugin_ready" != true ]]; then
  install_tmp="$(mktemp -d)"
  archive="$install_tmp/provider.tar.gz"
  curl -fsSL "$provider_url" -o "$archive"
  actual_sha256="$(shasum -a 256 "$archive" | awk '{print $1}')"
  if [[ "$actual_sha256" != "$provider_archive_sha256" ]]; then
    echo "Provider archive checksum mismatch" >&2
    exit 1
  fi

  mkdir -p "$install_tmp/source" "$provider_root"
  tar -xzf "$archive" -C "$install_tmp/source" --strip-components=1
  if [[ "$source_ready" != true ]]; then
    if [[ -d "$provider_root/server" ]]; then
      mv "$provider_root/server" "$provider_root/server.incomplete.$(date -u +%Y%m%dT%H%M%SZ)"
    fi
    cp -R "$install_tmp/source/server" "$provider_root/server"
  fi

  if [[ "$plugin_ready" != true ]]; then
    mkdir -p "$plugin_dir"
    /usr/bin/ditto -c -k --keepParent \
      "$install_tmp/source/plugin/yt_dlp_plugins" \
      "$plugin_archive"
    unzip -tq "$plugin_archive" >/dev/null
    plugin_sha256="$(shasum -a 256 "$plugin_archive" | awk '{print $1}')"
    printf '%s %s\n' "$provider_commit" "$plugin_sha256" > "$plugin_marker"
  fi
fi

# Upstream 1.x binds on all interfaces. bkgrnd's provider is a local helper
# with no authentication, so make the pinned source loopback-only before it is
# ever launched.
provider_main="$provider_root/server/src/main.ts"
/usr/bin/sed -i '' \
  -e 's/host: "::"/host: "127.0.0.1"/' \
  -e 's/host: "0.0.0.0"/host: "127.0.0.1"/' \
  "$provider_main"
if grep -Eq 'host: "(::|0\.0\.0\.0)"' "$provider_main"; then
  echo "Provider source still contains a non-loopback listener" >&2
  exit 1
fi

# Deno's frozen install is safe to repeat and repairs a missing/incomplete
# node_modules tree left by an interrupted earlier installer run.
(
  cd "$provider_root/server"
  "$deno_bin" install --prod --allow-scripts=npm:canvas --frozen
)
test -d "$provider_root/server/node_modules"
test -f "$plugin_archive"

mkdir -p "$launch_agents_dir" "$bkgrnd_data_dir/logs"
plist_tmp="$(mktemp)"
sed \
  -e "s|__DENO_BIN__|$deno_bin|g" \
  -e "s|__WORKING_DIR__|$provider_root/server/node_modules|g" \
  -e "s|__LOG_DIR__|$bkgrnd_data_dir/logs|g" \
  > "$plist_tmp" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>com.bedlamlabs.bkgrnd.bgutil-pot-provider</string>
  <key>ProgramArguments</key>
  <array>
    <string>__DENO_BIN__</string>
    <string>run</string>
    <string>--allow-env</string>
    <string>--allow-net</string>
    <string>--allow-ffi=.</string>
    <string>--allow-read=.</string>
    <string>../src/main.ts</string>
    <string>--port</string>
    <string>4416</string>
  </array>
  <key>WorkingDirectory</key>
  <string>__WORKING_DIR__</string>
  <key>RunAtLoad</key>
  <true/>
  <key>KeepAlive</key>
  <true/>
  <key>StandardOutPath</key>
  <string>__LOG_DIR__/bgutil-provider.log</string>
  <key>StandardErrorPath</key>
  <string>__LOG_DIR__/bgutil-provider.error.log</string>
</dict>
</plist>
PLIST

plutil -lint "$plist_tmp" >/dev/null
cp "$plist_tmp" "$launch_agent"
launchctl bootout "gui/$UID/com.bedlamlabs.bkgrnd.bgutil-pot-provider" >/dev/null 2>&1 || true
for _ in {1..20}; do
  launchctl print "gui/$UID/com.bedlamlabs.bkgrnd.bgutil-pot-provider" >/dev/null 2>&1 || break
  sleep 0.25
done
bootstrapped=false
for _ in {1..5}; do
  if launchctl bootstrap "gui/$UID" "$launch_agent"; then
    bootstrapped=true
    break
  fi
  sleep 1
done
if [[ "$bootstrapped" != true ]]; then
  echo "Could not register the patched POT provider launch agent" >&2
  exit 1
fi

for _ in {1..30}; do
  if curl -fsS http://127.0.0.1:4416/ping >/dev/null; then
    if ! lsof -nP -iTCP:4416 -sTCP:LISTEN | awk 'NR > 1 {print $9}' | grep -qx '127.0.0.1:4416'; then
      echo "Provider is healthy but is not bound exclusively to 127.0.0.1:4416" >&2
      exit 1
    fi
    echo "Patched bgutil POT provider is ready on 127.0.0.1:4416"
    exit 0
  fi
  sleep 1
done

echo "Provider did not become healthy; see $bkgrnd_data_dir/logs/bgutil-provider.error.log" >&2
exit 1
