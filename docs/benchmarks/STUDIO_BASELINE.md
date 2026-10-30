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

Shapes (D-031) on `fixtures/pdf/badges.pdf` (300 × 200 pt page): circle and ellipse rebuilt as
ellipses, the rounded button as a rounded rectangle (radius 34.7 px; the drawn 16 pt is 33.3 px),
the square box with radius 0, the white "SALE 50%" as text on it; no pictures. PDF export mean
pixel difference 1.05/255. The poster is unchanged (2.21/255).

Words over a picture (D-032) on `fixtures/pdf/caption.pdf` (white bold 30 pt and regular 14 pt
over a gradient picture): both lines recognised, white; inside each line's box the picture differs
from `caption-clean.pdf` (same page, no words) by 0.20/255 (64.65 and 34.86 before painting); no
white pixels left; PDF export mean difference 2.56/255.
