//! Visual signatures and initials (PRD 01 E "typed/drawn/image appearance signatures"): a typed
//! name, a drawn signature (vector strokes) or a picture is placed into a box on a page, upright as
//! the page is displayed. This is a picture of a signature, not a digital signature (PRD 05: label
//! them honestly); the UI and the matrix say so. The output is a new file, verified before commit.

use crate::document::{load, OpenOptions};
use crate::edit::{protection_warnings, save_bytes, source_encrypted, verify_output, EditOutcome, Expect};
use crate::{engine, map_pdfium_error};
use pdfium_render::prelude::*;
use pw_core::ops::{RectPt, SignatureMark, TypedStyle};
use pw_core::output::{write_atomic_bytes, CollisionPolicy};
use pw_core::{ErrorCategory, PwError, PwResult};
use std::path::Path;

/// Pictures larger than this (long side, pixels) are reduced before placing.
const MAX_IMAGE_SIDE: u32 = 2000;
/// Largest picture file accepted.
const MAX_IMAGE_BYTES: u64 = 20 * 1024 * 1024;

/// Maps a point in the box as displayed (u from the left, v from the bottom, points) to the page's
/// unrotated user space, for a page displayed turned clockwise by `rot`.
#[derive(Debug, Clone, Copy)]
pub(crate) struct BoxMap {
    l: f32,
    b: f32,
    r: f32,
    t: f32,
    rot: u32,
}

impl BoxMap {
    pub(crate) fn new(rect: &RectPt, rot: u32) -> Self {
        BoxMap {
            l: rect.left.min(rect.right),
            b: rect.bottom.min(rect.top),
            r: rect.left.max(rect.right),
            t: rect.bottom.max(rect.top),
            rot,
        }
    }
    /// Box size as displayed (width, height).
    pub(crate) fn size(&self) -> (f32, f32) {
        let (w, h) = (self.r - self.l, self.t - self.b);
        if self.rot % 180 == 90 {
            (h, w)
        } else {
            (w, h)
        }
    }
    pub(crate) fn map(&self, u: f32, v: f32) -> (f32, f32) {
        match self.rot {
            90 => (self.r - v, self.b + u),
            180 => (self.r - u, self.t - v),
            270 => (self.l + v, self.t - u),
            _ => (self.l + u, self.b + v),
        }
    }
}

fn err(e: PdfiumError) -> PwError {
    map_pdfium_error(&e, "signature mark")
}

fn invalid(code: &'static str, msg: &str) -> PwError {
    PwError::invalid(code, msg)
}

fn color(hex: &str) -> PwResult<PdfColor> {
    let v = u32::from_str_radix(hex.trim_start_matches('#'), 16)
        .map_err(|_| invalid("mark.color", "Colors must look like #rrggbb."))?;
    Ok(PdfColor::new((v >> 16) as u8, (v >> 8) as u8, v as u8, 255))
}

fn load_picture(path: &Path) -> PwResult<image::DynamicImage> {
    let meta = std::fs::metadata(path).map_err(|e| PwError::from_io(&e, "picture"))?;
    if meta.len() > MAX_IMAGE_BYTES {
        return Err(invalid("mark.image_large", "The picture is larger than 20 MB."));
    }
    let bytes = std::fs::read(path).map_err(|e| PwError::from_io(&e, "picture"))?;
    let fmt = image::guess_format(&bytes)
        .map_err(|_| invalid("mark.image_format", "Use a picture: PNG, JPEG, WebP, TIFF, GIF or BMP."))?;
    if !matches!(
        fmt,
        image::ImageFormat::Png
            | image::ImageFormat::Jpeg
            | image::ImageFormat::WebP
            | image::ImageFormat::Tiff
            | image::ImageFormat::Gif
            | image::ImageFormat::Bmp
    ) {
        return Err(invalid("mark.image_format", "Use a picture: PNG, JPEG, WebP, TIFF, GIF or BMP."));
    }
    let mut reader = image::ImageReader::with_format(std::io::Cursor::new(&bytes), fmt);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(20_000);
    limits.max_image_height = Some(20_000);
    limits.max_alloc = Some(512 * 1024 * 1024);
    reader.limits(limits);
    let img = reader.decode().map_err(|_| invalid("mark.image_unreadable", "The picture could not be read."))?;
    Ok(if img.width().max(img.height()) > MAX_IMAGE_SIDE {
        img.resize(MAX_IMAGE_SIDE, MAX_IMAGE_SIDE, image::imageops::FilterType::Lanczos3)
    } else {
        img
    })
}

/// Place `mark` in `rect` (unrotated page space) on page `page` (0-based) of a copy of `input`.
pub fn add_signature_mark(
    input: &Path,
    opts: &OpenOptions,
    page_index: u32,
    rect: &RectPt,
    mark: &SignatureMark,
    output: &Path,
    policy: CollisionPolicy,
) -> PwResult<EditOutcome> {
    let pdfium = engine()?;
    let mut doc = load(&pdfium, input, opts)?;
    let total = doc.pages().len().max(0) as u32;
    if page_index >= total {
        return Err(invalid("mark.page", "That page does not exist."));
    }
    let picture = match mark {
        SignatureMark::Image { path } => Some(load_picture(path)?),
        _ => None,
    };
    let signatures = doc.signatures().len();
    let font = match mark {
        SignatureMark::Typed { style, .. } => Some(match style {
            TypedStyle::Formal => doc.fonts_mut().times_italic(),
            TypedStyle::Plain => doc.fonts_mut().helvetica_oblique(),
            TypedStyle::Typewriter => doc.fonts_mut().courier_oblique(),
        }),
        _ => None,
    };
    let mut page = doc.pages().get(page_index as PdfPageIndex).map_err(err)?;
    let rot = crate::edit::rotation_of(&page);
    let bm = BoxMap::new(rect, rot);
    let (bw, bh) = bm.size();
    match mark {
        SignatureMark::Typed { text, color: c, .. } => {
            let font = font.ok_or_else(|| PwError::internal("mark.font", "Font missing."))?;
            let probe = PdfPageTextObject::new(&doc, text, font, PdfPoints::new(100.0)).map_err(err)?;
            let w100 = probe.width().map_err(err)?.value.max(1.0);
            // Fit: at most 80% of the box height (cap + descender ≈ 0.9 em) and 94% of its width.
            let size = (bh * 0.8 / 0.9).min(bw * 0.94 / (w100 / 100.0)).clamp(4.0, 200.0);
            let mut obj = PdfPageTextObject::new(&doc, text, font, PdfPoints::new(size)).map_err(err)?;
            obj.set_fill_color(color(c)?).map_err(err)?;
            let tw = obj.width().map_err(err)?.value;
            // Baseline centred in the box as displayed.
            let (u, v) = ((bw - tw) / 2.0, (bh - size * 0.7) / 2.0);
            let (x, y) = bm.map(u, v);
            obj.rotate_counter_clockwise_degrees(rot as f32).map_err(err)?;
            obj.translate(PdfPoints::new(x), PdfPoints::new(y)).map_err(err)?;
            page.objects_mut().add_text_object(obj).map_err(err)?;
        }
        SignatureMark::Drawn { strokes, color: c, width } => {
            let col = color(c)?;
            for s in strokes.iter().filter(|s| !s.is_empty()) {
                // Points are 0..1 across the box as displayed, y downwards (as drawn on screen).
                let p = |q: &[f32; 2]| bm.map(q[0] * bw, (1.0 - q[1]) * bh);
                let (x0, y0) = p(&s[0]);
                let mut path = PdfPagePathObject::new(
                    &doc,
                    PdfPoints::new(x0),
                    PdfPoints::new(y0),
                    Some(col),
                    Some(PdfPoints::new(*width)),
                    None,
                )
                .map_err(err)?;
                path.set_line_cap(PdfPageObjectLineCap::Round).map_err(err)?;
                path.set_line_join(PdfPageObjectLineJoin::Round).map_err(err)?;
                if s.len() == 1 {
                    // A dot: a zero-length segment draws a round cap.
                    path.line_to(PdfPoints::new(x0 + 0.01), PdfPoints::new(y0)).map_err(err)?;
                }
                for q in &s[1..] {
                    let (x, y) = p(q);
                    path.line_to(PdfPoints::new(x), PdfPoints::new(y)).map_err(err)?;
                }
                page.objects_mut().add_path_object(path).map_err(err)?;
            }
        }
        SignatureMark::Image { .. } => {
            let img = picture.as_ref().ok_or_else(|| PwError::internal("mark.image", "Picture missing."))?;
            let (iw, ih) = (img.width() as f32, img.height() as f32);
            // Fit inside the box, keeping proportions, centred.
            let k = (bw / iw).min(bh / ih);
            let (w, h) = (iw * k, ih * k);
            let (u0, v0) = ((bw - w) / 2.0, (bh - h) / 2.0);
            let o = bm.map(u0, v0);
            let ax = bm.map(u0 + w, v0);
            let ay = bm.map(u0, v0 + h);
            let mut obj = PdfPageImageObject::new(&doc, img).map_err(err)?;
            // A new image object has the identity matrix (the unit square), so this places it.
            obj.apply_matrix(PdfMatrix::new(ax.0 - o.0, ax.1 - o.1, ay.0 - o.0, ay.1 - o.1, o.0, o.1)).map_err(err)?;
            page.objects_mut().add_image_object(obj).map_err(err)?;
        }
    }
    let before = thumb_area(&pdfium, input, opts.password, page_index, rect)?;
    let bytes = save_bytes(&doc)?;
    let src_encrypted = source_encrypted(&doc);
    let mut encrypted = false;
    let c = write_atomic_bytes(output, policy, &[input.to_path_buf()], &bytes, |staged| {
        encrypted =
            verify_output(&pdfium, staged, &Expect { page_count: total, password: opts.password, rotations: None })?;
        let after = thumb_area(&pdfium, staged, if encrypted { opts.password } else { None }, page_index, rect)?;
        let changed =
            before.len() == after.len() && before.iter().zip(&after).filter(|(a, b)| a.abs_diff(**b) > 40).count() > 4;
        if !changed {
            return Err(PwError::new(
                ErrorCategory::ValidationFailed,
                "output.verify",
                "The signature did not appear on the page, so the copy was not saved.",
            ));
        }
        if let SignatureMark::Typed { text, .. } = mark {
            let doc = pdfium.load_pdf_from_file(staged, if encrypted { opts.password } else { None }).map_err(err)?;
            let p = doc.pages().get(page_index as PdfPageIndex).map_err(err)?;
            let all = p.text().map_err(err)?.all();
            let squash = |t: &str| t.chars().filter(|c| !c.is_whitespace()).collect::<String>();
            if !squash(&all).contains(&squash(text)) {
                return Err(PwError::new(
                    ErrorCategory::ValidationFailed,
                    "output.verify",
                    "The typed signature is missing from the copy.",
                ));
            }
        }
        Ok(())
    })?;
    let mut warnings = protection_warnings(src_encrypted, encrypted);
    if signatures > 0 {
        warnings.push(crate::properties::signature_warning(signatures));
    }
    Ok((c, total, warnings))
}

/// Grey pixels of the box area at ~72 dpi, for a before/after comparison.
fn thumb_area(
    pdfium: &Pdfium,
    path: &Path,
    password: Option<&str>,
    page_index: u32,
    rect: &RectPt,
) -> PwResult<Vec<u8>> {
    let doc = pdfium.load_pdf_from_file(path, password).map_err(err)?;
    let page = doc.pages().get(page_index as PdfPageIndex).map_err(err)?;
    let cfg = PdfRenderConfig::new().scale_page_by_factor(1.0).render_annotations(true).render_form_data(true);
    let img = page.render_with_config(&cfg).map_err(err)?.as_image().map_err(err)?.to_luma8();
    // The box in rendered pixels: map its corners through the page's display transform.
    let (w, h) = (page.width().value, page.height().value);
    let rot = crate::edit::rotation_of(&page);
    let to_px = |x: f32, y: f32| -> (f32, f32) {
        let b = page.boundaries().media().map(|b| b.bounds).ok();
        let (ox, oy) = b.map(|b| (b.left().value, b.bottom().value)).unwrap_or((0.0, 0.0));
        let (x, y) = (x - ox, y - oy);
        // Unrotated page size (w, h are as displayed).
        let (uw, uh) = if rot % 180 == 90 { (h, w) } else { (w, h) };
        match rot {
            90 => (y, x),
            180 => (uw - x, y),
            270 => (uh - y, uw - x),
            _ => (x, uh - y),
        }
    };
    let (ax, ay) = to_px(rect.left, rect.bottom);
    let (bx, by) = to_px(rect.right, rect.top);
    let (x0, x1) = (ax.min(bx).max(0.0) as u32, ax.max(bx).min(img.width() as f32) as u32);
    let (y0, y1) = (ay.min(by).max(0.0) as u32, ay.max(by).min(img.height() as f32) as u32);
    let mut out = Vec::new();
    for y in y0..y1 {
        for x in x0..x1 {
            out.push(img.get_pixel(x, y).0[0]);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn box_corners_follow_the_displayed_orientation() {
        let r = RectPt { left: 100.0, bottom: 200.0, right: 300.0, top: 260.0 };
        // Upright: bottom-left as displayed is the rectangle's bottom-left.
        assert_eq!(BoxMap::new(&r, 0).map(0.0, 0.0), (100.0, 200.0));
        assert_eq!(BoxMap::new(&r, 0).size(), (200.0, 60.0));
        // Turned 90° clockwise for display: the box shows 60 wide, 200 tall; its displayed
        // bottom-left is the rectangle's right-bottom corner.
        let m = BoxMap::new(&r, 90);
        assert_eq!(m.size(), (60.0, 200.0));
        assert_eq!(m.map(0.0, 0.0), (300.0, 200.0));
        assert_eq!(m.map(60.0, 200.0), (100.0, 260.0));
        assert_eq!(BoxMap::new(&r, 180).map(0.0, 0.0), (300.0, 260.0));
        assert_eq!(BoxMap::new(&r, 270).map(0.0, 0.0), (100.0, 260.0));
    }
}
