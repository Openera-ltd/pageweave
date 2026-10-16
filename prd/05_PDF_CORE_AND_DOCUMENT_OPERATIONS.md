# PRD 05 — PDF Core, Editing, Forms, Signatures and Print Production

## Viewer and document model
- Preserve original bytes when no change is needed. Safely parse metadata before rendering and enforce resource/recursion limits on hostile files.
- On-demand/tiled render, visible-page priority, bounded caches, invalidation after zoom/rotation/edit. Text hit-testing aligns with rendered glyphs under zoom, rotation and crop. Handle mixed page sizes, MediaBox/CropBox, transparency, fonts, colors and annotations.
- Local search index stores normalized text and geometry. OCR-derived matches are marked when relevant. Search opens exact page/region. Recent files keep paths/metadata, not content; missing/moved files recover gracefully.

## Editing and saving
- Explicit edit modes where needed; cursor/selection affordances reveal current mode. Edit text/style/position and image/vector object only where source/library supports it. Warn about missing/unembedded fonts, outlined text, complex columns and reconstruction changes.
- Save to a staged temp output; flush, parse and validate; commit atomically where possible. Preserve source and a recovery journal. Detect another process modifying the file; let users reload, compare or save a copy rather than silently overwrite.
- Digital signatures may be invalidated by edits: warn before action and keep signed original available. Undo/redo semantics and non-reversible operations are explicit.

## Forms
Fill/clear/reset AcroForm text, checkbox, radio and choice fields; manage known date formats; validate required fields; import/export supported field data; preserve tab order and values after save/reopen. Author fields with name/type/default/required/tooltip/accessible label/format-validation/tab order. Imported field data is untrusted; preview mapping. Support XFA only with proven safe implementation; otherwise detect and report limitations.

## Signatures and trust
Visually typed/drawn/image signatures are not equivalent to cryptographic digital signatures; label them honestly. Certificate workflows must protect private keys, support supported algorithms, record integrity hash, use correct incremental saves where required and revalidate after writing. Trust states: valid+trusted under policy, integrity-valid/untrusted signer, invalid, modified after signing, expired, revocation unchecked, network unavailable, unsupported. Offline validation cannot pretend revocation is known. No hosted signature request/approval workflows and no blanket claims of legal enforceability.

## Passwords, redaction and sanitization
Password settings must independently reopen/decrypt-test. PDF permission restrictions are not absolute security. Permanent redaction has distinct marked → previewed → applied → verified states; write a new file by default and independently inspect text extraction, search/copy-paste, objects/images, metadata and incremental revisions where applicable. Black rectangles are not redaction. Sanitization explicitly lists metadata, actions/scripts, attachments, hidden layers, annotations/form data and other supported removals. Failure to verify blocks success.

## Page operations
Merge/combine; split by ranges, size, bookmarks or N pages; extract, insert, replace, reorder, duplicate, rotate, crop, delete, label/bookmark, header/footer, page number, Bates numbering, watermark/background, attachments, metadata and page boxes. Handle mixed page sizes, bookmarks/metadata collisions and encrypted inputs. All outputs named predictably; collision policy explicit; never silently overwrite source. Range parser tests whitespace, empty/overlapping/repeated/descending/out-of-range/zero/negative values.

## Print and preflight
Print ranges/copies/orientation/scale/color/comments and printer settings through OS APIs; tiles/booklet only if supported. Print production: output preview, color/ICC/separation/overprint, font embedding, image resolution, transparency, trim/bleed boxes, preflight rules/fixups and PDF/X as actually supported. Each check cites exact rule/version, severity, page/object, evidence and fix. Changing color/boxes requires preview and explicit destination. PDF/A, PDF/X, PDF/E and PDF/UA conformance need exact versioned profile and independent validator; rerun after fixing. Never infer compliance from extension or optimistic UI.

## Compare and accessibility
Compare text/objects/visuals/formatting/metadata; align pages through insertions/deletions, filter/navigate differences and state false-positive limits. Accessibility workflows inspect/edit tags, reading order, headings/lists/tables/links/alt text/language/title/form labels; distinguish automated checks from manual-review items.

## Acceptance
Save/reopen every operation; use independent parsing/rendering where possible. Verify output validity, untouched-content preservation as promised, error/cancel/recovery/disk-full, performance, and truthful quality reports.
