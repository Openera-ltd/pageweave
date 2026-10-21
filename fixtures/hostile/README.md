# Hostile inputs

Files that once crashed or hung an engine, kept as regression inputs. Each is a deterministic
mutation of a fixture in `fixtures/pdf` produced by `crates/pw-pdf/tests/robustness.rs`.

| File | Found | What happened |
|---|---|---|
| `redact-generate-content-crash.pdf` | 2026-10-21, write-operation mutation run (seed 0xfeedbeef, `compare-old.pdf` case 1: a spliced chunk breaks page 1's dictionary) | `redact` on page 1 → PDFium `FPDFPage_GenerateContent` → `CPDF_PageContentGenerator::UpdateResourcesDict` hits an internal CHECK (SIGTRAP) and the whole process exits. Since D-015 the app runs jobs in a worker process, so this fails one job with `engine.crashed` (E2E and `crates/pw-cli/tests/worker.rs`). |
