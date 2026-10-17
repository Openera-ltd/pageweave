# PageWeave Decision Log

Record approved product/architecture decisions here. Do not mark proposals as approved. Include a date, status, rationale, consequences, evidence, and superseded decision if applicable.

## Decision template

### [ID] — [Short title]
- **Status:** Proposed / Approved / Rejected / Superseded
- **Date:** YYYY-MM-DD
- **Context:**
- **Options considered:**
- **Decision:**
- **Evidence:**
- **Consequences/trade-offs:**
- **Follow-up/tests/docs:**

## Initial scope questions — unresolved

### DW-001 — Built-in full word processor and spreadsheet editor
- **Status:** Open; requires user decision.
- **Context:** PageWeave already has broad viewing, editing, conversion, OCR, translation, and specialist document workflows. A full general-purpose office editor is a major scope expansion.
- **Question:** Should PageWeave build full from-scratch document/spreadsheet authoring, or focus on existing-file editing/conversion and use mature engines/integration for broader authoring?
- **Constraint:** Do not silently choose a scope that materially increases architecture, implementation, testing, and maintenance cost.

## Foundation decisions — 2026-10-16

These were made by Claude Code under the owner's standing instruction to "start building ... don't stop" (2026-10-16). They are reversible foundation choices, recorded as **Accepted (provisional)** — the owner may override any of them; nothing here is presented as owner-approved scope.

### D-001 — Desktop shell: Tauri 2 with a TypeScript UI
- **Status:** Accepted (provisional)
- **Date:** 2026-10-16
- **Context:** CLAUDE.md §4 and PRD 04 name Rust (orchestration) and C++ (native engines) as core, TypeScript allowed for UI when the shell stays responsive and packages cleanly on Windows.
- **Options considered:** Tauri 2 (Rust host, WebView2 on Windows, MIT/Apache-2.0); Qt 6 (C++, LGPL/commercial, heavier Rust interop); WinUI 3 / WPF (C#, adds a language without a core-engine benefit); egui/Slint (immature accessibility / licensing trade-offs).
- **Decision:** Tauri 2 host in Rust calling the shared engine crates directly; UI in TypeScript (React + Vite). Heavy work stays in Rust worker threads, never the WebView thread.
- **Evidence:** License compatibility with MIT; Rust host means the UI, CLI, API and MCP share one engine (PRD 04). Windows packaging (MSI/NSIS) and WebView2 runtime must still be verified on a clean Windows machine — not yet done.
- **Consequences/trade-offs:** Depends on the WebView2 runtime (present on supported Windows 10/11 builds; installer must bootstrap it if missing). Accessibility relies on WebView2/UIA mapping of ARIA — must be tested with a screen reader.
- **Follow-up/tests/docs:** Windows CI packaging job; screen-reader review; startup/memory baseline.

### D-002 — PDF engine: PDFium (chromium/7881) via `pdfium-render` 0.9.4
- **Status:** Accepted (provisional)
- **Date:** 2026-10-16
- **Options considered:** PDFium (BSD-3/Apache-2.0); MuPDF (AGPL-3.0 or commercial — rejected: incompatible with distributing an MIT app without relicensing); poppler (GPL — rejected for the same reason); pure-Rust `lopdf` (MIT — good for structure, no renderer).
- **Decision:** Use prebuilt PDFium from `bblanchon/pdfium-binaries` tag `chromium/7881` (V8 and XFA **disabled** — no PDF JavaScript execution), dynamically loaded through `pdfium-render` 0.9.4 with feature `pdfium_7881`. Binaries are downloaded by `tools/fetch_pdfium.py` and verified against pinned SHA-256 values in `resources/pdfium.lock.json`; never committed to Git. `lopdf` is used only in tests as an *independent* parser to validate outputs.
- **Evidence:** Linux and Windows x64 archives downloaded and hashed on 2026-10-16; bundled `licenses/` directory lists PDFium and its third-party components (FreeType, ICU, libjpeg-turbo, libpng, libtiff, OpenJPEG, lcms, zlib, abseil, AGG, fast_float, simdutf, llvm-libc).
- **Consequences:** XFA forms unsupported (must be detected and reported, PRD 05). Third-party notices must ship with the app (`THIRD_PARTY_NOTICES.md`).

### D-003 — Repository layout
- **Status:** Accepted (provisional)
- **Date:** 2026-10-16
- **Decision:** Cargo workspace: `crates/pw-core` (typed operations, errors, page ranges, safe output, jobs, logging, settings), `crates/pw-pdf` (PDFium adapter), `crates/pw-cli` (`pageweave` CLI), `app/` (Tauri app + UI). C++ is consumed only through PDFium's C ABI until a measured need justifies first-party C++.

### D-004 — Test fixtures are synthetic and generated independently of the engine
- **Status:** Accepted (provisional)
- **Date:** 2026-10-16
- **Decision:** `tools/fixtures/gen_fixtures.py` writes PDFs directly from PDF syntax (no PDFium, no third-party content), so fixtures are rights-clean (PRD 15) and tests are not circular. No downloaded PDFs are committed.

### D-005 — Credentials
- **Status:** Accepted
- **Date:** 2026-10-16
- **Decision:** Kaggle/GitHub/Cloudflare credentials are injected at runtime only (environment secrets or per-command), never written to the repository, configs, logs or the journal (PRD 16). `.gitignore` excludes credential/env files; CI runs a secret scan.

### D-006 — One audited FFI module for PDFium form filling
- **Status:** Accepted
- **Date:** 2026-10-17
- **Context:** Filling AcroForms correctly needs PDFium's form filler (focus/replace selection, clicks, list selection) so widget appearance streams are regenerated; writing `/V` alone leaves stale appearances that most viewers keep showing. pdfium-render 0.9.4 (latest) keeps its raw bindings and form handle crate-private, and also hard-codes `/Yes` as every checkbox's on-state.
- **Options considered:** vendor-patch pdfium-render (24 MB copy to change one visibility; rejected); write `/V` and our own appearance streams (re-implements text layout/fonts — rejected, CLAUDE.md §4.5); `NeedAppearances` (ignored by many viewers, deprecated in PDF 2.0 — rejected); resolve the needed functions from the already-loaded PDFium library (accepted).
- **Decision:** `unsafe_code` is `deny` workspace-wide (was `forbid`); only `crates/pw-pdf/src/forms.rs` opts in, enforced by the test `unsafe_is_confined_to_the_forms_module`. Every direct call runs under an exclusive engine lock that all other engine access shares (`pw_pdf::engine`), so direct calls never overlap pdfium-render's. Signatures follow the pinned chromium/7881 headers; documents are loaded from memory and torn down in PDFium's required order by a drop guard.
- **Consequences:** Upgrading PDFium requires re-checking these signatures. Form work briefly blocks other engine use (milliseconds for typical forms).

### D-007 — Honour `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS` on Windows
- **Status:** Accepted
- **Date:** 2026-10-17
- **Context:** Edge WebDriver attaches to WebView2 apps by setting the standard `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS` variable (remote-debugging port). wry always passes its own `AdditionalBrowserArguments`, and on the WebView2 runtime in use (154.0.4258.62) those take precedence, so the variable never reached the browser process (verified on the CI runner by the browser process command line). Windows UI automation was therefore impossible.
- **Decision:** The main window is created in `setup` from its config (`create: false`). When the variable is set, its value is appended to wry's default flags; when unset nothing changes. This restores documented platform behaviour for every WebView2 app rather than adding a PageWeave-specific switch, so the shipping binary is what gets tested.
- **Security note:** whoever can set environment variables for the user's processes can already run code as that user; no new privilege is exposed. The value is never logged.
