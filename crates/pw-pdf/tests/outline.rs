//! Editing bookmarks against the real engine; read back with PDFium.
use pw_core::ops::{BookmarkSpec, Operation, OperationRequest};
use pw_pdf::{execute, open_document, OpenOptions};
use std::path::{Path, PathBuf};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/pdf").join(name)
}

fn bm(title: &str, page: u32, depth: u32) -> BookmarkSpec {
    BookmarkSpec { title: title.into(), page, depth }
}

fn set(input: &Path, output: &Path, b: Vec<BookmarkSpec>) -> pw_core::PwResult<pw_core::ops::OperationResult> {
    execute(
        &OperationRequest::new(Operation::SetBookmarks { input: input.into(), bookmarks: b, output: output.into() }),
        None,
    )
}

fn read(path: &Path) -> Vec<(String, Option<u32>, u32)> {
    open_document(path, &OpenOptions { password: None })
        .expect("open")
        .bookmarks
        .into_iter()
        .map(|b| (b.title, b.page, b.depth))
        .collect()
}

#[test]
fn nested_outline_replaces_the_old_one() {
    let src = fixture("five-pages.pdf");
    let before = std::fs::read(&src).expect("read");
    let dir = tempfile::tempdir().expect("tmp");
    let out = dir.path().join("b.pdf");
    let r = set(
        &src,
        &out,
        vec![
            bm("Intro", 0, 0),
            bm("Part A", 1, 1),
            bm("A.1 — Überblick 概要", 2, 2),
            bm("Part B", 3, 1),
            bm("Ending", 4, 0),
        ],
    )
    .expect("set");
    assert!(r.warnings.iter().any(|w| w.contains("whole page")), "{:?}", r.warnings);
    assert_eq!(std::fs::read(&src).expect("read"), before, "source untouched");
    assert_eq!(
        read(&out),
        vec![
            ("Intro".into(), Some(0), 0),
            ("Part A".into(), Some(1), 1),
            ("A.1 — Überblick 概要".into(), Some(2), 2),
            ("Part B".into(), Some(3), 1),
            ("Ending".into(), Some(4), 0)
        ]
    );
    // Old outline items are gone from the file (independent parser).
    let doc = lopdf::Document::load(&out).expect("lopdf");
    let titles: Vec<String> = doc
        .objects
        .values()
        .filter_map(|o| o.as_dict().ok())
        .filter_map(|d| d.get(b"Title").ok())
        .map(|t| lopdf::decode_text_string(t).unwrap_or_default())
        .collect();
    assert!(!titles.iter().any(|t| t.starts_with("Chapter")), "{titles:?}");
}

#[test]
fn add_to_a_file_without_bookmarks_and_remove_all() {
    let dir = tempfile::tempdir().expect("tmp");
    let one = dir.path().join("one.pdf");
    set(&fixture("single-page.pdf"), &one, vec![bm("Only page", 0, 0)]).expect("set");
    assert_eq!(read(&one), vec![("Only page".into(), Some(0), 0)]);
    let none = dir.path().join("none.pdf");
    set(&fixture("five-pages.pdf"), &none, vec![]).expect("remove");
    assert!(read(&none).is_empty());
    let doc = lopdf::Document::load(&none).expect("lopdf");
    assert!(doc.catalog().expect("catalog").get(b"Outlines").is_err());
    assert!(doc.catalog().expect("catalog").get(b"PageMode").is_err(), "UseOutlines dropped with the outline");
}

#[test]
fn invalid_outlines_write_nothing() {
    let dir = tempfile::tempdir().expect("tmp");
    let out = dir.path().join("x.pdf");
    let src = fixture("five-pages.pdf");
    assert_eq!(set(&src, &out, vec![bm("Far", 9, 0)]).expect_err("page").code, "bookmarks.page");
    assert_eq!(set(&src, &out, vec![bm("A", 0, 0), bm("B", 0, 2)]).expect_err("depth").code, "bookmarks.depth");
    assert_eq!(set(&src, &out, vec![bm("A", 0, 1)]).expect_err("first depth").code, "bookmarks.depth");
    assert_eq!(set(&src, &out, vec![bm("  ", 0, 0)]).expect_err("title").code, "bookmarks.title_empty");
    let mut req = OperationRequest::new(Operation::SetBookmarks {
        input: fixture("encrypted-rc4-user-password.pdf"),
        bookmarks: vec![bm("A", 0, 0)],
        output: out.clone(),
    });
    req.password = Some("pw-user-test".into());
    assert_eq!(execute(&req, None).expect_err("protected").code, "bookmarks.protected");
    assert!(!out.exists());
}
