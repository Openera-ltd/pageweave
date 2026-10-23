//! PDF → Word (.docx) conversion (PRD 06 "PDF → Word and editable exports", decision D-016).
//!
//! Text comes from PDFium's character data (positions, fonts, sizes, weights, colours) and is
//! rebuilt as editable paragraphs, headings, columns and runs ([`layout`]); embedded pictures are
//! carried over as pictures (JPEG unchanged); web and mail links stay links. Nothing is
//! rasterized: a page without a text layer keeps its picture and is reported as needing text
//! recognition. Before the file is written it is read back ([`reader`]): it must open, every part
//! must parse, every picture must decode, and it must hold every character the converted pages
//! hold. Lines, shapes and table borders are not carried over yet (reported).

pub mod borderless;
pub mod layout;
pub mod reader;
pub mod tables;
pub mod translate;
pub mod writer;

use crate::document::{load, OpenOptions};
use crate::edit::rotation_of;
use crate::{engine, map_pdfium_error};
use layout::{Glyph, PageInput, PageLayout, Picture, Rect};
use pdfium_render::prelude::*;
use pw_core::output::{write_atomic_bytes, CollisionPolicy, CommittedOutput};
use pw_core::range::parse_page_ranges;
use pw_core::{ErrorCategory, PwError, PwResult};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::{BTreeSet, HashMap};
use std::path::Path;
use tables::Rule;
use writer::{DocxInput, Media, MediaKind};

/// Pictures larger than this on their long side are scaled down (PNG only; JPEG stays as stored).
const MAX_PICTURE_SIDE: u32 = 6000;
/// Nesting limit for form XObjects (hostile files).
const MAX_FORM_DEPTH: usize = 8;

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct DocxReport {
    pub pages: u32,
    pub paragraphs: usize,
    pub headings: usize,
    pub pictures: usize,
    pub links: usize,
    /// Ruled tables carried over as Word tables.
    pub tables: usize,
    /// Pages laid out in more than one column somewhere.
    pub pages_with_columns: Vec<u32>,
    /// Pages with no text layer (1-based): their pictures are kept, their text is not editable.
    pub pages_without_text: Vec<u32>,
    /// Pages whose text came from a recognized (invisible) layer; the scan picture was left out.
    pub pages_from_recognized_text: Vec<u32>,
    /// Fonts the document uses, as named for Word (a missing one is replaced by the editor).
    pub fonts: Vec<String>,
    /// Lines, shapes and borders not carried over.
    pub drawings_left_out: usize,
    /// Pictures inside grouped content that could not be placed.
    pub pictures_left_out: usize,
    /// Characters drawn turned or slanted, placed after their page's content.
    pub turned_characters: usize,
    /// Pages laid out unturned because their text reads upright that way (1-based).
    pub pages_turned_upright: Vec<u32>,
    /// Comments, stamps and markup not carried over.
    pub comments_left_out: usize,
    /// Fillable form fields not carried over (their printed labels are).
    pub form_fields_left_out: usize,
}

fn page_to_display(rot: u32, w: f32, h: f32, x: f32, y: f32) -> (f32, f32) {
    match rot {
        90 => (y, w - x),
        180 => (w - x, h - y),
        270 => (h - y, x),
        _ => (x, y),
    }
}

fn rect_to_display(rot: u32, w: f32, h: f32, r: &PdfRect, origin: (f32, f32)) -> Rect {
    let (ax, ay) = page_to_display(rot, w, h, r.left().value - origin.0, r.bottom().value - origin.1);
    let (bx, by) = page_to_display(rot, w, h, r.right().value - origin.0, r.top().value - origin.1);
    Rect { left: ax.min(bx), bottom: ay.min(by), right: ax.max(bx), top: ay.max(by) }
}

fn allowed_link(uri: &str) -> bool {
    let u = uri.trim().to_ascii_lowercase();
    (u.starts_with("http://") || u.starts_with("https://") || u.starts_with("mailto:")) && uri.len() <= 2048
}

/// Encode a picture for Word. Plain RGB/grey JPEG data is kept exactly as stored, unless the
/// picture has a transparency mask; everything else is taken as PDFium draws it at the picture's
/// own pixel size (masks, colour spaces and decode arrays applied) and saved as PNG.
fn picture_data(img: &PdfPageImageObject, doc: &PdfDocument) -> Option<Media> {
    let (pw, ph) = (img.width().ok()?.max(1), img.height().ok()?.max(1));
    let scale = (MAX_PICTURE_SIDE as f32 / pw.max(ph) as f32).min(1.0);
    let (tw, th) = (((pw as f32 * scale).round() as i32).max(1), ((ph as f32 * scale).round() as i32).max(1));
    let filters: Vec<String> = img.filters().iter().map(|f| f.name().to_string()).collect();
    let rgb_or_gray = matches!(img.color_space(), Ok(PdfColorSpace::DeviceRGB | PdfColorSpace::DeviceGray));
    // A small drawing of the picture tells whether a mask makes parts of it transparent.
    let masked = img
        .get_processed_image_with_size(doc, 48, 48)
        .map(|i| i.to_rgba8().pixels().any(|p| p.0[3] < 250))
        .unwrap_or(false);
    if filters == ["DCTDecode"] && rgb_or_gray && !masked {
        if let Ok(raw) = img.get_raw_image_data() {
            if image::load_from_memory_with_format(&raw, image::ImageFormat::Jpeg).is_ok() {
                return Some(Media { bytes: raw, kind: MediaKind::Jpeg });
            }
        }
    }
    let processed = || img.get_processed_image_with_size(doc, tw, th).ok();
    let raw = || {
        img.get_raw_image().ok().map(|r| {
            if r.width().max(r.height()) > MAX_PICTURE_SIDE {
                r.resize(MAX_PICTURE_SIDE, MAX_PICTURE_SIDE, image::imageops::FilterType::Triangle)
            } else {
                r
            }
        })
    };
    // Unmasked pictures keep their own pixels; masked ones are taken as drawn, with the mask.
    let decoded = if masked { processed().or_else(raw)? } else { raw().or_else(processed)? };
    // Opaque pictures need no alpha channel.
    let rgba = decoded.to_rgba8();
    let opaque = rgba.pixels().all(|p| p.0[3] == 255);
    let out: image::DynamicImage =
        if opaque { image::DynamicImage::ImageRgb8(decoded.to_rgb8()) } else { image::DynamicImage::ImageRgba8(rgba) };
    let mut png = Vec::new();
    out.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).ok()?;
    Some(Media { bytes: png, kind: MediaKind::Png })
}

/// Pictures of a form XObject that holds exactly one picture and nothing visible besides.
fn single_picture<'a>(form: &'a PdfPageXObjectFormObject<'a>, depth: usize) -> Option<PdfPageImageObject<'a>> {
    if depth > MAX_FORM_DEPTH || form.len() != 1 {
        return None;
    }
    match form.get(0).ok()? {
        PdfPageObject::Image(i) => Some(i),
        _ => None,
    }
}

fn count_pictures(o: &PdfPageObject, depth: usize) -> usize {
    match o {
        PdfPageObject::Image(_) => 1,
        PdfPageObject::XObjectForm(f) if depth < MAX_FORM_DEPTH => {
            (0..f.len()).filter_map(|i| f.get(i).ok()).map(|c| count_pictures(&c, depth + 1)).sum()
        }
        _ => 0,
    }
}

/// The straight horizontal and vertical lines a path draws (page coordinates as displayed).
fn path_rules(path: &PdfPagePathObject, rot: u32, w: f32, h: f32, origin: (f32, f32)) -> Vec<Rule> {
    let stroked = path.is_stroked().unwrap_or(false);
    let filled = !matches!(path.fill_mode(), Ok(PdfPathFillMode::None) | Err(_));
    if !stroked && !filled {
        return Vec::new();
    }
    let Ok(m) = path.matrix() else { return Vec::new() };
    let segs = path.segments().transform(m);
    let n = segs.len();
    if n > 5000 {
        return Vec::new();
    }
    let disp = |x: PdfPoints, y: PdfPoints| page_to_display(rot, w, h, x.value - origin.0, y.value - origin.1);
    let mut out = Vec::new();
    let (mut cur, mut start) = ((0.0f32, 0.0f32), (0.0f32, 0.0f32));
    for i in 0..n {
        let Ok(seg) = segs.get(i) else { continue };
        let (x, y) = seg.point();
        let p = disp(x, y);
        match seg.segment_type() {
            PdfPathSegmentType::MoveTo => {
                cur = p;
                start = p;
            }
            PdfPathSegmentType::LineTo => {
                if let Some(r) = tables::rule_from_segment(cur.0, cur.1, p.0, p.1) {
                    out.push(r);
                }
                cur = p;
            }
            _ => cur = p,
        }
        if seg.is_close() {
            if let Some(r) = tables::rule_from_segment(cur.0, cur.1, start.0, start.1) {
                out.push(r);
            }
            cur = start;
        }
    }
    out
}

/// How displayed coordinates relate to the page's own (unrotated) coordinates.
#[derive(Debug, Clone, Copy)]
struct Frame {
    rot: u32,
    w: f32,
    h: f32,
    origin: (f32, f32),
}

impl Frame {
    /// A displayed rectangle in the page's own coordinates (as the viewer and search use).
    fn to_page(self, r: &Rect) -> crate::SearchRect {
        let back = |dx: f32, dy: f32| -> (f32, f32) {
            let (x, y) = match self.rot {
                90 => (self.w - dy, dx),
                180 => (self.w - dx, self.h - dy),
                270 => (dy, self.h - dx),
                _ => (dx, dy),
            };
            (x + self.origin.0, y + self.origin.1)
        };
        let (ax, ay) = back(r.left, r.bottom);
        let (bx, by) = back(r.right, r.top);
        crate::SearchRect { left: ax.min(bx), bottom: ay.min(by), right: ax.max(bx), top: ay.max(by) }
    }
}

struct Extracted {
    input: PageInput,
    frame: Frame,
    /// Turned text, in engine order, split at line ends.
    turned: Vec<String>,
    recognized_layer: bool,
    drawings: usize,
    /// Drawn lines of each path object (for telling table lines from other drawings).
    path_rules: Vec<Vec<Rule>>,
    pictures_left_out: usize,
    turned_upright: bool,
    comments: usize,
    fields: usize,
}

struct Shared {
    media: Vec<Media>,
    media_by_hash: HashMap<[u8; 32], usize>,
    links: Vec<String>,
    link_ids: HashMap<String, usize>,
    fonts: BTreeSet<String>,
}

fn upright_at(angle: f32, rot: u32) -> bool {
    let up = |a: f32| !(3.0..=357.0).contains(&a);
    up((angle + rot as f32).rem_euclid(360.0)) || up((angle - rot as f32).rem_euclid(360.0))
}

fn extract_page(page: &PdfPage, doc: &PdfDocument, shared: &mut Shared, with_pictures: bool) -> PwResult<Extracted> {
    let text = page.text().map_err(|e| map_pdfium_error(&e, "text"))?;
    // Lay the page out the way its text reads: as displayed, or unturned when the page's turn
    // only turns its text sideways.
    let mut rot = rotation_of(page);
    let mut turned_upright = false;
    if rot != 0 {
        let (mut shown, mut unturned) = (0usize, 0usize);
        for c in text.chars().iter().filter(|c| !c.is_generated().unwrap_or(true)) {
            let a = c.angle_degrees().unwrap_or(0.0).rem_euclid(360.0);
            shown += upright_at(a, rot) as usize;
            unturned += upright_at(a, 0) as usize;
        }
        if unturned > shown {
            rot = 0;
            turned_upright = true;
        }
    }
    let bounds = page
        .boundaries()
        .bounding()
        .map(|b| b.bounds)
        .unwrap_or_else(|_| PdfRect::new_from_values(0.0, 0.0, page.height().value, page.width().value));
    let origin = (bounds.left().value, bounds.bottom().value);
    let (w, h) = (bounds.width().value, bounds.height().value);
    let (dw, dh) = if matches!(rot, 90 | 270) { (h, w) } else { (w, h) };

    // Links of the page (web and mail only).
    let mut page_links: Vec<(Rect, usize)> = Vec::new();
    for link in page.links().iter() {
        let Some(PdfAction::Uri(u)) = link.action() else { continue };
        let Ok(uri) = u.uri() else { continue };
        if !allowed_link(&uri) {
            continue;
        }
        let Ok(r) = link.rect() else { continue };
        let next = shared.links.len();
        let id = *shared.link_ids.entry(uri.clone()).or_insert_with(|| {
            shared.links.push(uri.clone());
            next
        });
        page_links.push((rect_to_display(rot, w, h, &r, origin), id));
    }

    let mut glyphs = Vec::new();
    let mut turned = Vec::new();
    let mut turned_line = String::new();
    let mut visible = 0usize;
    let mut invisible = 0usize;
    for c in text.chars().iter() {
        let Some(ch) = c.unicode_char() else { continue };
        let generated = c.is_generated().unwrap_or(false);
        if generated {
            glyphs.push(Glyph {
                ch,
                left: 0.0,
                right: 0.0,
                bottom: 0.0,
                top: 0.0,
                baseline: 0.0,
                size: 0.0,
                font: String::new(),
                bold: false,
                italic: false,
                color: [0, 0, 0],
                generated: true,
                hyphen: false,
                link: None,
            });
            if (ch == '\n' || ch == '\r') && !turned_line.is_empty() {
                turned.push(std::mem::take(&mut turned_line));
            }
            continue;
        }
        if ch.is_control() && ch != '\t' {
            continue;
        }
        let Ok(b) = c.loose_bounds() else { continue };
        let r = rect_to_display(rot, w, h, &b, origin);
        let (ox, oy) = c.origin().map(|(x, y)| (x.value, y.value)).unwrap_or((b.left().value, b.bottom().value));
        let (_, baseline) = page_to_display(rot, w, h, ox - origin.0, oy - origin.1);
        match c.render_mode() {
            Ok(PdfPageTextRenderMode::Invisible) => invisible += 1,
            _ => visible += 1,
        }
        // Upright on the displayed page: the character's own angle cancels the page's turn.
        let angle = c.angle_degrees().unwrap_or(0.0).rem_euclid(360.0);
        if !upright_at(angle, rot) {
            turned_line.push(ch);
            continue;
        }
        let font = c.font_name();
        let bold = c
            .font_weight()
            .map(|w| matches!(w, PdfFontWeight::Weight700Bold | PdfFontWeight::Weight800 | PdfFontWeight::Weight900))
            .unwrap_or(false)
            || c.font_is_bold_reenforced()
            || font.to_ascii_lowercase().contains("bold");
        let italic = c.font_is_italic() || {
            let f = font.to_ascii_lowercase();
            f.contains("italic") || f.contains("oblique")
        };
        let color = c.fill_color().map(|c| [c.red(), c.green(), c.blue()]).unwrap_or([0, 0, 0]);
        let (cx, cy) = ((r.left + r.right) / 2.0, (r.bottom + r.top) / 2.0);
        let link = page_links
            .iter()
            .find(|(lr, _)| cx >= lr.left && cx <= lr.right && cy >= lr.bottom && cy <= lr.top)
            .map(|(_, id)| *id);
        shared.fonts.insert(writer::family(&font));
        glyphs.push(Glyph {
            ch,
            left: r.left,
            right: r.right,
            bottom: r.bottom,
            top: r.top,
            baseline,
            size: c.scaled_font_size().value,
            font,
            bold,
            italic,
            color,
            generated: false,
            hyphen: c.is_hyphen().unwrap_or(false),
            link,
        });
    }
    if !turned_line.is_empty() {
        turned.push(turned_line);
    }
    let recognized_layer = invisible > 0 && visible == 0;

    let mut pictures = Vec::new();
    let mut rules: Vec<Rule> = Vec::new();
    let mut path_rules_all: Vec<Vec<Rule>> = Vec::new();
    let mut drawings = 0;
    let mut pictures_left_out = 0;
    let mut place = |img: &PdfPageImageObject, r: PdfRect, pictures: &mut Vec<Picture>, left_out: &mut usize| {
        let Some(m) = picture_data(img, doc) else {
            *left_out += 1;
            return;
        };
        let hash: [u8; 32] = Sha256::digest(&m.bytes).into();
        let next = shared.media.len();
        let id = *shared.media_by_hash.entry(hash).or_insert(next);
        if id == next {
            shared.media.push(m);
        }
        pictures.push(Picture { id, rect: rect_to_display(rot, w, h, &r, origin) });
    };
    for o in page.objects().iter() {
        match &o {
            PdfPageObject::Image(_) | PdfPageObject::XObjectForm(_) if !with_pictures => {}
            PdfPageObject::Image(img) => {
                if let Ok(q) = o.bounds() {
                    place(img, q.to_rect(), &mut pictures, &mut pictures_left_out);
                }
            }
            PdfPageObject::XObjectForm(f) => match single_picture(f, 0) {
                Some(img) => {
                    if let Ok(q) = o.bounds() {
                        place(&img, q.to_rect(), &mut pictures, &mut pictures_left_out);
                    }
                }
                None => pictures_left_out += count_pictures(&o, 0),
            },
            PdfPageObject::Path(path) => {
                drawings += 1;
                let r = path_rules(path, rot, w, h, origin);
                rules.extend(r.iter().copied());
                path_rules_all.push(r);
            }
            PdfPageObject::Shading(_) => drawings += 1,
            _ => {}
        }
    }
    let (mut comments, mut fields) = (0, 0);
    for a in page.annotations().iter() {
        match a.annotation_type() {
            PdfPageAnnotationType::Link | PdfPageAnnotationType::Popup => {}
            PdfPageAnnotationType::Widget | PdfPageAnnotationType::XfaWidget => fields += 1,
            _ => comments += 1,
        }
    }
    Ok(Extracted {
        input: PageInput { width: dw, height: dh, glyphs, pictures, rules },
        frame: Frame { rot, w, h, origin },
        path_rules: path_rules_all,
        turned_upright,
        comments,
        fields,
        turned,
        recognized_layer,
        drawings,
        pictures_left_out,
    })
}

fn fail(what: &str) -> PwError {
    PwError::new(
        ErrorCategory::ValidationFailed,
        "output.verify",
        format!("The Word file failed verification ({what}) and was not saved."),
    )
}

fn add_turned(layout: &mut PageLayout, turned: &[String]) {
    if turned.is_empty() {
        return;
    }
    let items = turned
        .iter()
        .map(|t| {
            layout::Item::Para(layout::Paragraph {
                runs: vec![layout::Run {
                    text: t.clone(),
                    font: "Arial".into(),
                    size: 10.0,
                    bold: false,
                    italic: false,
                    color: [0, 0, 0],
                    link: None,
                    vert: layout::VertAlign::Baseline,
                }],
                align: layout::Align::Left,
                indent: 0.0,
                first_line: 0.0,
                space_before: 6.0,
                pitch: 12.0,
                heading: None,
                tabs: Vec::new(),
                rect: Rect { left: 0.0, bottom: 0.0, right: 0.0, top: 0.0 },
                boxes: Vec::new(),
            })
        })
        .collect();
    let (left, right) = (layout.margin_left, layout.width - layout.margin_right);
    layout.bands.push(layout::Band { columns: vec![layout::Column { left, right, items }] });
}

/// The ruled tables of one page, as laid out for Word (cells with their paragraphs), top first.
pub(crate) fn page_tables(page: &PdfPage, doc: &PdfDocument) -> PwResult<Vec<layout::TableItem>> {
    let mut shared = Shared {
        media: Vec::new(),
        media_by_hash: HashMap::new(),
        links: Vec::new(),
        link_ids: HashMap::new(),
        fonts: BTreeSet::new(),
    };
    let ex = extract_page(page, doc, &mut shared, false)?;
    let lay = layout::layout_page(&ex.input);
    let mut found = Vec::new();
    for b in &lay.bands {
        for c in &b.columns {
            for it in &c.items {
                if let layout::Item::Table { table, .. } = it {
                    found.push(table.clone());
                }
            }
        }
    }
    // Tables without drawn lines too (D-020), in page order with the ruled ones.
    found.extend(layout::borderless_tables(&ex.input));
    found.sort_by(|a, b| b.rect.top.total_cmp(&a.rect.top).then(a.rect.left.total_cmp(&b.rect.left)));
    Ok(found)
}

/// One page's text in reading order (columns one after another, table cells row by row, turned
/// text last), paragraphs separated by line ends, with where each word is on the page (in the
/// page's own coordinates). Offsets count Unicode characters.
pub(crate) fn page_reading(page: &PdfPage, doc: &PdfDocument) -> PwResult<crate::reading::ReadingPage> {
    let mut shared = Shared {
        media: Vec::new(),
        media_by_hash: HashMap::new(),
        links: Vec::new(),
        link_ids: HashMap::new(),
        fonts: BTreeSet::new(),
    };
    let ex = extract_page(page, doc, &mut shared, false)?;
    let mut lay = layout::layout_page(&ex.input);
    add_turned(&mut lay, &ex.turned);
    let mut paras: Vec<&layout::Paragraph> = Vec::new();
    for b in &lay.bands {
        for c in &b.columns {
            for it in &c.items {
                match it {
                    layout::Item::Para(p) => paras.push(p),
                    layout::Item::Table { table, .. } => {
                        for cell in &table.cells {
                            paras.extend(cell.paragraphs.iter());
                        }
                    }
                    layout::Item::Pictures { .. } => {}
                }
            }
        }
    }
    let mut chars: Vec<char> = Vec::new();
    let mut boxes: Vec<Option<Rect>> = Vec::new();
    for p in paras {
        let text: String = p.runs.iter().map(|r| r.text.as_str()).collect();
        let n0 = chars.len();
        for ch in text.chars() {
            chars.push(if ch == '\t' { ' ' } else { ch });
        }
        let added = chars.len() - n0;
        // Turned text has no positions; a mismatch (never expected) drops them rather than misplacing.
        let have = p.boxes.len() == added;
        boxes.extend((0..added).map(|i| have.then(|| p.boxes[i])));
        while chars.last().is_some_and(|c| *c == ' ') && chars.len() > n0 {
            chars.pop();
            boxes.pop();
        }
        if chars.len() > n0 {
            chars.push('\n');
            boxes.push(None);
        }
    }
    if chars.last() == Some(&'\n') {
        chars.pop();
        boxes.pop();
    }
    let mut words = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_whitespace() {
            i += 1;
            continue;
        }
        let start = i;
        while i < chars.len() && !chars[i].is_whitespace() {
            i += 1;
        }
        // One rectangle per line the word is on (a hyphenated word can span two).
        let mut rects: Vec<Rect> = Vec::new();
        for b in boxes[start..i].iter().flatten() {
            if b.right <= b.left && b.top <= b.bottom {
                continue;
            }
            match rects.last_mut() {
                Some(r) if b.bottom < r.top && b.top > r.bottom && b.left >= r.left - 1.0 => {
                    r.left = r.left.min(b.left);
                    r.right = r.right.max(b.right);
                    r.bottom = r.bottom.min(b.bottom);
                    r.top = r.top.max(b.top);
                }
                _ => rects.push(*b),
            }
        }
        words.push(crate::reading::ReadingWord {
            start: start as u32,
            end: i as u32,
            rects: rects.iter().map(|r| ex.frame.to_page(r)).collect(),
        });
    }
    Ok(crate::reading::ReadingPage { page: 0, text: chars.into_iter().collect(), words })
}

/// What the PDF's chosen pages hold, laid out for an editable document: shared by Word and
/// PowerPoint export.
pub(crate) struct Analysis {
    pub layouts: Vec<PageLayout>,
    pub media: Vec<Media>,
    pub links: Vec<String>,
    /// Every character the output must hold (whitespace excluded).
    pub expected: HashMap<char, usize>,
    pub report: DocxReport,
    pub placed_pictures: usize,
    pub expected_tables: usize,
}

pub(crate) fn analyze(
    doc: &PdfDocument,
    idx: &[u32],
    cancelled: &dyn Fn() -> bool,
    progress: &dyn Fn(u64, u64),
) -> PwResult<Analysis> {
    let mut shared = Shared {
        media: Vec::new(),
        media_by_hash: HashMap::new(),
        links: Vec::new(),
        link_ids: HashMap::new(),
        fonts: BTreeSet::new(),
    };
    let mut layouts = Vec::with_capacity(idx.len());
    let mut expected: HashMap<char, usize> = HashMap::new();
    let mut report = DocxReport { pages: idx.len() as u32, ..Default::default() };
    let mut placed_pictures = 0usize;
    let mut expected_tables = 0usize;
    for (k, &p) in idx.iter().enumerate() {
        if cancelled() {
            return Err(PwError::cancelled());
        }
        let page = doc.pages().get(p as PdfPageIndex).map_err(|e| map_pdfium_error(&e, "page"))?;
        let mut ex = extract_page(&page, doc, &mut shared, true)?;
        let has_text = ex.input.glyphs.iter().any(|g| !g.generated && !g.ch.is_whitespace()) || !ex.turned.is_empty();
        if ex.recognized_layer {
            // The scan picture would sit under (and duplicate) the recognized text.
            let area = ex.input.width * ex.input.height;
            let before = ex.input.pictures.len();
            ex.input.pictures.retain(|pic| pic.rect.width() * pic.rect.height() < 0.5 * area);
            if ex.input.pictures.len() < before {
                report.pages_from_recognized_text.push(p + 1);
            }
        }
        if !has_text {
            report.pages_without_text.push(p + 1);
        }
        for (c, n) in layout::expected_chars(&ex.input) {
            if !writer::is_xml_invalid(c) {
                *expected.entry(c).or_default() += n;
            }
        }
        for t in &ex.turned {
            report.turned_characters += t.chars().count();
            for c in t.chars().filter(|c| !c.is_whitespace() && !writer::is_xml_invalid(*c)) {
                *expected.entry(c).or_default() += 1;
            }
        }
        let mut lay = layout::layout_page(&ex.input);
        add_turned(&mut lay, &ex.turned);
        if lay.bands.iter().any(|b| b.columns.len() > 1) {
            report.pages_with_columns.push(p + 1);
        }
        placed_pictures += lay.backgrounds.len()
            + lay.floating.len()
            + lay
                .bands
                .iter()
                .flat_map(|b| b.columns.iter())
                .flat_map(|c| c.items.iter())
                .map(|i| match i {
                    layout::Item::Pictures { row, .. } => row.len(),
                    _ => 0,
                })
                .sum::<usize>();
        // Paths that only draw the lines of a table were carried over as its borders.
        let on_table = |r: &Rule| {
            lay.tables.iter().any(|t| match *r {
                Rule::H { y, x0, x1 } => {
                    t.ys.iter().any(|ty| (ty - y).abs() <= 2.0) && x0 >= t.left() - 2.0 && x1 <= t.right() + 2.0
                }
                Rule::V { x, y0, y1 } => {
                    t.xs.iter().any(|tx| (tx - x).abs() <= 2.0) && y0 >= t.bottom() - 2.0 && y1 <= t.top() + 2.0
                }
            })
        };
        let table_paths = ex.path_rules.iter().filter(|rs| !rs.is_empty() && rs.iter().all(on_table)).count();
        report.drawings_left_out += ex.drawings - table_paths;
        expected_tables += lay.tables.len();
        report.comments_left_out += ex.comments;
        report.form_fields_left_out += ex.fields;
        if ex.turned_upright {
            report.pages_turned_upright.push(p + 1);
        }
        report.pictures_left_out += ex.pictures_left_out;
        layouts.push(lay);
        progress(k as u64 + 1, idx.len() as u64);
    }
    report.fonts = shared.fonts.iter().filter(|f| !f.is_empty()).cloned().collect();

    Ok(Analysis {
        layouts,
        media: shared.media,
        links: shared.links,
        expected,
        report,
        placed_pictures,
        expected_tables,
    })
}

/// Plain-language notes on what an editable export could not carry over.
pub(crate) fn notes(
    report: &DocxReport,
    layouts: &[PageLayout],
    encrypted: bool,
    app: &str,
    file: &str,
) -> Vec<String> {
    let mut warnings = Vec::new();
    let list = |v: &[u32]| {
        let s: Vec<String> = v.iter().take(10).map(|p| p.to_string()).collect();
        format!("{}{}", s.join(", "), if v.len() > 10 { ", …" } else { "" })
    };
    if !report.pages_without_text.is_empty() {
        warnings.push(format!(
            "Page(s) {} have no text layer, so they came out as pictures. Recognize text (OCR) first to get editable text.",
            list(&report.pages_without_text)
        ));
    }
    if !report.pages_from_recognized_text.is_empty() {
        warnings.push(format!(
            "Page(s) {} are scans: their recognized text was used and the scanned picture left out.",
            list(&report.pages_from_recognized_text)
        ));
    }
    if report.drawings_left_out > 0 {
        warnings.push(format!("{} drawn line(s) or shape(s) were not carried over.", report.drawings_left_out));
    }
    if report.pictures_left_out > 0 {
        warnings.push(format!("{} picture(s) inside grouped content could not be placed.", report.pictures_left_out));
    }
    if report.comments_left_out > 0 {
        warnings
            .push(format!("{} comment(s), stamp(s) or highlight(s) were not carried over.", report.comments_left_out));
    }
    if report.form_fields_left_out > 0 {
        warnings.push(format!(
            "{} fillable field(s) were not carried over; their labels were.",
            report.form_fields_left_out
        ));
    }
    if !report.pages_turned_upright.is_empty() {
        warnings
            .push(format!("Page(s) {} were turned so their text reads upright.", list(&report.pages_turned_upright)));
    }
    if report.turned_characters > 0 {
        warnings.push("Turned or slanted text was placed after its page's content.".into());
    }
    let shrunk: usize = layouts.iter().map(|l| l.shrunk_pictures).sum();
    if shrunk > 0 {
        warnings.push(format!("{shrunk} picture(s) were made smaller to fit their column."));
    }
    let uncommon: Vec<&String> = report
        .fonts
        .iter()
        .filter(|f| !["Arial", "Times New Roman", "Courier New", "Symbol", "Wingdings"].contains(&f.as_str()))
        .collect();
    if !uncommon.is_empty() {
        warnings.push(format!(
            "The document uses fonts {app} may not have ({}); {app} shows a similar font instead.",
            uncommon.iter().take(6).map(|s| s.as_str()).collect::<Vec<_>>().join(", "),
            app = app
        ));
    }
    if encrypted {
        warnings.push(format!("The original's password protection does not carry over to {file}."));
    }
    warnings
}

/// Convert `input` (all pages, or `pages`) to a Word document at `output`.
#[allow(clippy::too_many_arguments)]
pub fn convert_to_docx(
    input: &Path,
    opts: &OpenOptions,
    pages: Option<&str>,
    output: &Path,
    policy: CollisionPolicy,
    cancelled: &dyn Fn() -> bool,
    progress: &dyn Fn(u64, u64),
) -> PwResult<(CommittedOutput, DocxReport, Vec<String>)> {
    let pdfium = engine()?;
    let doc = load(&pdfium, input, opts)?;
    let n = doc.pages().len().max(0) as u32;
    let idx: Vec<u32> = match pages {
        Some(spec) => parse_page_ranges(spec, n)?,
        None => (0..n).collect(),
    };
    if idx.is_empty() {
        return Err(PwError::invalid("docx.no_pages", "Choose at least one page."));
    }
    let title = doc
        .metadata()
        .get(PdfDocumentMetadataTagType::Title)
        .map(|t| t.value().trim().to_string())
        .filter(|t| !t.is_empty());

    let Analysis { layouts, media, links, expected, mut report, placed_pictures, expected_tables } =
        analyze(&doc, &idx, cancelled, &|d, t| progress(d, t + 1))?;
    let input_doc = DocxInput { pages: &layouts, media: &media, links: &links, title: title.as_deref() };
    let bytes = writer::write_docx(&input_doc)?;
    let expected_sections: usize = layouts.iter().map(|l| l.bands.len().max(1)).sum();
    let verify = |bytes: &[u8]| -> PwResult<reader::DocxCheck> {
        let c = reader::check_docx(bytes)?;
        if c.sections != expected_sections {
            return Err(fail("its sections do not match the pages"));
        }
        if c.pictures != placed_pictures {
            return Err(fail("a picture is missing"));
        }
        if c.tables != expected_tables {
            return Err(fail("a table is missing"));
        }
        if c.chars != expected {
            return Err(fail("text is missing or changed"));
        }
        Ok(c)
    };
    let check = verify(&bytes)?;
    if cancelled() {
        return Err(PwError::cancelled());
    }
    let c = write_atomic_bytes(output, policy, &[input.to_path_buf()], &bytes, |staged| {
        let back = std::fs::read(staged).map_err(|e| PwError::from_io(&e, "output file"))?;
        if back != bytes {
            return Err(fail("the written file differs"));
        }
        verify(&back).map(|_| ())
    })?;
    progress(idx.len() as u64 + 1, idx.len() as u64 + 1);
    report.paragraphs = check.paragraphs;
    report.headings = check.headings;
    report.pictures = check.pictures;
    report.links = check.links;
    report.tables = check.tables;

    let warnings = notes(&report, &layouts, crate::edit::source_encrypted(&doc), "Word", "the Word file");
    Ok((c, report, warnings))
}
