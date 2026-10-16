# PageWeave — Permanent Engineering Instructions

These are mandatory project rules for every Claude Code session. They are not suggestions. Apply them to planning, coding, reviews, tests, documentation, packaging, and release work.

## 1. Read, remember, and preserve project truth

1. At the beginning of every task, read this file, `docs/INDEX.md`, the current product decisions, the relevant specifications, and the affected code/tests. Inspect `git status`, the recent diff, and the repository structure before changing anything.
2. Confirm that these instructions are loaded when practical. If important project context is missing, search the repository before asking the user to repeat it.
3. Treat tracked specifications, decision records, tests, and source code as the durable source of truth—not a previous chat transcript or a guess. After a correction or a repeated failure, add the rule to the correct permanent document and add a regression test where possible.
4. Keep `CLAUDE.md` concise. Put detailed, feature-specific requirements in the relevant `docs/` files and scoped `.claude/rules/` files. Consult them before changing the relevant code; update them when reality or an approved decision changes.
5. Do not invent product decisions, requirements, test results, supported hardware, commands, dependencies, or completed functionality. Flag unresolved scope decisions and avoid silently making irreversible choices.
6. Preserve existing working behavior. Do not delete, replace, reformat, or redesign unrelated code. Check the diff for unintended changes. Never discard user work or run destructive commands without explicit approval.

## 2. Non-negotiable quality rule: never ship unverified work

1. **Code compiling is not proof that a feature works. A green unit-test suite alone is not proof that the product works.** Verify the real workflow and failure paths.
2. Before marking work complete, map each requirement to implementation and evidence: automated test, reproducible manual test, benchmark, or clearly identified external blocker.
3. Run the relevant tests after each meaningful change, then run the broader regression suite. Build and test the packaged Windows application—not only a development server, mock, storybook, preview, or unit test.
4. Never claim a test, build, benchmark, accessibility check, security check, or manual verification passed unless it actually ran and its result was inspected. Say exactly what was run, what passed, what failed, and what could not be tested.
5. A required check that fails or cannot be executed means **release blocked**, not “probably fine.” Do not bypass, weaken, skip, delete, or rewrite tests just to get a green result. Any approved exception must be explicit, documented, narrow, time-limited, and never conceal a safety, data-loss, security, or core-functionality failure.
6. Never ship dead buttons, inert menu items, fake progress, placeholder success, silent failures, unfinished controls presented as complete, known data corruption, or an untested release package. Do not call something production-ready without evidence.
7. Every new user-visible interaction requires acceptance criteria and tests. Use `docs/QUALITY_GATES.md`, `docs/TEST_MATRIX.md`, and `docs/RELEASE_CHECKLIST.md` as mandatory gates. Implement CI/release automation for checks wherever possible; documentation alone is not an enforcement mechanism.

## 3. Required working loop

For non-trivial work: **inspect → plan → implement a small complete slice → test → run the actual app → profile → review the diff → update docs → verify release impact**. Work on the highest-risk assumptions first. Prefer small, reversible changes. If a task is too large for one pass, finish a useful vertical slice and report the remaining work honestly; never label the whole feature complete prematurely.

Before implementation, identify affected requirements, states, dependencies, data flows, risks, tests, and rollback behavior. After implementation, actively try to break the feature, not just demonstrate its happy path. Ask the user only when a decision materially changes scope, cost, privacy, compatibility, or destructive behavior; otherwise make the safest documented choice.

## 4. Product and architecture principles

1. PageWeave is a free/open-source Windows desktop document studio. It must be useful without an external AI account, API key, internet connection after required resources are installed, or MCP agent.
2. C++ and Rust are core implementation languages. Add TypeScript, Python, C, C#, or another language only where the UI stack, existing library, model runtime, or measured need justifies it. Do not use many languages for appearance alone.
3. Keep one stable document engine and typed operation contracts shared by the desktop UI, optional CLI, local API, and MCP. Do not implement the same operation differently in each interface.
4. Keep CPU-heavy document processing, model inference, file I/O, and long jobs off the UI thread. Async syntax does not itself move CPU-bound work off-thread. Use bounded workers, cancellation, explicit ownership, and safe C/C++/Rust/Python boundaries.
5. Reuse proven, appropriately licensed document engines and specialist models. Do not build a PDF/Office engine or foundation model from scratch without a documented benchmark-based reason.
6. Only the local general-purpose chat LLM is optional and must not be bundled. Specialist capabilities—including OCR, translation, layout understanding, table extraction, scan processing, speech, voice functions, and design reconstruction—are required product capabilities. Resource and model details are technical documentation, not model-name labels in the user-facing UI.
7. External AI APIs/MCP agents may interpret instructions and orchestrate calls. They must not replace PageWeave's own document conversion, OCR, translation, speech, or reconstruction engines. Ordinary actions and engine operations must remain available without external AI.
8. Preserve originals by default; use safe temporary files, atomic writes where appropriate, recoverable jobs, and explicit approval for destructive operations. See `docs/ARCHITECTURE.md` and `docs/AI_AND_RESOURCES.md`.

## 5. Interaction and visual design rules

1. Think through every input and state: left/right/middle click, double-click, drag/drop, selection, keyboard, wheel, focus, hover, touch/pen where applicable, cancellation, window resizing, and restart. Follow `docs/INTERACTION_CONTRACT.md`; do not improvise inconsistent behavior.
2. **Every visible button, icon-button, menu item, link, shortcut, toggle, selector, tab, slider, context-menu action, and drag target must work or be clearly disabled with a reason.** No decorative controls that look functional. Wire controls to real operations and test them.
3. For each action, define its purpose, availability, immediate feedback, progress, success, failure, cancellation, undo/recovery, shortcut, accessibility label, persistence, and test. Prevent accidental double-submission and stale or duplicate results.
4. Make the UI polished, compact, calm, professional, and responsive. Use a coherent icon system, consistent spacing/typography, a restrained blue-accented visual language, and deliberate visual hierarchy. Do not use random emoji, oversized generic cards, clutter, or explanatory paragraphs under every icon/control. Use concise labels and useful tooltips; put detailed help in searchable documentation.
5. Provide customizable light/dark/system themes, accent colors, density, text/display preferences, reduced motion, keyboard shortcuts, and sensible workspace preferences. Persist settings safely across restarts and upgrades.
6. Use purposeful, interruptible animations and transitions; no animation may block an action or delay feedback. Respect reduced-motion settings. Avoid jank, flicker, layout jumps, fake loading delays, and over-animation. Aim for smooth scrolling and approximately 60 FPS where feasible, backed by real-device measurement.
7. Research relevant current design references before a major UI design. Inspect actual screenshots/images, compare interaction patterns, and record what was learned. Use references for inspiration, not copied proprietary assets. See `docs/UX_DESIGN_SYSTEM.md`.
8. Accessibility is part of implementation: keyboard access, visible focus, logical focus order, screen-reader names, contrast, scalable text, reduced motion, and no color-only communication.

## 6. Performance and reliability

1. Measure before and after performance-sensitive changes. Track cold/warm startup, idle and peak RAM, CPU, UI responsiveness, scroll/render smoothness, document open/search/render, conversion/OCR/translation throughput, disk use, and long-session memory stability.
2. Virtualize long lists and thumbnails; render visible content first; avoid repeated parsing, unbounded caches, needless re-renders, copying huge documents, unbounded parallel jobs, and loading every specialist model simultaneously.
3. Load resources lazily where appropriate, cache with explicit limits, release memory, support CPU-only operation where the chosen components permit it, and adapt concurrency to available hardware. Do not promise hardware support that has not been tested.
4. Define budgets from a measured baseline on minimum-supported and recommended Windows hardware. UI input handling should not perform long CPU or disk work; investigate UI stalls around 50 ms or more. Treat performance regressions as defects with reproducible profiles.
5. Handle huge files, corrupted input, missing fonts, unsupported formats, low memory/disk, permissions, Unicode and long paths, slow drives, interrupted jobs, app termination, network loss, device removal, conflicting writes, and concurrent operations intentionally.

## 7. Security, privacy, licensing, and documentation

1. Validate untrusted files, paths, command arguments, archive contents, IPC/API inputs, MCP arguments, and external responses. Do not let agent requests execute arbitrary shell commands or access arbitrary files. Apply least privilege and explicit permission/approval boundaries.
2. Protect API keys and voice samples; never log secrets or document content by default. Explain what leaves the device before using an external provider. Sanitize exported diagnostics and crash reports.
3. Track dependency/model versions, provenance, license terms, vulnerability status, attribution, and redistribution conditions. Do not put license/credits screens or model names into the app UI merely to expose technical notices. Document project licensing and dependencies in repository README and required repository notices; meet each dependency's actual license requirements, which may require more than README alone.
4. Keep README and feature documentation synchronized with real behavior, supported formats/languages, limitations, build/test commands, privacy/offline behavior, architecture, API/MCP, install/update/recovery, and known gaps. Never document an aspirational feature as implemented.

## 8. Definition of done

A task is done only when its acceptance criteria are met; all applicable tests pass; meaningful error, edge, cancellation, and recovery paths are verified; real UI controls are exercised; performance and accessibility impact are checked; security/privacy implications are reviewed; documentation and regression tests are updated; and the diff contains no accidental changes. Release readiness additionally requires every mandatory item in `docs/RELEASE_CHECKLIST.md` and `docs/QUALITY_GATES.md` to pass.

At the end of each task, report: what changed; files affected; tests/commands actually run and their outcomes; manual or packaged-app checks performed; performance results if relevant; limitations and untested cases; known blockers; and whether release is **PASS** or **BLOCKED**. Be direct. Never hide uncertainty or failures.
