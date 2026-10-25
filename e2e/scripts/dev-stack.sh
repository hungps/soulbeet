#!/usr/bin/env bash
# Runs the e2e stack natively (no docker): the stub services plus two soulbeet
# servers from the binary produced by `dx bundle --package web --release`, the
# main one on 9765 and the issue #90 one on 9766. Works on Linux and macOS;
# CI uses compose.e2e.yml.
#
#   scripts/dev-stack.sh up      start stubs + servers, wait until ready
#   scripts/dev-stack.sh down    stop them
#
# Requires Python 3.11 or 3.12 (the beets config check runs `python -P`; on
# 3.13+ beets 2.11 wants numpy 2.3.5 while build/requirements-beets.txt pins
# 2.2.6). beets is installed into .runtime/beets-venv, pinned like the image.
# PYTHON picks the interpreter for a new venv; otherwise python3.11/3.12 is
# used, or `uv` fetches 3.11 when neither is installed. Its interpreter, not
# any system beets, runs the config check.
#
# Specs that need a media server expect a Navidrome you run yourself, with
# E2E_NAVIDROME=1 and E2E_NAVIDROME_URL pointing at it.
#
# The servers run with HOME=.runtime/home, like the image's HOME=/tmp, so a
# developer's ~/.config/beets/config.yaml does not leak into the stack.

set -euo pipefail

E2E_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
REPO_DIR="$(dirname "$E2E_DIR")"
SERVER_DIR="$REPO_DIR/target/dx/web/release/web"
RUNTIME="$E2E_DIR/.runtime"
PID_DIR="$RUNTIME/pids"
LOG_DIR="$RUNTIME/logs"

cmd="${1:-up}"

stop_pid() {
  local name="$1"
  if [[ -f "$PID_DIR/$name.pid" ]]; then
    kill "$(cat "$PID_DIR/$name.pid")" 2>/dev/null || true
    rm -f "$PID_DIR/$name.pid"
  fi
}

# The pids listening on `port`. lsof is the portable answer; ss covers a
# container image that ships it without lsof.
port_pids() {
  local port="$1"
  if command -v lsof >/dev/null 2>&1; then
    lsof -nP -iTCP:"$port" -sTCP:LISTEN -t 2>/dev/null || true
  elif command -v ss >/dev/null 2>&1; then
    ss -tlnp 2>/dev/null | grep ":$port " | grep -oE 'pid=[0-9]+' | cut -d= -f2 | head -1
  fi
}

# Kill whatever still listens on a stack port; pid files alone leak orphans
# when `up` runs twice, and killing by port also reaps children of npx.
free_port() {
  local port="$1" pid
  for pid in $(port_pids "$port"); do
    kill "$pid" 2>/dev/null || true
  done
  sleep 0.5
}

# Starts `command` detached: its own session where setsid exists (Linux),
# nohup elsewhere (macOS). The subshell closes the descriptors it inherited
# from the caller and `exec`s the command, so nothing keeps a caller's pipe
# open and the pid in the file is the process itself.
start_daemon() {
  local name="$1" log="$2" dir="$3"
  shift 3
  (
    local fd
    for fd in $(seq 3 63); do eval "exec $fd>&-" 2>/dev/null || true; done
    cd "$dir"
    if command -v setsid >/dev/null 2>&1; then
      exec setsid "$@"
    else
      exec nohup "$@"
    fi
  ) >"$log" 2>&1 </dev/null &
  echo $! >"$PID_DIR/$name.pid"
}

down() {
  stop_pid soulbeet
  stop_pid soulbeet-user90
  stop_pid stubs
  for port in 9765 9766 5030 5050; do
    free_port "$port"
  done
  echo "stack stopped"
}

# Point the pid file at the process that actually listens, not at the subshell
# that started it: children of npx and nohup outlive their parent.
record_port_pid() {
  local name="$1" port="$2" pid="" tries=0
  while [[ -z "$pid" && $tries -lt 20 ]]; do
    pid="$(port_pids "$port" | head -1)"
    [[ -z "$pid" ]] && sleep 0.1
    tries=$((tries + 1))
  done
  [[ -n "$pid" ]] && echo "$pid" >"$PID_DIR/$name.pid"
}

wait_for() {
  local url="$1" name="$2" tries=0
  until curl -fso /dev/null --max-time 2 "$url"; do
    tries=$((tries + 1))
    if [[ $tries -gt 60 ]]; then
      echo "$name did not come up ($url)" >&2
      exit 1
    fi
    sleep 0.5
  done
  echo "$name ready"
}

# The venv's interpreter, when it is one beets 2.11 can install on.
venv_ok() {
  local venv="$1"
  [[ -x "$venv/bin/python" ]] &&
    "$venv/bin/python" -c 'import sys; sys.exit(0 if (3, 11) <= sys.version_info[:2] <= (3, 12) else 1)' 2>/dev/null
}

# uv, including ~/.local/bin when a non-login shell does not have it on PATH.
uv_bin() {
  if command -v uv >/dev/null 2>&1; then
    command -v uv
  elif [[ -x "$HOME/.local/bin/uv" ]]; then
    echo "$HOME/.local/bin/uv"
  fi
}

# Creates the beets venv. PYTHON wins; then python3.11/3.12; then uv, which
# downloads 3.11 if no suitable interpreter is installed.
make_venv() {
  local venv="$1" candidate
  if [[ -n "${PYTHON:-}" ]]; then
    "$PYTHON" -m venv "$venv"
    return
  fi
  for candidate in python3.11 python3.12; do
    if command -v "$candidate" >/dev/null 2>&1; then
      "$candidate" -m venv "$venv"
      return
    fi
  done
  local uv
  uv="$(uv_bin)"
  if [[ -n "$uv" ]]; then
    "$uv" venv --python 3.11 "$venv" >/dev/null
    return
  fi
  echo "beets needs Python 3.11 or 3.12; install one, or uv, or set PYTHON" >&2
  exit 1
}

# Installs the pinned beets into the venv. uv is used when it is around; a
# uv-made venv has no pip, so ensurepip covers the case where uv went away.
install_beets() {
  local venv="$1" requirements="$2" uv
  uv="$(uv_bin)"
  if [[ -n "$uv" ]]; then
    "$uv" pip install --python "$venv/bin/python" --quiet -c "$requirements" \
      beets requests musicbrainzngs
    return
  fi
  if [[ ! -x "$venv/bin/pip" ]]; then
    "$venv/bin/python" -m ensurepip --upgrade >/dev/null
  fi
  "$venv/bin/pip" install --quiet -c "$requirements" \
    beets requests musicbrainzngs
}

up() {
  if [[ ! -x "$SERVER_DIR/server" ]]; then
    echo "server binary missing; run 'dx bundle --package web --release' in $REPO_DIR first" >&2
    echo "(the dioxus CLI, not any other 'dx' on PATH)" >&2
    exit 1
  fi

  for port in 9765 9766 5030 5050; do
    free_port "$port"
  done

  mkdir -p "$PID_DIR" "$LOG_DIR" "$RUNTIME/home"
  (cd "$E2E_DIR" && npm run --silent fixtures)

  local venv="$RUNTIME/beets-venv"
  if ! venv_ok "$venv"; then
    rm -rf "$venv"
    make_venv "$venv"
  fi
  install_beets "$venv" "$REPO_DIR/build/requirements-beets.txt"
  export PATH="$venv/bin:$PATH"
  # The servers' beets config check runs this interpreter.
  export BEETS_PYTHON="$venv/bin/python"

  start_daemon stubs "$LOG_DIR/stubs.log" "$E2E_DIR" npx tsx stubs/src/main.ts
  wait_for "http://127.0.0.1:5050/ws/2/recording?query=ping" "stubs"
  record_port_pid stubs 5030

  start_daemon soulbeet "$LOG_DIR/soulbeet.log" "$SERVER_DIR" \
    env HOME="$RUNTIME/home" \
    DATABASE_URL="sqlite:$RUNTIME/data/soulbeet.db" \
    SECRET_KEY="e2e-secret-key" \
    BEETS_CONFIG="$E2E_DIR/.fixtures/beets-e2e.yaml" \
    PORT=9765 IP=127.0.0.1 \
    ./server
  wait_for "http://127.0.0.1:9765/" "soulbeet"
  record_port_pid soulbeet 9765

  start_daemon soulbeet-user90 "$LOG_DIR/soulbeet-user90.log" "$SERVER_DIR" \
    env HOME="$RUNTIME/home" \
    DATABASE_URL="sqlite:$RUNTIME/data/user90.db" \
    SECRET_KEY="e2e-secret-key" \
    DOWNLOAD_PATH="$RUNTIME/downloads" \
    BEETS_CONFIG="$RUNTIME/user90-config/config.yaml" \
    MUSICBRAINZ_HOST="127.0.0.1:5050" \
    PORT=9766 IP=127.0.0.1 \
    ./server
  wait_for "http://127.0.0.1:9766/" "soulbeet-user90"
  record_port_pid soulbeet-user90 9766

  echo "stack up: app http://127.0.0.1:9765, issue #90 app http://127.0.0.1:9766,"
  echo "slskd stub :5030, mb stub :5050"
  echo "logs: $LOG_DIR"
}

case "$cmd" in
  up) up ;;
  down) down ;;
  *)
    echo "usage: $0 {up|down}" >&2
    exit 1
    ;;
esac
