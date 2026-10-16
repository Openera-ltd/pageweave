# PageWeave Work Journal

**Current status:** Master instruction pack and PRD documents created. This documentation task did not inspect or build the PageWeave application. No training run, cloud upload, scheduler, packaged build or production verification is claimed.

## Initial checkpoint
- Date: 2026-10-16
- Repository commit/branch: not provided in this documentation-generation task
- Next action: inspect the actual project, reconcile existing code/docs against PRDs, establish baseline build/tests/licenses/format support, make requirement traceability matrix, and implement/test the monitor and first vertical slice.
- Build/test status: application tests not run here.
- Model checkpoint: none verified here.
- Watchdog: specified, not installed or test-verified by this document-generation task.

## Append one entry per meaningful work unit
Timestamp; commit/branch; requirement IDs; changed files; exact commands/results; packaged app checked; performance/quality deltas; active run IDs; latest durable model checkpoint; blockers; next action/resume command. Never record secrets or document contents.

## 2026-10-16 — Session 1, checkpoint 1: PRD pack installed, foundation decisions
- Branch: `ccr-81913227-01wzmc`. Environment: Linux x64 cloud container (no Windows machine available here).
- Installed PRD suite (`prd/`), updated `CLAUDE.md`, `docs/INDEX.md`, `docs/FEATURE_PARITY_MATRIX.md`, `docs/HARDWARE_SUPPORT.md`, `.claude/rules/data-rights-and-experiments.md`.
- Decisions D-001…D-005 recorded as provisional in `docs/DECISIONS.md` (Tauri 2 + TS UI; PDFium chromium/7881 via pdfium-render 0.9.4; workspace layout; synthetic fixtures; runtime-only credentials). DW-001 still open.
- Added `.gitignore` (PRD 16 exclusions), `resources/pdfium.lock.json`, `tools/fetch_pdfium.py`, `tools/fixtures/gen_fixtures.py`, `fixtures/pdf/*`.
- Commands/results: `python3 -I tools/fetch_pdfium.py` → installed (exit 0); rerun → already_installed (exit 0); tampered archive → checksum mismatch (exit 4), nothing installed; network download → installed (exit 0). Fixtures: 6 valid PDFs parsed with `pypdf` strict mode (page counts/sizes/text as expected).
- Next: `crates/pw-core` foundation with unit tests.

## 2026-10-16 — Session 1, checkpoint 2–3: core, engine adapter, CLI
- Requirement IDs: CORE-RANGE-001, CORE-OUT-001, CORE-JOB-001, PDF-OPEN/INSPECT/SEARCH/TEXT/VIEW-001, PDF-ORG-*-001, CLI-001 (see `docs/FEATURE_PARITY_MATRIX.md`).
- Added `crates/pw-core`, `crates/pw-pdf`, `crates/pw-cli`, encrypted fixture (`tools/fixtures/gen_encrypted.py`), `THIRD_PARTY_NOTICES.md`, `third_party/licenses/pdfium/`.
- Commands/results (Linux x64 container, rustc 1.97.0): `cargo test --workspace` → 71 passed (core 43, engine 19, CLI 9), 0 failed; engine/job tests repeated 5–15× with no flakes; `cargo clippy --workspace --all-targets -D warnings` clean; `cargo build --release -p pw-cli` ok.
- Defects found by tests and fixed: (1) unencrypted PDFs reported as encrypted (security-handler mapping); (2) first bookmark dropped (`iter_siblings` skips self); (3) outputs from encrypted inputs failed verification — now reopened with the source password; extract/delete/merge/split outputs lose protection (measured) and now carry an explicit warning.
- Security fix from review: release builds no longer probe a compiled-in build path or env var for `pdfium.dll` (DLL hijack); verified release binary contains no `third_party` path and returns `missing_resource` (exit 10) without the DLL.
- NOT RUN: read-only destination test (container runs as root); anything on Windows; desktop UI; performance baselines.
- Next: Tauri desktop app slice (open/view/thumbnails/zoom/search/page tools/settings), then CI with a Windows job.

## 2026-10-16 — Session 1, checkpoints 4–6: desktop app, real-app E2E, CI
- Commits: `5cdf5a4` (app slice), `4633f35` (E2E green), `5660643` (audit + coverage).
- Added `app/` (Tauri 2 host + React/TS UI), `app/e2e/` (dependency-free WebDriver client + 25 scenarios run against the built binary via tauri-driver/WebKitWebDriver under Xvfb), `app/scripts/audit-controls.mjs` → `docs/INTERACTION_INVENTORY.md`, `.github/workflows/ci.yml`, `deny.toml`.
- Results (Linux container): Rust tests 80 passed; UI unit 16 passed; real-app E2E 25/25 passed on 3+ consecutive runs; audit 29/29 commands exercised; cargo-deny advisories/bans/licenses/sources ok; gitleaks full history clean.
- CI run #1 (`5cdf5a4`): Windows job PASSED — workspace tests (incl. PDFium engine) on windows-latest and packaged CLI smoke test from a clean folder (Unicode/space path, independent pypdf check, conflict exit 8, missing-engine exit 10). Linux job failed at clippy (workflow lacked WebKitGTK; fixed). Later runs pending at time of writing.
- Defects found only by running the real app, all fixed with regression coverage: percent-encoded page URLs rejected (blank pages); initial zoom overflow; outputs created 0600; palette blocked its own commands; option toggles didn't re-search; Escape in Find only worked in the text field; aria-disabled controls not visually disabled; Ctrl+Shift+Tab ignored on WebKitGTK; nondeterministic active tab for multiple launch files.
- Test-tool quirk (not an app bug, documented in harness): WebKitWebDriver drops Shift on synthetic typing after a pointer action; `type()` now verifies field contents.
- NOT RUN / gaps: Windows UI automation; screen-reader (Narrator/NVDA) review; performance baselines (startup, render, memory); drag-and-drop automation; non-US keyboard/IME; high-DPI/mixed-DPI; clean-VM install beyond CI runner; text selection layer not built.
- Next: confirm CI run #3 green (installer smoke on Windows); then PRD 12 watchdog; performance baseline harness; text selection/copy layer.

## 2026-10-16 — Session 1, checkpoints 7–9: watchdog, benchmark, text selection, Windows installer evidence
- Commits: `7db1bf3` (watchdog), `ac30855` (benchmark + `docs/PERFORMANCE_BASELINE.md`), `c209179` (text layer, rotation fix, bounded E2E).
- **Windows evidence (CI run #4, windows-latest, commit `1438e41`):** workspace tests passed; NSIS + MSI installers built; silent install; installed app launched with a PDF; privacy-safe log showed `startup` and `open … ok:true … pages:5` with no file name; silent uninstall removed the executable. Packaged CLI smoke test passed.
- Linux CI E2E step in run #4 ran >13 min (local: ~1.5 min) — suspected hang; WebDriver calls had no timeouts. All calls now bounded (30–60 s) and the suite capped at 15 min. Hypothesis to verify from run #5 logs: WebKitGTK sandbox blocked by Ubuntu 24.04 AppArmor user-namespace restriction.
- Watchdog (PRD 12): logic unit-tested (8); live dry run detected the real failing Linux job of run #1. **Blocked:** GitHub runs `schedule` only from the default branch — the 10-minute trigger cannot be proven until this branch is merged to `main` (owner decision).
- Defects fixed this checkpoint: PDFium page size is post-rotation → rotated pages laid out portrait and geometry off (engine now reports unrotated size; regression test fails without fix); rotated text runs over-stretched 16×; menu focus cleared page selection (snapshot + restore); shortcut capture focus race.
- Next: read CI run #5; fix Linux CI E2E; then text-selection keyboard support (Ctrl+A scope), annotations/print scoping decision, accessibility review plan.

## 2026-10-16 — Session 1, checkpoints 10–12: accessibility, bookmarks, license bundles, CI fully green
- Commits: `c3cdbaa` (axe + document list semantics + landmarks), `828d2f2` (bookmarks), `5bb456b` (license bundles), `21898ce` (E2E timing).
- **CI run #6 (commit `c3cdbaa`): all four jobs green** — Linux (fmt, clippy, Rust tests, UI tests, interaction audit, real-app E2E 28/28 with axe-core), Windows (tests, NSIS+MSI build, silent install → launch with PDF → log verified → uninstall), gitleaks, cargo-deny.
- CI E2E duration fixed: ~31 s per app launch on ubuntu-24.04 runners was a missing D-Bus session; running under `dbus-run-session` cut the step from ~14 min to 70 s for 28 scenarios.
- Accessibility: axe-core (test-only) reports 0 findings of any severity across 10 UI states (light/dark). Fixed nested interactive tabs → document list; landmarks/headings. Manual screen-reader review still NOT done.
- Licenses: generated bundles for all compiled Rust crates (cargo-about) and bundled UI packages, installed with the app; CI fails when stale.
- Local totals: Rust 80, UI unit 20, real-app E2E 29, watchdog 8; audit 31/31 commands exercised.

### Decisions needed from the owner (blocking further release progress)
1. **Merge to `main`** — required for the 10-minute watchdog schedule to run at all (GitHub only schedules from the default branch) and for release tagging.
2. **DW-001** — full document/spreadsheet authoring vs. existing-file editing + conversion via mature engines.
3. **Office conversion engine** — LibreOffice (MPL-2.0, ~300 MB runtime) is the realistic local option for DOCX/XLSX/PPTX/ODF; needs approval for size/licensing.
4. **Specialist model choices and rights** (OCR, translation, TTS, layout) — PRD 14/15 require license/provenance approval before any download or training; Kaggle use requires a rights-cleared dataset decision.
5. **Code signing** — Windows installers are unsigned (SmartScreen warnings); requires a certificate the owner must obtain.
6. **Credential hygiene** — the Kaggle/GitHub/Cloudflare tokens were shared in chat; PRD 16 requires rotation and runtime-only injection (environment secrets).

### Next engineering steps (no owner input needed)
- Annotations from selection (highlight/underline/strike/note) with save-to-new-file and E2E verification of the written annotations.
- Persisted job state + restart recovery; app startup and first-page timing in the real app.
- Windows UI automation (msedgedriver + tauri-driver on windows-latest) to run the E2E suite on Windows.
