# Design Studio baseline (D-028)

Measured 2026-10-30 in the Linux development container (4 vCPU, debug build of `pageweave`, OCR on
CPU), `pageweave --json design <input> -o out.pwdesign`. Wall time of the whole command (start-up,
render or decode, recognition, analysis, project write and read-back); peak RSS of the process.
Not yet measured on Windows hardware or with a release build (expected faster).

| Input | Pixels analysed | Time | Peak RSS | Result |
|---|---|---|---|---|
| `fixtures/pdf/poster.pdf` | 833 × 1042 | 2.4 s | 352 MB | 6 text, 3 shapes, 1 picture; editable 0.90 |
| `fixtures/pdf/photo-heavy.pdf` | 150 dpi page | 5.8 s | 509 MB | 9 pictures, no text |
| `fixtures/pdf/scanned.pdf` | 150 dpi page | 6.0 s | 641 MB | 70 text lines, mean confidence 0.999 |
| poster rendered at 6× (PNG), scaled to the 2400 px limit | 1920 × 2400 | 16.3 s | 781 MB | same objects as the poster |

Quality on the poster fixture (`crates/pw-pdf/tests/studio.rs`): word recall 1.0 (19/19), no
invented words; box IoU banner 0.996, button 0.989, picture 0.992, 3-pt rule 0.892; sizes within
12 %; weights right; PDF export mean pixel difference 2.2/255 against the source.
