# PageWeave Decision Log

Record approved product/architecture decisions here. Do not mark proposals as approved. Include a date, status, rationale, consequences, evidence, and superseded decision if applicable.

## Decision template

### [ID] — [Short title]
- **Status:** Proposed / Approved / Rejected / Superseded
- **Date:** YYYY-MM-DD
- **Context:**
- **Options considered:**
- **Decision:**
- **Evidence:**
- **Consequences/trade-offs:**
- **Follow-up/tests/docs:**

## Initial scope questions — unresolved

### DW-001 — Built-in full word processor and spreadsheet editor
- **Status:** Open; requires user decision.
- **Context:** PageWeave already has broad viewing, editing, conversion, OCR, translation, and specialist document workflows. A full general-purpose office editor is a major scope expansion.
- **Question:** Should PageWeave build full from-scratch document/spreadsheet authoring, or focus on existing-file editing/conversion and use mature engines/integration for broader authoring?
- **Constraint:** Do not silently choose a scope that materially increases architecture, implementation, testing, and maintenance cost.
