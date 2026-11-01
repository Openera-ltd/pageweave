//! Permanent redaction (PRD 05 "Passwords, redaction and sanitization", PRD 11).
//!
//! Black rectangles are not redaction: content under each area is removed from the file, then the
//! area is painted black. Per page:
//! - text: characters at least [`HIT_FRACTION`] under an area are removed; a text object that is
//!   only partly hit is replaced by its remaining characters, each placed at its original origin;
//! - images: removed when inside an area, otherwise replaced by a copy with the area's pixels
//!   painted black (the original image data is dropped);
//! - vector graphics and shadings: removed when inside an area; partly covered ones are kept
//!   (same default as MuPDF's redaction);
//! - annotations and form widgets touching an area are removed.
//!
//! A page whose content cannot be removed precisely (form XObjects or unsupported objects crossing
//! an area, unmappable characters, or a rendered result that differs outside the areas) is
//! replaced by an image of the page with the areas painted black, and the user is told.
//!
//! The output is a new document holding only the pages, then rewritten without unused resources,
//! thumbnails, page metadata, dangling annotation references or links to pages that no longer
//! exist. It is reopened and verified independently before it is committed: no character or
//! annotation remains under an area, image pixels under areas are black, removed image data is
//! absent from the file, the rendering matches outside the areas, and there is no revision history.

use crate::document::{load, OpenOptions};
use crate::edit::{save_bytes, EditOutcome, PROTECTION_REMOVED};
use crate::{engine, map_pdfium_error};
use image::{DynamicImage, Rgba, RgbaImage};
use lopdf::Object;
use pdfium_render::prelude::*;
use pw_core::ops::RedactArea;
use pw_core::output::{write_atomic_bytes, CollisionPolicy};
use pw_core::{ErrorCategory, PwError, PwResult};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::path::Path;

/// A character is removed when at least this share of its glyph box lies under an area.
const HIT_FRACTION: f32 = 0.4;
/// Scale (1.0 = 72 dpi) of the renders compared to check that nothing changed outside the areas.
const CHECK_SCALE: f32 = 1.5;
/// Resolution of pages that have to be replaced by an image.
const RASTER_DPI: f32 = 200.0;
const MAX_RASTER_PIXELS: f32 = 40_000_000.0;
/// Rendering may differ this much outside the areas (anti-aliasing of re-placed glyphs).
const MAX_CHANNEL_DELTA: u8 = 48;
const MAX_CHANGED_SHARE: f64 = 0.0005;

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct R {
    pub(crate) l: f32,
    pub(crate) b: f32,
    pub(crate) r: f32,
    pub(crate) t: f32,
}

impl R {
    pub(crate) fn from_pdf(r: &PdfRect) -> Self {
        R { l: r.left().value, b: r.bottom().value, r: r.right().value, t: r.top().value }
    }
    fn from_area(a: &RedactArea) -> Self {
        R { l: a.rect.left, b: a.rect.bottom, r: a.rect.right, t: a.rect.top }
    }
    fn area(&self) -> f32 {
        (self.r - self.l).max(0.0) * (self.t - self.b).max(0.0)
    }
    fn overlap(&self, o: &R) -> f32 {
        let w = self.r.min(o.r) - self.l.max(o.l);
        let h = self.t.min(o.t) - self.b.max(o.b);
        if w > 0.0 && h > 0.0 {
            w * h
        } else {
            0.0
        }
    }
    fn touches(&self, o: &R) -> bool {
        self.overlap(o) > 0.0
    }
    fn contains_point(&self, x: f32, y: f32) -> bool {
        x >= self.l && x <= self.r && y >= self.b && y <= self.t
    }
    /// `o` lies inside `self`, allowing `tol` points of slack.
    fn contains(&self, o: &R, tol: f32) -> bool {
        o.l >= self.l - tol && o.r <= self.r + tol && o.b >= self.b - tol && o.t <= self.t + tol
    }
    fn union(&self, o: &R) -> R {
        R { l: self.l.min(o.l), b: self.b.min(o.b), r: self.r.max(o.r), t: self.t.max(o.t) }
    }
    fn to_pdf(self) -> PdfRect {
        PdfRect::new_from_values(self.b, self.l, self.t, self.r)
    }
}

/// Whether a character with glyph box `glyph` (and origin, used for blank glyphs) is redacted by `area`.
fn char_hit(glyph: &R, origin: (f32, f32), area: &R) -> bool {
    let a = glyph.area();
    if a <= 0.01 {
        return area.contains_point(origin.0, origin.1);
    }
    glyph.overlap(area) >= HIT_FRACTION * a
}

fn quad_bounds(q: &PdfQuadPoints) -> R {
    let xs = [q.x1().value, q.x2().value, q.x3().value, q.x4().value];
    let ys = [q.y1().value, q.y2().value, q.y3().value, q.y4().value];
    R {
        l: xs.iter().copied().fold(f32::INFINITY, f32::min),
        r: xs.iter().copied().fold(f32::NEG_INFINITY, f32::max),
        b: ys.iter().copied().fold(f32::INFINITY, f32::min),
        t: ys.iter().copied().fold(f32::NEG_INFINITY, f32::max),
    }
}

/// Page-space bounding box of image pixel (x, y) of a `w`×`h` image placed by `m` (unit square → page).
fn pixel_footprint(m: &PdfMatrix, w: f32, h: f32, x: u32, y: u32) -> R {
    let corners = [(0.0f32, 0.0f32), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0)].map(|(dx, dy)| {
        let u = (x as f32 + dx) / w;
        let v = 1.0 - (y as f32 + dy) / h;
        (m.a() * u + m.c() * v + m.e(), m.b() * u + m.d() * v + m.f())
    });
    R {
        l: corners.iter().map(|c| c.0).fold(f32::INFINITY, f32::min),
        r: corners.iter().map(|c| c.0).fold(f32::NEG_INFINITY, f32::max),
        b: corners.iter().map(|c| c.1).fold(f32::INFINITY, f32::min),
        t: corners.iter().map(|c| c.1).fold(f32::NEG_INFINITY, f32::max),
    }
}

/// What redacting one area would remove (shown to the user before applying).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AreaPreview {
    /// 0-based page index.
    pub page: u32,
    /// Text under the area, in reading order.
    pub text: String,
    pub images: u32,
    pub graphics: u32,
    /// Drawings that cross the area's edge: covered by the black box but kept in the file.
    pub graphics_kept: u32,
    pub annotations: u32,
}

/// A character that is kept from a partly redacted text object.
struct KeptChar {
    /// Index in the page's text (consecutive indices form a run).
    index: i32,
    ch: char,
    x: f32,
    y: f32,
}

/// Group kept characters into runs of consecutive characters: (text, origin of the first).
fn runs(kept: &[KeptChar]) -> Vec<(String, f32, f32)> {
    let mut out: Vec<(String, f32, f32)> = Vec::new();
    let mut prev: Option<i32> = None;
    for k in kept {
        match out.last_mut() {
            Some(run) if prev.is_some_and(|p| p + 1 == k.index) => run.0.push(k.ch),
            _ => out.push((k.ch.to_string(), k.x, k.y)),
        }
        prev = Some(k.index);
    }
    out
}

#[derive(Default)]
struct PagePlan {
    /// Painted black (each area grown to cover the characters it removes).
    boxes: Vec<R>,
    areas: Vec<R>,
    /// Object indices to remove.
    remove: Vec<usize>,
    /// Partly redacted text objects: index and the characters to keep.
    split: Vec<(usize, Vec<KeptChar>)>,
    /// Images that cross an area edge: their pixels under the areas are painted black.
    edit_images: Vec<usize>,
    annotations: Vec<usize>,
    /// Characters removed (for verification): character and origin.
    removed_chars: Vec<(char, f32, f32)>,
    /// Why the page must be replaced by an image, if it must.
    raster: Option<&'static str>,
}

fn page_areas(areas: &[RedactArea], count: u32) -> PwResult<BTreeMap<u32, Vec<R>>> {
    let mut by_page: BTreeMap<u32, Vec<R>> = BTreeMap::new();
    for a in areas {
        if a.page >= count {
            return Err(PwError::invalid(
                "redact.page",
                format!("Page {} does not exist; the document has {count} pages.", a.page + 1),
            ));
        }
        by_page.entry(a.page).or_default().push(R::from_area(a));
    }
    Ok(by_page)
}

/// Text under each area, and how many images, graphics and annotations it touches.
pub fn redaction_preview(input: &Path, opts: &OpenOptions, areas: &[RedactArea]) -> PwResult<Vec<AreaPreview>> {
    let pdfium = engine()?;
    let doc = load(&pdfium, input, opts)?;
    let count = doc.pages().len().max(0) as u32;
    let by_page = page_areas(areas, count)?;
    let mut previews = Vec::with_capacity(areas.len());
    for (&p, rects) in &by_page {
        let page = doc.pages().get(p as PdfPageIndex).map_err(|e| map_pdfium_error(&e, "page"))?;
        let text = page.text().map_err(|e| map_pdfium_error(&e, "text"))?;
        let mut texts = vec![String::new(); rects.len()];
        let mut last: Vec<Option<i32>> = vec![None; rects.len()];
        for ch in text.chars().iter() {
            if ch.is_generated().unwrap_or(false) {
                continue;
            }
            let Some(c) = ch.unicode_char() else { continue };
            let glyph = ch.tight_bounds().map(|b| R::from_pdf(&b)).unwrap_or(R { l: 0.0, b: 0.0, r: 0.0, t: 0.0 });
            let origin = ch.origin().map(|(x, y)| (x.value, y.value)).unwrap_or((f32::NAN, f32::NAN));
            for (i, a) in rects.iter().enumerate() {
                if char_hit(&glyph, origin, a) {
                    let idx = ch.index() as i32;
                    if last[i].is_some_and(|l| l + 1 != idx) && !texts[i].ends_with(char::is_whitespace) {
                        texts[i].push(' ');
                    }
                    texts[i].push(c);
                    last[i] = Some(idx);
                }
            }
        }
        let mut images = vec![0u32; rects.len()];
        let mut graphics = vec![0u32; rects.len()];
        let mut graphics_kept = vec![0u32; rects.len()];
        for obj in page.objects().iter() {
            let Ok(q) = obj.bounds() else { continue };
            let bb = quad_bounds(&q);
            for (i, a) in rects.iter().enumerate() {
                if a.touches(&bb) {
                    match obj.object_type() {
                        PdfPageObjectType::Image => images[i] += 1,
                        PdfPageObjectType::Path | PdfPageObjectType::Shading => {
                            graphics[i] += 1;
                            if !a.contains(&bb, 0.5) {
                                graphics_kept[i] += 1;
                            }
                        }
                        PdfPageObjectType::XObjectForm => graphics[i] += 1,
                        _ => {}
                    }
                }
            }
        }
        let mut annots = vec![0u32; rects.len()];
        for annot in page.annotations().iter() {
            let Ok(b) = annot.bounds() else { continue };
            let bb = R::from_pdf(&b);
            for (i, a) in rects.iter().enumerate() {
                if a.touches(&bb) {
                    annots[i] += 1;
                }
            }
        }
        for i in 0..rects.len() {
            previews.push(AreaPreview {
                page: p,
                text: texts[i].trim().to_string(),
                images: images[i],
                graphics: graphics[i],
                graphics_kept: graphics_kept[i],
                annotations: annots[i],
            });
        }
    }
    Ok(previews)
}

fn plan_page(page: &PdfPage, areas: &[R]) -> PwResult<PagePlan> {
    let mut plan = PagePlan { areas: areas.to_vec(), boxes: areas.to_vec(), ..Default::default() };
    let text = page.text().map_err(|e| map_pdfium_error(&e, "text"))?;
    // Which area (if any) redacts each character, by text-page index.
    let mut hit: Vec<Option<usize>> = Vec::new();
    for ch in text.chars().iter() {
        let idx = ch.index();
        if hit.len() <= idx {
            hit.resize(idx + 1, None);
        }
        let glyph = ch.tight_bounds().map(|b| R::from_pdf(&b)).ok();
        let origin = ch.origin().map(|(x, y)| (x.value, y.value)).ok();
        let (Some(glyph), Some(origin)) = (glyph, origin) else {
            // Geometry unknown: if the character might be near an area, do not guess.
            if ch.is_generated().unwrap_or(false) {
                continue;
            }
            plan.raster.get_or_insert("character geometry unavailable");
            continue;
        };
        if let Some(i) = areas.iter().position(|a| char_hit(&glyph, origin, a)) {
            hit[idx] = Some(i);
            if glyph.area() > 0.01 {
                plan.boxes[i] = plan.boxes[i].union(&glyph);
            }
        }
    }
    for (oi, obj) in page.objects().iter().enumerate() {
        let bb = match obj.bounds() {
            Ok(q) => quad_bounds(&q),
            Err(_) => {
                plan.raster.get_or_insert("object bounds unavailable");
                continue;
            }
        };
        if !plan.boxes.iter().any(|b| b.touches(&bb)) && !areas.iter().any(|a| a.touches(&bb)) {
            continue;
        }
        let inside = plan.boxes.iter().any(|b| b.contains(&bb, 0.5));
        match obj.object_type() {
            PdfPageObjectType::Text => {
                let Some(t) = obj.as_text_object() else { continue };
                let chars = text.chars_for_object(t).map_err(|e| map_pdfium_error(&e, "text"))?;
                let mut kept = Vec::new();
                let mut any_hit = false;
                for ch in chars.iter() {
                    let idx = ch.index();
                    let origin = ch.origin().map(|(x, y)| (x.value, y.value));
                    let c = ch.unicode_char();
                    if hit.get(idx).copied().flatten().is_some() {
                        any_hit = true;
                        if let (Some(c), Ok(o)) = (c, origin) {
                            plan.removed_chars.push((c, o.0, o.1));
                        }
                    } else if ch.is_generated().unwrap_or(false) {
                        continue;
                    } else {
                        match (c, origin) {
                            (Some(c), Ok((x, y))) if c != '\u{fffe}' && c != '\u{fffd}' && !c.is_control() => {
                                kept.push(KeptChar { index: ch.index() as i32, ch: c, x, y })
                            }
                            _ => {
                                plan.raster.get_or_insert("text that cannot be re-placed crosses an area");
                            }
                        }
                    }
                }
                if !any_hit {
                    continue;
                }
                if kept.is_empty() {
                    plan.remove.push(oi);
                } else {
                    plan.split.push((oi, kept));
                }
            }
            PdfPageObjectType::Image => {
                if inside {
                    plan.remove.push(oi);
                    continue;
                }
                // Pixels under a box are painted; each box grows to cover whole painted pixels so
                // no partly painted pixel shows outside the black box.
                let dims =
                    obj.as_image_object().and_then(|i| Some((i.width().ok()?, i.height().ok()?, i.matrix().ok()?)));
                let Some((w, h, m)) = dims.filter(|d| d.0 > 0 && d.1 > 0) else {
                    plan.raster.get_or_insert("an image crossing an area cannot be read");
                    continue;
                };
                let (w, h) = (w as f32, h as f32);
                for bi in 0..plan.boxes.len() {
                    let b = plan.boxes[bi];
                    let mut grown = b;
                    for y in 0..h as u32 {
                        for x in 0..w as u32 {
                            let f = pixel_footprint(&m, w, h, x, y);
                            if f.touches(&b) {
                                grown = grown.union(&f);
                            }
                        }
                    }
                    plan.boxes[bi] = grown;
                }
                plan.edit_images.push(oi);
            }
            PdfPageObjectType::Path | PdfPageObjectType::Shading => {
                if inside {
                    plan.remove.push(oi);
                }
            }
            PdfPageObjectType::XObjectForm => {
                if inside {
                    plan.remove.push(oi);
                } else {
                    plan.raster.get_or_insert("grouped content crosses an area");
                }
            }
            _ => {
                plan.raster.get_or_insert("unsupported content crosses an area");
            }
        }
    }
    for (ai, annot) in page.annotations().iter().enumerate() {
        let bb = annot.bounds().map(|b| R::from_pdf(&b));
        // An annotation without readable bounds is removed too: it cannot be shown to be clear.
        if bb.map(|bb| plan.boxes.iter().any(|b| b.touches(&bb))).unwrap_or(true) {
            plan.annotations.push(ai);
        }
    }
    Ok(plan)
}

/// Paint `boxes` (page space) black in an image of the page rendered at `scale` with rotation
/// `rot` (clockwise) of a page whose displayed box is `bbox`.
fn paint_boxes(img: &mut RgbaImage, boxes: &[R], bbox: &R, rot: u32, scale: f32, grow_px: f32) {
    for b in boxes {
        let (x0, y0, x1, y1) = box_to_px(b, bbox, rot, scale);
        let (w, h) = (img.width() as f32, img.height() as f32);
        let xa = (x0 - grow_px).floor().clamp(0.0, w) as u32;
        let xb = (x1 + grow_px).ceil().clamp(0.0, w) as u32;
        let ya = (y0 - grow_px).floor().clamp(0.0, h) as u32;
        let yb = (y1 + grow_px).ceil().clamp(0.0, h) as u32;
        for y in ya..yb {
            for x in xa..xb {
                img.put_pixel(x, y, Rgba([0, 0, 0, 255]));
            }
        }
    }
}

/// Pixel rectangle (x0, y0, x1, y1) of a page-space box in a render of the page.
pub(crate) fn box_to_px(b: &R, bbox: &R, rot: u32, s: f32) -> (f32, f32, f32, f32) {
    let w = (bbox.r - bbox.l) * s;
    let h = (bbox.t - bbox.b) * s;
    let map = |x: f32, y: f32| {
        let (u, v) = ((x - bbox.l) * s, (bbox.t - y) * s);
        match rot {
            90 => (h - v, u),
            180 => (w - u, h - v),
            270 => (v, w - u),
            _ => (u, v),
        }
    };
    let (ax, ay) = map(b.l, b.b);
    let (bx, by) = map(b.r, b.t);
    (ax.min(bx), ay.min(by), ax.max(bx), ay.max(by))
}

pub(crate) fn rotation_degrees(page: &PdfPage) -> u32 {
    match page.rotation() {
        Ok(PdfPageRenderRotation::Degrees90) => 90,
        Ok(PdfPageRenderRotation::Degrees180) => 180,
        Ok(PdfPageRenderRotation::Degrees270) => 270,
        _ => 0,
    }
}

pub(crate) fn displayed_box(page: &PdfPage) -> R {
    page.boundaries().bounding().map(|b| R::from_pdf(&b.bounds)).unwrap_or(R {
        l: 0.0,
        b: 0.0,
        r: page.width().value,
        t: page.height().value,
    })
}

fn render(page: &PdfPage, scale: f32, with_annotations: bool) -> PwResult<RgbaImage> {
    let config = PdfRenderConfig::new()
        .scale_page_by_factor(scale)
        .render_form_data(with_annotations)
        .render_annotations(with_annotations);
    let bitmap = page.render_with_config(&config).map_err(|e| map_pdfium_error(&e, "render"))?;
    Ok(bitmap.as_image().map_err(|e| map_pdfium_error(&e, "render"))?.to_rgba8())
}

/// Number of pixels outside the (grown) boxes that differ noticeably between two renders.
fn changed_outside(a: &RgbaImage, b: &RgbaImage, mask: &RgbaImage) -> Option<(u64, u64)> {
    if a.dimensions() != b.dimensions() || a.dimensions() != mask.dimensions() {
        return None;
    }
    let mut changed = 0u64;
    for ((pa, pb), pm) in a.pixels().zip(b.pixels()).zip(mask.pixels()) {
        if pm.0[3] == 255 && pm.0[0] == 0 && pm.0[1] == 0 && pm.0[2] == 0 {
            continue;
        }
        if (0..3).any(|c| pa.0[c].abs_diff(pb.0[c]) > MAX_CHANNEL_DELTA) {
            changed += 1;
        }
    }
    Some((changed, a.width() as u64 * a.height() as u64))
}

/// How kept characters of a partly redacted text object are written back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TextMode {
    Runs,
    Chars,
}

/// Apply the plan to a page of the working copy. Returns the raw data of every image removed or
/// replaced (checked to be absent from the output).
fn apply_plan<'a>(
    doc: &PdfDocument<'a>,
    page: &mut PdfPage<'a>,
    plan: &PagePlan,
    text_mode: TextMode,
) -> Result<Vec<Vec<u8>>, &'static str> {
    page.set_content_regeneration_strategy(PdfPageContentRegenerationStrategy::Manual);
    let mut dropped_images = Vec::new();
    // Annotations first (highest index first so indices stay valid).
    let mut annots = plan.annotations.clone();
    annots.sort_unstable_by(|a, b| b.cmp(a));
    for ai in annots {
        let annots = page.annotations_mut();
        let a = annots.get(ai as PdfPageAnnotationIndex).map_err(|_| "annotation unreadable")?;
        annots.delete_annotation(a).map_err(|_| "annotation could not be removed")?;
    }
    // Replacements are inserted right after the original object (keeping drawing order) before it
    // is removed, so the original's font stays alive while the copies are made.
    let mut work: Vec<(usize, u8)> = plan.remove.iter().map(|&i| (i, 0u8)).collect();
    work.extend(plan.split.iter().map(|(i, _)| (*i, 1u8)));
    work.extend(plan.edit_images.iter().map(|&i| (i, 2u8)));
    work.sort_unstable_by_key(|a| std::cmp::Reverse(a.0));
    for (oi, kind) in work {
        let objs = page.objects_mut();
        let obj = objs.get(oi as PdfPageObjectIndex).map_err(|_| "object unreadable")?;
        match kind {
            1 => {
                let kept = &plan.split.iter().find(|(i, _)| *i == oi).ok_or("plan mismatch")?.1;
                let t = obj.as_text_object().ok_or("not text")?;
                let m = t.matrix().map_err(|_| "text matrix unreadable")?;
                let size = t.unscaled_font_size();
                let fill = t.fill_color().ok();
                let stroke = t.stroke_color().ok();
                let mode = t.render_mode();
                let font = t.font();
                // Runs keep words together for search and copy; single characters are exact even
                // when the original used character or word spacing.
                let pieces: Vec<(String, f32, f32)> = match text_mode {
                    TextMode::Runs => runs(kept),
                    TextMode::Chars => kept.iter().map(|k| (k.ch.to_string(), k.x, k.y)).collect(),
                };
                for (at, (piece, x, y)) in (oi + 1..).zip(pieces) {
                    let mut c = PdfPageTextObject::new(doc, piece, &font, size).map_err(|_| "glyph not available")?;
                    let mut cm = m;
                    cm.set_e(x);
                    cm.set_f(y);
                    c.apply_matrix(cm).map_err(|_| "text placement failed")?;
                    if let Some(f) = fill {
                        c.set_fill_color(f).map_err(|_| "text color failed")?;
                    }
                    if let Some(s) = stroke {
                        c.set_stroke_color(s).map_err(|_| "text color failed")?;
                    }
                    c.set_render_mode(mode).map_err(|_| "text mode failed")?;
                    objs.insert_object_at_index(at as PdfPageObjectIndex, PdfPageObject::Text(c))
                        .map_err(|_| "text insert failed")?;
                }
            }
            2 => {
                let img_obj = obj.as_image_object().ok_or("not an image")?;
                let raw = img_obj.get_raw_image().map_err(|_| "image could not be decoded")?;
                let data = img_obj.get_raw_image_data().map_err(|_| "image data unreadable")?;
                let m = img_obj.matrix().map_err(|_| "image matrix unreadable")?;
                let mut px = raw.to_rgba8();
                let (w, h) = (px.width() as f32, px.height() as f32);
                for y in 0..px.height() {
                    for x in 0..px.width() {
                        let f = pixel_footprint(&m, w, h, x, y);
                        if plan.boxes.iter().any(|b| b.overlap(&f) > 1e-3 * f.area().max(1e-6)) {
                            px.put_pixel(x, y, Rgba([0, 0, 0, 255]));
                        }
                    }
                }
                let mut fresh = PdfPageImageObject::new(doc, &DynamicImage::ImageRgba8(px))
                    .map_err(|_| "image could not be replaced")?;
                fresh.apply_matrix(m).map_err(|_| "image placement failed")?;
                objs.insert_object_at_index((oi + 1) as PdfPageObjectIndex, PdfPageObject::Image(fresh))
                    .map_err(|_| "image insert failed")?;
                dropped_images.push(data);
            }
            _ => {
                if let Some(img) = obj.as_image_object() {
                    if let Ok(data) = img.get_raw_image_data() {
                        dropped_images.push(data);
                    }
                }
            }
        }
        objs.remove_object_at_index(oi as PdfPageObjectIndex).map_err(|_| "object could not be removed")?;
    }
    for b in &plan.boxes {
        page.objects_mut()
            .create_path_object_rect(b.to_pdf(), None, None, Some(PdfColor::new(0, 0, 0, 255)))
            .map_err(|_| "box could not be drawn")?;
    }
    crate::edit::regenerate_keeping_colours(page).map_err(|_| "page content could not be written")?;
    Ok(dropped_images)
}

/// Build the output: redacted working pages, and image-only pages for `raster` pages rendered from
/// `source` (original content, areas painted black).
fn assemble<'p>(
    pdfium: &'p Pdfium,
    work: &PdfDocument,
    source: &PdfDocument,
    plans: &BTreeMap<u32, PagePlan>,
    raster: &HashSet<u32>,
    count: u32,
) -> PwResult<PdfDocument<'p>> {
    let mut out = pdfium.create_new_pdf().map_err(|e| map_pdfium_error(&e, "create"))?;
    let mut i = 0u32;
    while i < count {
        if raster.contains(&i) {
            let src = source.pages().get(i as PdfPageIndex).map_err(|e| map_pdfium_error(&e, "page"))?;
            let rot = rotation_degrees(&src);
            let bbox = displayed_box(&src);
            let (dw, dh) = if matches!(rot, 90 | 270) {
                (bbox.t - bbox.b, bbox.r - bbox.l)
            } else {
                (bbox.r - bbox.l, bbox.t - bbox.b)
            };
            let mut scale = RASTER_DPI / 72.0;
            while dw * dh * scale * scale > MAX_RASTER_PIXELS && scale > 0.5 {
                scale *= 0.8;
            }
            let mut img = render(&src, scale, true)?;
            if let Some(plan) = plans.get(&i) {
                paint_boxes(&mut img, &plan.boxes, &bbox, rot, scale, 1.0);
            }
            let mut page = out
                .pages_mut()
                .create_page_at_end(PdfPagePaperSize::Custom(PdfPoints::new(dw), PdfPoints::new(dh)))
                .map_err(|e| map_pdfium_error(&e, "create"))?;
            let obj = PdfPageImageObject::new_with_size(
                &out,
                &DynamicImage::ImageRgba8(img),
                PdfPoints::new(dw),
                PdfPoints::new(dh),
            )
            .map_err(|e| map_pdfium_error(&e, "create"))?;
            page.objects_mut().add_image_object(obj).map_err(|e| map_pdfium_error(&e, "create"))?;
            i += 1;
            continue;
        }
        let start = i;
        while i + 1 < count && !raster.contains(&(i + 1)) {
            i += 1;
        }
        let dest = out.pages().len();
        out.pages_mut()
            .copy_page_range_from_document(work, (start as PdfPageIndex)..=(i as PdfPageIndex), dest)
            .map_err(|e| map_pdfium_error(&e, "copy"))?;
        i += 1;
    }
    Ok(out)
}

/// Permanently remove the content under `areas` and write a verified new file.
pub fn redact(
    input: &Path,
    opts: &OpenOptions,
    areas: &[RedactArea],
    output: &Path,
    policy: CollisionPolicy,
    cancelled: &dyn Fn() -> bool,
) -> PwResult<EditOutcome> {
    let pdfium = engine()?;
    let source = load(&pdfium, input, opts)?;
    let count = source.pages().len().max(0) as u32;
    let by_page = page_areas(areas, count)?;
    let src_encrypted =
        !matches!(source.permissions().security_handler_revision(), Ok(PdfSecurityHandlerRevision::Unprotected));

    let mut plans: BTreeMap<u32, PagePlan> = BTreeMap::new();
    let mut raster: HashSet<u32> = HashSet::new();
    for (&p, rects) in &by_page {
        let page = source.pages().get(p as PdfPageIndex).map_err(|e| map_pdfium_error(&e, "page"))?;
        let plan = plan_page(&page, rects)?;
        if plan.raster.is_some() {
            raster.insert(p);
        }
        plans.insert(p, plan);
    }

    // Up to three builds: a page that fails the per-page check is retried with single-character
    // text, then replaced by an image.
    let mut chars_mode: HashSet<u32> = HashSet::new();
    let mut built = None;
    for _attempt in 0..3 {
        if cancelled() {
            return Err(PwError::new(ErrorCategory::Cancelled, "job.cancelled", "Redaction was cancelled."));
        }
        let work = load(&pdfium, input, opts)?;
        let mut dropped = Vec::new();
        for (&p, plan) in &plans {
            if raster.contains(&p) {
                continue;
            }
            let mut page = work.pages().get(p as PdfPageIndex).map_err(|e| map_pdfium_error(&e, "page"))?;
            let mode = if chars_mode.contains(&p) { TextMode::Chars } else { TextMode::Runs };
            match apply_plan(&work, &mut page, plan, mode) {
                Ok(d) => dropped.extend(d),
                Err(_) => {
                    raster.insert(p);
                }
            }
        }
        // Images on image-only pages are gone with the page's original content.
        for &p in &raster {
            let page = source.pages().get(p as PdfPageIndex).map_err(|e| map_pdfium_error(&e, "page"))?;
            for obj in page.objects().iter() {
                if let Some(img) = obj.as_image_object() {
                    if let Ok(d) = img.get_raw_image_data() {
                        dropped.push(d);
                    }
                }
            }
        }
        let out = assemble(&pdfium, &work, &source, &plans, &raster, count)?;
        let bytes = sanitize(&save_bytes(&out)?)?;
        let failed = check_pages(&pdfium, &bytes, &source, &plans, &raster)?;
        if failed.is_empty() {
            built = Some((bytes, dropped));
            break;
        }
        for p in failed {
            let split = plans.get(&p).is_some_and(|pl| !pl.split.is_empty());
            if split && chars_mode.insert(p) {
                continue;
            }
            raster.insert(p);
        }
    }
    let Some((bytes, dropped)) = built else {
        return Err(verify_failed("the redacted pages could not be checked"));
    };
    // Image data that must be gone: what was removed or replaced, unless the same image is still
    // shown somewhere the user did not redact (an image shared with another page).
    let mut dropped: HashSet<[u8; 32]> = dropped.iter().filter(|d| d.len() >= 16).map(|d| digest(d)).collect();
    if !dropped.is_empty() {
        for p in 0..count {
            if raster.contains(&p) {
                continue;
            }
            let plan = plans.get(&p);
            let page = source.pages().get(p as PdfPageIndex).map_err(|e| map_pdfium_error(&e, "page"))?;
            for (oi, obj) in page.objects().iter().enumerate() {
                if plan.is_some_and(|pl| pl.remove.contains(&oi) || pl.edit_images.contains(&oi)) {
                    continue;
                }
                if let Some(Ok(d)) = obj.as_image_object().map(|i| i.get_raw_image_data()) {
                    dropped.remove(&digest(&d));
                }
            }
        }
    }
    let protected = [input.to_path_buf()];
    let c = write_atomic_bytes(output, policy, &protected, &bytes, |staged| {
        let back = std::fs::read(staged).map_err(|e| PwError::from_io(&e, "output file"))?;
        verify_file(&back, &dropped)?;
        let failed = check_pages(&pdfium, &back, &source, &plans, &raster)?;
        if failed.is_empty() {
            Ok(())
        } else {
            Err(verify_failed("content remains under a redaction area"))
        }
    })?;
    let mut warnings = Vec::new();
    if !raster.is_empty() {
        let mut pages: Vec<u32> = raster.iter().map(|p| p + 1).collect();
        pages.sort_unstable();
        let list = pages.iter().map(u32::to_string).collect::<Vec<_>>().join(", ");
        warnings.push(format!(
            "Page{} {list} {} replaced by an image so the redaction is complete; text there can no longer be selected or searched.",
            if pages.len() == 1 { "" } else { "s" },
            if pages.len() == 1 { "was" } else { "were" },
        ));
    }
    warnings.push(
        "Bookmarks, document properties, attachments and fillable form fields are not carried over to the redacted copy."
            .into(),
    );
    if src_encrypted {
        warnings.push(PROTECTION_REMOVED.into());
    }
    Ok((c, count, warnings))
}

fn digest(data: &[u8]) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    Sha256::digest(data).into()
}

fn verify_failed(what: &str) -> PwError {
    PwError::new(
        ErrorCategory::ValidationFailed,
        "redact.verify",
        format!("The redacted copy failed verification ({what}) and was not saved."),
    )
}

/// Check each redacted page of `bytes`: nothing under the areas, and (for precisely redacted pages)
/// identical rendering outside the boxes. Returns pages that fail.
fn check_pages(
    pdfium: &Pdfium,
    bytes: &[u8],
    source: &PdfDocument,
    plans: &BTreeMap<u32, PagePlan>,
    raster: &HashSet<u32>,
) -> PwResult<Vec<u32>> {
    let out = pdfium.load_pdf_from_byte_slice(bytes, None).map_err(|_| verify_failed("could not be reopened"))?;
    if out.pages().len().max(0) as usize != source.pages().len().max(0) as usize {
        return Err(verify_failed("page count mismatch"));
    }
    let mut failed = Vec::new();
    for (&p, plan) in plans {
        let page = out.pages().get(p as PdfPageIndex).map_err(|_| verify_failed("page unreadable"))?;
        if raster.contains(&p) {
            let text = page.text().map_err(|_| verify_failed("text unreadable"))?;
            let objects: Vec<_> = page.objects().iter().collect();
            let ok = text.chars().is_empty()
                && page.annotations().is_empty()
                && objects.len() == 1
                && objects[0].object_type() == PdfPageObjectType::Image;
            if !ok {
                return Err(verify_failed("an image page holds other content"));
            }
            continue;
        }
        if !page_is_clear(&page, plan)? {
            failed.push(p);
            continue;
        }
        let src = source.pages().get(p as PdfPageIndex).map_err(|_| verify_failed("page unreadable"))?;
        let rot = rotation_degrees(&src);
        let bbox = displayed_box(&src);
        let mut expected = render(&src, CHECK_SCALE, false)?;
        paint_boxes(&mut expected, &plan.boxes, &bbox, rot, CHECK_SCALE, 0.0);
        let got = render(&page, CHECK_SCALE, false)?;
        let mut mask = RgbaImage::new(expected.width(), expected.height());
        paint_boxes(&mut mask, &plan.boxes, &bbox, rot, CHECK_SCALE, 2.0);
        match changed_outside(&expected, &got, &mask) {
            Some((changed, total)) if (changed as f64) <= MAX_CHANGED_SHARE * total as f64 => {}
            _ => failed.push(p),
        }
    }
    Ok(failed)
}

/// No character, annotation, or unblackened image pixel under any area of the page.
fn page_is_clear(page: &PdfPage, plan: &PagePlan) -> PwResult<bool> {
    let text = page.text().map_err(|_| verify_failed("text unreadable"))?;
    for ch in text.chars().iter() {
        if ch.is_generated().unwrap_or(false) {
            continue;
        }
        let glyph = ch.tight_bounds().map(|b| R::from_pdf(&b));
        let origin = ch.origin().map(|(x, y)| (x.value, y.value));
        let (Ok(glyph), Ok(origin)) = (glyph, origin) else { return Ok(false) };
        if plan.areas.iter().any(|a| char_hit(&glyph, origin, a)) {
            return Ok(false);
        }
        if let Some(c) = ch.unicode_char() {
            let back = plan
                .removed_chars
                .iter()
                .any(|&(rc, x, y)| rc == c && (x - origin.0).abs() < 0.75 && (y - origin.1).abs() < 0.75);
            if back {
                return Ok(false);
            }
        }
    }
    for annot in page.annotations().iter() {
        match annot.bounds() {
            Ok(b) if !plan.boxes.iter().any(|x| x.touches(&R::from_pdf(&b))) => {}
            _ => return Ok(false),
        }
    }
    for obj in page.objects().iter() {
        let Some(img) = obj.as_image_object() else { continue };
        let Ok(q) = obj.bounds() else { return Ok(false) };
        let bb = quad_bounds(&q);
        if !plan.areas.iter().any(|a| a.touches(&bb)) {
            continue;
        }
        let (Ok(raw), Ok(m)) = (img.get_raw_image(), img.matrix()) else { return Ok(false) };
        let px = raw.to_rgba8();
        let (w, h) = (px.width() as f32, px.height() as f32);
        for (x, y, p) in px.enumerate_pixels() {
            let u = (x as f32 + 0.5) / w;
            let v = 1.0 - (y as f32 + 0.5) / h;
            let (sx, sy) = (m.a() * u + m.c() * v + m.e(), m.b() * u + m.d() * v + m.f());
            if plan.areas.iter().any(|a| a.contains_point(sx, sy)) && p.0[..3].iter().any(|&c| c > 16) {
                return Ok(false);
            }
        }
    }
    Ok(true)
}

/// Rewrite the assembled file: drop unused XObjects, thumbnails, page-level metadata and actions,
/// annotation references to removed annotations, form field parents, and links to pages that are
/// not in the document. A full rewrite also guarantees there is no earlier revision in the file.
fn sanitize(bytes: &[u8]) -> PwResult<Vec<u8>> {
    use lopdf::Dictionary;
    let bad = |what: &str| verify_failed(what);
    let mut doc = lopdf::Document::load_mem(bytes).map_err(|_| bad("could not be re-read for cleaning"))?;
    let pages = doc.get_pages();
    let page_ids: HashSet<lopdf::ObjectId> = pages.values().copied().collect();
    for &page_id in pages.values() {
        // Names of XObjects the page's content uses.
        let content = doc.get_page_content(page_id);
        let ops = lopdf::content::Content::decode(&content).map_err(|_| bad("page content unreadable"))?;
        let used: HashSet<Vec<u8>> = ops
            .operations
            .iter()
            .filter(|op| op.operator == "Do")
            .filter_map(|op| op.operands.first().and_then(|o| o.as_name().ok()).map(<[u8]>::to_vec))
            .collect();
        // Resources: make a page-own copy, keep only used XObjects.
        let res = {
            let page = doc.get_dictionary(page_id).map_err(|_| bad("page unreadable"))?;
            match page.get(b"Resources") {
                Ok(Object::Reference(id)) => doc.get_dictionary(*id).cloned().unwrap_or_default(),
                Ok(Object::Dictionary(d)) => d.clone(),
                _ => Dictionary::new(),
            }
        };
        let mut res = res;
        let xobjects = match res.get(b"XObject") {
            Ok(Object::Reference(id)) => doc.get_dictionary(*id).cloned().ok(),
            Ok(Object::Dictionary(d)) => Some(d.clone()),
            _ => None,
        };
        if let Some(xo) = xobjects {
            let mut kept = Dictionary::new();
            for (name, v) in xo.iter() {
                if used.contains(name) {
                    kept.set(name.clone(), v.clone());
                }
            }
            res.set("XObject", Object::Dictionary(kept));
        }
        // Annotations: kept list, then clean references.
        let annot_ids: Vec<lopdf::ObjectId> = {
            let page = doc.get_dictionary(page_id).map_err(|_| bad("page unreadable"))?;
            let arr = match page.get(b"Annots") {
                Ok(Object::Reference(id)) => {
                    doc.get_object(*id).and_then(|o| o.as_array().cloned()).unwrap_or_default()
                }
                Ok(Object::Array(a)) => a.clone(),
                _ => Vec::new(),
            };
            arr.iter().filter_map(|o| o.as_reference().ok()).collect()
        };
        let present: HashSet<lopdf::ObjectId> = annot_ids.iter().copied().collect();
        let mut keep_annots = Vec::new();
        for id in annot_ids {
            let Ok(d) = doc.get_dictionary(id) else { continue };
            let subtype = d.get(b"Subtype").and_then(|s| s.as_name()).unwrap_or(b"");
            let parent = d.get(b"Parent").and_then(|o| o.as_reference()).ok();
            if subtype == b"Popup" && parent.is_none_or(|p| !present.contains(&p)) {
                continue;
            }
            let mut d = d.clone();
            d.remove(b"AA");
            if subtype == b"Widget" {
                // Fields (names, values, options, actions) are not carried over; the widget's
                // appearance stays as drawn.
                for key in [
                    &b"Parent"[..],
                    b"A",
                    b"FT",
                    b"T",
                    b"TU",
                    b"TM",
                    b"V",
                    b"DV",
                    b"Ff",
                    b"Opt",
                    b"TI",
                    b"I",
                    b"MaxLen",
                    b"DA",
                    b"DS",
                    b"RV",
                    b"Q",
                    b"Kids",
                ] {
                    d.remove(key);
                }
            }
            for key in [b"IRT".as_slice(), b"Popup".as_slice()] {
                if d.get(key).and_then(|o| o.as_reference()).is_ok_and(|r| !present.contains(&r)) {
                    d.remove(key);
                }
            }
            let dest_ok = |o: &Object| match o {
                Object::Array(a) => a.first().and_then(|f| f.as_reference().ok()).is_none_or(|r| page_ids.contains(&r)),
                _ => true,
            };
            if d.get(b"Dest").is_ok_and(|o| !dest_ok(o)) {
                d.remove(b"Dest");
            }
            let action = match d.get(b"A") {
                Ok(Object::Reference(r)) => doc.get_dictionary(*r).cloned().ok(),
                Ok(Object::Dictionary(a)) => Some(a.clone()),
                _ => None,
            };
            if let Some(a) = action {
                let s = a.get(b"S").and_then(|s| s.as_name()).unwrap_or(b"");
                let safe = match s {
                    b"URI" => true,
                    b"GoTo" => a.get(b"D").map(dest_ok).unwrap_or(true),
                    _ => false,
                };
                if safe {
                    d.set("A", Object::Dictionary(a));
                } else {
                    d.remove(b"A");
                }
            }
            doc.objects.insert(id, Object::Dictionary(d));
            keep_annots.push(Object::Reference(id));
        }
        let page = doc.get_dictionary_mut(page_id).map_err(|_| bad("page unreadable"))?;
        for key in [&b"Thumb"[..], b"PieceInfo", b"Metadata", b"AA", b"B", b"SeparationInfo"] {
            page.remove(key);
        }
        page.set("Resources", Object::Dictionary(res));
        if keep_annots.is_empty() {
            page.remove(b"Annots");
        } else {
            page.set("Annots", Object::Array(keep_annots));
        }
    }
    if let Ok(root_id) = doc.trailer.get(b"Root").and_then(|o| o.as_reference()) {
        if let Ok(root) = doc.get_dictionary_mut(root_id) {
            for key in [&b"Metadata"[..], b"Names", b"OpenAction", b"AA", b"AcroForm", b"Outlines", b"StructTreeRoot"] {
                root.remove(key);
            }
        }
    }
    doc.trailer.remove(b"Info");
    doc.trailer.remove(b"Prev");
    doc.prune_objects();
    let mut out = Vec::new();
    doc.save_to(&mut out).map_err(|_| bad("could not be written"))?;
    Ok(out)
}

/// Whole-file checks independent of PDFium: one revision, no encryption, every page object in the
/// page tree, and none of the removed image data present.
fn verify_file(bytes: &[u8], dropped_images: &HashSet<[u8; 32]>) -> PwResult<()> {
    let doc = lopdf::Document::load_mem(bytes).map_err(|_| verify_failed("could not be re-read"))?;
    if doc.trailer.get(b"Prev").is_ok() || doc.trailer.get(b"Encrypt").is_ok() {
        return Err(verify_failed("earlier revision present"));
    }
    let tree: HashSet<lopdf::ObjectId> = doc.get_pages().values().copied().collect();
    for (id, obj) in &doc.objects {
        match obj {
            Object::Dictionary(d) if d.get(b"Type").and_then(|t| t.as_name()).is_ok_and(|t| t == b"Page") => {
                if !tree.contains(id) {
                    return Err(verify_failed("a page outside the document remains"));
                }
            }
            Object::Stream(s) if s.content.len() >= 16 && dropped_images.contains(&digest(&s.content)) => {
                return Err(verify_failed("removed image data remains"));
            }
            _ => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(n: &str) -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/pdf").join(n)
    }

    /// The final checks must reject output that still holds removed content (not only pass good output).
    #[test]
    fn file_check_rejects_removed_image_data_revisions_and_stray_pages() {
        let bytes = std::fs::read(fixture("redaction.pdf")).expect("fixture");
        verify_file(&bytes, &HashSet::new()).expect("clean fixture passes");
        let im1: Vec<u8> = (0..192u32).map(|i| ((i * 7 + 13) % 256) as u8).collect();
        let err = verify_file(&bytes, &HashSet::from([digest(&im1)])).expect_err("must be rejected");
        assert!(err.message.contains("removed image data"), "{}", err.message);

        let mut doc = lopdf::Document::load_mem(&bytes).expect("lopdf");
        doc.trailer.set("Prev", 9);
        let mut with_prev = Vec::new();
        doc.save_to(&mut with_prev).expect("save");
        assert!(verify_file(&with_prev, &HashSet::new())
            .expect_err("must be rejected")
            .message
            .contains("earlier revision"));

        let mut doc = lopdf::Document::load_mem(&bytes).expect("lopdf");
        let mut stray = lopdf::Dictionary::new();
        stray.set("Type", Object::Name(b"Page".to_vec()));
        doc.add_object(Object::Dictionary(stray));
        let mut with_stray = Vec::new();
        doc.save_to(&mut with_stray).expect("save");
        assert!(verify_file(&with_stray, &HashSet::new())
            .expect_err("must be rejected")
            .message
            .contains("outside the document"));
    }

    #[test]
    fn page_check_rejects_text_annotations_and_pixels_left_under_an_area() {
        if crate::init(None).is_err() {
            return; // engine not installed
        }
        let pdfium = engine().expect("engine");
        let doc = load(&pdfium, &fixture("redaction.pdf"), &OpenOptions { password: None }).expect("load");
        let page = doc.pages().get(0).expect("page");
        let unchanged = |a: R| {
            let plan = plan_page(&page, &[a]).expect("plan");
            page_is_clear(&page, &plan).expect("check")
        };
        assert!(!unchanged(R { l: 60.0, b: 600.0, r: 400.0, t: 720.0 }), "text still under the area");
        assert!(!unchanged(R { l: 478.0, b: 518.0, r: 502.0, t: 542.0 }), "note still under the area");
        assert!(!unchanged(R { l: 320.0, b: 420.0, r: 340.0, t: 440.0 }), "image pixels still under the area");
        assert!(unchanged(R { l: 520.0, b: 600.0, r: 560.0, t: 640.0 }), "nothing there: clear");
    }

    #[test]
    fn char_hit_needs_a_real_share_of_the_glyph() {
        let area = R { l: 0.0, b: 0.0, r: 10.0, t: 10.0 };
        let inside = R { l: 1.0, b: 1.0, r: 5.0, t: 9.0 };
        let sliver = R { l: 9.0, b: 1.0, r: 19.0, t: 9.0 };
        let half = R { l: 5.0, b: 1.0, r: 15.0, t: 9.0 };
        assert!(char_hit(&inside, (1.0, 1.0), &area));
        assert!(!char_hit(&sliver, (9.0, 1.0), &area), "10% under the area stays");
        assert!(char_hit(&half, (5.0, 1.0), &area), "50% under the area goes");
        let blank = R { l: 3.0, b: 3.0, r: 3.0, t: 3.0 };
        assert!(char_hit(&blank, (3.0, 3.0), &area), "blank glyph decided by its origin");
        assert!(!char_hit(&blank, (30.0, 3.0), &area));
    }

    #[test]
    fn boxes_map_to_pixels_for_every_rotation() {
        // Page 100 x 200 pt with a box at the bottom-left corner, 10 x 20 pt.
        let bbox = R { l: 0.0, b: 0.0, r: 100.0, t: 200.0 };
        let b = R { l: 0.0, b: 0.0, r: 10.0, t: 20.0 };
        assert_eq!(box_to_px(&b, &bbox, 0, 1.0), (0.0, 180.0, 10.0, 200.0));
        // 90° clockwise: bottom-left corner moves to the top-left.
        assert_eq!(box_to_px(&b, &bbox, 90, 1.0), (0.0, 0.0, 20.0, 10.0));
        assert_eq!(box_to_px(&b, &bbox, 180, 1.0), (90.0, 0.0, 100.0, 20.0));
        assert_eq!(box_to_px(&b, &bbox, 270, 1.0), (180.0, 90.0, 200.0, 100.0));
        // Offset page box.
        let off = R { l: 50.0, b: 50.0, r: 150.0, t: 250.0 };
        let ob = R { l: 50.0, b: 50.0, r: 60.0, t: 70.0 };
        assert_eq!(box_to_px(&ob, &off, 0, 2.0), (0.0, 360.0, 20.0, 400.0));
    }
}
