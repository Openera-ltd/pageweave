//! Page-level document operations. Sources are never modified: every operation builds a new
//! document, saves it to memory, re-opens the staged output to verify it, and commits atomically.

use crate::document::{load, OpenOptions};
use crate::{engine, map_pdfium_error};
use pdfium_render::prelude::*;
use pw_core::ops::BookmarkSpec;
use pw_core::output::{write_atomic_bytes, CollisionPolicy, CommittedOutput};
use pw_core::range::parse_page_ranges;
use pw_core::{ErrorCategory, PwError, PwResult};
use std::path::{Path, PathBuf};

/// Expected properties of an output, checked after it is written and re-parsed.
pub(crate) struct Expect<'a> {
    pub(crate) page_count: u32,
    /// Password of the source, used if the output kept the source's encryption.
    pub(crate) password: Option<&'a str>,
    /// Expected intrinsic rotation per output page, when known.
    pub(crate) rotations: Option<Vec<u32>>,
}

/// Re-open and check the staged output. Returns whether the output is encrypted.
pub(crate) fn verify_output(pdfium: &Pdfium, staged: &Path, expect: &Expect) -> PwResult<bool> {
    let fail = |what: &str| {
        PwError::new(
            ErrorCategory::ValidationFailed,
            "output.verify",
            format!("The output failed verification ({what}) and was not saved."),
        )
    };
    let (doc, encrypted) = match pdfium.load_pdf_from_file(staged, None) {
        Ok(d) => (d, false),
        Err(PdfiumError::PdfiumLibraryInternalError(PdfiumInternalError::PasswordError))
            if expect.password.is_some() =>
        {
            (pdfium.load_pdf_from_file(staged, expect.password).map_err(|_| fail("could not be reopened"))?, true)
        }
        Err(_) => return Err(fail("could not be reopened")),
    };
    let n = doc.pages().len().max(0) as u32;
    if n != expect.page_count {
        return Err(fail("page count mismatch"));
    }
    if let Some(rots) = &expect.rotations {
        for (i, want) in rots.iter().enumerate() {
            let page = doc.pages().get(i as PdfPageIndex).map_err(|_| fail("page unreadable"))?;
            let got = match page.rotation() {
                Ok(PdfPageRenderRotation::Degrees90) => 90,
                Ok(PdfPageRenderRotation::Degrees180) => 180,
                Ok(PdfPageRenderRotation::Degrees270) => 270,
                _ => 0,
            };
            if got != *want {
                return Err(fail("rotation mismatch"));
            }
        }
    }
    Ok(encrypted)
}

pub(crate) const PROTECTION_REMOVED: &str =
    "The output is not password protected, although the original was. Protect it again before sharing if needed.";

pub(crate) fn save_bytes(doc: &PdfDocument) -> PwResult<Vec<u8>> {
    doc.save_to_bytes().map_err(|e| map_pdfium_error(&e, "save"))
}

/// Copy `indices` (0-based, in order) from `src` into a new document, grouping consecutive runs.
fn copy_pages<'p>(pdfium: &'p Pdfium, src: &PdfDocument, indices: &[u32]) -> PwResult<PdfDocument<'p>> {
    let mut out = pdfium.create_new_pdf().map_err(|e| map_pdfium_error(&e, "create"))?;
    let mut i = 0;
    while i < indices.len() {
        let start = indices[i];
        let mut end = start;
        while i + 1 < indices.len() && indices[i + 1] == end + 1 {
            end += 1;
            i += 1;
        }
        let dest = out.pages().len();
        out.pages_mut()
            .copy_page_range_from_document(src, (start as PdfPageIndex)..=(end as PdfPageIndex), dest)
            .map_err(|e| map_pdfium_error(&e, "copy_pages"))?;
        i += 1;
    }
    Ok(out)
}

/// Commit verified bytes. Returns the committed output and whether it is encrypted.
fn commit(
    pdfium: &Pdfium,
    bytes: &[u8],
    output: &Path,
    policy: CollisionPolicy,
    protected: &[PathBuf],
    expect: &Expect,
) -> PwResult<(CommittedOutput, bool)> {
    let mut encrypted = false;
    let c = write_atomic_bytes(output, policy, protected, bytes, |staged| {
        encrypted = verify_output(pdfium, staged, expect)?;
        Ok(())
    })?;
    Ok((c, encrypted))
}

pub(crate) fn source_encrypted(doc: &PdfDocument) -> bool {
    !matches!(doc.permissions().security_handler_revision(), Ok(PdfSecurityHandlerRevision::Unprotected))
}

/// Result of a single-output operation: committed file, page count, warnings.
pub type EditOutcome = (CommittedOutput, u32, Vec<String>);
/// Result of a split: each committed part with its page count, plus warnings.
pub type SplitOutcome = (Vec<(CommittedOutput, u32)>, Vec<String>);

pub(crate) fn protection_warnings(src_encrypted: bool, out_encrypted: bool) -> Vec<String> {
    if src_encrypted && !out_encrypted {
        vec![PROTECTION_REMOVED.to_string()]
    } else {
        Vec::new()
    }
}

pub fn extract_pages(
    input: &Path,
    opts: &OpenOptions,
    pages: &str,
    output: &Path,
    policy: CollisionPolicy,
) -> PwResult<EditOutcome> {
    let pdfium = engine()?;
    let src = load(&pdfium, input, opts)?;
    let count = src.pages().len().max(0) as u32;
    let indices = parse_page_ranges(pages, count)?;
    let out = copy_pages(&pdfium, &src, &indices)?;
    let bytes = save_bytes(&out)?;
    let n = indices.len() as u32;
    let expect = Expect { page_count: n, password: opts.password, rotations: None };
    let (c, enc) = commit(&pdfium, &bytes, output, policy, &[input.to_path_buf()], &expect)?;
    Ok((c, n, protection_warnings(source_encrypted(&src), enc)))
}

pub fn delete_pages(
    input: &Path,
    opts: &OpenOptions,
    pages: &str,
    output: &Path,
    policy: CollisionPolicy,
) -> PwResult<EditOutcome> {
    let pdfium = engine()?;
    let src = load(&pdfium, input, opts)?;
    let count = src.pages().len().max(0) as u32;
    let mut remove = parse_page_ranges(pages, count)?;
    remove.sort_unstable();
    if remove.len() as u32 >= count {
        return Err(PwError::invalid("delete.all_pages", "A PDF must keep at least one page."));
    }
    let keep: Vec<u32> = (0..count).filter(|i| remove.binary_search(i).is_err()).collect();
    let out = copy_pages(&pdfium, &src, &keep)?;
    let bytes = save_bytes(&out)?;
    let n = keep.len() as u32;
    let expect = Expect { page_count: n, password: opts.password, rotations: None };
    let (c, enc) = commit(&pdfium, &bytes, output, policy, &[input.to_path_buf()], &expect)?;
    Ok((c, n, protection_warnings(source_encrypted(&src), enc)))
}

fn to_rotation(deg: u32) -> PdfPageRenderRotation {
    match deg % 360 {
        90 => PdfPageRenderRotation::Degrees90,
        180 => PdfPageRenderRotation::Degrees180,
        270 => PdfPageRenderRotation::Degrees270,
        _ => PdfPageRenderRotation::None,
    }
}

/// Rotate selected pages (all pages when `pages` is None) by `degrees` (±90/180/270) relative to
/// their current rotation.
pub fn rotate_pages(
    input: &Path,
    opts: &OpenOptions,
    pages: Option<&str>,
    degrees: i32,
    output: &Path,
    policy: CollisionPolicy,
) -> PwResult<EditOutcome> {
    if degrees % 90 != 0 || degrees == 0 || degrees.abs() > 270 {
        return Err(PwError::invalid("rotate.degrees", "Rotate by 90, 180 or 270 degrees."));
    }
    let delta = degrees.rem_euclid(360) as u32;
    let pdfium = engine()?;
    let doc = load(&pdfium, input, opts)?;
    let count = doc.pages().len().max(0) as u32;
    let selected: Vec<u32> = match pages {
        Some(spec) => parse_page_ranges(spec, count)?,
        None => (0..count).collect(),
    };
    let mut rotations = Vec::with_capacity(count as usize);
    for i in 0..count {
        let page = doc.pages().get(i as PdfPageIndex).map_err(|e| map_pdfium_error(&e, "page"))?;
        let current = match page.rotation() {
            Ok(PdfPageRenderRotation::Degrees90) => 90,
            Ok(PdfPageRenderRotation::Degrees180) => 180,
            Ok(PdfPageRenderRotation::Degrees270) => 270,
            _ => 0,
        };
        rotations.push(current);
    }
    for &i in &selected {
        let mut page = doc.pages().get(i as PdfPageIndex).map_err(|e| map_pdfium_error(&e, "page"))?;
        let new = (rotations[i as usize] + delta) % 360;
        page.set_rotation(to_rotation(new));
        rotations[i as usize] = new;
    }
    let bytes = save_bytes(&doc)?;
    let expect = Expect { page_count: count, password: opts.password, rotations: Some(rotations) };
    let (c, enc) = commit(&pdfium, &bytes, output, policy, &[input.to_path_buf()], &expect)?;
    Ok((c, count, protection_warnings(source_encrypted(&doc), enc)))
}

/// Combine `inputs` in order. Bookmarks and document metadata of the inputs are not carried over
/// by the engine's page import; this is reported as a warning.
/// The merged document with `bookmarks` as its outline, checked: PDFium reads back exactly the
/// titles, pages and nesting written.
fn with_outline(pdfium: &Pdfium, bytes: Vec<u8>, bookmarks: &[BookmarkSpec]) -> PwResult<Vec<u8>> {
    let fail = || PwError::internal("merge.bookmarks", "The combined file's bookmarks could not be written.");
    let mut doc = lopdf::Document::load_mem(&bytes).map_err(|_| fail())?;
    crate::outline::write_outline(&mut doc, bookmarks)?;
    doc.prune_objects();
    let mut out = Vec::new();
    doc.save_to(&mut out).map_err(|_| fail())?;
    let back = pdfium.load_pdf_from_byte_slice(&out, None).map_err(|_| fail())?;
    let mut got = Vec::new();
    let mut cut = false;
    if let Some(root) = back.bookmarks().root() {
        crate::document::collect_bookmarks(&root, 0, &mut got, &mut cut);
    }
    let same = got.len() == bookmarks.len()
        && got
            .iter()
            .zip(bookmarks)
            .all(|(g, b)| g.title.trim() == b.title && g.page == Some(b.page) && g.depth == b.depth);
    if !same {
        return Err(fail());
    }
    drop(back);
    Ok(out)
}

/// The first file's title, author, subject and keywords (those it has), for the combined file.
fn properties_of(doc: &PdfDocument) -> Vec<(&'static str, String)> {
    let tags = [
        ("Title", PdfDocumentMetadataTagType::Title),
        ("Author", PdfDocumentMetadataTagType::Author),
        ("Subject", PdfDocumentMetadataTagType::Subject),
        ("Keywords", PdfDocumentMetadataTagType::Keywords),
    ];
    tags.into_iter()
        .filter_map(|(k, t)| {
            let v = doc.metadata().get(t)?.value().trim().to_string();
            (!v.is_empty()).then_some((k, v))
        })
        .collect()
}

/// `bytes` with `props` in a new document information dictionary, read back with PDFium.
fn with_properties(pdfium: &Pdfium, bytes: Vec<u8>, props: &[(&'static str, String)]) -> PwResult<Vec<u8>> {
    let fail = || PwError::internal("merge.properties", "The combined file's properties could not be written.");
    let mut doc = lopdf::Document::load_mem(&bytes).map_err(|_| fail())?;
    let mut info = lopdf::Dictionary::new();
    for (k, v) in props {
        info.set(*k, lopdf::text_string(v));
    }
    let id = doc.add_object(info);
    doc.trailer.set("Info", id);
    let mut out = Vec::new();
    doc.save_to(&mut out).map_err(|_| fail())?;
    let back = pdfium.load_pdf_from_byte_slice(&out, None).map_err(|_| fail())?;
    let title = back.metadata().get(PdfDocumentMetadataTagType::Title).map(|t| t.value().trim().to_string());
    if props.iter().any(|(k, v)| *k == "Title" && title.as_deref() != Some(v.as_str())) {
        return Err(fail());
    }
    drop(back);
    Ok(out)
}

pub fn merge(
    inputs: &[PathBuf],
    opts: &OpenOptions,
    output: &Path,
    policy: CollisionPolicy,
    cancelled: &dyn Fn() -> bool,
    progress: &dyn Fn(u64, u64),
) -> PwResult<EditOutcome> {
    if inputs.len() < 2 {
        return Err(PwError::invalid("merge.too_few", "Choose at least two files to combine."));
    }
    let pdfium = engine()?;
    let mut out = pdfium.create_new_pdf().map_err(|e| map_pdfium_error(&e, "create"))?;
    let mut warnings = Vec::new();
    let mut any_encrypted = false;
    // Each file's bookmarks, pointed at its pages in the combined document. A bookmark without a
    // page in its file (an action, another document) opens that file's first page.
    let mut bookmarks: Vec<BookmarkSpec> = Vec::new();
    let mut props = Vec::new();
    let mut truncated = false;
    let mut offset = 0u32;
    for (n, input) in inputs.iter().enumerate() {
        if cancelled() {
            return Err(PwError::cancelled());
        }
        let named = |k: usize, e: PwError| {
            let name = inputs[k].file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
            PwError { message: format!("File {} of {} ({name}): {}", k + 1, inputs.len(), e.message), ..e }
        };
        let src = match load(&pdfium, input, opts) {
            Ok(d) => d,
            Err(e) => {
                // Say which of the other files cannot be opened either, not one per attempt.
                let first = named(n, e);
                let others: Vec<String> = (n + 1..inputs.len())
                    .filter_map(|k| load(&pdfium, &inputs[k], opts).err().map(|e| named(k, e).message))
                    .collect();
                if others.is_empty() {
                    return Err(first);
                }
                let count = others.len() + 1;
                let message = std::iter::once(first.message.clone()).chain(others).collect::<Vec<_>>().join(" ");
                return Err(PwError { message: format!("{count} files can't be combined. {message}"), ..first });
            }
        };
        if let Some(root) = src.bookmarks().root() {
            let mut found = Vec::new();
            crate::document::collect_bookmarks(&root, 0, &mut found, &mut truncated);
            bookmarks.extend(found.into_iter().filter(|b| !b.title.trim().is_empty()).map(|b| BookmarkSpec {
                title: b.title.trim().to_string(),
                page: offset + b.page.unwrap_or(0),
                depth: b.depth,
            }));
        }
        if n == 0 {
            props = properties_of(&src);
        }
        any_encrypted |= source_encrypted(&src);
        offset += src.pages().len().max(0) as u32;
        out.pages_mut().append(&src).map_err(|e| map_pdfium_error(&e, "append"))?;
        progress(n as u64 + 1, inputs.len() as u64);
    }
    let total = out.pages().len().max(0) as u32;
    let mut bytes = save_bytes(&out)?;
    if !bookmarks.is_empty() {
        bytes = with_outline(&pdfium, bytes, &bookmarks)?;
        warnings.push("Bookmarks of the combined files are kept; each opens its whole page.".into());
        if truncated {
            warnings.push("Some files had more bookmarks than PageWeave reads; the rest were left out.".into());
        }
    }
    if !props.is_empty() {
        bytes = with_properties(&pdfium, bytes, &props)?;
        warnings.push("The combined file has the first file's title, author, subject and keywords.".into());
    }
    let expect = Expect { page_count: total, password: opts.password, rotations: None };
    let (c, enc) = commit(&pdfium, &bytes, output, policy, inputs, &expect)?;
    warnings.extend(protection_warnings(any_encrypted, enc));
    Ok((c, total, warnings))
}

pub(crate) fn rotation_of(page: &PdfPage) -> u32 {
    match page.rotation() {
        Ok(PdfPageRenderRotation::Degrees90) => 90,
        Ok(PdfPageRenderRotation::Degrees180) => 180,
        Ok(PdfPageRenderRotation::Degrees270) => 270,
        _ => 0,
    }
}

/// Build a new document from `pages` (each a page of the input or of an extra file, with an extra
/// clockwise turn), in order. Covers reorder, duplicate, drop, rotate and insert in one write. The
/// output is reopened and its page count and every page's rotation are verified before commit.
pub fn organize_pages(
    input: &Path,
    opts: &OpenOptions,
    extra_inputs: &[PathBuf],
    pages: &[pw_core::ops::PageRef],
    output: &Path,
    policy: CollisionPolicy,
    cancelled: &dyn Fn() -> bool,
) -> PwResult<EditOutcome> {
    let pdfium = engine()?;
    let mut sources = vec![load(&pdfium, input, opts)?];
    for (n, extra) in extra_inputs.iter().enumerate() {
        let doc = load(&pdfium, extra, &OpenOptions { password: None }).map_err(|e| PwError {
            message: format!("Inserted file {} of {}: {}", n + 1, extra_inputs.len(), e.message),
            ..e
        })?;
        sources.push(doc);
    }
    let counts: Vec<u32> = sources.iter().map(|d| d.pages().len().max(0) as u32).collect();
    for p in pages {
        let have = counts[p.source as usize];
        if p.page >= have {
            return Err(PwError::invalid(
                "organize.page",
                format!("Page {} does not exist; that document has {have} pages.", p.page + 1),
            ));
        }
    }
    let mut out = pdfium.create_new_pdf().map_err(|e| map_pdfium_error(&e, "create"))?;
    // Copy consecutive runs (same source, ascending pages) in one call: much faster than page by
    // page for typical reorders (measured: 2,000 scattered single copies ≈1.4 s).
    let mut i = 0;
    while i < pages.len() {
        if cancelled() {
            return Err(PwError::cancelled());
        }
        let first = pages[i];
        if first.blank {
            let like = sources[first.source as usize]
                .pages()
                .get(first.page as PdfPageIndex)
                .map_err(|e| map_pdfium_error(&e, "page"))?;
            let size = PdfPagePaperSize::Custom(like.width(), like.height());
            // width()/height() are as displayed; undo the turn so the blank page shows the same way.
            let size = if matches!(rotation_of(&like), 90 | 270) {
                PdfPagePaperSize::Custom(like.height(), like.width())
            } else {
                size
            };
            out.pages_mut().create_page_at_end(size).map_err(|e| map_pdfium_error(&e, "blank_page"))?;
            i += 1;
            continue;
        }
        let mut end = i;
        while end + 1 < pages.len()
            && !pages[end + 1].blank
            && pages[end + 1].source == first.source
            && pages[end + 1].page == pages[end].page + 1
        {
            end += 1;
        }
        let dest = out.pages().len();
        out.pages_mut()
            .copy_page_range_from_document(
                &sources[first.source as usize],
                (first.page as PdfPageIndex)..=(pages[end].page as PdfPageIndex),
                dest,
            )
            .map_err(|e| map_pdfium_error(&e, "copy_pages"))?;
        i = end + 1;
    }
    let mut rotations = Vec::with_capacity(pages.len());
    for (i, p) in pages.iter().enumerate() {
        let mut page = out.pages().get(i as PdfPageIndex).map_err(|e| map_pdfium_error(&e, "page"))?;
        // A blank page takes the turn of the page it is sized like.
        let current = if p.blank {
            let like = sources[p.source as usize]
                .pages()
                .get(p.page as PdfPageIndex)
                .map_err(|e| map_pdfium_error(&e, "page"))?;
            let r = rotation_of(&like);
            if r != 0 {
                page.set_rotation(to_rotation(r));
            }
            r
        } else {
            rotation_of(&page)
        };
        let new = (current as i32 + p.rotate).rem_euclid(360) as u32;
        if new != current {
            page.set_rotation(to_rotation(new));
        }
        rotations.push(new);
    }
    let any_encrypted = sources.iter().any(source_encrypted);
    let bytes = save_bytes(&out)?;
    let total = pages.len() as u32;
    let protected: Vec<PathBuf> = std::iter::once(input.to_path_buf()).chain(extra_inputs.iter().cloned()).collect();
    let expect = Expect { page_count: total, password: opts.password, rotations: Some(rotations) };
    let (c, enc) = commit(&pdfium, &bytes, output, policy, &protected, &expect)?;
    let mut warnings = protection_warnings(any_encrypted, enc);
    if sources.iter().any(|d| d.bookmarks().root().is_some()) {
        warnings.push("Bookmarks are not included in the organized file.".into());
    }
    Ok((c, total, warnings))
}

/// Map a point given in displayed-page space (origin bottom-left of the page as shown, after its
/// clockwise /Rotate) to unrotated page space. `w`, `h` are the unrotated page size.
pub(crate) fn display_to_page(rot: u32, w: f32, h: f32, x: f32, y: f32) -> (f32, f32) {
    match rot {
        90 => (w - y, x),
        180 => (w - x, h - y),
        270 => (y, h - x),
        _ => (x, y),
    }
}

/// Stamp text (page numbers, headers/footers, Bates-style numbers) onto a copy. Each stamp is
/// placed in the page's displayed orientation, so it reads upright on rotated pages too. The
/// output is reopened and every stamped page's text is checked for its expanded stamps.
#[allow(clippy::too_many_arguments)]
pub fn stamp_text(
    input: &Path,
    opts: &OpenOptions,
    pages: Option<&str>,
    stamps: &[pw_core::ops::StampSpec],
    start: u64,
    digits: u8,
    output: &Path,
    policy: CollisionPolicy,
) -> PwResult<EditOutcome> {
    use pw_core::ops::{stamp_text_for, StampPosition};
    let pdfium = engine()?;
    let mut doc = load(&pdfium, input, opts)?;
    let total = doc.pages().len().max(0) as u32;
    let indices = match pages {
        Some(spec) => parse_page_ranges(spec, total)?,
        None => (0..total).collect(),
    };
    let font = doc.fonts_mut().helvetica();
    let err = |e: PdfiumError| map_pdfium_error(&e, "stamp");
    let mut expected: Vec<(u32, Vec<String>)> = Vec::with_capacity(indices.len());
    for (k, &i) in indices.iter().enumerate() {
        let number = start + k as u64;
        let mut page = doc.pages().get(i as PdfPageIndex).map_err(err)?;
        let rot = rotation_of(&page);
        let (dw, dh) = (page.width().value, page.height().value);
        let (w, h) = if matches!(rot, 90 | 270) { (dh, dw) } else { (dw, dh) };
        let mut texts = Vec::new();
        for spec in stamps {
            let text = stamp_text_for(&spec.text, number, digits, total);
            let mut obj = PdfPageTextObject::new(&doc, &text, font, PdfPoints::new(spec.font_size)).map_err(err)?;
            obj.set_fill_color(parse_color(&spec.color)?).map_err(err)?;
            let tw = obj.width().map_err(err)?.value;
            // Anchor (baseline start) in displayed space.
            let x = match spec.position {
                StampPosition::TopLeft | StampPosition::BottomLeft => spec.margin,
                StampPosition::TopCenter | StampPosition::BottomCenter => (dw - tw) / 2.0,
                StampPosition::TopRight | StampPosition::BottomRight => dw - spec.margin - tw,
            };
            let y = match spec.position {
                StampPosition::TopLeft | StampPosition::TopCenter | StampPosition::TopRight => {
                    dh - spec.margin - spec.font_size
                }
                _ => spec.margin,
            };
            let (px, py) = display_to_page(rot, w, h, x.max(0.0), y.max(0.0));
            // Turn with the page so the text reads upright as displayed, then move into place.
            obj.rotate_counter_clockwise_degrees(rot as f32).map_err(err)?;
            obj.translate(PdfPoints::new(px), PdfPoints::new(py)).map_err(err)?;
            page.objects_mut().add_text_object(obj).map_err(err)?;
            texts.push(text);
        }
        expected.push((i, texts));
    }
    let bytes = save_bytes(&doc)?;
    let src_encrypted = source_encrypted(&doc);
    let protected = [input.to_path_buf()];
    let mut encrypted = false;
    let c = write_atomic_bytes(output, policy, &protected, &bytes, |staged| {
        encrypted =
            verify_output(&pdfium, staged, &Expect { page_count: total, password: opts.password, rotations: None })?;
        let reopened =
            pdfium.load_pdf_from_file(staged, if encrypted { opts.password } else { None }).map_err(|_| {
                PwError::new(ErrorCategory::ValidationFailed, "output.verify", "The output could not be reopened.")
            })?;
        let everything = PdfRect::new_from_values(-1.0e6, -1.0e6, 1.0e6, 1.0e6);
        for (i, texts) in &expected {
            let page = reopened.pages().get(*i as PdfPageIndex).map_err(|e| map_pdfium_error(&e, "page"))?;
            let got = page.text().map_err(|e| map_pdfium_error(&e, "text"))?.inside_rect(everything);
            let squash = |t: &str| t.chars().filter(|c| !c.is_whitespace()).collect::<String>();
            if texts.iter().any(|t| !squash(&got).contains(&squash(t))) {
                return Err(PwError::new(
                    ErrorCategory::ValidationFailed,
                    "output.verify",
                    format!("The output failed verification (page {} is missing its stamp).", i + 1),
                ));
            }
        }
        Ok(())
    })?;
    Ok((c, total, protection_warnings(src_encrypted, encrypted)))
}

/// Save pages as PNG files `<stem>_p<NNN>.png` in `output_dir` at `dpi`. Every file is decoded
/// again and its pixel size checked before it is committed. All or nothing: if any page fails or the
/// job is cancelled, files already written by this call are removed.
#[allow(clippy::too_many_arguments)]
pub fn export_images(
    input: &Path,
    opts: &OpenOptions,
    pages: Option<&str>,
    dpi: u32,
    output_dir: &Path,
    policy: CollisionPolicy,
    cancelled: &dyn Fn() -> bool,
    progress: &dyn Fn(u64, u64),
) -> PwResult<(Vec<CommittedOutput>, Vec<String>)> {
    let total = crate::document::page_count(input, opts)?;
    let indices = match pages {
        Some(spec) => parse_page_ranges(spec, total)?,
        None => (0..total).collect(),
    };
    let stem = input.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "page".into());
    let width = total.to_string().len().max(3);
    let scale = dpi as f32 / 72.0;
    let mut written: Vec<CommittedOutput> = Vec::new();
    let result = (|| -> PwResult<()> {
        for (k, &i) in indices.iter().enumerate() {
            if cancelled() {
                return Err(PwError::cancelled());
            }
            // One page per engine operation, so page rendering in the app can interleave.
            let (png, w, h) = crate::document::render_page_png(input, opts, i, scale)?;
            let name = output_dir.join(format!("{stem}_p{:0width$}.png", i + 1));
            let c = write_atomic_bytes(&name, policy, &[input.to_path_buf()], &png, |staged| {
                let back = std::fs::read(staged).map_err(|e| PwError::from_io(&e, "output file"))?;
                match image::load_from_memory_with_format(&back, image::ImageFormat::Png) {
                    Ok(img) if img.width() == w && img.height() == h => Ok(()),
                    _ => Err(PwError::new(
                        ErrorCategory::ValidationFailed,
                        "output.verify",
                        "An exported image failed verification and was not saved.",
                    )),
                }
            })?;
            written.push(c);
            progress(k as u64 + 1, indices.len() as u64);
        }
        Ok(())
    })();
    if let Err(e) = result {
        for c in &written {
            let _ = std::fs::remove_file(&c.path);
        }
        return Err(e);
    }
    Ok((written, Vec::new()))
}

/// Split into several files. All parts are built in memory first; if committing any part fails,
/// parts already written by this call are removed so the result is all-or-nothing.
pub fn split(
    input: &Path,
    opts: &OpenOptions,
    split: &pw_core::ops::SplitMode,
    output_dir: &Path,
    policy: CollisionPolicy,
    cancelled: &dyn Fn() -> bool,
) -> PwResult<SplitOutcome> {
    use pw_core::ops::SplitMode;
    let pdfium = engine()?;
    let src = load(&pdfium, input, opts)?;
    let count = src.pages().len().max(0) as u32;
    let stem = input.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "document".into());
    let mut warnings = Vec::new();
    let parts: Vec<SplitPart> = match split {
        SplitMode::EveryN { n } => {
            if *n == 0 {
                return Err(PwError::invalid("split.n_zero", "Split size must be at least 1 page."));
            }
            (0..count).collect::<Vec<_>>().chunks(*n as usize).map(|c| (c.to_vec(), None, None)).collect()
        }
        SplitMode::Ranges { ranges } => {
            if ranges.is_empty() {
                return Err(PwError::invalid("split.no_ranges", "Enter at least one page range."));
            }
            ranges
                .iter()
                .enumerate()
                .map(|(i, r)| {
                    parse_page_ranges(r, count)
                        .map(|g| (g, None, None))
                        .map_err(|e| PwError { message: format!("Part {}: {}", i + 1, e.message), ..e })
                })
                .collect::<PwResult<_>>()?
        }
        SplitMode::Bookmarks { level } => {
            bookmark_parts(&src, count, *level)?.into_iter().map(|(g, t)| (g, Some(t), None)).collect()
        }
        SplitMode::MaxSize { max_bytes } => size_parts(&pdfium, &src, count, *max_bytes, cancelled, &mut warnings)?
            .into_iter()
            .map(|(g, b)| (g, None, Some(b)))
            .collect(),
    };
    if parts.len() > 10_000 {
        return Err(PwError::invalid("split.too_many_parts", "That would create more than 10,000 files."));
    }
    let width = parts.len().to_string().len().max(2);
    let mut built = Vec::with_capacity(parts.len());
    for (i, (g, title, bytes)) in parts.into_iter().enumerate() {
        if cancelled() {
            return Err(PwError::cancelled());
        }
        let bytes = match bytes {
            Some(b) => b,
            None => save_bytes(&copy_pages(&pdfium, &src, &g)?)?,
        };
        let name = match title {
            Some(t) => format!("{stem}_{:0width$}_{}.pdf", i + 1, part_title(&t)),
            None => format!("{stem}_part{:0width$}.pdf", i + 1),
        };
        built.push((bytes, g.len() as u32, name));
    }
    let src_encrypted = source_encrypted(&src);
    let mut any_unprotected = false;
    let mut committed: Vec<(CommittedOutput, u32)> = Vec::new();
    for (i, (bytes, n, name)) in built.iter().enumerate() {
        let name = output_dir.join(name);
        let result = if cancelled() {
            Err(PwError::cancelled())
        } else {
            let expect = Expect { page_count: *n, password: opts.password, rotations: None };
            commit(&pdfium, bytes, &name, policy, &[input.to_path_buf()], &expect)
        };
        match result {
            Ok((c, enc)) => {
                any_unprotected |= !enc;
                committed.push((c, *n));
            }
            Err(e) => {
                for (c, _) in &committed {
                    let _ = std::fs::remove_file(&c.path);
                }
                return Err(PwError {
                    message: format!("Part {} could not be saved: {} No parts were kept.", i + 1, e.message),
                    ..e
                });
            }
        }
    }
    warnings.extend(protection_warnings(src_encrypted, !any_unprotected));
    Ok((committed, warnings))
}

/// One part to write: its pages, the bookmark title for its name, and its saved bytes when they
/// were already built (split by size measures them).
type SplitPart = (Vec<u32>, Option<String>, Option<Vec<u8>>);

/// A bookmark title as part of a file name: characters Windows forbids replaced, at most 60 characters.
fn part_title(t: &str) -> String {
    let s: String =
        t.chars()
            .map(|c| {
                if c.is_control() || matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*' | '/' | '\\') {
                    '_'
                } else {
                    c
                }
            })
            .collect();
    let s: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
    let s: String = s.trim_matches('.').chars().take(60).collect();
    let s = s.trim_end_matches(['.', ' ']).to_string();
    if s.is_empty() {
        "section".into()
    } else {
        s
    }
}

/// Page groups that start at each bookmark of `level` (1-based), with the bookmark's title.
fn bookmark_parts(src: &PdfDocument, count: u32, level: u32) -> PwResult<Vec<(Vec<u32>, String)>> {
    let mut all = Vec::new();
    let mut truncated = false;
    if let Some(root) = src.bookmarks().root() {
        crate::document::collect_bookmarks(&root, 0, &mut all, &mut truncated);
    }
    let mut starts: Vec<(u32, String)> = all
        .into_iter()
        .filter(|b| b.depth + 1 == level)
        .filter_map(|b| b.page.filter(|p| *p < count).map(|p| (p, b.title)))
        .collect();
    // Document order; bookmarks on the same page as an earlier one start no new part.
    starts.sort_by_key(|(p, _)| *p);
    starts.dedup_by_key(|(p, _)| *p);
    if starts.is_empty() {
        let msg = if level == 1 {
            "This document has no bookmarks that point to a page.".to_string()
        } else {
            format!("This document has no level {level} bookmarks that point to a page.")
        };
        return Err(PwError::invalid("split.no_bookmarks", msg));
    }
    let mut out = Vec::with_capacity(starts.len());
    for (i, (p, title)) in starts.iter().enumerate() {
        let from = if i == 0 { 0 } else { *p };
        let to = starts.get(i + 1).map(|(n, _)| *n).unwrap_or(count);
        out.push(((from..to).collect(), title.clone()));
    }
    Ok(out)
}

/// Consecutive page groups whose saved size stays within `max_bytes`, with each group's bytes.
/// Sizes are measured on the saved part (pages share fonts and images, so per-page sizes don't
/// add up); the largest fitting group is found by doubling, then bisection.
fn size_parts(
    pdfium: &Pdfium,
    src: &PdfDocument,
    count: u32,
    max_bytes: u64,
    cancelled: &dyn Fn() -> bool,
    warnings: &mut Vec<String>,
) -> PwResult<Vec<(Vec<u32>, Vec<u8>)>> {
    let build = |from: u32, n: u32| -> PwResult<Vec<u8>> {
        if cancelled() {
            return Err(PwError::cancelled());
        }
        save_bytes(&copy_pages(pdfium, src, &(from..from + n).collect::<Vec<_>>())?)
    };
    let fits = |b: &[u8]| b.len() as u64 <= max_bytes;
    let mut out = Vec::new();
    let mut oversized = Vec::new();
    let mut start = 0;
    while start < count {
        let left = count - start;
        let one = build(start, 1)?;
        if !fits(&one) {
            oversized.push(start + 1);
            out.push((vec![start], one));
            start += 1;
            continue;
        }
        // good: largest n known to fit; bad: smallest n known not to fit (or left + 1).
        let (mut good, mut good_bytes, mut bad) = (1, one, left + 1);
        let mut probe = 2;
        while probe <= left {
            let b = build(start, probe)?;
            if fits(&b) {
                good = probe;
                good_bytes = b;
                probe = probe.saturating_mul(2);
            } else {
                bad = probe;
                break;
            }
        }
        if bad == left + 1 && good < left {
            // Doubling stepped past the end; the remainder is the last candidate.
            let b = build(start, left)?;
            if fits(&b) {
                good = left;
                good_bytes = b;
            } else {
                bad = left;
            }
        }
        while bad - good > 1 {
            let mid = good + (bad - good) / 2;
            let b = build(start, mid)?;
            if fits(&b) {
                good = mid;
                good_bytes = b;
            } else {
                bad = mid;
            }
        }
        out.push(((start..start + good).collect(), good_bytes));
        start += good;
    }
    if !oversized.is_empty() {
        let list: Vec<String> = oversized.iter().take(10).map(|p| p.to_string()).collect();
        warnings.push(format!(
            "{} page(s) are larger than the limit on their own and were saved as single-page parts (page {}{}).",
            oversized.len(),
            list.join(", "),
            if oversized.len() > 10 { ", …" } else { "" }
        ));
    }
    Ok(out)
}

/// Smallest page side a crop may leave, in points (a quarter inch).
const MIN_CROPPED_SIDE: f32 = 18.0;

/// Displayed margins (top, right, bottom, left) → margins on the unrotated page's
/// (top, right, bottom, left) sides. `/Rotate` turns the page clockwise for display.
pub(crate) fn margins_to_page(rot: u32, m: &pw_core::ops::Margins) -> (f32, f32, f32, f32) {
    match rot {
        90 => (m.right, m.bottom, m.left, m.top),
        180 => (m.bottom, m.left, m.top, m.right),
        270 => (m.left, m.top, m.right, m.bottom),
        _ => (m.top, m.right, m.bottom, m.left),
    }
}

fn visible_box(page: &PdfPage) -> PdfRect {
    page.boundaries()
        .bounding()
        .map(|b| b.bounds)
        .unwrap_or_else(|_| PdfRect::new_from_values(0.0, 0.0, page.height().value, page.width().value))
}

/// Crop pages by margins as displayed. Verified: the chosen pages' visible areas are exactly the
/// cropped boxes, every other page is unchanged, and the page count is the same.
pub fn crop_pages(
    input: &Path,
    opts: &OpenOptions,
    pages: Option<&str>,
    margins: &pw_core::ops::Margins,
    output: &Path,
    policy: CollisionPolicy,
) -> PwResult<EditOutcome> {
    let pdfium = engine()?;
    let doc = load(&pdfium, input, opts)?;
    let total = doc.pages().len().max(0) as u32;
    let indices = match pages {
        Some(spec) => parse_page_ranges(spec, total)?,
        None => (0..total).collect(),
    };
    let chosen: std::collections::HashSet<u32> = indices.iter().copied().collect();
    let err = |e: PdfiumError| map_pdfium_error(&e, "crop");
    let mut expected: Vec<[f32; 4]> = Vec::with_capacity(total as usize);
    for i in 0..total {
        let mut page = doc.pages().get(i as PdfPageIndex).map_err(err)?;
        let v = visible_box(&page);
        let (l, b, r, t) = (v.left().value, v.bottom().value, v.right().value, v.top().value);
        if !chosen.contains(&i) {
            expected.push([l, b, r, t]);
            continue;
        }
        let (mt, mr, mb, ml) = margins_to_page(rotation_of(&page), margins);
        let nb = [l + ml, b + mb, r - mr, t - mt];
        if nb[2] - nb[0] < MIN_CROPPED_SIDE || nb[3] - nb[1] < MIN_CROPPED_SIDE {
            return Err(PwError::invalid(
                "crop.too_much",
                format!("The margins leave almost nothing of page {}; use smaller margins.", i + 1),
            ));
        }
        page.boundaries_mut().set_crop(PdfRect::new_from_values(nb[1], nb[0], nb[3], nb[2])).map_err(err)?;
        expected.push(nb);
    }
    let bytes = save_bytes(&doc)?;
    let src_encrypted = source_encrypted(&doc);
    let protected = [input.to_path_buf()];
    let mut encrypted = false;
    let c = write_atomic_bytes(output, policy, &protected, &bytes, |staged| {
        encrypted =
            verify_output(&pdfium, staged, &Expect { page_count: total, password: opts.password, rotations: None })?;
        let reopened =
            pdfium.load_pdf_from_file(staged, if encrypted { opts.password } else { None }).map_err(|_| {
                PwError::new(ErrorCategory::ValidationFailed, "output.verify", "The output could not be reopened.")
            })?;
        for (i, want) in expected.iter().enumerate() {
            let page = reopened.pages().get(i as PdfPageIndex).map_err(|e| map_pdfium_error(&e, "page"))?;
            let v = visible_box(&page);
            let got = [v.left().value, v.bottom().value, v.right().value, v.top().value];
            if got.iter().zip(want).any(|(g, w)| (g - w).abs() > 0.05) {
                return Err(PwError::new(
                    ErrorCategory::ValidationFailed,
                    "output.verify",
                    format!("The output failed verification (page {} does not have the expected size).", i + 1),
                ));
            }
        }
        Ok(())
    })?;
    let mut warnings = protection_warnings(src_encrypted, encrypted);
    warnings.push(
        "Content outside the cropped area is hidden, not removed; use Redact to remove sensitive content.".into(),
    );
    Ok((c, total, warnings))
}

/// Render a page small (longest side ≤ 300 px, annotations off) for visibility checks.
fn thumb(page: &PdfPage) -> PwResult<image::RgbaImage> {
    let longest = page.width().value.max(page.height().value).max(1.0);
    let cfg = PdfRenderConfig::new()
        .scale_page_by_factor((300.0 / longest).min(1.0))
        .render_annotations(false)
        .render_form_data(false);
    let bmp = page.render_with_config(&cfg).map_err(|e| map_pdfium_error(&e, "render"))?;
    Ok(bmp.as_image().map_err(|e| map_pdfium_error(&e, "render"))?.to_rgba8())
}

/// Add a text watermark, centred and turned by `angle` as the page is displayed, over or behind
/// the content. Verified: page count, the watermark text on every chosen page, and whether it can be
/// seen (pages where it is covered or invisible are reported, not hidden).
pub fn watermark(
    input: &Path,
    opts: &OpenOptions,
    pages: Option<&str>,
    w: &pw_core::ops::WatermarkSpec,
    output: &Path,
    policy: CollisionPolicy,
    cancelled: &dyn Fn() -> bool,
) -> PwResult<EditOutcome> {
    use pw_core::ops::WatermarkLayer;
    let pdfium = engine()?;
    let source = load(&pdfium, input, opts)?;
    let mut doc = load(&pdfium, input, opts)?;
    let total = doc.pages().len().max(0) as u32;
    let indices = match pages {
        Some(spec) => parse_page_ranges(spec, total)?,
        None => (0..total).collect(),
    };
    let font = doc.fonts_mut().helvetica();
    let err = |e: PdfiumError| map_pdfium_error(&e, "watermark");
    let base = parse_color(&w.color)?;
    let alpha = (w.opacity.clamp(0.0, 1.0) * 255.0).round() as u8;
    let color = PdfColor::new(base.red(), base.green(), base.blue(), alpha);
    for &i in &indices {
        if cancelled() {
            return Err(PwError::new(ErrorCategory::Cancelled, "job.cancelled", "The watermark was cancelled."));
        }
        let mut page = doc.pages().get(i as PdfPageIndex).map_err(err)?;
        let rot = rotation_of(&page);
        let b = page
            .boundaries()
            .bounding()
            .map(|b| b.bounds)
            .unwrap_or_else(|_| PdfRect::new_from_values(0.0, 0.0, page.height().value, page.width().value));
        let (cx, cy) = ((b.left().value + b.right().value) / 2.0, (b.bottom().value + b.top().value) / 2.0);
        let mut obj = PdfPageTextObject::new(&doc, &w.text, font, PdfPoints::new(w.font_size)).map_err(err)?;
        obj.set_fill_color(color).map_err(err)?;
        let tw = obj.width().map_err(err)?.value;
        // Centre the text on its own origin (cap height ≈ 0.7 em), turn it, then move it to the
        // page centre. The page's /Rotate is added so the angle is as displayed.
        obj.translate(PdfPoints::new(-tw / 2.0), PdfPoints::new(-w.font_size * 0.35)).map_err(err)?;
        obj.rotate_counter_clockwise_degrees(w.angle + rot as f32).map_err(err)?;
        obj.translate(PdfPoints::new(cx), PdfPoints::new(cy)).map_err(err)?;
        match w.layer {
            WatermarkLayer::Over => {
                page.objects_mut().add_text_object(obj).map_err(err)?;
            }
            WatermarkLayer::Behind => {
                page.objects_mut().insert_object_at_index(0, PdfPageObject::Text(obj)).map_err(err)?;
            }
        }
    }
    let bytes = save_bytes(&doc)?;
    let src_encrypted = source_encrypted(&doc);
    let protected = [input.to_path_buf()];
    let mut encrypted = false;
    let mut hidden: Vec<u32> = Vec::new();
    let squash = |t: &str| t.chars().filter(|c| !c.is_whitespace()).collect::<String>();
    let want = squash(&w.text);
    let c = write_atomic_bytes(output, policy, &protected, &bytes, |staged| {
        encrypted =
            verify_output(&pdfium, staged, &Expect { page_count: total, password: opts.password, rotations: None })?;
        let reopened =
            pdfium.load_pdf_from_file(staged, if encrypted { opts.password } else { None }).map_err(|_| {
                PwError::new(ErrorCategory::ValidationFailed, "output.verify", "The output could not be reopened.")
            })?;
        let everything = PdfRect::new_from_values(-1.0e6, -1.0e6, 1.0e6, 1.0e6);
        hidden.clear();
        for &i in &indices {
            let page = reopened.pages().get(i as PdfPageIndex).map_err(|e| map_pdfium_error(&e, "page"))?;
            let got = page.text().map_err(|e| map_pdfium_error(&e, "text"))?.inside_rect(everything);
            if !squash(&got).contains(&want) {
                return Err(PwError::new(
                    ErrorCategory::ValidationFailed,
                    "output.verify",
                    format!("The output failed verification (page {} is missing its watermark).", i + 1),
                ));
            }
            let src = source.pages().get(i as PdfPageIndex).map_err(|e| map_pdfium_error(&e, "page"))?;
            let (a, b) = (thumb(&src)?, thumb(&page)?);
            let changed = a.dimensions() == b.dimensions()
                && a.pixels().zip(b.pixels()).any(|(p, q)| (0..3).any(|k| p.0[k].abs_diff(q.0[k]) > 8));
            if !changed {
                hidden.push(i + 1);
            }
        }
        Ok(())
    })?;
    let mut warnings = protection_warnings(src_encrypted, encrypted);
    if !hidden.is_empty() {
        let list = hidden.iter().map(u32::to_string).collect::<Vec<_>>().join(", ");
        warnings.push(format!(
            "The watermark cannot be seen on page{} {list}: {}",
            if hidden.len() == 1 { "" } else { "s" },
            match w.layer {
                WatermarkLayer::Behind => "the page content covers it. Place it over the content instead.",
                WatermarkLayer::Over => "it matches the page colours there, or is too faint.",
            }
        ));
    }
    Ok((c, total, warnings))
}

fn parse_color(hex: &str) -> PwResult<PdfColor> {
    let v = u32::from_str_radix(hex.trim_start_matches('#'), 16)
        .map_err(|_| PwError::invalid("annotate.color", "Annotation colors must look like #rrggbb."))?;
    Ok(PdfColor::new((v >> 16) as u8, (v >> 8) as u8, v as u8, 255))
}

fn pdf_rect(r: &pw_core::ops::RectPt) -> PdfRect {
    PdfRect::new_from_values(r.bottom, r.left, r.top, r.right)
}

/// Add annotations to a loaded document. Markup annotations (highlight, underline, strike-out) get
/// one quad per rectangle; notes are placed at the first rectangle's top-left. Returns the expected
/// annotation count of every touched page, for verification after saving.
pub(crate) fn apply_annotations(
    doc: &PdfDocument,
    specs: &[pw_core::ops::AnnotationSpec],
) -> PwResult<std::collections::BTreeMap<u32, usize>> {
    use pw_core::ops::AnnotationKind;
    let count = doc.pages().len().max(0) as u32;
    let mut expected: std::collections::BTreeMap<u32, usize> = std::collections::BTreeMap::new();
    for spec in specs {
        if spec.page >= count {
            return Err(PwError::invalid(
                "annotate.page",
                format!("Page {} does not exist; the document has {count} pages.", spec.page + 1),
            ));
        }
    }
    for spec in specs {
        let mut page = doc.pages().get(spec.page as PdfPageIndex).map_err(|e| map_pdfium_error(&e, "page"))?;
        let before = *expected.entry(spec.page).or_insert_with(|| page.annotations().len());
        let color = parse_color(&spec.color)?;
        let first = spec
            .rects
            .first()
            .ok_or_else(|| PwError::invalid("annotate.rects", "Each annotation needs a location."))?;
        let bounds = spec.rects.iter().fold(*first, |acc, r| pw_core::ops::RectPt {
            left: acc.left.min(r.left),
            bottom: acc.bottom.min(r.bottom),
            right: acc.right.max(r.right),
            top: acc.top.max(r.top),
        });
        let annots = page.annotations_mut();
        let err = |e: PdfiumError| map_pdfium_error(&e, "annotate");
        macro_rules! markup {
            ($create:ident) => {{
                let mut a = annots.$create().map_err(err)?;
                for r in &spec.rects {
                    a.attachment_points_mut()
                        .create_attachment_point_at_end(PdfQuadPoints::from_rect(&pdf_rect(r)))
                        .map_err(err)?;
                }
                a.set_bounds(pdf_rect(&bounds)).map_err(err)?;
                a.set_stroke_color(color).map_err(err)?;
                if let Some(c) = &spec.contents {
                    a.set_contents(c).map_err(err)?;
                }
            }};
        }
        match spec.kind {
            AnnotationKind::Highlight => markup!(create_highlight_annotation),
            AnnotationKind::Underline => markup!(create_underline_annotation),
            AnnotationKind::StrikeOut => markup!(create_strikeout_annotation),
            AnnotationKind::Note => {
                let text = spec.contents.as_deref().unwrap_or_default();
                let mut a = annots.create_text_annotation(text).map_err(err)?;
                let icon = pw_core::ops::RectPt {
                    left: first.left,
                    bottom: first.top - 18.0,
                    right: first.left + 18.0,
                    top: first.top,
                };
                a.set_bounds(pdf_rect(&icon)).map_err(err)?;
                a.set_stroke_color(color).map_err(err)?;
            }
        }
        expected.insert(spec.page, before + 1);
    }
    Ok(expected)
}

/// Reopen a staged output and check the annotation count of every touched page.
pub(crate) fn verify_annotation_counts(
    pdfium: &Pdfium,
    staged: &Path,
    password: Option<&str>,
    expected: &std::collections::BTreeMap<u32, usize>,
) -> PwResult<()> {
    let reopened =
        pdfium.load_pdf_from_file(staged, None).or_else(|_| pdfium.load_pdf_from_file(staged, password)).map_err(
            |_| PwError::new(ErrorCategory::ValidationFailed, "output.verify", "The output could not be reopened."),
        )?;
    for (page, want) in expected {
        let got = reopened.pages().get(*page as PdfPageIndex).map(|p| p.annotations().len()).unwrap_or(0);
        if got != *want {
            return Err(PwError::new(
                ErrorCategory::ValidationFailed,
                "output.verify",
                format!("The output failed verification (page {} has {got} annotations, expected {want}).", page + 1),
            ));
        }
    }
    Ok(())
}

/// Add annotations to a copy of the document; the staged output is reopened and the annotation
/// count of every touched page is verified before it is committed.
pub fn annotate(
    input: &Path,
    opts: &OpenOptions,
    specs: &[pw_core::ops::AnnotationSpec],
    output: &Path,
    policy: CollisionPolicy,
) -> PwResult<EditOutcome> {
    let pdfium = engine()?;
    let doc = load(&pdfium, input, opts)?;
    let count = doc.pages().len().max(0) as u32;
    let expected = apply_annotations(&doc, specs)?;
    let bytes = save_bytes(&doc)?;
    let src_encrypted = source_encrypted(&doc);
    let protected = [input.to_path_buf()];
    let mut encrypted = false;
    let c = write_atomic_bytes(output, policy, &protected, &bytes, |staged| {
        encrypted =
            verify_output(&pdfium, staged, &Expect { page_count: count, password: opts.password, rotations: None })?;
        verify_annotation_counts(&pdfium, staged, opts.password, &expected)
    })?;
    Ok((c, count, protection_warnings(src_encrypted, encrypted)))
}

/// Regenerate a page's content after objects were removed or changed, keeping colours.
///
/// PDFium writes regenerated colours only for device colour spaces: a fill or stroke set through
/// an ICC-based space (how Word and many generators write every colour) came out as `0 0 0 rg`, so
/// table shading turned black after translation or editing. Each path and text object's colour is
/// set again as the RGB PDFium renders it with before writing; colours it cannot give (patterns)
/// are left as they are.
pub(crate) fn regenerate_keeping_colours(page: &mut PdfPage) -> Result<(), PdfiumError> {
    let n = page.objects().len();
    for i in 0..n {
        let Ok(mut o) = page.objects().get(i) else { continue };
        if !matches!(o, PdfPageObject::Path(_) | PdfPageObject::Text(_)) {
            continue;
        }
        if let Ok(c) = o.fill_color() {
            let _ = o.set_fill_color(c);
        }
        if let Ok(c) = o.stroke_color() {
            let _ = o.set_stroke_color(c);
        }
    }
    page.regenerate_content()
}
