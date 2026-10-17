//! Blank page detection against blank-pages.pdf (tools/fixtures/gen_fixtures.py): (1) text,
//! (2) empty, (3) off-white scan with dust specks, (4) only a page number, (5) faint grey table,
//! (6) text.
use pw_pdf::{find_blank_pages, OpenOptions};
use std::path::{Path, PathBuf};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/pdf").join(name)
}

fn blank(max_ink: f32) -> Vec<u32> {
    find_blank_pages(&fixture("blank-pages.pdf"), &OpenOptions { password: None }, max_ink, &|| false, &|_, _| {})
        .expect("analyse")
        .iter()
        .filter(|p| p.blank)
        .map(|p| p.page)
        .collect()
}

#[test]
fn sensitivity_decides_dusty_scans_and_content_is_never_blank() {
    assert_eq!(blank(0.0002), vec![1], "only the empty page");
    assert_eq!(blank(0.002), vec![1, 2], "plus the dusty scan");
    assert_eq!(blank(0.005), vec![1, 2], "faint table lines (~0.9% ink) are content");
    let pages =
        find_blank_pages(&fixture("blank-pages.pdf"), &OpenOptions { password: None }, 0.05, &|| false, &|_, _| {})
            .expect("analyse");
    assert!(!pages[3].blank && pages[3].text_chars > 0, "a page number alone is text, never blank");
    assert!(pages[4].ink > 0.005, "faint table measured as ink: {}", pages[4].ink);
}

#[test]
fn documents_without_blank_pages_and_bad_input() {
    let none =
        find_blank_pages(&fixture("five-pages.pdf"), &OpenOptions { password: None }, 0.002, &|| false, &|_, _| {})
            .expect("analyse");
    assert!(none.iter().all(|p| !p.blank));
    assert!(find_blank_pages(
        &fixture("corrupt-truncated.pdf"),
        &OpenOptions { password: None },
        0.002,
        &|| false,
        &|_, _| {}
    )
    .is_err());
    let e = find_blank_pages(&fixture("blank-pages.pdf"), &OpenOptions { password: None }, 0.002, &|| true, &|_, _| {})
        .expect_err("cancelled");
    assert_eq!(e.category, pw_core::ErrorCategory::Cancelled);
}
