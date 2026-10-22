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
    let join = |mut v: Vec<(f32, f32, f32)>| {
        v.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
        let mut out: Vec<(f32, f32, f32)> = Vec::new();
        for s in v {
            if let Some(last) = out.last_mut() {
                if (s.0 - last.0).abs() <= 1.0 && s.1 <= last.2 + TOL {
                    last.2 = last.2.max(s.2);
                    continue;
                }
            }
            out.push(s);
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
            cells.push(Cell { row: r, col: c, row_span: rs, col_span: cs });
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
    let mut tables: Vec<Table> = by_root.into_values().filter_map(|g| grid(&g)).collect();
    tables.sort_by(|a, b| b.top().total_cmp(&a.top()));
    tables
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
        assert_eq!(t.cells[0], Cell { row: 0, col: 0, row_span: 1, col_span: 3 });
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
