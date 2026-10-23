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
- **Amendment (2026-10-17):** the shared/exclusive scheme was replaced by one reentrant lock held for each whole engine operation. Per-call serialization (pdfium-render `thread_safe`) let multi-call sequences interleave across threads; PDFium's process-global last error made a corrupt file report "password protected" under concurrency (reproduced: 4 of 20 test runs; regression test fails 7/10 without the lock). Trade-off: a long operation (e.g. combining hundreds of files) now delays page rendering until it finishes; to be measured and, if needed, split into lock-releasing steps.

### D-007 — Honour `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS` on Windows
- **Status:** Accepted
- **Date:** 2026-10-17
- **Context:** Edge WebDriver attaches to WebView2 apps by setting the standard `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS` variable (remote-debugging port). wry always passes its own `AdditionalBrowserArguments`, and on the WebView2 runtime in use (154.0.4258.62) those take precedence, so the variable never reached the browser process (verified on the CI runner by the browser process command line). Windows UI automation was therefore impossible.
- **Decision:** The main window is created in `setup` from its config (`create: false`). When the variable is set, its value is appended to wry's default flags; when unset nothing changes. This restores documented platform behaviour for every WebView2 app rather than adding a PageWeave-specific switch, so the shipping binary is what gets tested.
- **Security note:** whoever can set environment variables for the user's processes can already run code as that user; no new privilege is exposed. The value is never logged.

### D-008 — Permanent redaction: PDFium edits, independent cleaning and verification
- **Status:** Accepted
- **Date:** 2026-10-17
- **Context:** PRD 05/11 require real removal ("black rectangles are not redaction"), distinct marked → previewed → applied → verified states, and verification by independent inspection. PDFium (D-002) can remove and add page objects but keeps unreferenced objects, resource entries, thumbnails and annotation back-references when saving, any of which can still hold removed content. MuPDF's redaction is AGPL (incompatible with the project licence); qpdf would add a C++ toolchain dependency.
- **Decision:** Content is removed with PDFium (per character, image pixel, object and annotation; a page that cannot be handled precisely becomes an image of the page). The output is a new document holding only the pages, rewritten by `lopdf` (MIT, already a test dependency; now shipped with default features off) to drop unused XObjects, thumbnails, page metadata/actions, form field data, dangling annotation references and links to pages that no longer exist. Success requires: PDFium re-reading the file and finding no character, annotation or unpainted image pixel under any area, an unchanged rendering outside the areas, and `lopdf` finding one revision, no stray page objects and none of the removed image data. Any failure means nothing is written.
- **Consequences:** ~30 additional permissively licensed crates in the shipped binaries (lopdf's mandatory parsing/crypto dependencies; cargo-deny clean). Bookmarks, document properties, attachments and fillable fields are not carried over to redacted copies (disclosed every time). Partly covered vector graphics are kept (as MuPDF does by default); they are covered by the black box but remain in the file.

### D-009 — OCR engine: PP-OCRv6 small on ONNX Runtime
- **Status:** Accepted (benchmark on synthetic data; to be re-confirmed on a real-world rights-cleared set)
- **Date:** 2026-10-17
- **Context:** OCR is a required specialist capability (AI_AND_RESOURCES). PRD 14 requires benchmarking proven models before any fine-tuning.
- **Options measured** (`docs/benchmarks/OCR_BASELINE.md`): Tesseract 5.3.4 (Apache-2.0) — 3.75% CER, 1.3 s/page; PP-OCRv6 small detection + recognition (Apache-2.0 code and weights, Baidu/PaddleOCR; ONNX conversions distributed by RapidOCR with a model license record) — 0.41% CER, 3.1 s/page, on a 72-page blind synthetic set across 9 scan conditions.
- **Decision:** Integrate PP-OCRv6 small via ONNX Runtime (MIT) loaded at runtime from a pinned, checksum-verified official build (as PDFium is, D-002), with detection/recognition pre- and post-processing implemented in Rust. Model files are installed resources (verified by SHA-256), not compiled in. The recogniser's character dictionary comes from the model's own metadata.
- **Consequences:** Model size ~31 MB (det 9.9 MB + rec 21.2 MB); higher memory than Tesseract (to be measured in Rust). The `ort` crate (MIT/Apache-2.0) is at 2.0 release candidate — pin exactly; fallback is a small audited FFI over the ONNX Runtime C API. Fine-tuning is deferred until a real-world evaluation shows a gap the cheap fixes cannot close; official training weights are not published for PP-OCRv6 small (only for PP-OCRv5 mobile), see `docs/benchmarks/OCR_BASELINE.md`.

### D-010 — Changing a file's protection honours its owner's restrictions
- **Status:** Accepted (scope limited; see open question)
- **Date:** 2026-10-17
- **Context:** A PDF can have an open password and, separately, owner restrictions (no printing, no editing…) lifted only by its permissions password. Removing the password, or re-protecting with new permissions, from a file opened with only the open password would silently lift those restrictions. PRD 15/data rules: do not bypass access controls.
- **Decision:** "Remove password" and re-protecting a protected file are refused (`pdf.owner_restricted`, category permission) unless PDFium reports full rights — i.e. the file has no restrictions, or it was opened with the permissions password. Decryption uses PDFium's own save with `FPDF_REMOVE_SECURITY`, which keeps bookmarks, properties and forms.
- **Consequences:** A user with only the open password of a restricted file cannot remove its protection; the error says what is needed.
- **Open question (needs a product decision):** page operations (extract, delete, split, combine, organize, redact, OCR) on a restricted file still produce unprotected outputs (existing behaviour, disclosed by a warning). Whether they should also require full rights, or check the specific permission (e.g. assemble for page operations), is not yet decided.

### D-011 — Signature verification with RustCrypto parsers and local RSA verification
- **Status:** Accepted
- **Date:** 2026-10-17
- **Context:** Checking PDF signatures needs CMS/X.509 parsing and RSA/ECDSA verification. The `rsa` crate (0.9, the current release line) carries RUSTSEC-2023-0071 (Marvin timing attack on private-key operations). The project rule is that vulnerabilities are never ignored in deny.toml, even when the vulnerable path is unused.
- **Decision:** Parse with `cms`/`x509-cert`/`der` and verify ECDSA with `p256`/`p384` (RustCrypto, pure Rust, MIT/Apache-2.0). RSA PKCS#1 v1.5 verification is implemented in `crates/pw-pdf/src/signatures.rs` with a public-key modular exponentiation (`num-bigint`) and an exact comparison of the encoded message against the expected DigestInfo — public operations only, so no secret-dependent timing exists. Trust is never claimed: certificate chains, revocation and timestamps are not checked yet and every report says so.
- **Consequences:** RSA-PSS and BER-encoded or non-canonically ordered signed attributes are reported as unsupported/unreadable, not as valid. Revisit when `rsa` 0.10 (no advisory) is released, or when a trust list is added.

### D-012 — Signing with a certificate: PKCS#12 via p12-keystore, signing via ring, incremental update
- **Status:** Accepted
- **Date:** 2026-10-18
- **Context:** PRD 05 requires certificate-based signing that protects private keys, uses supported algorithms, saves incrementally and revalidates after writing. The `rsa` crate is excluded (RUSTSEC-2023-0071 affects exactly the private-key operations signing needs, D-011).
- **Decision:** Read PKCS#12 identities with `p12-keystore` 0.4 (MIT/Apache-2.0; PBES2/AES and legacy 3DES/RC2). Sign with `ring` 0.17 (constant-time RSA PKCS#1 v1.5 SHA-256 and ECDSA P-256/P-384). Build a detached CMS (adbe.pkcs7.detached) with `cms`/`der`. Append the signature as an incremental update with lopdf's `IncrementalDocument` (matching the file's cross-reference style), then fill the byte range and contents in place. Before writing, PageWeave's verifier must report the new signature valid and covering the whole file, every previously valid signature still valid, and the original bytes unchanged as a prefix. Tests also verify with `openssl cms -verify` against the test CA. Password-protected files are refused (remove the password first). The certificate password travels outside the serialized request and is never journaled or logged.
- **Consequences:** No timestamp (RFC 3161), no long-term validation data, no CAdES signingCertificateV2, no Windows certificate store or hardware tokens yet; the visible box uses Helvetica (Latin-1). p12-keystore brings a second RustCrypto `der` generation (0.8) alongside 0.7 (allowed: duplicate versions only warn). Test identities in `fixtures/keys` are test-only, with a published password. Secret scanning flags PKCS#12 files by type, so `.gitleaks.toml` allows exactly `fixtures/keys/test-signer-*.p12` and references to those file names; every other default rule stays in force. `.gitignore` keeps ignoring `*.p12`/`*.pfx` everywhere except those same test identities. (They were ignored at first, so the signing tests failed in CI until this exception was added.)

### D-013 — Signer trust from certificates the user chooses, not the system root store
- **Status:** Accepted
- **Date:** 2026-10-18
- **Context:** PRD 05 requires honest trust states (trusted, unknown signer, expired, …). Document-signing trust is not web trust: PDF readers do not trust the operating system's TLS roots for signatures by default, and no rights-cleared document-signing trust list (such as AATL/EUTL) is bundled.
- **Decision:** A signature is "trusted" only when its signer's certificate chains — through certificates carried in the signature, every link's signature verified, issuers required to be CAs, every certificate valid today — to a certificate the user explicitly trusts. The app keeps these as DER files in its config folder (`trusted-certificates/`, named by SHA-256 fingerprint); the user adds one from a signature after seeing its fingerprint, choosing "only this signer" or "everything issued by this CA", and removes them in Trusted certificates…. The CLI takes `--trust FILE`. Revocation (CRL/OCSP) and timestamps are never checked and every report says so.
- **Consequences:** Signatures from public CAs show "signer unknown" until the user trusts something; validity is judged at today's date (no timestamp support), so a signature made while a certificate was valid shows "certificate expired" later; no policy/EKU checks beyond CA constraints; no Windows certificate store integration yet.

### D-014 — MCP server: stdio only, approved folders, new files only
- **Status:** Accepted
- **Date:** 2026-10-21
- **Context:** PRD 17 asks for MCP tools that call PageWeave's own operations, with file access only through user-approved folders or handles, no arbitrary filesystem/shell/settings access, explicit errors and real cancellation, and no tool advertised before an integration test proves it.
- **Decision:** `pageweave mcp --root DIR [--root DIR…] [--allow-write]` speaks MCP (JSON-RPC 2.0, protocol 2025-06-18, also 2025-03-26 / 2024-11-05) over stdio only — no network listener, so no port, authentication or rate limiting is needed for this transport. Every path is canonicalized (symlinks resolved) and must lie inside a root; outputs must be new files (an existing name, including a dangling symlink, is refused; the collision policy is always `fail`) with a fixed extension, inside an existing approved folder. Tools that write are not listed and are refused unless `--allow-write` was given. Each tool deserializes strictly (unknown arguments rejected) into one existing `Operation`; validation and work are the engine's own. Passwords, signing, redaction, sanitize, protection, attachments and form-data files are not exposed. Messages over 1 MB and more than 8 active calls are refused; progress is sent for clients that pass a progress token; `notifications/cancelled` cancels the job; when the client disconnects, running jobs are cancelled.
- **Consequences:** Agents can create any number of new files inside approved folders (bounded by disk). Password-protected documents can't be opened through MCP. A local HTTP API (PRD 17 "local API") is not built; when it is, it needs loopback binding, authentication and rate limits that stdio does not. Optional external AI provider orchestration is not built.

### D-015 — Document jobs run in a worker process
- **Status:** Accepted
- **Date:** 2026-10-21
- **Context:** The write-operation mutation run found a damaged file on which PDFium's `FPDFPage_GenerateContent` fails an internal CHECK and aborts the whole process (`fixtures/hostile/redact-generate-content-crash.pdf`). A Rust `catch_unwind` cannot catch that. In the app, every job ran on a thread of the app process, so one damaged file closed the window and lost unsaved work in other tabs. `docs/ARCHITECTURE.md` already allows isolating native code in a worker process when crash isolation warrants it.
- **Decision:** The app runs each job from its queue (`start_operation`) in a child copy of its own executable started with `--pw-worker` (`pw_pdf::worker`). The child reads one request line, then one secrets line (passwords never appear in the serialized request), runs the same `pw_pdf::execute`, and reports progress, phase and exactly one result or typed error as JSON lines. Cancel is a `"cancel"` line; end of input also cancels, so a child never outlives its parent's pipe. If the child's output ends without a result, the job fails with `engine.crashed` ("The PDF engine stopped unexpectedly …; nothing was saved"). Staged files carry the parent and child ids (`.pw-staged-<app>-w<child>-`), so after a crash the app removes only that child's leftovers. The CLI binary accepts the same flag, so tests drive the worker without the app.
- **Consequences:** One process start per job (a whole small `inspect` job through the worker: median 10 ms, 10 runs, debug build, Linux dev container; Windows not measured yet). The viewer's page rendering, text/search reads and other direct read commands of the app still run in the app process, so a crash there is not yet contained. The CLI and MCP server run jobs in their own process (a crash ends that command, which is the CLI's existing behaviour). Batch and queue concurrency are unchanged (bounded by the host job manager).

### D-016 — PDF → Word: PageWeave's own layout analysis and WordprocessingML writer
- **Status:** Accepted
- **Date:** 2026-10-22
- **Context:** PRD 06 asks for PDF → Word that rebuilds editable paragraphs, headings, columns, tables, links and pictures, never rasterizing pages, and that measures content and layout drift. Candidates: pdf2docx (Python, AGPL-3.0 — incompatible with the MIT licence and with shipping it in the installer), LibreOffice (imports PDFs into Draw as positioned text boxes, not flowing Word text; ~300 MB), commercial SDKs (not free/open). No proven open engine produces flowing Word text under a compatible licence.
- **Decision:** Build it on the engine already in use: PDFium's per-character data (position, font, size, weight, italic, colour, generated spaces, soft hyphens) feeds a pure Rust layout pass (`crates/pw-pdf/src/docx/layout.rs`: lines → column gutters and bands → paragraphs by wrap test and line pitch → headings, alignment, indents, spacing, superscripts; pictures in rows, floating beside text, or behind text). A small WordprocessingML writer (`writer.rs`, ECMA-376 transitional subset) packages it with the `zip` crate 9.0.5 (MIT; deflate only; 9.x needed for RUSTSEC-2026-0367/0368/0369, which concern extraction PageWeave does not use). Before writing, an independent reader (`reader.rs`, quick-xml) reopens the package: every part parses, every reference resolves, every picture decodes, the sections match the pages, and the text holds exactly the PDF's characters. Tests also open every file in LibreOffice and compare page count, word sequence and ink overlap with the original.
- **Consequences:** Fidelity is heuristic and measured, not guaranteed; thresholds are regression guards from synthetic fixtures (`docs/benchmarks/DOCX_BASELINE.md`), not a quality claim for real-world documents — a real-document benchmark is still needed. Ruled tables (grids of drawn lines, `tables.rs`) become Word tables with merged cells; not yet: borderless tables, other drawn shapes, comments, form fields, footnote linking, lists as Word numbering, right-to-left and vertical text layout, embedded fonts. Microsoft Word itself has not opened the files (no Windows Word in CI). Opening Office files (DOCX → PDF) is a separate decision (it needs a layout engine).

### D-017 — Table values are text unless they are plain exact numbers
- **Status:** Accepted
- **Date:** 2026-10-22
- **Context:** PRD 06 requires guarding leading zeros, identifiers, long numbers, decimal separators, dates and locale formatting from automatic type corruption when tables go to spreadsheets.
- **Decision:** A cell is written as a number only when its text is a plain decimal (`-?digits(.digits)?`) with no leading zero, no trailing fractional zero, no grouping separator, no exponent, and at most 15 significant digits, so the spreadsheet stores and shows exactly the same characters. Everything else is an inline string. CSV is UTF-8 with a byte-order mark and CRLF; a value a spreadsheet would treat as a formula (starting with `=`, `+`, `@`, tab, CR, or `-` when not a number) gets a leading apostrophe.
- **Consequences:** Amounts like "1,234.50" and dates stay text (users can convert them in the spreadsheet); no values are silently changed. Locale-aware parsing (decimal commas) is not attempted.

### D-018 — The viewer's page images and read-only calls run in reader processes
- **Status:** Accepted
- **Date:** 2026-10-22
- **Context:** D-015 moved jobs out of the app process; page rendering and the viewer's reads (open, text layer, search, fields, comments, attachments, signatures, objects, previews, hidden-information check) still ran inside it, so a PDFium crash on a damaged page closed the window.
- **Decision:** The app keeps a pool of two long-lived reader processes (`pw_pdf::service`, the app executable started with `--pw-reader`). Each call is one JSON line; answers are JSON or raw image bytes. A reader that dies during a call fails that call with `engine.crashed`; the call is remembered and answered with the same error at once afterwards (no crash loops), and the next call starts a fresh reader. A call over 120 s is treated as hung and its reader replaced. A reader found dead while idle is replaced without blaming the next call. Numbers cross the pipe exactly (serde_json `float_roundtrip`). Unit tests of the app crate (where the binary is not the app) run calls in-process.
- **Consequences:** Two extra processes while the app runs (each with its own PDFium; measured render time through a reader 227 ms vs 274 ms in-process for a photo page at 150 %, Linux debug build). Debug builds honour a test hook (`PW_READER_TEST` or a `pageweave-reader-test` file in the temp folder) to abort or hang on a page; release builds compile it out. Resource checks and settings stay in the app process (no PDF parsing).

### D-019 — Office files open through a locally installed LibreOffice (not bundled, for now)
- **Status:** Accepted (provisional; bundling is an owner decision)
- **Date:** 2026-10-22
- **Context:** PRD 06 lists DOCX/XLSX/PPTX/ODF/RTF/TXT/HTML as interchange formats and says to use proven Office engines, never run macros, and handle damaged/encrypted inputs safely. The only proven, licence-compatible layout engine is LibreOffice (MPL-2.0). Bundling it adds roughly 300 MB to the installer and its own update and security obligations — a scope/cost choice for the owner.
- **Decision:** Use LibreOffice when it is installed on the computer (`PAGEWEAVE_SOFFICE`, PATH, standard install folders). Every conversion runs `soffice --headless --convert-to pdf` on a copy of the input in a private temporary folder with a fresh profile whose settings set macro security to "very high", disable macro execution, switch off update checks and never update links; it has a 300-second limit and can be cancelled; the PDF must open in PDFium and have pages before it is written atomically next to (never over) the input. Without LibreOffice the feature says so with what to do; Settings ▸ Resources lists it as optional, so it does not make the installation "not ready".
- **Consequences:** Works only where LibreOffice is installed; layout fidelity is LibreOffice's. Windows CI has no LibreOffice, so this path is tested on Linux only. Revisit when the owner decides whether to bundle (installer size) or to download it as an optional resource (needs the in-app download decision).

### D-020 — Borderless tables are found for table export only
- **Status:** Accepted
- **Date:** 2026-10-22
- **Context:** PRD 06 table extraction covers tables without drawn grids (aligned columns, tables with only horizontal rules). Word and PowerPoint export already keep such rows on one line with tab stops, which LibreOffice re-lays-out with full word agreement; turning them into Word tables would change verified layouts and risks splitting prose.
- **Decision:** `docx/borderless.rs` finds, outside ruled grids, three or more consecutive rows (at most 2.6 em apart) whose gaps of at least 1 em line up in the same vertical channels; a line crossing a channel ends the table, as do trailing single-cell rows. At least 60 % of rows must have two or more cells and half of the grid must hold text; columns that are all full lines of five or more words on average (prose set in columns) are rejected. These tables are used by `export_tables` (Excel and CSV) alongside the ruled ones, top first. Grids whose cells are all empty are not exported.
- **Consequences:** A cell that wraps onto a second line becomes its own row; a column that is empty except for one row may merge with its neighbour; numbered lists with wide gaps after the number can be reported as two-column tables. Word/PowerPoint export are unchanged.

### D-021 — Read aloud uses the computer's own speech voices
- **Status:** Accepted
- **Date:** 2026-10-22
- **Context:** PRD 09 requires read aloud with highlighting that follows real speech timings, offline, with natural local voices, and CLAUDE.md requires specialist capabilities (speech included) without bundling a chat model. Bundled neural voices (e.g. Piper/VITS) need a phonemizer (espeak-ng, GPL-3.0) and per-voice dataset licences that must be cleared first (PRD 15) — an owner decision. Windows ships licensed, offline voices with an API that reports when each word is spoken.
- **Decision:** `pw-speech` synthesizes with `Windows.Media.SpeechSynthesis` (WinRT, Windows 10 1709+) with word-boundary metadata enabled; cue text is matched to the input in order (positions as fallback). Each sentence is synthesized on its own, so sentence timings are exact and cancellation is checked between them. On other systems (development and CI only) the `flite` program is used when installed (sentence timings). Nothing is bundled; voice names are the system's own. The app speaks paragraph by paragraph (next one prepared while the current plays), plays through Web Audio (WebKitGTK's media element could not play WAV data in CI-like setups; Web Audio works in both WebViews) and highlights from the audio context's clock. The text comes from the Word-export layout (reading order, columns, tables), with one rectangle per word in the page's own coordinates. Read aloud is a required capability in Settings ▸ Resources: a computer without a voice is reported as not ready, with what to do.
- **Consequences:** Voice quality and languages are the computer's; no pronunciation dictionary or language switching yet. Bundled voices remain possible later behind the same `pw-speech` API once their licences are cleared.

### D-022 — Translation with OPUS-MT models on the bundled ONNX Runtime
- **Status:** Accepted (models are downloaded on first use since D-023)
- **Date:** 2026-10-22
- **Context:** PRD 08 requires offline document translation with verified model licences. OPUS-MT (University of Helsinki) models are CC BY 4.0, small (about 110–140 MB per direction as int8 ONNX), and run on the ONNX Runtime PageWeave already ships for OCR. The owner asked for the remaining release work to be done now.
- **Decision:** Ship English ↔ German, French, Spanish, Italian (8 directions, pinned by commit and SHA-256 in `resources/translate.lock.json`, fetched by `tools/fetch_translate.py`, bundled under `translate/`); other pairs of these languages go through English. `pw-translate` implements the SentencePiece Unigram tokenizer (NFKC), the encoder and the merged decoder with key/value cache (greedy decoding), sentence splitting shared with speech (`pw_core::text`), masking of addresses, codes, names with inner capitals and capitalised names (restored after translation; pieces translated separately if a mask is dropped), function-word language detection, and a report of numbers/codes/addresses not kept. `translate_document` writes a Word document from the Word-export layout (each paragraph translated in its first run's style) or a text file; the engine lock is released while translating; one model is loaded at a time.
- **Consequences:** About 900 MB more in the installer. Quality is machine-translation quality (measured in `docs/benchmarks/TRANSLATION_BASELINE.md`); no beam search, glossary, side-by-side/bilingual output, RTL or CJK yet; mixed formatting inside a paragraph becomes one style (reported). Attribution in `third_party/licenses/opus-mt/`.

### D-023 — Translation models download when first used (not bundled)
- **Status:** Accepted (amends D-022's bundling)
- **Date:** 2026-10-23
- **Context:** Bundling all eight directions (D-022) added about 900 MB to the installer, and every app update would download them again. The owner asked for updates that do not re-download everything.
- **Decision:** The installer carries no translation model. Choosing a language the computer does not have offers a one-time download (size shown; progress, cancel, Try again). Each file of the direction is fetched from its pinned Hugging Face commit URL (`resources/translate.lock.json`) over HTTPS with the operating system's certificate store and proxy settings (`pw-net`: ureq + rustls), into `<file>.part` (resumed with a Range request after an interruption, up to four attempts with back-off), checked against the pinned size and SHA-256, and only then renamed into place. Models live in the user's data folder (`%LOCALAPPDATA%\io.github.kuyamcliff.pageweave\translate`), outside the install folder, so updates and reinstalls keep them. One download per direction at a time (an OS file lock that is released when a process dies). Damaged files are found by hash and fetched again. CLI: `pageweave translate --install en-de` / `--remove en-de`. A development checkout's `third_party/translate` is still used when present.
- **Consequences:** Translation needs the internet once per direction. Settings ▸ Resources reports how many directions are downloaded and verified; translation is "ready" without any. The Windows package smoke test downloads en→de through the runner's certificate store and translates with it.

### D-024 — App updates: signed installers from this project's GitHub releases
- **Status:** Accepted
- **Date:** 2026-10-23
- **Context:** The owner asked for a first official release where later versions install as updates without re-downloading everything. `tauri-plugin-updater` (2.13–2.14) depends on `zip` 4.x, which has open RUSTSEC advisories (2026-0367/0368/0369); advisories are never ignored here (`deny.toml`).
- **Decision:** PageWeave's own updater (`pw-update`). The release workflow (`.github/workflows/release.yml`, on a `vX.Y.Z` tag) checks that tag, app, crate and UI versions agree, builds the NSIS installer with the update public key built in (`PAGEWEAVE_UPDATE_PUBKEY` from the repository variable `TAURI_UPDATER_PUBKEY`), signs the NSIS installer with `tauri signer sign --app-version` (minisign; private key and password in repository secrets), checks the signature with the app's own code, installs and launches the release, and publishes the NSIS installer (not the MSI: a per-machine MSI copy beside the per-user one would confuse updates), the signature, a portable CLI zip and `latest.json` (version, notes, date, and per platform the URL, size, SHA-256 and signature). The app reads `latest.json` from `releases/latest/download/` at startup (setting `check_updates`, default on; Settings ▸ Updates has Check now). A newer version is offered; downloading is a job (`download_update`: progress, cancel, resume) into `%LOCALAPPDATA%\…\updates`; the installer must come from `https://github.com/kuyamcliff/pageweave/releases/download/`, match the size and SHA-256, carry a valid minisign signature from the built-in key, and be signed for exactly the announced version (an old signed installer cannot be passed off as new). Installing re-checks the signature, refuses while there are unsaved edits, starts the installer with `/P /UPDATE /R` (passive, update, restart PageWeave) and closes the app. Builds without the key (development builds) can check but never install. The CLI has `pageweave update` (check only). MCP has no update tool.
- **Consequences:** An update downloads only the installer (program, PDF engine, OCR runtime and models: tens of MB); models, settings, trusted certificates and logs are in the user's folders and are kept. Losing the private key means installed copies cannot verify later updates (they would need a manual reinstall with a new key). The installer is not Authenticode-signed (no certificate yet), so SmartScreen may warn on first install. The install hand-off and restart are exercised only on Windows with a real release; unit tests cover manifest checks, signature, version binding, tampering, resume and clean-up.

**D-023 amendment (2026-10-23, owner request "put it inside"):** each release also publishes a full installer (`PageWeave_<v>_x64-setup.exe`, about 1 GB) with all eight directions. Its post-install hook (`app/src-tauri/windows/hooks.nsh`) moves them from the install folder to the user's `translate` folder, where downloads also go; the small installer (`PageWeave_<v>_x64-update-setup.exe`) is what `latest.json` offers for updates, so updates still never download the languages. The release workflow checks, before publishing, that after a full install every pinned file is in the user folder with its pinned size and SHA-256, none is left in the install folder, a document opens, and the update installer run over it keeps them.
