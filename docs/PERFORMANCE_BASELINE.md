# Performance Baseline

Measured numbers only; every entry names its environment. These are **baselines, not budgets** — budgets
must be set from measurements on real minimum/recommended Windows hardware (`docs/HARDWARE_SUPPORT.md`),
which has not been done yet.

## 2026-10-16 — engine baseline (synthetic documents)

- Environment: Linux x64 cloud container, 4 vCPU Intel Xeon @ 2.10 GHz, 15 GB RAM; release build;
  PDFium chromium/7881 via pdfium-render 0.9.4; commit after `7db1bf3`.
- Command: `BENCH_PDFIUM_DIR=third_party/pdfium/linux-x64/lib cargo run --release -p pw-pdf --example bench -- fixtures/pdf/single-page.pdf large-1000.pdf OUT 20`
  (`large-1000.pdf` from `python3 -I tools/fixtures/gen_fixtures.py --big 1000 large-1000.pdf`, 380 KB).
- **Caveat:** fixtures are text-only Helvetica pages. Real documents with images, embedded fonts, vector
  art and transparency will be slower; a representative, rights-cleared corpus is still needed (PRD 13/15).

| Operation | p50 | p95 | n |
|---|---|---|---|
| Engine init | 0.52 ms | — | 1 |
| Open + inspect, 1 page | 0.40 ms | 0.51 ms | 20 |
| Open + inspect, 1000 pages (incl. SHA-256 of file) | 16.45 ms | 17.80 ms | 10 |
| Render page 1 at 1.5× (≈ 100% zoom on a 1× display) | 5.03 ms | 5.57 ms | 20 |
| Render page 1 at 3× | 18.93 ms | 21.90 ms | 20 |
| Render page 1000 of 1000 at 1.5× (re-opens document) | 8.73 ms | 13.55 ms | 20 |
| Search “PWMARK” across 1000 pages | 43.32 ms | 57.07 ms | 10 |
| Extract 100 of 1000 pages to a verified, committed file | 3.91 ms | 52.67 ms | 10 |
| Peak RSS for the whole run | 44.5 MB | | |

Observations / follow-ups:
- Each page render re-opens the document (simple and memory-bounded). Cost is small for these files;
  re-measure with complex real documents before deciding whether an open-document cache is needed.
- Extract p95 is dominated by fsync of the committed file.
- Not yet measured: app cold/warm start, time to first visible page in the app, scroll frame times,
  memory over long sessions, Windows numbers. These are release blockers under PRD 12.
