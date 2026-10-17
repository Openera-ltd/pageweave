//! Page-level document operations. Sources are never modified: every operation builds a new
//! document, saves it to memory, re-opens the staged output to verify it, and commits atomically.

use crate::document::{load, OpenOptions};
use crate::{engine, map_pdfium_error};
use pdfium_render::prelude::*;
use pw_core::output::{write_atomic_bytes, CollisionPolicy, CommittedOutput};
use pw_core::range::parse_page_ranges;
use pw_core::{ErrorCategory, PwError, PwResult};
use std::path::{Path, PathBuf};

/// Expected properties of an output, checked after it is written and re-parsed.
struct Expect<'a> {
    page_count: u32,
    /// Password of the source, used if the output kept the source's encryption.
    password: Option<&'a str>,
    /// Expected intrinsic rotation per output page, when known.
    rotations: Option<Vec<u32>>,
}

/// Re-open and check the staged output. Returns whether the output is encrypted.
fn verify_output(pdfium: &Pdfium, staged: &Path, expect: &Expect) -> PwResult<bool> {
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

fn source_encrypted(doc: &PdfDocument) -> bool {
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
    let mut had_bookmarks = false;
    let mut any_encrypted = false;
    for (n, input) in inputs.iter().enumerate() {
        if cancelled() {
            return Err(PwError::cancelled());
        }
        let src = load(&pdfium, input, opts)
            .map_err(|e| PwError { message: format!("File {} of {}: {}", n + 1, inputs.len(), e.message), ..e })?;
        had_bookmarks |= src.bookmarks().root().is_some();
        any_encrypted |= source_encrypted(&src);
        out.pages_mut().append(&src).map_err(|e| map_pdfium_error(&e, "append"))?;
        progress(n as u64 + 1, inputs.len() as u64);
    }
    if had_bookmarks {
        warnings.push("Bookmarks from the combined files are not included in the output.".into());
    }
    let total = out.pages().len().max(0) as u32;
    let bytes = save_bytes(&out)?;
    let expect = Expect { page_count: total, password: opts.password, rotations: None };
    let (c, enc) = commit(&pdfium, &bytes, output, policy, inputs, &expect)?;
    warnings.extend(protection_warnings(any_encrypted, enc));
    Ok((c, total, warnings))
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
    let groups: Vec<Vec<u32>> = match split {
        SplitMode::EveryN { n } => {
            if *n == 0 {
                return Err(PwError::invalid("split.n_zero", "Split size must be at least 1 page."));
            }
            (0..count).collect::<Vec<_>>().chunks(*n as usize).map(|c| c.to_vec()).collect()
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
                        .map_err(|e| PwError { message: format!("Part {}: {}", i + 1, e.message), ..e })
                })
                .collect::<PwResult<_>>()?
        }
    };
    if groups.len() > 10_000 {
        return Err(PwError::invalid("split.too_many_parts", "That would create more than 10,000 files."));
    }
    let stem = input.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "document".into());
    let width = groups.len().to_string().len().max(2);
    let mut built = Vec::with_capacity(groups.len());
    for g in &groups {
        if cancelled() {
            return Err(PwError::cancelled());
        }
        let doc = copy_pages(&pdfium, &src, g)?;
        built.push((save_bytes(&doc)?, g.len() as u32));
    }
    let src_encrypted = source_encrypted(&src);
    let mut any_unprotected = false;
    let mut committed: Vec<(CommittedOutput, u32)> = Vec::new();
    for (i, (bytes, n)) in built.iter().enumerate() {
        let name = output_dir.join(format!("{stem}_part{:0width$}.pdf", i + 1));
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
    Ok((committed, protection_warnings(src_encrypted, !any_unprotected)))
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
