# PRD 10 — Flattened Image to Editable Design Reconstruction Studio

## Purpose and limitation
Reconstruct posters, flyers, certificates, menus, infographics, forms, invitations and screenshots into independently editable objects. Flattened images do not contain original layer metadata; PageWeave infers likely components. Preserve source, expose uncertainty and never claim recovered original layers unless they exist in the source.

## Pipeline
1. Import image/PDF page and normalize display without mutating original.
2. Detect background, text, raster assets, logos, line art, geometry, borders, icons, shadows and layout groups.
3. OCR wording and infer text style/font/alignment/line breaks/boxes with confidence.
4. Segment foreground/background and independent objects where practical; retain masks/assets.
5. Rebuild text as editable text, shapes as vectors where reliable and images as separately selectable assets. Output must not merely be a newly flattened raster.
6. Provide object/layer tree: rename, select, visibility, lock, reorder, group/ungroup, snap, align/distribute, move/resize/rotate/crop/mask and duplicate.
7. Original vs reconstruction side-by-side, opacity overlay/blink and visual difference. Uncertain areas are marked in diagnostics, not covered by paragraphs all over the canvas.
8. Allow text correction, font replacement, color/gradient, outline, shape and image replacement, background edit, position/size and object merge/split.
9. Export editable SVG or tested layered project plus PDF/image formats where supported. Validate sanitized SVG and font redistribution rights.

## Editing interactions
Standard select and transform handles; zoom/pan; fit; grid/guides/margins/bleed; keyboard nudging and modifier keys; multi-select; undo/redo; z-order; grouping; predictable cursor and snapping. Hit-testing must work at zoom and high DPI. Drag/drop and keyboard alternatives are required. Font fallbacks are visible. Uncertain objects may remain raster but still be separately editable assets.

## Quality metrics
Measure OCR word error, object precision/recall, segmentation, box overlap, visual similarity, editable-object ratio, structural reconstruction, after-edit stability and export correctness. A visually close flat image fails if the promised output is editable. Test stylized fonts, low-res/noisy scans, mixed text/images, transparency, gradients, logos, shadows, tiny text, irregular aspect ratios and overlapping elements. Mark uncertainty; never invent logos or wording as fact.

## Resources and security
Heavy vision work is off the UI thread and memory-bounded. OOM/GPU loss is recoverable. Model resources use verified installation. No document upload by default. Embedded metadata, SVG, fonts and external image references are untrusted and sanitized.
