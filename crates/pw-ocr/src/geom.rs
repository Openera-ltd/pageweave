//! Geometry and image primitives for the detector and recogniser (no OpenCV dependency).
//! Resizing follows OpenCV's INTER_LINEAR sampling convention (pixel centres at +0.5) so results
//! match the reference pipeline closely; exact agreement is checked by the parity benchmark.

/// A 3-channel 8-bit image in BGR order (the order the PP-OCR models were trained on).
#[derive(Clone, Debug)]
pub struct Bgr {
    pub w: usize,
    pub h: usize,
    pub data: Vec<u8>,
}

impl Bgr {
    pub fn new(w: usize, h: usize) -> Self {
        Bgr { w, h, data: vec![0; w * h * 3] }
    }

    /// From RGB(A) pixels, `channels` 3 or 4.
    pub fn from_rgb(w: usize, h: usize, px: &[u8], channels: usize) -> Self {
        let mut out = Bgr::new(w, h);
        for i in 0..w * h {
            let s = &px[i * channels..i * channels + 3];
            out.data[i * 3] = s[2];
            out.data[i * 3 + 1] = s[1];
            out.data[i * 3 + 2] = s[0];
        }
        out
    }

    #[inline]
    fn px(&self, x: usize, y: usize, c: usize) -> f32 {
        self.data[(y * self.w + x) * 3 + c] as f32
    }
}

/// Bilinear resize (OpenCV INTER_LINEAR convention).
pub fn resize(src: &Bgr, w: usize, h: usize) -> Bgr {
    let mut out = Bgr::new(w, h);
    if src.w == 0 || src.h == 0 || w == 0 || h == 0 {
        return out;
    }
    let sx = src.w as f32 / w as f32;
    let sy = src.h as f32 / h as f32;
    let xs: Vec<(usize, usize, f32)> = (0..w)
        .map(|x| {
            let fx = ((x as f32 + 0.5) * sx - 0.5).max(0.0);
            let x0 = (fx as usize).min(src.w - 1);
            let x1 = (x0 + 1).min(src.w - 1);
            (x0, x1, fx - x0 as f32)
        })
        .collect();
    for y in 0..h {
        let fy = ((y as f32 + 0.5) * sy - 0.5).max(0.0);
        let y0 = (fy as usize).min(src.h - 1);
        let y1 = (y0 + 1).min(src.h - 1);
        let wy = fy - y0 as f32;
        for (x, &(x0, x1, wx)) in xs.iter().enumerate() {
            for c in 0..3 {
                let top = src.px(x0, y0, c) * (1.0 - wx) + src.px(x1, y0, c) * wx;
                let bot = src.px(x0, y1, c) * (1.0 - wx) + src.px(x1, y1, c) * wx;
                out.data[(y * w + x) * 3 + c] = (top * (1.0 - wy) + bot * wy).round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    out
}

pub type Pt = (f32, f32);

/// Convex hull (Andrew's monotone chain), counter-clockwise, no duplicate end point.
pub fn convex_hull(points: &[Pt]) -> Vec<Pt> {
    let mut p: Vec<Pt> = points.to_vec();
    p.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
    p.dedup();
    if p.len() < 3 {
        return p;
    }
    let cross = |o: Pt, a: Pt, b: Pt| (a.0 - o.0) * (b.1 - o.1) - (a.1 - o.1) * (b.0 - o.0);
    let mut lower: Vec<Pt> = Vec::new();
    for &q in &p {
        while lower.len() >= 2 && cross(lower[lower.len() - 2], lower[lower.len() - 1], q) <= 0.0 {
            lower.pop();
        }
        lower.push(q);
    }
    let mut upper: Vec<Pt> = Vec::new();
    for &q in p.iter().rev() {
        while upper.len() >= 2 && cross(upper[upper.len() - 2], upper[upper.len() - 1], q) <= 0.0 {
            upper.pop();
        }
        upper.push(q);
    }
    lower.pop();
    upper.pop();
    lower.extend(upper);
    lower
}

/// Minimum-area enclosing rectangle: (centre, width along `dir`, height, unit direction of width).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub cx: f32,
    pub cy: f32,
    pub w: f32,
    pub h: f32,
    pub ux: f32,
    pub uy: f32,
}

impl Rect {
    pub fn short_side(&self) -> f32 {
        self.w.min(self.h)
    }

    /// Grow every side by `d`.
    pub fn grow(&self, d: f32) -> Rect {
        Rect { w: self.w + 2.0 * d, h: self.h + 2.0 * d, ..*self }
    }

    pub fn corners(&self) -> [Pt; 4] {
        let (ux, uy) = (self.ux, self.uy);
        let (vx, vy) = (-uy, ux);
        let (hw, hh) = (self.w / 2.0, self.h / 2.0);
        [
            (self.cx - ux * hw - vx * hh, self.cy - uy * hw - vy * hh),
            (self.cx + ux * hw - vx * hh, self.cy + uy * hw - vy * hh),
            (self.cx + ux * hw + vx * hh, self.cy + uy * hw + vy * hh),
            (self.cx - ux * hw + vx * hh, self.cy - uy * hw + vy * hh),
        ]
    }
}

/// Rotating calipers over the hull edges.
pub fn min_area_rect(points: &[Pt]) -> Option<Rect> {
    let hull = convex_hull(points);
    match hull.len() {
        0 => return None,
        1 => return Some(Rect { cx: hull[0].0, cy: hull[0].1, w: 0.0, h: 0.0, ux: 1.0, uy: 0.0 }),
        _ => {}
    }
    let mut best: Option<(f32, Rect)> = None;
    for i in 0..hull.len() {
        let a = hull[i];
        let b = hull[(i + 1) % hull.len()];
        let len = ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt();
        if len < 1e-6 {
            continue;
        }
        let (ux, uy) = ((b.0 - a.0) / len, (b.1 - a.1) / len);
        let (vx, vy) = (-uy, ux);
        let (mut u0, mut u1, mut v0, mut v1) = (f32::MAX, f32::MIN, f32::MAX, f32::MIN);
        for p in &hull {
            let u = p.0 * ux + p.1 * uy;
            let v = p.0 * vx + p.1 * vy;
            u0 = u0.min(u);
            u1 = u1.max(u);
            v0 = v0.min(v);
            v1 = v1.max(v);
        }
        let area = (u1 - u0) * (v1 - v0);
        if best.as_ref().is_none_or(|(a, _)| area < *a) {
            let (cu, cv) = ((u0 + u1) / 2.0, (v0 + v1) / 2.0);
            let rect = Rect { cx: cu * ux + cv * vx, cy: cu * uy + cv * vy, w: u1 - u0, h: v1 - v0, ux, uy };
            best = Some((area, rect));
        }
    }
    best.map(|(_, r)| r)
}

/// Order four points as top-left, top-right, bottom-right, bottom-left (image coordinates, y down):
/// the two left-most points split by y, then the two right-most.
pub fn order_clockwise(pts: [Pt; 4]) -> [Pt; 4] {
    let mut by_x = pts;
    by_x.sort_by(|a, b| a.0.total_cmp(&b.0));
    let (mut left, mut right) = ([by_x[0], by_x[1]], [by_x[2], by_x[3]]);
    left.sort_by(|a, b| a.1.total_cmp(&b.1));
    right.sort_by(|a, b| a.1.total_cmp(&b.1));
    [left[0], right[0], right[1], left[1]]
}

pub fn dist(a: Pt, b: Pt) -> f32 {
    ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt()
}

/// Whether `p` lies inside or on the convex quadrilateral `q` (any winding).
pub fn in_convex(q: &[Pt; 4], p: Pt) -> bool {
    let mut sign = 0.0f32;
    for i in 0..4 {
        let a = q[i];
        let b = q[(i + 1) % 4];
        let c = (b.0 - a.0) * (p.1 - a.1) - (b.1 - a.1) * (p.0 - a.0);
        if c.abs() < 1e-4 {
            continue;
        }
        if sign == 0.0 {
            sign = c.signum();
        } else if c.signum() != sign {
            return false;
        }
    }
    true
}

/// Homography mapping `dst` rectangle corners (0,0),(w,0),(w,h),(0,h) to the `src` quadrilateral
/// (inverse mapping for sampling). Returns the 3×3 matrix row-major.
fn homography(src: &[Pt; 4], w: f32, h: f32) -> Option<[f32; 9]> {
    let dst = [(0.0, 0.0), (w, 0.0), (w, h), (0.0, h)];
    // Solve for H with dst → src: 8 unknowns.
    let mut a = [[0f64; 9]; 8];
    for i in 0..4 {
        let (x, y) = (dst[i].0 as f64, dst[i].1 as f64);
        let (u, v) = (src[i].0 as f64, src[i].1 as f64);
        a[2 * i] = [x, y, 1.0, 0.0, 0.0, 0.0, -u * x, -u * y, u];
        a[2 * i + 1] = [0.0, 0.0, 0.0, x, y, 1.0, -v * x, -v * y, v];
    }
    // Gaussian elimination with partial pivoting.
    for col in 0..8 {
        let piv = (col..8).max_by(|&r1, &r2| a[r1][col].abs().total_cmp(&a[r2][col].abs()))?;
        if a[piv][col].abs() < 1e-12 {
            return None;
        }
        a.swap(col, piv);
        let pivot_row = a[col];
        for (r, row) in a.iter_mut().enumerate() {
            if r != col {
                let f = row[col] / pivot_row[col];
                for (v, p) in row.iter_mut().zip(pivot_row.iter()).skip(col) {
                    *v -= f * p;
                }
            }
        }
    }
    let mut h9 = [0f32; 9];
    for i in 0..8 {
        h9[i] = (a[i][8] / a[i][i]) as f32;
    }
    h9[8] = 1.0;
    Some(h9)
}

/// Cubic convolution weights (OpenCV INTER_CUBIC, a = −0.75) for fractional offset `t`.
fn cubic_weights(t: f32) -> [f32; 4] {
    const A: f32 = -0.75;
    let w0 = ((A * (t + 1.0) - 5.0 * A) * (t + 1.0) + 8.0 * A) * (t + 1.0) - 4.0 * A;
    let w1 = ((A + 2.0) * t - (A + 3.0)) * t * t + 1.0;
    let w2 = ((A + 2.0) * (1.0 - t) - (A + 3.0)) * (1.0 - t) * (1.0 - t) + 1.0;
    [w0, w1, w2, 1.0 - w0 - w1 - w2]
}

/// Crop the quadrilateral `q` (tl, tr, br, bl) into an upright `w`×`h` image (bicubic sampling,
/// replicated border — as the reference pipeline's perspective warp).
pub fn warp_quad(src: &Bgr, q: &[Pt; 4], w: usize, h: usize) -> Bgr {
    let mut out = Bgr::new(w, h);
    let Some(m) = homography(q, w as f32, h as f32) else { return out };
    let (sw, sh) = (src.w as f32, src.h as f32);
    for y in 0..h {
        for x in 0..w {
            let (fx, fy) = (x as f32, y as f32);
            let z = m[6] * fx + m[7] * fy + m[8];
            let u = ((m[0] * fx + m[1] * fy + m[2]) / z).clamp(0.0, sw - 1.0);
            let v = ((m[3] * fx + m[4] * fy + m[5]) / z).clamp(0.0, sh - 1.0);
            let (ux, vy) = (u.floor(), v.floor());
            let (wx, wy) = (cubic_weights(u - ux), cubic_weights(v - vy));
            let clampx = |i: isize| i.clamp(0, src.w as isize - 1) as usize;
            let clampy = |i: isize| i.clamp(0, src.h as isize - 1) as usize;
            for c in 0..3 {
                let mut acc = 0.0f32;
                for (j, wyj) in wy.iter().enumerate() {
                    let sy = clampy(vy as isize - 1 + j as isize);
                    let mut row = 0.0f32;
                    for (i, wxi) in wx.iter().enumerate() {
                        row += wxi * src.px(clampx(ux as isize - 1 + i as isize), sy, c);
                    }
                    acc += wyj * row;
                }
                out.data[(y * w + x) * 3 + c] = acc.round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    out
}

/// Rotate 90° counter-clockwise (numpy `rot90`).
pub fn rot90(src: &Bgr) -> Bgr {
    let mut out = Bgr::new(src.h, src.w);
    for y in 0..src.h {
        for x in 0..src.w {
            let (nx, ny) = (y, src.w - 1 - x);
            for c in 0..3 {
                out.data[(ny * out.w + nx) * 3 + c] = src.data[(y * src.w + x) * 3 + c];
            }
        }
    }
    out
}

/// Python-style `round` (half to even), as used by the reference pipeline's size rounding.
pub fn round_half_even(x: f64) -> f64 {
    let r = x.round();
    if (x - x.trunc()).abs() == 0.5 && r % 2.0 != 0.0 {
        r - x.signum()
    } else {
        r
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn min_area_rect_of_a_rotated_box() {
        let r0 = Rect { cx: 50.0, cy: 40.0, w: 60.0, h: 10.0, ux: (0.2f32).cos(), uy: (0.2f32).sin() };
        let mut pts: Vec<Pt> = r0.corners().to_vec();
        pts.push((50.0, 40.0));
        let r = min_area_rect(&pts).expect("rect");
        assert!((r.w * r.h - 600.0).abs() < 1.0, "{r:?}");
        assert!((r.cx - 50.0).abs() < 0.01 && (r.cy - 40.0).abs() < 0.01);
        assert!((r.short_side() - 10.0).abs() < 0.01);
    }

    #[test]
    fn ordering_and_containment() {
        let q = order_clockwise([(10.0, 20.0), (0.0, 0.0), (10.0, 0.0), (0.0, 20.0)]);
        assert_eq!(q, [(0.0, 0.0), (10.0, 0.0), (10.0, 20.0), (0.0, 20.0)]);
        assert!(in_convex(&q, (5.0, 5.0)) && in_convex(&q, (0.0, 0.0)) && !in_convex(&q, (11.0, 5.0)));
    }

    #[test]
    fn cubic_weights_sum_to_one_and_interpolate() {
        for t in [0.0f32, 0.25, 0.5, 0.9] {
            let w = cubic_weights(t);
            assert!((w.iter().sum::<f32>() - 1.0).abs() < 1e-5);
        }
        assert_eq!(cubic_weights(0.0), [0.0, 1.0, 0.0, 0.0]);
    }

    #[test]
    fn python_rounding() {
        assert_eq!(round_half_even(62.5), 62.0);
        assert_eq!(round_half_even(63.5), 64.0);
        assert_eq!(round_half_even(2.4), 2.0);
    }

    #[test]
    fn warp_of_an_axis_aligned_quad_is_a_crop() {
        let mut img = Bgr::new(20, 10);
        for y in 0..10 {
            for x in 0..20 {
                img.data[(y * 20 + x) * 3] = (x * 10) as u8;
            }
        }
        let crop = warp_quad(&img, &[(5.0, 2.0), (15.0, 2.0), (15.0, 7.0), (5.0, 7.0)], 10, 5);
        assert_eq!(crop.data[0], 50);
        assert_eq!(crop.data[9 * 3], 140);
        let r = rot90(&crop);
        assert_eq!((r.w, r.h), (5, 10));
        assert_eq!(r.data[(9 * 5) * 3], 50, "left column ends at the bottom after a CCW turn");
    }

    #[test]
    fn resize_preserves_flat_regions() {
        let mut img = Bgr::new(8, 8);
        img.data.iter_mut().for_each(|v| *v = 200);
        let r = resize(&img, 3, 5);
        assert!(r.data.iter().all(|&v| v == 200));
    }
}
