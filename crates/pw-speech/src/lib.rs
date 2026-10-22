//! PageWeave speech (PRD 09 read aloud, decision D-021): text to speech with the computer's own
//! voices, and timings for highlighting what is being said.
//!
//! - Windows: the system speech synthesizer (Windows.Media.SpeechSynthesis), which reports when
//!   each word starts, so highlighting follows words.
//! - Other systems (development and CI only): the `flite` program when it is installed, timed per
//!   sentence (each sentence is synthesized on its own, so its start and end are exact).
//!
//! No voice data is bundled. Timings always come from the synthesized audio, never from a clock.
//! Character offsets count Unicode characters of the text given.

use pw_core::{ErrorCategory, PwError, PwResult};
use serde::{Deserialize, Serialize};

#[cfg(not(windows))]
mod flite;
#[cfg(windows)]
mod winrt;

#[cfg(not(windows))]
use flite as backend;
#[cfg(windows)]
use winrt as backend;

pub mod wav;

/// Longest text accepted in one call (characters).
pub const MAX_TEXT_CHARS: usize = 200_000;
/// Speaking rate limits (1.0 is the voice's normal speed).
pub const MIN_RATE: f32 = 0.5;
pub const MAX_RATE: f32 = 2.0;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Voice {
    /// Stable identifier to pass back to `speak`.
    pub id: String,
    /// Name to show (the system's own).
    pub name: String,
    /// BCP 47 language tag, e.g. "en-US".
    pub language: String,
    /// The system's default voice.
    pub default: bool,
}

/// How precisely the timings follow the speech.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Precision {
    Word,
    Sentence,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EngineInfo {
    /// What to show: "Windows speech" or "Flite".
    pub name: String,
    pub precision: Precision,
    pub voices: Vec<Voice>,
}

/// A stretch of text and when it is heard.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mark {
    /// First character and one past the last.
    pub start: u32,
    pub end: u32,
    /// Milliseconds from the start of the audio.
    pub start_ms: u32,
    pub end_ms: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Speech {
    pub sample_rate: u32,
    /// Mono, 16-bit.
    pub samples: Vec<i16>,
    /// Every sentence that was spoken, in order.
    pub sentences: Vec<Mark>,
    /// Every word, in order, when the engine reports words (empty otherwise).
    pub words: Vec<Mark>,
    pub precision: Precision,
}

impl Speech {
    pub fn duration_ms(&self) -> u32 {
        if self.sample_rate == 0 {
            return 0;
        }
        (self.samples.len() as u64 * 1000 / self.sample_rate as u64) as u32
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SpeakOptions {
    /// A voice id from `engine_info`; the system default when `None`.
    pub voice: Option<String>,
    /// 1.0 is normal; limited to `MIN_RATE..=MAX_RATE`.
    pub rate: f32,
}

impl Default for SpeakOptions {
    fn default() -> Self {
        SpeakOptions { voice: None, rate: 1.0 }
    }
}

/// One synthesized piece of text from a backend.
pub(crate) struct Chunk {
    pub sample_rate: u32,
    pub samples: Vec<i16>,
    /// Word marks relative to the piece (character offsets within it).
    pub words: Vec<Mark>,
}

pub(crate) fn engine_error(detail: impl std::fmt::Display) -> PwError {
    PwError::new(ErrorCategory::EngineFailure, "speech.failed", format!("Speech could not be produced ({detail})."))
}

pub(crate) fn no_engine() -> PwError {
    PwError::new(ErrorCategory::MissingResource, "speech.unavailable", "No speech voice is available on this computer.")
        .with_action(if cfg!(windows) {
            "Add a voice in Windows Settings ▸ Time & language ▸ Speech, then try again."
        } else {
            "Install the flite speech program, then try again."
        })
}

pub(crate) fn unknown_voice() -> PwError {
    PwError::new(ErrorCategory::NotFound, "speech.voice", "The chosen voice is not available on this computer.")
        .with_action("Choose another voice.")
}

/// The speech engine and its voices, or why there is none.
pub fn engine_info() -> PwResult<EngineInfo> {
    let voices = backend::voices()?;
    if voices.is_empty() {
        return Err(no_engine());
    }
    Ok(EngineInfo { name: backend::NAME.to_string(), precision: backend::PRECISION, voices })
}

/// Sentences of `text` as character ranges (whitespace around them excluded). A sentence ends at
/// `.`, `!`, `?`, `…` (and their full-width forms) followed by a space or the end, or at a line end.
/// Common abbreviations ("Mr.", "e.g.") and decimal numbers do not end a sentence.
pub fn sentences(text: &str) -> Vec<(usize, usize)> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut start: Option<usize> = None;
    let push = |out: &mut Vec<(usize, usize)>, s: usize, e: usize, chars: &[char]| {
        let mut e = e;
        while e > s && chars[e - 1].is_whitespace() {
            e -= 1;
        }
        if e > s {
            out.push((s, e));
        }
    };
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if start.is_none() {
            if !c.is_whitespace() {
                start = Some(i);
            }
            i += 1;
            continue;
        }
        let s = start.expect("start");
        if c == '\n' {
            push(&mut out, s, i, &chars);
            start = None;
            i += 1;
            continue;
        }
        if matches!(c, '.' | '!' | '?' | '…' | '。' | '！' | '？') {
            // Closing quotes and brackets belong to the sentence.
            let mut j = i + 1;
            while j < chars.len() && matches!(chars[j], '"' | '\'' | '”' | '’' | ')' | ']' | '»' | '.' | '!' | '?')
            {
                j += 1;
            }
            let at_end = j >= chars.len() || chars[j].is_whitespace() || matches!(c, '。' | '！' | '？');
            if at_end && !(c == '.' && is_abbreviation(&chars[s..i])) {
                push(&mut out, s, j, &chars);
                start = None;
                i = j;
                continue;
            }
        }
        i += 1;
    }
    if let Some(s) = start {
        push(&mut out, s, chars.len(), &chars);
    }
    out
}

fn is_abbreviation(before: &[char]) -> bool {
    let word: String =
        before.iter().rev().take_while(|c| !c.is_whitespace()).collect::<Vec<_>>().into_iter().rev().collect();
    let w = word.trim_start_matches(['(', '"', '\'']).to_ascii_lowercase();
    const ABBR: &[&str] =
        &["mr", "mrs", "ms", "dr", "prof", "sr", "jr", "st", "vs", "etc", "e.g", "i.e", "fig", "approx", "dept"];
    ABBR.contains(&w.as_str()) || (w.chars().count() == 1 && w.chars().all(|c| c.is_alphabetic()))
}

/// Speak `text`. Each sentence is synthesized separately (and `cancelled` checked between them),
/// so sentence timings are exact; word timings come from the engine where it reports them.
pub fn speak(text: &str, opts: &SpeakOptions, cancelled: &dyn Fn() -> bool) -> PwResult<Speech> {
    let n = text.chars().count();
    if n > MAX_TEXT_CHARS {
        return Err(PwError::invalid(
            "speech.too_long",
            format!("The text is too long to read in one go ({n} characters; at most {MAX_TEXT_CHARS})."),
        ));
    }
    if !opts.rate.is_finite() || !(MIN_RATE..=MAX_RATE).contains(&opts.rate) {
        return Err(PwError::invalid("speech.rate", "The reading speed must be between 0.5× and 2×."));
    }
    let chars: Vec<char> = text.chars().collect();
    let mut speech = Speech {
        sample_rate: 0,
        samples: Vec::new(),
        sentences: Vec::new(),
        words: Vec::new(),
        precision: backend::PRECISION,
    };
    for (s, e) in sentences(text) {
        if cancelled() {
            return Err(PwError::cancelled());
        }
        let piece: String = chars[s..e].iter().collect();
        if !piece.chars().any(|c| c.is_alphanumeric()) {
            continue; // nothing to say ("—", "•")
        }
        let chunk = backend::synthesize(&piece, opts)?;
        if speech.sample_rate == 0 {
            speech.sample_rate = chunk.sample_rate;
        }
        let samples = if chunk.sample_rate == speech.sample_rate {
            chunk.samples
        } else {
            wav::resample(&chunk.samples, chunk.sample_rate, speech.sample_rate)
        };
        let at = (speech.samples.len() as u64 * 1000 / speech.sample_rate.max(1) as u64) as u32;
        speech.samples.extend_from_slice(&samples);
        let end = (speech.samples.len() as u64 * 1000 / speech.sample_rate.max(1) as u64) as u32;
        speech.sentences.push(Mark { start: s as u32, end: e as u32, start_ms: at, end_ms: end });
        for w in chunk.words {
            // Offsets were within the piece; times within its audio (resampling keeps durations).
            speech.words.push(Mark {
                start: s as u32 + w.start,
                end: (s as u32 + w.end).min(e as u32),
                start_ms: (at + w.start_ms).min(end),
                end_ms: (at + w.end_ms).min(end),
            });
        }
    }
    if speech.sample_rate == 0 {
        // Nothing to say: a valid, empty recording.
        speech.sample_rate = 16_000;
    }
    Ok(speech)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cut(t: &str) -> Vec<String> {
        let c: Vec<char> = t.chars().collect();
        sentences(t).into_iter().map(|(s, e)| c[s..e].iter().collect()).collect()
    }

    #[test]
    fn sentences_split_at_ends_and_lines_not_abbreviations_or_numbers() {
        assert_eq!(
            cut("Hello world. It costs 12.50 dollars, Mr. Smith!  Really? Yes…\nA heading\n\n  Last one"),
            vec!["Hello world.", "It costs 12.50 dollars, Mr. Smith!", "Really?", "Yes…", "A heading", "Last one"]
        );
        assert_eq!(cut("See e.g. the table. Fig. 3 shows it."), vec!["See e.g. the table.", "Fig. 3 shows it."]);
        assert_eq!(cut("He said \"stop.\" Then left."), vec!["He said \"stop.\"", "Then left."]);
        assert_eq!(cut("日本語です。次の文。"), vec!["日本語です。", "次の文。"]);
        assert!(cut("   \n \n").is_empty());
        assert_eq!(cut("J. R. Tolkien wrote it."), vec!["J. R. Tolkien wrote it."]);
    }

    #[test]
    fn bad_rates_and_huge_text_are_refused_before_speaking() {
        let e = speak("Hi.", &SpeakOptions { voice: None, rate: 3.0 }, &|| false).expect_err("rate");
        assert_eq!(e.code, "speech.rate");
        let e = speak("Hi.", &SpeakOptions { voice: None, rate: f32::NAN }, &|| false).expect_err("nan");
        assert_eq!(e.code, "speech.rate");
        let long = "a ".repeat(MAX_TEXT_CHARS);
        assert_eq!(speak(&long, &SpeakOptions::default(), &|| false).expect_err("long").code, "speech.too_long");
        // Nothing to say is an empty recording, not an error.
        let s = speak(" — • ", &SpeakOptions::default(), &|| false).expect("empty");
        assert!(s.samples.is_empty() && s.sentences.is_empty());
    }
}
