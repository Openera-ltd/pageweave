//! Ruled tables (PRD 06 "Spreadsheet/table extraction", ruled case): grids of drawn horizontal
//! and vertical lines become tables with rows, columns and merged cells. Pure, unit tested with
//! synthetic rules. Coordinates are PDF points in the page's displayed orientation, y upwards.
//! Borderless tables are not detected here.

/// A straight drawn line, horizontal or vertical.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Rule {
    /// y, from x0 to x1 (x0 < x1).
    H { y: f32, x0: f32, x1: f32 },
    /// x, from y0 to y1 (y0 < y1).
    V { x: f32, y0: f32, y1: f32 },
}

/// Join tolerance between rules, points.
const TOL: f32 = 2.0;
/// Rules shorter than this are not table lines (tick marks, dots).
const MIN_RULE: f32 = 4.0;

/// Classify a line segment as a rule (or not).
pub fn rule_from_segment(x0: f32, y0: f32, x1: f32, y1: f32) -> Option<Rule> {
    if !(x0.is_finite() && y0.is_finite() && x1.is_finite() && y1.is_finite()) {
        return None;
    }
    if (y1 - y0).abs() < 0.8 && (x1 - x0).abs() >= MIN_RULE {
        Some(Rule::H { y: (y0 + y1) / 2.0, x0: x0.min(x1), x1: x0.max(x1) })
    } else if (x1 - x0).abs() < 0.8 && (y1 - y0).abs() >= MIN_RULE {
        Some(Rule::V { x: (x0 + x1) / 2.0, y0: y0.min(y1), y1: y0.max(y1) })
    } else {
        None
    }
}

#[cfg(test)]
/// Rules from a thin filled rectangle (many generators draw lines as filled boxes), or the four
/// edges of a framed rectangle.
pub fn rules_from_rect(left: f32, bottom: f32, right: f32, top: f32, filled_thin_only: bool) -> Vec<Rule> {
    let (w, h) = (right - left, top - bottom);
    if h < 2.5 && w >= MIN_RULE {
        return vec![Rule::H { y: (top + bottom) / 2.0, x0: left, x1: right }];
    }
    if w < 2.5 && h >= MIN_RULE {
        return vec![Rule::V { x: (left + right) / 2.0, y0: bottom, y1: top }];
    }
    if filled_thin_only {
        return Vec::new();
    }
    vec![
        Rule::H { y: top, x0: left, x1: right },
        Rule::H { y: bottom, x0: left, x1: right },
        Rule::V { x: left, y0: bottom, y1: top },
        Rule::V { x: right, y0: bottom, y1: top },
    ]
}

#[derive(Debug, Clone, PartialEq)]
pub struct Cell {
    /// First grid row and column (0-based, rows from the top).
    pub row: usize,
    pub col: usize,
    pub row_span: usize,
    pub col_span: usize,
    /// Edges drawn without a line: [above, below] (the gaps between framed boxes).
    pub open: [bool; 2],
}

#[derive(Debug, Clone, PartialEq)]
pub struct Table {
    /// Column edges, left to right (len = columns + 1).
    pub xs: Vec<f32>,
    /// Row edges, top to bottom (len = rows + 1).
    pub ys: Vec<f32>,
    /// Every cell once, by its top-left grid position, in reading order.
    pub cells: Vec<Cell>,
}

impl Table {
    pub fn left(&self) -> f32 {
        self.xs[0]
    }
    pub fn right(&self) -> f32 {
        *self.xs.last().expect("edges")
    }
    pub fn top(&self) -> f32 {
        self.ys[0]
    }
    pub fn bottom(&self) -> f32 {
        *self.ys.last().expect("edges")
    }
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x > self.left() && x < self.right() && y < self.top() && y > self.bottom()
    }
    /// The cell holding a point, if any.
    pub fn cell_at(&self, x: f32, y: f32) -> Option<usize> {
        let c = self.xs.windows(2).position(|w| x >= w[0] && x < w[1])?;
        let r = self.ys.windows(2).position(|w| y <= w[0] && y > w[1])?;
        self.cells.iter().position(|cell| {
            r >= cell.row && r < cell.row + cell.row_span && c >= cell.col && c < cell.col + cell.col_span
        })
    }
}

/// Merge rules that lie on one line and touch or overlap.
fn merge(mut rules: Vec<Rule>) -> Vec<Rule> {
    let mut hs: Vec<(f32, f32, f32)> = Vec::new();
    let mut vs: Vec<(f32, f32, f32)> = Vec::new();
    for r in rules.drain(..) {
        match r {
            Rule::H { y, x0, x1 } => hs.push((y, x0, x1)),
            Rule::V { x, y0, y1 } => vs.push((x, y0, y1)),
        }
    }
    // Segments on one line (positions within 1 pt) are joined in order along it.
    let join = |mut v: Vec<(f32, f32, f32)>| {
        v.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut out: Vec<(f32, f32, f32)> = Vec::new();
        let mut i = 0;
        while i < v.len() {
            let mut j = i + 1;
            while j < v.len() && v[j].0 - v[j - 1].0 <= 1.0 {
                j += 1;
            }
            let line = &mut v[i..j];
            line.sort_by(|a, b| a.1.total_cmp(&b.1));
            let start = out.len();
            for s in line.iter() {
                if out.len() > start {
                    let last = out.last_mut().expect("segment");
                    if s.1 <= last.2 + TOL {
                        last.2 = last.2.max(s.2);
                        continue;
                    }
                }
                out.push(*s);
            }
            i = j;
        }
        out
    };
    let mut out: Vec<Rule> = join(hs).into_iter().map(|(y, x0, x1)| Rule::H { y, x0, x1 }).collect();
    out.extend(join(vs).into_iter().map(|(x, y0, y1)| Rule::V { x, y0, y1 }));
    out
}

fn touches(a: &Rule, b: &Rule) -> bool {
    match (a, b) {
        (Rule::H { y, x0, x1 }, Rule::V { x, y0, y1 }) | (Rule::V { x, y0, y1 }, Rule::H { y, x0, x1 }) => {
            *x >= x0 - TOL && *x <= x1 + TOL && *y >= y0 - TOL && *y <= y1 + TOL
        }
        _ => false,
    }
}

/// Positions within `tol` of each other become one (their mean), sorted ascending.
fn cluster(mut v: Vec<f32>) -> Vec<f32> {
    v.sort_by(f32::total_cmp);
    let mut groups: Vec<Vec<f32>> = Vec::new();
    for x in v {
        match groups.last_mut() {
            Some(g) if x - g[g.len() - 1] <= TOL => g.push(x),
            _ => groups.push(vec![x]),
        }
    }
    groups.into_iter().map(|g| g.iter().sum::<f32>() / g.len() as f32).collect()
}

/// Is the stretch from `a` to `b` (at `at`) drawn by some rule in `rules` of the same direction?
fn drawn_h(rules: &[Rule], y: f32, a: f32, b: f32) -> bool {
    let mid = (a + b) / 2.0;
    rules.iter().any(|r| matches!(r, Rule::H { y: ry, x0, x1 } if (ry - y).abs() <= TOL && *x0 <= mid && *x1 >= mid))
}

fn drawn_v(rules: &[Rule], x: f32, a: f32, b: f32) -> bool {
    let mid = (a + b) / 2.0;
    rules.iter().any(|r| matches!(r, Rule::V { x: rx, y0, y1 } if (rx - x).abs() <= TOL && *y0 <= mid && *y1 >= mid))
}

fn grid(rules: &[Rule]) -> Option<Table> {
    let xs = cluster(rules.iter().filter_map(|r| if let Rule::V { x, .. } = r { Some(*x) } else { None }).collect());
    let mut ys =
        cluster(rules.iter().filter_map(|r| if let Rule::H { y, .. } = r { Some(*y) } else { None }).collect());
    ys.reverse();
    if xs.len() < 2 || ys.len() < 2 {
        return None;
    }
    let (rows, cols) = (ys.len() - 1, xs.len() - 1);
    if rows * cols < 2 || rows > 500 || cols > 64 {
        return None;
    }
    // Owner of each grid slot: cells grow right while no vertical rule separates them, then down
    // while no horizontal rule does (for the whole width).
    let mut owner: Vec<Option<usize>> = vec![None; rows * cols];
    let mut cells = Vec::new();
    for r in 0..rows {
        for c in 0..cols {
            if owner[r * cols + c].is_some() {
                continue;
            }
            let mut cs = 1;
            while c + cs < cols && !drawn_v(rules, xs[c + cs], ys[r + 1], ys[r]) && owner[r * cols + c + cs].is_none() {
                cs += 1;
            }
            let mut rs = 1;
            'down: while r + rs < rows {
                for k in c..c + cs {
                    if drawn_h(rules, ys[r + rs], xs[k], xs[k + 1]) || owner[(r + rs) * cols + k].is_some() {
                        break 'down;
                    }
                }
                rs += 1;
            }
            let id = cells.len();
            for rr in r..r + rs {
                for cc in c..c + cs {
                    owner[rr * cols + cc] = Some(id);
                }
            }
            let (l, rt) = (xs[c], xs[c + cs]);
            let open = [!drawn_h(rules, ys[r], l, rt), !drawn_h(rules, ys[r + rs], l, rt)];
            cells.push(Cell { row: r, col: c, row_span: rs, col_span: cs, open });
        }
    }
    // A grid that collapsed into one cell is a frame, not a table.
    if cells.len() < 2 {
        return None;
    }
    Some(Table { xs, ys, cells })
}

/// Find ruled tables among the page's rules.
pub fn find_tables(rules: &[Rule]) -> Vec<Table> {
    let rules = merge(rules.to_vec());
    let n = rules.len();
    if n > 20_000 {
        return Vec::new();
    }
    // Connected groups of crossing rules.
    let mut group: Vec<usize> = (0..n).collect();
    fn root(g: &mut [usize], mut i: usize) -> usize {
        while g[i] != i {
            g[i] = g[g[i]];
            i = g[i];
        }
        i
    }
    for i in 0..n {
        for j in i + 1..n {
            if touches(&rules[i], &rules[j]) {
                let (a, b) = (root(&mut group, i), root(&mut group, j));
                group[a] = b;
            }
        }
    }
    let mut by_root: std::collections::BTreeMap<usize, Vec<Rule>> = std::collections::BTreeMap::new();
    for (i, rule) in rules.iter().enumerate() {
        let r = root(&mut group, i);
        by_root.entry(r).or_default().push(*rule);
    }
    let mut tables = Vec::new();
    let mut frames = Vec::new();
    for g in by_root.into_values().flat_map(stacked) {
        if let Some(t) = grid(&g) {
            tables.push(t);
        } else if let Some(f) = frame(&g) {
            frames.push(f);
        }
    }
    tables.extend(box_rows(frames));
    tables.sort_by(|a, b| b.top().total_cmp(&a.top()));
    tables
}

/// Split a group of rules at horizontal lines where the columns change: some vertical rules end
/// there that do not go on below, and others start there that were not above. Grids stacked on one
/// shared line (a row of boxes sitting on a table) become separate tables; a header row that only
/// merges columns does not.
fn stacked(rules: Vec<Rule>) -> Vec<Vec<Rule>> {
    let vs: Vec<(f32, f32, f32)> =
        rules.iter().filter_map(|r| if let Rule::V { x, y0, y1 } = r { Some((*x, *y0, *y1)) } else { None }).collect();
    let has = |set: &[f32], x: f32| set.iter().any(|&o| (o - x).abs() <= TOL);
    let hy = cluster(rules.iter().filter_map(|r| if let Rule::H { y, .. } = r { Some(*y) } else { None }).collect());
    let mut cuts: Vec<f32> = hy
        .into_iter()
        .filter(|&y| {
            let up: Vec<f32> = vs.iter().filter(|v| v.2 > y + TOL && v.1 <= y + TOL).map(|v| v.0).collect();
            let down: Vec<f32> = vs.iter().filter(|v| v.1 < y - TOL && v.2 >= y - TOL).map(|v| v.0).collect();
            up.iter().any(|&x| !has(&down, x)) && down.iter().any(|&x| !has(&up, x))
        })
        .collect();
    if cuts.is_empty() {
        return vec![rules];
    }
    cuts.insert(0, f32::MIN);
    cuts.push(f32::MAX);
    cuts.windows(2)
        .map(|w| {
            let (lo, hi) = (w[0], w[1]);
            rules
                .iter()
                .filter_map(|r| match *r {
                    Rule::H { y, .. } => (y >= lo - TOL && y <= hi + TOL).then_some(*r),
                    Rule::V { x, y0, y1 } => {
                        let (a, b) = (y0.max(lo), y1.min(hi));
                        (b - a > TOL).then_some(Rule::V { x, y0: a, y1: b })
                    }
                })
                .collect::<Vec<Rule>>()
        })
        .filter(|g| g.iter().any(|r| matches!(r, Rule::V { .. })))
        .collect()
}

/// The rectangle a group of rules draws when it is one closed box (left, bottom, right, top).
fn frame(rules: &[Rule]) -> Option<(f32, f32, f32, f32)> {
    let xs = cluster(rules.iter().filter_map(|r| if let Rule::V { x, .. } = r { Some(*x) } else { None }).collect());
    let ys = cluster(rules.iter().filter_map(|r| if let Rule::H { y, .. } = r { Some(*y) } else { None }).collect());
    if xs.len() != 2 || ys.len() != 2 {
        return None;
    }
    let (l, r, b, t) = (xs[0], xs[1], ys[0], ys[1]);
    let closed =
        drawn_h(rules, t, l, r) && drawn_h(rules, b, l, r) && drawn_v(rules, l, b, t) && drawn_v(rules, r, b, t);
    (closed && r - l >= 12.0 && t - b >= 8.0).then_some((l, b, r, t))
}

/// Framed boxes of the same height side by side (a flow of steps, a row of cards) become one
/// table: a cell per box, and lineless cells for the gaps between them (where arrows sit).
fn box_rows(mut frames: Vec<(f32, f32, f32, f32)>) -> Vec<Table> {
    const SLACK: f32 = 2.0;
    frames.sort_by(|a, b| b.3.total_cmp(&a.3).then(a.0.total_cmp(&b.0)));
    let mut used = vec![false; frames.len()];
    let mut out = Vec::new();
    for i in 0..frames.len() {
        if used[i] {
            continue;
        }
        let (_, b0, _, t0) = frames[i];
        let mut row: Vec<usize> = (i..frames.len())
            .filter(|&j| !used[j] && (frames[j].1 - b0).abs() <= SLACK && (frames[j].3 - t0).abs() <= SLACK)
            .collect();
        row.sort_by(|&a, &b| frames[a].0.total_cmp(&frames[b].0));
        // Side by side, not overlapping, with modest gaps.
        let ok = row.len() >= 2
            && row.windows(2).all(|w| {
                let gap = frames[w[1]].0 - frames[w[0]].2;
                gap >= -0.5 && gap <= 0.75 * (frames[w[0]].2 - frames[w[0]].0).max(frames[w[1]].2 - frames[w[1]].0)
            });
        if !ok {
            continue;
        }
        let mut xs = Vec::new();
        let mut cells = Vec::new();
        for (k, &j) in row.iter().enumerate() {
            used[j] = true;
            let (l, _, r, _) = frames[j];
            if k > 0 {
                let prev = *xs.last().expect("edge");
                if l - prev > 1.0 {
                    cells.push(Cell { row: 0, col: xs.len() - 1, row_span: 1, col_span: 1, open: [true, true] });
                    xs.push(l);
                }
            } else {
                xs.push(l);
            }
            cells.push(Cell { row: 0, col: xs.len() - 1, row_span: 1, col_span: 1, open: [false, false] });
            xs.push(r);
        }
        let top = row.iter().map(|&j| frames[j].3).fold(f32::MIN, f32::max);
        let bottom = row.iter().map(|&j| frames[j].1).fold(f32::MAX, f32::min);
        out.push(Table { xs, ys: vec![top, bottom], cells });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(y: f32, x0: f32, x1: f32) -> Rule {
        Rule::H { y, x0, x1 }
    }
    fn v(x: f32, y0: f32, y1: f32) -> Rule {
        Rule::V { x, y0, y1 }
    }

    #[test]
    fn a_full_grid_is_a_table_in_reading_order() {
        // 3 columns × 2 rows: x 72/200/330/460, y 700/682/664.
        let mut r = vec![];
        for y in [700.0, 682.0, 664.0] {
            r.push(h(y, 72.0, 460.0));
        }
        for x in [72.0, 200.0, 330.0, 460.0] {
            r.push(v(x, 664.0, 700.0));
        }
        let t = find_tables(&r);
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].xs, [72.0, 200.0, 330.0, 460.0]);
        assert_eq!(t[0].ys, [700.0, 682.0, 664.0]);
        assert_eq!(t[0].cells.len(), 6);
        assert_eq!(t[0].cell_at(250.0, 670.0), Some(4), "row 2, column 2");
        assert_eq!(t[0].cell_at(500.0, 670.0), None);
    }

    #[test]
    fn missing_rules_merge_cells_and_split_segments_join() {
        // Header row spans all 3 columns (no inner verticals in it); lines drawn in pieces.
        let r = vec![
            h(700.0, 72.0, 260.0),
            h(700.0, 259.0, 460.0),
            h(682.0, 72.0, 460.0),
            h(664.0, 72.0, 460.0),
            v(72.0, 664.0, 700.0),
            v(460.0, 664.0, 700.0),
            v(200.0, 664.0, 682.0),
            v(330.0, 664.0, 682.0),
        ];
        let t = &find_tables(&r)[0];
        assert_eq!(t.cells[0], Cell { row: 0, col: 0, row_span: 1, col_span: 3, open: [false, false] });
        assert_eq!(t.cells.len(), 4);
    }

    #[test]
    fn frames_lines_and_ticks_are_not_tables() {
        // A framed box (one cell), a lone underline, a tick mark, a slanted line.
        let mut r = rules_from_rect(72.0, 600.0, 300.0, 700.0, false);
        r.push(h(500.0, 72.0, 300.0));
        assert!(rule_from_segment(10.0, 10.0, 12.0, 10.0).is_none(), "too short");
        assert!(rule_from_segment(10.0, 10.0, 50.0, 30.0).is_none(), "slanted");
        assert!(find_tables(&r).is_empty());
        // Thin filled rectangles are rules.
        assert_eq!(rules_from_rect(72.0, 699.5, 460.0, 700.5, true), vec![h(700.0, 72.0, 460.0)]);
        assert!(rules_from_rect(72.0, 600.0, 300.0, 700.0, true).is_empty(), "a filled box is not a rule");
    }

    #[test]
    fn framed_boxes_in_a_row_are_one_table_with_open_gaps() {
        // Four boxes 100 wide with 20-point gaps (a flow of steps), and a lone framed note below.
        let mut r = Vec::new();
        for k in 0..4 {
            let l = 60.0 + 120.0 * k as f32;
            r.extend(rules_from_rect(l, 600.0, l + 100.0, 660.0, false));
        }
        r.extend(rules_from_rect(60.0, 400.0, 400.0, 450.0, false));
        let t = find_tables(&r);
        assert_eq!(t.len(), 1, "the lone note stays a frame");
        assert_eq!(t[0].xs, [60.0, 160.0, 180.0, 280.0, 300.0, 400.0, 420.0, 520.0]);
        assert_eq!(t[0].ys, [660.0, 600.0]);
        let open: Vec<usize> = t[0].cells.iter().filter(|c| c.open == [true, true]).map(|c| c.col).collect();
        assert_eq!(open, [1, 3, 5]);
        assert_eq!(t[0].cells.len(), 7);
        // Boxes of different heights are not a row.
        let mut r = rules_from_rect(60.0, 600.0, 160.0, 660.0, false);
        r.extend(rules_from_rect(180.0, 560.0, 280.0, 660.0, false));
        assert!(find_tables(&r).is_empty());
    }

    #[test]
    fn boxes_sitting_on_a_table_are_a_separate_row() {
        // Two boxes (60 wide, 20 apart) whose bottoms lie on a 3-column table's top line.
        let mut r = rules_from_rect(60.0, 600.0, 120.0, 650.0, false);
        r.extend(rules_from_rect(140.0, 600.0, 200.0, 650.0, false));
        for y in [600.0, 580.0, 560.0] {
            r.push(h(y, 60.0, 300.0));
        }
        for x in [60.0, 100.0, 220.0, 300.0] {
            r.push(v(x, 560.0, 600.0));
        }
        let t = find_tables(&r);
        assert_eq!(t.len(), 2, "{t:?}");
        assert_eq!(t[1].xs, [60.0, 100.0, 220.0, 300.0], "the table keeps its three columns");
        assert_eq!(t[1].cells.len(), 6);
        assert_eq!(t[0].xs, [60.0, 120.0, 140.0, 200.0], "the boxes: {:?}", t[0]);
    }

    #[test]
    fn segments_on_one_edge_join_in_order_whatever_tiny_offsets_they_have() {
        // A box's right edge above a table's right edge, a hair apart in x (as PDF writers round).
        let r = merge(vec![v(538.58264, 610.0, 668.0), v(538.5827, 449.0, 602.0), v(538.5826, 300.0, 448.0)]);
        assert_eq!(r.len(), 2, "{r:?}");
        assert!(r.iter().any(|x| matches!(x, Rule::V { y0, y1, .. } if *y0 == 300.0 && *y1 == 602.0)), "{r:?}");
        assert!(r.iter().any(|x| matches!(x, Rule::V { y0, y1, .. } if *y0 == 610.0 && *y1 == 668.0)), "{r:?}");
    }

    #[test]
    fn two_separate_grids_are_two_tables_top_first() {
        let grid_at = |top: f32| {
            let mut r = vec![];
            for y in [top, top - 20.0, top - 40.0] {
                r.push(h(y, 72.0, 300.0));
            }
            for x in [72.0, 186.0, 300.0] {
                r.push(v(x, top - 40.0, top));
            }
            r
        };
        let mut r = grid_at(400.0);
        r.extend(grid_at(700.0));
        let t = find_tables(&r);
        assert_eq!(t.len(), 2);
        assert_eq!(t[0].top(), 700.0);
    }
}
