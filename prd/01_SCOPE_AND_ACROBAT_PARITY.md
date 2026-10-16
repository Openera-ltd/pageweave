# PRD 01 — Offline-Capable Acrobat-Style Feature Inventory

## Scope rule
Use current official Acrobat documentation to discover workflows, then implement comparable user jobs with original branding and legally reusable dependencies. Do not copy Adobe's UI assets, icons or trade dress. Maintain one feature-by-feature matrix. This is a comprehensive target list; it does not assert that any feature is implemented already. A gap in the first chosen library is not permission to remove a feature. Research alternatives, combine mature engines or implement a safe adapter. If a real technical/legal/safety blocker remains, document evidence and obtain explicit owner approval before marking it deferred.

## A. View, navigate and inspect
- Open via dialog, drag/drop, recent files, file association and command line; multi-document tabs; restore sessions safely.
- Single/facing/continuous views; fullscreen/presentation/read mode; fit page/width/visible; custom zoom; rotation; page thumbnails; outline/bookmarks; page labels; document properties; metadata; PDF version; fonts; encryption; tags; signature status; embedded attachments.
- Search text across page/document/index: case/whole-word options, next/previous, counts where computable, highlight matches and jump to geometry. Include OCR text when available and distinguish OCR-derived results.
- Select/copy text with Unicode correctness and sensible reading order; select objects/annotations; handle multiple columns, rotated pages, CropBox/MediaBox, mixed sizes, missing fonts and huge files.
- Safely handle links, embedded files and active actions. Never auto-run JavaScript, macros, executables or external launch actions.

## B. Create and convert PDFs
- Create PDFs from supported Office/OpenDocument/text/HTML/image files, multiple-file sequences, clipboard images and scanned image sets; OS Print-to-PDF where supported.
- Image-to-PDF batch settings: order, orientation, page dimensions, margins, fit/fill, DPI/quality, OCR and metadata. Blank/template creation only where a real implementation exists.
- PDF portfolios/containers, page packages, embedded files, 3D, audio/video and rich-media objects are part of the parity audit. Find maintained local readers/editors or a safe feature-specific implementation. Never execute active content. If secure rendering/editing is technically blocked after research, preserve the source, give an exact warning and record a release-blocking gap until owner-approved deferral; never return blank success.
- Convert among PDF, DOCX, XLSX, PPTX, ODT, ODS, ODP, RTF, TXT, HTML and supported image formats. Extract tables to CSV/XLSX. Keep per-direction format fidelity matrix; do not claim conversion just because extension changed.
- Preserve editable structure where the format supports it: paragraphs/runs, tables/cells, headers/footers, columns, footnotes, links, page breaks, image positioning, formulas, styles, metadata and reading order. No flattening whole pages to images unless the user explicitly chooses a visual-only export.
- Compress/optimize with output estimates honestly labeled; explicit image quality/DPI profiles; verify output validity, content, image quality and real final size.

## C. Edit and annotate
- Edit/add text and images where source allows; change styling/position; replace/crop/rotate images; edit links/objects; vector shape, line, arrow, rectangle, ellipse, freehand and object selection; warn on font substitution, outlined text and complex layout reconstruction.
- Highlight, underline, strikethrough, sticky note, free-text note, drawing, callout, stamps/custom stamps, local document comments/annotation list. Search/filter/sort annotations, edit/delete/export supported annotations, set author/opacity/color, and flatten only with clear warning.
- Local review is in scope; online shared review/live sync/invitation workflows are excluded. Support local annotation import/export and local shared review files only where interoperable formats are supported.

## D. Organize pages
Merge/combine; split by range, bookmarks, size or N pages; extract, insert blank/from file, replace, duplicate, reorder via drag/drop or commands, rotate, crop, delete, page labels, bookmarks/outline hierarchy/destinations, headers/footers, page numbering, Bates numbering, background, watermark, attachments, metadata, safe form flattening and page-box management. Range syntax `1-3,5,8-10` requires rigorous validation of 0, negative, empty, repeated, descending, overlap and out-of-range cases. Destructive actions preserve source by default and report every output.

## E. Fill forms and signatures
- Detect/fill/reset AcroForm fields: text, check, radio, choice, date/known field format; field order, tooltip, required status, validation, import/export data and save/reopen persistence.
- Author form fields with name/type/default/required/accessible label/format/validation/tab order. Unsupported XFA or other forms are detected and explained; no fake support.
- Typed/drawn/image appearance signatures and initials; separately implement certificate-based cryptographic signatures when a validated engine supports them. Distinguish a visual signature from a digital signature.
- Verify cryptographic integrity and trust state; show unknown signer, expired certificate, changed-after-signature, revocation-not-checked and network-unavailable separately. No hosted send-for-signature or online approval workflow. Do not assert universal legal enforceability.

## F. Protect, redact, sanitize
Password encryption and open-password/permission settings where supported, compatibility warnings, safe decrypt flow, hidden-data sanitization, metadata cleanup, action/script/attachment cleanup and sensitive-pattern detection. Permanent redaction must remove underlying text/object data—not draw a black rectangle—then produce a new file by default and independently test text extraction, search, copy/paste and residual object/metadata paths. Failed verification blocks success. User reviews false positives before removal.

## G. Compare, measure, print, preflight
- Compare two documents with page alignment even after insertions/deletions; text additions/deletions, moved blocks, format and visual differences; side-by-side/overlay, difference list and jump actions.
- Measure distance/perimeter/area/angles if engine supports it; unit and per-page calibration visible, persistent and testable.
- Print page/range, copies/collate, portrait/landscape, actual size/fit/shrink, tile/booklet only if implemented, color/grayscale, comments/stamps, duplex/printer properties via native OS printing.
- Print production: output preview, color-space/separation/overprint/ICC conversion, image DPI, embedded fonts, transparency, bleed/trim boxes, preflight profiles/fixups and PDF/X workflows only to the degree the chosen engine genuinely supports. Every check states rule/version/severity/evidence; rerun after fix.

## H. Accessibility and archival
Inspect/edit tags/structure tree, reading order, headings/lists/tables/links, alt text, language/title metadata, form labels and accessibility checks. Separate automated checks from manual review. Support PDF/A, PDF/X, PDF/E and PDF/UA-related creation/validation only if exact validator/profile has been tested. Never infer compliance from extension or a green unqualified result.

## I. Utilities and advanced behaviors
OCR/searchable PDF; deskew/despeckle/dewarp/denoise; remove blank pages with preview/threshold; extract images; repair/normalize scanned pages; compare; optimize; batch rename, convert, redact, watermark, OCR, extract and translate; attachments; stamps; metadata; file properties; page boxes; action wizard/presets for repeatable local workflows.

## Parity tracking
For each item record a stable ID, workflow, dependency/version/license, supported formats/constraints, offline classification, UI states, acceptance and failure tests, quality/performance measure, resource requirements, known limitations, test evidence and status (`Not assessed`, `Planned`, `Implemented`, `Verified`, `Blocked`, `Deferred by decision`). Do not claim “all Acrobat features” until the inventory audit is current and all agreed in-scope offline features are verified or explicitly deferred by the product owner.
