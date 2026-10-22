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
