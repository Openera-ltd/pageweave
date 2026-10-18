//! Turning scanned pages upright, against turned-scan.pdf (tools/fixtures/gen_scanned.py): pages
//! 1–3 are stored turned and need /Rotate 90, 180 and 270; page 4 is upright; page 5 has text.
use pw_core::ops::{Operation, OperationRequest, OperationResult};
use pw_pdf::execute;
use std::path::{Path, PathBuf};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/pdf").join(name)
}

fn run(input: &Path, pages: Option<&str>, output: &Path) -> pw_core::PwResult<OperationResult> {
    execute(
        &OperationRequest::new(Operation::FixOrientation {
            input: input.into(),
            pages: pages.map(Into::into),
            output: output.into(),
        }),
        None,
    )
}

fn rotations(p: &Path) -> Vec<i64> {
    let doc = lopdf::Document::load(p).expect("lopdf");
    doc.get_pages()
        .values()
        .map(|id| {
            doc.get_dictionary(*id).ok().and_then(|d| d.get(b"Rotate").ok()).and_then(|r| r.as_i64().ok()).unwrap_or(0)
        })
        .collect()
}

#[test]
fn turned_scans_are_turned_upright_and_the_rest_left_alone() {
    let src = fixture("turned-scan.pdf");
    let before = std::fs::read(&src).expect("read");
    let dir = tempfile::tempdir().expect("tmp");
    let out = dir.path().join("upright.pdf");
    let r = run(&src, None, &out).expect("orient");
    assert_eq!(std::fs::read(&src).expect("read"), before, "source untouched");
    assert_eq!(r.data["turned"], 3);
    let actions: Vec<&str> =
        r.data["pages"].as_array().expect("pages").iter().map(|p| p["action"].as_str().unwrap_or("?")).collect();
    assert_eq!(actions, vec!["turned", "turned", "turned", "upright", "has_text"]);
    // Independent check with lopdf: the page dictionaries carry the correcting rotations.
    assert_eq!(rotations(&out), vec![90, 180, 270, 0, 0]);
    // Turning the corrected copy again finds nothing to do.
    let again = dir.path().join("again.pdf");
    let e = run(&out, None, &again).expect_err("already upright");
    assert_eq!(e.code, "orient.nothing");
    assert!(!again.exists());
}

#[test]
fn chosen_pages_only_and_bad_input() {
    let dir = tempfile::tempdir().expect("tmp");
    let out = dir.path().join("o.pdf");
    let r = run(&fixture("turned-scan.pdf"), Some("2"), &out).expect("orient");
    assert_eq!(r.data["turned"], 1);
    assert_eq!(rotations(&out), vec![0, 180, 0, 0, 0]);
    let none = dir.path().join("none.pdf");
    assert_eq!(run(&fixture("turned-scan.pdf"), Some("4-5"), &none).expect_err("nothing").code, "orient.nothing");
    assert!(run(&fixture("turned-scan.pdf"), Some("9"), &none).is_err());
    assert!(run(&fixture("corrupt-truncated.pdf"), None, &none).is_err());
    assert!(!none.exists());
}
