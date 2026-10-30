//! Design Reconstruction Studio (PRD 10, decision D-028): a flat picture (an image file or a
//! rendered PDF page) is rebuilt as separately editable objects.
//!
//! Pipeline: the background colour is the most common colour along the edges; recognised text lines
//! (`pw-ocr`) become text objects whose colour, size, weight and baseline are measured from their
//! ink; the remaining ink is split into connected regions: a region filled with one colour becomes
//! a vector rectangle, anything else stays a picture (transparent where it was background) and is
//! marked uncertain. Nothing is invented: wording comes only from recognition, with its confidence.
//!
//! A design is saved as a `.pwdesign` project (JSON, the source picture inside) and exported as
//! SVG (text, rectangles, embedded PNGs; nothing external) or PDF (real text), each read back
//! before it is committed.

use crate::document::{load, OpenOptions};
use crate::edit::save_bytes;
use crate::{engine, map_pdfium_error};
use base64::Engine as _;
use pdfium_render::prelude::*;
use pw_core::output::{write_atomic_bytes, CollisionPolicy, CommittedOutput};
use pw_core::{ErrorCategory, PwError, PwResult};
use serde::{Deserialize, Serialize};
use std::path::Path;

pub const DESIGN_VERSION: u32 = 1;
pub const DESIGN_EXTENSION: &str = "pwdesign";
/// Analysis never needs more than this on the long side; larger inputs are scaled down.
const MAX_SIDE: u32 = 2400;
/// Limits for untrusted designs (project files, agent requests).
const MAX_OBJECTS: usize = 5000;
const MAX_TEXT: usize = 10_000;
const MAX_DESIGN_BYTES: u64 = 256 * 1024 * 1024;
const MAX_IMAGE_FILE: u64 = 100 * 1024 * 1024;
/// Colour distance (largest channel difference) that still counts as "the same colour".
const SAME: i32 = 28;
/// Ink of a text line differs from what surrounds it by at least this much.
const INK: i32 = 60;
/// Regions smaller than this many pixels are specks (scan noise), not objects.
const SPECK: usize = 6;
/// Beyond this many regions the rest are kept together as one picture.
const MAX_REGIONS: usize = 600;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObjectKind {
    Text,
    Shape,
    Image,
}

/// One editable object, in design pixels (origin top left).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DesignObject {
    pub id: String,
    pub name: String,
    pub kind: ObjectKind,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    #[serde(default = "yes")]
    pub visible: bool,
    #[serde(default)]
    pub locked: bool,
    /// How sure the reconstruction is (0–1): recognition confidence for text, colour uniformity
    /// for shapes; pictures are 0 (kept as found, not interpreted).
    #[serde(default)]
    pub confidence: f32,
    /// Shown in the Studio's diagnostics, never drawn on the canvas.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Text: wording, `#rrggbb`, size in design pixels, bold, baseline below the top of the box.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_size: Option<f32>,
    #[serde(default)]
    pub bold: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub baseline: Option<f32>,
    /// Shape: fill `#rrggbb`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill: Option<String>,
    /// Picture: PNG, base64.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub png: Option<String>,
}

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Design {
    pub version: u32,
    pub width: u32,
    pub height: u32,
    /// PDF points per design pixel (page size of a PDF export).
    pub pt_per_px: f32,
    pub background: String,
    /// The picture the design was rebuilt from (PNG, base64), kept for comparison.
    pub source_png: String,
    pub source_name: String,
    /// Objects from back to front.
    pub objects: Vec<DesignObject>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ReconstructSummary {
    pub text: u32,
    pub shapes: u32,
    pub images: u32,
    /// Share of objects that are editable as text or vectors (not pictures).
    pub editable_ratio: f32,
    pub low_confidence_text: u32,
    pub mean_text_confidence: f32,
}

/* ---------------- Pixels ---------------- */

/// An RGB picture, row-major.
pub struct Rgb {
    pub w: usize,
    pub h: usize,
    pub px: Vec<u8>,
}

impl Rgb {
    fn at(&self, x: usize, y: usize) -> [i32; 3] {
        let i = (y * self.w + x) * 3;
        [self.px[i] as i32, self.px[i + 1] as i32, self.px[i + 2] as i32]
    }
}

fn dist(a: [i32; 3], b: [i32; 3]) -> i32 {
    (a[0] - b[0]).abs().max((a[1] - b[1]).abs()).max((a[2] - b[2]).abs())
}

pub fn hex(c: [i32; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0].clamp(0, 255), c[1].clamp(0, 255), c[2].clamp(0, 255))
}

fn parse_hex(s: &str) -> Option<[i32; 3]> {
    let h = s.strip_prefix('#')?;
    if h.len() != 6 || !h.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let v = u32::from_str_radix(h, 16).ok()?;
    Some([(v >> 16) as i32 & 255, (v >> 8) as i32 & 255, v as i32 & 255])
}

/// Most common colour among `pixels` (5-bit buckets), averaged within the winning bucket, and the
/// share of pixels within `SAME` of it.
fn dominant(pixels: impl Iterator<Item = [i32; 3]> + Clone) -> ([i32; 3], f32) {
    let mut counts = std::collections::HashMap::<u32, (u32, [i64; 3])>::new();
    let mut n = 0u32;
    for c in pixels.clone() {
        let key = ((c[0] >> 3) << 10 | (c[1] >> 3) << 5 | (c[2] >> 3)) as u32;
        let e = counts.entry(key).or_insert((0, [0; 3]));
        e.0 += 1;
        for (sum, v) in e.1.iter_mut().zip(c) {
            *sum += v as i64;
        }
        n += 1;
    }
    let Some((_, (cnt, sum))) = counts.into_iter().max_by_key(|(k, (c, _))| (*c, *k)) else {
        return ([255, 255, 255], 0.0);
    };
    let mean = [0, 1, 2].map(|k| (sum[k] / cnt as i64) as i32);
    let near = pixels.filter(|&c| dist(c, mean) <= SAME).count() as f32;
    (mean, near / n.max(1) as f32)
}

/// A recognised line, axis-aligned, in picture pixels.
#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    pub text: String,
    pub score: f32,
    pub x0: usize,
    pub y0: usize,
    pub x1: usize,
    pub y1: usize,
}

impl Line {
    pub fn from_ocr(l: &pw_ocr::OcrLine, w: usize, h: usize) -> Line {
        let xs = l.quad.map(|p| p[0]);
        let ys = l.quad.map(|p| p[1]);
        let lo =
            |v: [f32; 4], max: usize| (v.iter().cloned().fold(f32::MAX, f32::min).floor().max(0.0) as usize).min(max);
        let hi =
            |v: [f32; 4], max: usize| (v.iter().cloned().fold(f32::MIN, f32::max).ceil().max(0.0) as usize).min(max);
        Line {
            text: l.text.trim().to_string(),
            score: l.score,
            x0: lo(xs, w),
            y0: lo(ys, h),
            x1: hi(xs, w),
            y1: hi(ys, h),
        }
    }
}

/// Rebuild `img` as a design from its recognised lines (no I/O; the engine's core, unit-tested).
pub fn reconstruct(img: &Rgb, lines: &[Line]) -> (Vec<DesignObject>, [i32; 3], ReconstructSummary) {
    let (w, h) = (img.w, img.h);
    let edge = (0..w).flat_map(|x| [(x, 0), (x, h - 1)]).chain((0..h).flat_map(|y| [(0, y), (w - 1, y)]));
    let (bg, _) = dominant(edge.map(|(x, y)| img.at(x, y)));
    // Ink: everything that is not background.
    let mut mask: Vec<bool> = (0..w * h).map(|i| dist(img.at(i % w, i / w), bg) > SAME).collect();

    let mut texts = Vec::new();
    let mut summary = ReconstructSummary::default();
    let mut conf_sum = 0f32;
    for l in lines.iter().filter(|l| !l.text.is_empty() && l.x1 > l.x0 + 1 && l.y1 > l.y0 + 1) {
        // What surrounds the line: a ring just outside its box.
        let (rx0, ry0, rx1, ry1) =
            (l.x0.saturating_sub(2), l.y0.saturating_sub(2), (l.x1 + 2).min(w), (l.y1 + 2).min(h));
        let ring =
            (rx0..rx1).flat_map(|x| [(x, ry0), (x, ry1 - 1)]).chain((ry0..ry1).flat_map(|y| [(rx0, y), (rx1 - 1, y)]));
        let (behind, uniform) = dominant(ring.map(|(x, y)| img.at(x, y)));
        // Ink of the line: pixels far from what is behind it.
        let (mut ix0, mut iy0, mut ix1, mut iy1) = (usize::MAX, usize::MAX, 0, 0);
        let mut ink = Vec::new();
        let mut runs = (0u32, 0u32); // (ink pixels, ink runs) along rows: stroke thickness
        for y in l.y0..l.y1 {
            let mut inside = false;
            for x in l.x0..l.x1 {
                let c = img.at(x, y);
                let on = dist(c, behind) > INK;
                if on {
                    ink.push(c);
                    (ix0, iy0, ix1, iy1) = (ix0.min(x), iy0.min(y), ix1.max(x + 1), iy1.max(y + 1));
                    runs.0 += 1;
                    if !inside {
                        runs.1 += 1;
                    }
                }
                inside = on;
            }
        }
        if ink.is_empty() {
            continue;
        }
        // The line's own pixels leave the ink mask: on plain background they were text; on a filled
        // box they are holes in the box, filled so the box is found whole.
        let on_background = dist(behind, bg) <= SAME;
        for y in l.y0..l.y1 {
            for x in l.x0..l.x1 {
                mask[y * w + x] = !on_background && uniform > 0.6;
            }
        }
        // Text colour: the ink pixels furthest from the background behind it (anti-aliased edges
        // are in between).
        ink.sort_by_key(|c| -dist(*c, behind));
        let core = &ink[..(ink.len() / 2).max(1)];
        let color = [0, 1, 2].map(|k| core.iter().map(|c| c[k]).sum::<i32>() / core.len() as i32);
        // Size from the ink height and which letters reach above x-height / below the baseline
        // (Helvetica-like proportions: cap height 0.72 em, x-height 0.52, descender 0.21).
        let tall = l.text.chars().any(|c| c.is_uppercase() || c.is_ascii_digit() || "bdfhklt/()[]{}|!?'\"".contains(c));
        let above = if tall { 0.72 } else { 0.52 };
        let below = if l.text.chars().any(|c| "gjpqy".contains(c)) {
            0.21
        } else if l.text.chars().any(|c| ",;".contains(c)) {
            0.12
        } else {
            0.0
        };
        let ink_h = (iy1 - iy0) as f32;
        let size = ink_h / (above + below);
        let stroke = runs.0 as f32 / runs.1.max(1) as f32;
        // Anti-aliased edges add about a pixel to every stroke. Measured on the poster fixture:
        // regular 0.08–0.10, bold 0.18–0.20 of the size.
        let bold = (stroke - 1.0) / size > 0.14;
        let note = if uniform < 0.6 {
            Some("Over a picture: the original wording also stays in the picture.".to_string())
        } else if l.score < 0.8 {
            Some("Low recognition confidence: check the wording.".to_string())
        } else {
            None
        };
        summary.low_confidence_text += (l.score < 0.8) as u32;
        conf_sum += l.score;
        texts.push(DesignObject {
            id: String::new(),
            name: l.text.chars().take(40).collect(),
            kind: ObjectKind::Text,
            x: ix0 as f32,
            y: iy0 as f32,
            w: (ix1 - ix0) as f32,
            h: ink_h,
            visible: true,
            locked: false,
            confidence: l.score,
            note,
            text: Some(l.text.clone()),
            color: Some(hex(color)),
            font_size: Some((size * 10.0).round() / 10.0),
            bold,
            baseline: Some(((above * size) * 10.0).round() / 10.0),
            fill: None,
            png: None,
        });
    }

    let mut regions = regions(img, &mask, bg);
    // Back to front: large regions first.
    regions.sort_by(|a, b| (b.w * b.h).total_cmp(&(a.w * a.h)));
    summary.text = texts.len() as u32;
    summary.shapes = regions.iter().filter(|o| o.kind == ObjectKind::Shape).count() as u32;
    summary.images = regions.iter().filter(|o| o.kind == ObjectKind::Image).count() as u32;
    let mut objects: Vec<DesignObject> = regions.into_iter().chain(texts).collect();
    let mut counts = [0u32; 3];
    for o in &mut objects {
        let k = o.kind as usize;
        counts[k] += 1;
        o.id = format!("{}-{}", ["text", "shape", "picture"][k], counts[k]);
        if o.kind != ObjectKind::Text {
            o.name = format!("{} {}", ["Text", "Shape", "Picture"][k], counts[k]);
        }
    }
    let total = objects.len().max(1) as f32;
    summary.editable_ratio = (summary.text + summary.shapes) as f32 / total;
    summary.mean_text_confidence = if summary.text > 0 { conf_sum / summary.text as f32 } else { 0.0 };
    (objects, bg, summary)
}

/// Connected ink regions (8-neighbours) as shapes or pictures.
fn regions(img: &Rgb, mask: &[bool], bg: [i32; 3]) -> Vec<DesignObject> {
    let (w, h) = (img.w, img.h);
    let mut label = vec![0u32; w * h];
    let mut found: Vec<(usize, usize, usize, usize, Vec<usize>)> = Vec::new();
    let mut stack = Vec::new();
    for start in 0..w * h {
        if !mask[start] || label[start] != 0 {
            continue;
        }
        let id = found.len() as u32 + 1;
        label[start] = id;
        stack.push(start);
        let mut pixels = Vec::new();
        let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0, 0);
        while let Some(i) = stack.pop() {
            let (x, y) = (i % w, i / w);
            pixels.push(i);
            (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x + 1), y1.max(y + 1));
            for dy in -1i64..=1 {
                for dx in -1i64..=1 {
                    let (nx, ny) = (x as i64 + dx, y as i64 + dy);
                    if nx < 0 || ny < 0 || nx >= w as i64 || ny >= h as i64 {
                        continue;
                    }
                    let j = ny as usize * w + nx as usize;
                    if mask[j] && label[j] == 0 {
                        label[j] = id;
                        stack.push(j);
                    }
                }
            }
        }
        if pixels.len() >= SPECK {
            found.push((x0, y0, x1, y1, pixels));
        }
    }
    // Too many regions (noise, texture): keep the largest, and the rest together as one picture.
    found.sort_by_key(|r| std::cmp::Reverse(r.4.len()));
    if found.len() > MAX_REGIONS {
        let rest = found.split_off(MAX_REGIONS - 1);
        let mut merged = (w, h, 0, 0, Vec::new());
        for (x0, y0, x1, y1, pixels) in rest {
            (merged.0, merged.1, merged.2, merged.3) =
                (merged.0.min(x0), merged.1.min(y0), merged.2.max(x1), merged.3.max(y1));
            merged.4.extend(pixels);
        }
        found.push(merged);
    }
    found
        .into_iter()
        .map(|(x0, y0, x1, y1, pixels)| {
            let (rw, rh) = (x1 - x0, y1 - y0);
            let (fill, share) = dominant(pixels.iter().map(|&i| img.at(i % w, i / w)));
            let filled = pixels.len() as f32 / (rw * rh) as f32;
            let base = DesignObject {
                id: String::new(),
                name: String::new(),
                kind: ObjectKind::Shape,
                x: x0 as f32,
                y: y0 as f32,
                w: rw as f32,
                h: rh as f32,
                visible: true,
                locked: false,
                confidence: share.min(filled),
                note: None,
                text: None,
                color: None,
                font_size: None,
                bold: false,
                baseline: None,
                fill: None,
                png: None,
            };
            if share > 0.92 && filled > 0.92 {
                DesignObject { fill: Some(hex(fill)), ..base }
            } else {
                // A picture: the region's own pixels; background around it is transparent.
                let mut rgba = image::RgbaImage::new(rw as u32, rh as u32);
                for y in y0..y1 {
                    for x in x0..x1 {
                        let c = img.at(x, y);
                        let a = if mask[y * w + x] || dist(c, bg) > SAME { 255 } else { 0 };
                        rgba.put_pixel(
                            (x - x0) as u32,
                            (y - y0) as u32,
                            image::Rgba([c[0] as u8, c[1] as u8, c[2] as u8, a]),
                        );
                    }
                }
                DesignObject {
                    kind: ObjectKind::Image,
                    confidence: 0.0,
                    note: Some("Kept as a picture: not recognised as text or a plain shape.".into()),
                    png: Some(png_base64(&image::DynamicImage::ImageRgba8(rgba))),
                    ..base
                }
            }
        })
        .collect()
}

fn png_base64(img: &image::DynamicImage) -> String {
    let mut out = Vec::new();
    // Encoding into memory cannot fail for a valid image.
    let _ = img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png);
    base64::engine::general_purpose::STANDARD.encode(out)
}

fn decode_png(b64: &str, what: &str) -> PwResult<image::DynamicImage> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(b64.as_bytes())
        .map_err(|_| PwError::invalid("design.png", format!("{what} is not valid picture data.")))?;
    let mut reader = image::ImageReader::with_format(std::io::Cursor::new(bytes), image::ImageFormat::Png);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(20_000);
    limits.max_image_height = Some(20_000);
    limits.max_alloc = Some(512 * 1024 * 1024);
    reader.limits(limits);
    reader.decode().map_err(|_| PwError::invalid("design.png", format!("{what} is not a readable PNG.")))
}

/* ---------------- Sources ---------------- */

fn rgb_from(img: image::DynamicImage) -> Rgb {
    let img = if img.width().max(img.height()) > MAX_SIDE {
        img.resize(MAX_SIDE, MAX_SIDE, image::imageops::FilterType::Lanczos3)
    } else {
        img
    };
    // Transparent areas count as white (how they are usually shown).
    let rgba = img.to_rgba8();
    let mut px = Vec::with_capacity(rgba.len() / 4 * 3);
    for p in rgba.pixels() {
        let a = p[3] as u32;
        for k in 0..3 {
            px.push(((p[k] as u32 * a + 255 * (255 - a)) / 255) as u8);
        }
    }
    Rgb { w: rgba.width() as usize, h: rgba.height() as usize, px }
}

/// The picture to rebuild and PDF points per pixel: a PNG/JPEG file, or a page of a PDF.
pub fn load_source(input: &Path, opts: &OpenOptions, page: u32) -> PwResult<(Rgb, f32)> {
    let head = {
        use std::io::Read;
        let mut f = std::fs::File::open(input).map_err(|e| PwError::from_io(&e, "file"))?;
        let mut b = [0u8; 8];
        let n = f.read(&mut b).map_err(|e| PwError::from_io(&e, "file"))?;
        b[..n].to_vec()
    };
    if head.starts_with(b"%PDF") {
        let pdfium = engine()?;
        let doc = load(&pdfium, input, opts)?;
        let pg = doc
            .pages()
            .get(page as PdfPageIndex)
            .map_err(|_| PwError::invalid("design.page", format!("The document has no page {}.", page + 1)))?;
        let (dw, dh) = (pg.width().value, pg.height().value);
        // About 150 dpi, at most MAX_SIDE pixels.
        let scale = (150.0 / 72.0f32).min(MAX_SIDE as f32 / dw.max(dh).max(1.0));
        let cfg = PdfRenderConfig::new().scale_page_by_factor(scale).render_form_data(true).render_annotations(true);
        let bmp = pg.render_with_config(&cfg).map_err(|e| map_pdfium_error(&e, "render"))?;
        let img = bmp.as_image().map_err(|e| map_pdfium_error(&e, "render"))?;
        let rgb = rgb_from(img);
        let pt = dw / rgb.w as f32;
        return Ok((rgb, pt));
    }
    let fmt =
        image::guess_format(&head).ok().filter(|f| matches!(f, image::ImageFormat::Png | image::ImageFormat::Jpeg));
    let Some(fmt) = fmt else {
        return Err(PwError::new(ErrorCategory::UnsupportedFormat, "design.format", "Choose a PDF, PNG or JPEG file."));
    };
    let meta = std::fs::metadata(input).map_err(|e| PwError::from_io(&e, "image"))?;
    if meta.len() > MAX_IMAGE_FILE {
        return Err(PwError::invalid("design.too_large", "The picture is larger than 100 MB."));
    }
    let bytes = std::fs::read(input).map_err(|e| PwError::from_io(&e, "image"))?;
    let mut reader = image::ImageReader::with_format(std::io::Cursor::new(bytes), fmt);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(20_000);
    limits.max_image_height = Some(20_000);
    limits.max_alloc = Some(1024 * 1024 * 1024);
    reader.limits(limits);
    let img = reader.decode().map_err(|_| PwError::invalid("design.unreadable", "The picture could not be read."))?;
    let scale = img.width().max(img.height()) as f32 / MAX_SIDE as f32;
    let rgb = rgb_from(img);
    // 96 pixels per inch, as an image of that size is usually meant (scaled with the picture).
    Ok((rgb, 0.75 * scale.max(1.0)))
}

/// A picture file (PNG or JPEG) for replacing a picture in a design: decoded with limits, at most
/// 4096 px on the long side, re-encoded as PNG (base64). Untrusted bytes are never passed through.
pub fn picture_png(path: &Path) -> PwResult<String> {
    let head = {
        use std::io::Read;
        let mut f = std::fs::File::open(path).map_err(|e| PwError::from_io(&e, "picture"))?;
        let mut b = [0u8; 8];
        let n = f.read(&mut b).map_err(|e| PwError::from_io(&e, "picture"))?;
        b[..n].to_vec()
    };
    let fmt = image::guess_format(&head)
        .ok()
        .filter(|f| matches!(f, image::ImageFormat::Png | image::ImageFormat::Jpeg))
        .ok_or_else(|| {
            PwError::new(ErrorCategory::UnsupportedFormat, "design.picture_format", "Choose a PNG or JPEG picture.")
        })?;
    if std::fs::metadata(path).map_err(|e| PwError::from_io(&e, "picture"))?.len() > MAX_IMAGE_FILE {
        return Err(PwError::invalid("design.too_large", "The picture is larger than 100 MB."));
    }
    let bytes = std::fs::read(path).map_err(|e| PwError::from_io(&e, "picture"))?;
    let mut reader = image::ImageReader::with_format(std::io::Cursor::new(bytes), fmt);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(20_000);
    limits.max_image_height = Some(20_000);
    limits.max_alloc = Some(1024 * 1024 * 1024);
    reader.limits(limits);
    let img = reader.decode().map_err(|_| PwError::invalid("design.unreadable", "The picture could not be read."))?;
    let img = if img.width().max(img.height()) > 4096 {
        img.resize(4096, 4096, image::imageops::FilterType::Lanczos3)
    } else {
        img
    };
    Ok(png_base64(&image::DynamicImage::ImageRgba8(img.to_rgba8())))
}

/// Rebuild `input` (page `page` of a PDF, or a picture) and save it as a design project.
pub fn reconstruct_design(
    input: &Path,
    opts: &OpenOptions,
    page: u32,
    output: &Path,
    policy: CollisionPolicy,
    cancelled: &dyn Fn() -> bool,
) -> PwResult<(CommittedOutput, ReconstructSummary)> {
    check_extension(output)?;
    let (img, pt_per_px) = load_source(input, opts, page)?;
    let resources = pw_ocr::default_resource_dir().ok_or_else(|| {
        PwError::new(
            ErrorCategory::MissingResource,
            "ocr.resource_missing",
            "Text recognition is not installed. Reinstall PageWeave or run the resource setup.",
        )
    })?;
    let ocr = pw_ocr::with_engine(&resources, |e| e.recognize(img.w, img.h, &img.px, 3, cancelled))?;
    if cancelled() {
        return Err(PwError::cancelled());
    }
    let lines: Vec<Line> = ocr.iter().map(|l| Line::from_ocr(l, img.w, img.h)).collect();
    let (objects, bg, summary) = reconstruct(&img, &lines);
    let source = image::RgbImage::from_raw(img.w as u32, img.h as u32, img.px.clone())
        .ok_or_else(|| PwError::internal("design.source", "The source picture could not be kept."))?;
    let name = input.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let design = Design {
        version: DESIGN_VERSION,
        width: img.w as u32,
        height: img.h as u32,
        pt_per_px,
        background: hex(bg),
        source_png: png_base64(&image::DynamicImage::ImageRgb8(source)),
        source_name: if name.to_lowercase().ends_with(".pdf") { format!("{name}, page {}", page + 1) } else { name },
        objects,
    };
    let c = save_design(&design, output, policy, &[input.to_path_buf()])?;
    Ok((c, summary))
}

/* ---------------- Projects ---------------- */

fn check_extension(output: &Path) -> PwResult<()> {
    if output.extension().and_then(|e| e.to_str()).map(|e| e.eq_ignore_ascii_case(DESIGN_EXTENSION)) != Some(true) {
        return Err(PwError::invalid("design.extension", "Design projects are saved as .pwdesign files."));
    }
    Ok(())
}

impl Design {
    /// Reject what a hand-edited or hostile project could hold: absurd sizes, bad colours or
    /// numbers, unreadable pictures, too many objects.
    pub fn validate(&self) -> PwResult<()> {
        let bad = |msg: String| Err(PwError::invalid("design.invalid", msg));
        if self.version != DESIGN_VERSION {
            return bad(format!("This design was made by a newer PageWeave (format {}).", self.version));
        }
        if !(1..=20_000).contains(&self.width) || !(1..=20_000).contains(&self.height) {
            return bad("The design size is not valid.".into());
        }
        if !(self.pt_per_px.is_finite() && self.pt_per_px > 0.0 && self.pt_per_px < 100.0) {
            return bad("The design scale is not valid.".into());
        }
        if parse_hex(&self.background).is_none() {
            return bad("The background colour is not valid.".into());
        }
        if self.objects.len() > MAX_OBJECTS {
            return bad(format!("A design can hold at most {MAX_OBJECTS} objects."));
        }
        let mut ids = std::collections::HashSet::new();
        for o in &self.objects {
            let finite =
                [o.x, o.y, o.w, o.h].iter().all(|v| v.is_finite() && v.abs() < 1.0e6) && o.w >= 0.0 && o.h >= 0.0;
            if !finite
                || o.id.is_empty()
                || o.id.len() > 64
                || !ids.insert(o.id.as_str())
                || o.name.chars().count() > 200
            {
                return bad(format!("The object “{}” is not valid.", o.name.chars().take(40).collect::<String>()));
            }
            match o.kind {
                ObjectKind::Text => {
                    let ok = o.text.as_ref().is_some_and(|t| t.chars().count() <= MAX_TEXT)
                        && o.color.as_deref().and_then(parse_hex).is_some()
                        && o.font_size.is_some_and(|s| s.is_finite() && s > 0.0 && s < 5000.0)
                        && o.baseline.is_none_or(|b| b.is_finite() && b.abs() < 1.0e5);
                    if !ok {
                        return bad(format!(
                            "The text “{}” is not valid.",
                            o.name.chars().take(40).collect::<String>()
                        ));
                    }
                }
                ObjectKind::Shape => {
                    if o.fill.as_deref().and_then(parse_hex).is_none() {
                        return bad(format!("The shape “{}” has no valid colour.", o.name));
                    }
                }
                ObjectKind::Image => {
                    decode_png(o.png.as_deref().unwrap_or(""), &o.name)?;
                }
            }
        }
        Ok(())
    }
}

pub fn read_design(path: &Path) -> PwResult<Design> {
    check_extension(path)?;
    let meta = std::fs::metadata(path).map_err(|e| PwError::from_io(&e, "design"))?;
    if meta.len() > MAX_DESIGN_BYTES {
        return Err(PwError::invalid("design.too_large", "The design file is larger than 256 MB."));
    }
    let bytes = std::fs::read(path).map_err(|e| PwError::from_io(&e, "design"))?;
    let d: Design = serde_json::from_slice(&bytes).map_err(|_| {
        PwError::new(ErrorCategory::UnsupportedFormat, "design.unreadable", "This is not a PageWeave design file.")
    })?;
    d.validate()?;
    Ok(d)
}

/// Save a design project (validated, written atomically, read back).
pub fn save_design(
    design: &Design,
    output: &Path,
    policy: CollisionPolicy,
    protected: &[std::path::PathBuf],
) -> PwResult<CommittedOutput> {
    check_extension(output)?;
    design.validate()?;
    let bytes = serde_json::to_vec(design)
        .map_err(|_| PwError::internal("design.write", "The design could not be written."))?;
    write_atomic_bytes(output, policy, protected, &bytes, |staged| {
        let back: Design = serde_json::from_slice(&std::fs::read(staged).map_err(|e| PwError::from_io(&e, "design"))?)
            .map_err(|_| {
                PwError::new(
                    ErrorCategory::ValidationFailed,
                    "output.verify",
                    "The saved design could not be read back.",
                )
            })?;
        if &back != design {
            return Err(PwError::new(
                ErrorCategory::ValidationFailed,
                "output.verify",
                "The saved design differs from the design.",
            ));
        }
        Ok(())
    })
}

/* ---------------- Export ---------------- */

pub use pw_core::ops::DesignFormat;

fn xml(s: &str) -> String {
    s.chars()
        .filter(|&c| matches!(c, '\t' | '\n' | '\r') || (c >= ' ' && c != '\u{fffe}' && c != '\u{ffff}'))
        .map(|c| match c {
            '&' => "&amp;".into(),
            '<' => "&lt;".into(),
            '>' => "&gt;".into(),
            '"' => "&quot;".into(),
            c => c.to_string(),
        })
        .collect()
}

/// Editable SVG: text as <text>, shapes as <rect>, pictures as embedded PNG <image>. Hidden
/// objects are left out. Nothing refers outside the file; no scripts or styles.
pub fn to_svg(d: &Design) -> String {
    let mut s = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w}\" height=\"{h}\" viewBox=\"0 0 {w} {h}\">\n<rect id=\"background\" width=\"{w}\" height=\"{h}\" fill=\"{bg}\"/>\n",
        w = d.width,
        h = d.height,
        bg = d.background
    );
    for o in d.objects.iter().filter(|o| o.visible) {
        let id = format!("id=\"{}\"", xml(&o.id));
        match o.kind {
            ObjectKind::Shape => s += &format!(
                "<rect {id} x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" fill=\"{}\"/>\n",
                o.x, o.y, o.w, o.h, o.fill.as_deref().unwrap_or("#000000")
            ),
            ObjectKind::Image => s += &format!(
                "<image {id} x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" preserveAspectRatio=\"none\" href=\"data:image/png;base64,{}\"/>\n",
                o.x, o.y, o.w, o.h, o.png.as_deref().unwrap_or("")
            ),
            ObjectKind::Text => {
                let size = o.font_size.unwrap_or(o.h);
                s += &format!(
                    "<text {id} x=\"{}\" y=\"{}\" font-family=\"Helvetica, Arial, sans-serif\" font-size=\"{}\" font-weight=\"{}\" fill=\"{}\" textLength=\"{}\" lengthAdjust=\"spacingAndGlyphs\" xml:space=\"preserve\">{}</text>\n",
                    o.x,
                    o.y + o.baseline.unwrap_or(size * 0.72),
                    size,
                    if o.bold { "bold" } else { "normal" },
                    o.color.as_deref().unwrap_or("#000000"),
                    o.w,
                    xml(o.text.as_deref().unwrap_or(""))
                )
            }
        }
    }
    s + "</svg>\n"
}

/// What an exported SVG holds, read back with an XML parser: element counts and text, plus
/// anything that must never be there.
fn check_svg(bytes: &[u8], d: &Design) -> PwResult<()> {
    use quick_xml::events::Event;
    use quick_xml::XmlVersion;
    let fail = |m: &str| {
        PwError::new(ErrorCategory::ValidationFailed, "output.verify", format!("The SVG did not verify: {m}."))
    };
    let mut r = quick_xml::Reader::from_reader(bytes);
    let (mut texts, mut rects, mut images) = (Vec::new(), 0usize, 0usize);
    let mut in_text = false;
    let mut buf = Vec::new();
    loop {
        match r.read_event_into(&mut buf).map_err(|_| fail("not well-formed"))? {
            Event::Start(e) | Event::Empty(e) => {
                match e.name().as_ref() {
                    "text" => {
                        in_text = true;
                        texts.push(String::new());
                    }
                    "rect" => rects += 1,
                    "image" => images += 1,
                    "svg" => {}
                    _ => return Err(fail("unexpected element")),
                }
                for a in e.attributes().flatten() {
                    let v = a.normalized_value(XmlVersion::Implicit1_0).map_err(|_| fail("bad attribute"))?;
                    let key = a.key.as_ref();
                    if key.ends_with("href") && !v.starts_with("data:image/png;base64,") {
                        return Err(fail("external reference"));
                    }
                    if key.starts_with("on") {
                        return Err(fail("event handler"));
                    }
                }
            }
            Event::Text(t) if in_text => {
                if let Some(last) = texts.last_mut() {
                    last.push_str(&t.xml10_content());
                }
            }
            Event::GeneralRef(e) if in_text => {
                let c = match e.resolve_char_ref() {
                    Ok(Some(c)) => c,
                    Ok(None) => match e.xml10_content().as_ref() {
                        "amp" => '&',
                        "lt" => '<',
                        "gt" => '>',
                        "quot" => '"',
                        "apos" => '\'',
                        _ => return Err(fail("unknown entity")),
                    },
                    Err(_) => return Err(fail("bad character reference")),
                };
                if let Some(last) = texts.last_mut() {
                    last.push(c);
                }
            }
            Event::End(e) if e.name().as_ref() == "text" => in_text = false,
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    let shown: Vec<&DesignObject> = d.objects.iter().filter(|o| o.visible).collect();
    let want_text: Vec<String> = shown
        .iter()
        .filter(|o| o.kind == ObjectKind::Text)
        .map(|o| xml_roundtrip(o.text.as_deref().unwrap_or("")))
        .collect();
    let want_rects = 1 + shown.iter().filter(|o| o.kind == ObjectKind::Shape).count();
    let want_images = shown.iter().filter(|o| o.kind == ObjectKind::Image).count();
    if texts != want_text || rects != want_rects || images != want_images {
        return Err(fail("its objects differ from the design"));
    }
    Ok(())
}

/// Text as it reads back from the SVG (characters XML cannot hold are dropped on export).
fn xml_roundtrip(s: &str) -> String {
    s.chars().filter(|&c| matches!(c, '\t' | '\n' | '\r') || (c >= ' ' && c != '\u{fffe}' && c != '\u{ffff}')).collect()
}

/// Characters the built-in Helvetica (WinAnsi) can carry.
fn winansi(c: char) -> bool {
    matches!(c, ' '..='~' | '\u{a0}'..='\u{ff}') || "€‚ƒ„…†‡ˆ‰Š‹ŒŽ‘’“”•–—˜™š›œžŸ".contains(c)
}

fn pdf_color(hex6: &str, alpha: u8) -> PdfColor {
    let c = parse_hex(hex6).unwrap_or([0, 0, 0]);
    PdfColor::new(c[0] as u8, c[1] as u8, c[2] as u8, alpha)
}

/// A one-page PDF of the design: real text (built-in Helvetica, stretched to the box width),
/// vector rectangles and embedded pictures.
fn to_pdf(d: &Design) -> PwResult<(Vec<u8>, Vec<String>)> {
    let pdfium = engine()?;
    let err = |e: PdfiumError| map_pdfium_error(&e, "design export");
    let mut doc = pdfium.create_new_pdf().map_err(err)?;
    let k = d.pt_per_px;
    let (pw, ph) = (d.width as f32 * k, d.height as f32 * k);
    let regular = doc.fonts_mut().helvetica();
    let bold = doc.fonts_mut().helvetica_bold();
    let mut page = doc
        .pages_mut()
        .create_page_at_end(PdfPagePaperSize::Custom(PdfPoints::new(pw), PdfPoints::new(ph)))
        .map_err(err)?;
    let rect = |x: f32, y: f32, w: f32, h: f32| {
        PdfRect::new_from_values((ph - (y + h) * k).max(-1.0e5), x * k, ph - y * k, (x + w) * k)
    };
    let bg = PdfPagePathObject::new_rect(
        &doc,
        rect(0.0, 0.0, d.width as f32, d.height as f32),
        None,
        None,
        Some(pdf_color(&d.background, 255)),
    )
    .map_err(err)?;
    page.objects_mut().add_path_object(bg).map_err(err)?;
    let mut dropped = 0usize;
    for o in d.objects.iter().filter(|o| o.visible && o.w > 0.0 && o.h > 0.0) {
        match o.kind {
            ObjectKind::Shape => {
                let p = PdfPagePathObject::new_rect(
                    &doc,
                    rect(o.x, o.y, o.w, o.h),
                    None,
                    None,
                    Some(pdf_color(o.fill.as_deref().unwrap_or(""), 255)),
                )
                .map_err(err)?;
                page.objects_mut().add_path_object(p).map_err(err)?;
            }
            ObjectKind::Image => {
                let img = decode_png(o.png.as_deref().unwrap_or(""), &o.name)?;
                let mut obj = PdfPageImageObject::new(&doc, &img).map_err(err)?;
                obj.apply_matrix(PdfMatrix::new(o.w * k, 0.0, 0.0, o.h * k, o.x * k, ph - (o.y + o.h) * k))
                    .map_err(err)?;
                page.objects_mut().add_image_object(obj).map_err(err)?;
            }
            ObjectKind::Text => {
                let wording: String =
                    o.text.as_deref().unwrap_or("").chars().filter(|&c| winansi(c) || c == ' ').collect();
                dropped += o.text.as_deref().unwrap_or("").chars().count() - wording.chars().count();
                if wording.trim().is_empty() {
                    continue;
                }
                let size = o.font_size.unwrap_or(o.h) * k;
                let mut t =
                    PdfPageTextObject::new(&doc, &wording, if o.bold { bold } else { regular }, PdfPoints::new(size))
                        .map_err(err)?;
                t.set_fill_color(pdf_color(o.color.as_deref().unwrap_or(""), 255)).map_err(err)?;
                let natural = t.width().map_err(err)?.value.max(0.01);
                let sx = (o.w * k / natural).clamp(0.2, 5.0);
                let base = ph - (o.y + o.baseline.unwrap_or(o.font_size.unwrap_or(o.h) * 0.72)) * k;
                t.apply_matrix(PdfMatrix::new(sx, 0.0, 0.0, 1.0, o.x * k, base)).map_err(err)?;
                page.objects_mut().add_text_object(t).map_err(err)?;
            }
        }
    }
    let mut warnings = Vec::new();
    if dropped > 0 {
        warnings.push(format!(
            "{dropped} character(s) are outside the PDF's built-in font and were left out of the PDF; the SVG export keeps them."
        ));
    }
    Ok((save_bytes(&doc)?, warnings))
}

fn check_pdf(path: &Path, d: &Design) -> PwResult<()> {
    let pdfium = engine()?;
    let fail = |m: &str| {
        PwError::new(ErrorCategory::ValidationFailed, "output.verify", format!("The PDF did not verify: {m}."))
    };
    let doc = pdfium.load_pdf_from_file(path, None).map_err(|_| fail("it could not be reopened"))?;
    if doc.pages().len() != 1 {
        return Err(fail("wrong page count"));
    }
    let page = doc.pages().get(0).map_err(|e| map_pdfium_error(&e, "page"))?;
    let everything = PdfRect::new_from_values(-1.0e6, -1.0e6, 1.0e6, 1.0e6);
    let squash = |t: &str| t.chars().filter(|c| !c.is_whitespace()).collect::<String>();
    let got = squash(&page.text().map_err(|e| map_pdfium_error(&e, "text"))?.inside_rect(everything));
    for o in d.objects.iter().filter(|o| o.visible && o.kind == ObjectKind::Text) {
        let want = squash(&o.text.as_deref().unwrap_or("").chars().filter(|&c| winansi(c)).collect::<String>());
        if !got.contains(&want) {
            return Err(fail("text is missing"));
        }
    }
    let pictures = page.objects().iter().filter(|o| matches!(o, PdfPageObject::Image(_))).count();
    let want = d.objects.iter().filter(|o| o.visible && o.kind == ObjectKind::Image && o.w > 0.0 && o.h > 0.0).count();
    if pictures != want {
        return Err(fail("pictures are missing"));
    }
    Ok(())
}

/// Export a design project as SVG or PDF; the file is read back and checked before it appears.
pub fn export_design(
    input: &Path,
    format: DesignFormat,
    output: &Path,
    policy: CollisionPolicy,
) -> PwResult<(CommittedOutput, Vec<String>)> {
    let ext = match format {
        DesignFormat::Svg => "svg",
        DesignFormat::Pdf => "pdf",
    };
    if output.extension().and_then(|e| e.to_str()).map(|e| e.eq_ignore_ascii_case(ext)) != Some(true) {
        return Err(PwError::invalid("design.extension", format!("Save the export as a .{ext} file.")));
    }
    let d = read_design(input)?;
    let protected = [input.to_path_buf()];
    match format {
        DesignFormat::Svg => {
            let svg = to_svg(&d);
            let c = write_atomic_bytes(output, policy, &protected, svg.as_bytes(), |staged| {
                check_svg(&std::fs::read(staged).map_err(|e| PwError::from_io(&e, "svg"))?, &d)
            })?;
            Ok((c, Vec::new()))
        }
        DesignFormat::Pdf => {
            let (bytes, warnings) = to_pdf(&d)?;
            let c = write_atomic_bytes(output, policy, &protected, &bytes, |staged| check_pdf(staged, &d))?;
            Ok((c, warnings))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn canvas(w: usize, h: usize, bg: [u8; 3]) -> Rgb {
        Rgb { w, h, px: (0..w * h).flat_map(|_| bg).collect() }
    }

    fn fill(img: &mut Rgb, x0: usize, y0: usize, x1: usize, y1: usize, c: [u8; 3]) {
        for y in y0..y1 {
            for x in x0..x1 {
                let i = (y * img.w + x) * 3;
                img.px[i..i + 3].copy_from_slice(&c);
            }
        }
    }

    #[test]
    fn background_box_and_picture_are_told_apart() {
        let mut img = canvas(120, 80, [250, 245, 230]);
        fill(&mut img, 10, 10, 60, 30, [30, 60, 120]); // a box
        for y in 40..70 {
            for x in 70..110 {
                let i = (y * 120 + x) * 3;
                img.px[i..i + 3].copy_from_slice(&[(x * 2) as u8, (y * 3) as u8, 90]);
                // a gradient
            }
        }
        let (objects, bg, s) = reconstruct(&img, &[]);
        assert_eq!(hex(bg), "#faf5e6");
        assert_eq!((s.shapes, s.images, s.text), (1, 1, 0));
        let shape = objects.iter().find(|o| o.kind == ObjectKind::Shape).expect("test value");
        assert_eq!((shape.x, shape.y, shape.w, shape.h), (10.0, 10.0, 50.0, 20.0));
        assert_eq!(shape.fill.as_deref(), Some("#1e3c78"));
        let pic = objects.iter().find(|o| o.kind == ObjectKind::Image).expect("test value");
        assert_eq!((pic.x, pic.y, pic.w, pic.h), (70.0, 40.0, 40.0, 30.0));
        assert!(pic.note.is_some() && pic.confidence == 0.0);
        assert!(decode_png(pic.png.as_deref().expect("test value"), "p").is_ok());
        assert!((s.editable_ratio - 0.5).abs() < 1e-6);
    }

    #[test]
    fn text_on_a_box_leaves_the_box_whole_and_takes_its_ink_colour() {
        let mut img = canvas(200, 60, [255, 255, 255]);
        fill(&mut img, 0, 0, 200, 60, [255, 255, 255]);
        fill(&mut img, 20, 10, 180, 50, [40, 80, 160]); // button
                                                        // "Text": three white bars standing for letters, 20 px tall (cap height).
        for x0 in [40, 70, 100] {
            fill(&mut img, x0, 20, x0 + 6, 40, [255, 255, 255]);
        }
        let line = Line { text: "HIT".into(), score: 0.95, x0: 36, y0: 16, x1: 112, y1: 44 };
        let (objects, _, s) = reconstruct(&img, &[line]);
        assert_eq!((s.shapes, s.images, s.text), (1, 0, 1));
        let t = objects.last().expect("test value");
        assert_eq!(t.kind, ObjectKind::Text);
        assert_eq!(t.color.as_deref(), Some("#ffffff"));
        assert_eq!((t.x, t.y, t.w, t.h), (40.0, 20.0, 66.0, 20.0));
        // Capitals only: ink height is the cap height.
        assert!((t.font_size.expect("test value") - 20.0 / 0.72).abs() < 0.1);
        let b = objects.iter().find(|o| o.kind == ObjectKind::Shape).expect("test value");
        assert_eq!((b.w, b.h), (160.0, 40.0));
    }

    #[test]
    fn noise_becomes_at_most_a_bounded_number_of_objects() {
        let mut img = canvas(300, 300, [255, 255, 255]);
        for y in (0..300).step_by(4) {
            for x in (0..300).step_by(4) {
                fill(&mut img, x, y, x + 3, y + 3, [0, 0, 0]);
            }
        }
        let (objects, _, _) = reconstruct(&img, &[]);
        assert!(objects.len() <= MAX_REGIONS, "{}", objects.len());
    }

    #[test]
    fn svg_escapes_text_and_reads_back() {
        let d = Design {
            version: 1,
            width: 100,
            height: 50,
            pt_per_px: 0.75,
            background: "#ffffff".into(),
            source_png: String::new(),
            source_name: "x".into(),
            objects: vec![DesignObject {
                id: "text-1".into(),
                name: "t".into(),
                kind: ObjectKind::Text,
                x: 1.0,
                y: 2.0,
                w: 50.0,
                h: 10.0,
                visible: true,
                locked: false,
                confidence: 1.0,
                note: None,
                text: Some("A & <B> \"q\" \u{1}".into()),
                color: Some("#123456".into()),
                font_size: Some(12.0),
                bold: true,
                baseline: Some(9.0),
                fill: None,
                png: None,
            }],
        };
        let svg = to_svg(&d);
        assert!(svg.contains("A &amp; &lt;B&gt; &quot;q&quot; </text>"), "{svg}");
        check_svg(svg.as_bytes(), &d).expect("test value");
        // Hostile additions are refused.
        let bad = svg.replace("</svg>", "<script>alert(1)</script></svg>");
        assert!(check_svg(bad.as_bytes(), &d).is_err());
        let ext = svg.replace("<rect id=\"background\"", "<image href=\"http://x/y.png\"/><rect id=\"background\"");
        assert!(check_svg(ext.as_bytes(), &d).is_err());
    }

    #[test]
    fn replacement_pictures_are_decoded_and_re_encoded() {
        let dir = tempfile::tempdir().expect("test value");
        let png = dir.path().join("p.png");
        image::RgbImage::from_pixel(30, 20, image::Rgb([200, 10, 10])).save(&png).expect("test value");
        let b64 = picture_png(&png).expect("test value");
        let back = decode_png(&b64, "p").expect("test value");
        assert_eq!((back.width(), back.height()), (30, 20));
        let txt = dir.path().join("x.png");
        std::fs::write(&txt, b"not a picture").expect("test value");
        assert_eq!(picture_png(&txt).expect_err("test value").code, "design.picture_format");
    }

    #[test]
    fn validation_refuses_bad_designs() {
        let ok = Design {
            version: 1,
            width: 10,
            height: 10,
            pt_per_px: 1.0,
            background: "#ffffff".into(),
            source_png: String::new(),
            source_name: String::new(),
            objects: vec![],
        };
        assert!(ok.validate().is_ok());
        assert!(Design { background: "red".into(), ..ok.clone() }.validate().is_err());
        assert!(Design { width: 0, ..ok.clone() }.validate().is_err());
        assert!(Design { pt_per_px: f32::NAN, ..ok.clone() }.validate().is_err());
        let shape = DesignObject {
            id: "a".into(),
            name: "a".into(),
            kind: ObjectKind::Shape,
            x: 0.0,
            y: 0.0,
            w: 1.0,
            h: 1.0,
            visible: true,
            locked: false,
            confidence: 1.0,
            note: None,
            text: None,
            color: None,
            font_size: None,
            bold: false,
            baseline: None,
            fill: Some("#000000".into()),
            png: None,
        };
        assert!(Design { objects: vec![shape.clone()], ..ok.clone() }.validate().is_ok());
        assert!(
            Design { objects: vec![shape.clone(), shape.clone()], ..ok.clone() }.validate().is_err(),
            "duplicate ids"
        );
        let pic = DesignObject { kind: ObjectKind::Image, png: Some("bm90IGEgcG5n".into()), ..shape.clone() };
        assert!(Design { objects: vec![pic], ..ok.clone() }.validate().is_err(), "not a PNG");
        let inf = DesignObject { x: f32::INFINITY, ..shape };
        assert!(Design { objects: vec![inf], ..ok }.validate().is_err());
    }
}
