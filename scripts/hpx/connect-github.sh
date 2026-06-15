#!/usr/bin/env bash
set -euo pipefail

HOST="${HPX_HOST:-hpx@hpx.lan}"
APP_ROOT="${HPX_APP_ROOT:-/etc/dokploy/compose/wopr-apps/code}"
REPO_DIR="${HPX_REPO_DIR:-$APP_ROOT/bkgrnd}"
REPO_URL="${HPX_REPO_URL:-https://github.com/bedlamlabs/bkgrnd.git}"
BRANCH="${HPX_BRANCH:-main}"
MODE="${1:-connect}"
SSH_BIN="${SSH_BIN:-/usr/bin/ssh}"

if [[ "$MODE" != "connect" && "$MODE" != "deploy" ]]; then
  echo "Usage: $0 [connect|deploy]" >&2
  exit 2
fi

"$SSH_BIN" "$HOST" bash -s -- "$APP_ROOT" "$REPO_DIR" "$REPO_URL" "$BRANCH" "$MODE" <<'REMOTE'
set -euo pipefail

APP_ROOT="$1"
REPO_DIR="$2"
REPO_URL="$3"
BRANCH="$4"
MODE="$5"
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
if docker compose version >/dev/null 2>&1; then
  COMPOSE=(docker compose)
elif command -v docker-compose >/dev/null 2>&1; then
  COMPOSE=(docker-compose)
else
  echo "Docker Compose is not installed on HPX" >&2
  exit 1
fi

echo "Rebuilding bkgrnd through Docker Compose"
"${COMPOSE[@]}" up -d --build bkgrnd-hpx 2>/dev/null || "${COMPOSE[@]}" up -d --build
REMOTE
