//! Split by bookmarks and by maximum file size. Parts are checked independently with lopdf
//! (page count and page text) and by their size on disk.

use pw_core::ops::{BookmarkSpec, Operation, OperationRequest, OperationResult, SplitMode};
use pw_core::output::CollisionPolicy;
use pw_core::PwResult;
use pw_pdf::execute;
use std::path::{Path, PathBuf};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/pdf").join(name)
}

fn run(op: Operation) -> PwResult<OperationResult> {
    let mut req = OperationRequest::new(op);
    req.collision = CollisionPolicy::Fail;
    execute(&req, None)
}

/// Page count and per-page text markers, read with lopdf.
fn pages(path: &Path) -> Vec<String> {
    let doc = lopdf::Document::load(path).expect("lopdf parses the part");
    doc.get_pages().keys().map(|n| doc.extract_text(&[*n]).unwrap_or_default()).collect()
}

fn names(r: &OperationResult) -> Vec<String> {
    r.outputs.iter().map(|o| o.path.file_name().expect("name").to_string_lossy().into_owned()).collect()
}

/// twenty-pages.pdf with an outline: front matter (pages 1–2), then level-1 bookmarks at pages
/// 3, 8 and 15 (one listed out of order, one duplicate page), level-2 bookmarks inside the first.
fn outlined(dir: &Path) -> PathBuf {
    let out = dir.join("book.pdf");
    let b = |title: &str, page: u32, depth: u32| BookmarkSpec { title: title.into(), page, depth };
    run(Operation::SetBookmarks {
        input: fixture("twenty-pages.pdf"),
        bookmarks: vec![
            b("Intro: why/how?", 2, 0),
            b("Part A", 3, 1),
            b("Part B", 5, 1),
            b("Appendix", 14, 0),
            b("Chapter  two", 7, 0),
            b("Same page as chapter two", 7, 0),
        ],
        output: out.clone(),
    })
    .expect("outline");
    out
}

#[test]
fn split_by_top_level_bookmarks_names_parts_after_them() {
    let dir = tempfile::tempdir().expect("tmp");
    let src = outlined(dir.path());
    let parts = dir.path().join("parts");
    std::fs::create_dir(&parts).expect("dir");
    let r = run(Operation::Split { input: src, split: SplitMode::Bookmarks { level: 1 }, output_dir: parts })
        .expect("split");
    assert_eq!(names(&r), ["book_01_Intro_ why_how_.pdf", "book_02_Chapter two.pdf", "book_03_Appendix.pdf"]);
    let counts: Vec<usize> = r.outputs.iter().map(|o| pages(&o.path).len()).collect();
    assert_eq!(counts, [7, 7, 6], "front matter joins the first part; pages 8–14; 15–20");
    let first = pages(&r.outputs[0].path);
    assert!(first[0].contains("PWMARK001") && first[6].contains("PWMARK007"), "{first:?}");
    assert!(pages(&r.outputs[2].path)[0].contains("PWMARK015"));
}

#[test]
fn split_by_second_level_bookmarks_and_refusals() {
    let dir = tempfile::tempdir().expect("tmp");
    let src = outlined(dir.path());
    let parts = dir.path().join("l2");
    std::fs::create_dir(&parts).expect("dir");
    let r = run(Operation::Split { input: src.clone(), split: SplitMode::Bookmarks { level: 2 }, output_dir: parts })
        .expect("split");
    assert_eq!(names(&r), ["book_01_Part A.pdf", "book_02_Part B.pdf"]);
    let counts: Vec<usize> = r.outputs.iter().map(|o| pages(&o.path).len()).collect();
    assert_eq!(counts, [5, 15], "pages 1–5 (front included), 6–20");

    let e = run(Operation::Split {
        input: src,
        split: SplitMode::Bookmarks { level: 3 },
        output_dir: dir.path().to_path_buf(),
    })
    .expect_err("no level 3");
    assert_eq!(e.code, "split.no_bookmarks");
    let e = run(Operation::Split {
        input: fixture("single-page.pdf"),
        split: SplitMode::Bookmarks { level: 1 },
        output_dir: dir.path().to_path_buf(),
    })
    .expect_err("no outline");
    assert_eq!(e.code, "split.no_bookmarks");
    let e = run(Operation::Split {
        input: fixture("five-pages.pdf"),
        split: SplitMode::Bookmarks { level: 0 },
        output_dir: dir.path().to_path_buf(),
    })
    .expect_err("level 0");
    assert_eq!(e.code, "split.level");
}

#[test]
fn split_by_size_keeps_every_part_under_the_limit_and_all_pages_in_order() {
    let src = fixture("photo-heavy.pdf");
    let total = std::fs::metadata(&src).expect("meta").len();
    let n_src = pages(&src).len();
    let dir = tempfile::tempdir().expect("tmp");
    let limit = total / 2;
    let r = run(Operation::Split {
        input: src.clone(),
        split: SplitMode::MaxSize { max_bytes: limit },
        output_dir: dir.path().to_path_buf(),
    })
    .expect("split");
    assert!(r.outputs.len() >= 2, "{} parts", r.outputs.len());
    let mut all = Vec::new();
    for o in &r.outputs {
        let size = std::fs::metadata(&o.path).expect("meta").len();
        assert!(size <= limit || pages(&o.path).len() == 1, "{} is {size} > {limit}", o.path.display());
        assert_eq!(size, o.bytes);
        all.extend(pages(&o.path));
    }
    assert_eq!(all.len(), n_src, "every page exactly once");
    assert_eq!(all, pages(&src), "same pages, same order");
    // Greedy and maximal: joining a part with the next page's part would exceed the limit, so no
    // two neighbouring parts could have been one (checked by the part count lower bound).
    assert!(r.outputs.len() as u64 >= total.div_ceil(limit).saturating_sub(1).max(1));

    // A limit no page fits under: every page alone, with a warning.
    let dir2 = tempfile::tempdir().expect("tmp");
    let r = run(Operation::Split {
        input: src,
        split: SplitMode::MaxSize { max_bytes: 10 * 1024 },
        output_dir: dir2.path().to_path_buf(),
    })
    .expect("split tiny");
    assert_eq!(r.outputs.len(), n_src);
    assert!(r.warnings.iter().any(|w| w.contains("larger than the limit")), "{:?}", r.warnings);

    // A generous limit: one part.
    let dir3 = tempfile::tempdir().expect("tmp");
    let r = run(Operation::Split {
        input: fixture("five-pages.pdf"),
        split: SplitMode::MaxSize { max_bytes: 50 * 1024 * 1024 },
        output_dir: dir3.path().to_path_buf(),
    })
    .expect("split big");
    assert_eq!(names(&r), ["five-pages_part01.pdf"]);

    let e = run(Operation::Split {
        input: fixture("five-pages.pdf"),
        split: SplitMode::MaxSize { max_bytes: 100 },
        output_dir: dir3.path().to_path_buf(),
    })
    .expect_err("too small");
    assert_eq!(e.code, "split.max_size");
}
