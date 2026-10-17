# PageWeave

PageWeave is a free/open-source, local-first Windows desktop document studio (see `prd/00_MASTER_PRD.md`
for the full product scope).

## Status — early development, not production ready

Release status: **BLOCKED**. What exists today is a first vertical slice; most of the product scope in
`prd/` (OCR, conversion, form authoring, redaction, translation, read-aloud, design reconstruction,
resource installer, API/MCP, …) is **not implemented**. `docs/FEATURE_PARITY_MATRIX.md` is the
authoritative per-capability ledger with evidence and limitations.

Implemented and tested:

| Area | What works | Evidence |
|---|---|---|
| PDF engine | Open (with password), inspect metadata/pages/bookmarks, extract text, search with match geometry, render pages | `crates/pw-pdf/tests/engine.rs` (Linux and Windows CI) |
| Page operations | Extract, delete, rotate, split, combine — always to a **new** file; source never modified; outputs re-opened and verified before an atomic commit; explicit conflict policy | engine tests (outputs re-parsed with an independent parser) |
| CLI | `pageweave` with JSON output and stable exit codes | `crates/pw-cli/tests/cli.rs`; packaged Windows smoke test in CI |
| Desktop app | Tabs, continuous viewer, thumbnails, bookmarks, zoom/fit, find with highlights, text selection and copy, page tools, settings (theme/accent/density/text size/motion/shortcuts), command palette, context menus, drag-and-drop and command-line open | `app/e2e/run.mjs` drives the real app (36 scenarios incl. axe-core accessibility checks) on Linux; Windows installer install/launch/uninstall smoke in CI |
| Forms | Fill AcroForm text, multiline, checkbox, radio, combo and list fields directly on the page; values are entered through PDFium's form filler so the saved copy's appearances show them; Ctrl+S saves all changes (fields and annotations) into one verified copy, then the tab switches to it | engine tests verify values, button states and regenerated appearance streams with an independent parser and by rendering; E2E fills every kind in the real app and checks the saved file with the CLI |
| Annotations | Highlight, underline, strikethrough and sticky notes from a text selection or a right-click; undo/redo; saved into a **new** PDF (verified by reopening); closing a tab or the window with unsaved annotations asks first | engine tests (independent parser + rendered pixels); E2E checks the saved file and the real window-close request |

Known limitations (selection):

- Combining PDFs does not carry over bookmarks or document metadata (the app says so).
- Outputs made by extract/delete/split/combine from a password-protected PDF are **not** protected (the app warns); rotate keeps the protection.
- Annotations: fixed colours, no editing/moving/deleting of individual or existing annotations yet (undo covers unsaved ones); not saved back into the original file by design.
- Forms: no form authoring, field validation scripts, calculation order, signatures or XFA filling (XFA is detected and reported). Native input undo only for field edits.
- No OCR, conversion or printing.
- XFA forms are detected but cannot be displayed or filled.
- No measured performance budgets or hardware support claims yet (`docs/HARDWARE_SUPPORT.md`).
- Screen-reader behaviour has not yet been verified with Narrator/NVDA.

## Architecture

- `crates/pw-core` — engine-independent foundation: typed operation schema, error taxonomy, page-range
  parser, atomic output writer, bounded job manager with cancellation, privacy-safe JSONL logging,
  versioned settings.
- `crates/pw-pdf` — adapter over PDFium (`chromium/7881`, JavaScript and XFA disabled), pinned by SHA-256
  in `resources/pdfium.lock.json`.
- `crates/pw-cli` — `pageweave` command-line interface over the same typed operations.
- `app/` — Tauri 2 desktop app: Rust host (`app/src-tauri`) and React/TypeScript UI (`app/src`).

All interfaces call one dispatcher (`pw_pdf::execute`). Decisions are recorded in `docs/DECISIONS.md`.

## Build and test

Prerequisites: Rust (stable), Node.js 22, Python 3. On Linux also `libwebkit2gtk-4.1-dev`, `librsvg2-dev`.

```sh
python3 -I tools/fetch_pdfium.py          # download + verify the pinned PDF engine
cd app && npm ci && npx vite build && cd ..
cargo test --workspace                    # Rust unit + integration tests
(cd app && npx vitest run)                # UI unit tests
cd app && npx tauri build --debug --no-bundle
./e2e/run.sh ../target/debug/pageweave-app ../e2e-artifacts   # real-app E2E (Linux: needs xvfb, webkit2gtk-driver, tauri-driver)
npx tauri build                           # Windows installers (NSIS + MSI)
```

CLI examples:

```sh
pageweave inspect report.pdf
pageweave extract --pages "1-3, 5" -o excerpt.pdf report.pdf
pageweave --json --on-conflict rename merge a.pdf b.pdf -o combined.pdf
pageweave --password-stdin inspect protected.pdf < password.txt
```

## Privacy

PageWeave's own code makes no network requests while you work with documents (this has not yet been
verified with network monitoring on Windows, where the WebView2 runtime is a separate Microsoft component). Logs record operation names, timings,
page counts and error codes — never document text, file names (paths are replaced by a short hash) or
passwords. Passwords are held in memory only while a document is open. Logs live in the app's local data
folder; open it with the command palette (Ctrl+K) → "Show log folder".

## License

MIT — see `LICENSE`. Redistributed third-party components and their licenses: `THIRD_PARTY_NOTICES.md`
and `third_party/licenses/`.
