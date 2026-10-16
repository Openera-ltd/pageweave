# Specialist AI, External AI, and Resource Lifecycle

## Capability policy

All specialist AI capabilities that PageWeave promises are required product features, even when their resources must be downloaded during setup. A missing resource must produce a clear setup/recovery path rather than a fake working feature. Do not display raw model names as normal user-facing labels; technical versions and license details belong in engineering documentation and required notices.

Required specialist capability groups include:

- OCR for scanned pages, photos, screenshots, and embedded images, with language/script coverage tracked individually.
- Layout analysis, reading order, document element detection, table/form extraction, and confidence reporting.
- Translation across as many useful language pairs as can be supported and evaluated, including RTL layout and document text inside images where editable reconstruction is possible.
- Scan cleanup/enhancement and applicable rotation, skew, dewarp, and restoration workflows.
- Text-to-speech with synchronized word/segment highlighting and readable language/voice coverage.
- Voice cloning with safeguards, plus separately supported standard synthetic voices.
- Image-to-editable-design reconstruction using an appropriate component pipeline; do not assume one monolithic model can recover original layers.
- Evaluation/validation for conversion, missing text, layout shifts, overflow, and uncertain extraction.

Use proven models/frameworks first, verify exact versions and redistribution licenses, measure representative baselines, and fine-tune only when evaluation demonstrates a need. Keep evaluation data split so held-out documents do not leak into training. Record known language/direction limits, hardware needs, memory, latency, and quality metrics.

## External AI and MCP are optional

- No bundled general-purpose local chat LLM.
- Users may configure an external AI provider/API key or connect an external agent through MCP. These are distinct setup paths and should be described separately.
- External AI interprets instructions/plans allowed actions; PageWeave's specialist engines perform OCR, document conversion, translation, TTS, voice processing, and reconstruction.
- Without an external AI, all ordinary interface operations and local processing work. Natural-language planning through a chat interface may be unavailable, but the app must not gate ordinary features behind it.
- External providers require network connectivity and may have their own cost/privacy rules. Explain data transfer before transmitting files or text. Never upload documents invisibly.
- A connected agent can invoke only authorized typed tools and permitted files/operations; it does not get arbitrary shell or filesystem access.

## First-run resource experience

If required resources are missing on first launch, show a clear explanation and one primary action such as “Download All Resources.” Show estimated required disk space/download size before commitment when known; represent estimates as estimates. Provide progress, downloaded/total bytes, speed, ETA when measurable, current stage, pause/resume, cancel, retry, and detailed failure recovery. Keep individual model names out of ordinary UI.

Handle at least:
- no internet at first launch;
- connection loss, changing networks, unstable/slow network, DNS/TLS failures, rate limiting and unavailable servers;
- pausing, cancelling, app exit/restart, OS shutdown, sleep/wake, and interrupted writes;
- partial downloads, corrupted payloads, wrong checksums/signatures, archive bombs, and incompatible versions;
- insufficient disk, permission failures, locked files, antivirus interference, failed extraction/install, and dependency conflicts;
- missing optional resources, failed one-of-many resources, and upgrade rollback.

Requirements: use trusted sources; verify hashes and signatures where available; safely resume only verified ranges/checkpoints; use bounded temporary storage; never execute an unverified download; install atomically where possible; persist setup state; clean up safely; and explain what the user can do next. Do not say complete until required resources pass verification and smoke tests. If only part of the suite succeeds, identify which capabilities remain unavailable.

## Offline claim

Once required resources and runtimes are installed, verify actual network-disconnected workflows on a clean test machine. Core local features must work without network where the chosen dependencies permit it. Do not make unsupported offline claims for external providers, cloud-only sources, online updates, or features whose source content is not locally available. Test an airplane-mode/offline scenario and capture evidence.

## Voice cloning safeguards

- Require the user to confirm they have rights/authorization to use the voice samples; provide safeguards against impersonation and abusive use.
- Protect source recordings and voice profiles with clear storage/deletion controls. Do not include samples in logs/telemetry by default.
- Provide honest quality, language, storage, and hardware expectations. Do not promise a perfect clone or a fixed minimum recording duration without benchmark evidence.
- Make the synthetic/cloned nature of generated audio clear where appropriate, and consider provenance/watermarking where feasible.
- Ensure cancellation and deletion cover temporary samples, derived profiles, caches, and generated artifacts subject to the product's data-retention policy.

## Language support registry

Track three distinct dimensions: UI localization, OCR recognition, and translation directions. Track TTS voices/pronunciation as an additional dimension. For each language/script/direction, record test corpus, supported fonts and shaping, RTL behavior, quality metrics, common failure modes, and release status. “Hundreds of languages” must never be claimed merely because a model card lists that many languages.
