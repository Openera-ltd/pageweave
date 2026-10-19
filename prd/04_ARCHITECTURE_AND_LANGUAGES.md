# PRD 04 — Architecture, Language Boundaries and Engine Contracts

## Shared engine
One stable typed document engine is used by desktop UI, optional CLI, versioned local API and optional MCP. The UI must not implement conversion/OCR/processing separately. Ordinary operations work without an external AI agent.

## Language responsibilities
- **Rust:** orchestration, typed operations, job queue, cancellation, resource manager, filesystem safety, task journal, structured errors/logs, settings, API/MCP/CLI and platform adapters when practical.
- **C++:** mature native PDF/rendering/graphics/image-processing integrations where justified by a library or measured performance need. Use RAII and narrow wrappers.
- **TypeScript:** UI when the chosen desktop framework supports it; typed state transitions and accessible controls. No heavy parsing/inference in UI thread.
- **Python:** model training/experiments/evaluation/export, and worker integrations when justified. Production build bundles required runtime or uses an appropriate inference runtime; users must not install development Python manually.
- **C/C#/other languages:** only for existing interfaces or measurable Windows/platform need; document every shipped runtime.

## Boundaries and contracts
Use stable C ABI or isolated worker IPC where safer than sharing C++/Rust/Python object ownership. All operations use versioned schemas, typed enums, validated inputs, correlation IDs and structured results. No raw pointers across async ownership boundaries without rigorous lifetime controls. Catch exceptions/panics at FFI boundaries and convert to structured failures. Hostile parsing/inference should be isolated in worker processes where practical.

Example request: `schema_version`, `request_id`, `operation_id`, validated document/input handles, options, output policy, cancellation ID and correlation ID. Result: status, outputs, warnings, quality report reference, metrics, errors, timestamps and retryability. Pass handles/path references rather than copying entire documents in memory.

Never build shell strings from untrusted input. Prefer libraries over subprocesses. If subprocess needed: fixed executable, argument array, allowlisted environment, validated paths, timeout, bounded output and explicit cleanup.

## Job system
Bound concurrency separately for I/O, render, conversion, OCR/inference, TTS, resource install and network work. Prioritize visible UI tasks over background indexing/prefetch. Use job IDs, meaningful phase progress, cooperative cancellation, durable checkpoints and safe restart recovery. Prevent duplicate runs via idempotency and locks. Never claim a job was cancelled if commit completed; report actual terminal state.

## Errors and results
Typed categories include invalid input, unsupported operation, corruption, permission, output conflict, disk full, missing/corrupt resource, network/rate limit, model failure, engine crash, cancellation, timeout, validation failure, security policy and internal bug. Each includes safe user message, technical code, retryability and recommended action. Don't place document text/secrets in the error.

## Storage/configuration
Separate settings, jobs, indexes, caches, logs and voice profiles with explicit retention and cleanup. Use SQLite or another embedded store where justified, with migrations, lock/corruption handling and backup policy. Model resources use a versioned manifest and checksum; do not put large weights/datasets into Git. Separate dev/test/prod settings; no secrets in source/config defaults/build artifacts.

## Architecture acceptance
Test each native/process boundary, request schema, cancellation, resource lifetime, crash/restart, compatibility upgrade, file writes and offline paths using real engines and representative samples—not only mocks.
