//! Straighten scanned pages (PRD 06 "scan cleanup: deskew"). The skew is measured on a render of
//! the page: dark pixels are projected onto the vertical axis at candidate angles and the angle
//! whose profile is sharpest (text lines collapse into narrow peaks) wins. The correction is
//! lossless: the page content is wrapped in a rotation about the page centre, so images are not
//! resampled and an existing text layer (for example from OCR) turns with them.

use crate::document::OpenOptions;
use crate::sanitize::load_plain;
use crate::{engine, map_pdfium_error};
use image::GrayImage;
use lopdf::{Object, Stream};
use pdfium_render::prelude::*;
use pw_core::output::{write_atomic_bytes, CollisionPolicy, CommittedOutput};
use pw_core::{ErrorCategory, PwError, PwResult};
use serde::Serialize;
use std::path::Path;

/// Longest side of the measuring render, in pixels.
const MEASURE_PIXELS: f32 = 1400.0;
/// A corrected page must measure within this of straight (degrees).
const MAX_RESIDUAL_DEG: f32 = 0.3;
/// Pages with more visible text characters than this are born-digital, not scans.
const MAX_VISIBLE_CHARS: usize = 20;

/// Largest skew corrected (degrees); beyond this a page is more likely turned or a drawing.
pub const MAX_SKEW_DEG: f32 = 15.0;
/// Below this a page counts as straight (degrees). Above the measurement's noise on a corrected
/// page (±0.1°), so straightening a straightened copy finds nothing to do; 0.2° is ~0.7 mm across a
/// Letter page.
pub const STRAIGHT_DEG: f32 = 0.2;
/// The sharpest profile must beat the average over all candidate angles by this factor.
const MIN_CONFIDENCE: f32 = 1.15;
/// Dark pixels sampled at most (spread evenly over the page).
const MAX_POINTS: usize = 250_000;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Skew {
    /// Angle of the text lines as displayed, degrees; positive = rising to the right
    /// (the page content is turned counter-clockwise).
    pub angle: f32,
    /// Sharpness of the best profile relative to the average; higher is more certain.
    pub confidence: f32,
}

/// Otsu threshold of a grey image: values at or below it are the dark class.
fn otsu(img: &GrayImage) -> u8 {
    let mut hist = [0u64; 256];
    for p in img.pixels() {
        hist[p.0[0] as usize] += 1;
    }
    let total: u64 = hist.iter().sum();
    let sum: f64 = hist.iter().enumerate().map(|(i, &h)| i as f64 * h as f64).sum();
    let (mut w_b, mut sum_b, mut best, mut threshold) = (0f64, 0f64, 0f64, 128u8);
    for (t, &h) in hist.iter().enumerate() {
        w_b += h as f64;
        if w_b == 0.0 {
            continue;
        }
        let w_f = total as f64 - w_b;
        if w_f == 0.0 {
            break;
        }
        sum_b += t as f64 * h as f64;
        let (m_b, m_f) = (sum_b / w_b, (sum - sum_b) / w_f);
        let between = w_b * w_f * (m_b - m_f) * (m_b - m_f);
        if between > best {
            best = between;
            threshold = t as u8;
        }
    }
    threshold
}

fn score(points: &[(f32, f32)], angle_deg: f32, height: f32) -> f64 {
    let (s, c) = angle_deg.to_radians().sin_cos();
    // Project onto the axis perpendicular to lines at this angle (image y grows downwards; a line
    // rising to the right has y decreasing with x, hence + x·sin).
    let span = height * 1.5;
    let offset = height * 0.25;
    let bins = span as usize + 2;
    let mut hist = vec![0u32; bins];
    for &(x, y) in points {
        let v = y * c + x * s + offset;
        if v >= 0.0 && (v as usize) < bins {
            hist[v as usize] += 1;
        }
    }
    hist.iter().map(|&h| f64::from(h) * f64::from(h)).sum()
}

/// The skew of a page render, or `None` when there is too little content to tell.
pub fn measure_skew(img: &GrayImage) -> Option<Skew> {
    let t = otsu(img);
    // Pages that are almost entirely one tone have no lines to measure.
    let dark: usize = img.pixels().filter(|p| p.0[0] <= t).count();
    let n = (img.width() * img.height()) as usize;
    if dark < n / 1000 || dark > n / 2 {
        return None;
    }
    let step = (dark / MAX_POINTS).max(1);
    let points: Vec<(f32, f32)> = img
        .enumerate_pixels()
        .filter(|(_, _, p)| p.0[0] <= t)
        .step_by(step)
        .map(|(x, y, _)| (x as f32, y as f32))
        .collect();
    let h = img.height() as f32;
    let mut coarse: Vec<(f32, f64)> = Vec::new();
    let mut a = -MAX_SKEW_DEG;
    while a <= MAX_SKEW_DEG + 1e-3 {
        coarse.push((a, score(&points, a, h)));
        a += 0.5;
    }
    let mean = coarse.iter().map(|c| c.1).sum::<f64>() / coarse.len() as f64;
    let best = coarse.iter().copied().max_by(|x, y| x.1.total_cmp(&y.1))?;
    let mut fine = best;
    let mut a = best.0 - 0.5;
    while a <= best.0 + 0.5 + 1e-3 {
        let s = score(&points, a, h);
        if s > fine.1 {
            fine = (a, s);
        }
        a += 0.05;
    }
    let confidence = (fine.1 / mean.max(1.0)) as f32;
    (confidence >= MIN_CONFIDENCE).then_some(Skew { angle: (fine.0 * 100.0).round() / 100.0, confidence })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DeskewAction {
    /// Turned by `angle` to straight.
    Straightened,
    /// Already straight (within 0.1°).
    Straight,
    /// Has visible text: a born-digital page, left as it is.
    HasText,
    /// No lines to measure (blank, a picture, or a skew beyond 15°).
    Unmeasured,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DeskewPage {
    /// 0-based page index.
    pub page: u32,
    pub action: DeskewAction,
    /// Skew found (degrees, positive = rising to the right), when measured.
    pub angle: Option<f32>,
    /// Skew measured on the corrected page.
    pub residual: Option<f32>,
}

fn render_gray(page: &PdfPage) -> PwResult<GrayImage> {
    let longest = page.width().value.max(page.height().value).max(1.0);
    let config = PdfRenderConfig::new()
        .scale_page_by_factor(MEASURE_PIXELS / longest)
        .render_form_data(false)
        .render_annotations(false);
    let bitmap = page.render_with_config(&config).map_err(|e| map_pdfium_error(&e, "render"))?;
    Ok(bitmap.as_image().map_err(|e| map_pdfium_error(&e, "render"))?.to_luma8())
}

fn visible_chars(page: &PdfPage) -> usize {
    page.objects()
        .iter()
        .filter_map(|o| o.as_text_object().map(|t| (t.render_mode(), t.text())))
        .filter(|(mode, _)| *mode != PdfPageTextRenderMode::Invisible)
        .map(|(_, text)| text.chars().filter(|c| !c.is_whitespace()).count())
        .sum()
}

fn fail(what: &str) -> PwError {
    PwError::new(
        ErrorCategory::ValidationFailed,
        "output.verify",
        format!("The straightened copy failed verification ({what}) and was not saved."),
    )
}

/// Straighten the chosen (or all) scanned pages into a new file.
pub fn deskew_pages(
    input: &Path,
    opts: &OpenOptions,
    pages: Option<&[u32]>,
    output: &Path,
    policy: CollisionPolicy,
    cancelled: &dyn Fn() -> bool,
    progress: &dyn Fn(u64, u64),
) -> PwResult<(CommittedOutput, Vec<DeskewPage>, Vec<String>)> {
    let pdfium = engine()?;
    let (source, mut doc, _raw) = load_plain(&pdfium, input, opts).map_err(|e| {
        if e.code == "sanitize.protected" {
            PwError::new(ErrorCategory::UnsupportedOperation, "deskew.protected", "This file is password protected.")
                .with_action("Remove the password first (Remove password…), then straighten that copy.")
        } else {
            e
        }
    })?;
    let n = source.pages().len().max(0) as u32;
    let chosen: Vec<u32> = match pages {
        Some(p) => p.to_vec(),
        None => (0..n).collect(),
    };
    let total = chosen.len() as u64 * 2;
    let page_ids = doc.get_pages();
    let mut report = Vec::new();
    let mut annotated = 0;
    for (k, &i) in chosen.iter().enumerate() {
        if cancelled() {
            return Err(PwError::new(ErrorCategory::Cancelled, "job.cancelled", "Straightening was cancelled."));
        }
        let page = source.pages().get(i as PdfPageIndex).map_err(|e| map_pdfium_error(&e, "page"))?;
        progress(k as u64 + 1, total);
        if visible_chars(&page) > MAX_VISIBLE_CHARS {
            report.push(DeskewPage { page: i, action: DeskewAction::HasText, angle: None, residual: None });
            continue;
        }
        let Some(skew) = measure_skew(&render_gray(&page)?) else {
            report.push(DeskewPage { page: i, action: DeskewAction::Unmeasured, angle: None, residual: None });
            continue;
        };
        if skew.angle.abs() < STRAIGHT_DEG {
            report.push(DeskewPage {
                page: i,
                action: DeskewAction::Straight,
                angle: Some(skew.angle),
                residual: None,
            });
            continue;
        }
        // Content rising to the right by `angle` turns clockwise by it: in PDF space (y up) that
        // is a rotation by -angle about the centre of the displayed box. A /Rotate of the page
        // does not change this (rotations commute).
        let b = crate::redact::displayed_box(&page);
        let (cx, cy) = ((b.l + b.r) / 2.0, (b.b + b.t) / 2.0);
        let (sn, cs) = (-skew.angle).to_radians().sin_cos();
        let e = cx - cx * cs + cy * sn;
        let f = cy - cx * sn - cy * cs;
        let pre = format!("q {cs:.6} {sn:.6} {:.6} {cs:.6} {e:.4} {f:.4} cm\n", -sn).into_bytes();
        let Some(&page_id) = page_ids.get(&(i + 1)) else { return Err(fail("page missing")) };
        let pre_id = doc.add_object(Stream::new(lopdf::Dictionary::new(), pre));
        let post_id = doc.add_object(Stream::new(lopdf::Dictionary::new(), b"\nQ".to_vec()));
        let mut contents: Vec<Object> = vec![Object::Reference(pre_id)];
        contents.extend(doc.get_page_contents(page_id).into_iter().map(Object::Reference));
        contents.push(Object::Reference(post_id));
        let dict = doc.get_dictionary_mut(page_id).map_err(|_| fail("page unreadable"))?;
        dict.set("Contents", Object::Array(contents));
        if !page.annotations().is_empty() {
            annotated += 1;
        }
        report.push(DeskewPage {
            page: i,
            action: DeskewAction::Straightened,
            angle: Some(skew.angle),
            residual: None,
        });
    }
    let changed: Vec<u32> = report.iter().filter(|r| r.action == DeskewAction::Straightened).map(|r| r.page).collect();
    if changed.is_empty() {
        return Err(PwError::new(
            ErrorCategory::UnsupportedOperation,
            "deskew.nothing",
            "No page needed straightening, so nothing was saved.",
        )
        .with_action("Pages with text of their own, blank pages and pages already straight are left as they are."));
    }
    doc.trailer.remove(b"Prev");
    doc.trailer.remove(b"XRefStm");
    let mut bytes = Vec::new();
    doc.save_to(&mut bytes)
        .map_err(|_| PwError::internal("deskew.save", "The straightened copy could not be written."))?;

    // Verify: same pages, and every corrected page now measures straight.
    {
        let out = pdfium.load_pdf_from_byte_slice(&bytes, None).map_err(|_| fail("it could not be reopened"))?;
        if out.pages().len() as u32 != n {
            return Err(fail("page count changed"));
        }
        for (k, r) in report.iter_mut().enumerate() {
            if r.action != DeskewAction::Straightened {
                continue;
            }
            if cancelled() {
                return Err(PwError::new(ErrorCategory::Cancelled, "job.cancelled", "Straightening was cancelled."));
            }
            progress(chosen.len() as u64 + k as u64 + 1, total);
            let page = out.pages().get(r.page as PdfPageIndex).map_err(|_| fail("page unreadable"))?;
            let residual = measure_skew(&render_gray(&page)?).map(|s| s.angle);
            match residual {
                Some(a) if a.abs() <= MAX_RESIDUAL_DEG => r.residual = Some(a),
                _ => return Err(fail(&format!("page {} is not straight afterwards", r.page + 1))),
            }
        }
    }
    let protected_sources = [input.to_path_buf()];
    let c = write_atomic_bytes(output, policy, &protected_sources, &bytes, |staged| {
        let back = std::fs::read(staged).map_err(|e| PwError::from_io(&e, "output file"))?;
        if back == bytes {
            Ok(())
        } else {
            Err(fail("the written file differs"))
        }
    })?;
    let mut warnings = Vec::new();
    if annotated > 0 {
        warnings.push(format!(
            "{annotated} straightened page(s) have comments or links; those keep their original position."
        ));
    }
    let signatures = source.signatures().len();
    if signatures > 0 {
        warnings.push(crate::properties::signature_warning(signatures));
    }
    Ok((c, report, warnings))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Lines of "text" (dark bars with gaps) rising at `angle` degrees.
    fn page(angle: f32) -> GrayImage {
        let (w, h) = (800u32, 1000u32);
        let mut img = GrayImage::from_pixel(w, h, image::Luma([255]));
        let t = angle.to_radians().tan();
        for line in 0..30 {
            let y0 = 100.0 + line as f32 * 28.0;
            for x in 80..720u32 {
                if (x / 9) % 5 == 4 {
                    continue; // word gaps
                }
                let y = y0 - x as f32 * t;
                for dy in 0..8 {
                    let yy = (y + dy as f32) as i64;
                    if yy >= 0 && (yy as u32) < h {
                        img.put_pixel(x, yy as u32, image::Luma([20]));
                    }
                }
            }
        }
        img
    }

    #[test]
    fn measures_known_angles() {
        for want in [0.0f32, 0.8, -1.5, 3.0, -7.25] {
            let s = measure_skew(&page(want)).expect("measurable");
            assert!((s.angle - want).abs() <= 0.1, "want {want}, got {s:?}");
            assert!(s.confidence > MIN_CONFIDENCE);
        }
    }

    #[test]
    fn blank_and_solid_pages_are_not_measured() {
        assert!(measure_skew(&GrayImage::from_pixel(400, 500, image::Luma([255]))).is_none());
        assert!(measure_skew(&GrayImage::from_pixel(400, 500, image::Luma([0]))).is_none());
    }
}
