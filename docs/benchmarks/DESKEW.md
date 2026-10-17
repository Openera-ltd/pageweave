# Skew measurement (Straighten pages)

Measured with `cargo run -p pw-pdf --release --example skew_bench -- <split dir>` on the synthetic,
rights-clean corpus (`ml/pwml/corpus.py`, corpus-v1), whose pages record the exact skew applied
(`params.rotate_deg`, conditions `skew` and `badscan`). Pages are downscaled to a 1400 px long side,
like the engine's measuring render. Algorithm: Otsu threshold, projection-profile sharpness over
-15…15° (0.5° steps, then 0.05° around the best), confidence ≥ 1.15; pages measuring under 0.2° count
as straight (0.1° when the table below was measured).

| Split | Skewed pages | Mean abs error | Max error | Straight pages turned | Unmeasured |
|---|---|---|---|---|---|
| val (36 pages; developed on) | 8 | 0.013° | 0.020° | 0 of 28 | 2 |
| blind (72 pages; scored once, 2026-10-17, after the algorithm was fixed) | 16 | 0.017° | 0.060° | 1 of 56 (measured −0.10°, at the threshold: a 0.1° turn) | — |

Threshold change after scoring: straightening a straightened copy re-measured its pages at ±0.1° (the
measurement's precision on re-rendered content) and turned them again, so the straight threshold was
raised from 0.1° to 0.2° (found on the fixture, not on blind data). From the blind run's recorded
values no straight page would be turned at 0.2°, but that is derived from the single blind run, not a
new one.

Timing: 6–110 ms per page (release, Linux container, one thread).

Limits: synthetic pages only (printed text, 0.7–3° skews); no real scans, handwriting, photos of
documents or pages dominated by pictures yet. Pages turned by 90/180° are not detected here (that
is orientation, not skew).
