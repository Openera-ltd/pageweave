# Interaction Contract: Every Input Has a Deliberate Outcome

This document defines expected interaction patterns. Before implementing a new surface, add feature-specific exceptions and acceptance tests; do not silently violate the shared contract.

## Universal interaction contract

Every interactive element must declare: accessible name and purpose; enabled/disabled condition and reason; left/right/middle-click behavior where relevant; keyboard/focus behavior; hover/pressed/selected/loading/success/error states; duplicate-action prevention; feedback; cancellation; undo/recovery; persistence; and tests. A visible control with no implemented action is a release blocker. If an action is unavailable, disable it visibly and explain why with concise text or a tooltip.

## Pointer behavior

### Left click

- Activates an enabled button/menu command once; provide immediate visual feedback.
- A second click while a non-repeatable action is already running must not enqueue a duplicate job unless that behavior is explicitly intended.
- Clicking a tab switches to it. If the previous tab has unsaved work, preserve it; never silently discard content.
- Clicking text starts or adjusts selection according to the document engine's selection model. Clicking a draggable object should not accidentally edit text or initiate an unrelated drag.
- Clicking outside a popover closes it only when appropriate; clicking inside must not close it before the action completes.

### Right click and context menus

Context menus must be relevant to the target and preserve the user's selection unless the product explicitly defines another behavior. If the user right-clicks within an existing multi-selection, do not silently clear that selection. If they right-click elsewhere, use a consistent documented selection/focus behavior.

- **Selected text:** Copy, Search, Translate, Read Aloud, Highlight/Annotate, Comment, Explain when enabled, and Redact when the document/edit mode supports a safe permanent-redaction flow. Only show commands valid for the current document and selection.
- **Unselected text or text cursor:** Select All, Find, reading actions, and other relevant document commands; do not show selection-only actions as enabled.
- **Image/object:** Open/view, copy where supported, save/export where permission permits, inspect properties, replace/annotate, or edit/select object. Avoid presenting text-only actions as valid by default.
- **Table/cell:** Copy table/cell, select row/column/table, extract/export where supported, and relevant formatting actions if editable.
- **Page thumbnail:** Open, select, rotate, crop, duplicate, extract, delete, insert/merge/split as supported. Destructive actions need clear labeling and undo/confirmation appropriate to their impact.
- **Tab:** Close, close others, reopen closed tab where feasible, pin or save session where implemented, and show unsaved status. Never lose unsaved edits.
- **File/list item:** Open, reveal location, rename/move only when supported, batch-select, remove from list versus delete from disk must be unambiguous.
- **Background/canvas whitespace:** Relevant zoom/view/layout commands; do not show invalid object or text actions.
- **Running job:** Show valid pause/resume/cancel/open-output/details/retry commands for its current state. Never show impossible state transitions as enabled.
- **Text selection while narration is running:** Context action should use the new selection without corrupting the narration state; explain whether Read Aloud replaces or queues the current selection and implement one consistent rule.

Menus must fit inside the active monitor/window, avoid clipping at screen edges, support keyboard navigation, expose focus, close predictably with Escape/outside click, and not appear underneath the pointer in a way that makes mis-clicks likely. Do not use right click as the only way to reach an essential command.

### Double-click, middle-click, wheel, drag and drop

- Define double-click behavior per target (for example word selection, opening a file, or renaming a tab only where appropriate). Do not assign conflicting outcomes to the same gesture.
- Middle-click must not accidentally close data or trigger an undocumented command.
- Mouse wheel scrolls the expected region based on pointer/focus, with standard modifiers for zoom where implemented. Avoid scroll-jacking and inconsistent nested-scroll behavior.
- Dragging text, objects, tabs, page thumbnails, files, and panel dividers must have visible affordances, a valid drop target, clear success/failure feedback, and safe cancellation. Invalid drops must not delete or lose the dragged item.
- External file drag/drop validates format, access, duplicate files, size, and whether the target operation supports the file. Show clear feedback for rejected files.

## Selection behavior

- Selection is visually clear, readable in all themes, and keyboard accessible. Copy must preserve meaningful text and line order as far as the document structure permits.
- Selecting text exposes contextual tools without covering selected content or obscuring the reading area. Toolbars/popup menus reposition to remain visible and disappear when selection is intentionally cleared or navigation makes it stale.
- OCR text that is uncertain should not be silently treated as unquestionably accurate. Where appropriate, show confidence or an editable correction path.
- Selecting image text, table cells, page objects, or multi-page regions must not be mistaken for native text unless the app can support the promised operation.
- Translate/Explain/Extract results must distinguish generated output from source text and preserve a way to compare with the original.
- Permanent redaction requires a selection review, explicit confirmation, creation of a sanitized output, verification that hidden text/content was actually removed, and an audit/result report. Merely drawing a black rectangle is never sufficient.

## Keyboard contract

Maintain a single documented shortcut registry. Include common system conventions and shortcuts for file open/save/close, undo/redo, search, navigation, zoom, selection, page actions, reading controls, command search, and accessibility. The exact bindings must be tested against the selected framework and OS.

- Every command available through a menu should expose its shortcut where one exists.
- Users can customize bindings, search commands, restore defaults, and see conflicts. Prevent or clearly resolve duplicate assignments and avoid hijacking text-entry behavior.
- Support Escape/cancel, tab/focus navigation, Enter/Space activation, arrow-key menu navigation, and standard editing keys where appropriate.
- Shortcuts must respect focus context: for example, Ctrl+A inside a search field should select its text, while Ctrl+A on the document canvas may select document content.
- Do not intercept OS-reserved shortcuts or break screen-reader commands without a deliberate, documented reason.

## Buttons, toggles, forms and feedback

- Buttons have default, hover, focus-visible, pressed, disabled, loading, success, and error states as applicable. Icon-only buttons require accessible names and tooltips; labels are concise and consistently placed.
- Toggle state must be obvious and persist only when it represents a durable preference. A switch must not trigger an unrelated navigation or irreversible operation.
- Sliders show meaningful values and units. Inputs validate inline, retain recoverable text on errors, support keyboard operation, and explain what needs correction.
- Use non-blocking notifications for routine success; use persistent, accessible messages for errors that require action. Do not flood users with duplicate notifications.
- Destructive, irreversible, privacy-sensitive, or overwrite actions require safeguards proportional to risk. Avoid pointless confirmation dialogs for safe, reversible actions.
- Never use fake delays to make progress look animated. Indeterminate progress must be identified as indeterminate; ETA appears only when there is enough data to estimate credibly.

## Tabs, windows, dialogs, and unsaved work

- Preserve open documents, scroll positions, relevant selections, panel/workspace preferences, and active tab where reasonable.
- On closing a modified document/window, explain the options to save, discard, or cancel. Default focus should avoid accidental destructive action.
- Escape closes the topmost dismissible surface or cancels the active operation according to a defined priority. It must not silently cancel data edits or close the document unexpectedly.
- Dialogs have initial focus, focus trapping when modal, a safe default action, keyboard shortcuts, screen-reader semantics, and correct restoration of focus on close.
- Notices (toasts) sit beneath the modal layer: while a modal dialog is open they are dimmed with the page and never cover or intercept the dialog's controls. Errors that belong to a dialog are shown inside it.
- Support multiple monitors, varied window sizes, high-DPI scaling, mixed display scaling, and display edge placement without clipped menus/dialogs.

## Theme and settings behavior

- Offer light, dark, system, accent color, compact/comfortable density, adjustable UI text sizing, reduced motion, shortcut customization, reading/audio preferences, default output behavior, privacy/storage controls, and reset-to-default functionality where implemented.
- Use design tokens; do not hard-code isolated colors that break custom themes. Persist settings safely and validate/migrate settings between versions. Bad or missing settings must recover to safe defaults without crashing.
- Theme previews should apply immediately where safe, include all states and dialogs, and remain legible in document selection, warning, disabled, and progress states.
- Settings that affect a running operation must define whether they apply now or to future jobs. Never silently corrupt in-progress work after a setting changes.

## TTS-specific interaction

The Read Aloud selection must be explicit. Playback controls must accurately reflect playing/paused/loading/stopped/error states. Word highlights must follow actual speech timing or reliable alignment, including punctuation and difficult words. Auto-scroll keeps the current text in view while yielding to manual scrolling according to a consistent rule. Read-from-here, selection replacement, pause/resume, speed/voice changes, language switching, seek, and end-of-document behavior must be tested. Keyboard and screen-reader behavior must not conflict with speech playback.

## Resource installer states

Provide explicit first-run/setup, checking prerequisites, waiting for network, downloading, verifying, installing, paused, retrying, cancelled, complete, partial failure, offline, insufficient disk, corrupt file, unavailable source, restart recovery, and update states as appropriate. Resume from verified checkpoints; re-verify bytes after resume. Never show completion until every required resource has passed integrity and installation checks. Keep model names hidden in ordinary UI; user-facing labels should describe capabilities/resources plainly.

## The interaction inventory requirement

For each screen and feature, maintain a table of target/gesture → expected action → state/availability → feedback → failure/cancel/undo → keyboard/accessibility → persistence → test ID. A feature is not complete until its inventory and tests agree with the shipped implementation.
