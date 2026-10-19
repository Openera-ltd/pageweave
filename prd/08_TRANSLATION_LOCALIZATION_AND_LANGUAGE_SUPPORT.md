# PRD 08 — Translation, Localization and Script Integrity

## Three separate dimensions
1. UI localization (menus, dialogs, tooltips, docs and accessible labels).
2. OCR language/script recognition.
3. Document translation directions/quality.

Do not claim language support from success in just one dimension. Maintain a machine-readable per-language registry: BCP-47/script; UI translated; OCR tested; TTS voice and alignment availability; source-target translation directions; RTL/vertical writing flags; fonts and shaping tests; model/resource/license; evaluation date, quality tier and known limitations.

## Translation workflows
Translate selection, page, selected pages, entire document, a folder/batch, tables/forms and OCR text within images. Detect source language with manual override. Preview source/target and scope, preserve original by default, offer side-by-side and bilingual output, show changed regions and quality report. Batch mode has output collision policy, retry and per-file status.

Preserve structure/styles/text boxes/tables/cells/headers/footers/footnotes/headings/links/reading order/images as much as possible. If target text expands, resize/reflow rather than silently shrinking to unreadability. Detect clipping, collisions and overflow. Support RTL shaping, bidi, mixed English/Arabic/Hebrew, punctuation/numerals/caret-selection/font/export consistency. Vertical text and complex scripts need independent coverage; do not promise universality.

Preserve names, brands, IDs, URLs, equations, monetary values, dates and model numbers; allow glossaries and do-not-translate spans. Separate OCR confidence from translation confidence so users can identify which stage is uncertain. Local models support offline directions; no silent fallback to remote processing. External APIs are called only after explicit selection and data disclosure.

## Evaluation
Use rights-cleared parallel corpora, blind human-reviewed examples and varied domains. Track adequacy, terminology, protected-token fidelity, omissions/insertions/hallucinations, layout overflow, font rendering and output readability; automatic metrics alone are insufficient. Unsupported directions are disabled with a reason or clearly labeled external option. Version quality tiers; never state “all languages” without evidence.
