# Translation baseline (D-022)

Measured, not estimated. Automatic metrics only; PRD 08 also requires blind human review, which has
not been done yet.

- Test set: FLORES-200 devtest (1012 sentences per language; CC BY-SA 4.0; downloaded from
  `dl.fbaipublicfiles.com/nllb/flores200_dataset.tar.gz`, SHA-256
  `b8b0b76783024b85797e5cc75064eb83fc5288b41e9654dabc7be6ae944011f6`). Not used for training or tuning.
  OPUS-MT models were not trained on FLORES (their training data is OPUS); overlap has not been checked
  sentence by sentence.
- Models: the pinned int8-quantized OPUS-MT ONNX files in `resources/translate.lock.json`; greedy
  decoding, one sentence at a time (as the app does), before masking of protected tokens was added.
- Scoring: sacreBLEU 2.x defaults (`sacrebleu ref -i out -m bleu chrf`).
- Command: `PAGEWEAVE_OCR_DIR=third_party/ocr/linux-x64 PAGEWEAVE_TRANSLATE_DIR=third_party/translate
  cargo run --release -p pw-translate --example eval -- en de eng_Latn.devtest out.txt`.
- Environment: Linux x64 cloud container, 4 vCPU Intel Xeon @ 2.10 GHz; runs overlapped with builds,
  so speeds are lower bounds.

| Direction | BLEU | chrF | Sentences/s |
|---|---|---|---|
| en → de | 27.8 | 60.7 | 2.00 |
| de → en | 35.4 | 63.6 | 1.47 |
| en → fr | 40.1 | 67.1 | 1.57 |
| fr → en, en → es, es → en, en → it, it → en | not yet measured (run stopped at its time limit) | | |

Known behaviour found while testing (not in these scores): short fragments starting with a name can
lose their start (en → de "PageWeave fixture page 1" → "Seite 1"); PageWeave now masks names, codes and
addresses and, if a mask is dropped, translates the pieces around it separately.

## Model choice for D-027 (2026-10-24)

Same test set and scoring, **first 200 sentences** of FLORES-200 devtest per language; greedy
decoding through PageWeave's own runtime (`examples/eval` with `PAGEWEAVE_EVAL_PAIR_DIR` pointing at a
converted model; multilingual models get their target token). Models converted locally with
`tools/convert_translate.py` (int8). Speeds are not comparable (several runs overlapped).

| Direction | Base OPUS-MT (D-022) | tc-big (one pair) | Chosen model | Chosen BLEU / chrF |
|---|---|---|---|---|
| en → fr | 37.6 / 65.3 | 50.5 / 71.7 | mul-itc | 47.9 / 70.4 |
| en → es | 21.2 / 53.4 | 27.7 / 56.4 | mul-itc | 27.0 / 55.7 |
| en → it | 24.5 / 56.4 | 28.9 / 59.1 | mul-itc | 29.0 / 58.7 |
| fr → en | 31.9 / 61.5 | 40.9 / 66.2 | itc-mul | 39.3 / 65.0 |
| es → en | 23.5 / 55.6 | 30.0 / 59.9 | itc-mul | 28.1 / 58.9 |
| it → en | 24.9 / 56.8 | 32.6 / 61.8 | itc-mul | 32.0 / 60.9 |
| en → de | 27.6 / 60.8 | — | gmw-gmw (tc-big) | 36.6 / 64.0 |
| de → en | 33.3 / 62.7 | — | gmw-gmw (tc-big) | 39.1 / 65.4 |
| de → it | through English | — | mul-itc | 25.8 / 54.7 |
| fr → es | through English | — | mul-itc | 23.0 / 51.8 |
| es → fr | through English | — | itc-mul | 28.7 / 57.6 |
| it → de | through English | — | itc-mul | 24.3 / 55.3 |

- mul-itc: `Helsinki-NLP/opus-mt-tc-bible-big-deu_eng_fra_por_spa-itc` (Apache-2.0).
- itc-mul: `Helsinki-NLP/opus-mt-tc-bible-big-itc-deu_eng_fra_por_spa` (Apache-2.0).
- gmw-gmw: `Helsinki-NLP/opus-mt-tc-big-gmw-gmw` (CC-BY-4.0).

The one-pair tc-big models score up to 2.6 BLEU higher on a few directions, but eight of them need
about 2.7 GB and still translate the other pairs through English; the three chosen models need about
900 MB and translate all 30 directions between English, German, French, Spanish, Italian and
Portuguese directly. Portuguese and the remaining direct pairs: see the next table when measured.
