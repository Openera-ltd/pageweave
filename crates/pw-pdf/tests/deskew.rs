//! Straightening scanned pages against the real engine. skewed-scan.pdf pages carry the corpus'
//! recorded skew (tools/fixtures/gen_scanned.py): -2.55°, +2.77°, +1.05° (stored with /Rotate 90),
//! a straight scan and a born-digital text page.
use pw_core::ops::{Operation, OperationRequest, OperationResult};
use pw_pdf::{execute, OpenOptions};
use std::path::{Path, PathBuf};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/pdf").join(name)
}

fn run(input: &Path, pages: Option<&str>, output: &Path) -> pw_core::PwResult<OperationResult> {
    execute(
        &OperationRequest::new(Operation::Deskew {
            input: input.into(),
            pages: pages.map(Into::into),
            output: output.into(),
        }),
        None,
    )
}

#[test]
fn skewed_scans_are_straightened_and_the_rest_left_alone() {
    let src = fixture("skewed-scan.pdf");
    let before = std::fs::read(&src).expect("read");
    let dir = tempfile::tempdir().expect("tmp");
    let out = dir.path().join("straight.pdf");
    let r = run(&src, None, &out).expect("deskew");
    assert_eq!(std::fs::read(&src).expect("read"), before, "source untouched");
    assert_eq!(r.data["straightened"], 3);
    let pages = r.data["pages"].as_array().expect("pages");
    let actions: Vec<&str> = pages.iter().map(|p| p["action"].as_str().unwrap_or("?")).collect();
    assert_eq!(actions, vec!["straightened", "straightened", "straightened", "straight", "has_text"]);
    for (i, want) in [(0, -2.55f64), (1, 2.77), (2, 1.05)] {
        let got = pages[i]["angle"].as_f64().expect("angle");
        assert!((got - want).abs() <= 0.1, "page {}: want {want}, measured {got}", i + 1);
        let residual = pages[i]["residual"].as_f64().expect("residual");
        assert!(residual.abs() <= 0.3, "page {} residual {residual}", i + 1);
    }
    // Independent check: the corrected file has the same pages, and the text page is unchanged.
    let info = pw_pdf::open_document(&out, &OpenOptions { password: None }).expect("open");
    assert_eq!(info.page_count, 5);
    let text = |p: &Path| {
        let r =
            execute(&OperationRequest::new(Operation::ExtractText { input: p.into(), pages: Some("5".into()) }), None)
                .expect("text");
        r.data["pages"][0]["text"].as_str().unwrap_or_default().to_string()
    };
    assert!(text(&out).contains("PWDIGITAL"));
    assert_eq!(text(&out), text(&src));
}

#[test]
fn straightening_a_straightened_copy_finds_nothing_to_do() {
    let dir = tempfile::tempdir().expect("tmp");
    let once = dir.path().join("once.pdf");
    run(&fixture("skewed-scan.pdf"), None, &once).expect("first pass");
    let twice = dir.path().join("twice.pdf");
    let e = run(&once, None, &twice).expect_err("already straight");
    assert_eq!(e.code, "deskew.nothing");
    assert!(!twice.exists());
}

#[test]
fn chosen_pages_only_and_nothing_to_do_is_explained() {
    let dir = tempfile::tempdir().expect("tmp");
    let out = dir.path().join("o.pdf");
    let r = run(&fixture("skewed-scan.pdf"), Some("2"), &out).expect("deskew");
    assert_eq!(r.data["straightened"], 1);
    assert_eq!(r.data["pages"].as_array().expect("pages").len(), 1);
    let out2 = dir.path().join("none.pdf");
    let e = run(&fixture("skewed-scan.pdf"), Some("4-5"), &out2).expect_err("nothing");
    assert_eq!(e.code, "deskew.nothing");
    assert!(!out2.exists());
}

#[test]
fn bad_requests_and_cancellation_write_nothing() {
    let dir = tempfile::tempdir().expect("tmp");
    let out = dir.path().join("o.pdf");
    assert!(run(&fixture("skewed-scan.pdf"), Some("9"), &out).is_err());
    assert!(run(&fixture("corrupt-truncated.pdf"), None, &out).is_err());
    let e = pw_pdf::deskew::deskew_pages(
        &fixture("skewed-scan.pdf"),
        &OpenOptions { password: None },
        None,
        &out,
        Default::default(),
        &|| true,
        &|_, _| {},
    )
    .expect_err("cancelled");
    assert_eq!(e.category, pw_core::ErrorCategory::Cancelled);
    assert_eq!(std::fs::read_dir(dir.path()).expect("dir").count(), 0);
}
