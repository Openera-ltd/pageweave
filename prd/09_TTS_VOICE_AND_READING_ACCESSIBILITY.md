# PRD 09 — Read Aloud, TTS, Voice Profiles and Reading Accessibility

## Read-aloud experience
Select text and choose Read Aloud from context menu, selection toolbar, shortcut or command palette. Read selection, paragraph, page, section or whole document/from here. Controls: play/pause/resume/stop, previous/next sentence, skip forward/back, rate, voice/language, pronunciation, progress and audio export.

Word highlight follows actual speech timestamps from TTS/aligner. If only phrase-level timing is reliable, use phrase/sentence highlighting and describe that precision; never drive word highlights with a timer that drifts. Auto-scroll follows current region gently. Manual scrolling temporarily suppresses auto-follow; user can resume it. Highlight does not hide text or destroy annotation. Reading order handles columns/footnotes/forms/language changes; do not read hidden text or duplicate OCR layers.

Resume exact position where possible using file identity/content hash, page, character/region offset, voice, rate and section. If document changed, detect stale position and offer relocation. Audio device loss, sleep/wake, document switching or unavailable voices get recoverable states.

## Voices and audio export
Natural local voices with preview, supported speed/rate/pitch/volume, language switching, pronunciation dictionary, abbreviation/acronym expansion and number/date/currency rules. Export to WAV and other tested encoder-supported formats, by document/chapter/section, with metadata and resumable long exports. Voice quality/language list comes from tests—not just model metadata. Offline voices/resources installed during setup; user sees friendly capability names, not technical filenames.

## Consent-based voice cloning
Only recordings the user is authorized to use. Require affirmative consent and purpose confirmation, explain variable similarity (never promise a perfect clone), microphone/sample checks, preview, secure local storage, delete/revoke/export controls, restrictive permissions and exclusion from ordinary logs/diagnostics. No outbound voice samples without a clear opt-in to a named external service. Include appropriate synthetic provenance labeling and safeguards against non-consensual impersonation/deception. Test voice-profile deletion and active-job cleanup.

## Accessibility
Reflow, adjustable zoom/spacing/line length, reading focus, page/section progress, highlight style/contrast, keyboard operations, screen-reader interoperability, accessible progress/errors and optional dyslexia-friendly display preferences without medical claims. TTS coverage, UI localization and OCR coverage remain separate.

## Failure tests
No audio output device; device removed mid-read; unavailable voice; mixed-language page; OCR failure; changed selection; pause/stop/speed switch during utterance; output path lost; interrupted export; sleep/wake; profile deleted while in use; concurrent jobs exhaust memory; alignment unavailable; manual scroll during follow; and restart recovery.
