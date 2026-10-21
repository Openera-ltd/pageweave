//! Opening, inspecting, searching and rendering documents.

use crate::{engine, map_pdfium_error};
use pdfium_render::prelude::*;
use pw_core::{ErrorCategory, PwError, PwResult};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Read;
use std::path::Path;

/// Inputs larger than this are refused before parsing (PRD 11: bound hostile inputs).
pub const MAX_INPUT_BYTES: u64 = 2 * 1024 * 1024 * 1024;
/// Upper bound for a rendered bitmap, in pixels (≈ 10k × 10k).
pub const MAX_RENDER_PIXELS: u64 = 100_000_000;
pub const MAX_SEARCH_HITS: usize = 10_000;
pub const MAX_BOOKMARKS: usize = 10_000;

#[derive(Debug, Clone, Default)]
pub struct OpenOptions<'a> {
    pub password: Option<&'a str>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PageInfo {
    pub index: u32,
    /// Page size in points *before* the intrinsic /Rotate is applied (the coordinate space of text,
    /// search geometry and annotations). The displayed size swaps these for 90°/270°.
    pub width_pt: f32,
    pub height_pt: f32,
    /// Intrinsic /Rotate value in degrees (0, 90, 180, 270).
    pub rotation: u32,
    pub label: Option<String>,
    /// Lower-left corner of the displayed page box (CropBox within MediaBox) in user space. Text,
    /// search and annotation coordinates are user space, so they are offset by this on screen.
    #[serde(default)]
    pub origin_x: f32,
    #[serde(default)]
    pub origin_y: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BookmarkInfo {
    pub title: String,
    pub page: Option<u32>,
    pub depth: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DocumentInfo {
    pub page_count: u32,
    pub file_bytes: u64,
    /// SHA-256 of the file bytes — a content identity for recovery/resume, not a security claim.
    pub sha256: String,
    pub pdf_version: String,
    pub title: Option<String>,
    pub author: Option<String>,
    pub subject: Option<String>,
    pub keywords: Option<String>,
    pub creator: Option<String>,
    pub producer: Option<String>,
    pub encrypted: bool,
    pub form_type: String,
    /// XFA forms are not supported by the pinned engine (no XFA support built in).
    pub has_xfa: bool,
    pub signature_count: u32,
    pub attachment_count: u32,
    pub pages: Vec<PageInfo>,
    pub bookmarks: Vec<BookmarkInfo>,
    pub bookmarks_truncated: bool,
    pub warnings: Vec<String>,
}

/// Cheap pre-parse checks: existence, size limit, PDF signature (per spec, `%PDF-` within the first 1024 bytes).
pub(crate) fn preflight(path: &Path) -> PwResult<u64> {
    let meta = fs::metadata(path).map_err(|e| PwError::from_io(&e, "file"))?;
    if meta.is_dir() {
        return Err(PwError::invalid("pdf.is_dir", "That is a folder, not a file."));
    }
    let len = meta.len();
    if len == 0 {
        return Err(PwError::new(ErrorCategory::Corrupt, "pdf.empty", "The file is empty."));
    }
    if len > MAX_INPUT_BYTES {
        return Err(PwError::new(
            ErrorCategory::UnsupportedOperation,
            "pdf.too_large",
            "The file is larger than the 2 GB limit.",
        ));
    }
    let mut head = vec![0u8; 1024.min(len as usize)];
    fs::File::open(path).and_then(|mut f| f.read_exact(&mut head)).map_err(|e| PwError::from_io(&e, "file"))?;
    if !head.windows(5).any(|w| w == b"%PDF-") {
        return Err(PwError::new(
            ErrorCategory::UnsupportedFormat,
            "pdf.not_pdf",
            "This file is not a PDF, even if its name ends in .pdf.",
        ));
    }
    Ok(len)
}

pub(crate) fn load<'p>(pdfium: &'p Pdfium, path: &Path, opts: &OpenOptions) -> PwResult<PdfDocument<'p>> {
    preflight(path)?;
    pdfium.load_pdf_from_file(path, opts.password).map_err(|e| map_pdfium_error(&e, "open"))
}

fn sha256_file(path: &Path) -> PwResult<String> {
    let mut f = fs::File::open(path).map_err(|e| PwError::from_io(&e, "file"))?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf).map_err(|e| PwError::from_io(&e, "file"))?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

fn rotation_degrees(r: Result<PdfPageRenderRotation, PdfiumError>) -> u32 {
    match r {
        Ok(PdfPageRenderRotation::Degrees90) => 90,
        Ok(PdfPageRenderRotation::Degrees180) => 180,
        Ok(PdfPageRenderRotation::Degrees270) => 270,
        _ => 0,
    }
}

fn version_string(v: PdfDocumentVersion) -> String {
    match v {
        PdfDocumentVersion::Unset => "unknown".into(),
        PdfDocumentVersion::Pdf1_0 => "1.0".into(),
        PdfDocumentVersion::Pdf1_1 => "1.1".into(),
        PdfDocumentVersion::Pdf1_2 => "1.2".into(),
        PdfDocumentVersion::Pdf1_3 => "1.3".into(),
        PdfDocumentVersion::Pdf1_4 => "1.4".into(),
        PdfDocumentVersion::Pdf1_5 => "1.5".into(),
        PdfDocumentVersion::Pdf1_6 => "1.6".into(),
        PdfDocumentVersion::Pdf1_7 => "1.7".into(),
        PdfDocumentVersion::Pdf2_0 => "2.0".into(),
        PdfDocumentVersion::Other(n) => format!("{}.{}", n / 10, n % 10),
    }
}

/// Open a document and report its structure. Never executes document actions or scripts.
pub fn open_document(path: &Path, opts: &OpenOptions) -> PwResult<DocumentInfo> {
    let pdfium = engine()?;
    let doc = load(&pdfium, path, opts)?;
    let file_bytes = fs::metadata(path).map_err(|e| PwError::from_io(&e, "file"))?.len();
    let mut warnings = Vec::new();

    let pages_api = doc.pages();
    let count = pages_api.len().max(0) as u32;
    let mut pages = Vec::with_capacity(count as usize);
    for i in 0..count {
        match pages_api.get(i as PdfPageIndex) {
            Ok(page) => {
                // PDFium reports the size after /Rotate; convert back to the unrotated size.
                let rotation = rotation_degrees(page.rotation());
                let (w, h) = (page.width().value, page.height().value);
                let (width_pt, height_pt) = if matches!(rotation, 90 | 270) { (h, w) } else { (w, h) };
                let origin = page.boundaries().bounding().map(|b| (b.bounds.left().value, b.bounds.bottom().value));
                let (origin_x, origin_y) = origin.unwrap_or((0.0, 0.0));
                pages.push(PageInfo {
                    index: i,
                    width_pt,
                    height_pt,
                    rotation,
                    label: page.label().map(str::to_string),
                    origin_x,
                    origin_y,
                })
            }
            Err(_) => {
                warnings.push(format!("Page {} could not be loaded.", i + 1));
                pages.push(PageInfo {
                    index: i,
                    width_pt: 0.0,
                    height_pt: 0.0,
                    rotation: 0,
                    label: None,
                    origin_x: 0.0,
                    origin_y: 0.0,
                });
            }
        }
    }

    let mut bookmarks = Vec::new();
    let mut truncated = false;
    if let Some(root) = doc.bookmarks().root() {
        collect_bookmarks(&root, 0, &mut bookmarks, &mut truncated);
    }

    let meta = doc.metadata();
    let tag =
        |t: PdfDocumentMetadataTagType| meta.get(t).map(|v| v.value().trim().to_string()).filter(|v| !v.is_empty());
    let form_type = doc.form().map(|f| f.form_type()).unwrap_or(PdfFormType::None);
    let has_xfa = matches!(form_type, PdfFormType::XfaFull | PdfFormType::XfaForeground);
    if has_xfa {
        warnings.push("This document contains an XFA form, which PageWeave cannot display or fill.".into());
    }
    // `Unprotected` means no security handler. Revisions 5/6 (AES-256) are unknown to the binding and
    // come back as an error, which still means the file is encrypted.
    let encrypted =
        !matches!(doc.permissions().security_handler_revision(), Ok(PdfSecurityHandlerRevision::Unprotected));

    Ok(DocumentInfo {
        page_count: count,
        file_bytes,
        sha256: sha256_file(path)?,
        pdf_version: version_string(doc.version()),
        title: tag(PdfDocumentMetadataTagType::Title),
        author: tag(PdfDocumentMetadataTagType::Author),
        subject: tag(PdfDocumentMetadataTagType::Subject),
        keywords: tag(PdfDocumentMetadataTagType::Keywords),
        creator: tag(PdfDocumentMetadataTagType::Creator),
        producer: tag(PdfDocumentMetadataTagType::Producer),
        encrypted,
        form_type: format!("{form_type:?}").to_lowercase(),
        has_xfa,
        signature_count: doc.signatures().len() as u32,
        attachment_count: doc.attachments().len() as u32,
        pages,
        bookmarks,
        bookmarks_truncated: truncated,
        warnings,
    })
}

pub(crate) fn collect_bookmarks(first: &PdfBookmark, depth: u32, out: &mut Vec<BookmarkInfo>, truncated: &mut bool) {
    // Outline trees can be cyclic in hostile files; bound total count and depth.
    if depth > 64 {
        *truncated = true;
        return;
    }
    // Walk first → next_sibling explicitly: `iter_siblings` excludes the starting node itself.
    let mut current = Some(first.clone());
    while let Some(bm) = current {
        if out.len() >= MAX_BOOKMARKS {
            *truncated = true;
            return;
        }
        out.push(BookmarkInfo {
            title: bm.title().unwrap_or_default(),
            page: bm.destination().and_then(|d| d.page_index().ok()).map(|p| p.max(0) as u32),
            depth,
        });
        if let Some(child) = bm.first_child() {
            collect_bookmarks(&child, depth + 1, out, truncated);
        }
        current = bm.next_sibling();
    }
}

/// Plain text per page (0-based page indices). Text order follows the engine's character order.
pub(crate) fn page_texts(path: &Path, opts: &OpenOptions, pages: &[u32]) -> PwResult<Vec<(u32, String)>> {
    let pdfium = engine()?;
    let doc = load(&pdfium, path, opts)?;
    let mut out = Vec::with_capacity(pages.len());
    for &i in pages {
        let page = doc.pages().get(i as PdfPageIndex).map_err(|e| map_pdfium_error(&e, "page"))?;
        // Not `PdfPageText::all()`: it bounds extraction by the page's *rotated* size, which drops
        // all text of a 90°/270° page. Text is in unrotated page space; take everything.
        let everything = PdfRect::new_from_values(-1.0e6, -1.0e6, 1.0e6, 1.0e6);
        let text = page.text().map_err(|e| map_pdfium_error(&e, "text"))?.inside_rect(everything);
        out.push((i, text));
    }
    Ok(out)
}

pub(crate) fn page_count(path: &Path, opts: &OpenOptions) -> PwResult<u32> {
    let pdfium = engine()?;
    let doc = load(&pdfium, path, opts)?;
    Ok(doc.pages().len().max(0) as u32)
}

/// Upper bound on text runs returned for one page (hostile files can contain millions).
pub const MAX_TEXT_SEGMENTS: usize = 20_000;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextSegment {
    pub text: String,
    /// Bounds in PDF points, unrotated page space, origin bottom-left.
    pub left: f32,
    pub bottom: f32,
    pub right: f32,
    pub top: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PageText {
    pub page: u32,
    pub segments: Vec<TextSegment>,
    pub truncated: bool,
}

/// Text runs of one page with geometry, in the engine's character order, for the selectable text layer.
pub fn page_text_segments(path: &Path, opts: &OpenOptions, page_index: u32) -> PwResult<PageText> {
    let pdfium = engine()?;
    let doc = load(&pdfium, path, opts)?;
    let count = doc.pages().len().max(0) as u32;
    if page_index >= count {
        return Err(PwError::invalid(
            "text.page_out_of_bounds",
            format!("Page {} does not exist; the document has {count} pages.", page_index + 1),
        ));
    }
    let page = doc.pages().get(page_index as PdfPageIndex).map_err(|e| map_pdfium_error(&e, "page"))?;
    let text = page.text().map_err(|e| map_pdfium_error(&e, "text"))?;
    let mut segments = Vec::new();
    let mut truncated = false;
    for seg in text.segments().iter() {
        if segments.len() >= MAX_TEXT_SEGMENTS {
            truncated = true;
            break;
        }
        let t = seg.text();
        if t.trim().is_empty() {
            continue;
        }
        let b = seg.bounds();
        segments.push(TextSegment {
            text: t,
            left: b.left().value,
            bottom: b.bottom().value,
            right: b.right().value,
            top: b.top().value,
        });
    }
    Ok(PageText { page: page_index, segments, truncated })
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchRect {
    pub left: f32,
    pub bottom: f32,
    pub right: f32,
    pub top: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchHit {
    /// 0-based page index.
    pub page: u32,
    /// Matched text as it appears in the document.
    pub text: String,
    /// Match geometry in PDF points (page user space, origin bottom-left).
    pub rects: Vec<SearchRect>,
}

/// Search every page. Results are capped at [`MAX_SEARCH_HITS`]; the bool reports truncation.
pub fn search(
    path: &Path,
    opts: &OpenOptions,
    query: &str,
    match_case: bool,
    whole_word: bool,
    cancelled: &dyn Fn() -> bool,
) -> PwResult<(Vec<SearchHit>, bool)> {
    let query = query.trim();
    if query.is_empty() {
        return Err(PwError::invalid("search.empty", "Enter text to search for."));
    }
    let pdfium = engine()?;
    let doc = load(&pdfium, path, opts)?;
    let options = PdfSearchOptions::new().match_case(match_case).match_whole_word(whole_word);
    let mut hits = Vec::new();
    for (i, page) in doc.pages().iter().enumerate() {
        if cancelled() {
            return Err(PwError::cancelled());
        }
        let text = page.text().map_err(|e| map_pdfium_error(&e, "text"))?;
        let s = text.search(query, &options).map_err(|e| map_pdfium_error(&e, "search"))?;
        for segments in s.iter(PdfSearchDirection::SearchForward) {
            if hits.len() >= MAX_SEARCH_HITS {
                return Ok((hits, true));
            }
            let mut matched = String::new();
            let mut rects = Vec::new();
            for seg in segments.iter() {
                matched.push_str(&seg.text());
                let b = seg.bounds();
                rects.push(SearchRect {
                    left: b.left().value,
                    bottom: b.bottom().value,
                    right: b.right().value,
                    top: b.top().value,
                });
            }
            hits.push(SearchHit { page: i as u32, text: matched, rects });
        }
    }
    Ok((hits, false))
}

/// Render one page (0-based) at `scale` (1.0 = 72 dpi) to PNG bytes. Annotations and form
/// appearances are drawn; nothing is executed.
pub fn render_page_png(path: &Path, opts: &OpenOptions, page_index: u32, scale: f32) -> PwResult<(Vec<u8>, u32, u32)> {
    if !scale.is_finite() || scale <= 0.0 {
        return Err(PwError::invalid("render.scale", "Zoom must be greater than zero."));
    }
    let pdfium = engine()?;
    let doc = load(&pdfium, path, opts)?;
    let count = doc.pages().len().max(0) as u32;
    if page_index >= count {
        return Err(PwError::invalid(
            "render.page_out_of_bounds",
            format!("Page {} does not exist; the document has {count} pages.", page_index + 1),
        ));
    }
    let page = doc.pages().get(page_index as PdfPageIndex).map_err(|e| map_pdfium_error(&e, "page"))?;
    // PDFium's width/height are already the displayed (rotated) size, which is what gets rendered.
    let (w_pt, h_pt) = (page.width().value, page.height().value);
    let w = (w_pt * scale).ceil().max(1.0) as u64;
    let h = (h_pt * scale).ceil().max(1.0) as u64;
    if w * h > MAX_RENDER_PIXELS || w > i32::MAX as u64 || h > i32::MAX as u64 {
        return Err(PwError::invalid("render.too_large", "That zoom level is too large for this page."));
    }
    let config = PdfRenderConfig::new().scale_page_by_factor(scale).render_form_data(true).render_annotations(true);
    let bitmap = page.render_with_config(&config).map_err(|e| map_pdfium_error(&e, "render"))?;
    let img = bitmap.as_image().map_err(|e| map_pdfium_error(&e, "render"))?;
    let (iw, ih) = (img.width(), img.height());
    let mut png = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .map_err(|_| PwError::internal("render.encode", "Could not encode the rendered page."))?;
    Ok((png, iw, ih))
}
