//! OCR into a searchable copy, against the real engine and models (fetched by tools/fetch_ocr.py).
//! Skipped with a message when the OCR resources are not installed.
use pw_core::ops::{Operation, OperationRequest};
use pw_pdf::{execute, render_page_png, search, OpenOptions};
use std::path::{Path, PathBuf};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/pdf").join(name)
}

fn have_resources() -> bool {
    let ok = pw_ocr::default_resource_dir().is_some();
    // In CI a missing resource is a failure, never a silent skip.
    assert!(ok || std::env::var_os("CI").is_none(), "OCR resources missing in CI (tools/fetch_ocr.py)");
    if !ok {
        eprintln!("OCR resources not installed (python3 tools/fetch_ocr.py); skipping");
    }
    ok
}

fn page_text(path: &Path, page: u32) -> String {
    let r = execute(
        &OperationRequest::new(Operation::ExtractText { input: path.into(), pages: Some(format!("{}", page + 1)) }),
        None,
    )
    .expect("text");
    r.data["pages"][0]["text"].as_str().unwrap_or_default().to_string()
}

fn words(s: &str) -> Vec<String> {
    s.split_whitespace()
        .map(|w| w.trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase())
        .filter(|w| w.len() > 1)
        .collect()
}

#[test]
fn scanned_pages_become_searchable_and_look_the_same() {
    if !have_resources() {
        return;
    }
    let src = fixture("scanned.pdf");
    let truth: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(fixture("scanned.truth.json")).expect("ok")).expect("ok");
    let dir = tempfile::tempdir().expect("ok");
    let out = dir.path().join("searchable.pdf");
    let r = execute(
        &OperationRequest::new(Operation::OcrPages {
            input: src.clone(),
            pages: None,
            skip_text_pages: true,
            dpi: 200,
            output: out.clone(),
        }),
        None,
    )
    .expect("ocr");
    assert_eq!(r.data["pages_processed"], 3, "{}", r.data);
    assert!(r.data["lines"].as_u64().expect("ok") > 50, "{}", r.data);
    for p in truth["pages"].as_array().expect("ok") {
        let i = p["page"].as_u64().expect("ok") as u32;
        assert!(page_text(&src, i).trim().is_empty(), "fixture page {i} has no text before OCR");
        let got: std::collections::HashSet<String> = words(&page_text(&out, i)).into_iter().collect();
        let want: Vec<String> =
            p["lines"].as_array().expect("ok").iter().flat_map(|l| words(l.as_str().expect("ok"))).collect();
        let found = want.iter().filter(|w| got.contains(*w)).count();
        let recall = found as f64 / want.len() as f64;
        assert!(recall >= 0.95, "page {} ({}, rotate {}): word recall {recall:.3}", i + 1, p["condition"], p["rotate"]);
    }
    // Search hits for recognised text sit on ink in the page image (the layer is placed correctly),
    // on the upright page and on the page stored with /Rotate 90.
    let (hits, _) =
        search(&out, &OpenOptions { password: None }, "Description", true, false, &|| false).expect("search");
    let hit = hits.iter().find(|h| h.page == 0).expect("found on page 1");
    let (png, w, h) = render_page_png(&out, &OpenOptions { password: None }, 0, 1.0).expect("render");
    let img = image::load_from_memory(&png).expect("ok").to_luma8();
    let r0 = &hit.rects[0];
    let (x0, x1) = (r0.left.max(0.0) as u32, (r0.right as u32).min(w - 1));
    let (y0, y1) = (((h as f32) - r0.top).max(0.0) as u32, (((h as f32) - r0.bottom) as u32).min(h - 1));
    let dark =
        (y0..=y1).flat_map(|y| (x0..=x1).map(move |x| (x, y))).filter(|&(x, y)| img.get_pixel(x, y)[0] < 128).count();
    let area = ((x1 - x0 + 1) * (y1 - y0 + 1)) as usize;
    assert!(dark * 20 > area, "hit box {r0:?} covers ink ({dark} dark of {area})");
    // Whole-word search on the page stored turned (/Rotate 90), for words that end and start
    // consecutive lines there (they fused before lines were kept apart).
    for word in ["received", "across"] {
        let (hits, _) = search(&out, &OpenOptions { password: None }, word, false, true, &|| false).expect("search");
        assert!(hits.iter().any(|h| h.page == 2), "rotated page: whole word {word:?} found");
    }
}

#[test]
fn pages_with_text_are_left_alone() {
    if !have_resources() {
        return;
    }
    let dir = tempfile::tempdir().expect("ok");
    let out = dir.path().join("o.pdf");
    let r = execute(
        &OperationRequest::new(Operation::OcrPages {
            input: fixture("single-page.pdf"),
            pages: None,
            skip_text_pages: true,
            dpi: 200,
            output: out.clone(),
        }),
        None,
    )
    .expect("ocr");
    assert_eq!((r.data["pages_processed"].as_u64(), r.data["pages_skipped_with_text"].as_u64()), (Some(0), Some(1)));
    assert!(r.warnings.iter().any(|w| w.contains("already had text")), "{:?}", r.warnings);
    assert_eq!(page_text(&out, 0), page_text(&fixture("single-page.pdf"), 0));
}

#[test]
fn bad_requests_are_refused() {
    let dir = tempfile::tempdir().expect("ok");
    let err = execute(
        &OperationRequest::new(Operation::OcrPages {
            input: fixture("scanned.pdf"),
            pages: None,
            skip_text_pages: true,
            dpi: 50,
            output: dir.path().join("o.pdf"),
        }),
        None,
    )
    .expect_err("must fail");
    assert_eq!(err.code, "ocr.dpi");
    let err = execute(
        &OperationRequest::new(Operation::OcrPages {
            input: fixture("scanned.pdf"),
            pages: Some("9".into()),
            skip_text_pages: true,
            dpi: 200,
            output: dir.path().join("o.pdf"),
        }),
        None,
    )
    .expect_err("must fail");
    assert_eq!(err.category, pw_core::ErrorCategory::InvalidInput);
    assert_eq!(std::fs::read_dir(dir.path()).expect("ok").count(), 0);
}
