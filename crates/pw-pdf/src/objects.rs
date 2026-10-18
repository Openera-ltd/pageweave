//! Edit existing text and images (PRD 01 C, PRD 05 "edit text/style/position and image object only
//! where source/library supports it"). The engine lists a page's editable objects — text grouped
//! into runs (one line drawn with one font), and images — and applies a batch of edits to a copy.
//!
//! Fonts: a run keeps its font when every new character is already drawn with that font on the
//! page (an embedded subset font holds at least those glyphs), or when the font is a standard,
//! non-embedded one and the text is Latin-1. Otherwise the text is set in the closest standard font
//! and a warning says so — the edit never silently produces missing glyphs. Text inside form
//! XObjects, rotated or shaped text, and fonts without a character map are listed as not editable.

use crate::document::{load, OpenOptions};
use crate::edit::{protection_warnings, rotation_of, save_bytes, source_encrypted, verify_output, EditOutcome, Expect};
use crate::{engine, map_pdfium_error};
use pdfium_render::prelude::*;
use pw_core::ops::{PageEdit, PageEdits, RectPt};
use pw_core::output::{write_atomic_bytes, CollisionPolicy};
use pw_core::{ErrorCategory, PwError, PwResult};
use serde::Serialize;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::Path;

/// A line of text drawn with one font: the objects it is made of, in order.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EditableText {
    pub objects: Vec<u32>,
    pub text: String,
    /// Bounds in unrotated page space.
    pub rect: RectPt,
    pub font: String,
    /// Size as drawn (points).
    pub size: f32,
    pub color: String,
    pub embedded: bool,
    pub bold: bool,
    pub italic: bool,
    /// Why it cannot be edited, when it cannot.
    pub locked: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EditableImage {
    pub object: u32,
    pub rect: RectPt,
    pub width_px: u32,
    pub height_px: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EditableObjects {
    pub page: u32,
    /// Displayed rotation of the page (clockwise degrees).
    pub rotation: u32,
    pub texts: Vec<EditableText>,
    pub images: Vec<EditableImage>,
}

fn err(e: PdfiumError) -> PwError {
    map_pdfium_error(&e, "edit")
}

fn rect_of(o: &PdfPageObject) -> Option<RectPt> {
    let q = o.bounds().ok()?;
    let r = q.to_rect();
    Some(RectPt { left: r.left().value, bottom: r.bottom().value, right: r.right().value, top: r.top().value })
}

fn hex(c: PdfColor) -> String {
    format!("#{:02x}{:02x}{:02x}", c.red(), c.green(), c.blue())
}

/// Is the text object drawn upright (no rotation or shear in its matrix)?
fn upright(t: &PdfPageTextObject) -> bool {
    t.matrix().map(|m| m.b().abs() < 1e-3 && m.c().abs() < 1e-3 && m.a() > 0.0 && m.d() > 0.0).unwrap_or(false)
}

/// Font identity for grouping and glyph coverage.
fn font_key(t: &PdfPageTextObject) -> String {
    t.font().name()
}

/// List the editable objects of page `page` (0-based).
pub fn editable_objects(input: &Path, opts: &OpenOptions, page_index: u32) -> PwResult<EditableObjects> {
    let pdfium = engine()?;
    let doc = load(&pdfium, input, opts)?;
    list_page(&doc, page_index)
}

fn list_page(doc: &PdfDocument, page_index: u32) -> PwResult<EditableObjects> {
    let page = doc
        .pages()
        .get(page_index as PdfPageIndex)
        .map_err(|_| PwError::invalid("edit.page", "That page does not exist."))?;
    let mut texts: Vec<EditableText> = Vec::new();
    let mut images = Vec::new();
    // Consecutive upright text objects with the same font, size and baseline join one run.
    let mut current: Option<(EditableText, f32, f32)> = None; // (run, right edge, baseline y)
    let flush = |cur: &mut Option<(EditableText, f32, f32)>, texts: &mut Vec<EditableText>| {
        if let Some((t, _, _)) = cur.take() {
            texts.push(t);
        }
    };
    for (i, o) in page.objects().iter().enumerate() {
        let i = i as u32;
        match &o {
            PdfPageObject::Text(t) => {
                let Some(r) = rect_of(&o) else { continue };
                let text = t.text();
                if text.trim().is_empty() && current.is_none() {
                    continue;
                }
                let font = t.font();
                let size = t.scaled_font_size().value;
                let key = font_key(t);
                let embedded = font.is_embedded().unwrap_or(false);
                let locked = if !upright(t) {
                    Some("Turned or slanted text cannot be edited yet.".to_string())
                } else if text.contains('\u{fffd}') || (text.trim().is_empty() && !t.text().is_empty()) {
                    Some("This text's font has no character map, so its letters are unknown.".to_string())
                } else if t.render_mode() == PdfPageTextRenderMode::Invisible {
                    Some("Invisible text (such as an OCR layer) is not edited here.".to_string())
                } else {
                    None
                };
                // The baseline is the text matrix's translation (ink bounds vary with descenders).
                let baseline = t.matrix().map(|m| m.f()).unwrap_or(r.bottom);
                let joins = current.as_ref().is_some_and(|(run, right, base)| {
                    run.locked.is_none()
                        && locked.is_none()
                        && run.font == key
                        && (run.size - size).abs() < 0.05
                        && (base - baseline).abs() < size * 0.05
                        && r.left >= *right - size * 0.2
                        && r.left - *right < size * 0.6
                });
                if joins {
                    let (run, right, _) = current.as_mut().expect("joined run");
                    let gap = r.left - *right;
                    if gap > size * 0.15 && !run.text.ends_with(' ') && !text.starts_with(' ') {
                        run.text.push(' ');
                    }
                    run.text.push_str(&text);
                    run.objects.push(i);
                    run.rect = RectPt {
                        left: run.rect.left.min(r.left),
                        bottom: run.rect.bottom.min(r.bottom),
                        right: run.rect.right.max(r.right),
                        top: run.rect.top.max(r.top),
                    };
                    *right = r.right;
                } else {
                    flush(&mut current, &mut texts);
                    let color = t.fill_color().map(hex).unwrap_or_else(|_| "#000000".into());
                    current = Some((
                        EditableText {
                            objects: vec![i],
                            text,
                            rect: r,
                            font: key,
                            size,
                            color,
                            embedded,
                            bold: font
                                .weight()
                                .map(|w| {
                                    matches!(
                                        w,
                                        PdfFontWeight::Weight700Bold
                                            | PdfFontWeight::Weight800
                                            | PdfFontWeight::Weight900
                                    )
                                })
                                .unwrap_or(false)
                                || font.is_bold_reenforced(),
                            italic: font.is_italic(),
                            locked,
                        },
                        r.right,
                        baseline,
                    ));
                }
            }
            PdfPageObject::Image(img) => {
                flush(&mut current, &mut texts);
                if let Some(r) = rect_of(&o) {
                    let (w, h) = (img.width().unwrap_or(0).max(0) as u32, img.height().unwrap_or(0).max(0) as u32);
                    images.push(EditableImage { object: i, rect: r, width_px: w, height_px: h });
                }
            }
            _ => flush(&mut current, &mut texts),
        }
    }
    flush(&mut current, &mut texts);
    texts.retain(|t| !t.text.trim().is_empty());
    Ok(EditableObjects { page: page_index, rotation: rotation_of(&page), texts, images })
}

fn color(hexs: &str) -> PwResult<PdfColor> {
    let v = u32::from_str_radix(hexs.trim_start_matches('#'), 16)
        .map_err(|_| PwError::invalid("edit.color", "Colors must look like #rrggbb."))?;
    Ok(PdfColor::new((v >> 16) as u8, (v >> 8) as u8, v as u8, 255))
}

fn load_picture(path: &Path) -> PwResult<image::DynamicImage> {
    let bytes = std::fs::read(path).map_err(|e| PwError::from_io(&e, "picture"))?;
    if bytes.len() > 50 * 1024 * 1024 {
        return Err(PwError::invalid("edit.image_large", "The picture is larger than 50 MB."));
    }
    let fmt =
        image::guess_format(&bytes).map_err(|_| PwError::invalid("edit.image_format", "Use a PNG or JPEG picture."))?;
    if !matches!(fmt, image::ImageFormat::Png | image::ImageFormat::Jpeg) {
        return Err(PwError::invalid("edit.image_format", "Use a PNG or JPEG picture."));
    }
    let mut reader = image::ImageReader::with_format(std::io::Cursor::new(&bytes), fmt);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(20_000);
    limits.max_image_height = Some(20_000);
    limits.max_alloc = Some(512 * 1024 * 1024);
    reader.limits(limits);
    let img =
        reader.decode().map_err(|_| PwError::invalid("edit.image_unreadable", "The picture could not be read."))?;
    Ok(if img.width().max(img.height()) > 3000 {
        img.resize(3000, 3000, image::imageops::FilterType::Lanczos3)
    } else {
        img
    })
}

/// What a verified output must show.
struct Expectation {
    page: u32,
    texts: Vec<String>,
    images: usize,
}

/// Apply `pages` of edits to a copy of `input`.
pub fn edit_objects(
    input: &Path,
    opts: &OpenOptions,
    pages: &[PageEdits],
    output: &Path,
    policy: CollisionPolicy,
) -> PwResult<EditOutcome> {
    let edited = apply_object_edits(input, opts, pages)?;
    let pdfium = engine()?;
    let mut encrypted = false;
    let c = write_atomic_bytes(output, policy, &[input.to_path_buf()], &edited.bytes, |staged| {
        encrypted = verify_output(
            &pdfium,
            staged,
            &Expect { page_count: edited.page_count, password: opts.password, rotations: None },
        )?;
        Ok(())
    })?;
    let mut warnings = protection_warnings(edited.source_encrypted, encrypted);
    warnings.extend(edited.warnings);
    Ok((c, edited.page_count, warnings))
}

/// A document with object edits applied, saved to bytes and already verified.
pub(crate) struct EditedBytes {
    pub bytes: Vec<u8>,
    pub page_count: u32,
    pub source_encrypted: bool,
    pub warnings: Vec<String>,
}

/// Apply object edits in memory; the saved bytes are reopened and checked before they are returned.
pub(crate) fn apply_object_edits(input: &Path, opts: &OpenOptions, pages: &[PageEdits]) -> PwResult<EditedBytes> {
    if pages.is_empty() || pages.iter().all(|p| p.edits.is_empty()) {
        return Err(PwError::invalid("edit.nothing", "There are no changes to save."));
    }
    let pdfium = engine()?;
    let mut doc = load(&pdfium, input, opts)?;
    // Listings of the source, to validate object indices and know what each run holds.
    let mut listings: HashMap<u32, EditableObjects> = HashMap::new();
    for p in pages {
        listings.insert(p.page, list_page(&doc, p.page)?);
    }
    let total = doc.pages().len().max(0) as u32;
    let signatures = doc.signatures().len();
    let mut substituted = 0usize;
    let mut expectations = Vec::new();
    for pe in pages {
        let listing = &listings[&pe.page];
        let text_of = |objs: &[u32]| listing.texts.iter().find(|t| t.objects == objs);
        let image_ids: HashSet<u32> = listing.images.iter().map(|i| i.object).collect();
        // Characters each font already draws on this page.
        let mut coverage: HashMap<String, HashSet<char>> = HashMap::new();
        for t in &listing.texts {
            coverage.entry(t.font.clone()).or_default().extend(t.text.chars());
        }
        // Pre-load pictures and fonts (need &mut doc) before borrowing the page.
        let mut pictures: HashMap<usize, image::DynamicImage> = HashMap::new();
        for (k, e) in pe.edits.iter().enumerate() {
            if let PageEdit::ReplaceImage { path, .. } = e {
                pictures.insert(k, load_picture(path)?);
            }
        }
        let rot = {
            let page = doc.pages().get(pe.page as PdfPageIndex).map_err(err)?;
            rotation_of(&page)
        };
        // Moves first: a picture that is also replaced or turned takes its (moved) placement from
        // the original object, which those edits then remove.
        let mut ordered: Vec<(usize, &PageEdit)> = pe.edits.iter().enumerate().collect();
        ordered.sort_by_key(|(_, e)| !matches!(e, PageEdit::MoveObjects { .. }));
        let mut remove: BTreeSet<u32> = BTreeSet::new();
        let mut expect_texts = Vec::new();
        let mut images_removed = 0usize;
        let mut images_added = 0usize;
        for (k, e) in ordered {
            match e {
                PageEdit::Text { objects, text, size, color: c, dx, dy } => {
                    let run = text_of(objects).ok_or_else(|| {
                        PwError::invalid(
                            "edit.stale",
                            "The text to change was not found; the document may have changed. Open it again.",
                        )
                    })?;
                    if let Some(why) = &run.locked {
                        return Err(PwError::invalid("edit.locked", why.clone()));
                    }
                    let new_text = text.trim_end_matches(['\r', '\n']).to_string();
                    let primary_index = objects[0];
                    // Font facts (immutable borrow), then decide.
                    let (keep_font, subst) = {
                        let page = doc.pages().get(pe.page as PdfPageIndex).map_err(err)?;
                        let o = page.objects().get(primary_index as PdfPageObjectIndex).map_err(err)?;
                        let t = o
                            .as_text_object()
                            .ok_or_else(|| PwError::invalid("edit.stale", "The text to change was not found."))?;
                        let f = t.font();
                        let covered = coverage.get(&run.font).cloned().unwrap_or_default();
                        let latin1 = new_text.chars().all(|c| (c as u32) < 256);
                        let keep = if run.embedded {
                            new_text.chars().all(|c| c == ' ' || covered.contains(&c))
                        } else {
                            latin1 && (f.is_built_in() || new_text.chars().all(|c| c == ' ' || covered.contains(&c)))
                        };
                        (keep, if keep { None } else { Some(latin1) })
                    };
                    if let Some(latin1) = subst {
                        if !latin1 {
                            return Err(PwError::invalid(
                                "edit.charset",
                                "These characters are not in the text's font, and the replacement fonts cover Latin letters only. Use characters already on the page.",
                            ));
                        }
                    }
                    let token = if keep_font {
                        None
                    } else {
                        // Font facts first (the page borrows the document), then the standard font.
                        let (name, fixed, serif, italic, bold) = {
                            let page = doc.pages().get(pe.page as PdfPageIndex).map_err(err)?;
                            let o = page.objects().get(primary_index as PdfPageObjectIndex).map_err(err)?;
                            let f = o
                                .as_text_object()
                                .map(|t| t.font())
                                .ok_or_else(|| PwError::internal("edit.font", "Font missing."))?;
                            let bold = f
                                .weight()
                                .map(|w| {
                                    matches!(
                                        w,
                                        PdfFontWeight::Weight700Bold
                                            | PdfFontWeight::Weight800
                                            | PdfFontWeight::Weight900
                                    )
                                })
                                .unwrap_or(false);
                            (f.name(), f.is_fixed_pitch(), f.is_serif(), f.is_italic(), bold)
                        };
                        Some(pick_standard(&mut doc, fixed, serif, italic, bold, &name))
                    };
                    let mut page = doc.pages().get(pe.page as PdfPageIndex).map_err(err)?;
                    let (tx, ty) = unrotate(rot, *dx, *dy);
                    match token {
                        None => {
                            let mut o = page.objects().get(primary_index as PdfPageObjectIndex).map_err(err)?;
                            let t =
                                o.as_text_object_mut().ok_or_else(|| PwError::internal("edit.text", "Not text."))?;
                            t.set_text(&new_text).map_err(err)?;
                            if let Some(s) = size {
                                let k = s / run.size.max(0.1);
                                let u = t.unscaled_font_size().value;
                                t.set_unscaled_font_size(PdfPoints::new(u * k)).map_err(err)?;
                            }
                            if let Some(c) = c {
                                t.set_fill_color(color(c)?).map_err(err)?;
                            }
                            if *dx != 0.0 || *dy != 0.0 {
                                t.translate(PdfPoints::new(tx), PdfPoints::new(ty)).map_err(err)?;
                            }
                            // In-place changes must be written into the content stream through this
                            // same page handle: a page loaded again starts from the saved content.
                            drop(o);
                            page.regenerate_content().map_err(err)?;
                        }
                        Some(font) => {
                            substituted += 1;
                            let (m, fill, unscaled) = {
                                let o = page.objects().get(primary_index as PdfPageObjectIndex).map_err(err)?;
                                let t =
                                    o.as_text_object().ok_or_else(|| PwError::internal("edit.text", "Not text."))?;
                                let fill = match c {
                                    Some(c) => color(c)?,
                                    None => t.fill_color().unwrap_or(PdfColor::BLACK),
                                };
                                let unscaled =
                                    t.unscaled_font_size().value * size.map(|s| s / run.size.max(0.1)).unwrap_or(1.0);
                                (t.matrix().map_err(err)?, fill, unscaled)
                            };
                            let mut n =
                                PdfPageTextObject::new(&doc, &new_text, font, PdfPoints::new(unscaled)).map_err(err)?;
                            n.set_fill_color(fill).map_err(err)?;
                            n.apply_matrix(m).map_err(err)?;
                            if *dx != 0.0 || *dy != 0.0 {
                                n.translate(PdfPoints::new(tx), PdfPoints::new(ty)).map_err(err)?;
                            }
                            page.objects_mut().add_text_object(n).map_err(err)?;
                            remove.insert(primary_index);
                        }
                    }
                    // The other objects of the run go: the first one now holds the whole line.
                    for &o in &objects[1..] {
                        remove.insert(o);
                    }
                    if !new_text.trim().is_empty() {
                        expect_texts.push(new_text);
                    }
                }
                PageEdit::DeleteObjects { objects } => {
                    for &o in objects {
                        if image_ids.contains(&o) {
                            images_removed += 1;
                        } else if !listing.texts.iter().any(|t| t.objects.contains(&o)) {
                            return Err(PwError::invalid("edit.stale", "The object to delete was not found."));
                        }
                        remove.insert(o);
                    }
                }
                PageEdit::MoveObjects { objects, dx, dy } => {
                    let (tx, ty) = unrotate(rot, *dx, *dy);
                    let mut page = doc.pages().get(pe.page as PdfPageIndex).map_err(err)?;
                    for &o in objects {
                        if !image_ids.contains(&o) && !listing.texts.iter().any(|t| t.objects.contains(&o)) {
                            return Err(PwError::invalid("edit.stale", "The object to move was not found."));
                        }
                        let mut obj = page.objects().get(o as PdfPageObjectIndex).map_err(err)?;
                        obj.translate(PdfPoints::new(tx), PdfPoints::new(ty)).map_err(err)?;
                    }
                    page.regenerate_content().map_err(err)?;
                }
                PageEdit::ReplaceImage { object, .. } | PageEdit::RotateImage { object, .. } => {
                    if !image_ids.contains(object) {
                        return Err(PwError::invalid("edit.stale", "The picture was not found."));
                    }
                    let mut page = doc.pages().get(pe.page as PdfPageIndex).map_err(err)?;
                    let (m, src) = {
                        let old = page.objects().get(*object as PdfPageObjectIndex).map_err(err)?;
                        let img =
                            old.as_image_object().ok_or_else(|| PwError::internal("edit.image", "Not an image."))?;
                        let src = match e {
                            PageEdit::RotateImage { .. } => Some(img.get_raw_image().map_err(err)?),
                            _ => None,
                        };
                        (img.matrix().map_err(err)?, src)
                    };
                    let new_img = match (e, src) {
                        (PageEdit::ReplaceImage { .. }, _) => {
                            pictures.remove(&k).ok_or_else(|| PwError::internal("edit.image", "Picture missing."))?
                        }
                        (PageEdit::RotateImage { degrees, .. }, Some(src)) => match degrees {
                            90 => src.rotate90(),
                            180 => src.rotate180(),
                            270 => src.rotate270(),
                            _ => {
                                return Err(PwError::invalid(
                                    "edit.degrees",
                                    "Turn pictures by 90, 180 or 270 degrees.",
                                ))
                            }
                        },
                        _ => return Err(PwError::internal("edit.image", "Picture missing.")),
                    };
                    // Keep the placement box (and its aspect) of the old picture: the new one fills it.
                    let mut n = PdfPageImageObject::new(&doc, &new_img).map_err(err)?;
                    n.apply_matrix(m).map_err(err)?;
                    page.objects_mut().add_image_object(n).map_err(err)?;
                    remove.insert(*object);
                    images_added += 1;
                    images_removed += 1;
                }
                PageEdit::AddText { x, y, text, size, color: c } => {
                    let t = text.trim_end_matches(['\r', '\n']);
                    if t.trim().is_empty() || t.chars().any(|ch| (ch as u32) > 255 || ch.is_control()) {
                        return Err(PwError::invalid(
                            "edit.add_text",
                            "Type text using Latin letters (Western European characters).",
                        ));
                    }
                    let font = doc.fonts_mut().helvetica();
                    let mut page = doc.pages().get(pe.page as PdfPageIndex).map_err(err)?;
                    let mut n =
                        PdfPageTextObject::new(&doc, t, font, PdfPoints::new(size.clamp(4.0, 200.0))).map_err(err)?;
                    n.set_fill_color(color(c)?).map_err(err)?;
                    n.rotate_counter_clockwise_degrees(rot as f32).map_err(err)?;
                    n.translate(PdfPoints::new(*x), PdfPoints::new(*y)).map_err(err)?;
                    page.objects_mut().add_text_object(n).map_err(err)?;
                    expect_texts.push(t.to_string());
                }
            }
        }
        // Remove from the highest index down so earlier indices stay valid.
        let mut page = doc.pages().get(pe.page as PdfPageIndex).map_err(err)?;
        for &o in remove.iter().rev() {
            page.objects_mut().remove_object_at_index(o as PdfPageObjectIndex).map_err(err)?;
        }
        page.regenerate_content().map_err(err)?;
        expectations.push(Expectation {
            page: pe.page,
            texts: expect_texts,
            images: listing.images.len() + images_added - images_removed,
        });
    }
    let bytes = save_bytes(&doc)?;
    let src_encrypted = source_encrypted(&doc);
    drop(doc);
    // Reopen the saved bytes and check every expectation.
    {
        let fail = |what: String| {
            PwError::new(
                ErrorCategory::ValidationFailed,
                "output.verify",
                format!("The edited copy failed verification ({what}) and was not saved."),
            )
        };
        let reopened = pdfium
            .load_pdf_from_byte_slice(&bytes, None)
            .or_else(|_| pdfium.load_pdf_from_byte_slice(&bytes, opts.password))
            .map_err(|_| fail("it could not be reopened".into()))?;
        if reopened.pages().len() as u32 != total {
            return Err(fail("the page count changed".into()));
        }
        for ex in &expectations {
            let listing = list_page(&reopened, ex.page)?;
            let squash = |t: &str| t.chars().filter(|c| !c.is_whitespace()).collect::<String>();
            let all: String = listing.texts.iter().map(|t| squash(&t.text)).collect();
            for t in &ex.texts {
                if !all.contains(&squash(t)) {
                    return Err(fail(format!("\"{t}\" is missing on page {}", ex.page + 1)));
                }
            }
            if listing.images.len() != ex.images {
                return Err(fail(format!(
                    "page {} has {} pictures, expected {}",
                    ex.page + 1,
                    listing.images.len(),
                    ex.images
                )));
            }
        }
    }
    let mut warnings = Vec::new();
    if substituted > 0 {
        warnings.push(format!(
            "{substituted} edited text{} used letters its font does not contain, so a similar standard font was used for {}.",
            if substituted == 1 { "" } else { "s" },
            if substituted == 1 { "it" } else { "them" }
        ));
    }
    if signatures > 0 {
        warnings.push(crate::properties::signature_warning(signatures));
    }
    Ok(EditedBytes { bytes, page_count: total, source_encrypted: src_encrypted, warnings })
}

fn pick_standard(
    doc: &mut PdfDocument,
    fixed: bool,
    serif: bool,
    italic: bool,
    bold: bool,
    name: &str,
) -> PdfFontToken {
    let n = name.to_lowercase();
    let fonts = doc.fonts_mut();
    if fixed || n.contains("mono") || n.contains("courier") {
        match (bold, italic) {
            (true, true) => fonts.courier_bold_oblique(),
            (true, false) => fonts.courier_bold(),
            (false, true) => fonts.courier_oblique(),
            _ => fonts.courier(),
        }
    } else if serif || n.contains("times") || (n.contains("serif") && !n.contains("sans")) {
        match (bold, italic) {
            (true, true) => fonts.times_bold_italic(),
            (true, false) => fonts.times_bold(),
            (false, true) => fonts.times_italic(),
            _ => fonts.times_roman(),
        }
    } else {
        match (bold, italic) {
            (true, true) => fonts.helvetica_bold_oblique(),
            (true, false) => fonts.helvetica_bold(),
            (false, true) => fonts.helvetica_oblique(),
            _ => fonts.helvetica(),
        }
    }
}

/// A move as displayed (dx right, dy up) in unrotated page space.
fn unrotate(rot: u32, dx: f32, dy: f32) -> (f32, f32) {
    match rot {
        90 => (-dy, dx),
        180 => (-dx, -dy),
        270 => (dy, -dx),
        _ => (dx, dy),
    }
}

/// Page `page` rendered with `edits` applied (PNG), for previewing pending changes. Nothing is
/// written; the edits are applied and checked in memory exactly as saving would.
pub fn preview_object_edits(
    input: &Path,
    opts: &OpenOptions,
    page: u32,
    edits: &[PageEdit],
    scale: f32,
) -> PwResult<Vec<u8>> {
    if !scale.is_finite() || !(0.05..=8.0).contains(&scale) {
        return Err(PwError::invalid("render.scale", "That zoom level cannot be previewed."));
    }
    let edited = apply_object_edits(input, opts, &[PageEdits { page, edits: edits.to_vec() }])?;
    let pdfium = engine()?;
    let doc = pdfium
        .load_pdf_from_byte_slice(&edited.bytes, None)
        .or_else(|_| pdfium.load_pdf_from_byte_slice(&edited.bytes, opts.password))
        .map_err(err)?;
    let p = doc.pages().get(page as PdfPageIndex).map_err(err)?;
    let (w, h) = (p.width().value * scale, p.height().value * scale);
    if (w * h) as u64 > 64_000_000 {
        return Err(PwError::invalid("render.too_large", "That zoom level is too large for this page."));
    }
    let cfg = PdfRenderConfig::new().scale_page_by_factor(scale).render_form_data(true).render_annotations(true);
    let img = p.render_with_config(&cfg).map_err(err)?.as_image().map_err(err)?;
    let mut png = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .map_err(|_| PwError::internal("render.encode", "Could not encode the preview."))?;
    Ok(png)
}

#[cfg(test)]
mod tests {
    use super::unrotate;

    #[test]
    fn moves_follow_the_displayed_orientation() {
        // Displayed right on a page turned 90° clockwise is up in unrotated space.
        assert_eq!(unrotate(90, 10.0, 0.0), (0.0, 10.0));
        assert_eq!(unrotate(90, 0.0, 10.0), (-10.0, 0.0));
        assert_eq!(unrotate(180, 10.0, 5.0), (-10.0, -5.0));
        assert_eq!(unrotate(270, 10.0, 0.0), (0.0, -10.0));
        assert_eq!(unrotate(0, 3.0, 4.0), (3.0, 4.0));
    }
}
