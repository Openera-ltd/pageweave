# PRD 14 — Kaggle, Model Benchmarking/Fine-Tuning and Checkpoint Recovery

## Parallel workstreams
A) app foundation and feature slices; B) lawful dataset collection/annotation; C) baseline evaluation and targeted fine-tuning; D) model export/package/integration/release validation. Start training/evaluation and software work in parallel when dependency, GPU and filesystem conditions permit. Use separate branches/worktrees and resource budgets. Experiments must not starve UI tests or corrupt application state.

## Model workflow
Benchmark existing models first. Measure accuracy/task quality, failure types, throughput, latency, size, RAM/VRAM, license, language coverage and deployability. Fine-tune a specialist only if error analysis proves a worthwhile gap and rights-cleared data exists. Keep baseline checkpoint. Use same blind test set, inspect failures, preserve acceptable runtime and size. Export optimized runtime and compare predictions to training framework. Never train a general-purpose chat LLM or foundation model from scratch for this initial app; no bundled chat LLM.

Specialist targets include OCR/layout, tables, OCR error correction, scan restoration/artifact classification, document classification/field extraction, selected translation directions, TTS/alignment, and image-to-editable object reconstruction.

## Run manifest
Record run ID, date, purpose, code commit, script/config hash, base model/checkpoint/hash/license, dataset manifest/licenses/split hashes, preprocessing version, runtime/libraries, hardware/GPU/RAM/VRAM, precision/seed, batch/accumulation, optimizer/scheduler/LR/step count, checkpoint interval, early stop, train/validation metrics, blind eval, peak memory, throughput, failures/NaNs/OOM, artifact checksums, model card and limitations.

## Resume and durable storage
Write checkpoints atomically at appropriate intervals. Preserve optimizer/scheduler and RNG state where necessary. Test forced interruption/resume and verify step/data-order/config continuation. Ephemeral Kaggle/local disk is a cache, not sole storage. Keep one known-good checkpoint; only delete an older valid copy after its successor's checksum and restore test pass. Upload on checkpoints, not every step; use retry/backoff, pending-upload queue and checksums. Never overwrite last-known-good in R2. Record checkpoint IDs and durable URI without credentials.

## Kaggle-specific safeguards
Use official CLI/API/secret mechanisms. Never log credentials or commit notebook outputs with secrets. Detect actual GPU, driver, RAM, storage, session/quota limits; fail usefully rather than assuming fixed hardware. Respect usage policies/resource quotas; no trick to bypass timeouts/quotas. Dataset/model downloads require cleared license/provenance. Export from temporary session to durable store only after integrity check. Never upload raw documents into public notebook outputs.

## Candidate model acceptance
Baseline vs fine-tuned on same validation/blind test; no leakage; approved data/model license; human failure review; quality improvement without critical regression; optimized inference test on supported Windows hardware; CPU fallback if claimed; memory/time budgets; clean reload/missing/corrupt model/cancel/offline checks; reproducible manifest and rollback path.

## Token/compute discipline
No unbounded agent/subagent loops, duplicate runs, restart loops or empty polling. Before launch check live process/artifacts, disk/GPU and last checkpoint. Define per-run GPU-hours, disk and API/token budget; stop or checkpoint safely at budget. Persist the next exact action in journal. Keep durable code/config/results so compute or Claude session interruption does not destroy progress.
