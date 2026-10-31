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

Font family (D-034), `python3 -m pwml.fontbench` (corpus pages rebuilt with the debug CLI; lines
matched by wording). Val split (36 pages, tuning): 91.8 %. Blind split (72 pages, held-out fonts,
measured once): 82.6 % serif-vs-sans over 2,562 lines (always-sans 35.8 %).

| Condition | clean | lowres | blur | noise | jpeg | faded | shadow | skew | badscan |
|---|---|---|---|---|---|---|---|---|---|
| Blind accuracy | 99.7 % | 93.9 % | 96.1 % | 93.3 % | 96.9 % | 96.8 % | 83.3 % | 29.2 % | 49.8 % |

By font (blind): Inter 99.3 %, DejaVu Sans 97.7 %, Carlito Bold 97.5 %, Carlito 94.1 %, DejaVu Serif
93.8 %, Caladea Italic 75.4 %, Caladea 70.1 %; DejaVu Sans Mono kept sans 92.9 %. Fixtures
(`fonts.pdf`, `fonts-holdout.pdf`): 22/22 families right, sizes within 7.5 %.

Frames (D-036) on `fixtures/pdf/frames.pdf`: 3 pt rectangular border and 2 pt elliptical ring
rebuilt as outlines (6.1 px / 4.2 px against 6.25 / 4.17), text inside kept; PDF export mean
difference 2.22/255. Paragraphs (D-035): poster PDF export 2.23/255.

## Real pages rebuilt from pictures (tools/corpus/studioquality.py, 2026-10-31)

Pages rendered at 144 dpi and rebuilt with `pageweave design` (debug build). Recall: words of the
editable text against the page's own text; export diff: mean absolute difference per channel
between the picture and the design's PNG export.

| Page | Words | Recall | Text / shapes / pictures | Export diff |
|---|---|---|---|---|
| Chromium web page | 402 | 0.980 | 22 / 9 / 8 (was 30) | 12.2 |
| LibreOffice slides | 3 | 1.000 | 2 / 0 / 0 | 1.1 |
| LibreOffice report | 506 | 0.988 | 20 / 0 / 4 | 10.5 |
| W3C table | 42 | 1.000 | 35 / 0 / 1 | 2.0 |
| NIST SP 800-63-3 cover | 171 | 0.994 | 30 / 1 / 2 | 8.6 |
| IRS Publication 15 cover | 302 | 0.815* | 45 / 3 / 28 (was 495) | 8.7 |

\* includes an invisible proof slug that is not on the page. Fixed while measuring: wording on page
background inside a picture's box showed twice (it stayed opaque in the picture); paragraph glyph
scale from the median let lines run past their original width (now the smallest ratio); a line in
another script inflated its paragraph's size (now the median); table rules inside a line's box
counted as glyphs (taller lines, false serifs) and darkened the background painted behind text on
shaded bands. Pictures that touch (within 3 px, small pieces within 4× their size) are merged — the IRS cover's dot leaders made 466 one-dot pictures. Known: some sans lines
are still read as serif; text-dense pages differ mostly by font metrics (built-in fonts).
