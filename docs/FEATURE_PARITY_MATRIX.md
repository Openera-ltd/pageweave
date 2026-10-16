# Live Feature Parity Matrix

This file is the implementation ledger. The PRD's extensive list is a target, not a status claim. Create one row per atomic capability before implementing broadly.

| ID | Capability | User workflow | Component | Dependency/version/license | Offline? | Tests | Benchmark/quality metric | Status | Evidence / known limitation |
|---|---|---|---|---|---|---|---|---|---|
| PDF-VIEW-001 | Open and render a valid PDF | Open → first page visible → navigate | Viewer/engine | TBD after audit | Yes, if renderer bundled | Unit + integration + E2E | First-page latency, render diff | Not assessed | Baseline required |
| PDF-RED-001 | Permanent redaction | Mark → preview → apply to new file → verify | Security/document engine | TBD after audit | Yes | Adversarial extraction tests | Zero recovered redacted text in corpus | Not assessed | Block release on failure |
| RES-INSTALL-001 | Verified resumable resource setup | Download → disconnect → resume → verify | Resource manager | TBD | Setup needs network | Fault injection | Transfer integrity/recovery | Not assessed | Actual monitor required |
| TTS-HL-001 | Speech-synchronized highlight | Select → read → highlight/scroll/pause/resume | TTS/reader | TBD | Offline for installed voice | Alignment E2E | Timing drift, time-to-audio | Not assessed | No timer simulation |
| DOC-CONV-001 | PDF-to-editable DOCX | Convert → inspect → compare | Conversion engine | TBD | Yes if runtime installed | Golden corpus | Content/layout/overflow metrics | Not assessed | No full-page raster shortcut |

## Status semantics
`Not assessed`, `Planned`, `In progress`, `Implemented`, `Verified`, `Blocked`, `Deferred by decision`. `Verified` requires linked evidence on a named build/platform. `Blocked` requires reason and next action. Keep this ledger updated; generate reports/coverage from it if possible.
