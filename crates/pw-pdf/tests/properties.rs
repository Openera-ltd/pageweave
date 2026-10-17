//! Editing document properties against the real engine; results read back by PDFium and lopdf.
use pw_core::ops::{Operation, OperationRequest};
use pw_core::ErrorCategory;
use pw_pdf::{execute, open_document, OpenOptions};
use std::path::{Path, PathBuf};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/pdf").join(name)
}

fn set(
    input: &Path,
    output: &Path,
    t: Option<&str>,
    a: Option<&str>,
    s: Option<&str>,
    k: Option<&str>,
) -> pw_core::PwResult<pw_core::ops::OperationResult> {
    execute(
        &OperationRequest::new(Operation::SetProperties {
            input: input.into(),
            title: t.map(Into::into),
            author: a.map(Into::into),
            subject: s.map(Into::into),
            keywords: k.map(Into::into),
            output: output.into(),
        }),
        None,
    )
}

fn everything(path: &Path) -> String {
    let doc = lopdf::Document::load(path).expect("lopdf");
    let mut all = Vec::new();
    for obj in doc.objects.values() {
        match obj {
            lopdf::Object::Stream(s) => all.extend(s.decompressed_content().unwrap_or_else(|_| s.content.clone())),
            other => all.extend(format!("{other:?}").into_bytes()),
        }
    }
    String::from_utf8_lossy(&all).into_owned()
}

#[test]
fn properties_change_and_everything_else_stays() {
    let src = fixture("five-pages.pdf");
    let before = std::fs::read(&src).expect("read");
    let dir = tempfile::tempdir().expect("tmp");
    let out = dir.path().join("props.pdf");
    let title = "Quarterly report — Überblick 文書";
    let r = set(&src, &out, Some(title), Some(""), Some("Finance"), None).expect("set");
    assert_eq!(r.outputs[0].pages, Some(5));
    assert_eq!(std::fs::read(&src).expect("read"), before, "source untouched");
    let info = open_document(&out, &OpenOptions { password: None }).expect("open");
    assert_eq!(info.title.as_deref(), Some(title));
    assert_eq!(info.author, None, "empty removes");
    assert_eq!(info.subject.as_deref(), Some("Finance"));
    assert_eq!(info.keywords, None, "unchanged (none before)");
    assert_eq!(info.bookmarks.len(), 5, "outline kept");
    assert_eq!(info.producer.as_deref(), Some("gen_fixtures.py"), "other properties kept");
    // Independent parser: the Info dictionary holds the new values.
    let doc = lopdf::Document::load(&out).expect("lopdf");
    let id = doc.trailer.get(b"Info").and_then(|o| o.as_reference()).expect("info");
    let d = doc.get_dictionary(id).expect("dict");
    assert_eq!(lopdf::decode_text_string(d.get(b"Title").expect("title")).expect("text"), title);
    assert!(d.get(b"Author").is_err());
    assert!(d.get(b"ModDate").is_ok());
}

#[test]
fn xmp_follows_and_old_values_are_gone() {
    let src = fixture("hidden-info.pdf");
    let dir = tempfile::tempdir().expect("tmp");
    let out = dir.path().join("props.pdf");
    let r = set(&src, &out, Some("New <title> & more"), Some("New author"), None, None).expect("set");
    assert!(r.warnings.iter().any(|w| w.contains("XMP")), "{:?}", r.warnings);
    assert!(r.warnings.iter().any(|w| w.contains("Earlier saved versions")), "{:?}", r.warnings);
    let all = everything(&out);
    assert!(all.contains("New &lt;title&gt; &amp; more"), "XMP carries the escaped title");
    assert!(all.contains("<pdfaid:part>2</pdfaid:part>"), "PDF/A identification kept");
    for gone in ["PWMETA-TITLE", "PWMETA-AUTHOR", "PWMETA-XMP", "PWOLD-REVISION"] {
        assert!(!all.contains(gone), "{gone} survived");
    }
    assert!(all.contains("PWMETA-CUSTOM"), "custom properties are not touched");
    let info = open_document(&out, &OpenOptions { password: None }).expect("open");
    assert_eq!(info.title.as_deref(), Some("New <title> & more"));
}

#[test]
fn bad_requests_write_nothing() {
    let dir = tempfile::tempdir().expect("tmp");
    let out = dir.path().join("x.pdf");
    let src = fixture("five-pages.pdf");
    assert_eq!(set(&src, &out, None, None, None, None).expect_err("empty").code, "properties.empty");
    assert_eq!(set(&src, &out, Some("a\nb"), None, None, None).expect_err("newline").code, "properties.control_chars");
    let long = "x".repeat(2001);
    assert_eq!(set(&src, &out, Some(&long), None, None, None).expect_err("long").code, "properties.too_long");
    let mut req = OperationRequest::new(Operation::SetProperties {
        input: fixture("encrypted-rc4-user-password.pdf"),
        title: Some("t".into()),
        author: None,
        subject: None,
        keywords: None,
        output: out.clone(),
    });
    req.password = Some("pw-user-test".into());
    assert_eq!(execute(&req, None).expect_err("protected").code, "properties.protected");
    assert!(!out.exists());
    std::fs::write(&out, b"keep").expect("write");
    let err = set(&src, &out, Some("t"), None, None, None).expect_err("exists");
    assert_eq!(err.category, ErrorCategory::OutputConflict);
    assert_eq!(std::fs::read(&out).expect("read"), b"keep");
}

#[test]
fn rewriting_a_signed_file_warns_that_signatures_no_longer_hold() {
    let src = fixture("signature-structure.pdf");
    assert_eq!(open_document(&src, &OpenOptions { password: None }).expect("open").signature_count, 1);
    let dir = tempfile::tempdir().expect("tmp");
    let out = dir.path().join("p.pdf");
    let r = set(&src, &out, Some("New"), None, None, None).expect("set");
    assert!(r.warnings.iter().any(|w| w.contains("digital signature")), "{:?}", r.warnings);
    let out2 = dir.path().join("s.pdf");
    let r = execute(
        &OperationRequest::new(Operation::Sanitize {
            input: src.clone(),
            options: pw_core::ops::SanitizeOptions::default(),
            output: out2,
        }),
        None,
    )
    .expect("sanitize");
    assert!(r.warnings.iter().any(|w| w.contains("digital signature")), "{:?}", r.warnings);
}
