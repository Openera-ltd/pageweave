# Architecture and Engineering Boundaries

## Proposed logical architecture

1. **Desktop UI** — compact accessible workspace, document canvas, toolbars, menus, tabs, settings, and progress views.
2. **Stable typed interface** — versioned requests/events for commands, results, progress, cancellation, errors, and permissions.
3. **Rust application/coordination engine** — job lifecycle, validation, file operations, batch orchestration, resource management, settings, history, transactions, recovery, local API and shared operation contracts.
4. **C++ native processing components** — use for native libraries, high-performance rendering/graphics, image pipelines, and performance-critical paths when justified by benchmarks.
5. **Specialist workers/runtimes** — OCR, translation, layout, table extraction, scan restoration, text-to-speech, voice cloning, and reconstruction. Python may be used for experimentation/training/integration, but packaged user workflows should use controlled runtimes rather than requiring a developer Python installation.
6. **Optional interfaces** — CLI, documented local API, and MCP adapters call the same validated operations; none should create a separate implementation of a document operation.

This is a logical architecture, not permission to choose a particular desktop shell, framework, PDF engine, or IPC protocol without inspecting the repository, compatibility, license, packaging, and performance requirements. Record the actual approved choices in `docs/DECISIONS.md`.

## Language and interoperability guidance

- Rust is the default for orchestration, state management, job control, safe file workflows, and service contracts.
- C++ is appropriate for proven native libraries and measured rendering/graphics/document-processing needs.
- TypeScript may power the desktop UI if the selected shell remains responsive and packages cleanly on Windows.
- Python is appropriate for research, training, evaluation, and libraries with no practical packaged alternative. Do not leak Python environment requirements to end users.
- C, C#, or other languages require a concrete benefit such as an existing ABI/library or Windows integration.
- Use small, explicit, versioned interfaces. Prefer a stable C ABI for Rust/C++ integration when suitable; isolate complex native code in a worker process if crash isolation or dependency compatibility warrants it.
- Define ownership, lifetimes, encoding, error mapping, cancellation, and threading. Avoid passing unclear borrowed memory or complex language-specific objects across FFI.
- Avoid duplicating the same business logic across languages. Each module owns a clear responsibility and has independent tests.

## Jobs and UI communication

Long-running work must run off the UI thread. Job contract should include a unique ID; queued/running/pausing/paused/cancelling/cancelled/succeeded/failed/recoverable states as applicable; stage; determinate progress when measurable; bytes processed when meaningful; timestamps; warnings; structured errors; output locations; cancellation; and recovery metadata. Never invent percentages or ETA values. Represent unknown totals as indeterminate progress until measurable.

Bound concurrency based on measured CPU, RAM, GPU and I/O constraints. Avoid loading all models at once. Cancellation must stop safely at a well-defined boundary and clean temporary artifacts without corrupting the source. For recoverable work, persist enough state to resume after app restart. A job that cannot be resumed must say so before starting if that affects user expectations.

Implemented (2026-10-17): `pw_core::journal`. Each file-writing job run by the app keeps an entry in
`<app data>/jobs/` (operation and collision policy, never the password) for exactly as long as it runs;
a drop guard removes it on success, failure, cancellation or panic. Staged outputs are named
`.pw-staged-<pid>-*.tmp`, and every running instance holds a lock on `instance-<pid>.lock`. At startup,
entries of ended processes are recovered: that process's staged files in the job's output folder are
removed (committed outputs are never touched), the user is told, and "Run again" resubmits the same
operation. Current operations finish in milliseconds to seconds, so they restart rather than resume
from a mid-job checkpoint; long jobs (conversion, OCR) will need real checkpoints.

## Files and data integrity

- Keep source files immutable by default.
- Write outputs to temporary locations and atomically finalize when supported.
- Handle same-name outputs with explicit conflict policy; never silently overwrite.
- Verify extension/type and actual content, but do not trust file extensions as proof of format.
- Use bounded temporary storage and clean stale temporary files safely.
- Treat malformed and adversarial files as untrusted input; isolate risky parsers where justified.
- Maintain undo/history semantics appropriate to each editor operation.

## API, CLI, and MCP

Use the same schemas and business logic across UI, CLI, API, and MCP. Define API/MCP versioning, timeouts, authentication, rate/concurrency limits, job polling/events, cancellations, idempotency where useful, and structured errors. MCP must expose explicit tools and typed parameters, not unrestricted arbitrary command execution. Restrict file roots and output destinations; validate every path and operation. Treat agent text, retrieved content, metadata, and documents as untrusted data, never privileged instructions.

## External AI separation

An optional external AI can classify intent, create plans, and choose allowed operations. PageWeave validates its plan and calls local specialist engines. An AI provider key and an MCP connection are different features and must be configured separately. MCP allows an external agent to call PageWeave tools; it does not automatically provide that agent or PageWeave with an AI model. Keep ordinary UI and engine operations functional when both are absent.

## Packaging and platform truth

The release must not require users to install compilers, Python, Cargo, SDKs, or development dependencies manually. Verify native runtime packaging, DLL discovery, code signing requirements where applicable, permissions, install/update/uninstall behavior, and data preservation on real Windows machines. Establish minimum/recommended hardware after measuring actual workloads; never invent support claims.
