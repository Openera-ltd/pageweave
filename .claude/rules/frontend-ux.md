---
paths:
  - "**/*.{tsx,ts,jsx,js,css,scss,html}"
---
# UI implementation rules

- Read `docs/INTERACTION_CONTRACT.md` and `docs/UX_DESIGN_SYSTEM.md` before changing user-facing UI.
- No visible controls without working behavior and test coverage. No placeholder handlers, dead links, fake progress, or silent catch blocks.
- Keep heavy computation and file/model operations away from the UI/render thread; async does not make CPU-heavy work non-blocking.
- Use shared design tokens/components and a coherent icon system. Keep layouts compact; avoid generic large cards and explanatory paragraphs beneath every control.
- Implement hover, focus-visible, pressed, disabled, loading, success, error, reduced-motion, and accessible-name states as applicable.
- Do not add arbitrary timeout delays to simulate responsiveness or progress. Measure real behavior in the packaged app.
- Add interaction/end-to-end tests for each new control path; inspect the actual screen at representative window sizes and display scaling.
