//! Windows backend: the system speech synthesizer (Windows.Media.SpeechSynthesis, Windows 10
//! 1709 or later), with word-boundary metadata for word timings. Voices are the ones installed
//! in Windows; nothing is bundled.

use crate::{engine_error, no_engine, unknown_voice, wav, Chunk, Mark, Precision, SpeakOptions, Voice};
use pw_core::PwResult;
use windows::core::{Interface, HSTRING};
use windows::Media::Core::SpeechCue;
use windows::Media::SpeechSynthesis::{SpeechSynthesizer, VoiceInformation};
use windows::Storage::Streams::DataReader;

pub const NAME: &str = "Windows speech";
pub const PRECISION: Precision = Precision::Word;

fn all() -> PwResult<Vec<VoiceInformation>> {
    let v = SpeechSynthesizer::AllVoices().map_err(|e| engine_error(e.message()))?;
    Ok(v.into_iter().collect())
}

pub fn voices() -> PwResult<Vec<Voice>> {
    let default = SpeechSynthesizer::DefaultVoice().and_then(|v| v.Id()).map(|h| h.to_string()).ok();
    let mut out = Vec::new();
    for v in all()? {
        let (Ok(id), Ok(name), Ok(lang)) = (v.Id(), v.DisplayName(), v.Language()) else { continue };
        let id = id.to_string();
        out.push(Voice {
            default: default.as_deref() == Some(id.as_str()),
            id,
            name: name.to_string(),
            language: lang.to_string(),
        });
    }
    Ok(out)
}

pub(crate) fn synthesize(text: &str, opts: &SpeakOptions) -> PwResult<Chunk> {
    let fail = |e: windows::core::Error| engine_error(e.message());
    let voices = all()?;
    if voices.is_empty() {
        return Err(no_engine());
    }
    let synth = SpeechSynthesizer::new().map_err(fail)?;
    if let Some(id) = &opts.voice {
        let v = voices.iter().find(|v| v.Id().map(|h| h == id.as_str()).unwrap_or(false)).ok_or_else(unknown_voice)?;
        synth.SetVoice(v).map_err(fail)?;
    }
    let o = synth.Options().map_err(fail)?;
    o.SetIncludeWordBoundaryMetadata(true).map_err(fail)?;
    o.SetSpeakingRate(opts.rate as f64).map_err(fail)?;
    let stream = synth.SynthesizeTextToStreamAsync(&HSTRING::from(text)).map_err(fail)?.join().map_err(fail)?;
    let size = stream.Size().map_err(fail)?;
    if size > 512 * 1024 * 1024 {
        return Err(engine_error("audio too large"));
    }
    let reader = DataReader::CreateDataReader(&stream.GetInputStreamAt(0).map_err(fail)?).map_err(fail)?;
    reader.LoadAsync(size as u32).map_err(fail)?.join().map_err(fail)?;
    let mut bytes = vec![0u8; size as usize];
    reader.ReadBytes(&mut bytes).map_err(fail)?;
    let (sample_rate, samples) = wav::decode(&bytes)?;

    // Word cues: the cue's own text is found in the sentence (in order), which is independent of
    // how the engine counts positions; its reported position is the fallback.
    let chars: Vec<char> = text.chars().collect();
    let mut words = Vec::new();
    let mut cursor = 0usize;
    for track in stream.TimedMetadataTracks().map_err(fail)? {
        if track.Id().map(|h| h.to_string()).unwrap_or_default() != "SpeechWord" {
            continue;
        }
        for cue in track.Cues().map_err(fail)? {
            let Ok(c) = cue.cast::<SpeechCue>() else { continue };
            let start_ms = (c.StartTime().map(|t| t.Duration).unwrap_or(0) / 10_000).max(0) as u32;
            let dur_ms = (c.Duration().map(|t| t.Duration).unwrap_or(0) / 10_000).max(0) as u32;
            let word: Vec<char> = c.Text().map(|h| h.to_string()).unwrap_or_default().chars().collect();
            let found = (!word.is_empty())
                .then(|| {
                    (cursor..chars.len().saturating_sub(word.len() - 1)).find(|&i| chars[i..i + word.len()] == word[..])
                })
                .flatten();
            let (s, e) = match found {
                Some(i) => (i, i + word.len()),
                None => {
                    let sp = c.StartPositionInInput().and_then(|r| r.Value()).unwrap_or(-1);
                    let ep = c.EndPositionInInput().and_then(|r| r.Value()).unwrap_or(-1);
                    if sp < 0 || ep < sp {
                        continue;
                    }
                    let (s, e) = (utf16_to_char(text, sp as usize), utf16_to_char(text, ep as usize + 1));
                    if s >= chars.len() {
                        continue;
                    }
                    (s, e.min(chars.len()))
                }
            };
            cursor = e;
            words.push(Mark { start: s as u32, end: e as u32, start_ms, end_ms: start_ms + dur_ms });
        }
    }
    Ok(Chunk { sample_rate, samples, words })
}

/// The character index at UTF-16 offset `u`.
fn utf16_to_char(text: &str, u: usize) -> usize {
    let mut units = 0;
    for (i, c) in text.chars().enumerate() {
        if units >= u {
            return i;
        }
        units += c.len_utf16();
    }
    text.chars().count()
}
