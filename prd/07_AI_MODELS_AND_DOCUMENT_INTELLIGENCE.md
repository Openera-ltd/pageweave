# PRD 07 — AI Models and Document Intelligence

## Product boundary
Required specialist capabilities are not the same as a local chat model. Do not bundle a general-purpose chat LLM. External AI providers/MCP agents can interpret requests and orchestrate typed operations, but PageWeave's engines perform conversion, OCR, translation, TTS and reconstruction. Ordinary UI actions work with no external agent.

## Required capabilities
OCR; layout and reading order; table structure/cell extraction; scan quality detection and restoration; multilingual document translation; local TTS plus reliable speech alignment; user-consented voice profiles/cloning; image-to-editable-design segmentation/object/text/vector reconstruction; classification/structured field extraction; optionally OCR-error correction only if it beats deterministic baselines and does not corrupt protected tokens.

## Model choices
Benchmark mature models before training. Candidate families previously discussed include PaddleOCR/PaddleOCR-VL (OCR/document parsing), Docling (processing framework, not one model), Table Transformer (tables), MADLAD-400-3B-MT (translation baseline), OPUS-MT direction-specific models and proven speech/vision runtimes. These are candidates, not approved dependencies. Verify the exact current checkpoint card, code and weight licenses, tokenizer, data restrictions, redistribution/commercial rights, maintenance, OS/runtime compatibility, download size, RAM/VRAM and benchmark results. A model's code license does not necessarily cover its weights/data. Reject non-commercial or ambiguous models by default for a general free/open-source app unless the owner explicitly accepts a clear limitation.

Avoid training foundational OCR, multilingual translation or a general LLM from scratch initially. Fine-tune only a specialist component after error analysis shows a gap and lawful training data exists. Optimize/quantize only when measured speed/size gains preserve acceptable output quality.

## Lifecycle
Resource registry stores capability ID/version, language coverage, compatibility, expected size, checksum/signature, license/provenance ID and readiness—not user-facing filenames. Load on demand; bound threads/GPU memory/concurrency; unload when appropriate; release caches; support CPU fallback where genuinely supported; recover from OOM, GPU loss and corrupted weights. No specialist operation blocks the UI. Crash-isolate workers where practical. Logs record versions/latency/resources/quality signals without document content.

## Document intelligence UX
- Local full-text index and search do not depend on chat or network after indexing. Index jobs are cancellable, resumable, versioned and deletable.
- Optional multi-document Q&A cites exact file/page/region and clicking citation navigates there. State uncertainty and “not found in sources”; no fabricated citations.
- AI-assisted edits show a reviewable diff and output plan before commit. Risky operations require confirmation. LLM-generated plans are untrusted inputs validated against typed operation schemas, path permissions and network policy.
- Request such as “translate these 20 PDFs into French and create editable Word copies” produces a plan with files, options, destinations, estimated resource/network needs, warnings and collision policy. User can edit before start.
- External AI receives only minimum necessary selected content and only after clear consent. If no provider is configured, hide/disable natural-language orchestration while local tools remain available.

## Evaluation and promotion
Use fixed train/validation/blind-test splits and prevent duplicate/near-duplicate leakage. Log dataset provenance, base checkpoint and hashes, config, commit, runtime, seeds, environment, quality, latency, RAM/VRAM, throughput, model size and known failure examples. Compare on same corpus. Promote `experimental → evaluated → candidate → approved → packaged`; only approved, verified artifacts ship. Test interruption/resume, optimized-runtime parity and rollback.
