//! Translation into a PDF with the original layout (PRD 08): every translated paragraph's text is
//! taken off the page and its translation is written in the same place, in the same font family,
//! size and colours (bold, italic and coloured words keep their style), wrapped to the same width
//! and made slightly smaller only when it would not fit. Everything else on the page (tables,
//! shading, lines, boxes, pictures, untranslated text) stays exactly as it was.

use super::layout::{Align, Frame, Item, PageLayout, Paragraph, Rect, Run};
use super::translate::StyledWord;
use crate::map_pdfium_error;
use pdfium_render::prelude::*;
use pw_core::{ErrorCategory, PwError, PwResult};
use std::collections::HashMap;

/// A block of text to translate in place: the lines of a paragraph, a box, a table cell, or one
/// of several pieces of text set apart on the same lines.
pub(super) struct Placed {
    pub page: u32,
    pub para: Paragraph,
    /// Lowest y the lines may reach (the next item's top, a cell's bottom).
    pub limit: f32,
    /// In a table cell (never justified; may use the cell's whole width).
    pub in_cell: bool,
}

/// A piece of one line: characters (with their run in `source` and their box), where it is.
struct LinePiece {
    source: usize,
    chars: Vec<(char, usize, Rect)>,
    left: f32,
    right: f32,
    baseline: f32,
    size: f32,
    /// Starts a new paragraph in the original layout (or is set apart by a tab).
    para_start: bool,
}

fn starts_list(text: &str) -> bool {
    let t = text.trim_start();
    let mut it = t.chars();
    match it.next() {
        Some('•' | '·' | '▪' | '‣' | '◦' | '–' | '—' | '-' | '*' | '►' | '✓') => true,
        Some(c) if c.is_ascii_digit() => {
            let rest: String = t.chars().skip_while(|c| c.is_ascii_digit()).take(2).collect();
            rest.starts_with(". ") || rest.starts_with(") ")
        }
        _ => false,
    }
}

/// Cut a paragraph's lines into pieces at tabs and wide gaps.
fn line_pieces(source: usize, para: &Paragraph, out: &mut Vec<LinePiece>) {
    let Some(frame) = para.frame else { return };
    let size = para.runs.iter().map(|r| r.size).fold(0.0, f32::max).max(1.0);
    let pitch = if para.pitch > 0.0 { para.pitch } else { size * 1.2 };
    let baseline_of =
        |line: usize| para.line_baselines.get(line).copied().unwrap_or(frame.baseline - pitch * line as f32);
    let mut line = 0usize;
    let mut prev: Option<(Rect, f32)> = None;
    let mut cur: Vec<(char, usize, Rect)> = Vec::new();
    let mut first_piece = true;
    let flush = |cur: &mut Vec<(char, usize, Rect)>, line: usize, first_piece: &mut bool, out: &mut Vec<LinePiece>| {
        while cur.last().is_some_and(|c| c.0.is_whitespace()) {
            cur.pop();
        }
        while cur.first().is_some_and(|c| c.0.is_whitespace()) {
            cur.remove(0);
        }
        let ink: Vec<Rect> = cur.iter().filter(|c| !c.0.is_whitespace()).map(|c| c.2).collect();
        if let (Some(l), Some(r)) =
            (ink.iter().map(|b| b.left).reduce(f32::min), ink.iter().map(|b| b.right).reduce(f32::max))
        {
            let size = cur.iter().map(|c| para.runs[c.1].size).fold(0.0, f32::max);
            out.push(LinePiece {
                source,
                chars: std::mem::take(cur),
                left: l,
                right: r,
                baseline: baseline_of(line),
                size,
                para_start: std::mem::replace(first_piece, false) || line == usize::MAX,
            });
        } else {
            cur.clear();
        }
    };
    let mut k = 0usize;
    let mut set_apart = false;
    for (ri, r) in para.runs.iter().enumerate() {
        for ch in r.text.chars() {
            let Some(b) = para.boxes.get(k).copied() else { break };
            k += 1;
            if !ch.is_whitespace() {
                if let Some((pb, ps)) = prev {
                    let gap = b.left - pb.right;
                    if (pb.bottom - b.bottom).abs() > 0.5 * size {
                        flush(&mut cur, line, &mut first_piece, out);
                        line += 1;
                    } else if gap > 1.4 * size || (gap > 0.5 * size && (ps - r.size).abs() > 0.2 * ps.max(r.size)) {
                        // A wide gap, or a gap and a change of size, sets text apart (an arrow
                        // between boxes).
                        flush(&mut cur, line, &mut first_piece, out);
                        set_apart = true;
                    }
                }
                prev = Some((b, r.size));
            }
            if ch == '\t' {
                flush(&mut cur, line, &mut first_piece, out);
                set_apart = true;
                continue;
            }
            if set_apart && cur.is_empty() {
                first_piece = true;
                set_apart = false;
            }
            cur.push((ch, ri, b));
        }
    }
    flush(&mut cur, line, &mut first_piece, out);
}

/// Stack line pieces into blocks: a piece continues the block whose last line is just above it,
/// overlaps it, has the same size, and either belongs to the same original paragraph or reads
/// as its continuation (the line above is not shorter; or both are centred on the same point).
fn blocks(pieces: &[LinePiece], paras: &[&Paragraph]) -> Vec<Vec<usize>> {
    let mut blocks: Vec<Vec<usize>> = Vec::new();
    let text = |lp: &LinePiece| lp.chars.iter().map(|c| c.0).collect::<String>();
    for (i, lp) in pieces.iter().enumerate() {
        let mut best: Option<(usize, f32)> = None;
        for (bi, b) in blocks.iter().enumerate() {
            let Some(&li) = b.last() else { continue };
            let last = &pieces[li];
            let gap = last.baseline - lp.baseline;
            let size = last.size.max(lp.size);
            if gap < 0.8 * size || gap > 1.75 * size || (last.size - lp.size).abs() > 0.12 * size {
                continue;
            }
            let overlap = last.right.min(lp.right) - last.left.max(lp.left);
            if overlap <= 0.0 {
                continue;
            }
            let same_para = last.source == lp.source && !lp.para_start;
            // Lines of different paragraphs join only in the same style (a bold title line stays
            // on its own above the text of a box).
            let style = |q: &LinePiece| {
                // The style most of the line's letters have.
                let mut count: HashMap<usize, usize> = HashMap::new();
                for c in q.chars.iter().filter(|c| c.0.is_alphanumeric()) {
                    *count.entry(c.1).or_default() += 1;
                }
                let ri = count
                    .into_iter()
                    .max_by_key(|&(ri, n)| (n, usize::MAX - ri))
                    .map(|(ri, _)| ri)
                    .unwrap_or(q.chars[0].1);
                let r = &paras[q.source].runs[ri];
                (r.bold, r.italic, r.color, r.font.clone())
            };
            if !same_para && style(last) != style(lp) {
                continue;
            }
            let block_right = b.iter().map(|&j| pieces[j].right).fold(f32::MIN, f32::max);
            let block_left = b.iter().map(|&j| pieces[j].left).fold(f32::MAX, f32::min);
            let continues = same_para
                || (!starts_list(&text(lp))
                    && paras[lp.source].heading.is_none()
                    && paras[last.source].heading.is_none()
                    && ((last.right >= lp.right - 5.0 && (lp.left - block_left).abs() < 2.0)
                        || ((last.left + last.right) / 2.0 - (lp.left + lp.right) / 2.0).abs() < 3.0)
                    && block_right - last.right < 0.25 * (block_right - block_left).max(1.0) + 1.0);
            if continues && best.is_none_or(|(_, o)| overlap > o) {
                best = Some((bi, overlap));
            }
        }
        match best {
            Some((bi, _)) => blocks[bi].push(i),
            None => blocks.push(vec![i]),
        }
    }
    blocks
}

/// One block as a paragraph to write in place.
fn block_paragraph(b: &[usize], pieces: &[LinePiece], paras: &[&Paragraph], grow_right: f32) -> Paragraph {
    let first = &pieces[b[0]];
    let src = paras[first.source];
    let mut runs: Vec<Run> = Vec::new();
    let mut boxes = Vec::new();
    let mut last_run: Option<(usize, usize)> = None;
    for (n, &i) in b.iter().enumerate() {
        let lp = &pieces[i];
        for (m, &(ch, ri, bx)) in lp.chars.iter().enumerate() {
            if n > 0 && m == 0 {
                if let Some(r) = runs.last_mut() {
                    // A soft hyphen at a line end joins the word; otherwise lines join with a space.
                    if !r.text.ends_with(char::is_whitespace) {
                        r.text.push(' ');
                        boxes.push(bx);
                    }
                }
            }
            if last_run != Some((lp.source, ri)) {
                runs.push(Run { text: String::new(), ..paras[lp.source].runs[ri].clone() });
                last_run = Some((lp.source, ri));
            }
            if let Some(r) = runs.last_mut() {
                r.text.push(ch);
            }
            boxes.push(bx);
        }
    }
    let left = b.iter().map(|&i| pieces[i].left).fold(f32::MAX, f32::min);
    let right = b.iter().map(|&i| pieces[i].right).fold(f32::MIN, f32::max);
    let top = b.iter().flat_map(|&i| pieces[i].chars.iter().map(|c| c.2.top)).fold(f32::MIN, f32::max);
    let bottom = b.iter().flat_map(|&i| pieces[i].chars.iter().map(|c| c.2.bottom)).fold(f32::MAX, f32::min);
    let spread = |v: Vec<f32>| v.iter().copied().fold(f32::MIN, f32::max) - v.iter().copied().fold(f32::MAX, f32::min);
    let lefts: Vec<f32> = b.iter().map(|&i| pieces[i].left).collect();
    let centres: Vec<f32> = b.iter().map(|&i| (pieces[i].left + pieces[i].right) / 2.0).collect();
    let rights: Vec<f32> = b.iter().map(|&i| pieces[i].right).collect();
    let single_source = b.iter().all(|&i| pieces[i].source == first.source);
    let align = if b.len() >= 2 && spread(centres.clone()) < 2.5 && spread(lefts.clone()) > 2.0 {
        Align::Center
    } else if single_source && b.len() == paras[first.source].line_baselines.len() {
        src.align
    } else if b.len() >= 3 && spread(rights[..rights.len() - 1].to_vec()) < 1.5 && spread(lefts[1..].to_vec()) < 1.5 {
        Align::Justify
    } else if b.len() == 1 && matches!(src.align, Align::Center | Align::Right) && single_source {
        src.align
    } else {
        Align::Left
    };
    let pitch = if b.len() >= 2 {
        (pieces[b[0]].baseline - pieces[*b.last().unwrap_or(&b[0])].baseline) / (b.len() - 1) as f32
    } else if src.pitch > 0.0 {
        src.pitch
    } else {
        first.size * 1.2
    };
    // A hanging first line (list items): later lines start at the second line's left.
    let first_line = if b.len() >= 2 { pieces[b[0]].left - pieces[b[1]].left } else { 0.0 };
    let indent = if b.len() >= 2 { pieces[b[1]].left - left } else { 0.0 };
    Paragraph {
        runs,
        align,
        indent,
        first_line,
        space_before: 0.0,
        pitch,
        heading: src.heading,
        tabs: Vec::new(),
        rect: Rect { left, bottom, right, top },
        boxes,
        frame: Some(Frame {
            baseline: first.baseline,
            left,
            right: grow_right.max(right),
            lines: b.len(),
            bottom_limit: None,
        }),
        line_baselines: b.iter().map(|&i| pieces[i].baseline).collect(),
        deco: src.deco,
    }
}

/// Blocks from a run of paragraphs sharing a column (or a table cell): `limit` is how low the
/// last one may go; `frame_right` the column's (cell's) right edge.
fn rebuild(page: u32, paras: &[&Paragraph], frame_right: f32, limit: f32, in_cell: bool, out: &mut Vec<Placed>) {
    let mut pieces = Vec::new();
    for (i, p) in paras.iter().enumerate() {
        line_pieces(i, p, &mut pieces);
    }
    let bl = blocks(&pieces, paras);
    let spans: Vec<(f32, f32, f32, f32)> = bl
        .iter()
        .map(|b| {
            (
                b.iter().map(|&i| pieces[i].left).fold(f32::MAX, f32::min),
                b.iter().map(|&i| pieces[i].right).fold(f32::MIN, f32::max),
                b.iter().map(|&i| pieces[i].baseline).fold(f32::MIN, f32::max),
                b.iter().map(|&i| pieces[i].baseline).fold(f32::MAX, f32::min),
            )
        })
        .collect();
    for (bi, b) in bl.iter().enumerate() {
        let (l, r, hi, lo) = spans[bi];
        let size = b.iter().map(|&i| pieces[i].size).fold(0.0, f32::max);
        // May grow right up to the next block beside it, and down to the next block below it.
        let grow = spans
            .iter()
            .enumerate()
            .filter(|&(j, s)| j != bi && s.0 > r && s.2 + size >= lo && s.3 - size <= hi)
            .map(|(_, s)| s.0 - 3.0)
            .fold(frame_right, f32::min);
        let below = spans
            .iter()
            .enumerate()
            .filter(|&(j, s)| j != bi && s.2 < lo && s.0 < r && s.1 > l)
            .map(|(_, s)| s.2 + 0.8 * size)
            .fold(limit, f32::max);
        let para = block_paragraph(b, &pieces, paras, grow);
        out.push(Placed { page, para, limit: below, in_cell });
    }
}

/// Every block of `layouts`, page by page.
pub(super) fn placed(layouts: &[PageLayout], idx: &[u32]) -> Vec<Placed> {
    let mut out = Vec::new();
    for (l, &page) in layouts.iter().zip(idx) {
        let floor = l.margin_bottom.min(l.height).max(0.0) * 0.5;
        for band in &l.bands {
            for c in &band.columns {
                let mut run: Vec<&Paragraph> = Vec::new();
                for it in &c.items {
                    match it {
                        Item::Para(p) if p.frame.is_some() => run.push(p),
                        Item::Para(_) => {}
                        Item::Table { table, .. } => {
                            rebuild(page, &run, c.right, table.rect.top + 1.0, false, &mut out);
                            run.clear();
                            for cell in &table.cells {
                                let ps: Vec<&Paragraph> =
                                    cell.paragraphs.iter().filter(|p| p.frame.is_some()).collect();
                                let Some(f) = ps.first().and_then(|p| p.frame) else { continue };
                                let floor = f.bottom_limit.unwrap_or(f32::MIN);
                                rebuild(page, &ps, f.right, floor, true, &mut out);
                            }
                        }
                        Item::Pictures { row, .. } => {
                            let top = row.iter().map(|r| r.rect.top).fold(floor, f32::max);
                            rebuild(page, &run, c.right, top + 1.0, false, &mut out);
                            run.clear();
                        }
                    }
                }
                rebuild(page, &run, c.right, floor, false, &mut out);
            }
        }
    }
    out
}

/// What was written, for checking the saved file and for the report.
#[derive(Default)]
pub(super) struct Written {
    /// Per page: the text of every line written (checked in the saved file).
    pub lines: HashMap<u32, Vec<String>>,
    /// Paragraphs made smaller to fit.
    pub shrunk: usize,
    /// Paragraphs that still run past their space at the smallest size.
    pub overflowing: usize,
    /// Paragraphs whose original text could not be taken off the page (it sits inside a form
    /// object); the translation was written over it on a white background.
    pub covered: usize,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct FontKey {
    family: u8, // 0 sans, 1 serif, 2 mono
    bold: bool,
    italic: bool,
}

fn font_key(r: &Run) -> FontKey {
    let n = r.font.to_lowercase();
    let family = if n.contains("mono") || n.contains("courier") || n.contains("consol") {
        2
    } else if n.contains("times")
        || n.contains("georgia")
        || n.contains("garamond")
        || n.contains("cambria")
        || n.contains("minion")
        || n.contains("palatino")
        || n.contains("book")
        || (n.contains("serif") && !n.contains("sans"))
    {
        1
    } else {
        0
    };
    let bold = r.bold || n.contains("bold") || n.contains("black") || n.contains("heavy");
    let italic = r.italic || n.contains("italic") || n.contains("oblique");
    FontKey { family, bold, italic }
}

fn token(doc: &mut PdfDocument, k: FontKey) -> PdfFontToken {
    let f = doc.fonts_mut();
    match (k.family, k.bold, k.italic) {
        (2, true, true) => f.courier_bold_oblique(),
        (2, true, false) => f.courier_bold(),
        (2, false, true) => f.courier_oblique(),
        (2, false, false) => f.courier(),
        (1, true, true) => f.times_bold_italic(),
        (1, true, false) => f.times_bold(),
        (1, false, true) => f.times_italic(),
        (1, false, false) => f.times_roman(),
        (_, true, true) => f.helvetica_bold_oblique(),
        (_, true, false) => f.helvetica_bold(),
        (_, false, true) => f.helvetica_oblique(),
        _ => f.helvetica(),
    }
}

/// `t` with every character the standard fonts can show (their WinAnsi set): others become
/// their closest equivalent (a letter without its accent, a plain dash or quote) or are left out,
/// so what is written is exactly what the page then holds.
pub(super) fn standard_text(t: &str) -> String {
    use unicode_normalization::UnicodeNormalization;
    const EXTRA: &str = "€‚ƒ„…†‡ˆ‰Š‹ŒŽ‘’“”•–—˜™š›œžŸ";
    let ok = |c: char| (' '..='~').contains(&c) || ('\u{a0}'..='\u{ff}').contains(&c) || EXTRA.contains(c);
    if t.chars().all(ok) {
        return t.to_string();
    }
    let t: String = t.nfc().collect();
    let mut out = String::with_capacity(t.len());
    for c in t.chars() {
        if ok(c) {
            out.push(c);
            continue;
        }
        let near = match c {
            '\u{2010}'..='\u{2012}' | '\u{2212}' => Some('-'),
            '\u{2032}' | '\u{201b}' => Some('\''),
            '\u{2033}' | '\u{201f}' => Some('"'),
            '\u{2002}'..='\u{200a}' | '\u{202f}' | '\u{205f}' => Some(' '),
            '\u{2044}' => Some('/'),
            _ => None,
        };
        if let Some(n) = near {
            out.push(n);
            continue;
        }
        // A letter with marks the fonts lack: its base letter (and any precomposed form they have).
        let nfc: String = std::iter::once(c).nfc().collect();
        if nfc.chars().all(ok) {
            out.push_str(&nfc);
        } else {
            out.extend(std::iter::once(c).nfd().filter(|&d| ok(d) && d != ' '));
        }
    }
    out
}

/// Space width per point of font size (standard font metrics).
fn space_em(k: FontKey) -> f32 {
    match k.family {
        2 => 0.6,
        1 => 0.25,
        _ => 0.278,
    }
}

struct Measure<'d, 'a> {
    doc: &'d PdfDocument<'a>,
    tokens: HashMap<FontKey, PdfFontToken>,
    cache: HashMap<(FontKey, String), f32>,
}

impl Measure<'_, '_> {
    /// Advance width of `text` at 1 pt.
    fn width(&mut self, k: FontKey, text: &str) -> f32 {
        if let Some(w) = self.cache.get(&(k, text.to_string())) {
            return *w;
        }
        let tok = self.tokens[&k];
        // Measured between two marks so side bearings do not count (ink bounds are not advances).
        let w = (|| -> Option<f32> {
            let a = PdfPageTextObject::new(self.doc, format!("|{text}|"), tok, PdfPoints::new(100.0)).ok()?;
            let b = PdfPageTextObject::new(self.doc, "||", tok, PdfPoints::new(100.0)).ok()?;
            Some(((a.width().ok()?.value - b.width().ok()?.value) / 100.0).max(0.0))
        })()
        .unwrap_or(0.5 * text.chars().count() as f32);
        self.cache.insert((k, text.to_string()), w);
        w
    }
}

struct LineOut {
    /// (word index range) of the words on the line.
    words: std::ops::Range<usize>,
    width: f32,
}

/// Greedy wrap at `scale` of the paragraph's sizes. Returns lines and whether every word fitted.
fn wrap(words: &[StyledWord], widths: &[f32], spaces: &[f32], first_width: f32, width: f32) -> (Vec<LineOut>, bool) {
    let mut lines = Vec::new();
    let mut start = 0usize;
    let mut cur = 0.0f32;
    let mut all_fit = true;
    for i in 0..words.len() {
        let avail = if lines.is_empty() { first_width } else { width };
        let add = if i == start { widths[i] } else { spaces[i] + widths[i] };
        if i > start && cur + add > avail + 0.01 {
            lines.push(LineOut { words: start..i, width: cur });
            start = i;
            cur = widths[i];
        } else {
            cur += add;
        }
        if widths[i] > (if lines.is_empty() { first_width } else { width }) + 0.01 {
            all_fit = false;
        }
    }
    if start < words.len() {
        lines.push(LineOut { words: start..words.len(), width: cur });
    }
    (lines, all_fit)
}

/// Take the translated paragraphs' text off `doc`'s pages and write `words[i]` in place of
/// `placed[i]` (paragraphs with `None` are left as they are).
pub(super) fn write_in_place(
    doc: &mut PdfDocument,
    placed: &[Placed],
    words: &[Option<Vec<StyledWord>>],
    cancelled: &dyn Fn() -> bool,
) -> PwResult<Written> {
    let err = |e: PdfiumError| map_pdfium_error(&e, "translate");
    let mut written = Written::default();
    let mut by_page: Vec<u32> = placed.iter().map(|p| p.page).collect();
    by_page.sort_unstable();
    by_page.dedup();
    // Fonts first (they borrow the document mutably).
    let mut tokens = HashMap::new();
    for (p, w) in placed.iter().zip(words) {
        if w.is_some() {
            for r in &p.para.runs {
                let k = font_key(r);
                if let std::collections::hash_map::Entry::Vacant(e) = tokens.entry(k) {
                    e.insert(token(doc, k));
                }
            }
        }
    }
    let mut measure = Measure { doc, tokens, cache: HashMap::new() };
    // Lay out every paragraph first (text objects are created per page afterwards).
    struct Piece {
        page: u32,
        text: String,
        key: FontKey,
        size: f32,
        color: [u8; 3],
        x: f32,
        y: f32,
    }
    let mut pieces: Vec<Piece> = Vec::new();
    let mut areas: HashMap<u32, Vec<(f32, f32, f32, f32)>> = HashMap::new();
    for (pl, w) in placed.iter().zip(words) {
        let (Some(words), Some(frame)) = (w, pl.para.frame) else { continue };
        if words.is_empty() || pl.para.runs.is_empty() {
            continue;
        }
        if cancelled() {
            return Err(PwError::cancelled());
        }
        let p = &pl.para;
        let r = p.rect;
        areas.entry(pl.page).or_default().push((r.left - 1.0, r.bottom - 1.0, r.right + 1.0, r.top + 1.0));
        let base_size = p.runs.iter().map(|r| r.size).fold(0.0, f32::max).max(1.0);
        let keys: Vec<FontKey> = words.iter().map(|w| font_key(&p.runs[w.style])).collect();
        let unit: Vec<f32> = words.iter().zip(&keys).map(|(w, k)| measure.width(*k, &standard_text(&w.text))).collect();
        let rel: Vec<f32> = words.iter().map(|w| p.runs[w.style].size / base_size).collect();
        // Horizontal room.
        let (first_left, rest_left, right) = match p.align {
            Align::Center => {
                let cx = (r.left + r.right) / 2.0;
                let half = (cx - frame.left).min(frame.right - cx).max((r.right - r.left) / 2.0);
                (cx - half, cx - half, cx + half)
            }
            Align::Right => (frame.left, frame.left, r.right.max(frame.left + 1.0)),
            _ => {
                let rest = frame.left + p.indent;
                let right =
                    if frame.lines >= 2 && !pl.in_cell { r.right.max(rest + 1.0) } else { frame.right.max(r.right) };
                (rest + p.first_line, rest, right)
            }
        };
        let pitch0 = if p.pitch > 0.0 { p.pitch } else { base_size * 1.2 };
        let mut s = 1.0f32;
        let chosen = loop {
            let size = base_size * s;
            let widths: Vec<f32> = unit.iter().zip(&rel).map(|(u, q)| u * size * q).collect();
            let spaces: Vec<f32> = keys.iter().zip(&rel).map(|(k, q)| space_em(*k) * size * q).collect();
            let (lines, all_fit) = wrap(words, &widths, &spaces, right - first_left, right - rest_left);
            let pitch = pitch0 * s;
            let last_base = frame.baseline - pitch * (lines.len().saturating_sub(1)) as f32;
            let fits = all_fit && (lines.len() <= frame.lines || last_base - 0.25 * size >= pl.limit);
            if fits || s <= 0.62 {
                if !fits {
                    written.overflowing += 1;
                }
                if s < 0.999 {
                    written.shrunk += 1;
                }
                break Some((s, lines, widths, spaces, pitch));
            }
            s -= 0.04;
        };
        let Some((s, lines, widths, spaces, pitch)) = chosen else { continue };
        let page_lines = written.lines.entry(pl.page).or_default();
        let n = lines.len();
        for (li, line) in lines.iter().enumerate() {
            let left = if li == 0 { first_left } else { rest_left };
            let avail = right - left;
            let gaps = line.words.len().saturating_sub(1);
            let (mut x, extra) = match p.align {
                Align::Center => (left + (avail - line.width) / 2.0, 0.0),
                Align::Right => (right - line.width, 0.0),
                Align::Justify if li + 1 < n && gaps > 0 && !pl.in_cell => {
                    (left, ((avail - line.width) / gaps as f32).max(0.0))
                }
                _ => (left, 0.0),
            };
            let y = frame.baseline - pitch * li as f32;
            // Words of one style on a line are written as one piece.
            let mut i = line.words.start;
            let mut text_line = String::new();
            while i < line.words.end {
                let style = words[i].style;
                let mut j = i;
                let mut text = String::new();
                let mut w = 0.0;
                while j < line.words.end && words[j].style == style && (extra == 0.0 || j == i) {
                    if j > i {
                        text.push(' ');
                        w += spaces[j];
                    }
                    text.push_str(&standard_text(&words[j].text));
                    w += widths[j];
                    j += 1;
                }
                let run = &p.runs[style];
                pieces.push(Piece {
                    page: pl.page,
                    text: text.clone(),
                    key: keys[i],
                    size: run.size * s,
                    color: run.color,
                    x,
                    y,
                });
                if !text_line.is_empty() {
                    text_line.push(' ');
                }
                text_line.push_str(&text);
                x += w + if j < line.words.end { spaces[j] + extra } else { 0.0 };
                i = j;
            }
            page_lines.push(text_line);
        }
    }
    let tokens = measure.tokens;
    for page_index in by_page {
        let Some(rects) = areas.get(&page_index) else { continue };
        let mut page = doc.pages().get(page_index as PdfPageIndex).map_err(err)?;
        // Take off the text that sits in the translated paragraphs' places.
        let mut remove = Vec::new();
        let mut invisible = Vec::new();
        for (i, o) in page.objects().iter().enumerate() {
            let PdfPageObject::Text(t) = &o else { continue };
            let Ok(q) = o.bounds() else { continue };
            let b = q.to_rect();
            let (cx, cy) = ((b.left().value + b.right().value) / 2.0, (b.bottom().value + b.top().value) / 2.0);
            if rects.iter().any(|&(l, bo, r, tp)| cx >= l && cx <= r && cy >= bo && cy <= tp) {
                if t.render_mode() == PdfPageTextRenderMode::Invisible {
                    invisible.push((b.left().value, b.bottom().value, b.right().value, b.top().value));
                }
                remove.push(i);
            }
        }
        for &i in remove.iter().rev() {
            page.objects_mut().remove_object_at_index(i as PdfPageObjectIndex).map_err(err)?;
        }
        // Recognized text over a scan: the scanned words are covered so only the translation shows.
        let cover_all = !invisible.is_empty();
        if cover_all {
            for &(l, b, r, t) in rects {
                let rect = PdfRect::new_from_values(b, l, t, r);
                let obj = PdfPagePathObject::new_rect(doc, rect, None, None, Some(PdfColor::new(255, 255, 255, 255)))
                    .map_err(err)?;
                page.objects_mut().add_path_object(obj).map_err(err)?;
                written.covered += 1;
            }
        }
        for pc in pieces.iter().filter(|pc| pc.page == page_index) {
            let mut obj =
                PdfPageTextObject::new(doc, &pc.text, tokens[&pc.key], PdfPoints::new(pc.size)).map_err(err)?;
            let [r, g, b] = pc.color;
            obj.set_fill_color(PdfColor::new(r, g, b, 255)).map_err(err)?;
            obj.translate(PdfPoints::new(pc.x), PdfPoints::new(pc.y)).map_err(err)?;
            page.objects_mut().add_text_object(obj).map_err(err)?;
        }
        crate::edit::regenerate_keeping_colours(&mut page).map_err(err)?;
    }
    Ok(written)
}

/// Check the saved PDF: every written line is in its page's text.
pub(super) fn verify_lines(doc: &PdfDocument, written: &Written) -> PwResult<()> {
    let squash = |t: &str| t.chars().filter(|c| !c.is_whitespace()).collect::<String>();
    let everything = PdfRect::new_from_values(-1.0e6, -1.0e6, 1.0e6, 1.0e6);
    for (&page_index, lines) in &written.lines {
        let page = doc.pages().get(page_index as PdfPageIndex).map_err(|e| map_pdfium_error(&e, "page"))?;
        let got = squash(&page.text().map_err(|e| map_pdfium_error(&e, "text"))?.inside_rect(everything));
        for l in lines {
            if !got.contains(&squash(l)) {
                if std::env::var_os("PW_DEBUG_VERIFY").is_some() {
                    eprintln!("missing line: {l:?}\npage text: {got:?}");
                }
                return Err(PwError::new(
                    ErrorCategory::ValidationFailed,
                    "output.verify",
                    format!(
                        "The translated PDF failed verification (page {} is missing translated text).",
                        page_index + 1
                    ),
                ));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::standard_text;

    #[test]
    fn text_is_limited_to_what_the_standard_fonts_show() {
        assert_eq!(standard_text("Größe – „Zins“ 5 €"), "Größe – „Zins“ 5 €");
        assert_eq!(standard_text("a\u{2011}b \u{2212}3 x\u{202f}%"), "a-b -3 x %");
        assert_eq!(standard_text("Łódź ćma"), "ódz cma");
        assert_eq!(standard_text("e\u{301}te\u{301}"), "été");
        assert_eq!(standard_text("日本"), "");
    }
}
