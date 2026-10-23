//! PageWeave translation (PRD 08, decision D-022): OPUS-MT (Marian) translation models, one per
//! language direction, run on the same pinned ONNX Runtime as text recognition. Each model folder
//! holds the encoder, the decoder (with key/value cache), `tokenizer.json` and `config.json`; every
//! file is checked against `resources/translate.lock.json` before first use.
//!
//! Text is translated sentence by sentence (greedy decoding, the end-of-sentence token or a length
//! cap ends a sentence); line breaks are kept. Nothing leaves the computer.

pub mod detect;
pub mod install;
pub mod tokenizer;

use ort::session::Session;
use ort::value::{DynValue, Tensor};
use pw_core::{ErrorCategory, PwError, PwResult};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use tokenizer::Tokenizer;

pub const ENCODER_FILE: &str = "encoder.onnx";
pub const DECODER_FILE: &str = "decoder.onnx";
pub const TOKENIZER_FILE: &str = "tokenizer.json";
pub const CONFIG_FILE: &str = "config.json";

const LOCK: &str = include_str!("../../../resources/translate.lock.json");
/// Longest sentence accepted (tokens); longer ones are split at commas or by length.
const MAX_INPUT_TOKENS: usize = 400;
/// Longest text accepted in one call (characters).
pub const MAX_TEXT_CHARS: usize = 500_000;

#[derive(Debug, Clone, Deserialize)]
struct LockFile {
    from: String,
    sha256: String,
    bytes: u64,
}

#[derive(Debug, Clone, Deserialize)]
struct LockPair {
    source: String,
    target: String,
    repository: String,
    commit: String,
    files: HashMap<String, LockFile>,
}

#[derive(Debug, Clone, Deserialize)]
struct Lock {
    pairs: Vec<LockPair>,
}

fn lock() -> PwResult<Lock> {
    serde_json::from_str(LOCK)
        .map_err(|_| PwError::internal("translate.lock", "The translation resource lock is unreadable."))
}

/// A language direction PageWeave can translate, and whether its model is installed and intact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Direction {
    /// BCP 47 codes, e.g. "en" → "de".
    pub source: String,
    pub target: String,
    pub installed: bool,
    /// Size of the model download (bytes).
    pub bytes: u64,
}

fn engine_err(e: impl std::fmt::Display) -> PwError {
    PwError::new(ErrorCategory::EngineFailure, "translate.engine", format!("Translation failed ({e})."))
}

fn missing(pair: &str) -> PwError {
    PwError::new(
        ErrorCategory::MissingResource,
        "translate.missing",
        format!("The translation model for {pair} is not installed."),
    )
    .with_action(
        "Download it when translating (File ▸ Translate…), or run `pageweave translate --install SOURCE-TARGET`.",
    )
}

fn corrupt(pair: &str, file: &str) -> PwError {
    PwError::new(
        ErrorCategory::CorruptResource,
        "translate.corrupt",
        format!("The translation model for {pair} is damaged ({file})."),
    )
    .with_action("Download it again (File ▸ Translate…, or `pageweave translate --install SOURCE-TARGET`).")
}

/// Folders that may hold one folder per direction (`en-de/`, …), searched in order: models shipped
/// next to the executable, the development install (debug builds), and the user's downloads.
pub fn roots() -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = default_resource_dir().into_iter().collect();
    if let Some(u) = install::user_dir() {
        if !v.contains(&u) {
            v.push(u);
        }
    }
    v
}

/// The folder of an installed direction (every pinned file present at its pinned size; the full
/// checksum is verified when the model is loaded).
pub fn pair_dir(source: &str, target: &str) -> Option<PathBuf> {
    let p = lock().ok()?.pairs.into_iter().find(|p| p.source == source && p.target == target)?;
    roots()
        .into_iter()
        .map(|r| r.join(format!("{source}-{target}")))
        .find(|d| p.files.iter().all(|(n, f)| std::fs::metadata(d.join(n)).is_ok_and(|m| m.len() == f.bytes)))
}

/// The folder holding one folder per direction (`en-de/`, …): `translate/` next to the executable
/// (or under `resources/`); in debug builds also `PAGEWEAVE_TRANSLATE_DIR` and the development
/// install `third_party/translate`.
pub fn default_resource_dir() -> Option<PathBuf> {
    if cfg!(debug_assertions) {
        if let Some(d) = std::env::var_os("PAGEWEAVE_TRANSLATE_DIR") {
            return Some(PathBuf::from(d));
        }
    }
    let mut c = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            c.push(dir.join("translate"));
            c.push(dir.join("resources").join("translate"));
        }
    }
    if cfg!(debug_assertions) {
        c.push(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../third_party/translate"));
    }
    c.into_iter().find(|d| d.is_dir())
}

fn sha256_file(p: &Path) -> PwResult<String> {
    use sha2::{Digest, Sha256};
    let mut f = std::fs::File::open(p).map_err(|e| PwError::from_io(&e, "translation model"))?;
    let mut h = Sha256::new();
    std::io::copy(&mut f, &mut h).map_err(|e| PwError::from_io(&e, "translation model"))?;
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// One installed model file checked against the lock (`None`: missing).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FileCheck {
    pub name: String,
    pub matches: Option<bool>,
    pub bytes: u64,
}

/// Check every file of every installed direction (reads each file once; loads nothing). Directions
/// not downloaded are not listed.
pub fn check_files() -> PwResult<Vec<FileCheck>> {
    let mut out = Vec::new();
    for p in lock()?.pairs {
        let pair = format!("{}-{}", p.source, p.target);
        // Only complete downloads: an interrupted one is "not downloaded yet", not damage.
        let Some(dir) = pair_dir(&p.source, &p.target) else {
            continue;
        };
        let mut names: Vec<&String> = p.files.keys().collect();
        names.sort();
        for name in names {
            let path = dir.join(name);
            let Ok(meta) = std::fs::metadata(&path) else {
                out.push(FileCheck { name: format!("{pair}/{name}"), matches: None, bytes: 0 });
                continue;
            };
            let f = &p.files[name];
            let ok = meta.len() == f.bytes && sha256_file(&path)? == f.sha256;
            out.push(FileCheck { name: format!("{pair}/{name}"), matches: Some(ok), bytes: meta.len() });
        }
    }
    Ok(out)
}

/// The directions PageWeave offers, with whether each one is installed and its download size.
pub fn directions() -> PwResult<Vec<Direction>> {
    Ok(lock()?
        .pairs
        .into_iter()
        .map(|p| {
            let installed = pair_dir(&p.source, &p.target).is_some();
            let bytes = p.files.values().map(|f| f.bytes).sum();
            Direction { source: p.source, target: p.target, installed, bytes }
        })
        .collect())
}

struct Config {
    start: u32,
    eos: u32,
    pad: u32,
    layers: usize,
    heads: usize,
    head_dim: usize,
    vocab: usize,
}

/// One loaded direction.
pub struct Translator {
    encoder: Session,
    decoder: Session,
    tok: Tokenizer,
    cfg: Config,
}

impl Translator {
    /// Verify and load the model for `source` → `target` from `dir` (the folder of directions).
    pub fn load(dir: &Path, source: &str, target: &str) -> PwResult<Translator> {
        let ocr = pw_ocr::default_resource_dir().ok_or_else(|| {
            PwError::new(ErrorCategory::MissingResource, "translate.runtime", "The model runtime is not installed.")
                .with_action("Reinstall PageWeave to add it.")
        })?;
        Self::load_with_runtime(dir, &ocr, source, target)
    }

    /// As [`Translator::load`], with the folder holding the ONNX Runtime library given (benchmarks).
    pub fn load_with_runtime(dir: &Path, runtime_dir: &Path, source: &str, target: &str) -> PwResult<Translator> {
        let pair = format!("{source}-{target}");
        let lp = lock()?.pairs.into_iter().find(|p| p.source == source && p.target == target).ok_or_else(|| {
            PwError::new(
                ErrorCategory::UnsupportedOperation,
                "translate.direction",
                format!("Translating from {source} to {target} is not available."),
            )
        })?;
        let d = dir.join(&pair);
        for (name, f) in &lp.files {
            let p = d.join(name);
            let meta = std::fs::metadata(&p).map_err(|_| missing(&pair))?;
            if meta.len() != f.bytes || sha256_file(&p)? != f.sha256 {
                return Err(corrupt(&pair, name));
            }
        }
        let env = pw_ocr::runtime(runtime_dir)?;
        let session = |name: &str| -> PwResult<Session> {
            Session::builder(env).map_err(engine_err)?.commit_from_file(d.join(name)).map_err(|_| corrupt(&pair, name))
        };
        let encoder = session(ENCODER_FILE)?;
        let decoder = session(DECODER_FILE)?;
        let tok = Tokenizer::from_json(
            &std::fs::read_to_string(d.join(TOKENIZER_FILE)).map_err(|e| PwError::from_io(&e, "translation model"))?,
        )?;
        let c: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(d.join(CONFIG_FILE)).map_err(|e| PwError::from_io(&e, "translation model"))?,
        )
        .map_err(|_| corrupt(&pair, CONFIG_FILE))?;
        let num = |k: &str| c[k].as_u64().map(|v| v as usize).ok_or_else(|| corrupt(&pair, CONFIG_FILE));
        let heads = num("decoder_attention_heads")?;
        let cfg = Config {
            start: num("decoder_start_token_id")? as u32,
            eos: num("eos_token_id")? as u32,
            pad: num("pad_token_id")? as u32,
            layers: num("decoder_layers")?,
            heads,
            head_dim: num("d_model")? / heads.max(1),
            vocab: num("vocab_size")?,
        };
        if tok.vocab_size() != cfg.vocab {
            return Err(corrupt(&pair, TOKENIZER_FILE));
        }
        Ok(Translator { encoder, decoder, tok, cfg })
    }

    /// Translate one sentence (greedy decoding with the decoder's key/value cache).
    pub fn sentence(&mut self, text: &str) -> PwResult<String> {
        let mut ids: Vec<i64> = self.tok.encode(text).into_iter().map(i64::from).collect();
        if ids.is_empty() {
            return Ok(String::new());
        }
        ids.truncate(MAX_INPUT_TOKENS);
        ids.push(self.cfg.eos as i64);
        let n = ids.len();
        let mask = vec![1i64; n];
        let enc_out = self
            .encoder
            .run(ort::inputs![
                "input_ids" => Tensor::from_array(([1usize, n], ids.clone())).map_err(engine_err)?,
                "attention_mask" => Tensor::from_array(([1usize, n], mask.clone())).map_err(engine_err)?,
            ])
            .map_err(engine_err)?;
        let (shape, hidden) = enc_out["last_hidden_state"].try_extract_tensor::<f32>().map_err(engine_err)?;
        let hidden_shape: Vec<usize> = shape.iter().map(|&d| d as usize).collect();
        let hidden = hidden.to_vec();
        drop(enc_out);

        let (l, h, hd) = (self.cfg.layers, self.cfg.heads, self.cfg.head_dim);
        let empty = || -> PwResult<DynValue> {
            Ok(Tensor::from_array(([1usize, h, 0, hd], Vec::<f32>::new())).map_err(engine_err)?.into_dyn())
        };
        // Self-attention cache (key, value per layer), grows each step; cross-attention cache
        // (key, value per layer), computed on the first step only (later steps return it empty).
        let mut own: Vec<DynValue> = (0..2 * l).map(|_| empty()).collect::<PwResult<_>>()?;
        let mut cross: Vec<DynValue> = (0..2 * l).map(|_| empty()).collect::<PwResult<_>>()?;
        let mut out: Vec<u32> = Vec::new();
        let max_new = (2 * n + 10).min(512);
        let mut next = self.cfg.start as i64;
        for step in 0..max_new {
            let mut inputs: Vec<(String, ort::session::SessionInputValue)> = vec![
                ("input_ids".into(), Tensor::from_array(([1usize, 1], vec![next])).map_err(engine_err)?.into()),
                (
                    "encoder_attention_mask".into(),
                    Tensor::from_array(([1usize, n], mask.clone())).map_err(engine_err)?.into(),
                ),
                (
                    "encoder_hidden_states".into(),
                    Tensor::from_array((hidden_shape.clone(), hidden.clone())).map_err(engine_err)?.into(),
                ),
                ("use_cache_branch".into(), Tensor::from_array(([1usize], vec![step > 0])).map_err(engine_err)?.into()),
            ];
            for (k, v) in std::mem::take(&mut own).into_iter().enumerate() {
                inputs.push((format!("past_key_values.{}.decoder.{}", k / 2, ["key", "value"][k % 2]), v.into()));
            }
            for (k, v) in cross.iter().enumerate() {
                inputs
                    .push((format!("past_key_values.{}.encoder.{}", k / 2, ["key", "value"][k % 2]), v.view().into()));
            }
            let mut outputs = self.decoder.run(inputs).map_err(|e| engine_err(format!("step {step}: {e}")))?;
            let (_, logits) = outputs["logits"].try_extract_tensor::<f32>().map_err(engine_err)?;
            let v = self.cfg.vocab;
            if logits.len() < v {
                return Err(engine_err("unexpected output size"));
            }
            let last = &logits[logits.len() - v..];
            let mut best = 0usize;
            let mut best_score = f32::NEG_INFINITY;
            for (i, &x) in last.iter().enumerate() {
                if i as u32 != self.cfg.pad && x > best_score {
                    best_score = x;
                    best = i;
                }
            }
            for k in 0..2 * l {
                let name = format!("present.{}.decoder.{}", k / 2, ["key", "value"][k % 2]);
                own.push(outputs.remove(&name).ok_or_else(|| engine_err("missing decoder state"))?);
            }
            if step == 0 {
                let mut c = Vec::with_capacity(2 * l);
                for k in 0..2 * l {
                    let name = format!("present.{}.encoder.{}", k / 2, ["key", "value"][k % 2]);
                    c.push(outputs.remove(&name).ok_or_else(|| engine_err("missing decoder state"))?);
                }
                drop(outputs);
                cross = c;
            }
            if best as u32 == self.cfg.eos {
                break;
            }
            out.push(best as u32);
            next = best as i64;
        }
        Ok(self.tok.decode(&out))
    }

    /// Translate a sentence with addresses and codes masked: they are replaced by placeholders the
    /// models copy unchanged (`PWX1`, …), and put back afterwards.
    pub fn masked(&mut self, sentence: &str) -> PwResult<String> {
        let (masked, originals) = mask(sentence);
        let t = self.sentence(&masked)?;
        if originals.is_empty() || placeholders_kept(&t, originals.len()) {
            return Ok(unmask(&t, &originals));
        }
        // The model dropped a placeholder (seen at the start of short fragments): translate the
        // pieces between protected items separately and keep the items in place.
        let mut out = String::new();
        let mut rest = masked.as_str();
        for (i, o) in originals.iter().enumerate() {
            let ph = format!("PWX{}", i + 1);
            let Some(at) = rest.find(&ph) else { break };
            let before = &rest[..at];
            out.push_str(&self.piece(before)?);
            out.push_str(o);
            rest = &rest[at + ph.len()..];
        }
        out.push_str(&self.piece(rest)?);
        Ok(out)
    }

    /// Translate a fragment between protected items, keeping its outer spacing and leaving
    /// punctuation-only fragments as they are.
    fn piece(&mut self, s: &str) -> PwResult<String> {
        if !s.chars().any(char::is_alphabetic) {
            return Ok(s.to_string());
        }
        let lead = &s[..s.len() - s.trim_start().len()];
        let trail = &s[s.trim_end().len()..];
        Ok(format!("{lead}{}{trail}", self.sentence(s.trim())?))
    }

    /// Translate text: line by line, sentence by sentence; empty lines and spacing between lines
    /// are kept. `progress(done, total)` counts sentences.
    pub fn text(&mut self, text: &str, cancelled: &dyn Fn() -> bool, progress: &dyn Fn(u64, u64)) -> PwResult<String> {
        let n = text.chars().count();
        if n > MAX_TEXT_CHARS {
            return Err(PwError::invalid(
                "translate.too_long",
                format!("The text is too long to translate in one go ({n} characters; at most {MAX_TEXT_CHARS})."),
            ));
        }
        let chars: Vec<char> = text.chars().collect();
        let spans = pw_core::text::sentences(text);
        let total = spans.len() as u64;
        let mut out = String::new();
        let mut at = 0usize;
        for (k, (s, e)) in spans.into_iter().enumerate() {
            if cancelled() {
                return Err(PwError::cancelled());
            }
            // What lies between sentences (spaces, line ends) is kept as it was.
            let gap: String = chars[at..s].iter().collect();
            out.push_str(&gap);
            let piece: String = chars[s..e].iter().collect();
            out.push_str(&if piece.chars().any(|c| c.is_alphabetic()) { self.masked(&piece)? } else { piece });
            at = e;
            progress(k as u64 + 1, total);
        }
        out.extend(chars[at..].iter());
        Ok(out)
    }
}

static ENGINES: OnceLock<Mutex<Option<(String, Translator)>>> = OnceLock::new();

/// Run `f` with the loaded model for `source` → `target` (one direction kept loaded at a time,
/// bounding memory to one model).
pub fn with_translator<T>(source: &str, target: &str, f: impl FnOnce(&mut Translator) -> PwResult<T>) -> PwResult<T> {
    let pair = format!("{source}-{target}");
    let cell = ENGINES.get_or_init(|| Mutex::new(None));
    let mut guard = cell.lock().unwrap_or_else(|p| p.into_inner());
    if guard.as_ref().map(|(p, _)| p != &pair).unwrap_or(true) {
        *guard = None;
        let dir =
            pair_dir(source, target).and_then(|d| d.parent().map(Path::to_path_buf)).ok_or_else(|| missing(&pair))?;
        *guard = Some((pair.clone(), Translator::load(&dir, source, target)?));
    }
    match guard.as_mut() {
        Some((_, t)) => f(t),
        None => Err(PwError::internal("translate.engine", "The translation engine is unavailable.")),
    }
}

/// The steps to translate `source` → `target`: directly, or through English when both halves are
/// pinned. Fails with a reason when the direction is not offered.
pub fn route(source: &str, target: &str) -> PwResult<Vec<(String, String)>> {
    if source == target {
        return Err(PwError::invalid("translate.same", "The document is already in that language."));
    }
    let pinned: Vec<(String, String)> = lock()?.pairs.into_iter().map(|p| (p.source, p.target)).collect();
    let has = |s: &str, t: &str| pinned.iter().any(|(a, b)| a == s && b == t);
    if has(source, target) {
        return Ok(vec![(source.into(), target.into())]);
    }
    if source != "en" && target != "en" && has(source, "en") && has("en", target) {
        return Ok(vec![(source.into(), "en".into()), ("en".into(), target.into())]);
    }
    Err(PwError::new(
        ErrorCategory::UnsupportedOperation,
        "translate.direction",
        format!("Translating from {} to {} is not available.", language_name(source), language_name(target)),
    ))
}

/// English name of a language code PageWeave knows.
pub fn language_name(code: &str) -> String {
    match code {
        "en" => "English",
        "de" => "German",
        "fr" => "French",
        "es" => "Spanish",
        "it" => "Italian",
        other => other,
    }
    .to_string()
}

/// Translate every text of `texts` from `source` to `target` (through English when needed; each
/// step translates all texts before the next model loads). `progress(done, total)` counts texts
/// over all steps.
pub fn translate_all(
    source: &str,
    target: &str,
    texts: &[String],
    cancelled: &dyn Fn() -> bool,
    progress: &dyn Fn(u64, u64),
) -> PwResult<Vec<String>> {
    let steps = route(source, target)?;
    let total = (texts.len() * steps.len()) as u64;
    let mut current: Vec<String> = texts.to_vec();
    for (k, (s, t)) in steps.iter().enumerate() {
        current = with_translator(s, t, |tr| {
            let mut out = Vec::with_capacity(current.len());
            for (i, text) in current.iter().enumerate() {
                out.push(tr.text(text, cancelled, &|_, _| {})?);
                progress((k * texts.len() + i + 1) as u64, total);
            }
            Ok(out)
        })?;
    }
    Ok(current)
}

/// Tokens a translation must keep unchanged: web and mail addresses, words with digits (numbers,
/// dates, amounts, codes, model numbers) and names in capitals (brands, acronyms).
pub fn protected_tokens(text: &str) -> Vec<String> {
    text.split_whitespace()
        .map(|w| {
            w.trim_matches(|c: char| {
                matches!(c, ',' | ';' | ':' | '(' | ')' | '"' | '\'' | '“' | '”' | '«' | '»' | '!' | '?')
            })
        })
        .map(|w| w.strip_suffix('.').unwrap_or(w))
        .filter(|w| {
            let url = w.starts_with("http://") || w.starts_with("https://") || w.starts_with("www.");
            let mail = w.contains('@') && w.contains('.');
            // Names written in capitals (brands, acronyms, codes) of three letters or more.
            let caps = w.chars().filter(|c| c.is_alphabetic()).count() >= 3 && w.chars().all(|c| !c.is_lowercase());
            url || mail || caps || w.chars().any(|c| c.is_ascii_digit())
        })
        .map(str::to_string)
        .collect()
}

/// Protected tokens of `source` missing from `translated`.
pub fn missing_tokens(source: &str, translated: &str) -> Vec<String> {
    protected_tokens(source).into_iter().filter(|t| !translated.contains(t.as_str())).collect()
}

/// Whether a word is masked before translation: web and mail addresses, domain names, codes
/// mixing letters and digits (RX-100, A4, ISO9001) and names with an inner capital (PageWeave) —
/// models tend to "translate" these.
fn maskable(w: &str) -> bool {
    let url = w.starts_with("http://") || w.starts_with("https://") || w.starts_with("www.");
    let mail = w.contains('@') && w.contains('.');
    let domain = {
        let parts: Vec<&str> = w.split('.').collect();
        parts.len() >= 2
            && parts.iter().all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'))
            && parts.last().is_some_and(|t| (2..=6).contains(&t.len()) && t.chars().all(|c| c.is_ascii_lowercase()))
    };
    let letters = w.chars().any(|c| c.is_alphabetic());
    let digits = w.chars().any(|c| c.is_ascii_digit());
    // Ordinals and units written together ("1st", "10km") read better translated.
    let ordinal = w.chars().take_while(|c| c.is_ascii_digit()).count() > 0
        && w.chars().skip_while(|c| c.is_ascii_digit()).all(|c| c.is_lowercase());
    // Names with a capital inside (PageWeave, iPhone, McDonald) are product or person names.
    let inner_capital = w.chars().skip(1).any(char::is_uppercase) && w.chars().any(char::is_lowercase);
    url || mail || domain || inner_capital || (letters && digits && !ordinal)
}

/// Replace maskable words with placeholders `PWX1`, `PWX2`, …; returns the text and the originals.
pub fn mask(text: &str) -> (String, Vec<String>) {
    let mut out = String::with_capacity(text.len());
    let mut originals: Vec<String> = Vec::new();
    // Names in capitals (brands, acronyms) are kept, unless the sentence is mostly capitals (a
    // heading set in capitals is translated).
    let words: Vec<&str> = text.split_whitespace().filter(|w| w.chars().any(char::is_alphabetic)).collect();
    let is_caps = |w: &str| w.chars().filter(|c| c.is_alphabetic()).count() >= 4 && !w.chars().any(char::is_lowercase);
    let caps_ok = words.iter().filter(|w| is_caps(w)).count() * 2 < words.len();
    for (i, part) in text.split(' ').enumerate() {
        if i > 0 {
            out.push(' ');
        }
        // Keep surrounding punctuation outside the placeholder.
        let core_start = part.find(|c: char| c.is_alphanumeric() || c == '+').unwrap_or(part.len());
        let core_end = part
            .rfind(|c: char| c.is_alphanumeric() || c == '/')
            .map(|e| e + part[e..].chars().next().map_or(1, char::len_utf8))
            .unwrap_or(core_start);
        let core = if core_start < core_end { &part[core_start..core_end] } else { "" };
        if !core.is_empty() && (maskable(core) || (caps_ok && is_caps(core))) {
            originals.push(core.to_string());
            out.push_str(&part[..core_start]);
            out.push_str(&format!("PWX{}", originals.len()));
            out.push_str(&part[core_end..]);
        } else {
            out.push_str(part);
        }
    }
    (out, originals)
}

/// Whether every placeholder PWX1..PWXn is in `text` (any case).
fn placeholders_kept(text: &str, n: usize) -> bool {
    let lower = text.to_lowercase();
    (1..=n).all(|i| {
        let ph = format!("pwx{i}");
        lower.match_indices(&ph).any(|(at, _)| !lower[at + ph.len()..].starts_with(|c: char| c.is_ascii_digit()))
    })
}

/// Put the originals back (placeholders matched without regard to case, highest number first so
/// PWX1 does not match inside PWX12).
pub fn unmask(text: &str, originals: &[String]) -> String {
    let mut s = text.to_string();
    for (i, o) in originals.iter().enumerate().rev() {
        let ph = format!("pwx{}", i + 1);
        let lower = s.to_lowercase();
        if let Some(at) = lower.find(&ph) {
            // Only when the lowercase form has the same byte length (ASCII placeholder).
            if lower.len() == s.len() {
                s.replace_range(at..at + ph.len(), o);
            }
        }
    }
    s
}

#[cfg(test)]
mod route_tests {
    use super::*;

    #[test]
    fn routes_protected_tokens_and_names() {
        assert_eq!(route("en", "de").expect("direct"), vec![("en".to_string(), "de".to_string())]);
        assert_eq!(
            route("de", "fr").expect("pivot"),
            vec![("de".to_string(), "en".to_string()), ("en".to_string(), "fr".to_string())]
        );
        assert_eq!(route("en", "en").expect_err("same").code, "translate.same");
        assert_eq!(route("en", "xx").expect_err("none").code, "translate.direction");
        assert_eq!(
            protected_tokens("Call +49 30 1234 or mail ada@example.org, see https://x.org/a. Model RX-100 costs 1,234.50 EUR on 2026-10-21 at NASA. I am OK."),
            vec!["+49", "30", "1234", "ada@example.org", "https://x.org/a", "RX-100", "1,234.50", "EUR", "2026-10-21", "NASA"]
        );
        assert_eq!(missing_tokens("Model RX-100 for 12 EUR", "Modell RX-100 für 12 Euro"), vec!["EUR"]);
        assert_eq!(missing_tokens("It costs 1,234.50", "Es kostet 1.234,50"), vec!["1,234.50"]);
        assert_eq!(language_name("fr"), "French");
        let (m, o) =
            mask("See rivers.example.org, mail ada@example.org (model RX-100) on the 1st of 10km roads in 2026.");
        assert_eq!(m, "See PWX1, mail PWX2 (model PWX3) on the 1st of 10km roads in 2026.");
        assert_eq!(o, vec!["rivers.example.org", "ada@example.org", "RX-100"]);
        assert_eq!(
            unmask("Siehe Pwx1, Mail PWX2 (Modell PWX3).", &o),
            "Siehe rivers.example.org, Mail ada@example.org (Modell RX-100)."
        );
        let many: Vec<String> = (1..=12).map(|i| format!("A{i}")).collect();
        assert_eq!(unmask("PWX12 PWX1", &many), "A12 A1");
        assert_eq!(mask("e.g. the U.S. and Dr. Who").1, Vec::<String>::new());
        assert_eq!(mask("Account summary PWTABLES for NASA and the EU").1, vec!["PWTABLES", "NASA"]);
        assert_eq!(mask("ANNUAL REVIEW OF RIVERS").1, Vec::<String>::new());
        assert!(placeholders_kept("Die pwx1 und PWX2.", 2));
        assert!(!placeholders_kept("Nur PWX12 hier", 1));
        assert!(!placeholders_kept("Seite 1", 1));
        assert_eq!(
            mask("PageWeave runs on an iPhone, says McDonald. Hello World.").1,
            vec!["PageWeave", "iPhone", "McDonald"]
        );
    }
}
