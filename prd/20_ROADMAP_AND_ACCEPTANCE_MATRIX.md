# PRD 20 — Roadmap, Status Model and Launch Acceptance

## Suggested dependency-aware phases
**Phase 0 — Audit/feasibility:** inspect repo/current code, build/package, tests, licenses, supported OS/hardware, PDF/Office library options, UI architecture and model runtimes. Build gap/risk/format matrix and benchmark baseline.

**Phase 1 — Foundation:** Rust/C++ interface, job queue, cancellation, safe output, structured errors/logging, resource registry/download/resume/integrity, versioned settings, crash recovery, performance instrumentation, CI, packaged test harness and 10-minute watchdog. Core viewer/open/save-as/search/navigation and offline startup.

**Phase 2 — PDF essentials:** annotations, organize pages, create/merge/split, compression, metadata, print, forms, password protection, local signatures, redaction/sanitization and comparison.

**Phase 3 — Conversion/OCR/layout:** office format matrix, scanned PDF/OCR, restoration, tables, layout-aware output reconstruction, quality reports and batch processing.

**Phase 4 — Specialist capabilities/accessibility:** language registry/translation, TTS alignment/voices, read modes, classification/field extraction, local full-text index, optional provider orchestration, audio export.

**Phase 5 — Design Reconstruction Studio:** editable components, text/vector recovery, segmentation, layers, difference overlay and exports.

**Phase 6 — Advanced parity:** accessibility tagging/validation, PDF/A/X/E/UA supported profiles, print production, measurement, attachments/metadata, advanced form/crypto workflows and edge formats.

**Phase 7 — Qualification:** full matrix audit, security/fuzz, memory soak, real hardware, accessibility/localization, update integrity, license/SBOM/provenance and installer/rollback/uninstall/offline/recovery.

Training/baseline eval may run in parallel from Phase 0 when it has rights-cleared data and adequate resources. Integrate only models that pass evaluation and license gates.

## Status model
`Not assessed`, `Planned`, `In progress`, `Implemented` (code exists but unverified), `Verified` (criteria and failure tests passed with evidence), `Blocked` (dependency/rights/hardware/choice issue and next action recorded), `Deferred by decision` (explicit owner approval and honest product documentation). Never mark Verified based on Claude's own summary alone.

## Mandatory launch checklist
- Every displayed actionable control is bound to real operation or clearly disabled.
- Complete in-scope offline PDF parity matrix and accurate gaps.
- Resource setup/retry/repair/offline operation and model lifecycle pass.
- Tested structural and visual conversion; no hidden full-page raster flattening in editable export.
- Redaction removed content with independent validation.
- Signature trust statuses honest and tested.
- OCR/tables/translation/TTS/reconstruction meet blind benchmarks and licensing is cleared.
- User-document and voice privacy, API/MCP authorization and secret handling pass.
- Theme, shortcuts, context menus, keyboard and screen reader essential workflows pass.
- Performance/memory/soak on minimum/recommended hardware pass.
- Actual 10-minute watchdog fires, logs, detects failures, deduplicates and avoids unapproved destructive repair.
- Kaggle experiments/checkpoints resumable; data provenance approved.
- Clean install, upgrade, rollback, uninstall, restart recovery and offline test pass.
- No critical/high security issue; SBOM/dependency/model inventory current.
- README, known limits, file/language/hardware support and troubleshooting match actual release.
- Exact build/commit/test/benchmark/package artifacts retained.

No feature is silently dropped to make a release milestone. Owner approval is required for scope deferral, and marketing must not claim deferred capability or full parity.
