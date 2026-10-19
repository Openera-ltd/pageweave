//! Find blank pages (PRD 01 "remove blank pages with preview/threshold"): a page is blank when it
//! has no visible text, no annotations, and the share of its pixels that differ clearly from the
//! page's own background (its most common tone, so off-white scans count) is under a threshold.
//! Read-only; removal goes through the verified delete-pages operation.

use crate::document::{load, OpenOptions};
use crate::{engine, map_pdfium_error};
use pdfium_render::prelude::*;
use pw_core::{ErrorCategory, PwError, PwResult};
use serde::Serialize;
use std::path::Path;

/// Longest side of the analysis render, in pixels (~100 dpi for Letter).
const ANALYSIS_PIXELS: f32 = 1100.0;
/// A pixel is ink when it differs from the background by more than this (0–255). Light grey rules
/// (~0.8 grey, 46 levels) are content; scanner noise and off-white paper are not.
const INK_DELTA: u8 = 32;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PageInk {
    /// 0-based page index.
    pub page: u32,
    /// Share of pixels that are ink (0–1).
    pub ink: f32,
    pub text_chars: usize,
    pub annotations: usize,
    pub blank: bool,
}

/// Every page's ink share, and whether it counts as blank at `max_ink`.
pub fn find_blank_pages(
    input: &Path,
    opts: &OpenOptions,
    max_ink: f32,
    cancelled: &dyn Fn() -> bool,
    progress: &dyn Fn(u64, u64),
) -> PwResult<Vec<PageInk>> {
    let pdfium = engine()?;
    let doc = load(&pdfium, input, opts)?;
    let n = doc.pages().len().max(0) as u32;
    let mut out = Vec::with_capacity(n as usize);
    for i in 0..n {
        if cancelled() {
            return Err(PwError::new(
                ErrorCategory::Cancelled,
                "job.cancelled",
                "Looking for blank pages was cancelled.",
            ));
        }
        let page = doc.pages().get(i as PdfPageIndex).map_err(|e| map_pdfium_error(&e, "page"))?;
        let text_chars = crate::deskew::visible_chars(&page);
        let annotations = page.annotations().len();
        let longest = page.width().value.max(page.height().value).max(1.0);
        let config = PdfRenderConfig::new()
            .scale_page_by_factor(ANALYSIS_PIXELS / longest)
            .render_form_data(true)
            .render_annotations(true);
        let bitmap = page.render_with_config(&config).map_err(|e| map_pdfium_error(&e, "render"))?;
        let gray = bitmap.as_image().map_err(|e| map_pdfium_error(&e, "render"))?.to_luma8();
        let ink = ink_share(&gray);
        let blank = text_chars == 0 && annotations == 0 && ink <= max_ink;
        out.push(PageInk { page: i, ink, text_chars, annotations, blank });
        progress(u64::from(i) + 1, u64::from(n));
    }
    Ok(out)
}

/// Share of pixels that differ clearly from the most common tone.
pub(crate) fn ink_share(gray: &image::GrayImage) -> f32 {
    let mut hist = [0u64; 256];
    for p in gray.pixels() {
        hist[p.0[0] as usize] += 1;
    }
    let bg = hist.iter().enumerate().max_by_key(|(_, &c)| c).map(|(v, _)| v as u8).unwrap_or(255);
    let total = (gray.width() as u64 * gray.height() as u64).max(1);
    let ink: u64 = hist.iter().enumerate().filter(|(v, _)| (*v as u8).abs_diff(bg) > INK_DELTA).map(|(_, &c)| c).sum();
    ink as f32 / total as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ink_is_measured_against_the_page_background() {
        let mut g = image::GrayImage::from_pixel(100, 100, image::Luma([240]));
        assert_eq!(ink_share(&g), 0.0, "an off-white page is not ink");
        for x in 0..10 {
            g.put_pixel(x, 0, image::Luma([20]));
        }
        assert!((ink_share(&g) - 0.001).abs() < 1e-6);
        for x in 0..100 {
            g.put_pixel(x, 50, image::Luma([215]));
        }
        assert!((ink_share(&g) - 0.001).abs() < 1e-6, "faint tone changes below the delta are not ink");
    }
}
