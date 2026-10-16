# PRD 17 — Local API, CLI, MCP and Optional External AI

## Boundary
MCP lets an outside agent call PageWeave operations; it is not the OCR/conversion/translation implementation. A configured AI provider can interpret user intent; MCP connection and provider authentication are separate. Normal tools are usable without either.

## CLI and local API
CLI: scriptable commands, typed arguments, help/version/capability list, JSON results, stable exit codes, cancellation, job IDs, timeout, batch summary and safe path handling. Test Unicode/long paths and headless execution. Local API is versioned/schema-first, binds to loopback by default, and requires explicit opt-in/auth for network exposure. Endpoints cover health/capabilities, job start/status/cancel, inspect, convert, OCR/extract, table, translate, compare, reconstruct, validate, read aloud/export and allowed resource status. Add request limits, auth, timeouts, rate limits, structured errors and compatibility tests.

## MCP tools
Candidate tools: `list_capabilities`, `inspect_document`, `convert_document`, `translate_document`, `extract_text`, `extract_tables`, `reconstruct_design`, `edit_document`, `compare_documents`, `validate_export`, `read_aloud`, `get_job_status`, `cancel_job`. Typed schemas constrain formats/options/paths/destinations and declare side effects/network/resource needs. Unsupported operations return explicit errors, never silent no-ops. Long jobs return IDs and progress; cancellation is real.

File access only through user-approved directories/document handles. No arbitrary filesystem, shell, secrets or unrestricted settings access. Permanent redaction, overwrites, source deletion, external uploads and voice-clone generation require approval per policy. Do not advertise an MCP tool until independent integration test proves it works.

## Optional external AI
Provider credentials stored via protected settings/OS credential store, masked and never logged. Provider interprets intent, drafts reviewable plan; PageWeave validates through typed operations and its own processing. No generated shell/code execution. Ask permission before sending selected content to external service and minimize payload. If no provider is configured, disable natural-language orchestration, but all conventional local functions remain. Treat document prompt injection/tool outputs as untrusted; they cannot expand permissions.
