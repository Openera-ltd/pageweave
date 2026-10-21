//! Pattern search for redaction against the real engine, then the found boxes are redacted and the
//! values must be gone from the copy (independent lopdf text extraction and PDFium search).

use pw_core::ops::{Operation, OperationRequest, OperationResult, RectPt, RedactArea, TextPattern};
use pw_core::PwResult;
use pw_pdf::{execute, PatternReport};
use std::path::{Path, PathBuf};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/pdf").join(name)
}

fn run(op: Operation) -> PwResult<OperationResult> {
    execute(&OperationRequest::new(op), None)
}

fn find(patterns: Vec<TextPattern>, custom: Option<&str>) -> PatternReport {
    let r = run(Operation::FindPatterns {
        input: fixture("sensitive-info.pdf"),
        patterns,
        custom: custom.map(str::to_string),
        pages: None,
    })
    .expect("find");
    serde_json::from_value(r.data).expect("report")
}

#[test]
fn finds_each_kind_once_and_not_the_look_alikes() {
    use TextPattern::*;
    let r = find(vec![Email, Phone, CardNumber, Iban, UsSsn], None);
    let got: Vec<(u32, &str, &str)> = r.hits.iter().map(|h| (h.page, h.kind.as_str(), h.text.as_str())).collect();
    assert_eq!(
        got,
        [
            (0, "email", "ada.lovelace@example.org"),
            (0, "phone", "+44 20 7946 0958"),
            (0, "card_number", "4111 1111 1111 1111"),
            (0, "iban", "GB82 WEST 1234 5698 7654 32"),
            (0, "us_ssn", "123-45-6789"),
            (1, "email", "grace@example.com"),
        ],
        "dates, short numbers and the card failing Luhn are not matched; IBAN digits are not also a phone"
    );
    for h in &r.hits {
        assert!(!h.rects.is_empty());
        for b in &h.rects {
            assert!(b.right > b.left && b.top > b.bottom, "{h:?}");
        }
    }
    // The turned page reports boxes in unrotated page space, inside the page (612 × 792).
    let turned = &r.hits[5].rects[0];
    assert!(turned.left >= 0.0 && turned.right <= 612.0 && turned.top <= 792.0, "{turned:?}");
}

#[test]
fn custom_patterns_and_refusals() {
    let r = find(vec![], Some(r"invoice \d+"));
    assert_eq!(r.hits.len(), 1);
    assert_eq!((r.hits[0].kind.as_str(), r.hits[0].text.as_str()), ("custom", "invoice 12345"));
    let code = |op| run(op).expect_err("must fail").code;
    let op = |patterns: Vec<TextPattern>, custom: Option<&str>| Operation::FindPatterns {
        input: fixture("sensitive-info.pdf"),
        patterns,
        custom: custom.map(str::to_string),
        pages: None,
    };
    assert_eq!(code(op(vec![], None)), "pattern.none");
    assert_eq!(code(op(vec![], Some("   "))), "pattern.none");
    assert_eq!(code(op(vec![], Some("(bad"))), "pattern.invalid");
    assert_eq!(code(op(vec![], Some(&"a".repeat(501)))), "pattern.too_long");
}

#[test]
fn found_boxes_redact_the_values_out_of_the_copy() {
    use TextPattern::*;
    let r = find(vec![Email, Phone, CardNumber, Iban, UsSsn], None);
    let areas: Vec<RedactArea> = r
        .hits
        .iter()
        .flat_map(|h| {
            h.rects.iter().map(move |b| RedactArea {
                page: h.page,
                rect: RectPt { left: b.left, bottom: b.bottom, right: b.right, top: b.top },
            })
        })
        .collect();
    let dir = tempfile::tempdir().expect("tmp");
    let out = dir.path().join("redacted.pdf");
    run(Operation::Redact { input: fixture("sensitive-info.pdf"), areas, output: out.clone() }).expect("redact");
    let doc = lopdf::Document::load(&out).expect("lopdf");
    let text: String = doc.get_pages().keys().map(|n| doc.extract_text(&[*n]).unwrap_or_default()).collect();
    for secret in ["lovelace@", "7946", "4111 1111 1111 1111", "WEST", "123-45-6789", "grace@"] {
        assert!(!text.contains(secret), "{secret} left in: {text}");
    }
    assert!(text.contains("PWSENS") && text.contains("4111 1111 1111 1112"), "other text kept: {text}");
    let again: PatternReport = serde_json::from_value(
        run(Operation::FindPatterns { input: out, patterns: vec![Email, UsSsn], custom: None, pages: None })
            .expect("find")
            .data,
    )
    .expect("report");
    assert!(again.hits.is_empty(), "{:?}", again.hits);
}
