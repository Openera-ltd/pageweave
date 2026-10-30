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
use ort::value::Tensor;
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
/// Beam width: 4 hypotheses (the usual setting for these Marian models). Greedy decoding (1) turned
/// "The house is small." into "Das Haus ist sehr schön." with the English → German model; measured
/// gains in docs/benchmarks/TRANSLATION_BASELINE.md.
const BEAM: usize = 4;
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
    /// Folder of the model (directions sharing one multilingual model share it); `source-target`
    /// when absent.
    #[serde(default)]
    folder: Option<String>,
    /// Token put before the source text (the target language of a multilingual model).
    #[serde(default)]
    prefix: Option<String>,
    /// Where the files are published (`url/from`); Hugging Face's `repository/resolve/commit/from`
    /// when absent.
    #[serde(default)]
    url: Option<String>,
}

impl LockPair {
    fn folder(&self) -> String {
        self.folder.clone().unwrap_or_else(|| format!("{}-{}", self.source, self.target))
    }

    fn file_url(&self, f: &LockFile) -> String {
        match &self.url {
            Some(u) => format!("{}/{}", u.trim_end_matches('/'), f.from),
            None => format!("{}/resolve/{}/{}", self.repository, self.commit, f.from),
        }
    }
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
    let folder = p.folder();
    roots()
        .into_iter()
        .map(|r| r.join(&folder))
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
    let mut seen = std::collections::HashSet::new();
    for p in lock()?.pairs {
        // A model shared by several directions is checked once.
        let pair = p.folder();
        if !seen.insert(pair.clone()) {
            continue;
        }
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
/// Every direction served by the same model folder as `source` → `target` (itself included).
pub fn sharing(source: &str, target: &str) -> PwResult<Vec<(String, String)>> {
    let pairs = lock()?.pairs;
    let Some(p) = pairs.iter().find(|p| p.source == source && p.target == target) else {
        return Ok(Vec::new());
    };
    let folder = p.folder();
    Ok(pairs.iter().filter(|q| q.folder() == folder).map(|q| (q.source.clone(), q.target.clone())).collect())
}

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
    /// Token put before the source (the target language, for multilingual models).
    prefix: Option<u32>,
    /// Hypotheses kept while decoding (1 = greedy).
    beam: usize,
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
        let d = dir.join(lp.folder());
        for (name, f) in &lp.files {
            let p = d.join(name);
            let meta = std::fs::metadata(&p).map_err(|_| missing(&pair))?;
            if meta.len() != f.bytes || sha256_file(&p)? != f.sha256 {
                return Err(corrupt(&pair, name));
            }
        }
        let mut t = Self::from_files(&d, runtime_dir, &pair)?;
        if let Some(pre) = &lp.prefix {
            t.prefix = Some(t.tok.id(pre).ok_or_else(|| corrupt(&pair, TOKENIZER_FILE))?);
        }
        Ok(t)
    }

    /// Load the model files in `d` (one direction) without checking them against the pinned
    /// checksums: for evaluating candidate models only.
    pub fn from_files(d: &Path, runtime_dir: &Path, pair: &str) -> PwResult<Translator> {
        let pair = pair.to_string();
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
        // Models with several target languages start the source with the target's token.
        let prefix = match c["_pageweave_target_token"].as_str() {
            Some(t) => Some(tok.id(t).ok_or_else(|| corrupt(&pair, TOKENIZER_FILE))?),
            None => None,
        };
        Ok(Translator { encoder, decoder, tok, cfg, prefix, beam: BEAM })
    }

    /// Hypotheses kept while decoding (1 = greedy); for benchmarks.
    pub fn set_beam(&mut self, width: usize) {
        self.beam = width.clamp(1, 16);
    }

    /// Translate one sentence: beam search (`self.beam` hypotheses, 1 = greedy) with the decoder's
    /// key/value cache, finished hypotheses compared by log-probability per token.
    pub fn sentence(&mut self, text: &str) -> PwResult<String> {
        let mut ids: Vec<i64> = self.tok.encode(text).into_iter().map(i64::from).collect();
        if ids.is_empty() {
            return Ok(String::new());
        }
        ids.truncate(MAX_INPUT_TOKENS);
        if let Some(p) = self.prefix {
            ids.insert(0, p as i64);
        }
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

        let (l, h, hd, v) = (self.cfg.layers, self.cfg.heads, self.cfg.head_dim, self.cfg.vocab);
        let width = self.beam.max(1);
        let max_new = (2 * n + 10).min(512);
        // Active hypotheses: tokens so far and summed log-probability. Every row of the decoder's
        // batch is one hypothesis; its self-attention cache rows follow it when hypotheses are kept.
        let mut beams: Vec<(Vec<u32>, f32)> = vec![(Vec::new(), 0.0)];
        let mut finished: Vec<(Vec<u32>, f32)> = Vec::new();
        // Self-attention cache per layer (key, value) as [rows, heads, t, head_dim]; the
        // cross-attention cache is computed on the first step for one row and repeated per row.
        let mut own: Vec<Vec<f32>> = vec![Vec::new(); 2 * l];
        let mut own_t = 0usize;
        let mut cross: Vec<(Vec<usize>, Vec<f32>)> = Vec::new();
        let repeat = |data: &[f32], rows: usize| -> Vec<f32> { data.repeat(rows) };
        for step in 0..max_new {
            let rows = beams.len();
            let last: Vec<i64> =
                beams.iter().map(|(t, _)| t.last().map(|&x| x as i64).unwrap_or(self.cfg.start as i64)).collect();
            let mut inputs: Vec<(String, ort::session::SessionInputValue)> = vec![
                ("input_ids".into(), Tensor::from_array(([rows, 1usize], last)).map_err(engine_err)?.into()),
                (
                    "encoder_attention_mask".into(),
                    Tensor::from_array(([rows, n], mask.repeat(rows))).map_err(engine_err)?.into(),
                ),
                (
                    "encoder_hidden_states".into(),
                    Tensor::from_array(([rows, hidden_shape[1], hidden_shape[2]], repeat(&hidden, rows)))
                        .map_err(engine_err)?
                        .into(),
                ),
                ("use_cache_branch".into(), Tensor::from_array(([1usize], vec![step > 0])).map_err(engine_err)?.into()),
            ];
            for (k, data) in own.iter().enumerate() {
                let t = Tensor::from_array(([rows, h, own_t, hd], data.clone())).map_err(engine_err)?;
                inputs.push((format!("past_key_values.{}.decoder.{}", k / 2, ["key", "value"][k % 2]), t.into()));
            }
            for (k, c) in cross.iter().enumerate() {
                let value: ort::session::SessionInputValue = if step == 0 {
                    Tensor::from_array(([rows, h, 0usize, hd], Vec::<f32>::new())).map_err(engine_err)?.into()
                } else {
                    Tensor::from_array(([rows, c.0[1], c.0[2], c.0[3]], repeat(&c.1, rows))).map_err(engine_err)?.into()
                };
                inputs.push((format!("past_key_values.{}.encoder.{}", k / 2, ["key", "value"][k % 2]), value));
            }
            if step == 0 {
                for k in 0..2 * l {
                    let t = Tensor::from_array(([rows, h, 0usize, hd], Vec::<f32>::new())).map_err(engine_err)?;
                    inputs.push((format!("past_key_values.{}.encoder.{}", k / 2, ["key", "value"][k % 2]), t.into()));
                }
            }
            let outputs = self.decoder.run(inputs).map_err(|e| engine_err(format!("step {step}: {e}")))?;
            let (_, logits) = outputs["logits"].try_extract_tensor::<f32>().map_err(engine_err)?;
            if logits.len() < rows * v {
                return Err(engine_err("unexpected output size"));
            }
            let per_row = logits.len() / rows;
            // Candidates: each hypothesis extended by its `width` best tokens (log-softmax).
            let mut cand: Vec<(usize, u32, f32)> = Vec::with_capacity(rows * width);
            for (r, (_, score)) in beams.iter().enumerate() {
                let row = &logits[r * per_row + per_row - v..(r + 1) * per_row];
                let max = row.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
                let lse = max + row.iter().map(|&x| (x - max).exp()).sum::<f32>().ln();
                let mut top: Vec<(u32, f32)> = Vec::with_capacity(width + 1);
                for (i, &x) in row.iter().enumerate() {
                    if i as u32 == self.cfg.pad {
                        continue;
                    }
                    if top.len() < width || x > top[top.len() - 1].1 {
                        let at = top.iter().position(|&(_, y)| x > y).unwrap_or(top.len());
                        top.insert(at, (i as u32, x));
                        top.truncate(width);
                    }
                }
                cand.extend(top.into_iter().map(|(tok, x)| (r, tok, score + x - lse)));
            }
            cand.sort_by(|a, b| b.2.total_cmp(&a.2));
            let mut keep: Vec<(usize, Vec<u32>, f32)> = Vec::with_capacity(width);
            for (r, tok, score) in cand {
                if keep.len() == width {
                    break;
                }
                if tok == self.cfg.eos {
                    let len = beams[r].0.len().max(1) as f32;
                    finished.push((beams[r].0.clone(), score / len));
                } else {
                    let mut t = beams[r].0.clone();
                    t.push(tok);
                    keep.push((r, t, score));
                }
            }
            // The step's caches: own grows by one position; cross is kept from the first step.
            let mut present: Vec<(Vec<usize>, Vec<f32>)> = Vec::with_capacity(2 * l);
            for k in 0..2 * l {
                let name = format!("present.{}.decoder.{}", k / 2, ["key", "value"][k % 2]);
                let (sh, data) = outputs[name.as_str()].try_extract_tensor::<f32>().map_err(engine_err)?;
                present.push((sh.iter().map(|&d| d as usize).collect(), data.to_vec()));
            }
            if step == 0 {
                for k in 0..2 * l {
                    let name = format!("present.{}.encoder.{}", k / 2, ["key", "value"][k % 2]);
                    let (sh, data) = outputs[name.as_str()].try_extract_tensor::<f32>().map_err(engine_err)?;
                    let sh: Vec<usize> = sh.iter().map(|&d| d as usize).collect();
                    let one = data.len() / sh[0].max(1);
                    cross.push((sh, data[..one].to_vec()));
                }
            }
            drop(outputs);
            // Stop when nothing is left to extend, or when `width` hypotheses have finished and no
            // live one can still beat them (scores only fall; per-token scores are compared). Stopping
            // at the first `width` finished ones would prefer early ends (dropped final full stops).
            if keep.is_empty() {
                break;
            }
            if finished.len() >= width {
                finished.sort_by(|a, b| b.1.total_cmp(&a.1));
                finished.truncate(width);
                let worst = finished[width - 1].1;
                let best_live = keep.iter().map(|(_, t, sc)| sc / t.len() as f32).fold(f32::NEG_INFINITY, f32::max);
                if best_live <= worst {
                    break;
                }
            }
            own_t = present.first().map(|p| p.0[2]).unwrap_or(own_t + 1);
            let row_len = h * own_t * hd;
            own = present
                .iter()
                .map(|(_, data)| {
                    keep.iter().flat_map(|(r, _, _)| data[r * row_len..(r + 1) * row_len].iter().copied()).collect()
                })
                .collect();
            beams = keep.into_iter().map(|(_, t, s)| (t, s)).collect();
        }
        // Unfinished hypotheses compete too (length limit reached).
        finished.extend(beams.into_iter().map(|(t, s)| {
            let len = t.len().max(1) as f32;
            (t, s / len)
        }));
        let best = finished.into_iter().max_by(|a, b| a.1.total_cmp(&b.1)).map(|(t, _)| t).unwrap_or_default();
        Ok(self.tok.decode(&best))
    }

    /// Translate a sentence with addresses and codes masked: they are replaced by placeholders the
    /// models copy unchanged (`PWX1`, …), and put back afterwards.
    pub fn masked(&mut self, sentence: &str) -> PwResult<String> {
        let first = self.masked_with(sentence, false)?;
        // A number changed (2025 → 2020): again with every number masked.
        let first = if lost_numbers(sentence, &first).is_empty() { first } else { self.masked_with(sentence, true)? };
        if degenerate(sentence, &first) {
            // A runaway translation ("Bonus" → "Hey hey hey …"): short texts are kept as they
            // are, longer ones are translated sentence piece by piece.
            if sentence.split_whitespace().count() <= 3 {
                return Ok(sentence.to_string());
            }
            let mut out = String::new();
            for part in sentence.split_inclusive([',', ';', ':']) {
                let t = self.piece(part)?;
                out.push_str(if degenerate(part, &t) { part } else { &t });
            }
            return Ok(out);
        }
        Ok(first)
    }

    fn masked_with(&mut self, sentence: &str, all_numbers: bool) -> PwResult<String> {
        let (masked, originals) = mask_with(sentence, all_numbers);
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
        // Two directions of one multilingual model are loaded separately (they differ in the
        // target token).
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
        "pt" => "Portuguese",
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
    let caps_ok = !mostly_caps(text);
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
            url || mail || (caps && caps_ok) || w.chars().any(|c| c.is_ascii_digit())
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

/// Whether a text is mostly words in capitals (a heading set in capitals: its words are
/// translated, not kept as names).
fn mostly_caps(text: &str) -> bool {
    let words: Vec<&str> = text.split_whitespace().filter(|w| w.chars().any(char::is_alphabetic)).collect();
    let is_caps = |w: &&str| w.chars().filter(|c| c.is_alphabetic()).count() >= 3 && !w.chars().any(char::is_lowercase);
    !words.is_empty() && words.iter().filter(|w| is_caps(w)).count() * 2 >= words.len()
}

/// The numbers of a text as digit strings (separators dropped: "2 025,5" and "2,025.5" are the
/// same), in order.
pub fn numbers(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let chars: Vec<char> = text.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        if c.is_ascii_digit() {
            cur.push(c);
        } else if matches!(c, '.' | ',' | ' ' | '\u{a0}' | '\u{202f}' | '\'')
            && !cur.is_empty()
            && chars.get(i + 1).is_some_and(|n| n.is_ascii_digit())
            && !(c == ' '
                && chars.get(i + 1..i + 4).is_some_and(|g| g.iter().filter(|x| x.is_ascii_digit()).count() < 3))
        {
            continue;
        } else if !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// Numbers of `source` that `translated` does not have (each counted as often as it appears).
pub fn lost_numbers(source: &str, translated: &str) -> Vec<String> {
    let mut have = numbers(translated);
    let mut lost = Vec::new();
    for n in numbers(source) {
        match have.iter().position(|h| *h == n) {
            Some(i) => {
                have.remove(i);
            }
            None => lost.push(n),
        }
    }
    lost
}

/// A translation that ran away: a word or a few words repeated three times or more in a row
/// (when the source does not do so), or far longer than its source.
pub fn degenerate(source: &str, translated: &str) -> bool {
    let words = |t: &str| {
        t.split_whitespace()
            .map(|w| w.trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase())
            .filter(|w| !w.is_empty())
            .collect::<Vec<_>>()
    };
    let (s, t) = (words(source), words(translated));
    let repeats = |w: &[String]| {
        (1..=4).any(|n| {
            w.len() >= 3 * n
                && (0..=w.len() - 3 * n).any(|i| (1..3).all(|k| w[i..i + n] == w[i + k * n..i + (k + 1) * n]))
        })
    };
    (repeats(&t) && !repeats(&s)) || t.len() > 3 * s.len() + 6
}

/// Replace maskable words with placeholders `PWX1`, `PWX2`, …; returns the text and the originals.
pub fn mask(text: &str) -> (String, Vec<String>) {
    mask_with(text, false)
}

/// As [`mask`]; with `all_numbers`, every number is masked too (after a translation changed one).
pub fn mask_with(text: &str, all_numbers: bool) -> (String, Vec<String>) {
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
        let number = all_numbers && core.chars().any(|c| c.is_ascii_digit());
        if !core.is_empty() && (maskable(core) || number || (caps_ok && is_caps(core))) {
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
        // Every pair of the six languages is pinned directly (D-027): no route through English.
        assert_eq!(route("de", "fr").expect("direct"), vec![("de".to_string(), "fr".to_string())]);
        assert_eq!(route("pt", "it").expect("direct"), vec![("pt".to_string(), "it".to_string())]);
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

#[cfg(test)]
mod guard_tests {
    use super::*;

    #[test]
    fn numbers_compare_values_not_separators() {
        assert_eq!(numbers("Alternance 2025-2026, 95,24 euros, 1 000 000 €"), vec!["2025", "2026", "9524", "1000000"]);
        assert_eq!(lost_numbers("Alternance 2025-2026", "Alternatives 2020-2026"), vec!["2025"]);
        assert!(lost_numbers("95,24 euros (100 / 1,05)", "95.24 euros (100/1.05)").is_empty());
        assert!(lost_numbers("Ex. 2, 4, 5", "Ex. 2, 4, 5").is_empty());
    }

    #[test]
    fn runaway_translations_are_caught() {
        assert!(degenerate("Bonus", "Hey hey hey hey hey hey"));
        assert!(degenerate("Le prix", "The price the price the price the price"));
        assert!(!degenerate("Chapitre 1 : Monnaie et prix", "Chapter 1: Currency and prices"));
        assert!(!degenerate("très très très bien", "very very very good"));
    }

    #[test]
    fn capital_headings_are_words_not_codes() {
        assert!(protected_tokens("ÉCONOMIE MONÉTAIRE ET BANCAIRE").is_empty());
        assert_eq!(protected_tokens("La BCE et le FMI"), vec!["BCE", "FMI"]);
        let (m, o) = mask_with("Alternance 2025-2026 et 95,24 euros", true);
        assert_eq!(o, vec!["2025-2026", "95,24"]);
        assert_eq!(m, "Alternance PWX1 et PWX2 euros");
    }
}
