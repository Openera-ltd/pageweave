# PageWeave Engineering Knowledge Base

This directory is the durable product/engineering memory. Use it with the root `CLAUDE.md`.

- `PRODUCT_VISION.md` — purpose, capabilities, boundaries, and unresolved product decisions.
- `ARCHITECTURE.md` — component boundaries, language responsibilities, interfaces, jobs, and security.
- `INTERACTION_CONTRACT.md` — precise expected behavior for mouse, selection, keyboard, controls, dialogs, settings, and state transitions.
- `UX_DESIGN_SYSTEM.md` — design research, visual language, motion, accessibility, themes, and responsive behavior.
- `AI_AND_RESOURCES.md` — required specialist capabilities, optional external AI, first-run resource setup, offline guarantees, TTS, and voice cloning.
- `QUALITY_GATES.md` — non-negotiable quality standards and ship-blocking conditions.
- `TEST_MATRIX.md` — testing layers, edge cases, hardware/platform matrix, and interaction coverage.
- `RELEASE_CHECKLIST.md` — final packaged release gate.
- `FEATURE_ACCEPTANCE_TEMPLATE.md` — use for each feature or meaningful feature change.
- `DECISIONS.md` — record approved architectural/product decisions, date, rationale, consequences, and status.

- `../prd/INDEX.md` — detailed multi-document product requirements suite, including Acrobat-style desktop parity, user interactions, specialist AI, model training/data rights, offline resources, watchdog and release acceptance.

## Required maintenance

Before changing a feature, consult the relevant documents and current implementation. After changes, update documentation only when the actual behavior or an approved decision changes. Add regression tests for discovered defects. Do not turn plans into claims of existing functionality.

If a requirement is ambiguous and would materially change cost, scope, compatibility, security, privacy, or data integrity, record it as an open decision and ask the user rather than guessing. Otherwise choose the safest, most reversible path consistent with these instructions.
