#!/usr/bin/env bash
# Fast local gate — run before every push. Mirrors CI's non-E2E checks and stops at the first failure.
# Usage: tools/check.sh            (add --e2e to also run the real-app E2E suite on Linux)
set -euo pipefail
cd "$(dirname "$0")/.."
step() { printf '\n== %s\n' "$*"; }
step "rustfmt";            cargo fmt --all -- --check
step "clippy";             cargo clippy --workspace --all-targets -q -- -D warnings
step "rust tests";         cargo test --workspace -q 2>&1 | grep -E 'test result|FAILED|panicked' ; test "${PIPESTATUS[0]}" -eq 0
step "tool tests";         python3 -I -m unittest discover -s tools/tests -q
# Note: `set -e` does not stop on a failure left of `&&`, so each check is its own command.
step "fixtures reproducible"; python3 -I tools/fixtures/gen_fixtures.py >/dev/null
git diff --exit-code --stat -- fixtures/pdf ':!fixtures/pdf/encrypted-*'
step "UI typecheck";       (cd app && npx tsc --noEmit)
step "UI unit tests";      (cd app && npx vitest run --reporter=dot)
step "interaction audit";  (cd app && node scripts/audit-controls.mjs --strict)
git diff --exit-code --stat -- docs/INTERACTION_INVENTORY.md
step "license bundles";    python3 -I tools/gen_licenses.py --check
step "secret scan (staged + history)"; if command -v gitleaks >/dev/null; then gitleaks git --redact --no-banner . ; else echo "gitleaks not installed — SKIPPED (CI still runs it)"; fi
if [ "${1:-}" = "--e2e" ]; then
  step "real-app E2E"
  (cd app && npx tauri build --debug --no-bundle >/dev/null)
  (cd app && PAGEWEAVE_CLI="$PWD/../target/debug/pageweave" ./e2e/run.sh ../target/debug/pageweave-app ../e2e-artifacts)
fi
printf '\nAll local checks passed.\n'
