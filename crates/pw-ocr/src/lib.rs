//! PageWeave OCR (decision D-009): PP-OCRv6 small text detection + recognition on ONNX Runtime.
//!
//! The pipeline reproduces the configuration measured best in `docs/benchmarks/OCR_BASELINE.md`
//! (RapidOCR's PP-OCRv6 settings): images in BGR order; long side capped at 2000 px; detector input
//! with the short side at least 736 px, sides rounded to multiples of 32, normalised to [-1, 1];
//! DB post-processing (threshold 0.3, 2×2 dilation, box score ≥ 0.5, unclip ratio 1.6, minimum
//! side 3); top-to-bottom / left-to-right ordering; perspective crops (turned when tall); recogniser
//! input height 48 in batches of 6 sorted by aspect ratio; greedy CTC decoding; lines scoring
//! below 0.5 or decoding to whitespace are dropped. Agreement with the reference is measured by
//! the parity benchmark (`ml/`), not assumed.
//!
//! ONNX Runtime is loaded at runtime from the pinned, verified library (resources/ocr.lock.json);
//! telemetry is disabled. Model files are verified by SHA-256 before first use.

pub mod geom;

use geom::{dist, in_convex, min_area_rect, order_clockwise, resize, rot90, round_half_even, warp_quad, Bgr, Pt};
use ort::session::Session;
use ort::value::Tensor;
use pw_core::{ErrorCategory, PwError, PwResult};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

const MAX_SIDE: usize = 2000;
const MIN_SIDE: usize = 30;
const DET_LIMIT_MIN_SIDE: usize = 736;
const DB_THRESH: f32 = 0.3;
const BOX_THRESH: f32 = 0.5;
const UNCLIP_RATIO: f32 = 1.6;
const MIN_BOX: f32 = 3.0;
const MAX_CANDIDATES: usize = 1000;
const SORT_Y_TOLERANCE: f32 = 10.0;
const REC_H: usize = 48;
const REC_MIN_W: usize = 320;
const REC_BATCH: usize = 6;
const TEXT_SCORE: f32 = 0.5;
/// Refuse absurd inputs before allocating (pixels).
const MAX_INPUT_PIXELS: usize = 120_000_000;

/// One recognised text line in the input image's pixel coordinates.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OcrLine {
    pub text: String,
    /// Mean per-character confidence (0–1).
    pub score: f32,
    /// Top-left, top-right, bottom-right, bottom-left.
    pub quad: [[f32; 2]; 4],
}

/// File names inside a resource directory (as installed by tools/fetch_ocr.py).
pub const DET_FILE: &str = "det.onnx";
pub const REC_FILE: &str = "rec.onnx";
pub const DICT_FILE: &str = "rec_dict.txt";
#[cfg(windows)]
pub const RUNTIME_FILE: &str = "onnxruntime.dll";
#[cfg(not(windows))]
pub const RUNTIME_FILE: &str = "libonnxruntime.so";

const LOCK: &str = include_str!("../../../resources/ocr.lock.json");

fn missing(what: &str) -> PwError {
    PwError::new(
        ErrorCategory::MissingResource,
        "ocr.resource_missing",
        format!(
            "Text recognition is not installed ({what} is missing). Reinstall PageWeave or run the resource setup."
        ),
    )
}

fn corrupt(what: &str) -> PwError {
    PwError::new(
        ErrorCategory::CorruptResource,
        "ocr.resource_corrupt",
        format!("A text recognition file is damaged ({what}). Reinstall PageWeave or run the resource setup again."),
    )
}

fn engine_err(e: impl std::fmt::Display) -> PwError {
    PwError::new(ErrorCategory::EngineFailure, "ocr.engine", format!("Text recognition failed: {e}"))
}

/// Pinned SHA-256 of each model file and the dictionary, from the committed lock.
fn pinned() -> PwResult<[(String, &'static str); 3]> {
    #[derive(serde::Deserialize)]
    struct File {
        sha256: String,
    }
    #[derive(serde::Deserialize)]
    struct Models {
        files: std::collections::HashMap<String, File>,
        dictionary: File,
    }
    #[derive(serde::Deserialize)]
    struct Lock {
        models: Models,
    }
    let lock: Lock = serde_json::from_str(LOCK)
        .map_err(|_| PwError::internal("ocr.lock", "The OCR resource lock is unreadable."))?;
    let get = |n: &str| {
        lock.models
            .files
            .get(n)
            .map(|f| f.sha256.clone())
            .ok_or_else(|| PwError::internal("ocr.lock", "The OCR resource lock is incomplete."))
    };
    Ok([(get(DET_FILE)?, DET_FILE), (get(REC_FILE)?, REC_FILE), (lock.models.dictionary.sha256, DICT_FILE)])
}

fn sha256_file(p: &Path) -> PwResult<String> {
    use sha2::{Digest, Sha256};
    let bytes = std::fs::read(p).map_err(|e| PwError::from_io(&e, "OCR resource"))?;
    Ok(Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect())
}

/// Locate the OCR resource directory: `PAGEWEAVE_OCR_DIR`, next to the executable (`ocr/`), or the
/// development install (third_party/ocr/<platform>).
pub fn default_resource_dir() -> Option<PathBuf> {
    if let Some(d) = std::env::var_os("PAGEWEAVE_OCR_DIR") {
        return Some(PathBuf::from(d));
    }
    let mut candidates = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join("ocr"));
            candidates.push(dir.join("resources").join("ocr"));
        }
    }
    let plat = if cfg!(windows) { "win-x64" } else { "linux-x64" };
    candidates.push(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../third_party/ocr").join(plat));
    candidates.into_iter().find(|d| d.join(DET_FILE).exists())
}

/// ONNX Runtime allows one environment per process; it is created once from the first resource
/// directory used.
static RUNTIME: OnceLock<Result<ort::environment::Environment, String>> = OnceLock::new();

fn runtime(dir: &Path) -> PwResult<&'static ort::environment::Environment> {
    let r = RUNTIME.get_or_init(|| {
        let lib = dir.join(RUNTIME_FILE);
        if !lib.exists() {
            return Err(format!("{RUNTIME_FILE} not found"));
        }
        ort::init_from(&lib)
            .map_err(|e| format!("could not load {RUNTIME_FILE}: {e}"))?
            .with_name("pageweave-ocr")
            .with_telemetry(false)
            .build()
            .map_err(|e| format!("could not start the runtime: {e}"))
    });
    match r {
        Ok(env) => Ok(env),
        Err(e) if e.contains("not found") => Err(missing(RUNTIME_FILE)),
        Err(e) => Err(engine_err(e)),
    }
}

/// A loaded detector + recogniser. Inference needs `&mut` sessions, so use [`with_engine`] for the
/// process-wide instance.
pub struct OcrEngine {
    det: Session,
    rec: Session,
    /// Index 0 is the CTC blank; then the dictionary; the last entry is the space character.
    chars: Vec<String>,
}

impl OcrEngine {
    /// Verify and load the resources in `dir`.
    pub fn load(dir: &Path) -> PwResult<OcrEngine> {
        for (sha, name) in pinned()? {
            let p = dir.join(name);
            if !p.exists() {
                return Err(missing(name));
            }
            if sha256_file(&p)? != sha {
                return Err(corrupt(name));
            }
        }
        let env = runtime(dir)?;
        let session = |name: &str| -> PwResult<Session> {
            Session::builder(env).map_err(engine_err)?.commit_from_file(dir.join(name)).map_err(|e| {
                PwError::new(
                    ErrorCategory::CorruptResource,
                    "ocr.model_load",
                    format!("The text recognition model could not be loaded: {e}"),
                )
            })
        };
        let det = session(DET_FILE)?;
        let rec = session(REC_FILE)?;
        let dict = std::fs::read_to_string(dir.join(DICT_FILE)).map_err(|e| PwError::from_io(&e, "OCR dictionary"))?;
        let mut chars = vec![String::new()];
        chars.extend(dict.split('\n').filter(|l| !l.is_empty()).map(str::to_string));
        chars.push(" ".into());
        Ok(OcrEngine { det, rec, chars })
    }

    /// Recognise all text lines in an image (RGB or RGBA pixels, row-major).
    pub fn recognize(
        &mut self,
        w: usize,
        h: usize,
        px: &[u8],
        channels: usize,
        cancelled: &dyn Fn() -> bool,
    ) -> PwResult<Vec<OcrLine>> {
        if w == 0
            || h == 0
            || w.saturating_mul(h) > MAX_INPUT_PIXELS
            || px.len() < w * h * channels
            || !(3..=4).contains(&channels)
        {
            return Err(PwError::invalid("ocr.image", "The page image is empty or too large for text recognition."));
        }
        let original = Bgr::from_rgb(w, h, px, channels);
        // Long side ≤ 2000, short side ≥ 30, both rounded to multiples of 32.
        let (img, rx, ry) = prepare(&original);
        let quads = self.detect(&img)?;
        if cancelled() {
            return Err(PwError::new(ErrorCategory::Cancelled, "job.cancelled", "Text recognition was cancelled."));
        }
        let crops: Vec<Bgr> = quads.iter().map(|q| crop(&img, q)).collect();
        let texts = self.recognize_crops(&crops, cancelled)?;
        let mut out = Vec::new();
        for (q, (text, score)) in quads.iter().zip(texts) {
            if text.trim().is_empty() || score < TEXT_SCORE {
                continue;
            }
            let quad = q.map(|(x, y)| [x * rx, y * ry]);
            out.push(OcrLine { text, score, quad });
        }
        Ok(out)
    }

    fn detect(&mut self, img: &Bgr) -> PwResult<Vec<[Pt; 4]>> {
        let (mut dw, mut dh) = (img.w as f64, img.h as f64);
        let min = dw.min(dh);
        if min < DET_LIMIT_MIN_SIDE as f64 {
            let r = DET_LIMIT_MIN_SIDE as f64 / min;
            dw = (dw * r).trunc();
            dh = (dh * r).trunc();
        }
        let dw = (round_half_even(dw / 32.0) * 32.0).max(32.0) as usize;
        let dh = (round_half_even(dh / 32.0) * 32.0).max(32.0) as usize;
        let resized = resize(img, dw, dh);
        let mut input = vec![0f32; 3 * dw * dh];
        for c in 0..3 {
            for i in 0..dw * dh {
                input[c * dw * dh + i] = (resized.data[i * 3 + c] as f32 / 255.0 - 0.5) / 0.5;
            }
        }
        let tensor = Tensor::from_array(([1usize, 3, dh, dw], input)).map_err(engine_err)?;
        let outputs = self.det.run(ort::inputs![tensor]).map_err(engine_err)?;
        let (shape, prob) = outputs[0].try_extract_tensor::<f32>().map_err(engine_err)?;
        let (ph, pw) = (shape[shape.len() - 2] as usize, shape[shape.len() - 1] as usize);
        if ph * pw != prob.len() {
            return Err(engine_err("unexpected detector output shape"));
        }
        Ok(db_boxes(prob, pw, ph, img.w, img.h))
    }

    fn recognize_crops(&mut self, crops: &[Bgr], cancelled: &dyn Fn() -> bool) -> PwResult<Vec<(String, f32)>> {
        let mut order: Vec<usize> = (0..crops.len()).collect();
        let ratio = |c: &Bgr| c.w as f32 / c.h.max(1) as f32;
        order.sort_by(|&a, &b| ratio(&crops[a]).total_cmp(&ratio(&crops[b])));
        let mut results = vec![(String::new(), 0.0f32); crops.len()];
        for batch in order.chunks(REC_BATCH) {
            if cancelled() {
                return Err(PwError::new(ErrorCategory::Cancelled, "job.cancelled", "Text recognition was cancelled."));
            }
            let max_ratio = batch.iter().map(|&i| ratio(&crops[i])).fold(REC_MIN_W as f32 / REC_H as f32, f32::max);
            let width = (REC_H as f32 * max_ratio) as usize;
            let n = batch.len();
            let mut input = vec![0f32; n * 3 * REC_H * width];
            for (b, &i) in batch.iter().enumerate() {
                let c = &crops[i];
                let rw = ((REC_H as f32 * ratio(c)).ceil() as usize).clamp(1, width);
                let r = resize(c, rw, REC_H);
                for ch in 0..3 {
                    for y in 0..REC_H {
                        for x in 0..rw {
                            input[((b * 3 + ch) * REC_H + y) * width + x] =
                                (r.data[(y * rw + x) * 3 + ch] as f32 / 255.0 - 0.5) / 0.5;
                        }
                    }
                }
            }
            let tensor = Tensor::from_array(([n, 3, REC_H, width], input)).map_err(engine_err)?;
            let outputs = self.rec.run(ort::inputs![tensor]).map_err(engine_err)?;
            let (shape, logits) = outputs[0].try_extract_tensor::<f32>().map_err(engine_err)?;
            let (steps, classes) = (shape[1] as usize, shape[2] as usize);
            if classes != self.chars.len() {
                return Err(corrupt("dictionary does not match the recognition model"));
            }
            for (b, &i) in batch.iter().enumerate() {
                results[i] =
                    ctc_decode(&logits[b * steps * classes..(b + 1) * steps * classes], steps, classes, &self.chars);
            }
        }
        Ok(results)
    }
}

/// Greedy CTC: best class per step, collapse repeats, drop blanks; score = mean best probability.
pub fn ctc_decode(logits: &[f32], steps: usize, classes: usize, chars: &[String]) -> (String, f32) {
    let mut text = String::new();
    let mut probs = Vec::new();
    let mut prev = usize::MAX;
    for t in 0..steps {
        let row = &logits[t * classes..(t + 1) * classes];
        let (idx, p) = row.iter().enumerate().fold((0, f32::MIN), |acc, (i, &v)| if v > acc.1 { (i, v) } else { acc });
        if idx != prev && idx != 0 {
            if let Some(c) = chars.get(idx) {
                text.push_str(c);
                probs.push(p);
            }
        }
        prev = idx;
    }
    let score = if probs.is_empty() { 0.0 } else { probs.iter().sum::<f32>() / probs.len() as f32 };
    (text, score)
}

/// Resize so the long side is at most 2000 and the short side at least 30 (each rounded to a
/// multiple of 32), returning the factors that map back to the original.
fn prepare(img: &Bgr) -> (Bgr, f32, f32) {
    let (w, h) = (img.w as f64, img.h as f64);
    let scaled = |k: f64| -> (usize, usize) {
        let (rw, rh) = ((w * k).trunc(), (h * k).trunc());
        ((round_half_even(rw / 32.0) * 32.0).max(32.0) as usize, (round_half_even(rh / 32.0) * 32.0).max(32.0) as usize)
    };
    let (nw, nh) = if w.max(h) > MAX_SIDE as f64 {
        scaled(MAX_SIDE as f64 / w.max(h))
    } else if w.min(h) < MIN_SIDE as f64 {
        scaled(MIN_SIDE as f64 / w.min(h))
    } else {
        (img.w, img.h)
    };
    if (nw, nh) == (img.w, img.h) {
        return (img.clone(), 1.0, 1.0);
    }
    (resize(img, nw, nh), img.w as f32 / nw as f32, img.h as f32 / nh as f32)
}

/// DB post-processing: probability map (pw×ph) → text quadrilaterals in image (dest) coordinates,
/// ordered top-to-bottom, left-to-right.
pub fn db_boxes(prob: &[f32], pw: usize, ph: usize, dest_w: usize, dest_h: usize) -> Vec<[Pt; 4]> {
    let bin: Vec<bool> = prob.iter().map(|&p| p > DB_THRESH).collect();
    // 2×2 dilation (OpenCV anchor at the kernel centre): a pixel is set if it or its left, upper or
    // upper-left neighbour is.
    let mut mask = vec![false; pw * ph];
    for y in 0..ph {
        for x in 0..pw {
            let at = |dx: usize, dy: usize| x >= dx && y >= dy && bin[(y - dy) * pw + (x - dx)];
            mask[y * pw + x] = at(0, 0) || at(1, 0) || at(0, 1) || at(1, 1);
        }
    }
    // 8-connected components; their boundary pixels stand in for the traced contour.
    let mut label = vec![0u32; pw * ph];
    let mut next = 0u32;
    let mut boxes: Vec<([Pt; 4], f32)> = Vec::new();
    let mut stack = Vec::new();
    for start in 0..pw * ph {
        if !mask[start] || label[start] != 0 {
            continue;
        }
        next += 1;
        if next as usize > MAX_CANDIDATES {
            break;
        }
        let mut boundary: Vec<Pt> = Vec::new();
        label[start] = next;
        stack.push(start);
        while let Some(i) = stack.pop() {
            let (x, y) = ((i % pw) as isize, (i / pw) as isize);
            let mut edge = false;
            for (dx, dy) in [(-1, -1), (0, -1), (1, -1), (-1, 0), (1, 0), (-1, 1), (0, 1), (1, 1)] {
                let (nx, ny) = (x + dx, y + dy);
                if nx < 0 || ny < 0 || nx >= pw as isize || ny >= ph as isize {
                    edge = true;
                    continue;
                }
                let j = ny as usize * pw + nx as usize;
                if !mask[j] {
                    edge = true;
                } else if label[j] == 0 {
                    label[j] = next;
                    stack.push(j);
                }
            }
            if edge {
                boundary.push((x as f32, y as f32));
            }
        }
        let Some(rect) = min_area_rect(&boundary) else { continue };
        if rect.short_side() < MIN_BOX {
            continue;
        }
        let quad = rect.corners();
        let score = box_score(prob, pw, ph, &quad);
        if score < BOX_THRESH {
            continue;
        }
        // Unclip: offset by area × ratio / perimeter; the min-area rectangle of the rounded offset
        // of a rectangle is the rectangle grown by that distance on every side.
        let d = rect.w * rect.h * UNCLIP_RATIO / (2.0 * (rect.w + rect.h)).max(1e-6);
        let grown = rect.grow(d);
        if grown.short_side() < MIN_BOX + 2.0 {
            continue;
        }
        let sx = dest_w as f32 / pw as f32;
        let sy = dest_h as f32 / ph as f32;
        let q = grown
            .corners()
            .map(|(x, y)| ((x * sx).round().clamp(0.0, dest_w as f32), (y * sy).round().clamp(0.0, dest_h as f32)));
        let q = order_clockwise(q)
            .map(|(x, y)| (x.clamp(0.0, (dest_w - 1) as f32).trunc(), y.clamp(0.0, (dest_h - 1) as f32).trunc()));
        if dist(q[0], q[1]).trunc() <= 3.0 || dist(q[0], q[3]).trunc() <= 3.0 {
            continue;
        }
        boxes.push((q, score));
    }
    sort_boxes(boxes.into_iter().map(|(q, _)| q).collect())
}

/// Mean probability inside the quadrilateral (over its bounding rectangle's pixels).
fn box_score(prob: &[f32], pw: usize, ph: usize, q: &[Pt; 4]) -> f32 {
    let qi = q.map(|(x, y)| (x.trunc(), y.trunc()));
    let xmin = (q.iter().map(|p| p.0).fold(f32::MAX, f32::min).floor().max(0.0) as usize).min(pw - 1);
    let xmax = (q.iter().map(|p| p.0).fold(f32::MIN, f32::max).ceil().max(0.0) as usize).min(pw - 1);
    let ymin = (q.iter().map(|p| p.1).fold(f32::MAX, f32::min).floor().max(0.0) as usize).min(ph - 1);
    let ymax = (q.iter().map(|p| p.1).fold(f32::MIN, f32::max).ceil().max(0.0) as usize).min(ph - 1);
    let (mut sum, mut n) = (0f64, 0u32);
    for y in ymin..=ymax {
        for x in xmin..=xmax {
            if in_convex(&qi, (x as f32, y as f32)) {
                sum += prob[y * pw + x] as f64;
                n += 1;
            }
        }
    }
    if n == 0 {
        0.0
    } else {
        (sum / n as f64) as f32
    }
}

/// Stable sort by top-left y, group into lines where consecutive y differ by < 10 px, then by x.
fn sort_boxes(mut boxes: Vec<[Pt; 4]>) -> Vec<[Pt; 4]> {
    boxes.sort_by(|a, b| a[0].1.total_cmp(&b[0].1));
    let mut line = 0usize;
    let mut keyed: Vec<(usize, f32, [Pt; 4])> = Vec::with_capacity(boxes.len());
    for (i, b) in boxes.iter().enumerate() {
        if i > 0 && b[0].1 - boxes[i - 1][0].1 >= SORT_Y_TOLERANCE {
            line += 1;
        }
        keyed.push((line, b[0].0, *b));
    }
    keyed.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)));
    keyed.into_iter().map(|k| k.2).collect()
}

/// Upright crop of a detected quadrilateral; tall crops are turned to read horizontally.
fn crop(img: &Bgr, q: &[Pt; 4]) -> Bgr {
    let w = dist(q[0], q[1]).max(dist(q[2], q[3])) as usize;
    let h = dist(q[0], q[3]).max(dist(q[1], q[2])) as usize;
    let c = warp_quad(img, q, w.max(1), h.max(1));
    if c.h as f32 / c.w.max(1) as f32 >= 1.5 {
        rot90(&c)
    } else {
        c
    }
}

static ENGINE: OnceLock<Mutex<Option<OcrEngine>>> = OnceLock::new();

/// Run `f` with the process-wide engine, loading it from `dir` on first use.
pub fn with_engine<T>(dir: &Path, f: impl FnOnce(&mut OcrEngine) -> PwResult<T>) -> PwResult<T> {
    let cell = ENGINE.get_or_init(|| Mutex::new(None));
    let mut guard = cell.lock().unwrap_or_else(|p| p.into_inner());
    if guard.is_none() {
        *guard = Some(OcrEngine::load(dir)?);
    }
    match guard.as_mut() {
        Some(engine) => f(engine),
        None => Err(PwError::internal("ocr.engine", "The text recognition engine is unavailable.")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ctc_collapses_repeats_and_drops_blanks() {
        let chars: Vec<String> = ["", "a", "b", " "].iter().map(|s| s.to_string()).collect();
        // steps: a a blank a b b space
        let seq = [1, 1, 0, 1, 2, 2, 3];
        let mut logits = vec![0f32; seq.len() * 4];
        for (t, &c) in seq.iter().enumerate() {
            logits[t * 4 + c] = 0.9;
        }
        let (text, score) = ctc_decode(&logits, seq.len(), 4, &chars);
        assert_eq!(text, "aab ");
        assert!((score - 0.9).abs() < 1e-6);
        assert_eq!(ctc_decode(&[0.0; 8], 2, 4, &chars).1, 0.0);
    }

    #[test]
    fn db_finds_two_separate_lines_in_reading_order() {
        let (pw, ph) = (200, 60);
        let mut prob = vec![0f32; pw * ph];
        for (y0, x0, x1) in [(10, 20, 180), (40, 30, 120)] {
            for y in y0..y0 + 8 {
                for x in x0..x1 {
                    prob[y * pw + x] = 0.9;
                }
            }
        }
        let boxes = db_boxes(&prob, pw, ph, pw, ph);
        assert_eq!(boxes.len(), 2, "{boxes:?}");
        assert!(boxes[0][0].1 < boxes[1][0].1, "top line first");
        // Grown beyond the 8 px core but not into the other line.
        let h0 = boxes[0][3].1 - boxes[0][0].1;
        assert!(h0 > 8.0 && h0 < 30.0, "{h0}");
    }

    #[test]
    fn prepare_caps_the_long_side() {
        let img = Bgr::new(1700, 2200);
        let (p, rx, ry) = prepare(&img);
        assert!(p.w.max(p.h) <= 2016 && p.w % 32 == 0 && p.h % 32 == 0);
        assert!((rx * p.w as f32 - 1700.0).abs() < 1.0 && (ry * p.h as f32 - 2200.0).abs() < 1.0);
    }
}
