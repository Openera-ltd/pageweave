//! Development/CI backend (not Windows): the `flite` program (BSD-style licence) when installed;
//! never bundled. Each call synthesizes one sentence, so timings are per sentence.

use crate::{engine_error, no_engine, unknown_voice, wav, Chunk, Precision, SpeakOptions, Voice};
use pw_core::PwResult;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub const NAME: &str = "Flite";
pub const PRECISION: Precision = Precision::Sentence;
/// Explicit program (tests); when set (even empty) nothing else is looked at.
pub const FLITE_ENV: &str = "PAGEWEAVE_FLITE";
const TIMEOUT: Duration = Duration::from_secs(60);

fn program() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os(FLITE_ENV) {
        let p = PathBuf::from(p);
        return p.is_file().then_some(p);
    }
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).map(|d| d.join("flite")).find(|p| p.is_file())
}

pub fn voices() -> PwResult<Vec<Voice>> {
    let Some(p) = program() else { return Ok(Vec::new()) };
    let out = Command::new(&p).arg("-lv").stdin(Stdio::null()).stderr(Stdio::null()).output();
    let Ok(out) = out else { return Ok(Vec::new()) };
    let text = String::from_utf8_lossy(&out.stdout);
    let ids: Vec<&str> = text
        .split_once(':')
        .map(|(_, v)| v.split_whitespace().filter(|v| !v.contains("time")).collect())
        .unwrap_or_default();
    let default = if ids.contains(&"slt") { "slt" } else { ids.first().copied().unwrap_or("") };
    Ok(ids
        .iter()
        .map(|id| Voice {
            id: id.to_string(),
            name: format!("Flite {}", id.to_ascii_uppercase()),
            language: "en-US".into(),
            default: *id == default,
        })
        .collect())
}

pub(crate) fn synthesize(text: &str, opts: &SpeakOptions) -> PwResult<Chunk> {
    let p = program().ok_or_else(no_engine)?;
    let voices = voices()?;
    let voice = match &opts.voice {
        Some(v) => voices.iter().find(|x| &x.id == v).ok_or_else(unknown_voice)?.id.clone(),
        None => voices.iter().find(|v| v.default).ok_or_else(no_engine)?.id.clone(),
    };
    let dir = tempfile::Builder::new().prefix("pageweave-speech-").tempdir().map_err(|e| engine_error(e.kind()))?;
    let input = dir.path().join("text.txt");
    let output = dir.path().join("speech.wav");
    std::fs::write(&input, text).map_err(|e| engine_error(e.kind()))?;
    let mut child = Command::new(&p)
        .args(["-voice", &voice, "--setf", &format!("duration_stretch={:.3}", 1.0 / opts.rate), "-f"])
        .arg(&input)
        .arg("-o")
        .arg(&output)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| engine_error(e.kind()))?;
    let started = Instant::now();
    let status = loop {
        match child.try_wait().map_err(|e| engine_error(e.kind()))? {
            Some(s) => break s,
            None if started.elapsed() > TIMEOUT => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(engine_error("took too long"));
            }
            None => std::thread::sleep(Duration::from_millis(10)),
        }
    };
    if !status.success() {
        return Err(engine_error("the speech program failed"));
    }
    let bytes = std::fs::read(&output).map_err(|e| engine_error(e.kind()))?;
    let (sample_rate, samples) = wav::decode(&bytes)?;
    Ok(Chunk { sample_rate, samples, words: Vec::new() })
}
