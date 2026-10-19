//! Compare two documents (PRD 01 G, PRD 05 "Compare"): pages are aligned through insertions and
//! deletions, then each matched pair is compared word by word (insertions, deletions,
//! replacements, moved passages, font/size changes) and pixel by pixel (changes that are not text:
//! images, drawings, colours, positions). Results carry rectangles in unrotated page space for both
//! sides so a viewer can highlight them.
//!
//! Limits, stated in the report: words are compared in the engine's character order (no reading-
//! order analysis), so reflowed columns can show as changes; pages without text are aligned by a
//! small thumbnail; pages moved further than the alignment band show as deleted and inserted (a
//! later pass labels identical ones as moved); visual differences below the pixel threshold at the
//! comparison resolution are not reported.

use crate::document::{load, OpenOptions};
use crate::redact::{box_to_px, displayed_box, rotation_degrees, R};
use crate::{engine, map_pdfium_error};
use image::{imageops::FilterType, RgbaImage};
use pdfium_render::prelude::*;
use pw_core::ops::RectPt;
use pw_core::{ErrorCategory, PwError, PwResult};
use serde::Serialize;
use similar::{capture_diff_slices_deadline, Algorithm, DiffOp};
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::Path;
use std::time::{Duration, Instant};

/// Largest document compared (pages).
pub const MAX_COMPARE_PAGES: u32 = 2000;
/// Words read per page (the rest is not compared; reported).
const MAX_WORDS: usize = 50_000;
/// Time allowed for one page's word diff before it falls back to a coarser result.
const DIFF_DEADLINE: Duration = Duration::from_secs(2);
/// Pages are matched when at least this similar (0–1).
const MIN_PAGE_MATCH: f32 = 0.25;
/// Pages further apart than this (beyond the length difference) are not aligned with each other.
const ALIGN_BAND: usize = 60;
/// Unmatched pages at least this similar are reported as moved.
const MOVED_PAGE_MATCH: f32 = 0.95;
/// Passages of at least this many words found deleted in one place and inserted in another are moved.
const MOVE_MIN_WORDS: usize = 4;
/// Thumbnail side for aligning pages without text.
const FINGERPRINT: u32 = 32;
/// Longest side of the visual comparison renders, in pixels.
const VISUAL_PIXELS: f32 = 800.0;
/// Grid cell (pixels) for grouping visual differences, and what makes a pixel/cell "changed".
const CELL: u32 = 8;
const PIXEL_DELTA: u8 = 40;
const CELL_MIN_PIXELS: u32 = 3;
/// Visual regions reported per page (the rest are merged into one).
const MAX_REGIONS: usize = 40;
/// Font sizes differing by more than this (points) are a format change.
const SIZE_TOLERANCE: f32 = 0.5;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PairStatus {
    Same,
    Changed,
    Inserted,
    Deleted,
    Moved,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    Inserted,
    Deleted,
    Replaced,
    Moved,
    Format,
    Visual,
    PageSize,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Change {
    pub kind: ChangeKind,
    /// The words involved (for a replacement: the old words; `new_text` holds the new ones).
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub new_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    pub old_rects: Vec<RectPt>,
    pub new_rects: Vec<RectPt>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PagePair {
    /// 0-based page indexes; `None` on the side where the page does not exist.
    pub old: Option<u32>,
    pub new: Option<u32>,
    pub status: PairStatus,
    pub changes: Vec<Change>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MetaChange {
    pub field: String,
    pub old: String,
    pub new: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct CompareSummary {
    pub pages_changed: u32,
    pub pages_inserted: u32,
    pub pages_deleted: u32,
    pub pages_moved: u32,
    pub text_changes: u32,
    pub format_changes: u32,
    pub visual_changes: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CompareReport {
    pub old_pages: u32,
    pub new_pages: u32,
    pub pairs: Vec<PagePair>,
    pub metadata: Vec<MetaChange>,
    pub summary: CompareSummary,
    /// Limits that applied to this comparison (for example pages with too many words).
    pub notes: Vec<String>,
}

#[derive(Debug, Clone)]
struct Word {
    text: String,
    rect: R,
    font: String,
    size: f32,
}

struct PageData {
    words: Vec<Word>,
    /// Hashed word bag, sorted by hash.
    bag: Vec<(u64, u32)>,
    total: u32,
    thumb: Vec<u8>,
    /// Displayed size in points and rotation.
    size: (f32, f32),
    rotation: u32,
    bbox: R,
}

fn hash_word(w: &str) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    w.hash(&mut h);
    h.finish()
}

fn union(a: &R, b: &R) -> R {
    R { l: a.l.min(b.l), b: a.b.min(b.b), r: a.r.max(b.r), t: a.t.max(b.t) }
}

fn render(page: &PdfPage, longest_px: f32) -> PwResult<(RgbaImage, f32)> {
    let longest = page.width().value.max(page.height().value).max(1.0);
    let scale = longest_px / longest;
    let config = PdfRenderConfig::new().scale_page_by_factor(scale).render_form_data(true).render_annotations(true);
    let bitmap = page.render_with_config(&config).map_err(|e| map_pdfium_error(&e, "render"))?;
    Ok((bitmap.as_image().map_err(|e| map_pdfium_error(&e, "render"))?.to_rgba8(), scale))
}

fn words_of(page: &PdfPage, truncated: &mut bool) -> PwResult<Vec<Word>> {
    let text = page.text().map_err(|e| map_pdfium_error(&e, "text"))?;
    let mut words: Vec<Word> = Vec::new();
    let mut cur: Option<Word> = None;
    for ch in text.chars().iter() {
        let c = ch.unicode_char().unwrap_or(' ');
        if c.is_whitespace() || c == '\u{fffe}' || c == '\0' {
            if let Some(w) = cur.take() {
                words.push(w);
            }
            continue;
        }
        let rect = ch.loose_bounds().map(|b| R::from_pdf(&b)).ok();
        match (&mut cur, rect) {
            (Some(w), Some(r)) => {
                w.text.push(c);
                w.rect = union(&w.rect, &r);
            }
            (Some(w), None) => w.text.push(c),
            (None, r) => {
                cur = Some(Word {
                    text: c.to_string(),
                    rect: r.unwrap_or(R { l: 0.0, b: 0.0, r: 0.0, t: 0.0 }),
                    font: ch.font_name(),
                    size: ch.scaled_font_size().value,
                });
            }
        }
        if words.len() >= MAX_WORDS {
            *truncated = true;
            break;
        }
    }
    if let Some(w) = cur.take() {
        words.push(w);
    }
    Ok(words)
}

fn page_data(page: &PdfPage, truncated: &mut bool) -> PwResult<PageData> {
    let words = words_of(page, truncated)?;
    let mut counts: HashMap<u64, u32> = HashMap::new();
    for w in &words {
        *counts.entry(hash_word(&w.text)).or_default() += 1;
    }
    let mut bag: Vec<(u64, u32)> = counts.into_iter().collect();
    bag.sort_unstable();
    let (img, _) = render(page, (FINGERPRINT * 4) as f32)?;
    let gray = image::DynamicImage::ImageRgba8(img).into_luma8();
    let thumb = image::imageops::resize(&gray, FINGERPRINT, FINGERPRINT, FilterType::Triangle).into_raw();
    Ok(PageData {
        total: words.len() as u32,
        words,
        bag,
        thumb,
        size: (page.width().value, page.height().value),
        rotation: rotation_degrees(page),
        bbox: displayed_box(page),
    })
}

/// How alike two pages are (0–1): shared words, or the thumbnails when neither page has text.
fn similarity(a: &PageData, b: &PageData) -> f32 {
    match (a.total, b.total) {
        (0, 0) => {
            let mad: f32 = a.thumb.iter().zip(&b.thumb).map(|(x, y)| f32::from(x.abs_diff(*y))).sum::<f32>()
                / a.thumb.len().max(1) as f32;
            (1.0 - 4.0 * mad / 255.0).max(0.0)
        }
        (0, _) | (_, 0) => 0.0,
        _ => {
            let (mut i, mut j, mut shared) = (0, 0, 0u32);
            while i < a.bag.len() && j < b.bag.len() {
                match a.bag[i].0.cmp(&b.bag[j].0) {
                    std::cmp::Ordering::Less => i += 1,
                    std::cmp::Ordering::Greater => j += 1,
                    std::cmp::Ordering::Equal => {
                        shared += a.bag[i].1.min(b.bag[j].1);
                        i += 1;
                        j += 1;
                    }
                }
            }
            shared as f32 / (a.total + b.total - shared).max(1) as f32
        }
    }
}

/// Monotone page alignment maximising total similarity (pairs below the match threshold never pair).
fn align(old: &[PageData], new: &[PageData]) -> Vec<(Option<usize>, Option<usize>)> {
    let (n, m) = (old.len(), new.len());
    let band = n.abs_diff(m) + ALIGN_BAND;
    let near = |i: usize, j: usize| {
        // Compare positions scaled to the other document's length.
        let expected = if n == 0 { 0 } else { i * m / n.max(1) };
        expected.abs_diff(j) <= band
    };
    let mut score = vec![0f32; (n + 1) * (m + 1)];
    let mut back = vec![0u8; (n + 1) * (m + 1)]; // 1 = skip old, 2 = skip new, 3 = match
    let at = |i: usize, j: usize| i * (m + 1) + j;
    for i in 0..=n {
        for j in 0..=m {
            if i == 0 && j == 0 {
                continue;
            }
            let mut best = f32::NEG_INFINITY;
            let mut how = 0u8;
            // Ties prefer skipping the new page here, which (traced back from the end) lists a
            // deletion before an insertion at the same place.
            if j > 0 && score[at(i, j - 1)] > best {
                best = score[at(i, j - 1)];
                how = 2;
            }
            if i > 0 && score[at(i - 1, j)] > best {
                best = score[at(i - 1, j)];
                how = 1;
            }
            if i > 0 && j > 0 && near(i - 1, j - 1) {
                let s = similarity(&old[i - 1], &new[j - 1]);
                if s >= MIN_PAGE_MATCH && score[at(i - 1, j - 1)] + s > best {
                    best = score[at(i - 1, j - 1)] + s;
                    how = 3;
                }
            }
            score[at(i, j)] = best;
            back[at(i, j)] = how;
        }
    }
    let mut out = Vec::new();
    let (mut i, mut j) = (n, m);
    while i > 0 || j > 0 {
        match back[at(i, j)] {
            3 => {
                out.push((Some(i - 1), Some(j - 1)));
                i -= 1;
                j -= 1;
            }
            1 => {
                out.push((Some(i - 1), None));
                i -= 1;
            }
            _ => {
                out.push((None, Some(j - 1)));
                j -= 1;
            }
        }
    }
    out.reverse();
    out
}

fn rect_pt(r: &R) -> RectPt {
    RectPt { left: r.l, bottom: r.b, right: r.r, top: r.t }
}

/// Word boxes merged into one box per line.
fn line_rects(words: &[Word]) -> Vec<RectPt> {
    let mut out: Vec<R> = Vec::new();
    for w in words {
        if w.rect.r <= w.rect.l {
            continue;
        }
        match out.last_mut() {
            Some(last) => {
                let h = (last.t - last.b).max(w.rect.t - w.rect.b).max(1.0);
                let same_line = (last.b - w.rect.b).abs() < h * 0.5 && w.rect.l >= last.l - h;
                if same_line {
                    *last = union(last, &w.rect);
                } else {
                    out.push(w.rect);
                }
            }
            None => out.push(w.rect),
        }
    }
    out.iter().map(rect_pt).collect()
}

fn join(words: &[Word]) -> String {
    words.iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" ")
}

/// Text changes, plus the boxes (old, new) of unchanged words that only moved on the page
/// (reflow after an edit), which explain visual differences without being changes themselves.
fn text_changes(old: &[Word], new: &[Word], deadline: Instant) -> (Vec<Change>, Vec<(RectPt, RectPt)>) {
    let a: Vec<&str> = old.iter().map(|w| w.text.as_str()).collect();
    let b: Vec<&str> = new.iter().map(|w| w.text.as_str()).collect();
    let mut out = Vec::new();
    let mut reflowed = Vec::new();
    for op in capture_diff_slices_deadline(Algorithm::Myers, &a, &b, Some(deadline)) {
        match op {
            DiffOp::Equal { old_index, new_index, len } => {
                for k in 0..len {
                    let (x, y) = (&old[old_index + k].rect, &new[new_index + k].rect);
                    if (x.l - y.l).abs() > 1.0 || (x.b - y.b).abs() > 1.0 {
                        reflowed.push((rect_pt(x), rect_pt(y)));
                    }
                }
                // Same words: font or size changes are format changes (consecutive ones grouped).
                let mut k = 0;
                while k < len {
                    let differs = |k: usize| {
                        let (x, y) = (&old[old_index + k], &new[new_index + k]);
                        x.font != y.font || (x.size - y.size).abs() > SIZE_TOLERANCE
                    };
                    if !differs(k) {
                        k += 1;
                        continue;
                    }
                    let start = k;
                    while k < len && differs(k) {
                        k += 1;
                    }
                    let (o, n) = (&old[old_index + start..old_index + k], &new[new_index + start..new_index + k]);
                    let detail = format!("{} {:.1} pt → {} {:.1} pt", o[0].font, o[0].size, n[0].font, n[0].size);
                    out.push(Change {
                        kind: ChangeKind::Format,
                        text: join(n),
                        new_text: None,
                        detail: Some(detail),
                        old_rects: line_rects(o),
                        new_rects: line_rects(n),
                    });
                }
            }
            DiffOp::Delete { old_index, old_len, .. } => {
                let o = &old[old_index..old_index + old_len];
                out.push(Change {
                    kind: ChangeKind::Deleted,
                    text: join(o),
                    new_text: None,
                    detail: None,
                    old_rects: line_rects(o),
                    new_rects: Vec::new(),
                });
            }
            DiffOp::Insert { new_index, new_len, .. } => {
                let n = &new[new_index..new_index + new_len];
                out.push(Change {
                    kind: ChangeKind::Inserted,
                    text: join(n),
                    new_text: None,
                    detail: None,
                    old_rects: Vec::new(),
                    new_rects: line_rects(n),
                });
            }
            DiffOp::Replace { old_index, old_len, new_index, new_len } => {
                let (o, n) = (&old[old_index..old_index + old_len], &new[new_index..new_index + new_len]);
                out.push(Change {
                    kind: ChangeKind::Replaced,
                    text: join(o),
                    new_text: Some(join(n)),
                    detail: None,
                    old_rects: line_rects(o),
                    new_rects: line_rects(n),
                });
            }
        }
    }
    (out, reflowed)
}

/// Pixel-level differences between two renders as regions in each page's own space.
fn visual_changes(
    a: &RgbaImage,
    b: &RgbaImage,
    (old_box, old_rot, old_scale): (&R, u32, f32),
    (new_box, new_rot, new_scale): (&R, u32, f32),
    covered_old: &[RectPt],
    covered_new: &[RectPt],
) -> Vec<Change> {
    let (w, h) = (a.width().min(b.width()), a.height().min(b.height()));
    let (cw, ch) = (w.div_ceil(CELL), h.div_ceil(CELL));
    let mut marked = vec![false; (cw * ch) as usize];
    for cy in 0..ch {
        for cx in 0..cw {
            let mut n = 0;
            'cell: for y in cy * CELL..((cy + 1) * CELL).min(h) {
                for x in cx * CELL..((cx + 1) * CELL).min(w) {
                    let (p, q) = (a.get_pixel(x, y).0, b.get_pixel(x, y).0);
                    if (0..3).any(|c| p[c].abs_diff(q[c]) > PIXEL_DELTA) {
                        n += 1;
                        if n >= CELL_MIN_PIXELS {
                            break 'cell;
                        }
                    }
                }
            }
            marked[(cy * cw + cx) as usize] = n >= CELL_MIN_PIXELS;
        }
    }
    // Connected groups of changed cells (8-neighbourhood, so diagonal strokes stay together).
    let mut seen = vec![false; marked.len()];
    let mut boxes: Vec<(u32, u32, u32, u32)> = Vec::new();
    for start in 0..marked.len() {
        if !marked[start] || seen[start] {
            continue;
        }
        let mut stack = vec![start];
        seen[start] = true;
        let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0, 0);
        while let Some(i) = stack.pop() {
            let (x, y) = (i as u32 % cw, i as u32 / cw);
            (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
            for dy in -1i64..=1 {
                for dx in -1i64..=1 {
                    let (nx, ny) = (i64::from(x) + dx, i64::from(y) + dy);
                    if nx < 0 || ny < 0 || nx >= i64::from(cw) || ny >= i64::from(ch) {
                        continue;
                    }
                    let k = (ny as u32 * cw + nx as u32) as usize;
                    if marked[k] && !seen[k] {
                        seen[k] = true;
                        stack.push(k);
                    }
                }
            }
        }
        boxes.push((x0 * CELL, y0 * CELL, ((x1 + 1) * CELL).min(w), ((y1 + 1) * CELL).min(h)));
    }
    // Regions already explained by a text change on the new page are not repeated.
    // Text boxes grow by a cell: regions are cell-aligned, so they overhang the words they cover.
    let grow = CELL as f32;
    let to_px = |r: &RectPt, bbox: &R, rot: u32, scale: f32| {
        let (a, b, c, d) = box_to_px(&R { l: r.left, b: r.bottom, r: r.right, t: r.top }, bbox, rot, scale);
        (a - grow, b - grow, c + grow, d + grow)
    };
    let covered_px: Vec<(f32, f32, f32, f32)> = covered_old
        .iter()
        .map(|r| to_px(r, old_box, old_rot, old_scale))
        .chain(covered_new.iter().map(|r| to_px(r, new_box, new_rot, new_scale)))
        .collect();
    boxes.retain(|&(x0, y0, x1, y1)| {
        let area = ((x1 - x0) * (y1 - y0)) as f32;
        let inside: f32 = covered_px
            .iter()
            .map(|&(a0, b0, a1, b1)| {
                let iw = (a1.min(x1 as f32) - a0.max(x0 as f32)).max(0.0);
                let ih = (b1.min(y1 as f32) - b0.max(y0 as f32)).max(0.0);
                iw * ih
            })
            .sum();
        inside < area * 0.6
    });
    if boxes.len() > MAX_REGIONS {
        let all = boxes.iter().fold((u32::MAX, u32::MAX, 0, 0), |acc, b| {
            (acc.0.min(b.0), acc.1.min(b.1), acc.2.max(b.2), acc.3.max(b.3))
        });
        boxes = vec![all];
    }
    boxes
        .into_iter()
        .map(|(x0, y0, x1, y1)| {
            let px = (x0 as f32, y0 as f32, x1 as f32, y1 as f32);
            Change {
                kind: ChangeKind::Visual,
                text: String::new(),
                new_text: None,
                detail: None,
                old_rects: vec![px_to_rect(px, old_box, old_rot, old_scale)],
                new_rects: vec![px_to_rect(px, new_box, new_rot, new_scale)],
            }
        })
        .collect()
}

/// Inverse of `box_to_px`: a pixel rectangle of a render back to unrotated page space.
fn px_to_rect((x0, y0, x1, y1): (f32, f32, f32, f32), bbox: &R, rot: u32, s: f32) -> RectPt {
    let w = (bbox.r - bbox.l) * s;
    let h = (bbox.t - bbox.b) * s;
    // Undo the rotation: displayed pixel (px, py) -> unrotated (u, v) from the top-left, in pixels.
    let unmap = |px: f32, py: f32| match rot {
        90 => (py, h - px),
        180 => (w - px, h - py),
        270 => (w - py, px),
        _ => (px, py),
    };
    let (ua, va) = unmap(x0, y0);
    let (ub, vb) = unmap(x1, y1);
    let (u0, u1, v0, v1) = (ua.min(ub), ua.max(ub), va.min(vb), va.max(vb));
    RectPt { left: bbox.l + u0 / s, right: bbox.l + u1 / s, top: bbox.t - v0 / s, bottom: bbox.t - v1 / s }
}

fn cancelled_error() -> PwError {
    PwError::new(ErrorCategory::Cancelled, "job.cancelled", "The comparison was cancelled.")
}

/// Compare `old` with `new` (read-only).
pub fn compare(
    old: &Path,
    new: &Path,
    opts: &OpenOptions,
    cancelled: &dyn Fn() -> bool,
    progress: &dyn Fn(u64, u64),
) -> PwResult<CompareReport> {
    let pdfium = engine()?;
    let da = load(&pdfium, old, opts)?;
    let db = load(&pdfium, new, opts)?;
    let (na, nb) = (da.pages().len().max(0) as u32, db.pages().len().max(0) as u32);
    if na > MAX_COMPARE_PAGES || nb > MAX_COMPARE_PAGES {
        return Err(PwError::new(
            ErrorCategory::UnsupportedOperation,
            "compare.too_large",
            format!("Documents with more than {MAX_COMPARE_PAGES} pages cannot be compared yet."),
        ));
    }
    let mut notes = Vec::new();
    let total = u64::from(na) * 2 + u64::from(nb);
    let mut done = 0u64;
    let mut read = |doc: &PdfDocument, n: u32, side: &str| -> PwResult<Vec<PageData>> {
        let mut out = Vec::with_capacity(n as usize);
        for i in 0..n {
            if cancelled() {
                return Err(cancelled_error());
            }
            let page = doc.pages().get(i as PdfPageIndex).map_err(|e| map_pdfium_error(&e, "page"))?;
            let mut truncated = false;
            out.push(page_data(&page, &mut truncated)?);
            if truncated {
                notes.push(format!(
                    "Only the first {MAX_WORDS} words of page {} of the {side} document were compared.",
                    i + 1
                ));
            }
            done += 1;
            progress(done, total);
        }
        Ok(out)
    };
    let pa = read(&da, na, "old")?;
    let pb = read(&db, nb, "new")?;

    let mut pairs = Vec::new();
    let mut summary = CompareSummary::default();
    for (oi, ni) in align(&pa, &pb) {
        if cancelled() {
            return Err(cancelled_error());
        }
        let pair = match (oi, ni) {
            (Some(i), Some(j)) => {
                let (a, b) = (&pa[i], &pb[j]);
                let (mut changes, reflowed) = text_changes(&a.words, &b.words, Instant::now() + DIFF_DEADLINE);
                let size_differs =
                    (a.size.0 - b.size.0).abs() > 1.0 || (a.size.1 - b.size.1).abs() > 1.0 || a.rotation != b.rotation;
                if size_differs {
                    changes.push(Change {
                        kind: ChangeKind::PageSize,
                        text: String::new(),
                        new_text: None,
                        detail: Some(format!(
                            "{:.0} × {:.0} pt{} → {:.0} × {:.0} pt{}",
                            a.size.0,
                            a.size.1,
                            if a.rotation != 0 { format!(", turned {}°", a.rotation) } else { String::new() },
                            b.size.0,
                            b.size.1,
                            if b.rotation != 0 { format!(", turned {}°", b.rotation) } else { String::new() },
                        )),
                        old_rects: Vec::new(),
                        new_rects: Vec::new(),
                    });
                } else {
                    let pa_page = da.pages().get(i as PdfPageIndex).map_err(|e| map_pdfium_error(&e, "page"))?;
                    let pb_page = db.pages().get(j as PdfPageIndex).map_err(|e| map_pdfium_error(&e, "page"))?;
                    let (ra, sa) = render(&pa_page, VISUAL_PIXELS)?;
                    let (rb, sb) = render(&pb_page, VISUAL_PIXELS)?;
                    // Visual differences explained by text changes or reflowed text are not repeated.
                    let covered_old: Vec<RectPt> = changes
                        .iter()
                        .flat_map(|c| c.old_rects.iter().copied())
                        .chain(reflowed.iter().map(|r| r.0))
                        .collect();
                    let covered_new: Vec<RectPt> = changes
                        .iter()
                        .flat_map(|c| c.new_rects.iter().copied())
                        .chain(reflowed.iter().map(|r| r.1))
                        .collect();
                    changes.extend(visual_changes(
                        &ra,
                        &rb,
                        (&a.bbox, a.rotation, sa),
                        (&b.bbox, b.rotation, sb),
                        &covered_old,
                        &covered_new,
                    ));
                }
                done += 1;
                progress(done, total);
                let status = if changes.is_empty() { PairStatus::Same } else { PairStatus::Changed };
                PagePair { old: Some(i as u32), new: Some(j as u32), status, changes }
            }
            (Some(i), None) => {
                PagePair { old: Some(i as u32), new: None, status: PairStatus::Deleted, changes: Vec::new() }
            }
            (None, Some(j)) => {
                PagePair { old: None, new: Some(j as u32), status: PairStatus::Inserted, changes: Vec::new() }
            }
            (None, None) => continue,
        };
        pairs.push(pair);
    }

    // Whole pages deleted in one place and inserted in another: moved.
    let deleted: Vec<usize> = (0..pairs.len()).filter(|&k| pairs[k].status == PairStatus::Deleted).collect();
    let inserted: Vec<usize> = (0..pairs.len()).filter(|&k| pairs[k].status == PairStatus::Inserted).collect();
    let mut used = vec![false; pairs.len()];
    for &d in &deleted {
        let Some(oi) = pairs[d].old else { continue };
        let best = inserted
            .iter()
            .filter(|&&k| !used[k])
            .filter_map(|&k| pairs[k].new.map(|nj| (k, similarity(&pa[oi as usize], &pb[nj as usize]))))
            .filter(|&(_, s)| s >= MOVED_PAGE_MATCH)
            .max_by(|x, y| x.1.total_cmp(&y.1));
        if let Some((k, _)) = best {
            used[k] = true;
            let nj = pairs[k].new;
            pairs[d].status = PairStatus::Moved;
            pairs[d].new = nj;
            pairs[k].status = PairStatus::Moved;
            pairs[k].old = Some(oi);
        }
    }
    // Remove the second half of each moved page pair (the first one now names both pages).
    let mut seen_moves = std::collections::HashSet::new();
    pairs.retain(|p| p.status != PairStatus::Moved || seen_moves.insert((p.old, p.new)));

    // Passages deleted on one page and inserted on another (or elsewhere on the same page): moved.
    mark_moved_passages(&mut pairs);

    for p in &pairs {
        match p.status {
            PairStatus::Changed => summary.pages_changed += 1,
            PairStatus::Inserted => summary.pages_inserted += 1,
            PairStatus::Deleted => summary.pages_deleted += 1,
            PairStatus::Moved => summary.pages_moved += 1,
            PairStatus::Same => {}
        }
        for c in &p.changes {
            match c.kind {
                ChangeKind::Format => summary.format_changes += 1,
                ChangeKind::Visual | ChangeKind::PageSize => summary.visual_changes += 1,
                _ => summary.text_changes += 1,
            }
        }
    }

    let meta = |d: &PdfDocument, t: PdfDocumentMetadataTagType| {
        d.metadata().get(t).map(|v| v.value().trim().to_string()).unwrap_or_default()
    };
    let mut metadata = Vec::new();
    for (field, t) in [
        ("Title", PdfDocumentMetadataTagType::Title),
        ("Author", PdfDocumentMetadataTagType::Author),
        ("Subject", PdfDocumentMetadataTagType::Subject),
        ("Keywords", PdfDocumentMetadataTagType::Keywords),
    ] {
        let (a, b) = (meta(&da, t), meta(&db, t));
        if a != b {
            metadata.push(MetaChange { field: field.into(), old: a, new: b });
        }
    }
    Ok(CompareReport { old_pages: na, new_pages: nb, pairs, metadata, summary, notes })
}

fn mark_moved_passages(pairs: &mut [PagePair]) {
    let norm = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut deletions: Vec<(usize, usize, String)> = Vec::new();
    let mut insertions: Vec<(usize, usize, String)> = Vec::new();
    for (pi, p) in pairs.iter().enumerate() {
        for (ci, c) in p.changes.iter().enumerate() {
            let words = c.text.split_whitespace().count();
            if words < MOVE_MIN_WORDS {
                continue;
            }
            match c.kind {
                ChangeKind::Deleted => deletions.push((pi, ci, norm(&c.text))),
                ChangeKind::Inserted => insertions.push((pi, ci, norm(&c.text))),
                _ => {}
            }
        }
    }
    let mut taken = vec![false; insertions.len()];
    for (dp, dc, text) in deletions {
        let Some(k) = (0..insertions.len()).find(|&k| !taken[k] && insertions[k].2 == text) else { continue };
        taken[k] = true;
        let (ip, ic, _) = insertions[k];
        let to = pairs[ip].new.map(|n| n + 1);
        let from = pairs[dp].old.map(|n| n + 1);
        let d = &mut pairs[dp].changes[dc];
        d.kind = ChangeKind::Moved;
        d.detail = to.map(|n| format!("moved to page {n} of the new document"));
        let i = &mut pairs[ip].changes[ic];
        i.kind = ChangeKind::Moved;
        i.detail = from.map(|n| format!("moved from page {n} of the old document"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(text: &str, x: f32, font: &str, size: f32) -> Word {
        Word { text: text.into(), rect: R { l: x, b: 700.0, r: x + 20.0, t: 712.0 }, font: font.into(), size }
    }

    fn words(s: &str) -> Vec<Word> {
        s.split_whitespace().enumerate().map(|(i, t)| w(t, 72.0 + 25.0 * i as f32, "Helvetica", 12.0)).collect()
    }

    #[test]
    fn word_changes_are_classified() {
        let far = Instant::now() + Duration::from_secs(5);
        let (c, _) = text_changes(&words("the quick brown fox"), &words("the slow brown fox jumps"), far);
        let kinds: Vec<ChangeKind> = c.iter().map(|c| c.kind).collect();
        assert_eq!(kinds, vec![ChangeKind::Replaced, ChangeKind::Inserted]);
        assert_eq!((c[0].text.as_str(), c[0].new_text.as_deref()), ("quick", Some("slow")));
        assert_eq!(c[1].text, "jumps");
        assert_eq!(c[1].new_rects.len(), 1);
        assert!(c[1].old_rects.is_empty());

        let mut bold = words("a b c d");
        bold[1].font = "Helvetica-Bold".into();
        bold[2].font = "Helvetica-Bold".into();
        let (c, reflowed) = text_changes(&words("a b c d"), &bold, far);
        assert!(reflowed.is_empty(), "nothing moved");
        assert_eq!(c.len(), 1, "{c:?}");
        assert_eq!((c[0].kind, c[0].text.as_str()), (ChangeKind::Format, "b c"));
        assert!(c[0].detail.as_deref().unwrap_or_default().contains("Helvetica-Bold"));
    }

    #[test]
    fn line_boxes_merge_words_on_one_line_only() {
        let mut ws = words("one two three");
        ws.push(Word {
            text: "next".into(),
            rect: R { l: 72.0, b: 680.0, r: 100.0, t: 692.0 },
            font: "H".into(),
            size: 12.0,
        });
        let r = line_rects(&ws);
        assert_eq!(r.len(), 2);
        assert_eq!((r[0].left, r[0].right), (72.0, 142.0));
    }

    fn page(text: &str) -> PageData {
        let words = words(text);
        let mut counts: HashMap<u64, u32> = HashMap::new();
        for w in &words {
            *counts.entry(hash_word(&w.text)).or_default() += 1;
        }
        let mut bag: Vec<(u64, u32)> = counts.into_iter().collect();
        bag.sort_unstable();
        PageData {
            total: words.len() as u32,
            words,
            bag,
            thumb: vec![255; (FINGERPRINT * FINGERPRINT) as usize],
            size: (612.0, 792.0),
            rotation: 0,
            bbox: R { l: 0.0, b: 0.0, r: 612.0, t: 792.0 },
        }
    }

    #[test]
    fn alignment_survives_inserted_and_deleted_pages() {
        let old: Vec<PageData> =
            ["alpha one", "beta two", "gamma three", "delta four"].iter().map(|t| page(t)).collect();
        let new: Vec<PageData> =
            ["alpha one", "brand new page", "gamma three", "delta four changed"].iter().map(|t| page(t)).collect();
        let a = align(&old, &new);
        assert_eq!(
            a,
            vec![(Some(0), Some(0)), (Some(1), None), (None, Some(1)), (Some(2), Some(2)), (Some(3), Some(3))],
            "beta deleted, new page inserted, delta matched despite the edit"
        );
        let blank = |v: u8| PageData { thumb: vec![v; (FINGERPRINT * FINGERPRINT) as usize], ..page("") };
        let (b1, b2) = (blank(255), blank(250));
        assert!(similarity(&b1, &b2) > 0.9, "near-identical scans match");
        assert!(similarity(&blank(255), &blank(0)) == 0.0, "different scans do not");
        assert_eq!(similarity(&page("text"), &blank(255)), 0.0);
    }

    #[test]
    fn passages_moved_between_pages_are_labelled() {
        let ch = |kind, text: &str| Change {
            kind,
            text: text.into(),
            new_text: None,
            detail: None,
            old_rects: vec![],
            new_rects: vec![],
        };
        let mut pairs = vec![
            PagePair {
                old: Some(0),
                new: Some(0),
                status: PairStatus::Changed,
                changes: vec![ch(ChangeKind::Deleted, "a whole moved paragraph here")],
            },
            PagePair {
                old: Some(1),
                new: Some(1),
                status: PairStatus::Changed,
                changes: vec![
                    ch(ChangeKind::Inserted, "a whole  moved paragraph here"),
                    ch(ChangeKind::Inserted, "short one"),
                ],
            },
        ];
        mark_moved_passages(&mut pairs);
        assert_eq!(pairs[0].changes[0].kind, ChangeKind::Moved);
        assert_eq!(pairs[0].changes[0].detail.as_deref(), Some("moved to page 2 of the new document"));
        assert_eq!(pairs[1].changes[0].kind, ChangeKind::Moved);
        assert_eq!(pairs[1].changes[1].kind, ChangeKind::Inserted, "short passages are not matched");
    }

    #[test]
    fn pixel_rects_map_back_for_every_rotation() {
        let bbox = R { l: 10.0, b: 20.0, r: 622.0, t: 812.0 };
        let r = R { l: 100.0, b: 300.0, r: 250.0, t: 340.0 };
        for rot in [0, 90, 180, 270] {
            let px = box_to_px(&r, &bbox, rot, 1.5);
            let back = px_to_rect(px, &bbox, rot, 1.5);
            for (a, b) in [(back.left, r.l), (back.bottom, r.b), (back.right, r.r), (back.top, r.t)] {
                assert!((a - b).abs() < 0.01, "rotation {rot}: {back:?} vs {r:?}");
            }
        }
    }

    #[test]
    fn visual_regions_group_cells_and_skip_text_changes() {
        let bbox = R { l: 0.0, b: 0.0, r: 100.0, t: 100.0 };
        let a = RgbaImage::from_pixel(100, 100, image::Rgba([255, 255, 255, 255]));
        let mut b = a.clone();
        for y in 10..30 {
            for x in 10..40 {
                b.put_pixel(x, y, image::Rgba([0, 0, 0, 255]));
            }
        }
        for y in 70..80 {
            for x in 70..80 {
                b.put_pixel(x, y, image::Rgba([200, 0, 0, 255]));
            }
        }
        let side = (&bbox, 0, 1.0);
        let v = visual_changes(&a, &b, side, side, &[], &[]);
        assert_eq!(v.len(), 2, "{v:?}");
        // Pixel rows 10–30 from the top are page y 70–90 (origin bottom-left), within a cell.
        let r = v.iter().map(|c| c.new_rects[0]).find(|r| r.left < 50.0).expect("first region");
        assert!(r.left <= 10.0 && r.right >= 40.0 && r.bottom <= 70.0 && r.top >= 90.0, "{r:?}");
        // A region explained by a text change is not repeated.
        let covered = [RectPt { left: 68.0, bottom: 18.0, right: 82.0, top: 32.0 }];
        assert_eq!(visual_changes(&a, &b, side, side, &[], &covered).len(), 1);
        assert_eq!(visual_changes(&a, &b, side, side, &covered, &[]).len(), 1, "old-side boxes explain too");
        // Faint differences are not changes.
        let faint = RgbaImage::from_pixel(100, 100, image::Rgba([240, 240, 240, 255]));
        assert!(visual_changes(&a, &faint, side, side, &[], &[]).is_empty());
    }
}
