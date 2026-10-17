//! Cropping against the real engine; crop boxes read back independently with lopdf.
use pw_core::ops::{Margins, Operation, OperationRequest};
use pw_pdf::{execute, open_document, OpenOptions};
use std::path::{Path, PathBuf};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/pdf").join(name)
}

fn crop(
    input: &Path,
    output: &Path,
    pages: Option<&str>,
    m: Margins,
) -> pw_core::PwResult<pw_core::ops::OperationResult> {
    execute(
        &OperationRequest::new(Operation::CropPages {
            input: input.into(),
            pages: pages.map(Into::into),
            margins: m,
            output: output.into(),
        }),
        None,
    )
}

/// CropBox of each page (or MediaBox when absent), as [l, b, r, t].
fn boxes(path: &Path) -> Vec<Vec<f32>> {
    let doc = lopdf::Document::load(path).expect("lopdf");
    doc.get_pages()
        .values()
        .map(|&id| {
            let d = doc.get_dictionary(id).expect("page");
            let a = d.get(b"CropBox").or_else(|_| d.get(b"MediaBox")).expect("box").as_array().expect("array").clone();
            a.iter().map(|o| o.as_float().or_else(|_| o.as_i64().map(|i| i as f32)).expect("num")).collect()
        })
        .collect()
}

#[test]
fn margins_apply_as_displayed_including_rotated_pages() {
    let src = fixture("mixed-sizes-rotated.pdf"); // page 2 is A4, page 3 has /Rotate 90
    let before = std::fs::read(&src).expect("read");
    let dir = tempfile::tempdir().expect("tmp");
    let out = dir.path().join("c.pdf");
    let m = Margins { top: 100.0, right: 10.0, bottom: 20.0, left: 30.0 };
    let r = crop(&src, &out, None, m).expect("crop");
    assert!(r.warnings.iter().any(|w| w.contains("hidden, not removed")), "{:?}", r.warnings);
    assert_eq!(std::fs::read(&src).expect("read"), before, "source untouched");
    let b = boxes(&out);
    assert_eq!(b[0], vec![30.0, 20.0, 602.0, 692.0], "letter page: l+30 b+20 r-10 t-100");
    assert_eq!(b[1], vec![30.0, 20.0, 585.0, 742.0], "A4 page");
    // /Rotate 90: displayed top is the page's left side, displayed right its top, and so on.
    assert_eq!(b[2], vec![100.0, 30.0, 592.0, 782.0], "rotated page");
    assert_eq!(open_document(&out, &OpenOptions { password: None }).expect("open").page_count, 3);
}

#[test]
fn chosen_pages_only_and_text_outside_stays_in_the_file() {
    let src = fixture("five-pages.pdf");
    let dir = tempfile::tempdir().expect("tmp");
    let out = dir.path().join("c.pdf");
    // Cut the top 200 pt (where the text is) from page 2 only.
    crop(&src, &out, Some("2"), Margins { top: 200.0, ..Margins::default() }).expect("crop");
    let b = boxes(&out);
    assert_eq!(b[1], vec![0.0, 0.0, 612.0, 592.0]);
    for (i, bx) in b.iter().enumerate() {
        if i != 1 {
            assert_eq!(bx, &vec![0.0, 0.0, 612.0, 792.0], "page {} unchanged", i + 1);
        }
    }
    let raw = lopdf::Document::load(&out).expect("lopdf");
    let mut found = false;
    for o in raw.objects.values() {
        if let lopdf::Object::Stream(s) = o {
            found |= String::from_utf8_lossy(&s.decompressed_content().unwrap_or_else(|_| s.content.clone()))
                .contains("PWMARK002");
        }
    }
    assert!(found, "cropping hides; the text is still in the file (hence the warning)");
}

#[test]
fn impossible_or_empty_crops_are_refused() {
    let dir = tempfile::tempdir().expect("tmp");
    let out = dir.path().join("x.pdf");
    let src = fixture("five-pages.pdf");
    assert_eq!(crop(&src, &out, None, Margins::default()).expect_err("empty").code, "crop.empty");
    assert_eq!(
        crop(&src, &out, None, Margins { top: -1.0, ..Margins::default() }).expect_err("neg").code,
        "crop.margin"
    );
    let err =
        crop(&src, &out, Some("3"), Margins { left: 300.0, right: 300.0, ..Margins::default() }).expect_err("too much");
    assert_eq!(err.code, "crop.too_much");
    assert!(err.message.contains("page 3"), "{}", err.message);
    assert!(!out.exists());
}
