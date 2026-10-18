//! Page orientation for scans (PRD 06 "detect orientation"). Upright text reads with high
//! confidence as horizontal lines; the recogniser also reads tall (sideways) boxes by turning their
//! crops, so only horizontal lines count. A first pass on the page as it is tells "upright or
//! upside down" (mostly horizontal lines) from "sideways" (mostly tall lines); the two candidate
//! turns are then compared, and a turn is only chosen by a clear margin. The correction is lossless
//! (the page's /Rotate).

use image::{imageops, RgbImage};
use pw_core::PwResult;

/// Longest side of the image recognised for orientation, in pixels.
pub const ORIENT_PIXELS: u32 = 1000;
/// The chosen turn must read at least this much better than the other candidate.
const MIN_MARGIN: f32 = 1.5;
/// ... and amount to at least this many confidently read characters.
const MIN_QUALITY: f32 = 20.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Orientation {
    /// Clockwise quarter turn (0, 90, 180, 270) that makes the text upright as displayed.
    pub turn: u32,
    /// Horizontal-line quality of the chosen turn and of the other candidate.
    pub quality: f32,
    pub runner_up: f32,
}

/// Confidence-weighted characters on horizontal lines, and on tall lines.
fn read(engine: &mut pw_ocr::OcrEngine, img: &RgbImage, cancelled: &dyn Fn() -> bool) -> PwResult<(f32, f32)> {
    let lines = engine.recognize(img.width() as usize, img.height() as usize, img.as_raw(), 3, cancelled)?;
    let (mut horizontal, mut tall) = (0f32, 0f32);
    for l in &lines {
        let d = |a: [f32; 2], b: [f32; 2]| (a[0] - b[0]).hypot(a[1] - b[1]);
        let (w, h) = (d(l.quad[0], l.quad[1]), d(l.quad[1], l.quad[2]));
        let q = l.score * l.text.chars().filter(|c| !c.is_whitespace()).count() as f32;
        if w >= h {
            horizontal += q;
        } else {
            tall += q;
        }
    }
    Ok((horizontal, tall))
}

fn turned(img: &RgbImage, turn: u32) -> RgbImage {
    match turn {
        90 => imageops::rotate90(img),
        180 => imageops::rotate180(img),
        270 => imageops::rotate270(img),
        _ => img.clone(),
    }
}

/// The upright turn of a page image, or `None` when it cannot be told with confidence.
pub fn detect_orientation(
    engine: &mut pw_ocr::OcrEngine,
    img: &RgbImage,
    cancelled: &dyn Fn() -> bool,
) -> PwResult<Option<Orientation>> {
    let k = ORIENT_PIXELS as f32 / img.width().max(img.height()).max(1) as f32;
    let small = if k < 1.0 {
        imageops::resize(
            img,
            (img.width() as f32 * k) as u32,
            (img.height() as f32 * k) as u32,
            imageops::FilterType::Triangle,
        )
    } else {
        img.clone()
    };
    // All four turns are read: shortcuts from fewer readings were wrong on skewed pages, where text a
    // quarter turn off still reads partly (val corpus, docs/benchmarks/ORIENTATION.md).
    let mut q = [0f32; 4];
    for (i, t) in [0u32, 90, 180, 270].into_iter().enumerate() {
        q[i] = read(engine, &turned(&small, t), cancelled)?.0;
    }
    let mut order = [0usize, 1, 2, 3];
    order.sort_by(|&x, &y| q[y].total_cmp(&q[x]));
    let (turn, best, other) = (order[0] as u32 * 90, q[order[0]], q[order[1]]);
    Ok((best >= MIN_QUALITY && best >= other * MIN_MARGIN).then_some(Orientation {
        turn,
        quality: best,
        runner_up: other,
    }))
}
