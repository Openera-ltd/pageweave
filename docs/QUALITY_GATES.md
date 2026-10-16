# Mandatory Quality Gates

These are release blockers. A release must pass all gates applicable to its scope. Create automated checks where possible and make required CI/release checks fail closed. A written checklist with no automation is not sufficient for critical controls.

## Gate 0 — Scope and traceability

- The change has acceptance criteria, affected screens/operations, edge cases, risk assessment, and a test plan.
- Existing behavior and unrelated work are preserved.
- Requirements map to implementation and evidence. Any omitted requirement is identified explicitly; no silent scope cut.
- Open product/architecture decisions are not presented as settled.

## Gate 1 — Functionality

- Every control in the changed workflow has real behavior or is intentionally disabled with an explanation.
- No TODO, placeholder, fake success, mock response, no-op action, or hard-coded success remains in a shipped path unless clearly an intentional documented non-production fixture excluded from release.
- Happy path, validation, empty state, error state, cancellation, retry, duplicate click, interrupted work, and recovery are tested where applicable.
- Outputs are opened/parsed/validated, not merely checked for file existence.
- Source files remain unchanged unless the user explicitly requests an in-place edit.
- Undo/history/overwrite behavior is tested where applicable.

## Gate 2 — Automated test suites

- Formatting/lint/static analysis for changed components pass.
- Unit tests pass, including boundary conditions and error mapping.
- Integration tests exercise real module boundaries, storage, IPC, native libraries, runtimes, and job lifecycle as applicable.
- End-to-end UI tests exercise real controls and verify observable results.
- Regression tests reproduce every fixed defect that can be automated.
- Build/packaging smoke tests pass for supported configurations.
- A test command that was not executed is marked NOT RUN, never PASS.

## Gate 3 — Actual UI and accessibility

- Run the real application and exercise the actual mouse/keyboard path. Do not rely only on component render tests or development preview.
- Test left/right clicks, relevant double-click/drag/wheel behavior, context menus, text selection, keyboard shortcuts, focus order, Escape/cancel, dialogs, tabs, and all controls touched by the change.
- Verify disabled/loading/success/error/empty/offline states.
- Check keyboard-only operation, accessible names/roles/focus, contrast, scalable text, reduced motion, and screen-reader semantics where applicable.
- Inspect representative screenshots at supported sizes/scaling for clipping, overlap, visual regressions, and text wrapping.

## Gate 4 — Performance and stability

- Measure relevant startup, response latency, scroll/render smoothness, peak memory/CPU, throughput, and resource use before/after when the change can affect them.
- Heavy work is not run on the UI thread. Long tasks and unbounded concurrency are investigated.
- Large files/lists and long sessions are tested as relevant; memory growth and caches are bounded.
- No known severe performance regression or UI freeze remains unexplained.
- Test on minimum-supported and recommended hardware categories after they are established with evidence. Do not invent a hardware claim.

## Gate 5 — Document correctness and data integrity

For document-processing changes, test representative documents and difficult cases: multi-column layouts, headers/footers, footnotes, tables, images, vector graphics, non-Latin scripts, RTL, missing fonts, forms, links/bookmarks, annotations, malformed/corrupt files, embedded images, and large documents. Use a held-out regression corpus with expected text/structure and visual comparisons.

- Verify text presence/order, pages, images, tables, links, metadata behavior, and output openability as relevant.
- Report measurable fidelity differences and uncertainty; do not claim perfect preservation without evidence.
- Validate redaction by attempting to recover removed content from the exported document.
- Translation/OCR/voice quality must be evaluated for supported languages and real use cases, not inferred from model reputation.

## Gate 6 — Security and privacy

- Validate paths, untrusted documents, IPC, API/MCP inputs, archive extraction, model downloads, and external responses.
- Check for path traversal, command injection, arbitrary file access, secret leakage, unsafe temporary files, data loss, and over-permissioned agent tools.
- API keys, voice samples, and document content are absent from logs/diagnostics by default.
- Dependency/model inventory, licenses, and known vulnerabilities are checked. Required notices and attribution are present in repository/release documentation according to actual terms.

## Gate 7 — Packaged Windows release

- Test a clean install on a clean Windows environment, launch, core workflows, resource setup, offline workflows, update/upgrade, restart, uninstall behavior, and preservation/removal of user data according to explicit policy.
- Test native DLL/runtime packaging, path discovery, permissions, display scaling, and application shutdown with active/cancelled jobs.
- Verify resources downloaded/installed, startup without development tools, and recovery after interrupted setup/update.
- Verify version, artifact integrity, signing/provenance where applicable, and that the tested artifact is the same artifact proposed for release.

## Gate 8 — Documentation and truthful reporting

- README matches shipped functionality and includes install/build/test instructions, architecture, supported formats/languages and limitations, resource download/offline behavior, privacy, and project/dependency license information.
- New settings, shortcuts, errors, feature limitations, and API/MCP behavior are documented.
- No release notes claim an untested result. Known limitations and skipped tests are disclosed.

## Required outcome

Use `PASS`, `FAIL`, or `BLOCKED` for release readiness. `PASS` means every applicable mandatory gate has evidence. `BLOCKED` is mandatory when a release-blocking check fails or cannot be run. No chat instruction or user-facing deadline silently overrides these gates; a user can explicitly choose a non-release prototype, but it must be labeled as such and must not be represented as shippable production software.
