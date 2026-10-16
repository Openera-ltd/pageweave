# Test Matrix and Verification Strategy

## Test pyramid

1. **Static checks:** formatter, lint, type checks, compiler warnings, dependency audit, license/provenance inventory, secret scan, and unsafe-pattern checks.
2. **Unit tests:** parsing/validation, format and option handling, job-state machine, progress math, path rules, encoding, preference migrations, shortcut conflict rules, selection/context menu availability, permission decisions, and error mapping.
3. **Integration tests:** Rust/C++ ABI contracts, storage, rendering engines, OCR/model runtimes, resource downloader/integrity verifier, settings/history, IPC/API/MCP, job cancellation, and atomic output writes.
4. **End-to-end tests:** start the packaged app, open representative files, use actual UI controls, perform operations, validate generated output, close/reopen, and verify saved state.
5. **Performance/stability tests:** realistic and stress documents, large lists, prolonged workloads, repeated open/close, cancellation storms, memory growth, cold/warm startup, resource contention, and UI frame pacing.
6. **Release tests:** clean install, first-run resources, no-network behavior, upgrade/rollback, permissions, shutdown, and user-data preservation.

## Required representative data corpus

Maintain small redistributable/licensed fixtures and a larger controlled evaluation corpus that includes:
- text-based PDF; scanned PDF; mixed text/image PDF; skewed and low-resolution scans; photos/screenshots;
- multi-column papers, footnotes, headers/footers, complex page breaks, dense tables, borderless tables, merged cells and multi-page tables;
- forms, annotations, links, bookmarks, digital signatures, embedded fonts, missing fonts, unusual encodings, vector drawings, charts, equations, transparency, and flattened posters;
- DOCX/XLSX/PPTX/ODT/ODS/ODP/RTF/TXT/HTML/SVG/image files only for formats actually supported;
- Unicode, combining marks, CJK, Arabic, Hebrew, RTL mixed with LTR, diacritics, long names, numerals/dates, and fonts with different shaping behavior;
- empty, huge, corrupt, truncated, encrypted/password-protected, unsupported, malformed, and adversarial files;
- output files with same-name conflicts, read-only destinations, nonexistent paths, permission denied, long/Unicode paths, and low-disk conditions.

Keep expected extracted text, page/object counts, table data, operation results, and visual baselines where valid. Avoid relying on exact pixel equality where nondeterministic rendering is expected; use justified tolerances and structural checks. Separate tuning/training data from held-out evaluation data.

## Interaction coverage matrix

For each shipped control, test:
- normal mouse activation;
- keyboard focus and activation;
- disabled state and explanation;
- busy/loading and repeated activation;
- success feedback and actual side effect;
- invalid input and error feedback;
- Escape/cancel behavior;
- undo/retry/recovery when applicable;
- custom theme and reduced-motion behavior where applicable;
- accessibility name/focus state;
- persistence/restart where relevant.

For each relevant target, cover right-click context menu, selection preservation, menu clipping, keyboard navigation, and stale selection/job state. Maintain stable test identifiers for interactions so visible controls can be mapped to tests.

## Resource/long-job scenarios

Simulate or test no network, intermittent network, slow transfer, server failure, TLS/DNS error, partial download, checksum mismatch, disk full, permission denied, app crash/restart, sleep/wake, cancellation, one resource failing among many, outdated resource manifest, and corrupt installation. Verify correct progress, recoverability, cleanup, and no false completion.

## Hardware and Windows matrix

Establish minimum-supported and recommended hardware only after empirical tests. At minimum, categorize and measure:
- low-resource supported machine with CPU-only processing;
- normal recommended machine;
- higher-spec machine with supported accelerator, if applicable;
- supported Windows versions and clean Windows user accounts;
- standard user permissions without administrator privileges where intended.

For every category, record OS/build, CPU, RAM, GPU/driver, storage type/free space, display resolution/scaling, test artifact version, dataset, duration, results, crashes, and limitations. Test mixed-DPI/multiple-monitor behavior and window resizing. Do not claim compatibility beyond the tested matrix.

## Performance baselines

Measure rather than guessing: cold and warm launch; time to first usable window; document first-page display; page scroll and zoom smoothness; search latency; conversion/OCR/translation/TTS throughput and peak resources; memory release after closing documents; cancellation latency; and long-session stability. Track regressions across commits on reproducible fixtures. Investigate UI stalls around 50 ms or longer, while recognizing that this is a diagnostic threshold, not proof of a specific root cause.

## Test evidence

Store concise machine-readable results where feasible, including command, environment, exit code, artifact/hash, test case IDs, metrics, and timestamp. Avoid storing sensitive documents or credentials. Manual checks must identify operator path and observed outcome. The release report must clearly state tests not run and why.
