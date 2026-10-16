# PRD 02 — Interaction Contract and User Workflow Catalogue

## Universal rule
Every input has a defined result, immediate feedback, availability condition, next step, cancellation, recovery, undo, keyboard equivalent, accessible state and automated/reproducible test. Define behavior by hit-target and current context—not just by screen.

## Pointer and focus
- Left click performs the stated primary command. Double-click follows a predictable platform convention; text double-click selects a word unless edit mode specifies otherwise. Middle-click is only assigned if documented and tested.
- Right-click opens a context menu for the actual hit target and current selection. Right-clicking existing selection preserves it; menu stays in work area, avoids obscuring the target where possible, supports arrows/Enter/Escape and visibly disables unavailable operations with a reason.
- Wheel scrolls; Ctrl+wheel may zoom; Shift+wheel may horizontal-scroll. Respect trackpads and don't steal scroll unexpectedly.
- Drag/drop has clear insertion targets and feedback for files, pages, images, tabs and objects. Invalid drops are explained; no implicit overwrite. Handles remain accurate at zoom/DPI and while scrolling.
- Hover feedback is subtle; stable tooltip delay; no vital information only on hover. Focus is visible, tab order logical and focus return predictable. Escape dismisses/cancels the innermost safe operation first. Background completion must not steal focus.
- Menus/dialogs reposition near screen edges and handle resize, minimize, multiple monitors, mixed DPI and screen readers.

## Selection-specific actions
When text is selected, offer a compact contextual toolbar/context menu where it helps: Copy, Search, Translate, Read Aloud, Highlight, Underline, Strikethrough, Note/Comment, Redact, Extract/Quote, and Explain/Summarize only when optional external AI is enabled. Avoid duplicating every command in every toolbar. Text, image/object, annotation, page thumbnail, bookmark, form field and batch-row selections each get distinct relevant actions.
- Selection across columns/pages follows known reading order; if ambiguous, show a correction/region selection option.
- Copy preserves Unicode and bidi text.
- Translate previews source/target and never silently overwrites originals.
- Read Aloud starts at selection and synchronizes highlight to actual audio alignment, not a drifting timer.
- “Mark for redaction” is distinct from “Apply permanent redaction”; final apply confirms consequences, creates new output by default and must verify removal.

## Context menus by target
Canvas blank: pan/select/zoom/add annotation/view settings. Text: copy/find/translate/read/annotate/redact/edit only when supported. Image: select/copy/export/save/crop/replace/transform/properties. Annotation: edit properties/copy/delete/flatten with warning. Page thumbnail: insert/replace/duplicate/extract/rotate/crop/move/label/delete. Bookmark: rename/reorder/nest/delete/set destination. Form field: fill/clear/validate/properties. Batch row: retry/cancel/remove/open output/error details. Every item maps to a working command and test.

## Keyboard and command registry
Use one central command registry with stable IDs, descriptions, default shortcuts, context, enablement, dispatch, accessibility name and tests. Support customization, collision detection/resolution, reset defaults, import/export, search-command palette, keyboard-only operation and documented behavior in text fields. Baseline conventional bindings include Ctrl+O, Ctrl+S, Ctrl+Shift+S, Ctrl+P, Ctrl+F, F3/Shift+F3, Ctrl+Z, Ctrl+Y or Ctrl+Shift+Z, Ctrl+C/V/X, Ctrl+A (context-sensitive), Ctrl+W, Ctrl+Tab and Escape. Confirm platform conventions before finalizing. Test non-US keyboard layouts, IME/input methods, accessibility and shortcut conflicts; never trigger app commands accidentally while typing.

## Every control's state model
Relevant states: default, hover, pressed, focus-visible, selected, disabled-with-reason, loading, success, recoverable error, terminal error, cancelling, cancelled, offline, missing resource, permission-required, read-only. No inert control may look active. Prevent double submission/stale results. Buttons that launch long work show the real progress/cancel state and are safely debounced.

## Dialog and progress contract
Dialogs have clear title, consequence, safe default, action ordering, focus behavior, keyboard behavior, Escape semantics and return-focus destination. Dangerous actions never default to destructive confirmation. Progress uses actual measurable phases; shows bytes/items/size/speed/ETA only where valid; ETA is explicitly an estimate; 100% appears only after integrity/validation/commit. Cancellation behavior is truthful and partial successes are reported.

## Essential end-to-end scenarios
1. First run with Internet, no Internet, download interrupted/restarted, pause/resume, insufficient disk, invalid checksum, missing resource, retry/cancel.
2. Open valid/large/encrypted/corrupt/unsupported file; edit/undo/redo/save-as/reopen; close with unsaved changes; recover from crash.
3. Batch import, select preset, destination/collision policy, run, pause where safe, cancel, retry failed rows, open output folder, export summary.
4. Select text → Translate/Read/Annotate/Redact; verify selection persistence, correct scope, no data loss, no external transmission unless permitted.
5. TTS continues across page/document boundaries, handles manual scrolling, audio-device removal, speed changes, resume after restart and unavailable voice.
6. Theme/density change, shortcut collision, window resize, high DPI, language/RTL change, file lock, disk full, worker crash, OOM/GPU loss, cancellation race and external file modification.

## Acceptance registry
Every displayed actionable control must map to a command/action ID, tooltip/accessibility name, click/keyboard semantics, disabled reason, state transitions, error/retry behavior, undo/persistence rules and test ID. Build static/runtime audits to detect controls with missing handlers and tests. Don't use icon decoration as a substitute for functionality.
