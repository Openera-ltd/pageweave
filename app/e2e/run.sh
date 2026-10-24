#!/usr/bin/env bash
# Run the real-app E2E suite under a virtual display with isolated config/data directories.
# Usage: e2e/run.sh <app-binary> <artifact-dir>
set -euo pipefail
APP="$(realpath "${1:?app binary}")"
ART="$(realpath -m "${2:-e2e-artifacts}")"
mkdir -p "$ART"
rm -f "$ART"/FAIL-* "$ART"/e2e-results.json
HOME_DIR="$(mktemp -d)"
export XDG_CONFIG_HOME="$HOME_DIR/config" XDG_DATA_HOME="$HOME_DIR/data" XDG_CACHE_HOME="$HOME_DIR/cache"
cd "$(dirname "$0")/.."
# GTK apps wait out a D-Bus/accessibility-bus timeout per launch when no session bus exists (seen on CI
# runners: ~30 s per app start). Provide a private session bus when none is available.
RUNNER=()
if [ -z "${DBUS_SESSION_BUS_ADDRESS:-}" ] && command -v dbus-run-session >/dev/null; then RUNNER=(dbus-run-session --); fi
"${RUNNER[@]}" xvfb-run -a -s "-screen 0 1366x900x24" bash -c '
  tauri-driver --port 4444 > "'"$ART"'/tauri-driver.log" 2>&1 &
  DRIVER=$!
  for i in $(seq 1 50); do curl -sf http://127.0.0.1:4444/status >/dev/null && break; sleep 0.2; done
  timeout 1800 node "'"${E2E_SCRIPT:-e2e/run.mjs}"'" "'"$APP"'" "'"$ART"'" '"${E2E_ARGS:-}"'; code=$?
  [ "$code" = 124 ] && echo "E2E suite exceeded 30 minutes and was stopped" >&2
  kill $DRIVER 2>/dev/null || true
  exit $code
'
