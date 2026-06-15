#!/usr/bin/env bash
set -euo pipefail

HOST="${HPX_HOST:-hpx@hpx.lan}"
APP_ROOT="${HPX_APP_ROOT:-/etc/dokploy/compose/wopr-apps/code}"
REPO_DIR="${HPX_REPO_DIR:-$APP_ROOT/bkgrnd}"
REPO_URL="${HPX_REPO_URL:-https://github.com/bedlamlabs/bkgrnd.git}"
BRANCH="${HPX_BRANCH:-main}"
MODE="${1:-connect}"
SSH_BIN="${SSH_BIN:-/usr/bin/ssh}"
COMPOSE_SERVICE="${HPX_COMPOSE_SERVICE:-bkgrnd}"

if [[ "$MODE" != "connect" && "$MODE" != "deploy" && "$MODE" != "inspect" ]]; then
  echo "Usage: $0 [connect|deploy|inspect]" >&2
  exit 2
fi

"$SSH_BIN" "$HOST" bash -s -- "$APP_ROOT" "$REPO_DIR" "$REPO_URL" "$BRANCH" "$MODE" "$COMPOSE_SERVICE" <<'REMOTE'
set -euo pipefail

APP_ROOT="$1"
REPO_DIR="$2"
REPO_URL="$3"
BRANCH="$4"
MODE="$5"
COMPOSE_SERVICE="$6"
STAMP="$(date -u +%Y%m%dT%H%M%SZ)"

echo "HPX app root: $APP_ROOT"
echo "HPX repo dir:  $REPO_DIR"
echo "GitHub repo:   $REPO_URL"
echo "Branch:        $BRANCH"

if ! command -v git >/dev/null 2>&1; then
  echo "git is not installed on HPX" >&2
  exit 1
fi

mkdir -p "$APP_ROOT"

if [[ "$MODE" == "inspect" ]]; then
  echo "Compose root listing:"
  find "$APP_ROOT" -maxdepth 2 -type f \( -name 'compose*.yml' -o -name 'docker-compose*.yml' -o -name '.env' -o -name '.env.*' \) -print | sort
  echo
  echo "Recent bkgrnd backups:"
  find "$APP_ROOT" -maxdepth 1 -type d -name 'bkgrnd.bak.*' -print | sort | tail -5
  echo
  echo "Compose env references:"
  for file in "$APP_ROOT"/compose*.yml "$APP_ROOT"/docker-compose*.yml; do
    [[ -f "$file" ]] || continue
    echo "--- $file"
    grep -nE 'env_file|\.env|bkgrnd|build:|context:|image:' "$file" || true
  done
  exit 0
fi

if [[ -d "$REPO_DIR/.git" ]]; then
  echo "Existing git worktree found. Updating remote metadata only."
  git -C "$REPO_DIR" remote set-url origin "$REPO_URL" 2>/dev/null || git -C "$REPO_DIR" remote add origin "$REPO_URL"
  git -C "$REPO_DIR" fetch origin "$BRANCH"
  git -C "$REPO_DIR" checkout "$BRANCH"
  git -C "$REPO_DIR" pull --ff-only origin "$BRANCH"
else
  BACKUP_DIR=""
  if [[ -e "$REPO_DIR" ]]; then
    BACKUP_DIR="${REPO_DIR}.bak.${STAMP}"
    echo "Backing up existing non-git directory to $BACKUP_DIR"
    mv "$REPO_DIR" "$BACKUP_DIR"
  fi

  echo "Cloning GitHub repo into $REPO_DIR"
  git clone --branch "$BRANCH" "$REPO_URL" "$REPO_DIR"

  if [[ -n "$BACKUP_DIR" ]]; then
    for env_file in .env .env.local server/.env server/.env.local; do
      if [[ -f "$BACKUP_DIR/$env_file" && ! -f "$REPO_DIR/$env_file" ]]; then
        mkdir -p "$(dirname "$REPO_DIR/$env_file")"
        cp "$BACKUP_DIR/$env_file" "$REPO_DIR/$env_file"
        echo "Preserved $env_file from backup"
      fi
    done
  fi
fi

git -C "$REPO_DIR" status --short --branch

if [[ "$MODE" != "deploy" ]]; then
  echo "Connected HPX worktree to GitHub. Container was not rebuilt."
  exit 0
fi

if ! command -v docker >/dev/null 2>&1; then
  echo "docker is not installed on HPX" >&2
  exit 1
fi

cd "$APP_ROOT"
if [[ ! -f "$APP_ROOT/.env" ]]; then
  printf '# Created by bkgrnd HPX deploy wrapper so Docker Compose can parse shared env_file references.\n' > "$APP_ROOT/.env"
  echo "Created missing shared Compose env placeholder: $APP_ROOT/.env"
fi

if docker compose version >/dev/null 2>&1; then
  COMPOSE=(docker compose)
elif command -v docker-compose >/dev/null 2>&1; then
  COMPOSE=(docker-compose)
else
  echo "Docker Compose is not installed on HPX" >&2
  exit 1
fi

echo "Rebuilding $COMPOSE_SERVICE through Docker Compose"
"${COMPOSE[@]}" up -d --build "$COMPOSE_SERVICE"
"${COMPOSE[@]}" ps "$COMPOSE_SERVICE"
REMOTE
