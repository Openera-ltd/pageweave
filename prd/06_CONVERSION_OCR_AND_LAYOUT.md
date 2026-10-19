# PRD 06 — Conversion, OCR, Layout Understanding and Scan Restoration

## Conversion is reconstruction, not file renaming
Maintain a matrix for each import/export direction: fonts, tables, images, formulas, links, comments, forms, metadata, macros, tags, layout, RTL, size, batch support and limits. Use proven Office/document engines; never execute macros. Unsupported/damaged/encrypted paths are clear and safe.

## PDF → Word and editable exports
Extract glyphs/words/lines and style, geometry, columns, reading order, tables/cells, captions, footnotes, headers/footers, page breaks and links. Rebuild editable paragraphs/runs, tables, sections/columns, page size/margins, lists, hyperlinks and image placement. Never rasterize every page into a Word document except in explicitly labeled visual-only mode. Detect outlined text, scans and missing fonts. Compare text, page count, object positions, layout/visual differences, overflows, clipping and missing content. Provide preview/report and disclose losses.

## Spreadsheet/table extraction
Detect borders and borderless tables, merged cells, repeated headers, multi-page/rotated/nested tables, footnotes and totals. Preserve structure and cell values. Guard leading zeros, identifiers, long numbers, decimal separators, dates and locale formatting from automatic type corruption. CSV's loss of formatting/sheets must be explained. Validate cell-level values and table structure against ground truth.

## OCR and scan cleanup
Recognize searchable/image-only PDFs, scans, photos, screenshots and page images. Detect orientation; rotate, deskew, crop, dewarp, denoise, despeckle, remove shadows, normalize contrast and binarize where useful while preserving meaningful color. Produce line/word/block/page geometry, reading order, script, language and confidence. OCR overlay lets users compare text with image. Keep OCR original layer distinct from corrected text. Low-confidence names, dates, monetary figures, legal text and serial numbers require review; never silently “correct” them. Handwriting is a distinct capability that needs its own benchmark.

## Layout understanding
Detect headings, paragraphs, lists, columns, tables, figures/captions, footnotes, headers/footers, page numbers, formulas, form fields, objects and reading order where tested. Record confidence and source region. Test academic multi-column pages, reports, invoices, manuals, forms, magazines, scans, mixed scripts and low-res/skewed content.

## Quality scorecard
Report separate measures: parse validity; page count/size; text coverage/edit distance; OCR confidence/error; table structure/value accuracy; object count; geometry drift; visual difference; font fallback/missing glyphs; overflow/clipping; accessibility/tags; output size and reopenability. A composite must not mask one catastrophic dimension. Thresholds come from annotated representative test sets, never invented post hoc.

## Failures to test
Corrupt files, unsupported compression, oversized images, missing fonts, encryption, partial OCR, OOM, GPU loss, worker crash, office engine crash, unsupported shapes, conversion drift, invalid output, interrupted write, full disk, path collision, stale index/cache and cancellation. Each needs specific behavior, useful message and regression test.
