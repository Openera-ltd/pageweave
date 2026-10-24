//! Page layout analysis for Word export (PRD 06 "PDF → Word"): glyphs → lines → bands of columns →
//! paragraphs and pictures in reading order. Pure (no engine calls), so it is unit tested on
//! synthetic pages. Coordinates are PDF points in the page's displayed orientation, y upwards.

use super::tables::{find_tables, Rule, Table};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq)]
pub struct Glyph {
    pub ch: char,
    pub left: f32,
    pub right: f32,
    pub bottom: f32,
    pub top: f32,
    pub baseline: f32,
    pub size: f32,
    pub font: String,
    pub bold: bool,
    pub italic: bool,
    pub color: [u8; 3],
    /// Inserted by the text engine (spaces, line ends); has no reliable position.
    pub generated: bool,
    /// A hyphen the text engine says ends a line inside a word.
    pub hyphen: bool,
    /// Index into the page's links.
    pub link: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub left: f32,
    pub bottom: f32,
    pub right: f32,
    pub top: f32,
}

impl Rect {
    pub fn width(&self) -> f32 {
        self.right - self.left
    }
    pub fn height(&self) -> f32 {
        self.top - self.bottom
    }
    fn cx(&self) -> f32 {
        (self.left + self.right) / 2.0
    }
    fn overlaps(&self, o: &Rect) -> bool {
        self.left < o.right && o.left < self.right && self.bottom < o.top && o.bottom < self.top
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Picture {
    /// Index into the page's picture data.
    pub id: usize,
    pub rect: Rect,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PageInput {
    pub width: f32,
    pub height: f32,
    pub glyphs: Vec<Glyph>,
    pub pictures: Vec<Picture>,
    /// Drawn horizontal and vertical lines (for ruled tables).
    pub rules: Vec<Rule>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    Left,
    Center,
    Right,
    Justify,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VertAlign {
    Baseline,
    Superscript,
    Subscript,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Run {
    pub text: String,
    pub font: String,
    pub size: f32,
    pub bold: bool,
    pub italic: bool,
    pub color: [u8; 3],
    pub link: Option<usize>,
    pub vert: VertAlign,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Paragraph {
    pub runs: Vec<Run>,
    pub align: Align,
    /// Left indent from the column's left edge.
    pub indent: f32,
    /// First line relative to `indent` (negative: hanging).
    pub first_line: f32,
    pub space_before: f32,
    /// Distance between baselines (exact line spacing), points.
    pub pitch: f32,
    /// 1–3 for headings.
    pub heading: Option<u8>,
    /// Left tab stops (from the column's left edge) for pieces of a line set apart, such as
    /// table cells.
    pub tabs: Vec<f32>,
    /// Where its lines are on the page (for slide export).
    pub rect: Rect,
    /// Where each character of the runs' text is on the page, in order (read-aloud highlighting).
    /// Spaces joining lines take the position of the character before them.
    pub boxes: Vec<Rect>,
    /// Where the lines sit on the page (paragraphs read from a page; `None` for made-up ones), for
    /// writing new text in their place (translation into a PDF).
    pub frame: Option<Frame>,
    /// Each line's baseline, top to bottom (paragraphs read from a page).
    pub line_baselines: Vec<f32>,
    /// Drawn around it on the page: a shaded box, its coloured left bar, a line below.
    pub deco: Deco,
}

/// Colours drawn with a paragraph (Word: shading and borders).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Deco {
    pub shade: Option<[u8; 3]>,
    pub left_bar: Option<([u8; 3], f32)>,
    pub rule_below: Option<([u8; 3], f32)>,
}

/// A filled rectangle or a drawn horizontal line on the page, with its colour.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Paint {
    pub rect: Rect,
    pub color: [u8; 3],
    /// A stroked line (`rect` is the line with its width).
    pub line: bool,
}

/// The space a paragraph's lines take on the page.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Frame {
    /// First line's baseline.
    pub baseline: f32,
    /// The column (or table cell) the lines may use.
    pub left: f32,
    pub right: f32,
    /// How many lines it had.
    pub lines: usize,
    /// Lowest point new lines may reach (a table cell's bottom), when known.
    pub bottom_limit: Option<f32>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RowPicture {
    /// Where it is on the page (for slide export).
    pub rect: Rect,
    pub id: usize,
    pub width: f32,
    pub height: f32,
    /// From the column's left edge.
    pub offset: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Item {
    Para(Paragraph),
    /// Pictures side by side on one row (one paragraph).
    Pictures {
        row: Vec<RowPicture>,
        space_before: f32,
    },
    /// A ruled table.
    Table {
        table: TableItem,
        indent: f32,
        space_before: f32,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct TableCell {
    pub row: usize,
    pub col: usize,
    pub row_span: usize,
    pub col_span: usize,
    pub paragraphs: Vec<Paragraph>,
    /// Background colour.
    pub fill: Option<[u8; 3]>,
    /// Edges drawn without a line: [above, below] (the gaps between framed boxes).
    pub open: [bool; 2],
    /// Text centred between the cell's top and bottom.
    pub middle: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TableItem {
    /// Column widths, points.
    pub widths: Vec<f32>,
    /// Row heights, points (minimum: rows grow to fit their text).
    pub heights: Vec<f32>,
    pub cells: Vec<TableCell>,
    /// Where it is on the page (for slide export).
    pub rect: Rect,
    /// Colour of its lines, when drawn in one colour.
    pub border: Option<[u8; 3]>,
}

/// Inner space between a cell's lines and its text, points.
pub const CELL_PAD: f32 = 2.0;
/// Tables travel through the column flow as placeholder pictures with ids from here up.
const TABLE_ID_BASE: usize = usize::MAX / 2;

#[derive(Debug, Clone, PartialEq)]
pub struct Column {
    pub left: f32,
    pub right: f32,
    pub items: Vec<Item>,
}

/// A horizontal slice of the page with one or more columns.
#[derive(Debug, Clone, PartialEq)]
pub struct Band {
    pub columns: Vec<Column>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PageLayout {
    pub width: f32,
    pub height: f32,
    pub margin_left: f32,
    pub margin_right: f32,
    pub margin_top: f32,
    pub margin_bottom: f32,
    pub bands: Vec<Band>,
    /// Pictures behind text, placed absolutely on the page.
    pub backgrounds: Vec<Picture>,
    /// Pictures with text beside them, placed on the page with text wrapping around.
    pub floating: Vec<Picture>,
    /// Pictures narrower columns could not hold at full size (shrunk to fit).
    pub shrunk_pictures: usize,
    /// Ruled tables found on the page (grids, for reporting which drawn lines were used).
    pub tables: Vec<Table>,
}

#[derive(Debug, Clone)]
struct Line {
    glyphs: Vec<Glyph>,
    rect: Rect,
    baseline: f32,
    size: f32,
    /// Left edges of pieces joined onto this line after a wide gap (tab stops).
    tabs: Vec<f32>,
}

impl Line {
    fn new(g: Glyph) -> Line {
        Line { rect: rect_of(&g), baseline: g.baseline, size: g.size, glyphs: vec![g], tabs: Vec::new() }
    }
    fn push(&mut self, g: Glyph) {
        if !g.generated {
            let r = rect_of(&g);
            self.rect = Rect {
                left: self.rect.left.min(r.left),
                bottom: self.rect.bottom.min(r.bottom),
                right: self.rect.right.max(r.right),
                top: self.rect.top.max(r.top),
            };
            self.size = self.size.max(g.size);
        }
        self.glyphs.push(g);
    }
    fn text(&self) -> String {
        self.glyphs.iter().map(|g| g.ch).collect()
    }
}

fn rect_of(g: &Glyph) -> Rect {
    Rect { left: g.left, bottom: g.bottom, right: g.right, top: g.top }
}

fn space(after: &Glyph) -> Glyph {
    Glyph { ch: ' ', generated: true, hyphen: false, left: after.right, right: after.right, ..after.clone() }
}

/// Group glyphs (in the text engine's order) into visual lines with word spaces.
fn build_lines(glyphs: &[Glyph]) -> Vec<Line> {
    let mut lines: Vec<Line> = Vec::new();
    let mut cur: Option<Line> = None;
    let finish = |cur: &mut Option<Line>, lines: &mut Vec<Line>| {
        if let Some(mut l) = cur.take() {
            while l.glyphs.last().is_some_and(|g| g.ch.is_whitespace()) {
                l.glyphs.pop();
            }
            while l.glyphs.first().is_some_and(|g| g.ch.is_whitespace()) {
                l.glyphs.remove(0);
            }
            if l.glyphs.iter().any(|g| !g.generated) {
                lines.push(l);
            }
        }
    };
    for g in glyphs {
        if g.ch == '\r' || g.ch == '\n' {
            finish(&mut cur, &mut lines);
            continue;
        }
        if g.generated {
            // Generated spaces keep words apart; their position is not used.
            if let Some(l) = cur.as_mut() {
                if g.ch.is_whitespace() && !l.glyphs.last().is_some_and(|x| x.ch.is_whitespace()) {
                    let last = l.glyphs.last().cloned().expect("line has glyphs");
                    l.glyphs.push(space(&last));
                }
            }
            continue;
        }
        if g.size <= 0.0 || !g.left.is_finite() || !g.baseline.is_finite() {
            continue;
        }
        let breaks = cur.as_ref().is_none_or(|l| {
            let tol = 0.5 * g.size.max(l.size);
            (g.baseline - l.baseline).abs() > tol
                || g.left < l.rect.right - 0.6 * g.size.max(1.0)
                || g.left - l.rect.right > 1.5 * g.size.max(l.size)
        });
        if breaks {
            finish(&mut cur, &mut lines);
            cur = Some(Line::new(g.clone()));
            continue;
        }
        let l = cur.as_mut().expect("current line");
        let last = l.glyphs.last().cloned().expect("line has glyphs");
        if g.left - l.rect.right > 0.2 * g.size && !last.ch.is_whitespace() && !g.ch.is_whitespace() {
            l.glyphs.push(space(&last));
        }
        if g.ch.is_whitespace() && last.ch.is_whitespace() {
            continue;
        }
        l.push(g.clone());
    }
    finish(&mut cur, &mut lines);
    lines
}

/// Most common glyph size (by count, 0.5 pt steps).
fn body_size(lines: &[Line]) -> f32 {
    let mut counts: HashMap<i32, usize> = HashMap::new();
    for l in lines {
        for g in l.glyphs.iter().filter(|g| !g.generated) {
            *counts.entry((g.size * 2.0).round() as i32).or_default() += 1;
        }
    }
    counts.into_iter().max_by_key(|&(s, n)| (n, -s)).map(|(s, _)| s as f32 / 2.0).unwrap_or(12.0)
}

/// Vertical gaps between columns: x ranges no line of a column-like width crosses.
fn gutters(lines: &[Line], left: f32, right: f32) -> Vec<(f32, f32)> {
    let width = right - left;
    if width < 100.0 || lines.len() < 8 {
        return Vec::new();
    }
    let narrow: Vec<&Line> = lines.iter().filter(|l| l.rect.width() < 0.55 * width).collect();
    let bins = width.ceil() as usize + 1;
    let mut cover = vec![0u32; bins];
    for l in &narrow {
        let a = ((l.rect.left - left).floor().max(0.0) as usize).min(bins - 1);
        let b = ((l.rect.right - left).ceil().max(0.0) as usize).min(bins - 1);
        for c in &mut cover[a..=b] {
            *c += 1;
        }
    }
    // A few lines may cross a gap (a centred subtitle, a caption): they become full-width bands.
    let tol = (narrow.len() as u32 / 20).max(1);
    let mut out = Vec::new();
    let mut i = 0;
    while i < bins {
        if cover[i] <= tol {
            let start = i;
            while i < bins && cover[i] <= tol {
                i += 1;
            }
            let (g0, g1) = (left + start as f32, left + i as f32);
            // Column gaps are narrow; a wide empty stretch is a label/value or form layout.
            if start > 0 && i < bins && g1 - g0 >= 10.0 && g1 - g0 <= (0.12 * width).max(48.0) {
                out.push((g0, g1));
            }
        } else {
            i += 1;
        }
    }
    // Every column must look like prose: enough lines that mostly fill it.
    let edges: Vec<f32> =
        std::iter::once(left).chain(out.iter().flat_map(|&(a, b)| [a, b])).chain(std::iter::once(right)).collect();
    let ok = edges.chunks(2).all(|c| {
        let (c0, c1) = (c[0], c[1]);
        let mut widths: Vec<f32> = narrow
            .iter()
            .filter(|l| l.rect.left >= c0 - 1.0 && l.rect.right <= c1 + 1.0)
            .map(|l| l.rect.width())
            .collect();
        if widths.len() < 4 || c1 - c0 < 0.2 * width {
            return false;
        }
        widths.sort_by(f32::total_cmp);
        widths[widths.len() / 2] >= 0.5 * (c1 - c0)
    });
    if ok {
        out
    } else {
        Vec::new()
    }
}

fn starts_list_item(text: &str) -> bool {
    let t = text.trim_start();
    let mut chars = t.chars();
    match chars.next() {
        Some('•' | '◦' | '▪' | '■' | '●' | '–' | '-' | '*') => chars.next() == Some(' '),
        Some(c) if c.is_ascii_digit() => {
            let digits: String = t.chars().take_while(char::is_ascii_digit).collect();
            let rest = &t[digits.len()..];
            digits.len() <= 3 && (rest.starts_with(". ") || rest.starts_with(") "))
        }
        _ => false,
    }
}

/// Join pieces of one visual line that a wide gap set apart (table cells, label and value) into
/// one line with tab stops. Lines arrive top to bottom.
fn join_pieces(lines: Vec<Line>) -> Vec<Line> {
    let mut sorted = lines;
    sorted.sort_by(|a, b| b.baseline.total_cmp(&a.baseline).then(a.rect.left.total_cmp(&b.rect.left)));
    let mut out: Vec<Line> = Vec::new();
    for l in sorted {
        if let Some(prev) = out.last_mut() {
            let tol = 0.3 * prev.size.max(l.size);
            if (prev.baseline - l.baseline).abs() < tol && l.rect.left > prev.rect.right {
                let last = prev.glyphs.last().cloned().expect("line has glyphs");
                prev.glyphs.push(Glyph { ch: '\t', generated: true, hyphen: false, ..last });
                prev.tabs.push(l.rect.left);
                prev.rect = Rect {
                    left: prev.rect.left,
                    bottom: prev.rect.bottom.min(l.rect.bottom),
                    right: l.rect.right,
                    top: prev.rect.top.max(l.rect.top),
                };
                prev.size = prev.size.max(l.size);
                prev.glyphs.extend(l.glyphs);
                continue;
            }
        }
        out.push(l);
    }
    out
}

/// Width of a line's first word.
fn first_word_width(l: &Line) -> f32 {
    let mut right = l.rect.left;
    for g in &l.glyphs {
        if g.ch.is_whitespace() {
            break;
        }
        if !g.generated {
            right = g.right;
        }
    }
    right - l.rect.left
}

/// Split a column's lines (top to bottom) into paragraphs. Lines join only where the text
/// wrapped: the next line's first word would not have fitted on the line before.
fn paragraphs(lines: Vec<Line>, col_left: f32, col_right: f32) -> Vec<Vec<Line>> {
    // The column's right edge includes some slack (see `layout_page`).
    let fit_right = col_right - (0.02 * (col_right - col_left)).max(3.0);
    // The column's usual line pitch, for telling paragraph gaps from line spacing.
    let usual = median(
        lines
            .windows(2)
            .map(|w| w[0].baseline - w[1].baseline)
            .filter(|&d| d > 0.0 && d < 3.0 * w_size(&lines))
            .collect(),
    );
    let mut out: Vec<Vec<Line>> = Vec::new();
    for l in lines {
        let new = match out.last() {
            None => true,
            Some(p) => {
                let prev = p.last().expect("paragraph has lines");
                let gap = prev.baseline - l.baseline;
                let size = prev.size.max(l.size);
                let pitch = usual.filter(|&u| u >= 0.9 * size).unwrap_or(1.2 * size);
                let body_left = if p.len() >= 2 { p[1].rect.left } else { prev.rect.left };
                let space = 0.3 * l.size;
                let wrapped = prev.rect.right + space + first_word_width(&l) > fit_right - 0.5;
                gap <= 0.0
                    || !prev.tabs.is_empty()
                    || !l.tabs.is_empty()
                    || gap > 1.3 * pitch
                    || (l.size - prev.size).abs() > 0.15 * prev.size
                    || starts_list_item(&l.text())
                    || !wrapped
                    || (p.len() >= 2 && l.rect.left > body_left + size)
            }
        };
        if new {
            out.push(vec![l]);
        } else {
            out.last_mut().expect("paragraph").push(l);
        }
    }
    out
}

/// Typical glyph size of some lines.
fn w_size(lines: &[Line]) -> f32 {
    median(lines.iter().map(|l| l.size).collect()).unwrap_or(12.0)
}

fn same_style(a: &Glyph, b: &Glyph) -> bool {
    a.font == b.font
        && (a.size - b.size).abs() < 0.25
        && a.bold == b.bold
        && a.italic == b.italic
        && a.color == b.color
        && a.link == b.link
}

fn runs_of(lines: &[Line]) -> (Vec<Run>, Vec<Rect>) {
    let mut runs: Vec<Run> = Vec::new();
    let mut boxes: Vec<Rect> = Vec::new();
    let mut last_style: Option<(Glyph, VertAlign)> = None;
    for (k, line) in lines.iter().enumerate() {
        let n = line.glyphs.len();
        for (i, g) in line.glyphs.iter().enumerate() {
            // A soft line-end hyphen joins the word across lines.
            if g.hyphen && i + 1 == n {
                continue;
            }
            let shift = g.baseline - line.baseline;
            let vert = if g.generated {
                last_style.as_ref().map(|s| s.1).unwrap_or(VertAlign::Baseline)
            } else if shift > 0.25 * line.size && g.size < 0.85 * line.size {
                VertAlign::Superscript
            } else if shift < -0.15 * line.size && g.size < 0.85 * line.size {
                VertAlign::Subscript
            } else {
                VertAlign::Baseline
            };
            let style =
                if g.generated { last_style.as_ref().map(|s| s.0.clone()).unwrap_or(g.clone()) } else { g.clone() };
            let joins = runs.last().is_some_and(|r: &Run| {
                last_style.as_ref().is_some_and(|(s, v)| same_style(s, &style) && *v == vert) && !r.text.is_empty()
            });
            boxes.push(rect_of(g));
            if joins {
                runs.last_mut().expect("run").text.push(g.ch);
            } else {
                runs.push(Run {
                    text: g.ch.to_string(),
                    font: style.font.clone(),
                    size: style.size,
                    bold: style.bold,
                    italic: style.italic,
                    color: style.color,
                    link: style.link,
                    vert,
                });
            }
            last_style = Some((style, vert));
        }
        // Lines of a paragraph are joined by a space unless a soft hyphen joined the word.
        if k + 1 < lines.len() {
            let joined_word = line.glyphs.last().is_some_and(|g| g.hyphen);
            if !joined_word {
                if let Some(r) = runs.last_mut() {
                    if !r.text.ends_with(' ') {
                        r.text.push(' ');
                        let at = boxes.last().copied().unwrap_or(line.rect);
                        boxes.push(Rect { left: at.right, right: at.right, ..at });
                    }
                }
            }
        }
    }
    (runs, boxes)
}

fn median(mut v: Vec<f32>) -> Option<f32> {
    if v.is_empty() {
        return None;
    }
    v.sort_by(f32::total_cmp);
    Some(v[v.len() / 2])
}

fn alignment(lines: &[Line], col_left: f32, col_right: f32) -> Align {
    let col_w = col_right - col_left;
    let col_c = (col_left + col_right) / 2.0;
    let tol: f32 = 3.0;
    let lefts: Vec<f32> = lines.iter().map(|l| l.rect.left).collect();
    let rights: Vec<f32> = lines.iter().map(|l| l.rect.right).collect();
    let spread = |v: &[f32]| v.iter().copied().fold(f32::MIN, f32::max) - v.iter().copied().fold(f32::MAX, f32::min);
    let centered = lines.iter().all(|l| (l.rect.cx() - col_c).abs() < tol.max(0.02 * col_w));
    if lines.len() == 1 {
        let l = &lines[0];
        if centered && l.rect.left - col_left > 0.15 * col_w {
            return Align::Center;
        }
        if (col_right - l.rect.right) < tol && l.rect.left - col_left > 0.3 * col_w {
            return Align::Right;
        }
        return Align::Left;
    }
    if centered && spread(&lefts) > 2.0 * tol {
        return Align::Center;
    }
    let body = &rights[..rights.len() - 1];
    if lines.len() >= 3 && spread(body) < tol && spread(&lefts[1..]) < tol {
        return Align::Justify;
    }
    if spread(&rights) < tol && spread(&lefts) > 2.0 * tol {
        return Align::Right;
    }
    Align::Left
}

struct Pending {
    top: f32,
    bottom: f32,
    item: Item,
    /// Baseline of the last line (paragraphs), for the next item's spacing.
    last_baseline: Option<f32>,
    first_baseline: Option<f32>,
}

/// One paragraph from its lines: alignment, indents, line pitch, heading level, tab stops and
/// runs. Returns it with its top and bottom.
fn make_paragraph(
    p: &[Line],
    col_left: f32,
    col_right: f32,
    body: f32,
    heading_sizes: &[f32],
) -> (Paragraph, f32, f32) {
    let size = p.iter().map(|l| l.size).fold(0.0, f32::max);
    let left = if p.len() >= 2 { p[1..].iter().map(|l| l.rect.left).fold(f32::MAX, f32::min) } else { p[0].rect.left };
    let pitch = median(p.windows(2).map(|w| w[0].baseline - w[1].baseline).collect())
        .filter(|&d| d >= size)
        .unwrap_or(size * 1.2);
    let chars: usize = p.iter().map(|l| l.glyphs.len()).sum();
    let avg_size = {
        let real: Vec<&Glyph> = p.iter().flat_map(|l| l.glyphs.iter()).filter(|g| !g.generated).collect();
        real.iter().map(|g| g.size).sum::<f32>() / real.len().max(1) as f32
    };
    let heading = (avg_size >= 1.25 * body && p.len() <= 3 && chars <= 200)
        .then(|| heading_sizes.iter().position(|&s| (s - avg_size).abs() < 0.5).map(|i| (i + 1).min(3) as u8))
        .flatten();
    let tabs: Vec<f32> = p[0].tabs.iter().map(|x| (x - col_left).max(0.0)).collect();
    let align = if tabs.is_empty() { alignment(p, col_left, col_right) } else { Align::Left };
    let (indent, first_line) = match align {
        Align::Center | Align::Right => (0.0, 0.0),
        _ => ((left - col_left).max(0.0), p[0].rect.left - left),
    };
    let top = p[0].rect.top;
    let bottom = p.last().map(|l| l.rect.bottom).unwrap_or(top);
    let rect = p.iter().skip(1).fold(p[0].rect, |a, l| Rect {
        left: a.left.min(l.rect.left),
        bottom: a.bottom.min(l.rect.bottom),
        right: a.right.max(l.rect.right),
        top: a.top.max(l.rect.top),
    });
    (
        {
            let (runs, boxes) = runs_of(p);
            let frame = Some(Frame {
                baseline: p[0].baseline,
                left: col_left,
                right: col_right,
                lines: p.len(),
                bottom_limit: None,
            });
            let line_baselines = p.iter().map(|l| l.baseline).collect();
            Paragraph {
                runs,
                align,
                indent,
                first_line,
                space_before: 0.0,
                pitch,
                heading,
                tabs,
                rect,
                boxes,
                frame,
                line_baselines,
                deco: Deco::default(),
            }
        },
        top,
        bottom,
    )
}

/// The text of one table: each cell's glyphs (in engine order) become that cell's paragraphs.
fn table_item(t: &Table, cell_glyphs: Vec<Vec<Glyph>>, body: f32) -> TableItem {
    let widths: Vec<f32> = t.xs.windows(2).map(|w| w[1] - w[0]).collect();
    let heights: Vec<f32> = t.ys.windows(2).map(|w| w[0] - w[1]).collect();
    let cells = t
        .cells
        .iter()
        .zip(cell_glyphs)
        .map(|(c, glyphs)| {
            let (c0, c1) = (t.xs[c.col] + CELL_PAD, t.xs[c.col + c.col_span] - CELL_PAD);
            let mut lines = build_lines(&glyphs);
            lines.sort_by(|a, b| b.baseline.total_cmp(&a.baseline));
            let middle = match (lines.first(), lines.last()) {
                (Some(f), Some(l)) => {
                    let (above, below) = (t.ys[c.row] - f.rect.top, l.rect.bottom - t.ys[c.row + c.row_span]);
                    let h = t.ys[c.row] - t.ys[c.row + c.row_span];
                    above > 4.0 && below > 4.0 && (above - below).abs() < 0.2 * h
                }
                _ => false,
            };
            let mut paragraphs_out = Vec::new();
            let mut prev_base: Option<f32> = None;
            let cell_bottom = t.ys[c.row + c.row_span] + CELL_PAD;
            for p in paragraphs(join_pieces(lines), c0, c1) {
                let (mut par, _, _) = make_paragraph(&p, c0, c1, body, &[]);
                if let Some(f) = par.frame.as_mut() {
                    f.bottom_limit = Some(cell_bottom);
                }
                if let Some(b) = prev_base {
                    par.space_before = (b - p[0].baseline - par.pitch).max(0.0);
                }
                prev_base = p.last().map(|l| l.baseline);
                paragraphs_out.push(par);
            }
            TableCell {
                row: c.row,
                col: c.col,
                row_span: c.row_span,
                col_span: c.col_span,
                paragraphs: paragraphs_out,
                fill: None,
                open: c.open,
                middle,
            }
        })
        .collect();
    // Centred only when the whole row is (a one-line cell next to a two-line one is not).
    let mut cells: Vec<TableCell> = cells;
    for r in 0..heights.len() {
        let all = cells.iter().filter(|c| c.row == r && !c.paragraphs.is_empty()).all(|c| c.middle);
        if !all {
            cells.iter_mut().filter(|c| c.row == r).for_each(|c| c.middle = false);
        }
    }
    TableItem {
        widths,
        heights,
        cells,
        rect: Rect { left: t.left(), bottom: t.bottom(), right: t.right(), top: t.top() },
        border: None,
    }
}

fn column_items(
    lines: Vec<Line>,
    pictures: Vec<Picture>,
    col: (f32, f32),
    body: f32,
    heading_sizes: &[f32],
    start: ColumnStart,
    tables: &[TableItem],
) -> (Vec<Item>, usize, Vec<Picture>, Option<End>) {
    let (col_left, col_right) = col;
    let line_rects: Vec<Rect> = lines.iter().map(|l| l.rect).collect();
    let col_w = (col_right - col_left).max(1.0);
    let mut pending: Vec<Pending> = Vec::new();
    for p in paragraphs(join_pieces(lines), col_left, col_right) {
        let (par, top, bottom) = make_paragraph(&p, col_left, col_right, body, heading_sizes);
        pending.push(Pending {
            top,
            bottom,
            first_baseline: Some(p[0].baseline),
            last_baseline: p.last().map(|l| l.baseline),
            item: Item::Para(par),
        });
    }
    // Tables (placeholder pictures) take their place in the flow.
    let (table_pics, pictures): (Vec<Picture>, Vec<Picture>) =
        pictures.into_iter().partition(|p| p.id >= TABLE_ID_BASE);
    for tp in table_pics {
        let Some(t) = tables.get(tp.id - TABLE_ID_BASE) else { continue };
        let mut table = t.clone();
        let width: f32 = table.widths.iter().sum();
        if width > col_w {
            let k = col_w / width;
            for w in &mut table.widths {
                *w *= k;
            }
        }
        let indent = (tp.rect.left - col_left).clamp(0.0, (col_w - table.widths.iter().sum::<f32>()).max(0.0));
        pending.push(Pending {
            top: tp.rect.top,
            bottom: tp.rect.bottom,
            first_baseline: None,
            last_baseline: None,
            item: Item::Table { table, indent, space_before: 0.0 },
        });
    }
    let mut shrunk = 0;
    // Pictures with text of this column beside them float; the rest go in rows.
    let (floating, mut inline): (Vec<Picture>, Vec<Picture>) = pictures.into_iter().partition(|pic| {
        line_rects.iter().any(|r| {
            r.top > pic.rect.bottom + 1.0
                && r.bottom < pic.rect.top - 1.0
                && (r.right <= pic.rect.left || r.left >= pic.rect.right)
        })
    });
    inline.sort_by(|a, b| b.rect.top.total_cmp(&a.rect.top));
    let mut rows: Vec<Vec<Picture>> = Vec::new();
    for pic in inline {
        let joins = rows.last().is_some_and(|row| {
            let bottom = row.iter().map(|p| p.rect.bottom).fold(f32::MAX, f32::min);
            pic.rect.top > bottom + 1.0
        });
        if joins {
            rows.last_mut().expect("row").push(pic);
        } else {
            rows.push(vec![pic]);
        }
    }
    for mut row in rows {
        row.sort_by(|a, b| a.rect.left.total_cmp(&b.rect.left));
        let top = row.iter().map(|p| p.rect.top).fold(f32::MIN, f32::max);
        let bottom = row.iter().map(|p| p.rect.bottom).fold(f32::MAX, f32::min);
        // Shrink the whole row when it is wider than the column.
        let span = row.last().map(|p| p.rect.right).unwrap_or(col_left) - row[0].rect.left.min(col_left).max(col_left);
        let scale = if span > col_w { col_w / span } else { 1.0 };
        if scale < 1.0 {
            shrunk += row.len();
        }
        let mut items = Vec::new();
        let mut x = 0.0f32;
        for p in &row {
            let w = p.rect.width() * scale;
            let h = p.rect.height() * scale;
            // Never overlap the picture before; never run past the column.
            let offset = ((p.rect.left - col_left) * scale).max(x).min((col_w - w).max(0.0));
            x = offset + w;
            items.push(RowPicture { rect: p.rect, id: p.id, width: w, height: h, offset });
        }
        pending.push(Pending {
            top,
            bottom: top - (top - bottom) * scale,
            first_baseline: None,
            last_baseline: None,
            item: Item::Pictures { row: items, space_before: 0.0 },
        });
    }
    pending.sort_by(|a, b| b.top.total_cmp(&a.top));
    // Space before each item: from the previous item's bottom to this item's top, less the part
    // of the gap the line spacing itself produces.
    let mut prev: Option<End> = match start {
        ColumnStart::Top(_) => None,
        ColumnStart::After(end) => Some(end),
    };
    let band_top = match start {
        ColumnStart::Top(t) => t,
        ColumnStart::After(_) => 0.0,
    };
    let mut items = Vec::new();
    for p in pending {
        let space = match (&p.item, prev) {
            (_, None) => (band_top - p.top).max(0.0),
            (Item::Para(par), Some((_, Some(last_base)))) => {
                (last_base - p.first_baseline.unwrap_or(p.top) - par.pitch).max(0.0)
            }
            (Item::Para(par), Some((prev_bottom, None))) => {
                // After a picture: the first line's box starts about a line's ascent above its baseline.
                (prev_bottom - p.first_baseline.unwrap_or(p.top) - 0.8 * par.pitch).max(0.0)
            }
            (Item::Pictures { .. } | Item::Table { .. }, Some((prev_bottom, _))) => (prev_bottom - p.top).max(0.0),
        };
        let mut item = p.item;
        match &mut item {
            Item::Para(par) => par.space_before = space,
            Item::Pictures { space_before, .. } | Item::Table { space_before, .. } => *space_before = space,
        }
        prev = Some((p.bottom, p.last_baseline));
        items.push(item);
    }
    let end = match start {
        ColumnStart::After(e) if items.is_empty() => Some(e),
        _ if items.is_empty() => None,
        _ => prev,
    };
    (items, shrunk, floating, end)
}

/// Where a column's first item starts: at the top of the page's content, or below what the
/// band above ended with.
#[derive(Debug, Clone, Copy)]
enum ColumnStart {
    Top(f32),
    After(End),
}

/// (bottom, last baseline when the last item is text).
type End = (f32, Option<f32>);

/// Adds a band of lines and pictures with the given column edges.
type AddBand<'a> = dyn FnMut(Vec<Line>, Vec<Picture>, &[(f32, f32)], &mut Vec<Band>) + 'a;

/// Split the page's glyphs into those outside `tables` and those of each table cell; generated
/// spaces follow the glyph before them.
#[allow(clippy::type_complexity)]
fn assign(glyphs: &[Glyph], tables: &[Table]) -> (Vec<Glyph>, Vec<Vec<Vec<Glyph>>>) {
    let mut outside: Vec<Glyph> = Vec::new();
    let mut cell_glyphs: Vec<Vec<Vec<Glyph>>> = tables.iter().map(|t| vec![Vec::new(); t.cells.len()]).collect();
    let mut dest: Option<(usize, usize)> = None;
    for g in glyphs {
        if !g.generated {
            let (cx, cy) = ((g.left + g.right) / 2.0, (g.bottom + g.top) / 2.0);
            dest = tables
                .iter()
                .enumerate()
                .find_map(|(ti, t)| t.contains(cx, cy).then(|| t.cell_at(cx, cy).map(|c| (ti, c))).flatten());
        }
        match dest {
            Some((ti, ci)) => cell_glyphs[ti][ci].push(g.clone()),
            None => outside.push(g.clone()),
        }
    }
    (outside, cell_glyphs)
}

/// The page's borderless tables (decision D-020) as table items, top first: for table export
/// only. Ruled tables are left to `layout_page`.
pub fn borderless_tables(page: &PageInput) -> Vec<TableItem> {
    let avoid: Vec<Rect> = find_tables(&page.rules)
        .iter()
        .map(|t| Rect { left: t.left(), bottom: t.bottom(), right: t.right(), top: t.top() })
        .collect();
    let found = super::borderless::find_borderless(&page.glyphs, &avoid);
    if found.is_empty() {
        return Vec::new();
    }
    let (outside, cell_glyphs) = assign(&page.glyphs, &found);
    let body = body_size(&build_lines(&outside));
    found.iter().zip(cell_glyphs).map(|(t, cg)| table_item(t, cg, body)).collect()
}

/// Lay out one page.
pub fn layout_page(page: &PageInput) -> PageLayout {
    // Ruled tables take the glyphs inside them; generated spaces follow the glyph before them.
    let found = find_tables(&page.rules);
    let (outside, cell_glyphs) = assign(&page.glyphs, &found);
    let lines = build_lines(&outside);
    let body = body_size(&lines);
    let table_items: Vec<TableItem> = found.iter().zip(cell_glyphs).map(|(t, cg)| table_item(t, cg, body)).collect();
    let table_pics: Vec<Picture> = found
        .iter()
        .enumerate()
        .map(|(i, t)| Picture {
            id: TABLE_ID_BASE + i,
            rect: Rect { left: t.left(), bottom: t.bottom(), right: t.right(), top: t.top() },
        })
        .collect();
    let page_area = (page.width * page.height).max(1.0);
    // Backgrounds: pictures over half the page that text overlaps.
    let (backgrounds, pictures): (Vec<Picture>, Vec<Picture>) = page.pictures.iter().cloned().partition(|p| {
        p.rect.width() * p.rect.height() > 0.5 * page_area && lines.iter().any(|l| l.rect.overlaps(&p.rect))
    });
    let pictures: Vec<Picture> =
        pictures.into_iter().filter(|p| p.rect.width() >= 2.0 && p.rect.height() >= 2.0).chain(table_pics).collect();

    let mut content: Option<Rect> = None;
    for r in lines.iter().map(|l| l.rect).chain(pictures.iter().map(|p| p.rect)) {
        content = Some(match content {
            None => r,
            Some(c) => Rect {
                left: c.left.min(r.left),
                bottom: c.bottom.min(r.bottom),
                right: c.right.max(r.right),
                top: c.top.max(r.top),
            },
        });
    }
    let content =
        content.unwrap_or(Rect { left: 72.0, bottom: 72.0, right: page.width - 72.0, top: page.height - 72.0 });
    let left = content.left.clamp(0.0, page.width / 2.0);
    // A little room on the right: an editor measures text slightly differently, and a line that
    // exactly fills the PDF's width must not wrap.
    let slack = (0.015 * (content.right - left)).max(3.0);
    let right = (content.right + slack).min(page.width).max(left + 36.0);
    let margin_left = left;
    let margin_right = (page.width - right).max(0.0);
    let margin_top = (page.height - content.top).max(0.0);
    // A normal bottom margin at most: Word's line boxes differ slightly from the PDF's, and
    // content must never spill onto an extra page.
    let margin_bottom = (content.bottom - 18.0).clamp(0.0, 36.0);

    let mut heading_sizes: Vec<f32> = Vec::new();
    for l in &lines {
        if l.size >= 1.25 * body && !heading_sizes.iter().any(|&s| (s - l.size).abs() < 0.5) {
            heading_sizes.push(l.size);
        }
    }
    heading_sizes.sort_by(|a, b| b.total_cmp(a));

    let gut = gutters(&lines, left, right);
    let mut bands: Vec<Band> = Vec::new();
    let mut shrunk_pictures = 0;
    let mut floating: Vec<Picture> = Vec::new();
    let mut band_end: Option<End> = None;
    let mut add_band = |band_lines: Vec<Line>, band_pics: Vec<Picture>, edges: &[(f32, f32)], bands: &mut Vec<Band>| {
        let top =
            band_lines.iter().map(|l| l.rect.top).chain(band_pics.iter().map(|p| p.rect.top)).fold(f32::MIN, f32::max);
        let start = match band_end {
            Some(e) => ColumnStart::After(e),
            None => ColumnStart::Top(top),
        };
        let mut columns = Vec::new();
        let mut lowest: Option<End> = None;
        for &(c0, c1) in edges {
            let in_col = |x: f32| x >= c0 - 0.5 && x <= c1 + 0.5 || edges.len() == 1;
            let col_lines: Vec<Line> = band_lines.iter().filter(|l| in_col(l.rect.cx())).cloned().collect();
            let col_pics: Vec<Picture> = band_pics.iter().filter(|p| in_col(p.rect.cx())).cloned().collect();
            let (items, s, f, end) =
                column_items(col_lines, col_pics, (c0, c1), body, &heading_sizes, start, &table_items);
            shrunk_pictures += s;
            floating.extend(f);
            if let Some(e) = end {
                if lowest.is_none_or(|l| e.0 < l.0) {
                    lowest = Some(e);
                }
            }
            columns.push(Column { left: c0, right: c1, items });
        }
        band_end = lowest.or(band_end);
        bands.push(Band { columns });
    };
    if gut.is_empty() {
        let mut sorted = lines;
        sorted.sort_by(|a, b| b.baseline.total_cmp(&a.baseline));
        add_band(sorted, pictures, &[(left, right)], &mut bands);
    } else {
        let mut edges: Vec<(f32, f32)> = Vec::new();
        let mut x = left;
        for &(g0, g1) in &gut {
            edges.push((x, g0));
            x = g1;
        }
        edges.push((x, right));
        let crosses = |r: &Rect| gut.iter().any(|&(g0, g1)| r.left < g1 && r.right > g0);
        // Items in top order; spanning ones (titles, wide pictures) start one-column bands.
        enum Thing {
            L(Line),
            P(Picture),
        }
        let mut things: Vec<(f32, bool, Thing)> = lines
            .into_iter()
            .map(|l| (l.rect.top, crosses(&l.rect), Thing::L(l)))
            .chain(pictures.into_iter().map(|p| (p.rect.top, crosses(&p.rect), Thing::P(p))))
            .collect();
        things.sort_by(|a, b| b.0.total_cmp(&a.0));
        let mut group: (bool, Vec<Line>, Vec<Picture>) = (false, Vec::new(), Vec::new());
        let flush = |g: &mut (bool, Vec<Line>, Vec<Picture>), bands: &mut Vec<Band>, add: &mut AddBand| {
            if g.1.is_empty() && g.2.is_empty() {
                return;
            }
            let ls = std::mem::take(&mut g.1);
            let ps = std::mem::take(&mut g.2);
            if g.0 {
                add(ls, ps, &[(left, right)], bands);
            } else {
                add(ls, ps, &edges, bands);
            }
        };
        for (_, spanning, t) in things {
            if spanning != group.0 {
                flush(&mut group, &mut bands, &mut add_band);
                group.0 = spanning;
            }
            match t {
                Thing::L(l) => group.1.push(l),
                Thing::P(p) => group.2.push(p),
            }
        }
        flush(&mut group, &mut bands, &mut add_band);
    }
    // Space before the first item of a later band: from the previous band's lowest content.
    PageLayout {
        width: page.width,
        height: page.height,
        margin_left,
        margin_right,
        margin_top,
        margin_bottom,
        bands,
        backgrounds,
        floating,
        shrunk_pictures,
        tables: found,
    }
}

/// Every character the layout emits (whitespace excluded).
pub(crate) fn emitted_chars(layout: &PageLayout) -> HashMap<char, usize> {
    let mut m = HashMap::new();
    for b in &layout.bands {
        for c in &b.columns {
            for it in &c.items {
                let paras: Vec<&Paragraph> = match it {
                    Item::Para(p) => vec![p],
                    Item::Table { table, .. } => table.cells.iter().flat_map(|c| c.paragraphs.iter()).collect(),
                    Item::Pictures { .. } => vec![],
                };
                for p in paras {
                    for r in &p.runs {
                        for ch in r.text.chars().filter(|c| !c.is_whitespace()) {
                            *m.entry(ch).or_default() += 1;
                        }
                    }
                }
            }
        }
    }
    m
}

/// Characters of the input that must appear in the output (whitespace and joined hyphens excluded).
pub fn expected_chars(page: &PageInput) -> HashMap<char, usize> {
    let lines = build_lines(&page.glyphs);
    let mut m = HashMap::new();
    for l in &lines {
        let n = l.glyphs.len();
        for (i, g) in l.glyphs.iter().enumerate() {
            if g.ch.is_whitespace() || (g.hyphen && i + 1 == n) {
                continue;
            }
            *m.entry(g.ch).or_default() += 1;
        }
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Glyphs of `text` on one line starting at `x`, `baseline`, monospaced at 0.5 em.
    fn line(text: &str, x: f32, baseline: f32, size: f32) -> Vec<Glyph> {
        let w = size * 0.5;
        text.chars()
            .enumerate()
            .filter(|(_, c)| *c != ' ')
            .map(|(i, ch)| Glyph {
                ch,
                left: x + i as f32 * w,
                right: x + (i as f32 + 1.0) * w,
                bottom: baseline - 0.2 * size,
                top: baseline + 0.7 * size,
                baseline,
                size,
                font: "Helvetica".into(),
                bold: false,
                italic: false,
                color: [0, 0, 0],
                generated: false,
                hyphen: false,
                link: None,
            })
            .collect()
    }

    fn page(glyphs: Vec<Glyph>) -> PageInput {
        PageInput { width: 612.0, height: 792.0, glyphs, pictures: vec![], rules: vec![] }
    }

    fn texts(l: &PageLayout) -> Vec<Vec<String>> {
        l.bands
            .iter()
            .flat_map(|b| b.columns.iter())
            .map(|c| {
                c.items
                    .iter()
                    .filter_map(|i| match i {
                        Item::Para(p) => Some(p.runs.iter().map(|r| r.text.as_str()).collect::<String>()),
                        _ => None,
                    })
                    .collect()
            })
            .collect()
    }

    #[test]
    fn words_lines_and_paragraphs() {
        let mut g = Vec::new();
        // A paragraph of three wrapped lines (the last short), a gap, then another paragraph.
        g.extend(line("The quick brown fox jumps over the lazy dog and keeps", 72.0, 700.0, 12.0));
        g.extend(line("running across the wide green field until it reaches", 72.0, 686.0, 12.0));
        g.extend(line("the river.", 72.0, 672.0, 12.0));
        g.extend(line("A second paragraph starts here after a blank line.", 72.0, 640.0, 12.0));
        let l = layout_page(&page(g.clone()));
        let t = texts(&l);
        assert_eq!(t.len(), 1, "one column");
        assert_eq!(
            t[0],
            [
                "The quick brown fox jumps over the lazy dog and keeps running across the wide green field until it reaches the river.",
                "A second paragraph starts here after a blank line."
            ]
        );
        let Item::Para(p2) = &l.bands[0].columns[0].items[1] else { panic!("para") };
        assert!((p2.pitch - 14.4).abs() < 0.01, "single line → 1.2 × size");
        // 672 - 640 = 32 between baselines, less one line of 14.4.
        assert!((p2.space_before - 17.6).abs() < 0.1, "{}", p2.space_before);
        assert_eq!(emitted_chars(&l), expected_chars(&page(g)));
    }

    #[test]
    fn headings_alignment_and_lists() {
        let mut g = Vec::new();
        g.extend(line("Title", 291.0, 740.0, 24.0)); // 5 chars × 12 = 60 wide → centred on 321? (page centre 306)
        g.extend(line("Body text that fills the line from margin to margin ok", 72.0, 700.0, 12.0));
        g.extend(line("• first item", 72.0, 680.0, 12.0));
        g.extend(line("• second item", 72.0, 666.0, 12.0));
        let l = layout_page(&page(g));
        let items = &l.bands[0].columns[0].items;
        let Item::Para(title) = &items[0] else { panic!() };
        assert_eq!(title.heading, Some(1));
        let Item::Para(a) = &items[2] else { panic!() };
        let Item::Para(b) = &items[3] else { panic!() };
        assert_eq!(a.runs[0].text, "• first item");
        assert_eq!(b.runs[0].text, "• second item", "each list item is its own paragraph");
    }

    #[test]
    fn two_columns_read_column_by_column_under_a_spanning_title() {
        let mut g = Vec::new();
        g.extend(line("A title that spans across both of the columns below it", 72.0, 740.0, 12.0));
        for i in 0..8 {
            let y = 700.0 - i as f32 * 14.0;
            g.extend(line(&format!("left column line {i} text with more words"), 72.0, y, 10.0));
            g.extend(line(&format!("right column line {i} has more words too"), 290.0, y, 10.0));
        }
        let l = layout_page(&page(g.clone()));
        assert_eq!(l.bands.len(), 2, "{:#?}", l.bands.iter().map(|b| b.columns.len()).collect::<Vec<_>>());
        assert_eq!(l.bands[0].columns.len(), 1);
        assert_eq!(l.bands[1].columns.len(), 2);
        let t = texts(&l);
        assert!(t[1][0].starts_with("left column line 0 text with more words left column line 1"), "{:?}", t[1]);
        assert!(t[2][0].starts_with("right column line 0"), "{:?}", t[2]);
        assert_eq!(emitted_chars(&l), expected_chars(&page(g)));
    }

    #[test]
    fn cells_on_one_row_stay_on_one_line_with_tab_stops() {
        let mut g = line("Region", 72.0, 700.0, 10.0);
        g.extend(line("Sales", 200.0, 700.0, 10.0));
        g.extend(line("North", 72.0, 686.0, 10.0));
        g.extend(line("120", 200.0, 686.0, 10.0));
        let l = layout_page(&page(g.clone()));
        let items = &l.bands[0].columns[0].items;
        assert_eq!(items.len(), 2, "{items:?}");
        let Item::Para(p) = &items[0] else { panic!() };
        assert_eq!(p.runs.iter().map(|r| r.text.as_str()).collect::<String>(), "Region\tSales");
        assert_eq!(p.tabs, [128.0]);
        assert_eq!(emitted_chars(&l), expected_chars(&page(g)));
    }

    #[test]
    fn label_value_layouts_are_not_columns() {
        let mut g = Vec::new();
        for i in 0..10 {
            let y = 700.0 - i as f32 * 20.0;
            g.extend(line("Name:", 72.0, y, 12.0));
            g.extend(line("Value", 400.0, y, 12.0));
        }
        let l = layout_page(&page(g));
        assert_eq!(l.bands.len(), 1);
        assert_eq!(l.bands[0].columns.len(), 1, "short labels and values are not prose columns");
    }

    #[test]
    fn soft_hyphens_join_words_and_superscripts_are_kept() {
        let mut a = line("infor-", 72.0, 700.0, 12.0);
        a.last_mut().expect("value").hyphen = true;
        let mut b = line("mation x", 72.0, 686.0, 12.0);
        // "2" raised and smaller after x.
        let mut sup = line("2", 72.0 + 8.0 * 6.0, 690.0, 7.0);
        sup[0].left = b.last().expect("value").right;
        sup[0].right = sup[0].left + 3.5;
        b.extend(sup);
        let mut g = a;
        g.extend(b);
        let l = layout_page(&page(g.clone()));
        let Item::Para(p) = &l.bands[0].columns[0].items[0] else { panic!() };
        let text: String = p.runs.iter().map(|r| r.text.as_str()).collect();
        assert_eq!(text, "information x2");
        assert_eq!(p.runs.last().expect("value").vert, VertAlign::Superscript);
        assert_eq!(emitted_chars(&l), expected_chars(&page(g)));
    }

    #[test]
    fn pictures_take_their_place_and_backgrounds_stay_behind() {
        let mut g = line("Text above the picture.", 72.0, 700.0, 12.0);
        g.extend(line("Text below the picture.", 72.0, 500.0, 12.0));
        let mut p = page(g);
        p.pictures.push(Picture { id: 0, rect: Rect { left: 100.0, bottom: 520.0, right: 300.0, top: 680.0 } });
        p.pictures.push(Picture { id: 1, rect: Rect { left: 0.0, bottom: 0.0, right: 612.0, top: 792.0 } });
        let l = layout_page(&p);
        let items = &l.bands[0].columns[0].items;
        let Item::Pictures { row, .. } = &items[1] else { panic!("{items:?}") };
        assert_eq!(
            row,
            &[RowPicture {
                rect: Rect { left: 100.0, bottom: 520.0, right: 300.0, top: 680.0 },
                id: 0,
                width: 200.0,
                height: 160.0,
                offset: 28.0
            }]
        );
        assert_eq!(l.backgrounds.len(), 1);
        assert_eq!(l.backgrounds[0].id, 1);
    }

    #[test]
    fn pictures_side_by_side_share_a_row_and_pictures_beside_text_float() {
        let mut g = line("Heading line of the page", 72.0, 740.0, 12.0);
        g.extend(line("Caption beside the photo", 300.0, 600.0, 12.0));
        let mut p = page(g);
        p.pictures.push(Picture { id: 0, rect: Rect { left: 72.0, bottom: 400.0, right: 216.0, top: 544.0 } });
        p.pictures.push(Picture { id: 1, rect: Rect { left: 300.0, bottom: 400.0, right: 444.0, top: 544.0 } });
        p.pictures.push(Picture { id: 2, rect: Rect { left: 72.0, bottom: 560.0, right: 200.0, top: 680.0 } });
        let l = layout_page(&p);
        assert_eq!(l.floating.iter().map(|f| f.id).collect::<Vec<_>>(), [2], "text sits beside picture 2");
        let rows: Vec<&Vec<RowPicture>> = l.bands[0].columns[0]
            .items
            .iter()
            .filter_map(|i| match i {
                Item::Pictures { row, .. } => Some(row),
                _ => None,
            })
            .collect();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].iter().map(|r| (r.id, r.offset)).collect::<Vec<_>>(), [(0, 0.0), (1, 228.0)]);
        assert_eq!(l.margin_bottom, 36.0, "bottom margin capped");
    }

    #[test]
    fn empty_pages_and_bad_glyphs_do_not_fail() {
        let l = layout_page(&page(vec![]));
        assert_eq!(l.bands.len(), 1);
        let mut g = line("ok", 72.0, 700.0, 12.0);
        g[0].size = 0.0;
        g[1].left = f32::NAN;
        let l = layout_page(&page(g));
        assert!(texts(&l)[0].is_empty());
    }
}
