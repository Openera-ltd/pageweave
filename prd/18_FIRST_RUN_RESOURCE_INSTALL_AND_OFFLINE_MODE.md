# PRD 18 — First-Run Setup, Resource Manager and Offline Behavior

## User-facing setup
If required resources are absent on first launch and Internet is available, show a plain-language action such as **Download All Resources**. Keep technical model filenames out of normal UI. State estimated total required size and that installed local resources enable supported offline capabilities. Do not imply online-only API features are local.

## Verified resource manifest
Versioned manifest records capability ID, resource revision, compatible app/OS/CPU/GPU, actual expected size, checksum/signature, source, license/provenance reference, dependencies, install path, health check and required/optional status. App UI labels capabilities/voices/languages in user language, not model filenames.

## Download screen
Show actual total and transferred bytes where known, item count, phase, progress, speed and recalculated ETA estimate, available disk, pause/resume/cancel/retry. Do not invent progress for unknown-size work. 100% comes only after checksum, installation and health test. Resource install must use genuine progress signals, not a timer or staged fake percentages.

## Failure cases to design/test
First launch offline; captive portal; DNS/TLS/certificate issue; server 403/404/429/5xx; instability/drop/restart/sleep; pause/resume/cancel; insufficient disk; permissions; corrupt partial/existing file; mismatched checksum/manifest; wrong architecture/runtime; missing dependency; app termination during installation; AV quarantine; stale manifest; full disk during final commit; proxy; time skew; installed-but-incompatible resource.

## Safe recovery
Use resumable downloads only when HTTP range/ETag semantics can be validated; otherwise restart safely. Download to temporary part file, verify cryptographic digest/signature and size, install into versioned staging directory, health-test, then atomically switch active manifest. Preserve previous known-good resources until new ones pass; don't mark set ready while required resources are missing. On restart offer resume/retry/discard only after examining integrity. Never trust mere file existence. Preflight space with safety margin. Partial optional failure must leave unrelated features usable; failed required capability is disabled with useful reason.

## Offline first run and operation
App shell and available core features launch without internet. If renderer is available, users can open/view even if specialist resources are missing. Offer retry/download later instead of dead-end onboarding. Capability registry declares online/offline/dependency. After resource setup, verify offline conversion, OCR, translation, TTS and all other designated local tasks. Network-required operation explains why, never silently routes to cloud. If model update fails, previous known-good resource continues working.

## Resource lifecycle
Compatibility check on upgrades; size confirmation for major downloads; resource audit/repair; separate cache cleanup from required model deletion; never delete only working version before replacement passes. Resource storage may be configurable after testing. Include reinstall/repair and actual disk usage.

## Acceptance
Automated/manual tests for offline/drop/restart/pause/resume/corrupt/hash-fail/low-disk/retry/rollback, exact offline capability matrix, model names absent from normal UI, no secrets/content in logs, resource removal/reinstall, and healthy operation after partial failure.
