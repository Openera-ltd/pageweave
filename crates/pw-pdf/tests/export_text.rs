//! Export as text against the real engine; the file is read back as raw bytes.
use pw_core::ops::{Operation, OperationRequest};
use pw_pdf::execute;
use std::path::{Path, PathBuf};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/pdf").join(name)
}

fn export(input: &Path, pages: Option<&str>, output: &Path) -> pw_core::PwResult<pw_core::ops::OperationResult> {
    execute(
        &OperationRequest::new(Operation::ExportText {
            input: input.into(),
            pages: pages.map(Into::into),
            output: output.into(),
        }),
        None,
    )
}

#[test]
fn all_pages_in_order_with_separators_and_crlf() {
    let src = fixture("five-pages.pdf");
    let dir = tempfile::tempdir().expect("tmp");
    let out = dir.path().join("t.txt");
    let r = export(&src, None, &out).expect("export");
    assert_eq!(r.data["pages"], 5);
    assert_eq!(r.data["pages_without_text"], 0);
    assert!(r.warnings.is_empty(), "{:?}", r.warnings);
    let bytes = std::fs::read(&out).expect("read");
    let text = String::from_utf8(bytes).expect("utf-8");
    assert!(!text.starts_with('\u{feff}'), "no byte-order mark");
    // Every line break is CRLF: no bare LF or CR.
    let b = text.as_bytes();
    for (i, &c) in b.iter().enumerate() {
        if c == b'\n' {
            assert_eq!(b.get(i.wrapping_sub(1)), Some(&b'\r'), "bare LF at {i}");
        }
        if c == b'\r' {
            assert_eq!(b.get(i + 1), Some(&b'\n'), "bare CR at {i}");
        }
    }
    let blocks: Vec<&str> = text.split('\u{c}').collect();
    assert_eq!(blocks.len(), 5, "one form feed between pages");
    for (i, block) in blocks.iter().enumerate() {
        let block = block.trim_start_matches("\r\n");
        assert!(block.starts_with(&format!("--- Page {} ---\r\n", i + 1)), "block {i}: {block:?}");
        assert!(block.contains(&format!("PWMARK00{}", i + 1)), "block {i} text: {block:?}");
    }
}

#[test]
fn chosen_pages_only() {
    let dir = tempfile::tempdir().expect("tmp");
    let out = dir.path().join("t.txt");
    let r = export(&fixture("five-pages.pdf"), Some("2,4-5"), &out).expect("export");
    assert_eq!(r.data["pages"], 3);
    let text = std::fs::read_to_string(&out).expect("read");
    for n in [2, 4, 5] {
        assert!(text.contains(&format!("--- Page {n} ---")) && text.contains(&format!("PWMARK00{n}")), "{text:?}");
    }
    for n in [1, 3] {
        assert!(!text.contains(&format!("PWMARK00{n}")), "page {n} must be left out: {text:?}");
    }
}

#[test]
fn scanned_pages_warn_that_ocr_is_needed() {
    let dir = tempfile::tempdir().expect("tmp");
    let out = dir.path().join("t.txt");
    let r = export(&fixture("scanned.pdf"), None, &out).expect("export");
    assert_eq!(r.data["pages_without_text"], r.data["pages"]);
    assert!(r.warnings.iter().any(|w| w.contains("OCR")), "{:?}", r.warnings);
    assert!(out.exists(), "the (empty-text) file is still written");
}

#[test]
fn bad_ranges_corrupt_input_and_existing_output_are_refused_without_writing() {
    let dir = tempfile::tempdir().expect("tmp");
    let out = dir.path().join("t.txt");
    assert!(export(&fixture("five-pages.pdf"), Some("9"), &out).is_err());
    assert!(!out.exists());
    assert!(export(&fixture("corrupt-truncated.pdf"), None, &out).is_err());
    assert!(!out.exists());
    std::fs::write(&out, b"keep me").expect("write");
    assert!(export(&fixture("five-pages.pdf"), None, &out).is_err(), "default policy never overwrites");
    assert_eq!(std::fs::read(&out).expect("read"), b"keep me");
}

#[test]
fn output_may_not_replace_the_source() {
    let dir = tempfile::tempdir().expect("tmp");
    let src = dir.path().join("in.pdf");
    std::fs::copy(fixture("five-pages.pdf"), &src).expect("copy");
    let before = std::fs::read(&src).expect("read");
    assert!(export(&src, None, &src).is_err());
    assert_eq!(std::fs::read(&src).expect("read"), before);
}
