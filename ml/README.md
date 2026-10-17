# PageWeave ML workspace

Benchmarking, data preparation and (when justified) fine-tuning for PageWeave's specialist models,
following PRD 14 (benchmark first, fine-tune only on a measured gap), PRD 15 (data rights) and
PRD 16 (credentials). Nothing here is shipped in the app; chosen models are integrated separately
with their own license and verification records.

## Layout

| Path | What |
|---|---|
| `pwml/textgen.py`, `pwml/corpus.py` | Deterministic synthetic OCR corpus: authored text, vendored fonts, 4 layouts × 9 scan conditions, line-level ground truth with the exact degradation transform |
| `fonts/`, `manifests/fonts.json` | Fonts used for rendering (OFL-1.1 / Bitstream Vera), with license texts and SHA-256; FreeFont (GPL) excluded; Caladea held out of training |
| `pwml/metrics.py` | CER/WER, line recall, false positives; OCR boxes mapped back through the inverse transform |
| `pwml/engines.py`, `pwml/bench.py` | Baseline engines behind one interface; benchmark runner with raw-output cache (`--rescore`) |
| `pwml/rights.py` | Training gate: every item needs an approved rights record; the sealed blind split is never trainable |
| `pwml/kaggle_run.py`, `config/budget.json`, `kaggle/` | Budgeted Kaggle runs via the official CLI; per-run manifests in `runs/` |
| `tests/` | Scorer, corpus, rights gate and Kaggle safety tests (run in CI and `tools/check.sh`) |

## Quick start

```sh
python3 -m venv ml/.venv && ml/.venv/bin/pip install -r ml/requirements-bench.txt   # + apt: tesseract-ocr tesseract-ocr-eng
cd ml
.venv/bin/python -m pwml.corpus out/corpus-v1 --blind 72 --val 36          # ~0.5 s/page
.venv/bin/python -m pwml.bench out/corpus-v1 --split blind --engine tesseract --engine ppocr --details --out out/bench-v1
python3 -m unittest discover -s tests -t .
```

`out/` and `.venv/` are not committed; corpora regenerate bit-identically from the code and fonts.

## Splits

`train` and `val` use only training fonts. `blind` (sealed with a `SEALED` marker) mixes all fonts and
puts two in five pages in typefaces never seen in training. Every 36 consecutive pages of a split
cover each layout × condition once. Seed ranges are disjoint (train 1 000 000+, val 2 000 000+,
blind 3 000 000+).

## Kaggle

Credentials are read only from the environment (`KAGGLE_API_TOKEN`, or `KAGGLE_USERNAME` +
`KAGGLE_KEY`) — add them as environment secrets; never paste them into chat, files or notebooks.

1. `python -m pwml.kaggle_run preflight` — confirms the CLI and that credentials work (read-only call).
2. `python -m pwml.kaggle_run publish-dataset out/corpus-train --slug pageweave-ocr-corpus-v1` —
   private dataset; refuses the sealed split and anything failing the rights gate or the size budget.
3. `python -m pwml.kaggle_run push kaggle/probe --purpose "environment probe"` — records the real
   GPU, driver, RAM, disk and libraries, and verifies the corpus hashes, before any training design.
4. `wait RUN_ID`, then `pull RUN_ID` — outputs are accepted only if every file matches `SHA256SUMS`.

Budgets (`config/budget.json`): ≤ 3 GPU-hours per run (enforced server-side with `--timeout`),
1 concurrent run, ≤ 2 GB per dataset. Raising them is a human decision (PRD 16).
