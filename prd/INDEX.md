# PageWeave PRD Index

Read `00_MASTER_PRD.md` first. The PRDs supplement root `CLAUDE.md` and `docs/` engineering standards.

- [`00_MASTER_PRD.md`](./00_MASTER_PRD.md) — PageWeave — Master Product Requirements Document
- [`01_SCOPE_AND_ACROBAT_PARITY.md`](./01_SCOPE_AND_ACROBAT_PARITY.md) — PRD 01 — Offline-Capable Acrobat-Style Feature Inventory
- [`02_USER_WORKFLOWS_AND_INTERACTIONS.md`](./02_USER_WORKFLOWS_AND_INTERACTIONS.md) — PRD 02 — Interaction Contract and User Workflow Catalogue
- [`03_DESIGN_SYSTEM_AND_MOTION.md`](./03_DESIGN_SYSTEM_AND_MOTION.md) — PRD 03 — Visual Design, Customization, Motion and Accessibility
- [`04_ARCHITECTURE_AND_LANGUAGES.md`](./04_ARCHITECTURE_AND_LANGUAGES.md) — PRD 04 — Architecture, Language Boundaries and Engine Contracts
- [`05_PDF_CORE_AND_DOCUMENT_OPERATIONS.md`](./05_PDF_CORE_AND_DOCUMENT_OPERATIONS.md) — PRD 05 — PDF Core, Editing, Forms, Signatures and Print Production
- [`06_CONVERSION_OCR_AND_LAYOUT.md`](./06_CONVERSION_OCR_AND_LAYOUT.md) — PRD 06 — Conversion, OCR, Layout Understanding and Scan Restoration
- [`07_AI_MODELS_AND_DOCUMENT_INTELLIGENCE.md`](./07_AI_MODELS_AND_DOCUMENT_INTELLIGENCE.md) — PRD 07 — AI Models and Document Intelligence
- [`08_TRANSLATION_LOCALIZATION_AND_LANGUAGE_SUPPORT.md`](./08_TRANSLATION_LOCALIZATION_AND_LANGUAGE_SUPPORT.md) — PRD 08 — Translation, Localization and Script Integrity
- [`09_TTS_VOICE_AND_READING_ACCESSIBILITY.md`](./09_TTS_VOICE_AND_READING_ACCESSIBILITY.md) — PRD 09 — Read Aloud, TTS, Voice Profiles and Reading Accessibility
- [`10_DESIGN_RECONSTRUCTION_STUDIO.md`](./10_DESIGN_RECONSTRUCTION_STUDIO.md) — PRD 10 — Flattened Image to Editable Design Reconstruction Studio
- [`11_SECURITY_PRIVACY_AND_DATA_PROTECTION.md`](./11_SECURITY_PRIVACY_AND_DATA_PROTECTION.md) — PRD 11 — Security, Privacy, Trust and Data Protection
- [`12_PERFORMANCE_OBSERVABILITY_AND_LOGGING.md`](./12_PERFORMANCE_OBSERVABILITY_AND_LOGGING.md) — PRD 12 — Performance, Logging, Telemetry and Ten-Minute Watchdog
- [`13_TESTING_ACCEPTANCE_AND_RELEASE_GATES.md`](./13_TESTING_ACCEPTANCE_AND_RELEASE_GATES.md) — PRD 13 — Test Strategy, Acceptance Evidence and Release Gates
- [`14_KAGGLE_TRAINING_AND_MODEL_EXPERIMENTS.md`](./14_KAGGLE_TRAINING_AND_MODEL_EXPERIMENTS.md) — PRD 14 — Kaggle, Model Benchmarking/Fine-Tuning and Checkpoint Recovery
- [`15_DATASET_ACQUISITION_AND_RIGHTS.md`](./15_DATASET_ACQUISITION_AND_RIGHTS.md) — PRD 15 — PDF Dataset Acquisition, Rights and Data Quality
- [`16_CREDENTIALS_GITHUB_CLOUDFLARE_AND_SECRET_HANDLING.md`](./16_CREDENTIALS_GITHUB_CLOUDFLARE_AND_SECRET_HANDLING.md) — PRD 16 — Kaggle, GitHub, Cloudflare R2 and Credential Safety
- [`17_API_CLI_MCP_AND_OPTIONAL_EXTERNAL_AI.md`](./17_API_CLI_MCP_AND_OPTIONAL_EXTERNAL_AI.md) — PRD 17 — Local API, CLI, MCP and Optional External AI
- [`18_FIRST_RUN_RESOURCE_INSTALL_AND_OFFLINE_MODE.md`](./18_FIRST_RUN_RESOURCE_INSTALL_AND_OFFLINE_MODE.md) — PRD 18 — First-Run Setup, Resource Manager and Offline Behavior
- [`19_AUTOMATION_BUILD_LOOP_AND_TASK_JOURNAL.md`](./19_AUTOMATION_BUILD_LOOP_AND_TASK_JOURNAL.md) — PRD 19 — Continuous Build/Test/Fix Loop and Durable Project Memory
- [`20_ROADMAP_AND_ACCEPTANCE_MATRIX.md`](./20_ROADMAP_AND_ACCEPTANCE_MATRIX.md) — PRD 20 — Roadmap, Status Model and Launch Acceptance
- [`21_REFERENCE_AND_RESEARCH_REGISTER.md`](./21_REFERENCE_AND_RESEARCH_REGISTER.md) — PRD 21 — Design, Capability and Dependency Research Register

## Source-of-truth and conflict rules
- Master PRD states overall product boundary; domain PRDs set detailed acceptance. Root `CLAUDE.md`, `docs/QUALITY_GATES.md` and `docs/RELEASE_CHECKLIST.md` are mandatory engineering/release rules.
- Newer explicit user decisions supersede old assumptions; record decisions in `docs/DECISIONS.md` and update affected specs.
- PRDs are requirements, not proof of implementation. Reconcile against live code/tests and never delete existing work just to match the docs.
