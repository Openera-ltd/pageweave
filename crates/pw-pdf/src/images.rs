//! Images to PDF (PRD 06): one JPEG or PNG per page. JPEG data is embedded unchanged (PDFium's inline
//! JPEG loading keeps the DCT stream), so photos keep their quality and size; PNGs are decoded and
//! embedded. The EXIF orientation of photos is honoured by turning the page (lossless). Headers are
//! checked by small parsers here before anything reaches PDFium, and damaged files are refused.

use crate::edit::save_bytes;
use crate::{engine, map_pdfium_error};
use pdfium_render::prelude::*;
use pw_core::ops::ImagePageSize;
use pw_core::output::{write_atomic_bytes, CollisionPolicy, CommittedOutput};
use pw_core::{ErrorCategory, PwError, PwResult};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

/// Largest image file accepted, and largest decoded PNG (pixels).
const MAX_IMAGE_BYTES: u64 = 200 * 1024 * 1024;
const MAX_PNG_PIXELS: u64 = 100_000_000;
const A4: (f32, f32) = (595.276, 841.89);
const LETTER: (f32, f32) = (612.0, 792.0);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Jpeg,
    Png,
    /// WebP, TIFF, GIF or BMP: decoded and embedded like a PNG (first page or frame).
    Other(image::ImageFormat),
}

/// What the headers say about an image.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ImageHeader {
    pub kind: Kind,
    pub width: u32,
    pub height: u32,
    /// Pixels per inch, when recorded.
    pub dpi: Option<f32>,
    /// Clockwise turn needed to show the image upright (EXIF orientation), and whether EXIF also
    /// asks for a mirror image (not applied).
    pub turn: u32,
    pub mirrored: bool,
}

fn damaged(name: &str) -> PwError {
    PwError::new(ErrorCategory::Corrupt, "images.damaged", format!("{name} is damaged or not a complete image."))
}

fn be16(d: &[u8], i: usize) -> Option<u16> {
    Some(u16::from_be_bytes([*d.get(i)?, *d.get(i + 1)?]))
}

/// EXIF orientation (tag 0x0112) from an APP1 payload starting at "Exif\0\0".
fn exif_orientation(app1: &[u8]) -> Option<u16> {
    let t = app1.strip_prefix(b"Exif\0\0")?;
    let le = match t.get(..2)? {
        b"II" => true,
        b"MM" => false,
        _ => return None,
    };
    let r16 = |i: usize| -> Option<u16> {
        let b = [*t.get(i)?, *t.get(i + 1)?];
        Some(if le { u16::from_le_bytes(b) } else { u16::from_be_bytes(b) })
    };
    let r32 = |i: usize| -> Option<u32> {
        let b = [*t.get(i)?, *t.get(i + 1)?, *t.get(i + 2)?, *t.get(i + 3)?];
        Some(if le { u32::from_le_bytes(b) } else { u32::from_be_bytes(b) })
    };
    let ifd = r32(4)? as usize;
    let n = r16(ifd)? as usize;
    (0..n.min(512)).find_map(|k| {
        let e = ifd + 2 + k * 12;
        (r16(e)? == 0x0112).then(|| r16(e + 8)).flatten()
    })
}

fn orientation_turn(o: u16) -> (u32, bool) {
    match o {
        3 => (180, false),
        6 => (90, false),
        8 => (270, false),
        2 => (0, true),
        4 => (180, true),
        5 => (90, true),
        7 => (270, true),
        _ => (0, false),
    }
}

pub(crate) fn jpeg_header(d: &[u8]) -> Option<ImageHeader> {
    if d.get(..3)? != [0xFF, 0xD8, 0xFF] || d.get(d.len().checked_sub(2)?..)? != [0xFF, 0xD9] {
        return None;
    }
    let (mut i, mut dpi, mut orient, mut size) = (2usize, None, None, None);
    while i + 4 <= d.len() {
        if d[i] != 0xFF {
            return None;
        }
        let marker = d[i + 1];
        if marker == 0xFF {
            i += 1;
            continue;
        }
        let len = be16(d, i + 2)? as usize;
        let seg = d.get(i + 4..i + 2 + len)?;
        match marker {
            0xE0 if seg.starts_with(b"JFIF\0") && seg.len() >= 12 => {
                let (units, x) = (seg[7], u16::from_be_bytes([seg[8], seg[9]]) as f32);
                dpi = match units {
                    1 if x > 0.0 => Some(x),
                    2 if x > 0.0 => Some(x * 2.54),
                    _ => dpi,
                };
            }
            0xE1 => orient = orient.or_else(|| exif_orientation(seg)),
            0xC0..=0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF if seg.len() >= 6 => {
                let h = u16::from_be_bytes([seg[1], seg[2]]) as u32;
                let w = u16::from_be_bytes([seg[3], seg[4]]) as u32;
                size = Some((w, h));
            }
            0xDA => break,
            _ => {}
        }
        i += 2 + len;
    }
    let (width, height) = size.filter(|&(w, h)| w > 0 && h > 0)?;
    let (turn, mirrored) = orientation_turn(orient.unwrap_or(1));
    Some(ImageHeader { kind: Kind::Jpeg, width, height, dpi, turn, mirrored })
}

/// Pages (images) in a TIFF file, counted up to 10 000; 1 when it cannot be read that far.
fn tiff_pages(d: &[u8]) -> usize {
    let Ok(mut dec) = tiff::decoder::Decoder::new(std::io::Cursor::new(d)) else { return 1 };
    let mut n = 1;
    while n < 10_000 && dec.more_images() && dec.next_image().is_ok() {
        n += 1;
    }
    n
}

/// WebP, TIFF, GIF or BMP, recognised by content (not by name).
fn other_format(d: &[u8]) -> Option<image::ImageFormat> {
    use image::ImageFormat as F;
    image::guess_format(d).ok().filter(|f| matches!(f, F::WebP | F::Tiff | F::Gif | F::Bmp))
}

pub(crate) fn png_header(d: &[u8]) -> Option<ImageHeader> {
    if d.get(..8)? != [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A] {
        return None;
    }
    let (mut i, mut dpi, mut size) = (8usize, None, None);
    while i + 8 <= d.len() {
        let len = u32::from_be_bytes(d.get(i..i + 4)?.try_into().ok()?) as usize;
        let kind = d.get(i + 4..i + 8)?;
        let body = d.get(i + 8..i + 8 + len)?;
        match kind {
            b"IHDR" if len >= 8 => {
                size = Some((
                    u32::from_be_bytes(body[0..4].try_into().ok()?),
                    u32::from_be_bytes(body[4..8].try_into().ok()?),
                ));
            }
            b"pHYs" if len >= 9 && body[8] == 1 => {
                let ppm = u32::from_be_bytes(body[0..4].try_into().ok()?) as f32;
                if ppm > 0.0 {
                    dpi = Some(ppm * 0.0254);
                }
            }
            b"IEND" => {
                let (width, height) = size.filter(|&(w, h)| w > 0 && h > 0)?;
                return Some(ImageHeader { kind: Kind::Png, width, height, dpi, turn: 0, mirrored: false });
            }
            _ => {}
        }
        i += 12 + len;
    }
    None
}

/// Page size (as displayed) and the image's displayed size and position, in points.
fn layout(h: &ImageHeader, size: ImagePageSize, margin: f32) -> ((f32, f32), (f32, f32, f32, f32)) {
    let (dw, dh) = if matches!(h.turn, 90 | 270) { (h.height, h.width) } else { (h.width, h.height) };
    let dpi = h.dpi.filter(|d| (10.0..=10_000.0).contains(d)).unwrap_or(96.0);
    let natural = (dw as f32 / dpi * 72.0, dh as f32 / dpi * 72.0);
    let page = match size {
        ImagePageSize::Image => (natural.0 + 2.0 * margin, natural.1 + 2.0 * margin),
        ImagePageSize::A4 | ImagePageSize::Letter => {
            let (a, b) = if size == ImagePageSize::A4 { A4 } else { LETTER };
            if dw > dh {
                (b, a)
            } else {
                (a, b)
            }
        }
    };
    let (aw, ah) = ((page.0 - 2.0 * margin).max(1.0), (page.1 - 2.0 * margin).max(1.0));
    let k = if size == ImagePageSize::Image { 1.0 } else { (aw / natural.0).min(ah / natural.1) };
    let (iw, ih) = (natural.0 * k, natural.1 * k);
    (page, ((page.0 - iw) / 2.0, (page.1 - ih) / 2.0, iw, ih))
}

/// What each output page must be: displayed size, turn, and the JPEG data digest (JPEG only).
type PlannedPage = ((f32, f32), u32, Option<[u8; 32]>);

fn display_name(p: &Path) -> String {
    crate::attachments::clean_name(&p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default())
}

#[allow(clippy::too_many_arguments)]
pub fn images_to_pdf(
    inputs: &[PathBuf],
    size: ImagePageSize,
    margin: f32,
    output: &Path,
    policy: CollisionPolicy,
    cancelled: &dyn Fn() -> bool,
    progress: &dyn Fn(u64, u64),
) -> PwResult<(CommittedOutput, u32, Vec<String>)> {
    let pdfium = engine()?;
    let mut doc = pdfium.create_new_pdf().map_err(|e| map_pdfium_error(&e, "create"))?;
    let mut expected: Vec<PlannedPage> = Vec::with_capacity(inputs.len());
    let mut mirrored = Vec::new();
    let mut more_pages = Vec::new();
    for (n, path) in inputs.iter().enumerate() {
        if cancelled() {
            return Err(PwError::cancelled());
        }
        let name = display_name(path);
        let meta = std::fs::metadata(path).map_err(|e| PwError::from_io(&e, "image"))?;
        if meta.len() > MAX_IMAGE_BYTES {
            return Err(PwError::invalid("images.too_large", format!("{name} is larger than 200 MB.")));
        }
        let bytes = std::fs::read(path).map_err(|e| PwError::from_io(&e, "image"))?;
        let header = if bytes.starts_with(&[0xFF, 0xD8]) {
            jpeg_header(&bytes).ok_or_else(|| damaged(&name))?
        } else if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
            png_header(&bytes).ok_or_else(|| damaged(&name))?
        } else if let Some(fmt) = other_format(&bytes) {
            let (width, height) = image::ImageReader::with_format(std::io::Cursor::new(&bytes), fmt)
                .into_dimensions()
                .map_err(|_| damaged(&name))?;
            if width == 0 || height == 0 {
                return Err(damaged(&name));
            }
            if fmt == image::ImageFormat::Tiff {
                let pages = tiff_pages(&bytes);
                if pages > 1 {
                    more_pages.push(format!("{name} ({pages} pages)"));
                }
            }
            ImageHeader { kind: Kind::Other(fmt), width, height, dpi: None, turn: 0, mirrored: false }
        } else {
            return Err(PwError::new(
                ErrorCategory::UnsupportedFormat,
                "images.format",
                format!("{name} is not a JPEG, PNG, WebP, TIFF, GIF or BMP image."),
            ));
        };
        if header.mirrored {
            mirrored.push(n + 1);
        }
        let (page_size, (x, y, iw, ih)) = layout(&header, size, margin);
        // The page is built unturned and then given /Rotate = the EXIF turn: lossless, and the image
        // data stays exactly as stored.
        let (pw, ph) = if matches!(header.turn, 90 | 270) { (page_size.1, page_size.0) } else { page_size };
        let (ux, uy, uw, uh) = match header.turn {
            90 | 270 => (y, x, ih, iw),
            _ => (x, y, iw, ih),
        };
        let mut obj = match header.kind {
            Kind::Jpeg => PdfPageImageObject::new_from_jpeg_reader(&doc, std::io::Cursor::new(bytes.clone()))
                .map_err(|_| damaged(&name))?,
            Kind::Png | Kind::Other(_) => {
                if header.width as u64 * header.height as u64 > MAX_PNG_PIXELS {
                    return Err(PwError::invalid("images.too_large", format!("{name} has too many pixels.")));
                }
                let fmt = match header.kind {
                    Kind::Other(f) => f,
                    _ => image::ImageFormat::Png,
                };
                let mut reader = image::ImageReader::with_format(std::io::Cursor::new(&bytes), fmt);
                let mut limits = image::Limits::default();
                limits.max_alloc = Some(2 * 1024 * 1024 * 1024);
                reader.limits(limits);
                let img = reader.decode().map_err(|_| damaged(&name))?;
                PdfPageImageObject::new(&doc, &img).map_err(|e| map_pdfium_error(&e, "image"))?
            }
        };
        obj.apply_matrix(PdfMatrix::new(uw, 0.0, 0.0, uh, ux, uy)).map_err(|e| map_pdfium_error(&e, "image"))?;
        let mut page = doc
            .pages_mut()
            .create_page_at_end(PdfPagePaperSize::Custom(PdfPoints::new(pw), PdfPoints::new(ph)))
            .map_err(|e| map_pdfium_error(&e, "page"))?;
        page.objects_mut().add_image_object(obj).map_err(|e| map_pdfium_error(&e, "image"))?;
        if header.turn != 0 {
            page.set_rotation(match header.turn {
                90 => PdfPageRenderRotation::Degrees90,
                180 => PdfPageRenderRotation::Degrees180,
                _ => PdfPageRenderRotation::Degrees270,
            });
        }
        let jpeg_digest = (header.kind == Kind::Jpeg).then(|| Sha256::digest(&bytes).into());
        expected.push((page_size, header.turn, jpeg_digest));
        progress(n as u64 + 1, inputs.len() as u64);
    }
    let out = save_bytes(&doc)?;
    let count = expected.len() as u32;
    let c = write_atomic_bytes(output, policy, inputs, &out, |staged| {
        let fail = |what: String| {
            PwError::new(
                ErrorCategory::ValidationFailed,
                "output.verify",
                format!("The new PDF failed verification ({what})."),
            )
        };
        let back = pdfium.load_pdf_from_file(staged, None).map_err(|_| fail("it could not be reopened".into()))?;
        if back.pages().len().max(0) as u32 != count {
            return Err(fail("page count".into()));
        }
        for (i, (size, turn, digest)) in expected.iter().enumerate() {
            let page = back.pages().get(i as PdfPageIndex).map_err(|_| fail(format!("page {}", i + 1)))?;
            // PDFium reports the displayed size.
            if (page.width().value - size.0).abs() > 0.5 || (page.height().value - size.1).abs() > 0.5 {
                return Err(fail(format!("page {} size", i + 1)));
            }
            let rot = crate::edit::rotation_of(&page);
            if rot != *turn {
                return Err(fail(format!("page {} orientation", i + 1)));
            }
            let objs: Vec<_> = page.objects().iter().collect();
            let img = match objs.as_slice() {
                [o] => o.as_image_object().ok_or_else(|| fail(format!("page {} image", i + 1)))?,
                _ => return Err(fail(format!("page {} holds {} objects", i + 1, objs.len()))),
            };
            if let Some(d) = digest {
                let raw = img.get_raw_image_data().map_err(|_| fail(format!("page {} image data", i + 1)))?;
                let got: [u8; 32] = Sha256::digest(&raw).into();
                if &got != d {
                    return Err(fail(format!("page {}: the JPEG data was changed", i + 1)));
                }
            }
        }
        Ok(())
    })?;
    let mut warnings = Vec::new();
    if !mirrored.is_empty() {
        let list = mirrored.iter().map(usize::to_string).collect::<Vec<_>>().join(", ");
        warnings.push(format!("Image {list} asks to be shown mirrored; it was turned upright but not mirrored."));
    }
    if !more_pages.is_empty() {
        warnings.push(format!(
            "Only the first page of {} was used; save the other pages as separate images to add them.",
            more_pages.join(", ")
        ));
    }
    Ok((c, count, warnings))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layouts_fit_and_follow_orientation() {
        let photo =
            ImageHeader { kind: Kind::Jpeg, width: 4000, height: 3000, dpi: Some(72.0), turn: 0, mirrored: false };
        let (page, (x, y, w, h)) = layout(&photo, ImagePageSize::A4, 0.0);
        assert_eq!(page, (A4.1, A4.0), "landscape image → landscape page");
        // 4:3 is narrower than A4 landscape (≈1.41:1): fitted by height, centred horizontally.
        assert!((h - A4.0).abs() < 0.01 && w < A4.1 && y.abs() < 0.01 && x > 0.0, "{w}x{h} at {x},{y}");
        let turned = ImageHeader { turn: 90, ..photo };
        assert_eq!(layout(&turned, ImagePageSize::Letter, 36.0).0, LETTER, "displayed portrait");
        let (page, (x, y, w, h)) =
            layout(&ImageHeader { dpi: Some(300.0), width: 600, height: 300, ..photo }, ImagePageSize::Image, 10.0);
        assert_eq!(page, (164.0, 92.0));
        assert_eq!((x, y, w, h), (10.0, 10.0, 144.0, 72.0));
        // No or absurd density: 96 dpi.
        assert_eq!(
            layout(&ImageHeader { dpi: Some(1.0), width: 96, height: 96, ..photo }, ImagePageSize::Image, 0.0).0,
            (72.0, 72.0)
        );
    }

    #[test]
    fn orientations_map_to_turns() {
        assert_eq!(orientation_turn(6), (90, false));
        assert_eq!(orientation_turn(8), (270, false));
        assert_eq!(orientation_turn(5), (90, true));
        assert_eq!(orientation_turn(1), (0, false));
        assert_eq!(orientation_turn(42), (0, false));
    }
}
