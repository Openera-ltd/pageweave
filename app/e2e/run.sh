#!/usr/bin/env bash
# Run the real-app E2E suite under a virtual display with isolated config/data directories.
# Usage: e2e/run.sh <app-binary> <artifact-dir>
set -euo pipefail
APP="$(realpath "${1:?app binary}")"
ART="$(realpath -m "${2:-e2e-artifacts}")"
mkdir -p "$ART"
HOME_DIR="$(mktemp -d)"
export XDG_CONFIG_HOME="$HOME_DIR/config" XDG_DATA_HOME="$HOME_DIR/data" XDG_CACHE_HOME="$HOME_DIR/cache"
cd "$(dirname "$0")/.."
xvfb-run -a -s "-screen 0 1366x900x24" bash -c '
  tauri-driver --port 4444 > "'"$ART"'/tauri-driver.log" 2>&1 &
  DRIVER=$!
  for i in $(seq 1 50); do curl -sf http://127.0.0.1:4444/status >/dev/null && break; sleep 0.2; done
  node e2e/run.mjs "'"$APP"'" "'"$ART"'"; code=$?
  kill $DRIVER 2>/dev/null || true
  exit $code
'
