# PRD 03 — Visual Design, Customization, Motion and Accessibility

## Direction
Compact, polished, calm, blue-accented professional desktop software. Content first. Avoid generic AI dashboard styling, oversized cards, unnecessary gradients, large empty panels, random emoji, and explanatory paragraphs under every icon. Use concise labels, recognizable icons, contextual tooltips and separate searchable help.

## Research-first UI
Before major UI work, research current Acrobat desktop plus respected PDF editors, Office editors and Windows-native patterns. Find and inspect real screenshots/images and documented workflows; note URLs/date and the specific interaction lesson. Use references for inspiration only—never copy proprietary assets, logos, icons or exact trade dress. Verify appearance in the real packaged app rather than relying on a mockup.

## Design system
Use centralized semantic tokens for surfaces, canvas, text/muted text, border, accent blue, focus, success/warning/error, typography, spacing, radius, elevation, icon sizing, motion and density. Built-in themes: Light, Dark, Follow System. Allow user accent color, Compact/Comfortable density, display/text scaling, reduced motion, tooltip preferences, configurable toolbar, show/hide panels, saved workspace and reset defaults. All settings persist safely; version and migrate settings. If theme preview is live, Cancel must restore the exact previous values. Theme/imported settings are schema validated, never trusted blindly.

## Layout/navigation
Clear document tabs, canvas, page thumbnail/outline pane, compact primary tool rail, context-relevant properties/annotation/search panels and a status area for page/zoom/job state. Panels hide/resize predictably; don't open every pane by default. Include Home/recents, viewer/editing, page tools, conversion/batch, OCR/scan, translation, design reconstruction, read aloud, form/signatures, redaction/security, print production, setup/resources, settings and diagnostics. No screen may have unfunctional navigation or dead placeholder controls.

## Icons and typography
Use one maintained icon system with consistent size/stroke/optical alignment and semantic action IDs. Icon-only buttons require accessible names and tooltips. No random emoji as functional icons. Keep UI labels legible and information-dense. Do not theme/recolor document page content unless in an explicit reading/recolor tool.

## Motion and responsiveness
Use purposeful, short, interruptible transitions that clarify state changes. Respect system reduced-motion setting and let users disable nonessential motion. Animation must never delay input, error display, cancellation or progress. Avoid jitter, flicker, layout jumps, excessive spring/bounce, whole-page repainting, fake loading delays and long decorative animations. Start feedback immediately; long work uses real progress. Aim at ~60 FPS where feasible and measure p50/p95/p99 frame time and long tasks on supported hardware.

## Accessibility
Every essential workflow is keyboard-operable. Visible focus; logical tab order; correct roles/name/value/state; screen-reader text; sufficient contrast; scalable UI text; no color-only semantics; reduced motion; accessible validation/errors; screen-reader-friendly custom controls and progress reporting. Accessibility checkers must distinguish automatic results from manual review. App accessibility and PDF accessibility are separate scopes.

## Visual QA
Test supported resolutions, DPI scales, theme/density and window sizes; inspect default/hover/focus/pressed/disabled/loading/error/selected states. No clipping, unreadable labels, off-screen dialogs, menus hidden beyond edges, misaligned hit targets, flicker, scroll capture bugs or focus loss. Store QA screenshot artifacts for major changes. A polished mockup with inert controls fails acceptance.
