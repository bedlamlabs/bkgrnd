#!/usr/bin/env bash
set -euo pipefail

# Sync local bkgrnd data (history + derived playlists) to the bkgrnd server.
#
# Env vars:
# - BKGRND_WOPR_BASE_URL: e.g. "https://bkgrnd.bedl.am" (default)
# - BKGRND_WOPR_TOKEN: optional; sent as Authorization header for auth-enabled servers
# - BKGRND_HISTORY_PATH: optional; defaults to ~/.bkgrnd/history.json

BASE_URL="${BKGRND_WOPR_BASE_URL:-https://bkgrnd.bedl.am}"
TOKEN="${BKGRND_WOPR_TOKEN:-}"
HISTORY_PATH="${BKGRND_HISTORY_PATH:-$HOME/.bkgrnd/history.json}"

if [[ ! -f "$HISTORY_PATH" ]]; then
  echo "missing history file: $HISTORY_PATH" >&2
  exit 1
fi

auth_args=()
if [[ -n "$TOKEN" ]]; then
  auth_args=(-H "Authorization: Bearer $TOKEN")
fi

echo "sync: history -> $BASE_URL/api/v1/history.json"
curl -fsS -X PUT \
  "${auth_args[@]}" \
  -H "content-type: application/json" \
  --data-binary @"$HISTORY_PATH" \
  "$BASE_URL/api/v1/history.json" \
  >/dev/null

echo "sync: playlists (derived from history) -> $BASE_URL/api/v1/playlists.json"
HISTORY_PATH="$HISTORY_PATH" /usr/bin/python3 - <<'PY' | curl -fsS -X PUT "${auth_args[@]}" -H "content-type: application/json" --data-binary @- "$BASE_URL/api/v1/playlists.json" >/dev/null
import json, os, sys, datetime

path = os.environ.get("HISTORY_PATH") or (os.path.expanduser("~/.bkgrnd/history.json"))
with open(path, "r", encoding="utf-8") as f:
    history = json.load(f)
if not isinstance(history, list):
    history = []

items = []
for h in history:
    if not isinstance(h, dict):
        continue
    url = h.get("url") or ""
    if not url:
        continue
    items.append({
        "url": url,
        "title": h.get("title") or url,
        "channel": "",
        "thumbnail": h.get("thumbnail") or "",
        "addedAt": h.get("playedAt") or h.get("played_at") or datetime.datetime.utcnow().isoformat() + "Z",
    })

doc = {
    "version": 1,
    "updatedAt": datetime.datetime.utcnow().isoformat() + "Z",
    "playlists": [{
        "id": "recent",
        "name": "Recent",
        "items": items,
    }],
}
sys.stdout.write(json.dumps(doc))
PY

echo "ok"
