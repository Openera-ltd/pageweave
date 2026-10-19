# PageWeave — Master Product Requirements Document

**Status:** Binding specification, not proof of implementation. **Product:** Free/open-source, Windows-first desktop document studio.

## 1. Vision and audience
PageWeave is a local-first application for viewing, creating, editing, converting, repairing, comparing, protecting, reading, translating, analyzing and reconstructing documents. It aims to cover the offline-capable desktop PDF workflows people expect from Acrobat Pro, excluding collaboration and services whose core purpose requires cloud hosting, while adding more useful local document intelligence, synchronized read-aloud, and editable reconstruction of flattened designs. It is not an Adobe clone: learn from workflows, but use original branding, interaction design and appropriately licensed assets.

Audience: students/researchers (reading, search, annotation, citations, comparison, extraction, translation, narration); office/admin users (conversion, forms, merge/split, redaction, signatures, print); creators (posters/flyers/design reconstruction); accessibility users; and power users who need batch jobs, CLI, local API or MCP.

## 2. Binding product rules
1. Free and open-source; no ads, subscriptions, credits or paywalled features.
2. Core document work must not require an external AI account, API key, MCP agent, or connectivity after required resources are installed.
3. Do **not** bundle a general-purpose chat LLM. Required specialist capabilities (OCR, layout, tables, translation, restoration, speech and design reconstruction) are separate and remain product requirements. External AI can optionally interpret user intent and orchestrate typed operations; PageWeave's own engines execute them.
4. Documents, extracted text, tables, designs and voice recordings remain local by default. Never upload user content to cloud storage, telemetry, Kaggle, GitHub or an AI provider unless the specific operation is disclosed and explicitly authorized.
5. Preserve originals by default; use safe temp files, validated output copies, atomic writes where possible, undo and crash recovery.
6. Every visible control works or is clearly disabled with a useful reason. No dead buttons, fake progress, placeholder success, silent failures or incomplete controls presented as finished.
7. Never claim fidelity, quality, language coverage, signature trust, standard compliance, hardware support or production readiness without measured evidence.
8. Continue through implementation → testing → actual app use → error reproduction → root-cause repair → profiling → regression tests → packaging → release verification. Compilation or a demo is not the finish line.
9. Be bounded and resumable: no infinite recursive agent loops, duplicate model training, uncontrolled cloud spending, or empty token-consuming review loops. Use a journal, checkpoints, budgets, locks and durable evidence.
10. Training data must be rights-cleared for its intended use. “Found online” is not permission to use a PDF for model training.

## 3. Scope
Required families: PDF viewing/navigation/search/selection; create PDF; edit text and objects; annotations; page organization; metadata; links/bookmarks/attachments; export and conversion; compress/optimize; OCR and scan cleanup; forms and field authoring; local signatures and certificate-based digital signatures where supported; password protection; permanent redaction and sanitization; compare documents; measure and print; print production/preflight; accessibility tagging/checking; archival standards where actually validated; batch jobs; translation including text inside images; document classification and field/table extraction; local full-text index; optional external-AI Q&A with citations; TTS with word-synchronized highlighting and auto-scroll; voice profiles/cloning with consent; editable flattened-image reconstruction; customizable themes/shortcuts; diagnostics/logging; optional CLI/local API/MCP.

Supported interchange targets include PDF, DOCX, XLSX, PPTX, ODT, ODS, ODP, RTF, TXT, HTML and image formats, plus CSV for extracted tables—only where a real tested conversion path exists. Maintain a matrix for each import/export direction, limitations, licensing and test evidence.

## 4. Explicit exclusions and network boundary
Exclude cloud file storage, hosted team workspaces, shared/live review, live co-authoring, invitation-based comment collaboration, hosted signature requests/approvals, web collaboration rooms and cloud-only AI features. Local annotations and local form/signature features remain in scope.

Permitted optional network functions: resource installation and updates, verified app updates, approved model-artifact downloads, user-initiated downloads, explicitly configured external AI calls and externally initiated MCP access. Connectivity loss must not disable unrelated local features. Offline certificate verification must say whether revocation/chain status could be checked.

## 5. Required implementation loop
**A. Inspect:** read project instructions, this PRD suite, decisions, journal, repository, status/diff, builds, tests, licenses and packages. Run existing checks and establish baseline; distinguish pre-existing failures.

**B. Plan:** trace each requirement to components, interactions, tests, benchmark, resource, privacy/security issue and acceptance criteria. Record risk, unresolved decisions, compatibility and rollback.

**C. Build foundation:** stable typed engine operations, job queue, cancellation, safe output writes, structured errors/logs, settings migration, resource manager, resumable task journal, CI/test harness, performance instrumentation and watchdog.

**D. Vertical slices:** ship small complete features; write acceptance tests; test happy and failure paths; launch actual app; profile; inspect diffs; update docs and regression tests. Build app and baseline/evaluation training work in parallel when safe; never integrate a model only because training completed.

**E. Qualify:** run clean-install/upgrade/rollback/uninstall, offline, security/license/provenance, accessibility, performance, soak, real hardware and packaged-release tests. Mandatory missing/failed checks mean **BLOCKED**, not release.

## 6. Quality and performance targets
Define supported hardware only after validation. Measure cold/warm startup, idle/peak RAM, CPU/GPU, p50/p95/p99 UI latency, frame-time/jank, first-page and page-render latency, search, conversion/OCR/translation/TTS throughput, memory growth, cancel latency, cache sizes and long-session stability. Investigate UI stalls ≥50 ms; aim for ~60 FPS where feasible. Set numeric budgets from measured baselines, not guesses.

Output quality is multidimensional: valid files, correct page count/size, content coverage, text/table accuracy, object/text geometry drift, missing glyphs, overflow, structural fidelity, visual diff, OCR confidence, output reopenability and accessible structure. A composite score must not hide catastrophic errors.

## 7. Durable source of truth
Approved decisions in `docs/DECISIONS.md`, master PRD and domain PRDs, acceptance/test matrix, then implementation evidence. Conversation history is not a durable requirement store. If a newer user correction conflicts with an older doc, resolve in decisions and update all affected files. Never quietly change scope to make tests green.

## 8. Deliverables
Reproducible code/build; exact feature parity and format/language matrix; requirements-to-test evidence; benchmark corpus with rights provenance; training/fine-tuning/evaluation/checkpoint-resume tooling; signed/checksummed resource manifest; installer/updater/rollback; offline operation; structured logs and privacy-safe diagnostics; API/MCP/CLI schemas/tests; documentation and dependency/model/license inventory. Do not commit tokens, credential screenshots, user PDFs, unauthorized source corpora, private voice samples or unlicensed weights.

## 9. Release report
Identify commit/package/OS/hardware, tests actually run, pass/fail/skip counts, benchmarks versus baseline, raw artifacts, regressions, untested areas, license/security findings, clean install/offline/recovery proof and explicit **PASS** or **BLOCKED**. “Production ready” is forbidden while mandatory gates are unverified.
