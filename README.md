# PageWeave

PageWeave is a free/open-source, local-first Windows desktop document studio (see `prd/00_MASTER_PRD.md`
for the full product scope).

## Status — early development, not production ready

Release status: **BLOCKED**. What exists today is a first vertical slice; most of the product scope in
`prd/` (conversion, form authoring, sanitization, translation, read-aloud, design reconstruction,
resource installer, API/MCP, …) is **not implemented**. `docs/FEATURE_PARITY_MATRIX.md` is the
authoritative per-capability ledger with evidence and limitations.

Implemented and tested:

| Area | What works | Evidence |
|---|---|---|
| PDF engine | Open (with password), inspect metadata/pages/bookmarks, extract text, search with match geometry, render pages | `crates/pw-pdf/tests/engine.rs` (Linux and Windows CI) |
| Page operations | Organize (reorder, duplicate, rotate, remove, insert pages from another file in one step), page numbers / headers / footers / Bates-style numbering, export pages as PNG images, extract, delete, rotate, split (every N pages, by ranges, by bookmarks with parts named after them, or by largest file size), combine — always to a **new** file; source never modified; outputs re-opened and verified before an atomic commit; explicit conflict policy | engine tests (outputs re-parsed with an independent parser) |
| CLI | `pageweave` with JSON output and stable exit codes | `crates/pw-cli/tests/cli.rs`; packaged Windows smoke test in CI |
| Desktop app | Tabs, continuous viewer, thumbnails, bookmarks, zoom/fit, find with highlights, text selection and copy, page tools, settings (theme/accent/density/text size/motion/shortcuts), command palette, context menus, drag-and-drop and command-line open | `app/e2e/run.mjs` drives the real app (66 scenarios incl. axe-core accessibility checks) on Linux and Windows CI; Windows installer install/launch/uninstall smoke in CI |
| Forms | Fill AcroForm text, multiline, checkbox, radio, combo and list fields directly on the page; values are entered through PDFium's form filler so the saved copy's appearances show them; Ctrl+S saves all changes (fields and annotations) into one verified copy, then the tab switches to it | engine tests verify values, button states and regenerated appearance streams with an independent parser and by rendering; E2E fills every kind in the real app and checks the saved file with the CLI |
| Form data | Export the form's values (including unsaved changes) as XFDF — readable by most PDF apps — or JSON; import an XFDF or JSON file with a preview of which fields it fills and why any value is not used (unknown name, read-only field, no such option), then the values become unsaved changes (Undo available) to save as a copy. CLI: `pageweave form-export`, `pageweave form-import` (`--preview` or `-o`). Data files are treated as untrusted: size, count and nesting limits; XML document type declarations and custom entities are refused | format tests (round trips, nested names, special and control characters, hostile files); engine tests fill copies and check values with an independent parser, including that the data file can never be overwritten; E2E exports, previews, imports, undoes and saves in the real app, then checks the copy with the CLI |
| OCR | Recognise text on scanned pages and save a searchable copy (invisible text layer over the words; pages look identical); pages that already have text are left alone | Benchmark against Tesseract on a synthetic, rights-clean corpus (`docs/benchmarks/OCR_BASELINE.md`); engine tests check word recall, placement and search; E2E runs it in the real app; the Windows package runs it in CI |
| Password protection | Encrypted copy (AES-256) with an open password and optional permissions; protected originals can be re-protected with a new password | engine tests check refusal without the password, content, and the encryption dictionary with an independent parser; E2E protects and reopens in the real app |
| Images to PDF | JPEG and PNG files to a PDF, one per page (A4, Letter or image size); JPEGs embedded unchanged, phone photos turned upright; drop images on the window to start | JPEG bytes compared inside the PDF; orientation and transparency checked in renders |
| Edit bookmarks | Add from a page's menu, rename, re-point, nest, reorder and delete bookmarks; saved into a new file | the saved outline is read back by PDFium and compared item by item; E2E in the real app |
| Attachments | See embedded files in the sidebar, save one out exactly as stored (never opened), attach files into a new copy | contents compared byte-for-byte after saving and attaching; E2E in the real app |
| Crop pages | Trim margins (mm, in or pt) as displayed on all or chosen pages; content outside is hidden, not removed (said each time) | crop boxes read back by an independent parser, rotated pages included; E2E in the real app |
| Watermark | Text watermark with size, colour, opacity and angle, over or behind the content, on chosen pages | text found on exactly the chosen pages, translucency and angle measured in renders; pages where it cannot be seen are reported; E2E in the real app |
| Document properties | View, and edit title, author, subject and keywords into a new file (XMP kept in step) | values read back by PDFium and an independent parser; every page renders as before; E2E edits and re-inspects in the real app |
| Remove password | Copy that opens without a password; files whose owner restricted them need the permissions password | engine tests check the copy opens without a password with identical text and no encryption dictionary (independent parser), and that restrictions are honoured; E2E in the real app |
| Remove hidden information | Lists and removes metadata, scripts and actions, attachments, hidden layers and comments (optionally form fields and web links) in a new file; earlier saved versions always removed | every removed kind is searched for in the whole output with an independent parser, and each page must render exactly as before; E2E cleans and re-inspects in the real app |
| Redaction | Mark text (selection or every search match), dragged areas or whole pages; review what each area removes; apply to a **new** file. Content under the areas is removed (text per character, image pixels, drawings, comments/links, form widgets), the areas painted black, and the copy rewritten without unused resources, thumbnails, metadata or revision history; it is verified before it is written (no text, annotation or image data left under any area; rendering unchanged elsewhere; removed image data absent from the file) | engine tests search every object and stream of the output with an independent parser, including negative tests that prove the checks reject leaky output; CLI `redact`; E2E marks, reviews and applies in the real app, then checks the copy with the CLI and its raw bytes |
| Edit text and pictures | Change the wording of a text line in place, move lines and pictures, turn, replace or delete pictures, add new text; the engine renders an exact preview; undo/redo; saved into a **new** copy together with annotations and form fields (CLI: `pageweave objects`, `pageweave edit`) | engine tests reopen every saved copy and check its text, fonts and pictures; E2E edits in the real app and checks the copy with the CLI |
| Annotations | Highlight, underline, strikethrough and sticky notes from a text selection or a right-click; undo/redo; saved into a **new** PDF (verified by reopening); closing a tab or the window with unsaved annotations asks first | engine tests (independent parser + rendered pixels); E2E checks the saved file and the real window-close request |

Known limitations (selection):

- Combining PDFs does not carry over bookmarks or document metadata (the app says so).
- Outputs made by extract/delete/split/combine from a password-protected PDF are **not** protected (the app warns); rotate keeps the protection.
- Editing text: each line is edited on its own (no paragraph reflow); letters missing from an embedded subset font switch the line to a standard font (the app warns); text in scripts the font cannot draw is refused; turned or slanted text cannot be edited yet; pictures have no resize handles.
- Annotations: fixed colours, no editing/moving/deleting of individual or existing annotations yet (undo covers unsaved ones); not saved back into the original file by design.
- Forms: no form authoring, field validation scripts, calculation order, signatures or XFA filling (XFA is detected and reported). Native input undo only for field edits; Clear form empties text fields and checkboxes but cannot empty choice lists or radio groups, and there is no reset to default values yet. Form data: FDF files are not read yet (refused with a message); a list box takes one value per import; rich-text values and clearing a radio choice are reported as not used.
- Redaction: bookmarks, document properties, attachments and fillable fields are not carried over to the redacted copy; a page whose content cannot be removed precisely (grouped content or unreadable text crossing an area) becomes an image of the page (the app says which); drawings only partly under an area (e.g. backgrounds) are covered but kept; pattern search finds e-mail addresses, phone numbers, card numbers (Luhn-checked), IBANs (checksum-checked), US SSNs and custom regular expressions in the text layer only — scanned text must be recognized first, and every match should be reviewed (look-alikes and unusual formats are possible).
- OCR: accuracy measured on synthetic English/Latin pages only; the text layer carries Latin characters; no handwriting, layout or table structure; Windows speed not yet measured.
- No conversion to or from Office formats yet.
- XFA forms are detected but cannot be displayed or filled.
- No measured performance budgets or hardware support claims yet (`docs/HARDWARE_SUPPORT.md`).
- Screen-reader behaviour has not yet been verified with Narrator/NVDA.
- Accessibility check: automated structure checks plus a list of what to review by hand; it does not edit or add tags and is not a PDF/UA or WCAG conformance verdict.

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
pageweave --password-stdin unprotect -o open.pdf protected.pdf < password.txt
pageweave properties --title "Q3 report" --author "" -o report-q3.pdf report.pdf
pageweave text --pages 2-4 -o report.txt report.pdf     # without -o the text is printed
pageweave images-to-pdf scan1.jpg scan2.jpg --page-size a4 -o scans.pdf
pageweave attachments report.pdf                       # list; --save 0 -o file / --add a.csv -o copy.pdf
pageweave crop --top 10 --bottom 10 --unit mm -o report-trimmed.pdf report.pdf
pageweave watermark --text DRAFT --opacity 20 --angle 45 -o report-draft.pdf report.pdf
pageweave optimize -o report-smaller.pdf report.pdf    # lossless; images unchanged
pageweave optimize --image-dpi 150 --image-quality 75 -o photos-small.pdf photos.pdf   # also resample images
pageweave extract-images -o images/ report.pdf          # embedded images: JPEG as stored, others PNG
pageweave blank-pages scan.pdf                           # list blank pages; remove with delete --pages
pageweave orient -o scan-upright.pdf scan.pdf           # turn sideways/upside-down scans upright
pageweave deskew -o scan-straight.pdf scan.pdf           # straighten skewed scanned pages (lossless)
pageweave signature-mark --box 350,40,560,100 --text "Ada Lovelace" -o signed.pdf form.pdf   # picture of a signature
pageweave sign --certificate me.pfx -o contract-signed.pdf contract.pdf   # password on stdin
pageweave signatures contract.pdf                       # integrity of each signature (trust not checked yet)
pageweave compare report-v1.pdf report-v2.pdf          # page, text, format and visual differences
pageweave split --by-bookmarks 1 --out-dir chapters/ book.pdf   # or --max-size 10MB
pageweave form-export -o answers.xfdf form.pdf             # or answers.json / --format json
pageweave form-import --preview form.pdf answers.xfdf   # which fields it fills; -o filled.pdf to fill
pageweave accessibility report.pdf                     # automated checks + what to review by hand
pageweave patterns --kinds email,phone,card,iban,ssn report.pdf   # sensitive text with positions
pageweave sanitize --inspect report.pdf
pageweave sanitize --remove-web-links -o report-clean.pdf report.pdf
```

## Use with an AI agent (MCP)

`pageweave mcp` lets an AI agent that supports the Model Context Protocol (for example Claude Desktop or
Claude Code) use PageWeave's own document tools. It runs over standard input/output — nothing listens on
the network — and only touches files inside the folders you name:

```json
{
  "mcpServers": {
    "pageweave": {
      "command": "C:\\Program Files\\PageWeave\\pageweave.exe",
      "args": ["mcp", "--root", "C:\\Users\\you\\Documents\\PDFs", "--allow-write"]
    }
  }
}
```

- Read tools: `list_capabilities`, `inspect_document`, `extract_text`, `search_text`, `list_form_fields`,
  `compare_documents`, `check_signatures`, `check_accessibility`, `find_sensitive_text`.
- With `--allow-write` also: `extract_pages`, `delete_pages`, `rotate_pages`, `merge_documents`,
  `split_document`, `fill_form`, `recognize_text` (OCR, with progress and cancel), `reduce_file_size`,
  `export_text`. These only create **new** files; existing files are never replaced or changed.
- Not available through MCP: password-protected files, signing, redaction, removing hidden information,
  passwords and attachments. Paths outside the named folders (including through symbolic links) are refused.
- The agent and its AI model are separate from PageWeave; nothing here sends documents anywhere. What the
  agent does with text it reads from your documents is up to the agent.

## Privacy

PageWeave's own code makes no network requests while you work with documents (this has not yet been
verified with network monitoring on Windows, where the WebView2 runtime is a separate Microsoft component). Logs record operation names, timings,
page counts and error codes — never document text, file names (paths are replaced by a short hash) or
passwords. Passwords are held in memory only while a document is open. Logs live in the app's local data
folder; open it with the command palette (Ctrl+K) → "Show log folder".

## License

MIT — see `LICENSE`. Redistributed third-party components and their licenses: `THIRD_PARTY_NOTICES.md`
and `third_party/licenses/`.
