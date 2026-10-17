//! Document comparison against the real engine with a fixture pair whose differences are known
//! (tools/fixtures/gen_fixtures.py `compare_pair`).
use pw_core::ops::RectPt;
use pw_pdf::{compare, ChangeKind, CompareReport, OpenOptions, PairStatus};
use std::path::{Path, PathBuf};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/pdf").join(name)
}

fn run(old: &Path, new: &Path) -> pw_core::PwResult<CompareReport> {
    compare(old, new, &OpenOptions { password: None }, &|| false, &|_, _| {})
}

fn kinds(r: &CompareReport, pair: usize) -> Vec<ChangeKind> {
    r.pairs[pair].changes.iter().map(|c| c.kind).collect()
}

fn inside(r: &RectPt, l: f32, b: f32, rr: f32, t: f32) -> bool {
    r.left >= l && r.bottom >= b && r.right <= rr && r.top <= t
}

#[test]
fn known_differences_are_found_and_nothing_else() {
    let r = run(&fixture("compare-old.pdf"), &fixture("compare-new.pdf")).expect("compare");
    assert_eq!((r.old_pages, r.new_pages), (6, 6));
    let pages: Vec<(Option<u32>, Option<u32>, PairStatus)> = r.pairs.iter().map(|p| (p.old, p.new, p.status)).collect();
    assert_eq!(
        pages,
        vec![
            (Some(0), Some(0), PairStatus::Same),
            (Some(1), Some(1), PairStatus::Changed),
            (Some(2), None, PairStatus::Deleted),
            (None, Some(2), PairStatus::Inserted),
            (Some(3), Some(3), PairStatus::Changed),
            (Some(4), Some(4), PairStatus::Changed),
            (Some(5), Some(5), PairStatus::Changed),
        ]
    );
    // Page 2: a replaced word, an inserted word, a deleted line and a line set in bold. The
    // unchanged words that moved because of these edits are not reported.
    assert_eq!(kinds(&r, 1), vec![ChangeKind::Replaced, ChangeKind::Inserted, ChangeKind::Deleted, ChangeKind::Format]);
    let c = &r.pairs[1].changes;
    assert_eq!((c[0].text.as_str(), c[0].new_text.as_deref()), ("quarterly", Some("annual")));
    assert_eq!(c[1].text, "full");
    assert_eq!(c[2].text, "This line will be deleted PWGONE");
    // The deleted line is the third line of old page 2: baseline 720 - 2 * 18 = 684.
    assert_eq!(c[2].old_rects.len(), 1);
    assert!(inside(&c[2].old_rects[0], 70.0, 678.0, 300.0, 698.0), "{:?}", c[2].old_rects);
    assert!(c[3].detail.as_deref().unwrap_or_default().contains("Helvetica-Bold"));
    // Paragraph moved from old page 4 to new page 5; the drawn box on page 5 is a visual change.
    assert_eq!(kinds(&r, 4), vec![ChangeKind::Moved]);
    assert_eq!(kinds(&r, 5), vec![ChangeKind::Moved, ChangeKind::Visual]);
    let v = &r.pairs[5].changes[1];
    assert!(inside(&v.new_rects[0], 280.0, 280.0, 470.0, 420.0), "box drawn at 300,300 150x100: {:?}", v.new_rects);
    assert!(v.new_rects[0].right - v.new_rects[0].left > 140.0);
    // Page 6: Letter -> A4.
    assert_eq!(kinds(&r, 6), vec![ChangeKind::PageSize]);
    assert_eq!(r.metadata.len(), 1);
    assert_eq!((r.metadata[0].old.as_str(), r.metadata[0].new.as_str()), ("Report v1", "Report v2"));
    assert_eq!(
        (r.summary.pages_changed, r.summary.pages_inserted, r.summary.pages_deleted, r.summary.text_changes),
        (4, 1, 1, 5)
    );
    assert!(r.notes.is_empty());
}

#[test]
fn reversed_comparison_mirrors_the_result() {
    let r = run(&fixture("compare-new.pdf"), &fixture("compare-old.pdf")).expect("compare");
    let statuses: Vec<PairStatus> = r.pairs.iter().map(|p| p.status).collect();
    assert_eq!(statuses.iter().filter(|s| **s == PairStatus::Inserted).count(), 1);
    assert_eq!(statuses.iter().filter(|s| **s == PairStatus::Deleted).count(), 1);
    assert_eq!(kinds(&r, 1), vec![ChangeKind::Replaced, ChangeKind::Deleted, ChangeKind::Inserted, ChangeKind::Format]);
}

#[test]
fn identical_documents_have_no_differences() {
    for name in ["five-pages.pdf", "mixed-sizes-rotated.pdf", "scanned.pdf", "form-fields.pdf", "photo-heavy.pdf"] {
        let r = run(&fixture(name), &fixture(name)).expect("compare");
        assert!(r.pairs.iter().all(|p| p.status == PairStatus::Same && p.changes.is_empty()), "{name}: {:?}", r.pairs);
        assert!(r.metadata.is_empty());
    }
}

#[test]
fn pages_without_text_align_by_appearance() {
    // The scanned fixture against a copy with a page removed: the rest still pair up.
    let dir = tempfile::tempdir().expect("tmp");
    let fewer = dir.path().join("fewer.pdf");
    let n = pw_pdf::open_document(&fixture("scanned.pdf"), &OpenOptions { password: None }).expect("open").page_count;
    assert!(n >= 2, "the scanned fixture needs several pages for this test");
    pw_pdf::execute(
        &pw_core::ops::OperationRequest::new(pw_core::ops::Operation::DeletePages {
            input: fixture("scanned.pdf"),
            pages: "1".into(),
            output: fewer.clone(),
        }),
        None,
    )
    .expect("delete");
    let r = run(&fixture("scanned.pdf"), &fewer).expect("compare");
    assert_eq!(r.pairs[0].status, PairStatus::Deleted);
    assert!(r.pairs[1..].iter().all(|p| p.status == PairStatus::Same), "{:?}", r.pairs);
}

#[test]
fn bad_inputs_and_cancellation() {
    let opts = OpenOptions { password: None };
    assert!(run(&fixture("corrupt-truncated.pdf"), &fixture("five-pages.pdf")).is_err());
    assert!(run(&fixture("five-pages.pdf"), &fixture("not-a-pdf.pdf")).is_err());
    let e = run(&fixture("encrypted-rc4-user-password.pdf"), &fixture("five-pages.pdf")).expect_err("password");
    assert_eq!(e.category, pw_core::ErrorCategory::PasswordRequired);
    let e = compare(&fixture("five-pages.pdf"), &fixture("five-pages.pdf"), &opts, &|| true, &|_, _| {})
        .expect_err("cancelled");
    assert_eq!(e.category, pw_core::ErrorCategory::Cancelled);
}
