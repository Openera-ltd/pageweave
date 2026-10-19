# Page orientation (detect 0/90/180/270° turned scans)

Status: in the product as Pages ▸ Turn pages upright… and CLI `orient` (`pw-pdf/src/orient.rs`).

Measured with `PAGEWEAVE_OCR_DIR=third_party/ocr/linux-x64 cargo run -p pw-pdf --release --example orient_bench -- ml/out/corpus-v1/val`:
every val page (36) turned by 0, 90, 180 and 270° = 144 cases. Method: the page reduced to a 1000 px
long side is recognised in all four quarter turns by the shipped OCR models; only horizontal lines
count (confidence × characters); the best turn must beat the second by 1.5× and read ≥ 20 characters,
otherwise the page is left undecided.

| Variant (val, 144 cases) | Right | Wrong | Undecided | Time per page |
|---|---|---|---|---|
| All four turns, every line counted | 8 | 0 | 136 | 4.1 s |
| One reading picks the axis (wide vs tall lines), then 2 more | 142 | 2 | 0 | 2.8 s |
| Two readings (0°, 90°) pick the axis, then 1 more | 142 | 2 | 0 | 3.1 s |
| **All four turns, horizontal lines only (chosen)** | **144** | **0** | **0** | **4.1 s** |

**Blind split (72 pages × 4 turns = 288 cases), scored once on 2026-10-18 after the method was final:
288 right, 0 wrong, 0 undecided** (7.2 s per page measured while another build ran on the same CPU).

Why the shortcuts fail: the recogniser also reads tall boxes (it turns their crops), so counting all
lines makes sideways pages read as well as upright ones; and on skewed pages text a quarter turn off
still partly reads (val-2000020: 2076 upright vs 1170 a quarter turn off), so fewer than four readings
picked the wrong axis twice. A wrong turn is worse than no turn, so the slower variant was chosen.

A 640 px reading image was also tried: still 144/144, and no faster (4.7 s per page): the time is spent
recognising lines, not on image size, so 1000 px stays.

Not yet done: speed (4 OCR passes per page; try
a smaller reading image or the detector's line geometry first); real scans; pages with little text.
Timing: release build, Linux container, one thread.
