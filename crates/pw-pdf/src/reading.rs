//! Text for reading aloud (PRD 09): one page's words in reading order with where each is on the
//! page, from the same layout analysis as Word export (columns read one after another, tables
//! cell by cell). Offsets count Unicode characters (not bytes, not UTF-16 units).

use crate::document::{load, OpenOptions};
use crate::{engine, map_pdfium_error, SearchRect};
use pdfium_render::prelude::*;
use pw_core::{PwError, PwResult};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReadingWord {
    /// First character and one past the last, in `ReadingPage::text`.
    pub start: u32,
    pub end: u32,
    /// Where the word is, in the page's own coordinates (one rectangle per line it is on; none
    /// for text without a position, such as turned text).
    pub rects: Vec<SearchRect>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReadingPage {
    /// 0-based page.
    pub page: u32,
    /// Paragraphs separated by `\n`.
    pub text: String,
    pub words: Vec<ReadingWord>,
}

/// The reading-order text of one page.
pub fn reading_text(path: &Path, opts: &OpenOptions, page_index: u32) -> PwResult<ReadingPage> {
    let pdfium = engine()?;
    let doc = load(&pdfium, path, opts)?;
    let count = doc.pages().len().max(0) as u32;
    if page_index >= count {
        return Err(PwError::invalid(
            "text.page_out_of_bounds",
            format!("Page {} does not exist; the document has {count} pages.", page_index + 1),
        ));
    }
    let page = doc.pages().get(page_index as PdfPageIndex).map_err(|e| map_pdfium_error(&e, "page"))?;
    let mut r = crate::docx::page_reading(&page, &doc)?;
    r.page = page_index;
    Ok(r)
}

/// What a WAV export said.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ReadAloudReport {
    pub pages: u32,
    /// Pages with nothing to read (no text layer).
    pub pages_without_text: u32,
    pub duration_ms: u64,
    pub sentences: usize,
    pub sample_rate: u32,
    pub engine: String,
    /// The voice used (its system name).
    pub voice: String,
}

/// Longest recording written (the WAV format's limit is far above this; memory is the bound).
pub const MAX_RECORDING_MS: u64 = 3 * 60 * 60 * 1000;

/// Read pages (all when `pages` is absent) aloud into a WAV file (mono, 16-bit PCM).
#[allow(clippy::too_many_arguments)]
pub fn read_aloud_to_wav(
    input: &Path,
    opts: &OpenOptions,
    pages: Option<&str>,
    voice: Option<&str>,
    rate: f32,
    output: &Path,
    policy: pw_core::output::CollisionPolicy,
    cancelled: &dyn Fn() -> bool,
    progress: &dyn Fn(u64, u64),
) -> PwResult<(pw_core::output::CommittedOutput, ReadAloudReport)> {
    use pw_core::ErrorCategory;
    let info = pw_speech::engine_info()?;
    let voice_name = match voice {
        Some(v) => info.voices.iter().find(|x| x.id == v).map(|x| x.name.clone()).ok_or_else(|| {
            PwError::new(ErrorCategory::NotFound, "speech.voice", "The chosen voice is not available on this computer.")
                .with_action("Choose another voice.")
        })?,
        None => {
            info.voices.iter().find(|x| x.default).or(info.voices.first()).map(|x| x.name.clone()).unwrap_or_default()
        }
    };
    // Text first, with the engine lock released before speaking (which takes much longer).
    let texts: Vec<String> = {
        let pdfium = engine()?;
        let doc = load(&pdfium, input, opts)?;
        let n = doc.pages().len().max(0) as u32;
        let idx: Vec<u32> = match pages {
            Some(spec) => pw_core::range::parse_page_ranges(spec, n)?,
            None => (0..n).collect(),
        };
        let mut v = Vec::with_capacity(idx.len());
        for &p in &idx {
            if cancelled() {
                return Err(PwError::cancelled());
            }
            let page = doc.pages().get(p as PdfPageIndex).map_err(|e| map_pdfium_error(&e, "page"))?;
            v.push(crate::docx::page_reading(&page, &doc)?.text);
        }
        v
    };
    let total = texts.len() as u64;
    let without = texts.iter().filter(|t| !t.chars().any(char::is_alphanumeric)).count() as u32;
    if without as u64 == total {
        return Err(PwError::new(
            ErrorCategory::UnsupportedOperation,
            "speech.no_text",
            "These pages have no text to read aloud.",
        )
        .with_action("If they are scans, recognize their text first (Make searchable)."));
    }
    let sopts = pw_speech::SpeakOptions { voice: voice.map(str::to_string), rate };
    let mut rate_hz = 0u32;
    let mut samples: Vec<i16> = Vec::new();
    let mut sentences = 0usize;
    for (k, t) in texts.iter().enumerate() {
        let s = pw_speech::speak(t, &sopts, cancelled)?;
        if !s.samples.is_empty() {
            if rate_hz == 0 {
                rate_hz = s.sample_rate;
            }
            let part = pw_speech::wav::resample(&s.samples, s.sample_rate, rate_hz);
            samples.extend_from_slice(&part);
            // A short pause between pages.
            samples.extend(std::iter::repeat_n(0i16, (rate_hz / 2) as usize));
        }
        sentences += s.sentences.len();
        if samples.len() as u64 * 1000 / rate_hz.max(1) as u64 > MAX_RECORDING_MS {
            return Err(PwError::invalid(
                "speech.too_long",
                "The recording would be longer than 3 hours. Choose fewer pages.",
            ));
        }
        progress(k as u64 + 1, total);
    }
    let bytes = pw_speech::wav::encode(rate_hz, &samples);
    let want = samples.len();
    let c = pw_core::output::write_atomic_bytes(output, policy, &[input.to_path_buf()], &bytes, |staged| {
        let back = std::fs::read(staged).map_err(|e| PwError::from_io(&e, "output file"))?;
        let (r, s) = pw_speech::wav::decode(&back)?;
        if r != rate_hz || s.len() != want || back != bytes {
            return Err(PwError::new(
                ErrorCategory::ValidationFailed,
                "output.verify",
                "The written recording differs and was not kept.",
            ));
        }
        Ok(())
    })?;
    let report = ReadAloudReport {
        pages: total as u32,
        pages_without_text: without,
        duration_ms: want as u64 * 1000 / rate_hz.max(1) as u64,
        sentences,
        sample_rate: rate_hz,
        engine: info.name,
        voice: voice_name,
    };
    Ok((c, report))
}
