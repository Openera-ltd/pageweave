# PRD 13 — Test Strategy, Acceptance Evidence and Release Gates

## Binding rule
Never ship untested features. Failed, missing, skipped-without-approved-reason or unexecuted mandatory checks mean **BLOCKED**. A compile, happy-path demo, screenshot or green unit suite alone is not proof.

## Traceability
Every requirement has ID, component, implementation location, automated/manual/package test, benchmark if relevant, status, evidence link, limitations and owner. `Unknown` is not pass. Statuses: Not assessed, Planned, In progress, Implemented, Verified, Blocked, Deferred by explicit decision. Only observed and recorded evidence allows `Verified`.

## Testing layers
1. Static analysis, compiler warnings, format/lint/type checks and secret scanning.
2. Unit tests: parsers/ranges, settings migrations, state machines, path safety, resource manifests, error categories, validators and metrics.
3. Component tests: renderer, selection, command registry, context menu, dialogs, jobs, logs, settings, cancellation, TTS alignment and resource installer.
4. Real-engine integration using representative files/models.
5. Golden-output/regression tests for render, conversion, OCR, tables, translation, reconstruction and TTS; thresholds documented; don't hide catastrophic per-dimension failure inside averages.
6. UI automation/E2E: real mouse/keyboard, text selection, menu, tabs, open/edit/save, settings, errors, theme, accessibility, restart and setup.
7. Packaged system tests: clean user/VM install, upgrade/rollback/uninstall, file association, offline mode, launch and no developer runtime dependencies.
8. Performance, memory, cancellation, soak, load/stress and fault injection.
9. Security, dependency/SBOM/license/provenance checks.
10. Accessibility automation plus manual screen reader/keyboard review.

## Interaction audit
Automated/static inventory must flag every visible actionable control missing command ID/handler, accessible name, enabled/disabled behavior and test. Exercise every main navigation action, button, icon-button, context menu, keyboard shortcut, slider/select, dialog, cancellation/retry, theme/density, panel resize and high-risk command. Actual application testing is required.

## Golden corpus
Create redistributable/openly licensed or synthetic test samples plus access-controlled evaluation corpus. Include vector/scanned/multicolumn PDFs, forms, invoices/tables, mixed size/rotation, Unicode/RTL, missing fonts, encryption/corruption, signatures/attachments, redaction, low-resolution scans, weird colors/transparency and design images. Store labels/ground truth, license/provenance, checksums, split and protocol. Split by source and near-duplicate cluster; preserve blind test.

## Fault injection
Network drops, DNS/TLS, 403/404/429/5xx, rate limits, checksum mismatch, partial artifact, low disk, permission denial, crash during install/commit, worker failure, OOM/GPU loss, device loss, file lock/external modify, corrupt DB/config/log, duplicate job, cancellation race and reboot/resume.

## Ship blocks
Data loss/corruption, source overwrite, incorrect redaction; dead controls/fake progress/silent errors; unexecuted mandatory tests; broken installer; severe performance/hang/memory regression; secret leak; exploitable file handling; unauthorized network upload; corrupt/tampered model; missing license/provenance; accidental network dependency for offline core; critical accessibility break; false signature/conformance/quality/language claims.

## Production definition
All mandatory launch-scope features pass acceptance and failure tests; any deferred scope is owner-approved and clearly disclosed. Clean install/upgrade/rollback/uninstall/offline/recovery proven. No open critical/high security issue. Performance budgets, quality thresholds, license/provenance and accessibility pass. Watchdog and diagnostic bundle work. Docs accurately state support and limitations. Include exact commit/build/package, test counts, results, skip reasons, benchmarks and artifact links. Do not weaken tests merely to get green.
