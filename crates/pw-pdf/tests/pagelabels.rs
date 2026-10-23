//! Page labels: written, read back by PDFium (the app) and lopdf (independent), replaced, removed.

use pw_core::ops::{LabelStyle, Operation, OperationRequest, OperationResult, PageLabelRange};
use pw_core::PwResult;
use pw_pdf::{execute, open_document, OpenOptions};
use std::path::{Path, PathBuf};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/pdf").join(name)
}

fn set(input: PathBuf, ranges: Vec<PageLabelRange>, output: &Path) -> PwResult<OperationResult> {
    execute(&OperationRequest::new(Operation::SetPageLabels { input, ranges, output: output.into() }), None)
}

fn r(start: u32, style: LabelStyle, prefix: &str, first: u32) -> PageLabelRange {
    PageLabelRange { start, style, prefix: prefix.into(), first }
}

fn labels(p: &Path) -> Vec<Option<String>> {
    open_document(p, &OpenOptions::default()).expect("open").pages.into_iter().map(|pg| pg.label).collect()
}

#[test]
fn front_matter_roman_then_decimal_with_a_prefixed_appendix() {
    let dir = tempfile::tempdir().expect("tmp");
    let out = dir.path().join("labelled.pdf");
    set(
        fixture("twenty-pages.pdf"),
        vec![
            r(0, LabelStyle::RomanLower, "", 1),
            r(4, LabelStyle::Decimal, "", 1),
            r(16, LabelStyle::Decimal, "A-", 1),
        ],
        &out,
    )
    .expect("labels");
    let got: Vec<String> = labels(&out).into_iter().map(|l| l.unwrap_or_default()).collect();
    assert_eq!(&got[..6], ["i", "ii", "iii", "iv", "1", "2"]);
    assert_eq!(got[15], "12");
    assert_eq!(&got[16..], ["A-1", "A-2", "A-3", "A-4"]);

    // Independent: the number tree as lopdf reads it.
    let doc = lopdf::Document::load(&out).expect("lopdf");
    let cat = doc.catalog().expect("catalog");
    let tree = cat.get(b"PageLabels").and_then(lopdf::Object::as_dict).expect("PageLabels");
    let nums = tree.get(b"Nums").and_then(lopdf::Object::as_array).expect("Nums");
    assert_eq!(nums.len(), 6);
    assert_eq!(nums[2].as_i64().ok(), Some(4));
    let third = nums[5].as_dict().expect("dict");
    assert_eq!(lopdf::decode_text_string(third.get(b"P").expect("P")).ok().as_deref(), Some("A-"));

    // Relabel the labelled copy, then remove labels altogether.
    let out2 = dir.path().join("relabelled.pdf");
    set(out.clone(), vec![r(0, LabelStyle::LettersUpper, "Plate ", 1)], &out2).expect("relabel");
    assert_eq!(labels(&out2)[1].as_deref(), Some("Plate B"));
    let out3 = dir.path().join("unlabelled.pdf");
    set(out2, vec![], &out3).expect("remove");
    let doc = lopdf::Document::load(&out3).expect("lopdf");
    assert!(doc.catalog().expect("catalog").get(b"PageLabels").is_err(), "labels removed");
    assert!(labels(&out3).iter().all(|l| l.as_deref().unwrap_or("").is_empty()), "{:?}", labels(&out3));
}

#[test]
fn refusals_are_specific() {
    let dir = tempfile::tempdir().expect("tmp");
    let out = dir.path().join("x.pdf");
    let code = |ranges| set(fixture("five-pages.pdf"), ranges, &out).expect_err("must fail").code;
    assert_eq!(code(vec![r(1, LabelStyle::Decimal, "", 1)]), "labels.first_page");
    assert_eq!(code(vec![r(0, LabelStyle::Decimal, "", 1), r(0, LabelStyle::RomanLower, "", 1)]), "labels.order");
    assert_eq!(code(vec![r(0, LabelStyle::Decimal, "", 0)]), "labels.first");
    assert_eq!(code(vec![r(0, LabelStyle::None, " ", 1)]), "labels.empty");
    assert_eq!(
        code(vec![r(0, LabelStyle::Decimal, "", 1), r(9, LabelStyle::Decimal, "", 1)]),
        "labels.page_out_of_range"
    );
    assert_eq!(code(vec![r(0, LabelStyle::Decimal, &"x".repeat(51), 1)]), "labels.prefix");
    assert!(!out.exists());
}
