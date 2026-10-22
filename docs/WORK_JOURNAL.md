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
- ~~Annotations from selection~~ — done in checkpoint 13.
- Persisted job state + restart recovery; app startup and first-page timing in the real app.
- Windows UI automation (msedgedriver + tauri-driver on windows-latest) to run the E2E suite on Windows.

## 2026-10-16 — Session 1, checkpoint 13: annotations end to end, unsaved-changes protection
- Engine (`2dc0f6b`, `7cff5e0`): typed `annotate` operation; output reopened and verified; lopdf + rendered-pixel tests.
- App: Highlight / Underline / Strikethrough / Add note from the page context menu (selection snapshot taken before the menu steals focus), shortcuts Ctrl+Shift+H / Ctrl+Shift+U, Ctrl+Z / Ctrl+Y / Ctrl+Shift+Z, Ctrl+S "Save annotated copy…" (default `<name>-annotated.pdf`), status-bar unsaved count (button), Discard with Undo in the notice.
- Selection → PDF: only selected text-node fragments are measured (never element boxes, so Ctrl+A cannot become a whole-page highlight); clipped per page; zoom derived from the measured page box; merged per line in PDF space; inverse mapping unit-tested for 0/90/180/270°.
- Unsaved protection: tab close prompt (Save copy… focused) and a host-side window close guard — the Rust `CloseRequested` handler holds the close only while the UI reports unsaved annotations, so a hung UI cannot block a clean close.
- E2E: 33/33 locally (new: annotate + verified saved file, tab-close prompt, real `WM_DELETE_WINDOW` close via `app/e2e/wm_close.py`: clean close, Cancel, Discard, Save-then-quit). WebDriver's own close-window bypasses the OS close request, so it cannot test this.
- Harness findings: WebKitWebDriver loses Shift for capitals in element send-keys after pointer actions — `type()` now falls back to explicit Shift key actions and still verifies the field. `tools/check.sh` silently passed a failing strict audit because `set -e` ignores failures left of `&&`; fixed (each check is its own command).
- Totals: Rust 83, UI unit 32, E2E 33, audit 39/39.
- Next: CI result for this push (incl. first `windows-e2e` run); persisted job state/recovery; startup/first-page timing.

## 2026-10-17 — Session 1, checkpoints 14–15: startup baseline, restart recovery
- `477469c` (local, pushed with this checkpoint): host-recorded startup marks; release build on the Linux container: UI ready 822 ms p50, first page 1043 ms p50 (`docs/PERFORMANCE_BASELINE.md`). E2E retries a failed app *launch* once (CI run #10 lost one launch to GTK failing to reach Xvfb) and reports every retry.
- Restart recovery (`pw_core::journal`): entries for running file-writing jobs, staged files tagged with the owning pid, per-instance locks so a second live instance is never "recovered"; startup notice with Run again. MSRV raised to Rust 1.89 for std file locks.
- Bug found and fixed before commit: the annotate-save completion cleared the active document's pending annotations for *any* annotate job (a recovered Run again would have wiped unrelated unsaved work); now scoped to the save dialog.
- Test-harness fix: `axeCheck` could sample a just-mounted notice mid-fade (contrast 3.66 at 50% opacity); it now waits two frames and requires two settled checks, and reports axe's measured colours.
- CI run #11 (`91a7c48`): Linux (incl. E2E 33/33), Windows tests + installer smoke, gitleaks, cargo-deny green; first Windows E2E still running at the time of writing (>8 min vs ~90 s locally) — read its log next.
- Local totals: Rust 90, UI unit 32, E2E 35/35, audit 39/39.

## 2026-10-17 — Session 1, checkpoints 16–18: forms, Windows E2E diagnosis
- Forms (`ffd5549`, `e2e6462`, this checkpoint): engine reads/fills AcroForms through PDFium's form filler (decision D-006: one audited FFI module, `unsafe_code` denied elsewhere, enforced by a test; engine-wide shared/exclusive lock with a lock-order guard). Typed ops `form_fields`, `fill_form`, `apply_edits`; CLI `pageweave fields`. App: native controls over widgets, edits counted as unsaved changes with annotations, Ctrl+S saves everything into one verified copy and the tab switches to it.
- Windows E2E (`5130c46`, `0ac5e86`): every WebDriver session timed out. Diagnostic on the runner: the *debug* build starts but its web UI never loads (no ui_ready, no WebView2 DevTools) while the installed *release* build reaches ui_ready in 5.4 s and renders the first page. Windows E2E now runs the release build. **Open investigation:** why the debug build's UI does not load on the Windows runner.
- CI toolchain moved to Rust 1.99 (new clippy lint caught in tests); local toolchain updated to match.
- Local totals: Rust 101, UI unit 37, E2E 36/36, audit 39/39.

## 2026-10-17 — Session 1, checkpoint 19: organize pages; rotated-page text bug
- Organize pages (Ctrl+Shift+O, Pages menu, page context menu): reorder, duplicate, rotate, remove, insert pages from another file (opened as a preview without touching recent files), undo/redo, drag-and-drop; saved through the typed `organize_pages` op in one verified write.
- **Bug found by the new E2E's independent check:** text extraction (`extract_text`, CLI `text`) returned nothing for any 90°/270° page — pdfium-render's `PdfPageText::all()` bounds by the rotated page size. Files were never affected (pypdf extracts the text); PageWeave's own extraction was. Fixed and covered by a regression test that fails without the fix.
- axe caught two defects in the first version of the organize tiles (visible label not in the accessible name; contrast on the selected tile); fixed before commit.
- Local totals: E2E 37/37, UI unit 46, audit 40/40.
- Windows E2E root cause found (`4cebdae` diagnostic): the WebView2 browser process command line contained only wry's flags — the `--remote-debugging-port` from `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS` never arrived (app-supplied arguments win on runtime 154), no Edge policies present. Fix per decision D-007; awaiting the Windows CI run to confirm.

## 2026-10-17 — Session 1, checkpoint 20: stamps; engine concurrency bug fixed
- Page numbers / headers / footers / Bates numbering (`stamp_text`): placed upright in the displayed orientation (verified by rendering a 90° page).
- **Bug (found via a Windows CI test failure, reproduced locally 4/20):** concurrent PDFium use could misreport errors — PDFium's last error is process-global and read in a separate call, so a corrupt file came back as "password protected". Engine use is now serialized per operation (reentrant lock; D-006 amendment). Regression test fails 7/10 without the lock, 0/10 with it; full engine suite 0/30 failures under 8 threads.
- Windows E2E with D-007 (`b4ba010`) is running scenarios at the time of writing (no longer failing fast) — read results next.
- Local totals: E2E 38/38.

## 2026-10-17 — Session 1, checkpoint 21: Windows UI automation unblocked (pending confirmation)
- Chain of root causes, each proven by a CI diagnostic before fixing:
  1. Debug build's web UI never loaded on the runner → E2E switched to the release (shipping) build.
  2. WebView2 ignored `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS` because wry always sets its own arguments → D-007; DevTools reachable, sessions attach, scenarios without files pass.
  3. Edge WebDriver passes app arguments as Chromium switches (`--d:\…\file.pdf`) → app accepts `--open=<path>`; Windows E2E uses it.
- Also fixed on the way: UI tab recovery after a page reload (`42ce9e3`); per-operation engine lock (concurrency bug found by Windows CI); organize copies page runs in one call (95 ms typical for 1,000 pages).
- Open: debug-build UI not loading on the Windows runner (does not affect shipping builds); confirm the full Windows E2E pass on `5554072`.

## 2026-10-17 — Session 1, checkpoint 22: Windows E2E 37/40; startup-file hardening
- Windows E2E on `7be5447`: 37 passed, 1 failed, 2 skipped (window-close scenarios had no Windows close request yet; `77d0673` adds WM_CLOSE).
- The failure: second launch of the settings scenario showed the saved dark theme but no document tab, and the next launch took 19 s (others ~1 s). No app log was captured, so the cause is not proven. Changes made from what the code allowed:
  - startup files were consumed on the first `launch_files` call and any IPC failure was swallowed (`catch(() => undefined)`) — a silent failure. Files now stay pending until the UI reports each handled (`launch_file_done`), so a reload part-way re-offers the rest; failures show an error notice; the startup log records the number of files received.
  - E2E failure capture now saves the host log since the failing launch (`FAIL-*.log.jsonl`) so the next occurrence can be diagnosed.
- Found in the failure screenshot: recent files on Windows showed `\\?\D:\…` paths (std canonicalize). Paths now use `dunce::canonicalize` (plain form where equivalent; unchanged elsewhere).
- Local: gate passes; Linux E2E 40/40. Next: read the Windows run for this push.

## 2026-10-17 — Session 1, checkpoint 23: permanent redaction (engine, CLI, app)
- Windows real-app E2E fully green on `4b97eeb` (40/40, incl. real WM_CLOSE window-close scenarios). Second Windows startup baseline recorded.
- Redaction engine (`41652d6`, decision D-008): content removed under each area (text per character, image pixels repainted as new image data, drawings/widgets/annotations inside), black boxes painted, output rebuilt and rewritten with lopdf (no unused resources, thumbnails, metadata, field data, revisions). Verified before commit; failure writes nothing. Negative tests prove the checks reject leaky output.
- Found while building it: the first per-pixel check flagged half-painted image pixels at an area edge (each box now grows to whole image pixels); single-character re-placement broke text extraction order on a rotated page (kept text is now written as runs, falling back to characters, then to an image page, per page); merged field/widget dictionaries kept field names and values in the copy (field keys are now stripped from kept widgets).
- **Pre-existing bug fixed:** the app placed the text layer, search highlights, annotations and form controls relative to (0,0), but pages whose CropBox/MediaBox starts elsewhere use offset user-space coordinates — overlays were shifted on such pages and annotations would have been saved in the wrong place. The engine now reports the page box origin; all UI mappings use it (unit tests for every rotation; no E2E fixture yet for an offset page in the viewer).
- App: mark selected text (Ctrl+Shift+X / context menu), every search match (find bar), dragged areas (mark mode, Esc ends it) or a whole page; marks share undo/redo with annotations, can be removed from their context menu (with Undo); review dialog lists what each area removes; applying opens the redacted copy. Marks are never saved as annotations (typed separately; unit test); the unsaved-changes prompt says marks are not saved.
- CI run on `41652d6`: Linux E2E failed 1 scenario ("page reload… tab order kept ()"): the test counted the old page's tabs before `location.reload()` navigated. Test now marks the old page and waits for the new one. Not an app defect (already-open files are not duplicated or reordered).
- Also fixed: dark-theme danger button text contrast 2.3:1 (axe, found by the new scenario) → `--on-danger` token; page context-menu order kept stable for keyboard users (redaction group after page operations).
- Local: gate passes (audit 47/47 commands), UI unit 56, Linux E2E 41/41. Next: confirm on CI (Windows E2E runs the redaction scenario for the first time); then a real-world-shaped redaction corpus (compressed streams, Type0/CID fonts, inline images, clipped/rotated images).
- CI on `661c74d`: all jobs green; real-app E2E 41/41 on Linux and on Windows (redaction scenario passes on WebView2).

## 2026-10-17 — Session 1, checkpoint 24: ML workspace and OCR engine decision
- User asked to keep building and to train the required models on Kaggle. Kaggle credentials are not in this environment (CLI installed, kaggle.com reachable); asked for them as an environment secret. Training is gated by PRD 14 (benchmark first; fine-tune only on a measured gap with rights-cleared data).
- `ml/` workspace (`2e88ab2`): synthetic rights-clean corpus, scorer (verified: perfect engine = 0 on all conditions), benchmark harness, rights gate, budgeted Kaggle runner with env-probe kernel; 15 tests in CI (green, corpus reproducible on the CI machine).
- OCR benchmark (`docs/benchmarks/OCR_BASELINE.md`): PP-OCRv6 small 0.41% CER vs Tesseract 3.75% on the 72-page blind set → decision D-009 (PP-OCRv6 on ONNX Runtime).
- Bug in my own scorer found before trusting results: axis-aligned boxes on skewed pages were rejected (Tesseract skew CER showed 159%); fixed (centre matching) with a regression test.
- Process correction: a detector setting was first compared on blind pages; redone on validation, where the apparent 40% gain did not reproduce. Rule now: tune on val, report blind once.
- Fine-tuning not yet justified on synthetic numbers; measured gap = word spacing and Latin-script confusions. Next: OCR integration in Rust (task 15); script constraint and spacing post-processing measured on val; Kaggle env probe as soon as credentials exist.

## 2026-10-17 — Session 1, checkpoint 25: OCR in the product
- `pw-ocr` (Rust, ONNX Runtime 1.31 loaded at runtime, official PP-OCRv6 small models, pinned + verified; telemetry off). Parity with the Python reference on the same scorer: blind 0.41% CER both, 1.6 vs 3.1 s/page. First version lost on bad scans (bilinear crops) — fixed with bicubic sampling.
- Official model files proven identical in behaviour to the benchmarked copies; official `inference.yml` settings measured worse (1.32% val CER) and not used.
- `ocr_pages` operation (render with brief engine lock → recognise without it → write + verify: page images pixel-identical, every line extractable), CLI `ocr`, app "Recognize text (OCR)…" with progress/cancel, E2E 42/42.
- Bugs found while building: (1) verification reused `PdfPageText::all()`, which drops text on /Rotate 90 pages — the checkpoint-19 bug in a new place; now `inside_rect(everything)`. (2) PDFium does not break lines of 90°-turned text, so line-end and line-start words fused ("nextacross") and whole-word search failed; each line now ends with a space (test asserts whole-word hits). (3) My OCR resource lookup honoured an env override and dev path in release builds (DLL-hijack risk); now debug-only, like PDFium.
- Packaging: installers and the CLI zip ship `ocr/` + ONNX Runtime/model licenses; CI fetches resources so OCR tests run (fail, not skip, under CI); Windows smoke runs OCR from the package with an independent pypdf check and a missing-model exit-code check.
- Kaggle still blocked on credentials. Next: CI confirmation (first Windows OCR run), Windows timing, then the fine-tuning question with a real-world rights-cleared evaluation set.

## 2026-10-17 — Session 1, checkpoint 26: Windows OCR confirmed; redaction hardening; password protection
- CI `5ce9979` all green: OCR engine tests ran on Windows; packaged CLI ran OCR from a clean folder (pypdf independent check) and returned exit 10 with a model removed; installer ships `ocr/`; Windows E2E 42/42 (OCR scenario 15.6 s for 3 pages, release build).
- Found by that CI: Windows checkouts converted the pinned dictionary to CRLF → hash refusal (correct behaviour). Fixed with `.gitattributes` (`-text` for pinned text resources).
- Redaction: real-world-shaped fixture (Flate, TJ-split, Tz, rotated text, inline image, nested forms, secret only in an earlier incremental revision) — all removed, verified in every decompressed object. Open: Type0/CID fonts.
- Fine-tuning: no official training weights for PP-OCRv6 small (empty link in PaddleOCR's model list); options recorded. Script-confusion filter not added (no evidence on validation).
- Password protection (PRD 05): AES-256 via lopdf, verified by PDFium (refuses without password; both passwords open; text intact) and lopdf (V5/R6, exact permission bits, decrypts). Bug found by verification: lopdf's decryption of the RC4 fixture produced a malformed file; protected sources are now decrypted by PDFium (page copy). Passwords travel only in non-serialized request fields (journal test extended). CLI reads them from stdin only. Linux E2E 43/43.

## 2026-10-17 — Session 1, checkpoint 27: sanitization (remove hidden information)
- CI `ebcb78a` (password protection) all five jobs green, Windows E2E included.
- Sanitize (PRD 05): inspect + remove per category (metadata, scripts/actions, attachments, hidden layers, comments; form fields and web links opt-in); earlier versions always removed (lopdf rewrite, `Prev` dropped, pruned).
- Finding: lopdf's content decode/encode cannot round-trip inline images (re-emits them as stream objects; skips ones it cannot parse). Hidden-layer removal therefore edits content with a new byte-preserving tokenizer (`content_ops.rs`) that drops/replaces whole operations and copies every other byte.
- Hidden content: marked sections of OFF groups (OCG/OCMD incl. /P policies and /VE) lose their drawing operators (graphics state kept, painting → `n`), hidden XObjects/annotations are deleted, visible `/OC … BDC` become `/OC BMC`, XObjects no longer drawn drop out of each stream's resources (so images only drawn in hidden layers leave the file).
- Verification before commit: independent whole-file walk per category + lopdf-decoded content has no `/OC` marks; every page renders identically to the original (PDFium, annotations off). Unit tests show both checks fire (each category left behind; layer revealed instead of removed; page changed).
- E2E flake fixed before commit: polling helper threw while the dialog mounted (palette closes first) → null-safe query. Screenshot showed fieldset min-content overflow → fixed (`min-width: 0`, truncation with tooltip).
- Full-suite E2E found a real UX bug: notices (toasts) were layered above modal dialogs, so two XFA notices covered and intercepted the dialog's Remove button. Notices now sit beneath the modal layer (rule added to INTERACTION_CONTRACT). Linux E2E 44/44 after the fix.
- Next: "Remove password" (PDFium save with FPDF_REMOVE_SECURITY needs raw bindings; pdfium-render hardcodes flags 0) so protected files can be cleaned; Type0/CID redaction fixture.

## 2026-10-17 — Session 1, checkpoint 28: remove password; owner restrictions honoured
- "Remove password" (PRD 05): PDFium `FPDF_SaveAsCopy` with `FPDF_REMOVE_SECURITY` (pdfium-render hardcodes flags 0, so it goes through the existing direct-call module). Keeps bookmarks, properties and forms. Verified: opens without a password, same pages/text, lopdf finds no /Encrypt.
- Re-protecting a protected file now uses the same decryption, so bookmarks/properties are kept (previous limitation removed).
- D-010: changing protection requires full rights (no restrictions, or opened with the permissions password). Open question recorded: page operations on restricted files.
- Sanitize's refusal of protected files now points to Remove password.
- E2E: withApp waits for a tab, so a protected file is launched alongside an unprotected one.
- CI `2bf5632`: Windows E2E failed in the new remove-password scenario (Ctrl+K pressed while the protected file was still opening; the first tab already showed a page) → waits for both tabs now. cargo-deny failed on advisories published today (RUSTSEC-2026-0370/0371/0372: zvariant, zbus_names, zvariant_derive unmaintained, merged into zbus 6) reached only via tauri-plugin-opener's Linux D-Bus support; not in the Windows build; no stable upgrade (only opener 3.0.0-alpha). Narrow, documented ignores added to deny.toml with a revisit condition.

## 2026-10-17 — Session 1, checkpoint 29: document properties; Kaggle pipeline live
- Edit document properties (PRD 05 metadata): lopdf rewrite of the Info dictionary, XMP regenerated with the same values (PDF/A id kept), verified by PDFium read-back and render equality. Signed files now warn that signatures no longer hold (also sanitize); signature-structure fixture added.
- Kaggle (user supplied a token in chat and explicitly asked for it to be used; stored only in ~/.kaggle/access_token, 0600, never in the repo; rotation recommended): preflight ok; private dataset kuyamblaisetimchia/pageweave-ocr-corpus-v1 (720 synthetic train pages, 364 MB); probe: 2x Tesla T4 15 GB, 4 CPU, 34 GB RAM, 21 GB disk, torch 2.11+cu128, no paddle/onnxruntime, no internet. First probe ran before the dataset finished processing (no inputs) → re-run queued.
- Fine-tune design: recognizer ONNX → PyTorch via onnx2torch (SAME_UPPER pads made explicit; fidelity 8e-6 vs ONNX Runtime); trained values written back into the ORIGINAL ONNX initializers (all 151 float weights map 1:1) because torch re-export bakes the traced width (0.13 error at other widths).
- Where validation errors are: line recall 1.0; errors are recognition on degraded scans (badscan 0.96% CER, 68% exact lines; faded/noise/jpeg next). Training pairs come from running the real detector on train pages and keeping one-to-one, ≥90%-coverage matches (85% of lines).
- Watermark (PRD 05): PDFium text object with alpha, centred on the page box, angle as displayed (page /Rotate added), over or behind (insert at index 0). Verified per chosen page by text extraction and a render diff; pages where it cannot be seen (e.g. behind a scan) are reported, not hidden. axe flagged the faint HTML preview text → preview is an SVG with an accessible description.
- Disk allowance ran out during the gate (target/debug 25 GB + ML crops); freed incremental/release caches. Crop build rerun.
- Crop pages (PRD 05 page boxes): CropBox from the visible box with margins mapped through /Rotate (displayed top on a /Rotate 90 page is the page's left side); verified exactly per page; warns that cropping hides rather than removes.
- Attachments (PRD 05): list / save one as stored / attach into a copy (PDFium embedded files; name tree is kept sorted so lookups go by name). Found while testing: the sidebar's third tab overflowed the 200 px sidebar (click intercepted) → icon tabs with accessible names; success notices offered "Open" for non-PDF outputs (exported images, saved attachments) → Open only for PDFs.
- Kaggle: the GPU queue held the probe re-run ~2 h; a CPU probe (supersede option added) ran at once and verified the dataset mount (720/720 image hashes). Training run pushed (T4, 3 GPU-h ceiling) — queued.
- Edit bookmarks (PRD 05): outline rebuilt with lopdf (/Fit destinations), verified by PDFium read-back; editing logic in a pure module (subtree-aware indent/outdent/move/delete).
- Organize: blank pages (PageRef.blank) sized and turned like the page they follow; verified by engine and component tests (no separate E2E: the organize E2E covers the dialog's keyboard flow and save path).
- Images to PDF (PRD 06): JPEG passthrough (inline DCT; bytes verified identical), PNG decode, EXIF orientation via page /Rotate, own JPEG/PNG header parsers before PDFium; fixtures from tools/fixtures/gen_images.py (Pillow, committed).

- Export as text (PRD 06): `export_text` op (UTF-8, CRLF, page headings, form feed separators; atomic write verified by byte read-back; OCR hint when pages have no text); CLI `text -o`; Pages ▸ Export as text…. Found while adding it: the CLI capability list was hand-written and had drifted (set_bookmarks missing) → `pw_core::ops::OPERATION_IDS`, kept in step with `Operation::id` by a unit test.
- Reduce file size (PRD 20 phase 2 "compression"), lossless slice: `optimize` op. Plain and weakly Flate-compressed streams are recompressed only after a strict decode (lopdf's Flate decoder returns partial output on corrupt data, so it is not used for this) and a byte-for-byte round trip; image codecs, predictors and XMP are left alone; identical streams/fonts/ExtGStates stored once (pages, annotations and fields keep their identity); unused objects and earlier versions dropped; object + xref streams. Verified per page with annotations and form data rendered, text, annotation counts, bookmarks and attachments; a copy that is not smaller is refused with a reason. Fixture `uncompressed-report.pdf` (16 KB → 2.2 KB). 500 pages: 2.3 s, 23 MB peak (release, Linux container).
- Found while adding it: menu item React keys were the command id, and the tab menu has two items with `file.close` → keys now include the position.
- Next slice: image downsampling/recompression (lossy, opt-in, with an effective-DPI target), which is where most real-world size is.
- Image resolution for Reduce file size (opt-in, lossy): placements from content (cm, form /Matrix; largest use), conservative "unknown use" taint (annotations, patterns, Type 3, unreadable content), supported layouts only, unshared soft masks resampled along. Tolerances set from measurements on the new photo-heavy.pdf fixture (tools/fixtures/gen_photo_pdf.py): per-image JPEG error ≤20 (measured ≤8.6 at 36 dpi/q30), changed pages mean ≤4 and ≤1% strongly different pixels (measured ≤1.0 / 0.03%); quality floor raised to 30 after q10 measured 13.5. Fixture fix: Pillow's CMYK JPEG needs /Decode [1 0 …] in PDF (rendered almost black without it).
- Compare documents (PRD 01 G / 05), engine + CLI slice: aligned pages, Myers word diff (similar 2.7), moved passages/pages, format changes, visual regions, page size, metadata. First run on the new compare-old/new fixture pair found every planted change but also reported "visual" changes where unchanged words had shifted after an edit → reflowed words (equal text, moved >1 pt) and old-side change boxes now explain pixel differences. 500 vs 499 pages: 3.1–3.4 s, 24 MB (release). Next: app compare view (side by side, difference list, jump).
- Compare view (app): chooser (open tab or file by path/Browse, Swap) → full-window result with difference list and side-by-side highlighted pages. Found while testing: (1) after results replaced the chooser, focus was on a removed button, so Escape did nothing → focus moves to the list when results arrive; (2) muted page labels on the selected item failed axe contrast; (3) a coloured left border on rounded list items left coloured specks at the right corners in WebKitGTK → inset box-shadow stripe; (4) E2E screenshots could return a stale frame (showed the chooser after results were asserted) → the screenshot helper waits two animation frames.
- Signature verification (launch checklist "signature trust statuses honest"): integrity + signer signature, never trust (D-011). cargo-deny flagged RUSTSEC-2023-0071 in `rsa`; project policy forbids ignoring vulnerabilities → RSA PKCS#1 v1.5 verification implemented with a public-key modpow instead. Signed fixtures generated with Python cryptography and cross-checked with openssl before writing the verifier. UI: status bar summary + Signatures sidebar tab.
- Disk allowance filled again (target 19 GB): removed target/release and incremental caches.
- Print (PRD 01 G / 05): prepared page images with per-size @page rules → platform print UI (no native FFI). Ctrl+P is prevented everywhere so WebView2's built-in print can never print the app's screens. Real printers can't be exercised in CI: E2E stubs window.print and inspects the prepared sheets; on Windows the W3C print-to-PDF endpoint additionally renders print media (WebKitWebDriver answers 404, so Linux checks the DOM only).
- Straighten pages (PRD 06 scan cleanup): skew via projection profiles, lossless correction by a content transform, verified by re-measuring. Found while building: (1) an Otsu off-by-one (dark = ≤ threshold) made two-tone pages unmeasurable; (2) gen_scanned.py documented page 2 of scanned.pdf as "skewed" but it is a faded page → comment fixed, new skewed-scan.pdf fixture; (3) straightening a straightened copy re-measured ±0.1° and turned it again → straight threshold 0.1° → 0.2° plus an idempotence test. Benchmarks in docs/benchmarks/DESKEW.md (blind scored once, before the threshold change; stated).
- Remove blank pages: read-only detection + preview, removal through delete_pages. Found while calibrating on the fixture: faint grey table lines (0.82 grey) fell under the first ink delta (48) and the page was called blank → delta 32; the "lenient" preset was 1% and still swallowed that page (0.9% ink) → 0.5%.
- **Kaggle overrun (2026-10-18):** run `20261017T194302Z-rec-finetune` (ceiling 3 GPU h) was still RUNNING after 10 h 12 m; account GPU use 9 h 22 m of 30 h this week. Root cause: the script's 2.4 h budget was checked only inside the training-step loop, so any stall outside it (load, evaluate, export) was unbounded, and outputs/SHA256SUMS were written only at the very end. The local waiter died with a worker restart, so nothing noticed. It could not be cancelled from here: the API's CancelKernelSession needs a session id that no status/metadata/push response returns; deleting the kernel was not done (destructive, user's decision). Kaggle ends it at its 12 h session limit (~07:43 UTC). Fix: hard wall-clock watchdog for the whole process (PW_HARD_DEADLINE_S, default 2.75 h) that writes a final checkpoint and exits; metrics.json + SHA256SUMS checkpointed after the baseline and every epoch; atomic exports; `pull` records the kernel's own end status (`result_status`). Verified locally: normal run → "complete", 25 s deadline → "stopped_at_deadline" with verified sums. Regression tests in ml/tests/test_rights_kaggle.py. Next: once Kaggle reports it ended, try `pull` (likely no output) and re-run with the fixed kernel only within the ceiling.
- Overrun resolved 2026-10-18 06:12 UTC: on the owner's instruction the Kaggle notebook `pageweave-rec-finetune` was deleted (its code was saved first and matches `ml/kaggle/rec-finetune/`). GPU time used then read 0:01:15 and stayed flat over 4 minutes, so nothing is running. Before the delete it read 9:22:30; Kaggle doesn't say why it dropped (it may stop counting a deleted notebook's time), so no refund is assumed. The run produced no output; the fine-tune experiment must be re-run with the fixed script, which needs a new notebook (same slug).
- Windows E2E intermittent "first tab never appears" (0045bf5 remove-password, 7f51650 forms): failure capture showed an empty <body> without #root and no Tauri bridge — the driver was attached to a blank document that never navigated to the app (slow runner, app start ~2x usual). Not app logic. Harness now waits for the app document itself (#root + bridge) before a session counts as launched; a blank document is a launch failure, retried once and listed in the summary ("app launch retries"); a second one fails. If retries keep appearing on Windows, investigate WebView2 initial navigation in the host.
- Windows CI (fb26548): every CLI test failed — "thread main has overflowed its stack" (Windows main thread: 1 MB; debug build argument parsing outgrew it as commands were added; Linux 8 MB hid it). Fix: the CLI runs on a 16 MB worker thread on every platform; regression test runs it under `ulimit -s 1024` on Unix (fails on the old code with the same overflow).
- Edit existing text and pictures (PRD 05 "edit text/images"), Edit mode: engine `objects.rs` (listing grouped into lines by font/size/baseline; in-memory apply, then the saved bytes are reopened and checked), typed ops `list_page_objects` / `edit_objects` and `apply_edits.object_edits`, host preview (exact render of pending edits), CLI `objects` / `edit`, app Edit mode (Ctrl+E). Found while building: (1) PDFium `set_text` changes are lost unless `regenerate_content` runs on the same page handle; (2) grouping by ink bounds split lines at descenders → baseline from the text matrix; (3) the first real-app run of Edit mode failed to save: the shared contract validation still said "no changes" for a save holding only content edits (engine tests had called `execute` without `validate`) → validation shared with `edit_objects`, regression test validates first; (4) the control audit requires a literal onClick: box activation moved from pointer-up to onClick (native Enter/Space) and a drag suppresses the click that follows it. Replace picture opens the native picker, so it is engine-tested only. Next: remaining backlog (task #28).

## 2026-10-21 — Session 2, checkpoint 1: form data export and import (PRD 05 forms)
- Branch `ccr-9d40a15a-w5penl`, from `8d6e9d8`. Fresh container: PDFium/OCR fetched (pinned, verified), WebKitGTK + tauri-driver installed; baseline Rust suite green before changes.
- Requirement: PRD 05 "import/export supported field data … Imported field data is untrusted; preview mapping." Row PDF-FORMDATA-001.
- pw-core `formdata`: XFDF writer (nested `<field>` tree, XML escaping, CR kept as `&#13;`, control characters XML cannot hold left out with a warning) and reader (quick-xml 0.42, already in the graph via Tauri; DOCTYPE refused, only predefined entities and character references, depth/size/count/name limits, fields counted only inside `<fields>`), JSON writer/reader (PageWeave format v1 or a plain name → value object). FDF detected and refused with a message.
- pw-pdf `formdata`: matching per field kind (checkbox export value / Off / yes-no words, radio options, combo/list choices, read-only, unknown, duplicates, several values) with a reason for every unused entry; ops `export_form_data` (unsaved values overlaid; file read back and compared before commit), `read_form_data` (read-only preview), `import_form_data` (fills only changed values through the existing verified fill path). CLI `form-export`, `form-import --preview | -o`.
- **Bug found by the new engine test:** importing with the output path set to the data file replaced the data file — the fill path only protected the PDF. Fill now takes the full protected list (`apply_edits_protecting`); regression test.
- **Pre-existing bug found by the new E2E screenshot:** a list box whose value is not one of its options (e.g. after Undo back to "no choice") displayed the first option as selected (React's select fallback) — the form showed a choice the document does not have. Hidden placeholder option, like the combo box already had; E2E asserts it.
- Also fixed before commit: import preview listed fields in file order (JSON objects arrive sorted) instead of form order.
- App: File ▸ Export form data… / Import form data… (palette; tab menu when the document has a form; disabled with a reason without a fillable form, while fields load, or for XFA). Import shows fields to fill, "already the same", and a "Not used" disclosure with reasons; Fill puts values in as unsaved edits with Undo; Enter reads but never fills a stale preview.
- Fixture `form-nested.pdf` (generator; pypdf strict parses it; other fixtures byte-identical).
- Ran: `cargo fmt --check`, `cargo clippy -D warnings` clean; `cargo test --workspace` 306 passed, 0 failed; UI typecheck; vitest 21 files / 108 tests passed; interaction audit 73/73; license bundles `--check` ok; tool + ML tests ok; fixtures reproducible. Linux real-app E2E full suite: 66/67 — the failure was "interrupted task…" ending with WebKit "session deleted because of page crash or hang" (scenario does not touch forms); it then passed 3/3 alone. Form scenarios (old + new) pass.
- NOT RUN here: Windows E2E/packaging (CI), cargo-deny and gitleaks (not installed in this container; CI runs both), screen-reader review.
- Next: watch CI for this push (first Windows run of the form data scenario); then continue the backlog (form reset/clear, FDF reading if wanted, split by bookmarks/size).

## 2026-10-21 — Session 2, checkpoint 2: split by bookmarks / size; clear form
- CI run #68 on `8c09e97` (form data): all jobs green, including Windows E2E and installer smoke.
- Split (PRD 05 "split by ranges, size, bookmarks or N pages"): `SplitMode::Bookmarks { level }` (parts start at each bookmark of the level, sorted by page, one per distinct page, front matter joins the first part, names `<stem>_NN_<title>.pdf` with forbidden characters replaced) and `SplitMode::MaxSize { max_bytes }` (largest group that fits found by doubling + bisection on the actual saved bytes; the measured bytes are the ones committed; a page over the limit alone is its own part with a warning). CLI `--by-bookmarks LEVEL`, `--max-size 10MB` (binary units). App Split dialog: two more modes with live part count / level list.
- Measured (release CLI, Linux container, 500-page generated document): by size 20 KB → 10 parts in 0.27 s, 100 KB → 2 parts in 0.15 s, 11–12 MB peak; every 10 pages for comparison 0.11 s.
- First real-app split scenario added (there was none before): bookmarks and size, files checked on disk.
- Clear form fields (Edit, palette): text emptied, checkboxes unticked, editable combos emptied, as unsaved edits with Undo; non-editable choices and radios cannot be emptied by the fill engine, so they are kept and the notice says how many. Reset to /DV defaults not done.
- Ran: fmt, clippy clean; Rust 311 passed; vitest 111 passed; audit 74/74; E2E new scenarios pass (split, clear form).

## 2026-10-21 — Session 2, checkpoint 3: MCP server (PRD 17)
- New crate `pw-mcp`, exposed as `pageweave mcp --root DIR [--allow-write] [--workers N]` (decision D-014). Stdio JSON-RPC 2.0; initialize/ping/tools.list/tools.call; protocol 2025-06-18 (2025-03-26, 2024-11-05 accepted).
- 16 tools, each one existing typed `Operation` (no second implementation): 7 read-only, 9 that create new files (only listed and allowed with `--allow-write`). Not exposed: protected files, signing, redaction, sanitize, passwords, attachments, form-data files.
- Path policy: canonicalize (symlinks resolved), must be inside a root; outputs must not exist (dangling symlinks included), fixed extension, folder must exist inside a root; collision policy always `fail`. Unknown arguments rejected. Limits: 1 MB per message (over-long lines consumed and refused), 8 active calls. Progress notifications when a progress token is given; `notifications/cancelled` cancels the job (verified: no output committed); EOF cancels running jobs; finished jobs pruned.
- Tests: pw-mcp path policy (2, incl. symlink escapes on Unix); `pw-cli/tests/mcp.rs` (4, real server process, every advertised tool called and checked; OCR progress + cancel), run 4× without a failure.
- Ran: fmt, clippy clean; Rust 317 passed, 0 failed; license bundle regenerated (adds pw-mcp only).
- NOT done: local HTTP API, external AI provider orchestration, MCP on Windows verified by hand (CI runs the same tests on Windows).

## 2026-10-21 — Session 2, checkpoint 4: pattern search for redaction; MCP race fixed
- Resumed from the scheduled task at 14:27 UTC. CI #70 (MCP) green; #72 failed in `pw-cli --test mcp`: "The operation was lost." Root cause in my MCP code: every finished call pruned all finished jobs, so a quick job finishing before its own waiter collected it was removed. Fix: `JobManager::remove(id)` (only that terminal job), regression test with eight calls in flight (fails 2–3 of 3 with the old pruning, passes 5/5). The first version of that test also failed because the test client rejected out-of-order responses — a test-client bug, fixed (buffers by id). Pushed as `724b53d`.
- Pattern search (PRD 05 redaction; README limitation "no pattern search"): `find_patterns` op over PDFium characters with boxes; kinds e-mail, phone (7–15 digits, date/boundary filters), card (13–19 digits + Luhn), IBAN (ISO 13616 mod 97), US SSN (invalid area/group/serial excluded), custom regex (linear-time `regex`, size/DFA/nesting limits, errors explained); overlaps resolved by kind priority; boxes merged per line. Fixture `sensitive-info.pdf` (look-alikes + /Rotate 90 page). Engine test redacts the found boxes and re-checks the copy with lopdf and a second search.
- Gate: fmt, clippy clean; Rust 326 passed; vitest 113; audit 75/75; license check ok; Linux real-app E2E full suite 70/70 (disk allowance filled during the run; cleared build caches and rebuilt).
- CLI `patterns --kinds … --custom …`; MCP read-only `find_sensitive_text` (17 tools); app Redact ▸ Mark patterns for redaction… (review list, untick, Mark → existing marks/apply flow). E2E: find, untick SSN, mark, apply; CLI confirms the four values gone and the SSN, phone and Luhn-failing card kept.

## 2026-10-21 — Session 2, checkpoint 5: accessibility check (PRD 05)
- `check_accessibility` (read-only): tags (MarkInfo + StructTreeRoot), title (Info or XMP), DisplayDocTitle, /Lang, text on every page, embedded fonts (page + form XObject resources, subset tags stripped), structure walk with RoleMap (bounded 200k nodes / depth 256, cycle-safe): Figure alt/ActualText, Table → TH, headings; widgets → /TU (push buttons exempt); links → /Contents; /Tabs /S on interactive pages; bookmarks when > 20 pages; 4 manual items (reading order, alt-text quality, contrast, colour-only). Protected files are read through PDFium's decryption with their password.
- Fixtures `tagged-accessible.pdf` / `tagged-issues.pdf` from new `tools/fixtures/gen_tagged.py` (DejaVu Sans subset via fontTools in ml/.venv; byte-identical on re-run; pypdf strict parses both).
- CLI `accessibility`, MCP `check_accessibility` (18 tools), app File ▸ Check accessibility… (status words in text, not colour alone; page buttons; axe clean).
- Not a PDF/UA or WCAG verdict (stated in UI, CLI and MCP description). No tag editing/auto-fix yet.
- Ran: fmt, clippy clean; Rust 331 passed; vitest 114; audit 76/76; license check ok; new E2E scenario passes.
- CI run #73 (MCP fix) Linux E2E 68/69: "print" failed with "stale element reference" — the scenario's check returned the dialog element itself from a script, and the dialog was removed while the driver serialized it. Test-harness defect, not app behaviour (print code unchanged; scenario passes locally). Eight such checks now return booleans; rule noted on the harness `exec` helper.

## 2026-10-21 — Session 2, checkpoint 6: page labels
- `set_page_labels` (PRD 05 "label"): typed ranges → catalog /PageLabels number tree (lopdf, rewritten copy); verified per page against `pw_core::labels::label_for` as PDFium reads it, plus render equality; empty list removes. Validation: first range on page 1, increasing starts, prefix ≤ 50 chars without control characters, start 1–1,000,000, prefix-only ranges need a prefix.
- Ran: fmt, clippy clean; Rust 335 passed; vitest 117; audit 77/77; new E2E scenario passes.
- CLI `labels --range PAGE:STYLE[:FIRST[:PREFIX]]` / `--remove`; app Pages ▸ Page labels… (editable ranges, live preview, invalid ranges explained, Remove labels when present); thumbnails already showed labels. First screenshot showed unbalanced columns → fixed widths.
- **Pre-existing safety bug fixed:** saving text/picture edits together with form values or new annotations ran the last stage on a staged copy and protected only that copy, so typing the original's own path and choosing Replace overwrote the source (contrary to "originals are never changed"). The final write now protects the original too; regression test `combined_edits_never_replace_the_source_even_with_replace` failed before the fix.

## 2026-10-21 — Session 2, checkpoint 7: existing comments (list, delete, change text)
- `list_annotations` (read-only) and `apply_edits.annotation_changes` (stage 1 of the save, lopdf rewrite): delete (with its pop-up) or change the comment text of annotations already in the file, addressed by page + position + subtype (a stale listing is refused). Verified before use: lopdf re-read of touched pages (subtypes in order, new texts), PDFium annotation counts, render equality of page content.
- Save path restructured into stages (annotation changes → content edits → form values/new annotations), each staged next to the output and removed afterwards; the original protected at every stage.
- **Pre-existing bug fixed:** closing a tab asked first only for unsaved annotations and form edits; unsaved text/picture edits were closed without asking. Now every kind of unsaved change asks (E2E covers comment changes).
- CLI `annotations` (list, `--delete PAGE:INDEX:TYPE`, `--set-text PAGE:INDEX:TYPE=TEXT`); app Sidebar ▸ Comments.
- Ran: fmt, clippy clean; Rust 340 passed; vitest 121; audit 77/77; Linux real-app E2E full suite 73/73.

## 2026-10-21 — Session 2, checkpoint 8: batch processing (PRD 06)
- App File ▸ Process several files… (also on the start screen): one action over many PDFs, each file one existing typed job (host queue, bounded workers); names unique within the batch, collision "rename" so existing results are kept; per-file state and reason; Stop the rest cancels queued and running jobs; Close disabled while running.
- Ran: fmt, clippy clean; vitest 125; audit 78/78; new E2E scenario passes (Rust unchanged).
- First E2E assumption was wrong (the five-page fixture can be made smaller too) — the app was right; the scenario now checks a repeat run keeps both results, and a component test covers the failure display.

## 2026-10-21 — Session 2, checkpoint 9: resource audit (PRD 18, offline part)
- Locks now pin the extracted engine libraries (PDFium, ONNX Runtime) per platform as well as the archives; fetch scripts verify them (reinstall checked on Linux; Windows hashes computed from the pinned archives fetched with the same scripts).
- `pw_pdf::resources::resource_status` (PDF engine found by the engine's own search, OCR files via `pw_ocr::check_files`) → Ready / Missing / Damaged per capability; CLI `resources`; Settings ▸ Resources (no file or model names in the UI, asserted by E2E); Windows packaged-CLI smoke now checks `resources` and its exit 10 with a model removed.
- Ran: fmt, clippy clean; Rust 344 passed; vitest 125; license check ok; new E2E scenario passes.
- Not built: download / resume / repair inside the app. Decision needed (owner): PRD 18 asks for in-app downloads, while the README states the app makes no network requests; installers currently ship all resources and repair means reinstalling.

## 2026-10-21 — Session 2, checkpoint 10: robustness and crash containment (D-015)
- `crates/pw-pdf/tests/robustness.rs`: deterministic mutations (truncation, bit flips, splices, number/keyword tampering, zeroed blocks) of every small fixture through every read-only operation; no panic, no case over 20 s. `PW_FUZZ_CASES` / `PW_FUZZ_SEED` widen the run.
- The write-operation run found a PDFium CHECK abort (SIGTRAP) in redaction on a damaged page dictionary → kept as `fixtures/hostile/redact-generate-content-crash.pdf`. Not catchable in-process, so jobs now run in a worker process (D-015, `pw_pdf::worker`); a crash fails one job with `engine.crashed` and the app stays open.
- Tests: `crates/pw-cli/tests/worker.rs` (password via the secrets line + result; hostile file → `engine.crashed`, then the next job works; progress then cancel; mutated files through write operations, 2 cases per fixture by default — 40 cases ran locally in 129 s with 2 crashes contained, 0 hangs); `stage_owner_tests` for the staged-file prefix; new E2E scenario (redaction of the hostile file fails with the reason, the window and other documents keep working).
- Ran: fmt, clippy clean; `cargo test --workspace` all passed; vitest 125; audit 78/78; license check ok; Linux real-app E2E full suite 76/76.
- Still in-process: viewer rendering and direct read commands of the app (documented in D-015 and README).

## 2026-10-21 — Session 2, checkpoint 11: reset form to default values
- `form_fields` now reports each field's `/DV` (`default_value`), read with lopdf (`pw_pdf::formdefaults`: full names from the field tree, inherited defaults, cycle/depth/count limits, bounded decompression, encrypted files with the given password; unreadable → no defaults, never an error).
- App Edit ▸ Reset form to default values (palette): unsaved edits with Undo; fields without a default are emptied where possible, choices/radio groups without a usable default keep their value and the notice says how many. Saved like any other form edit (verified copy).
- New fixture `form-defaults.pdf` (same form as `form-fields.pdf` with /DV on four fields; generator reproducible).
- Ran: fmt, clippy clean; pw-pdf lib + forms tests (new: defaults read, inherited radio default, reset fill keeps /DV, cyclic tree); vitest (new resetEdits test); new E2E scenario passes (reset, Undo, nothing left to reset, saved copy read back by the CLI, source unchanged).

## 2026-10-22 — Session 2, checkpoint 12: PDF → Word (DOC-CONV-001, D-016)
- Engine `crates/pw-pdf/src/docx/`: per-character extraction from PDFium (display orientation; pages whose text reads upright unturned are laid out unturned and reported), pure layout pass (lines, word spaces, column gutters with bands for spanning titles, paragraphs by wrap test and column line pitch, headings, alignment, indents, exact spacing carried between bands, superscripts, soft hyphens, cells on tab stops, picture rows / floating / background), WordprocessingML writer, verifying reader. Op `convert_to_docx`; CLI `to-docx`; MCP `convert_to_docx`; app File ▸ Convert to Word… and batch "Convert to Word".
- Found by the LibreOffice round trip and fixed: right margin hugging the widest line (lines wrapped → pages doubled); every line a paragraph at generous spacing; bottom margin = content bottom (spill to extra pages); side-by-side pictures stacked; subtitle crossing the gutter hid the columns; band spacing lost; masked picture lost its mask; "DejaVuSans" named "Deja Vu Sans".
- New fixture `article.pdf` (`tools/fixtures/gen_article.py`, fontTools; deterministic): title + 2 columns, headings, link, superscript, picture, list, ruled table, justified paragraph, A4 landscape page.
- Ran: fmt/clippy (below), pw-pdf docx tests 7/7 + unit 12, CLI + MCP tests, worker mutation test with conversion (10 cases/fixture), E2E "convert to Word" passes. Measurements in `docs/benchmarks/DOCX_BASELINE.md` (175 pages in 3.3 s, 46 MB, debug).
- CI: Linux job installs LibreOffice (writer, no GUI) + DejaVu/Liberation fonts for the round trip; Windows sets PW_SKIP_SOFFICE=1 (no LibreOffice there; the test says SKIPPED).
- Not done: Word tables from ruled tables, shapes/borders, comments, form fields, Microsoft Word check, real-document benchmark.
- Follow-up: ruled tables → Word tables (`docx/tables.rs`: rules from path segments incl. closing edges, merged collinear pieces, connected grids, merged cells by missing rules; cells' text laid out per cell). Article page 2 ink coverage 0.794 → 0.995; drawings on table grids no longer reported as left out.

## 2026-10-22 — Session 2, checkpoint 13: tables to Excel and CSV (TABLE-EXPORT-001, D-017)
- `export_tables` op (xlsx file or csv folder), `pw-pdf/src/sheet.rs` (grid from the Word layout's tables, SpreadsheetML writer/reader, CSV writer/reader, exact-number rule), CLI `tables`, MCP `export_tables` (Excel), app File ▸ Export tables to Excel or CSV….
- New fixture `tables.pdf` (gen_fixtures.py): merged header, vertical merge, values a spreadsheet could corrupt.
- Ran: pw-pdf tables tests 4/4 (LibreOffice reads the workbook back unchanged), sheet unit tests, CLI + MCP tests, E2E "export tables".
- Not done: borderless tables, cell formatting, Microsoft Excel check.

## 2026-10-22 — Session 2, checkpoint 14: reader processes for the viewer (D-018)
- `pw_pdf::service` (ReadCall, ReaderPool, run_reader); app commands open/search/page text/fields/redaction preview/attachments/comments/signatures/objects/edit preview/hidden-info and the `pwpage` images go through the pool (2 readers); CLI and app executables accept `--pw-reader`.
- Found while testing: serde_json's default float parsing is not exact (last-digit differences) → `float_roundtrip`; a process-wide env var raced between parallel tests → per-pool env; the WebDriver starts the app with its own environment → debug-only hook file.
- Ran: reader tests 4/4 (same answers as in-process, passwords, parallel calls, crash fails one call and is remembered, hang times out, idle reader killed and replaced, speed), app crate unit tests, E2E "viewer: an engine crash …" passes.
- CI #88: cargo-deny flagged zip 8.6.0 (RUSTSEC-2026-0367/0368/0369, archive extraction — not used here; never ignored per project rule) → zip 9.0.5 (`file_names()` now yields Results). The LibreOffice workbook check failed on CI because only Writer was installed → CI installs Calc too.

## 2026-10-22 — Session 2, checkpoint 15: PDF → PowerPoint (DOC-CONV-PPTX-001)
- `convert_to_pptx` op; `pw-pdf/src/slides.rs` (PresentationML writer: master, blank layout, theme, slides with text boxes, tables (merged cells), pictures, links; verifying reader); the Word export's page analysis was split out (`docx::analyze`, `docx::notes`) and paragraphs/pictures/tables now carry their page rectangles. CLI `to-pptx`, MCP `convert_to_pptx`, app File ▸ Convert to PowerPoint…, batch.
- Found by looking at LibreOffice's rendering: single-line headings wrapped (synthetic bold is wider) → one-line boxes never wrap; body boxes too wide → 1.5 % slack.
- Found while adding MCP tests: the export_tables MCP checks from checkpoint 13 had never been added (a text replacement silently missed after formatting) — added now with the PowerPoint ones; tool count 21. Edit scripts now assert that every replacement matched.
- Ran: pptx tests 3/3 (LibreOffice Impress round trip), CLI + MCP tests, E2E "convert to PowerPoint". CI installs Impress.

## 2026-10-22 — Session 2, checkpoint 16: open Office files (DOC-IMPORT-001, D-019)
- `convert_office_to_pdf` op, `pw-pdf/src/office.rs` (find LibreOffice, private hardened profile, copy of the input, time limit, cancel, PDFium-verified atomic write), CLI `from-office`, MCP `convert_office_to_pdf`, app "Open a Word, Excel or PowerPoint file…" (start screen, palette, drop, launch argument, Open) with `OfficeDialog`; Settings ▸ Resources gains "Office documents (optional)" (`required: false`, not counted in `all_ready`).
- Tried to prove the macro hardening: the test document's macro did not run even in a deliberately unsafe profile (LibreOffice 24.2 headless conversion), so the test guards but does not prove; recorded in the test and D-019.
- Ran: office tests 3/3, OfficeDialog tests 4/4, MCP tests, E2E "open an Office file".

## 2026-10-22 — Session 2, checkpoint 17: borderless tables for table export (D-020)
- `pw-pdf/src/docx/borderless.rs` (rows of glyphs → cells split at ≥ 1 em gaps → runs of rows sharing free vertical channels → grid; prose-in-columns rejection), `layout::borderless_tables`, used by `page_tables` for table export only. Empty drawn grids are no longer exported (blank-pages.pdf page 5 had produced a 15 × 4 sheet of empty cells).
- New fixture `borderless-tables.pdf` (gen_fixtures.py): a lineless table, a table with horizontal rules only, text around both, and a page of two-column prose.
- Checked every fixture with `pageweave tables --format csv`: only article.pdf (its ruled table, once), tables.pdf (unchanged), borderless-tables.pdf (both tables, exact values) produce tables.
- CI #92 (Windows): `resources` exited non-zero because the optional Office entry (no LibreOffice on the runner) counted as missing → the exit code now follows required capabilities only; the CLI test checks it with LibreOffice hidden.
- Ran: fmt, clippy, all Rust tests, vitest 130, control audit, licenses, full E2E 82/82.
- Not done: wrapped cells merged into one row; borderless tables in Word/PowerPoint export; a real-document benchmark.
