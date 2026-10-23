//! Borderless tables (PRD 06 "Spreadsheet/table extraction", unruled case, decision D-020): rows of
//! text whose wide gaps line up in the same vertical channels on three or more consecutive lines
//! become a grid. Used for table export to spreadsheets only; Word and PowerPoint export keep such
//! text as lines with tab stops, which already reproduces it. Pure and unit tested on synthetic
//! pages. Coordinates are PDF points in the page's displayed orientation, y upwards.
//!
//! Limits (documented in D-020): a cell that wraps onto a second line becomes its own row; a
//! column whose cells are all empty except one header is merged into its neighbour; prose set in
//! columns is rejected by its long, full lines.

use super::layout::{Glyph, Rect};
use super::tables::{Cell, Table};

/// A gap at least this wide (in ems of the larger neighbour) separates cells in a row.
const CELL_GAP_EM: f32 = 1.0;
/// Fewer rows than this are not a table.
const MIN_ROWS: usize = 3;
/// Rows further apart than this (baseline to baseline, in ems) end a table.
const MAX_ROW_PITCH_EM: f32 = 2.6;
/// Larger grids are not considered (pathological pages).
const MAX_ROWS: usize = 2_000;
const MAX_COLS: usize = 64;

#[derive(Debug, Clone, Copy)]
struct Seg {
    left: f32,
    right: f32,
    words: usize,
}

#[derive(Debug, Clone)]
struct Row {
    baseline: f32,
    /// Vertical centre of the row's glyphs.
    mid: f32,
    top: f32,
    bottom: f32,
    size: f32,
    segs: Vec<Seg>,
}

fn rows_of(glyphs: &[&Glyph]) -> Vec<Row> {
    let mut gs: Vec<&Glyph> = glyphs.to_vec();
    gs.sort_by(|a, b| b.baseline.total_cmp(&a.baseline).then(a.left.total_cmp(&b.left)));
    // Glyphs on (nearly) the same baseline form one row.
    let mut groups: Vec<Vec<&Glyph>> = Vec::new();
    for g in gs {
        match groups.last_mut() {
            Some(row) if (row[0].baseline - g.baseline).abs() <= 0.35 * row[0].size.max(g.size) => row.push(g),
            _ => groups.push(vec![g]),
        }
    }
    groups
        .into_iter()
        .map(|mut row| {
            row.sort_by(|a, b| a.left.total_cmp(&b.left));
            let size = row.iter().map(|g| g.size).fold(0.0f32, f32::max);
            let mut segs: Vec<Seg> = Vec::new();
            let mut prev: Option<&Glyph> = None;
            for g in &row {
                let gap = prev.map(|p| g.left - p.right);
                match (segs.last_mut(), gap) {
                    (Some(s), Some(gap)) if gap < CELL_GAP_EM * g.size.max(prev.map_or(0.0, |p| p.size)) => {
                        if gap > 0.15 * g.size || g.ch.is_whitespace() {
                            s.words += 1;
                        }
                        s.right = s.right.max(g.right);
                    }
                    _ => segs.push(Seg { left: g.left, right: g.right, words: 1 }),
                }
                prev = Some(g);
            }
            let top = row.iter().map(|g| g.top).fold(f32::MIN, f32::max);
            let bottom = row.iter().map(|g| g.bottom).fold(f32::MAX, f32::min);
            Row { baseline: row[0].baseline, mid: (top + bottom) / 2.0, top, bottom, size, segs }
        })
        .collect()
}

/// The part of channel `(a, b)` left free by `segs` (the widest free piece), if wide enough.
fn narrow(ch: (f32, f32), segs: &[Seg], min: f32) -> Option<(f32, f32)> {
    let mut pieces = vec![ch];
    for s in segs {
        pieces = pieces
            .into_iter()
            .flat_map(|(a, b)| {
                if s.right <= a || s.left >= b {
                    vec![(a, b)]
                } else {
                    let mut v = Vec::new();
                    if s.left > a {
                        v.push((a, s.left));
                    }
                    if s.right < b {
                        v.push((s.right, b));
                    }
                    v
                }
            })
            .collect();
    }
    pieces.into_iter().filter(|(a, b)| b - a >= min).max_by(|x, y| (x.1 - x.0).total_cmp(&(y.1 - y.0)))
}

/// Free vertical channels between the cells of one row.
fn gaps(row: &Row, min: f32) -> Vec<(f32, f32)> {
    row.segs.windows(2).map(|w| (w[0].right, w[1].left)).filter(|(a, b)| b - a >= min).collect()
}

fn median(mut v: Vec<f32>) -> f32 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(f32::total_cmp);
    v[v.len() / 2]
}

/// Turn rows `rows` (top to bottom) with channels `channels` into a table, if it looks like one.
fn table_of(rows: &[Row], channels: &[(f32, f32)]) -> Option<Table> {
    let multi = rows.iter().filter(|r| r.segs.len() >= 2).count();
    if rows.len() < MIN_ROWS || rows.len() > MAX_ROWS || channels.is_empty() || channels.len() + 1 > MAX_COLS {
        return None;
    }
    if multi * 5 < rows.len() * 3 || multi < 2 {
        return None;
    }
    let left = rows.iter().flat_map(|r| r.segs.iter().map(|s| s.left)).fold(f32::MAX, f32::min);
    let right = rows.iter().flat_map(|r| r.segs.iter().map(|s| s.right)).fold(f32::MIN, f32::max);
    let mut xs = vec![left - 1.0];
    xs.extend(channels.iter().map(|(a, b)| (a + b) / 2.0));
    xs.push(right + 1.0);
    let cols = xs.len() - 1;
    // Which slots hold text, and how full each column's cells are.
    let mut filled = 0usize;
    let mut fill: Vec<Vec<f32>> = vec![Vec::new(); cols];
    let mut words = 0usize;
    let mut segs = 0usize;
    for r in rows {
        let mut used = vec![false; cols];
        for s in &r.segs {
            let c = xs.windows(2).position(|w| (s.left + s.right) / 2.0 < w[1]).unwrap_or(cols - 1);
            used[c] = true;
            fill[c].push((s.right - s.left) / (xs[c + 1] - xs[c]).max(1.0));
            words += s.words;
            segs += 1;
        }
        filled += used.iter().filter(|u| **u).count();
    }
    if filled * 2 < rows.len() * cols {
        return None;
    }
    // Prose in columns: every column's lines fill it and hold many words.
    let full = fill.iter().all(|f| median(f.clone()) >= 0.75);
    if full && words as f32 / segs.max(1) as f32 >= 5.0 {
        return None;
    }
    let mut ys = vec![rows[0].top + 1.0];
    for w in rows.windows(2) {
        ys.push((w[0].mid + w[1].mid) / 2.0);
    }
    ys.push(rows[rows.len() - 1].bottom - 1.0);
    if ys.windows(2).any(|w| w[1] >= w[0]) {
        return None;
    }
    let mut cells = Vec::with_capacity(rows.len() * cols);
    for row in 0..rows.len() {
        for col in 0..cols {
            cells.push(Cell { row, col, row_span: 1, col_span: 1 });
        }
    }
    Some(Table { xs, ys, cells })
}

/// Borderless tables among `glyphs`, top first. Glyphs inside `avoid` (ruled tables) are ignored
/// and no table overlaps them.
pub fn find_borderless(glyphs: &[Glyph], avoid: &[Rect]) -> Vec<Table> {
    let inside = |g: &Glyph, r: &Rect| {
        let (x, y) = ((g.left + g.right) / 2.0, (g.bottom + g.top) / 2.0);
        x >= r.left && x <= r.right && y >= r.bottom && y <= r.top
    };
    let usable: Vec<&Glyph> = glyphs
        .iter()
        .filter(|g| {
            !g.generated
                && !g.ch.is_whitespace()
                && g.size > 0.0
                && [g.left, g.right, g.bottom, g.top, g.baseline].iter().all(|v| v.is_finite())
                && !avoid.iter().any(|r| inside(g, r))
        })
        .collect();
    if usable.len() > 200_000 {
        return Vec::new();
    }
    let rows = rows_of(&usable);
    let mut out = Vec::new();
    let mut i = 0;
    while i < rows.len() {
        let min = CELL_GAP_EM * rows[i].size;
        let mut channels = gaps(&rows[i], min);
        if channels.is_empty() {
            i += 1;
            continue;
        }
        let mut j = i + 1;
        while j < rows.len() {
            let (prev, r) = (&rows[j - 1], &rows[j]);
            if prev.baseline - r.baseline > MAX_ROW_PITCH_EM * prev.size.max(r.size) {
                break;
            }
            let next: Vec<(f32, f32)> = channels.iter().filter_map(|&c| narrow(c, &r.segs, min)).collect();
            if next.len() < channels.len() {
                break;
            }
            channels = next;
            j += 1;
        }
        // Single-cell rows at the end are text after the table.
        let mut end = j;
        while end > i && rows[end - 1].segs.len() < 2 {
            end -= 1;
        }
        let candidate = table_of(&rows[i..end], &channels).filter(|t| {
            let r = Rect { left: t.left(), bottom: t.bottom(), right: t.right(), top: t.top() };
            !avoid.iter().any(|a| a.left < r.right && r.left < a.right && a.bottom < r.top && r.bottom < a.top)
        });
        match candidate {
            Some(t) => {
                out.push(t);
                i = end;
            }
            None => i += 1,
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Glyphs of `text` starting at `x` on `baseline`, 0.5 em wide each; spaces leave a gap.
    fn text(s: &str, x: f32, baseline: f32) -> Vec<Glyph> {
        let size = 10.0;
        s.chars()
            .enumerate()
            .filter(|(_, c)| *c != ' ')
            .map(|(i, ch)| Glyph {
                ch,
                left: x + i as f32 * 5.0,
                right: x + i as f32 * 5.0 + 5.0,
                bottom: baseline - 2.0,
                top: baseline + 7.0,
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

    fn row(cells: &[(&str, f32)], baseline: f32) -> Vec<Glyph> {
        cells.iter().flat_map(|(s, x)| text(s, *x, baseline)).collect()
    }

    #[test]
    fn aligned_columns_are_a_table_and_the_text_around_it_is_not() {
        let mut g = text("Quarterly results for the north region are below.", 72.0, 700.0);
        g.extend(row(&[("Item", 72.0), ("Units", 200.0), ("Price", 300.0)], 680.0));
        g.extend(row(&[("Apples", 72.0), ("120", 200.0), ("0.50", 300.0)], 666.0));
        g.extend(row(&[("Pears", 72.0), ("80", 200.0), ("0.75", 300.0)], 652.0));
        g.extend(row(&[("Plums and damsons", 72.0), ("7", 200.0)], 638.0));
        g.extend(text("Prices are per kilogram and exclude tax.", 72.0, 610.0));
        let t = find_borderless(&g, &[]);
        assert_eq!(t.len(), 1, "{t:?}");
        assert_eq!(t[0].xs.len() - 1, 3);
        assert_eq!(t[0].ys.len() - 1, 4);
        // The sentence above and the note below are outside.
        assert!(t[0].top() < 700.0 && t[0].bottom() > 615.0);
        assert!(t[0].cell_at(205.0, 669.0).is_some());
    }

    #[test]
    fn prose_and_prose_in_columns_are_not_tables() {
        let mut g = Vec::new();
        for k in 0..8 {
            let y = 700.0 - k as f32 * 13.0;
            g.extend(text("the quick brown fox jumps over a lazy dog", 72.0, y));
            g.extend(text("pack my box with five dozen liquor jugs", 300.0, y));
        }
        assert!(find_borderless(&g, &[]).is_empty());
        let mut p = Vec::new();
        for k in 0..6 {
            p.extend(text("a single column of ordinary running text here", 72.0, 700.0 - k as f32 * 13.0));
        }
        assert!(find_borderless(&p, &[]).is_empty());
    }

    #[test]
    fn two_rows_or_misaligned_gaps_are_not_tables() {
        let mut g = row(&[("A", 72.0), ("1", 200.0)], 700.0);
        g.extend(row(&[("B", 72.0), ("2", 200.0)], 686.0));
        assert!(find_borderless(&g, &[]).is_empty());
        // Gaps in different places on every line.
        let mut m = row(&[("Alpha", 72.0), ("x", 150.0)], 700.0);
        m.extend(row(&[("Beta gamma delta epsilon", 72.0), ("y", 260.0)], 686.0));
        m.extend(row(&[("Zeta eta theta iota kappa lambda", 72.0), ("z", 330.0)], 672.0));
        assert!(find_borderless(&m, &[]).is_empty());
    }

    #[test]
    fn rows_far_apart_and_ruled_areas_are_kept_out() {
        let mut g = Vec::new();
        for k in 0..3 {
            g.extend(row(&[("Key", 72.0), ("Value", 200.0)], 700.0 - k as f32 * 60.0));
        }
        assert!(find_borderless(&g, &[]).is_empty());
        let mut t = Vec::new();
        for k in 0..4 {
            t.extend(row(&[("Key", 72.0), ("Value", 200.0)], 700.0 - k as f32 * 14.0));
        }
        assert_eq!(find_borderless(&t, &[]).len(), 1);
        let ruled = Rect { left: 60.0, bottom: 600.0, right: 400.0, top: 720.0 };
        assert!(find_borderless(&t, &[ruled]).is_empty());
    }

    #[test]
    fn bad_glyphs_and_empty_pages_do_not_fail() {
        assert!(find_borderless(&[], &[]).is_empty());
        let mut g = row(&[("A", 72.0), ("1", 200.0)], 700.0);
        g[0].left = f32::NAN;
        g[1].size = 0.0;
        assert!(find_borderless(&g, &[]).is_empty());
    }
}
