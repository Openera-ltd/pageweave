# UX, Visual Design, Motion, and Accessibility

## Design goal

Create a professional, compact, modern desktop experience that feels deliberate and responsive. Prioritize the real document canvas and users' tasks. Avoid generic AI dashboards, overly large cards, noisy gradients, random emoji, inconsistent icons, excessive borders, and paragraphs explaining each control.

## Visual research workflow

Before a major UI design/rework:
1. Identify mature products and patterns relevant to that workflow (for example PDF editors, Office applications, document scanners, screen readers, professional design tools, and desktop settings).
2. Research publicly accessible references and screenshots. Download/inspect images where permitted and available; do not claim to have inspected an image that was not actually viewed.
3. Record useful patterns (information hierarchy, canvas/toolbar arrangement, selection tools, navigation, menu behavior, errors, keyboard discovery, animation) and what PageWeave will adapt.
4. Do not copy proprietary branding, icons, paid assets, or exact screen designs. Use a coherent original design system.
5. Validate the design against the actual workflows, edge states, accessibility and target display sizes—not only a static screenshot.

## Visual system

- Default visual direction: restrained blue accent, neutral surfaces, high text legibility, compact professional controls, and consistent iconography.
- Define shared tokens for spacing, type scale, line height, border, radius, elevation, focus rings, colors, opacity, motion, and layers. Use them consistently.
- Prefer alignment, spacing, typography, and subtle separators over boxes around everything. Reserve strong color for meaningful action/state.
- Use one coherent icon pack and consistent sizes/strokes. Do not replace missing icons with random emoji. Icon-only controls need accessible labels and tooltips.
- Do not put explanatory paragraphs below each toolbar item. Use clear names, familiar symbols, discoverable tooltips, accessible descriptions, and searchable documentation.
- Make hierarchy clear: document canvas first, contextual tools second, secondary panels optional/collapsible. Avoid letting AI/assistant surfaces crowd core document work.

## Themes and density

Support light, dark, and system modes; configurable accent color; compact and comfortable density; accessible contrast; scalable interface text; high-contrast considerations; and reduced motion. Ensure custom themes apply across every screen, popup, selected text, disabled control, validation message, canvas tool, and installer state. Preferences must survive restart and upgrade. Theme previews should not create a separate inconsistent UI implementation.

## Motion and feedback

- Motion communicates cause/effect, hierarchy, orientation, and state—not decoration.
- Use short, purposeful transitions (often roughly 120–220 ms for simple UI changes as a starting point; tune against actual feel and framework behavior). Avoid hard-coded timing that makes controls feel slow.
- Respect the OS reduced-motion preference and provide an app-level reduced-motion control. Prefer fades or immediate changes when motion is reduced.
- Never delay a command's execution or completion until an animation finishes. No artificial spinners, fake progress, flashing, jarring focus jumps, layout shifts, or endless skeletons.
- Keep animations interruptible; rapidly repeated input must not stack stale animations or leave state incorrect.
- Target smooth scrolling and around 60 FPS where feasible on supported hardware. Measure frame pacing and long frames in the actual packaged app; do not use “60 FPS” as an unverified claim.

## Layout and responsiveness

- Support resizable windows, small laptop screens, multiple monitors, high DPI, 100–200% display scaling where available, mixed scaling, and long localized labels. Define minimum viable window dimensions from tested workflows.
- Prioritize visible document content and keep toolbars/panels compact. Let secondary panels collapse or resize without breaking document space.
- Place popovers and context menus within visible bounds. Tooltips must not cover critical content, flash immediately, or prevent pointer travel.
- Virtualize large lists and page thumbnails; avoid rendering hidden/unneeded content.
- Provide meaningful empty, loading, error, offline, partial-result, no-match, unsupported-format, and permission-denied states. Never rely on a blank panel to signal failure.

## Accessibility

- Complete keyboard navigation and visible focus across the application, editor, dialogs, context menus, settings, and installer.
- Semantic names/roles/states for assistive technologies, logical reading/focus order, scalable UI text, sufficient contrast, and no color-only status.
- Respect system preferences for reduced motion and where feasible high contrast.
- Focus indicators must be visible in all themes and not be removed for aesthetic reasons.
- Use accessible names for controls and announce job status/errors appropriately without over-announcing progress.
- Verify text selection, table navigation, reading aloud, highlighting, and keyboard shortcuts alongside screen-reader behavior.

## Visual QA

Capture screenshots of key screens/states at representative resolutions and display scales. Compare for layout regressions, clipping, overlap, inconsistent spacing, contrast failures, and text wrapping. Screenshot snapshots are evidence, not a replacement for interaction testing. Review both normal and error/disabled/loading states.

## Brand identity

- Official logo: `docs/assets/pageweave-logo.png` (master, 1254 × 1254) and `docs/assets/pageweave-logo-512.png`
  (README). A white page with text lines wrapped in blue-to-violet layers on a near-black background.
- App and installer icons (`app/src-tauri/icons/`) are generated from the logo cut to a rounded square:
  `npx tauri icon <rounded-square.png> -o src-tauri/icons` (run in `app/`; delete the android/ios folders,
  which PageWeave does not ship).
- The interface accent (`#2f6fde`) stays the default; the logo's blues and violet are its brand range.
