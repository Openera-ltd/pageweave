//! Document translation with the installed models: Word output keeps structure, text output per
//! page, language detection, translation through English, protected tokens, clear refusals.

use pw_core::ops::{Operation, OperationRequest, OperationResult};
use pw_core::output::CollisionPolicy;
use pw_core::{ErrorCategory, PwResult};
use pw_pdf::execute;
use std::io::Read;
use std::path::{Path, PathBuf};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/pdf").join(name)
}

fn translate(
    name: &str,
    pages: Option<&str>,
    source: Option<&str>,
    target: &str,
    output: &Path,
) -> PwResult<OperationResult> {
    let mut req = OperationRequest::new(Operation::TranslateDocument {
        input: fixture(name),
        pages: pages.map(str::to_string),
        source: source.map(str::to_string),
        target: target.into(),
        output: output.into(),
    });
    req.collision = CollisionPolicy::Fail;
    execute(&req, None)
}

#[test]
fn tables_to_word_and_text_with_detection_and_pivot() {
    let dir = tempfile::tempdir().expect("tmp");
    // English detected; Word keeps the two tables.
    let out = dir.path().join("t.docx");
    let r = translate("tables.pdf", None, None, "fr", &out).expect("translate");
    assert_eq!(r.data["source"], "en", "{}", r.data);
    assert_eq!(r.data["detected"], true);
    assert_eq!(r.data["tables"], 2, "{}", r.data);
    let mut z = zip::ZipArchive::new(std::fs::File::open(&out).expect("open")).expect("zip");
    let mut xml = String::new();
    z.by_name("word/document.xml").expect("doc").read_to_string(&mut xml).expect("utf8");
    // Values that must not change are kept; the sentences are French.
    for v in ["007", "2026-03-12", "1,234.50", "123456789012345678", "PWTABLES"] {
        assert!(xml.contains(v), "{v} kept");
    }
    assert!(xml.contains("trimestre"), "French text: {xml}");
    assert!(r.warnings.iter().any(|w| w.contains("Machine translation from English to French")));

    // Text output, page 1 only, given language; German to French goes through English.
    let txt = dir.path().join("p1.txt");
    let r = translate("five-pages.pdf", Some("1"), Some("en"), "de", &txt).expect("text");
    let t = std::fs::read_to_string(&txt).expect("txt");
    assert!(
        t.starts_with("--- Page 1 ---\r\n")
            && t.contains("PWMARK001")
            && t.contains("PageWeave")
            && t.to_lowercase().contains("fuchs"),
        "{t}"
    );
    assert_eq!(r.data["detected"], false);
    let via = dir.path().join("via.txt");
    std::fs::write(dir.path().join("de.txt"), "unused").expect("w");
    let r = translate("five-pages.pdf", Some("1"), Some("de"), "fr", &via).expect("pivot");
    assert_eq!(r.data["via_english"], true, "{}", r.data);
    assert!(r.warnings.iter().any(|w| w.contains("(through English)")));
}

#[test]
fn refusals_write_nothing() {
    let dir = tempfile::tempdir().expect("tmp");
    let out = dir.path().join("x.docx");
    assert_eq!(translate("scanned.pdf", None, None, "de", &out).expect_err("scan").code, "translate.no_text");
    assert_eq!(translate("five-pages.pdf", None, Some("en"), "en", &out).expect_err("same").code, "translate.same");
    assert_eq!(translate("five-pages.pdf", None, Some("en"), "ja", &out).expect_err("dir").code, "translate.direction");
    assert_eq!(
        translate("five-pages.pdf", None, None, "de", &dir.path().join("x.pdf")).expect_err("ext").code,
        "translate.extension"
    );
    let e = translate("encrypted-rc4-user-password.pdf", None, None, "de", &out).expect_err("pw");
    assert_eq!(e.category, ErrorCategory::PasswordRequired);
    assert!(!out.exists());
}
