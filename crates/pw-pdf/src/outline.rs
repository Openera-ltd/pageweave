//! Edit bookmarks (PRD 05 "label/bookmark"): replace the document outline in a new file. PDFium can
//! read but not write outlines, so the outline dictionaries are built with lopdf; every bookmark
//! opens its page (fit to window). Verified before commit: PDFium reads back exactly the requested
//! titles, pages and nesting, and every page renders as before.

use crate::document::{open_document, OpenOptions};
use crate::sanitize::{catalog, check_pages, load_plain};
use crate::{engine, properties::signature_warning};
use lopdf::{Dictionary, Object, ObjectId};
use pw_core::ops::BookmarkSpec;
use pw_core::output::{write_atomic_bytes, CollisionPolicy, CommittedOutput};
use pw_core::{ErrorCategory, PwError, PwResult};
use std::path::Path;

fn fail(what: &str) -> PwError {
    PwError::new(
        ErrorCategory::ValidationFailed,
        "output.verify",
        format!("The copy with edited bookmarks failed verification ({what}) and was not saved."),
    )
}

/// Parent index (in the flat list) of each bookmark, or None for top level.
fn parents(specs: &[BookmarkSpec]) -> Vec<Option<usize>> {
    let mut stack: Vec<usize> = Vec::new();
    specs
        .iter()
        .enumerate()
        .map(|(i, b)| {
            while stack.len() > b.depth as usize {
                stack.pop();
            }
            let parent = stack.last().copied();
            stack.push(i);
            parent
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
pub fn set_bookmarks(
    input: &Path,
    opts: &OpenOptions,
    specs: &[BookmarkSpec],
    output: &Path,
    policy: CollisionPolicy,
    cancelled: &dyn Fn() -> bool,
    progress: &dyn Fn(u64, u64),
) -> PwResult<(CommittedOutput, u32, Vec<String>)> {
    let pdfium = engine()?;
    {
        let src = crate::document::load(&pdfium, input, opts)?;
        if crate::unprotect::is_protected(&src) {
            return Err(PwError::new(
                ErrorCategory::UnsupportedOperation,
                "bookmarks.protected",
                "The bookmarks of a password-protected file cannot be edited here.",
            )
            .with_action("Remove the password first (Remove password…), then edit the bookmarks of that copy."));
        }
    }
    let (source, mut doc, _raw) = load_plain(&pdfium, input, opts)?;
    let count = source.pages().len().max(0) as u32;
    if let Some(b) = specs.iter().find(|b| b.page >= count) {
        return Err(PwError::invalid(
            "bookmarks.page",
            format!("“{}” points to page {}, but the document has {count} pages.", b.title, b.page + 1),
        ));
    }
    let had_outline = catalog(&doc).is_some_and(|c| c.has(b"Outlines"));
    let root_id = doc.trailer.get(b"Root").and_then(Object::as_reference).map_err(|_| fail("no catalog"))?;

    if specs.is_empty() {
        let root = doc.get_dictionary_mut(root_id).map_err(|_| fail("no catalog"))?;
        root.remove(b"Outlines");
        if root.get(b"PageMode").and_then(Object::as_name).ok() == Some(b"UseOutlines") {
            root.remove(b"PageMode");
        }
    } else {
        write_outline(&mut doc, specs)?;
    }
    doc.trailer.remove(b"Prev");
    doc.trailer.remove(b"XRefStm");
    doc.prune_objects();
    let mut bytes = Vec::new();
    doc.save_to(&mut bytes).map_err(|_| PwError::internal("bookmarks.save", "The copy could not be written."))?;
    check_pages(&pdfium, &source, &bytes, cancelled, progress)?;
    let want: Vec<(String, Option<u32>, u32)> =
        specs.iter().map(|b| (b.title.trim().to_string(), Some(b.page), b.depth)).collect();
    let protected = [input.to_path_buf()];
    let c = write_atomic_bytes(output, policy, &protected, &bytes, |staged| {
        let info =
            open_document(staged, &OpenOptions { password: None }).map_err(|_| fail("it could not be reopened"))?;
        let got: Vec<(String, Option<u32>, u32)> =
            info.bookmarks.iter().map(|b| (b.title.trim().to_string(), b.page, b.depth)).collect();
        if got != want {
            return Err(fail("the bookmarks read back differ"));
        }
        Ok(())
    })?;
    let mut warnings = Vec::new();
    let signatures = source.signatures().len();
    if signatures > 0 {
        warnings.push(signature_warning(signatures));
    }
    if had_outline && !specs.is_empty() {
        warnings.push(
            "Each bookmark now opens its whole page; positions within pages and bookmark actions were not kept.".into(),
        );
    }
    Ok((c, count, warnings))
}

/// Replace the outline of `doc` with `specs` (non-empty; pages are 0-based indices into `doc`):
/// every bookmark opens its page, fit to the window. Also used by merge to keep the combined
/// files' bookmarks.
pub(crate) fn write_outline(doc: &mut lopdf::Document, specs: &[BookmarkSpec]) -> PwResult<()> {
    let pages: Vec<ObjectId> = doc.get_pages().values().copied().collect();
    if specs.is_empty() || specs.iter().any(|b| b.page as usize >= pages.len()) {
        return Err(fail("a bookmark points to a missing page"));
    }
    let root_id = doc.trailer.get(b"Root").and_then(Object::as_reference).map_err(|_| fail("no catalog"))?;
    let outlines_id = doc.new_object_id();
    let ids: Vec<ObjectId> = specs.iter().map(|_| doc.new_object_id()).collect();
    let parent = parents(specs);
    let children = |p: Option<usize>| -> Vec<usize> { (0..specs.len()).filter(|&i| parent[i] == p).collect() };
    // Descendant counts (all items open).
    let mut descendants = vec![0i64; specs.len()];
    for i in (0..specs.len()).rev() {
        if let Some(p) = parent[i] {
            descendants[p] += 1 + descendants[i];
        }
    }
    for (i, b) in specs.iter().enumerate() {
        let siblings = children(parent[i]);
        let pos = siblings.iter().position(|&s| s == i).expect("own sibling list");
        let mut d = Dictionary::new();
        d.set("Title", lopdf::text_string(b.title.trim()));
        d.set("Parent", Object::Reference(parent[i].map_or(outlines_id, |p| ids[p])));
        if pos > 0 {
            d.set("Prev", Object::Reference(ids[siblings[pos - 1]]));
        }
        if pos + 1 < siblings.len() {
            d.set("Next", Object::Reference(ids[siblings[pos + 1]]));
        }
        let kids = children(Some(i));
        if let (Some(&first), Some(&last)) = (kids.first(), kids.last()) {
            d.set("First", Object::Reference(ids[first]));
            d.set("Last", Object::Reference(ids[last]));
            d.set("Count", Object::Integer(descendants[i]));
        }
        d.set("Dest", Object::Array(vec![Object::Reference(pages[b.page as usize]), Object::Name(b"Fit".to_vec())]));
        doc.objects.insert(ids[i], Object::Dictionary(d));
    }
    let top = children(None);
    let mut outlines = Dictionary::new();
    outlines.set("Type", Object::Name(b"Outlines".to_vec()));
    outlines.set("First", Object::Reference(ids[top[0]]));
    outlines.set("Last", Object::Reference(ids[*top.last().expect("non-empty")]));
    outlines.set("Count", Object::Integer(specs.len() as i64));
    doc.objects.insert(outlines_id, Object::Dictionary(outlines));
    let root = doc.get_dictionary_mut(root_id).map_err(|_| fail("no catalog"))?;
    root.set("Outlines", Object::Reference(outlines_id));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b(depth: u32) -> BookmarkSpec {
        BookmarkSpec { title: "x".into(), page: 0, depth }
    }

    #[test]
    fn nesting_follows_depths() {
        let specs = [b(0), b(1), b(2), b(1), b(0), b(1)];
        assert_eq!(parents(&specs), vec![None, Some(0), Some(1), Some(0), None, Some(4)]);
    }
}
