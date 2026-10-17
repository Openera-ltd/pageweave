//! Searchable copies of scanned pages (decision D-009): each selected page is rendered, its text is
//! recognised with `pw-ocr`, and the text is added as an invisible layer (render mode 3) placed over
//! the words it came from, so it can be selected, searched and copied while the page looks exactly
//! as before. Written to a new file and verified before commit.
//!
//! Locking: PDFium is held only to render one page at a time and to write the result; recognition
//! (about 1–2 s per page) runs without the engine lock so page rendering in the app stays responsive.

use crate::document::{load, OpenOptions};
use crate::edit::{display_to_page, protection_warnings, rotation_of, save_bytes, source_encrypted};
use crate::{engine, map_pdfium_error};
use pdfium_render::prelude::*;
use pw_core::output::{write_atomic_bytes, CollisionPolicy, CommittedOutput};
use pw_core::range::parse_page_ranges;
use pw_core::{ErrorCategory, PwError, PwResult};
use serde::Serialize;
use std::path::Path;

/// Pages with at least this many non-space characters are treated as already having text.
const TEXT_PAGE_MIN_CHARS: usize = 20;
/// Recognition never needs more than ~2000 px on the long side (the engine caps there anyway).
const MAX_RENDER_LONG_SIDE: f32 = 2000.0;
/// Lines below this confidence are counted and reported.
const LOW_CONFIDENCE: f32 = 0.8;

#[derive(Debug, Clone, Default, Serialize)]
pub struct OcrSummary {
    pub pages_processed: u32,
    pub pages_skipped_with_text: u32,
    pub pages_without_text_found: u32,
    pub lines: u32,
    pub characters: u64,
    pub low_confidence_lines: u32,
    pub mean_confidence: f32,
    /// Characters recognised but not representable in the text layer's font (reported, not hidden).
    pub characters_not_added: u64,
}

/// A recognised line placed in page space.
struct Placed {
    text: String,
    /// Baseline start in page user space.
    x: f32,
    y: f32,
    /// Text height and length along the line, in points.
    height: f32,
    width: f32,
    /// Counter-clockwise angle of the line in page user space, degrees.
    angle: f32,
}

/// Characters the built-in Helvetica (WinAnsi) can carry: Latin-1 plus the WinAnsi extras.
fn winansi(c: char) -> bool {
    matches!(c, ' '..='~' | '\u{a0}'..='\u{ff}') || "€‚ƒ„…†‡ˆ‰Š‹ŒŽ‘’“”•–—˜™š›œžŸ".contains(c)
}

#[allow(clippy::too_many_arguments)]
pub fn ocr_pages(
    input: &Path,
    opts: &OpenOptions,
    pages: Option<&str>,
    skip_text_pages: bool,
    dpi: u32,
    output: &Path,
    policy: CollisionPolicy,
    cancelled: &dyn Fn() -> bool,
    progress: &dyn Fn(u64, u64),
) -> PwResult<(CommittedOutput, u32, OcrSummary, Vec<String>)> {
    let resources = pw_ocr::default_resource_dir().ok_or_else(|| {
        PwError::new(
            ErrorCategory::MissingResource,
            "ocr.resource_missing",
            "Text recognition is not installed. Reinstall PageWeave or run the resource setup.",
        )
    })?;
    let total = crate::document::page_count(input, opts)?;
    let indices = match pages {
        Some(spec) => parse_page_ranges(spec, total)?,
        None => (0..total).collect(),
    };
    let mut summary = OcrSummary::default();
    let mut placed: Vec<(u32, Vec<Placed>)> = Vec::new();
    let mut conf_sum = 0f64;
    for (k, &i) in indices.iter().enumerate() {
        if cancelled() {
            return Err(PwError::cancelled());
        }
        progress(k as u64, indices.len() as u64);
        // Phase 1 (engine lock): render the page, or skip it if it already has text.
        let rendered = {
            let pdfium = engine()?;
            let doc = load(&pdfium, input, opts)?;
            let page = doc.pages().get(i as PdfPageIndex).map_err(|e| map_pdfium_error(&e, "page"))?;
            let everything = PdfRect::new_from_values(-1.0e6, -1.0e6, 1.0e6, 1.0e6);
            let existing = page
                .text()
                .map(|t| t.inside_rect(everything).chars().filter(|c| !c.is_whitespace()).count())
                .unwrap_or(0);
            if skip_text_pages && existing >= TEXT_PAGE_MIN_CHARS {
                None
            } else {
                let (dw, dh) = (page.width().value, page.height().value);
                let scale = (dpi as f32 / 72.0).min(MAX_RENDER_LONG_SIDE / dw.max(dh).max(1.0));
                let cfg = PdfRenderConfig::new()
                    .scale_page_by_factor(scale)
                    .render_form_data(false)
                    .render_annotations(false);
                let bmp = page.render_with_config(&cfg).map_err(|e| map_pdfium_error(&e, "render"))?;
                let img = bmp.as_image().map_err(|e| map_pdfium_error(&e, "render"))?.to_rgb8();
                let origin = page
                    .boundaries()
                    .bounding()
                    .map(|b| (b.bounds.left().value, b.bounds.bottom().value))
                    .unwrap_or((0.0, 0.0));
                Some((img, rotation_of(&page), dw, dh, origin))
            }
        };
        let Some((img, rot, dw, dh, origin)) = rendered else {
            summary.pages_skipped_with_text += 1;
            continue;
        };
        // Phase 2 (no PDFium lock): recognise.
        let (iw, ih) = (img.width() as usize, img.height() as usize);
        let lines = pw_ocr::with_engine(&resources, |e| e.recognize(iw, ih, img.as_raw(), 3, cancelled))?;
        summary.pages_processed += 1;
        if lines.is_empty() {
            summary.pages_without_text_found += 1;
            continue;
        }
        // Pixel → displayed points (y up) → page user space.
        let sx = iw as f32 / dw;
        let sy = ih as f32 / dh;
        let (uw, uh) = if matches!(rot, 90 | 270) { (dh, dw) } else { (dw, dh) };
        let to_page = |px: f32, py: f32| {
            let (x, y) = display_to_page(rot, uw, uh, px / sx, dh - py / sy);
            (x + origin.0, y + origin.1)
        };
        let mut out = Vec::with_capacity(lines.len());
        for l in &lines {
            let text: String = l.text.chars().filter(|&c| winansi(c)).collect();
            summary.characters_not_added += (l.text.chars().count() - text.chars().count()) as u64;
            let text = text.trim().to_string();
            if text.is_empty() {
                continue;
            }
            summary.lines += 1;
            summary.characters += text.chars().count() as u64;
            conf_sum += l.score as f64;
            if l.score < LOW_CONFIDENCE {
                summary.low_confidence_lines += 1;
            }
            let [tl, tr, _br, bl] = l.quad;
            let (ux, uy) = (tr[0] - tl[0], tr[1] - tl[1]);
            let (vx, vy) = (bl[0] - tl[0], bl[1] - tl[1]);
            let width_px = (ux * ux + uy * uy).sqrt();
            let height_px = (vx * vx + vy * vy).sqrt();
            // The detector's box is padded around the ink; the glyphs span about 60% of its
            // height, with the baseline about a third of the way up from the box's bottom edge.
            let base = (bl[0] - vx * 0.33, bl[1] - vy * 0.33);
            let base_end = (base.0 + ux, base.1 + uy);
            let (x0, y0) = to_page(base.0, base.1);
            let (x1, y1) = to_page(base_end.0, base_end.1);
            let height = height_px / sy * 0.6;
            let width = width_px / sx;
            out.push(Placed { text, x: x0, y: y0, height, width, angle: (y1 - y0).atan2(x1 - x0).to_degrees() });
        }
        placed.push((i, out));
    }
    progress(indices.len() as u64, indices.len() as u64);
    if summary.lines > 0 {
        summary.mean_confidence = (conf_sum / summary.lines as f64) as f32;
    }
    if cancelled() {
        return Err(PwError::cancelled());
    }

    // Phase 3 (engine lock): add the text layer, save, verify.
    let pdfium = engine()?;
    let mut doc = load(&pdfium, input, opts)?;
    let font = doc.fonts_mut().helvetica();
    let err = |e: PdfiumError| map_pdfium_error(&e, "ocr");
    let mut before: Vec<(u32, Vec<u8>)> = Vec::new();
    for (i, lines) in &placed {
        let mut page = doc.pages().get(*i as PdfPageIndex).map_err(err)?;
        before.push((*i, thumbnail(&page)?));
        for p in lines {
            let size = p.height.max(1.0) / 0.93;
            // A trailing space keeps lines apart in extraction: PDFium does not break lines of
            // 90°-turned text, so line-end and line-start words would otherwise fuse.
            let mut obj =
                PdfPageTextObject::new(&doc, format!("{} ", p.text), font, PdfPoints::new(size)).map_err(err)?;
            obj.set_render_mode(PdfPageTextRenderMode::Invisible).map_err(err)?;
            let natural = obj.width().map_err(err)?.value.max(0.01);
            let space =
                PdfPageTextObject::new(&doc, " ", font, PdfPoints::new(size)).map_err(err)?.width().map_err(err)?.value;
            obj.scale(p.width / (natural - space).max(0.01), 1.0).map_err(err)?;
            obj.rotate_counter_clockwise_degrees(p.angle).map_err(err)?;
            obj.translate(PdfPoints::new(p.x), PdfPoints::new(p.y)).map_err(err)?;
            page.objects_mut().add_text_object(obj).map_err(err)?;
        }
    }
    let bytes = save_bytes(&doc)?;
    let src_encrypted = source_encrypted(&doc);
    let protected = [input.to_path_buf()];
    let mut encrypted = false;
    let c = write_atomic_bytes(output, policy, &protected, &bytes, |staged| {
        let fail = |what: &str| {
            PwError::new(
                ErrorCategory::ValidationFailed,
                "output.verify",
                format!("The searchable copy failed verification ({what}) and was not saved."),
            )
        };
        let reopened = match pdfium.load_pdf_from_file(staged, None) {
            Ok(d) => d,
            Err(_) => {
                encrypted = true;
                pdfium.load_pdf_from_file(staged, opts.password).map_err(|_| fail("could not be reopened"))?
            }
        };
        if reopened.pages().len().max(0) as u32 != total {
            return Err(fail("page count changed"));
        }
        let squash = |t: &str| t.chars().filter(|c| !c.is_whitespace()).collect::<String>();
        // Not `PdfPageText::all()`: it is bounded by the rotated page size and drops text on 90°/270°
        // pages (the checkpoint-19 extraction bug).
        let everything = PdfRect::new_from_values(-1.0e6, -1.0e6, 1.0e6, 1.0e6);
        for ((i, lines), (_, thumb)) in placed.iter().zip(&before) {
            let page = reopened.pages().get(*i as PdfPageIndex).map_err(|_| fail("page unreadable"))?;
            let got = squash(&page.text().map_err(|_| fail("text unreadable"))?.inside_rect(everything));
            if lines.iter().any(|l| !got.contains(&squash(&l.text))) {
                return Err(fail(&format!("recognised text missing on page {}", i + 1)));
            }
            if &thumbnail(&page)? != thumb {
                return Err(fail(&format!("page {} looks different", i + 1)));
            }
        }
        Ok(())
    })?;
    let mut warnings = protection_warnings(src_encrypted, encrypted);
    if summary.pages_skipped_with_text > 0 {
        warnings.push(format!(
            "{} page{} already had text and {} left unchanged.",
            summary.pages_skipped_with_text,
            if summary.pages_skipped_with_text == 1 { "" } else { "s" },
            if summary.pages_skipped_with_text == 1 { "was" } else { "were" }
        ));
    }
    if summary.characters_not_added > 0 {
        warnings.push(format!(
            "{} recognised character{} outside the Latin character set could not be added to the text layer.",
            summary.characters_not_added,
            if summary.characters_not_added == 1 { "" } else { "s" }
        ));
    }
    if summary.low_confidence_lines > 0 {
        warnings.push(format!(
            "{} line{} recognised with low confidence; check important details against the page.",
            summary.low_confidence_lines,
            if summary.low_confidence_lines == 1 { " was" } else { "s were" }
        ));
    }
    Ok((c, total, summary, warnings))
}

/// Small rendering used to prove the visible page did not change.
fn thumbnail(page: &PdfPage) -> PwResult<Vec<u8>> {
    let cfg = PdfRenderConfig::new().scale_page_by_factor(0.5).render_form_data(true).render_annotations(true);
    let bmp = page.render_with_config(&cfg).map_err(|e| map_pdfium_error(&e, "render"))?;
    Ok(bmp.as_rgba_bytes())
}

#[cfg(test)]
mod tests {
    use super::winansi;

    #[test]
    fn winansi_covers_latin_and_typographic_marks_only() {
        assert!("Café straße “quoted” – 5 € • ok".chars().all(winansi));
        assert!(!winansi('β') && !winansi('中') && !winansi('\u{3000}'));
    }
}
