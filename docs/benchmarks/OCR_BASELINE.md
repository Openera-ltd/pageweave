# OCR baseline benchmark (2026-10-17)

Purpose: PRD 14 "benchmark existing models first" — choose PageWeave's OCR engine and decide whether
fine-tuning is justified. Harness and corpus: `ml/` (`pwml.corpus`, `pwml.bench`); scorer verified by
tests (a perfect engine scores 0 on every condition).

**Data.** Synthetic, rights-clean corpus v1 (authored text; OFL/Bitstream fonts with recorded
licenses), 200 dpi pages, 4 layouts × 9 scan conditions. Blind split: 72 pages / 3,109 lines /
171,361 characters; two in five pages use typefaces never used for training (Caladea). Validation
split: 36 pages / 1,497 lines, training fonts only. **Limitation:** synthetic only — no real scans,
photos, handwriting, non-Latin scripts or complex real-world layouts yet; real-world numbers will be
worse.

**Machine.** 4 CPU threads (cloud container), CPU only, Python harness. Timing excludes model warm-up.

## Blind split (sealed; evaluated once per engine)

| Engine | CER | WER | Line recall | Exact lines | False-positive lines | Median s/page | p90 s/page | Peak RSS |
|---|---|---|---|---|---|---|---|---|
| Tesseract 5.3.4 (LSTM, eng, psm 3) | 3.75% | 8.03% | 98.7% | 58.7% | 16 | 1.31 | 2.62 | ~60 MB |
| PP-OCRv6 small det+rec (ONNX Runtime 1.31, via RapidOCR 3.10) | **0.41%** | **1.18%** | **99.8%** | **93.8%** | 2 | 3.09 | 4.54 | ~820 MB (Python process incl. runtime) |

CER by condition (blind):

| Condition | Tesseract | PP-OCRv6 |
|---|---|---|
| clean | 1.00% | 0.02% |
| skew (0.7–3°) | 3.28% | 0.02% |
| jpeg (q10–30) | 0.89% | 0.04% |
| lowres (100–150 dpi) | 1.07% | 0.07% |
| blur | 2.68% | 0.11% |
| noise | 1.15% | 0.14% |
| badscan (combined) | 13.74% | 0.87% |
| faded | 0.98% | 1.00% |
| shadow | 10.30% | 1.57% |

By layout (PP-OCRv6): prose 0.09%, mixed 0.26%, invoice 0.58%, two-column 0.71%. Unseen typeface:
0.34% vs 0.59% on training typefaces (no generalisation penalty measured).

## Error analysis

- **Tesseract** fails mainly by *missing whole lines* where its global binarisation erases text
  (shadowed halves, bad scans) and on tables; misread characters are a minor share.
- **PP-OCRv6**: (1) missed or merged lines on low-contrast pages; (2) missing spaces
  ("probablybecome", "5.Of") — on validation WER is 8× CER; (3) script confusion from its
  18,708-character multilingual dictionary (German ß → Greek β, spurious accents);
  (4) l/I, rn/m-type confusions on bad scans; (5) dropped punctuation.

## Experiments (decided on the validation split)

| Variant | Val CER | Val WER | Notes |
|---|---|---|---|
| PP-OCRv6 defaults | **0.22%** | 1.76% | kept |
| detector without dilation | 0.24% | 2.07% | rejected |
| background normalisation before detection | 0.46% | 1.69% | rejected (faded 2.47%) |

Process note: dilation was first compared on blind faded/shadow pages, where it looked like a 40%
error reduction. That is tuning on the sealed split; the decision was redone on validation, where the
gain did not reproduce (it came from a few held-out-font pages). Blind results above are for the
untuned defaults only. Tuning happens on validation from now on.

## Conclusions

1. Adopt PP-OCRv6 small (det + rec, Apache-2.0 weights) on ONNX Runtime as PageWeave's OCR engine
   (decision D-009). Tesseract is ~9× less accurate on this corpus.
2. Fine-tuning is **not yet justified** by these numbers alone (0.4% CER). The measured, model-level
   gap is word spacing and Latin-script confusions; both first get cheap fixes (dictionary/script
   constraint, spacing post-processing) measured on validation. Fine-tuning the recogniser on
   Kaggle becomes worthwhile if a real-world, rights-cleared evaluation set shows a larger gap.
3. Speed: ~3 s/page on 4 threads is acceptable for a background job but must be measured in the Rust
   integration on Windows target hardware; memory must be re-measured outside Python.

## Product implementation parity (Rust `pw-ocr`)

`crates/pw-ocr` reimplements the pipeline in Rust on ONNX Runtime 1.31 (official Microsoft build,
loaded at runtime) with the **official** PP-OCRv6 small model files (pinned Hugging Face revisions,
`resources/ocr.lock.json`). The official files are functionally identical to the copies benchmarked
above (same validation result to the line); the processing settings in the official
`inference.yml` scored worse on validation (1.32% CER) than the measured configuration and were not
used. Scored by the same scorer via `crates/pw-ocr/examples/ocr_dir.rs` + `bench --rescore`:

| Split | Engine | CER | WER | Line recall | Exact lines | Median s/page |
|---|---|---|---|---|---|---|
| validation | reference (Python) | 0.218% | 1.76% | 100% | 92.4% | 2.2 |
| validation | pw-ocr, bilinear crops (first version) | 0.226% | 1.90% | 100% | 91.9% | 1.8 |
| validation | **pw-ocr, bicubic crops (final)** | 0.201% | 1.61% | 100% | 92.8% | 1.7 |
| blind (single run of the final version) | reference (Python) | 0.41% | 1.18% | 99.8% | 93.8% | 3.1 |
| blind (single run of the final version) | **pw-ocr** | 0.41% | 1.19% | 99.8% | 93.8% | 1.6 |

Per condition on blind the two agree within ±0.02 percentage points. The first Rust version was
worse on bad scans (validation WER 10.5% vs 8.7%); the cause was bilinear crop sampling, fixed by
OpenCV-style bicubic sampling. Same 4-thread container; Windows timing still to be measured.
