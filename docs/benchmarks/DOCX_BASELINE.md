# PDF → Word (DOCX) baseline

Measured 2026-10-22 on the Linux development container (4 vCPU Intel Xeon @ 2.10 GHz, debug build),
LibreOffice from the container's Ubuntu packages as the reader. Synthetic fixtures only: these
numbers guard against regressions; they are **not** a quality claim for real-world documents (a
representative, rights-cleared corpus with annotated ground truth is still needed — PRD 06/15).

## Method (`crates/pw-pdf/tests/docx.rs`)

1. Convert each fixture with `convert_to_docx`.
2. Independent reader in the test: the Word file holds exactly the PDF's characters (whitespace excluded).
3. LibreOffice opens each Word file headless and exports it to PDF.
4. Compare with the original: page count; word-sequence similarity (Myers diff, 2·same / (a+b));
   ink coverage per page at 36 dpi with a 3-pixel tolerance, both directions (original ink found in
   the new rendering, new ink found on the original); the worst page counts.

## Results

| Fixture | Pages | Words | Worst page ink coverage | Notes |
|---|---|---|---|---|
| article.pdf | 3 → 3 | 1.0000 | 0.794 | page 2: table borders not carried over (reported); pages 1 and 3 ≥ 0.985 |
| five-pages.pdf | 5 → 5 | 1.0000 | 1.000 | |
| uncompressed-report.pdf | 4 → 4 | 1.0000 | 0.996 | |
| photo-heavy.pdf | 4 → 4 | 1.0000 | 1.000 | page 4 excluded: its only picture is a stamp comment (not carried over, reported) |
| tagged-accessible.pdf | 1 → 1 | 1.0000 | 1.000 | |
| mixed-sizes-rotated.pdf | 3 → 3 | 1.0000 | 1.000 | page 3 laid out unturned so its text reads upright (reported); excluded from ink and word order |
| search-terms.pdf | 3 → 3 | 1.0000 | 1.000 | |

Test guards: words ≥ 0.99 for all; ink ≥ 0.78 (article), ≥ 0.85–0.90 (others).

## Conversion time (debug build, same machine)

| Fixture | Time |
|---|---|
| article.pdf (3 pages, 2 columns, picture, link) | 46 ms |
| uncompressed-report.pdf (4 pages, 160 lines) | 87 ms |
| five-pages.pdf | 9 ms |

| 175 pages (article + report merged 25×; CLI, whole process) | 3.3 s, peak memory 46 MB |

Release-build and Windows timings, and image-heavy large documents, are not yet measured.
