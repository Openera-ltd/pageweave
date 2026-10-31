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
    translate_as(name, pages, source, target, output, false)
}

fn translate_as(
    name: &str,
    pages: Option<&str>,
    source: Option<&str>,
    target: &str,
    output: &Path,
    bilingual: bool,
) -> PwResult<OperationResult> {
    let mut req = OperationRequest::new(Operation::TranslateDocument {
        input: fixture(name),
        pages: pages.map(str::to_string),
        source: source.map(str::to_string),
        target: target.into(),
        output: output.into(),
        bilingual,
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
    // German → French is a direct pair (D-027): no route through English, nothing said about one.
    let r = translate("five-pages.pdf", Some("1"), Some("de"), "fr", &via).expect("direct");
    assert_eq!(r.data["via_english"], false, "{}", r.data);
    assert!(!r.warnings.iter().any(|w| w.contains("(through English)")));
}

#[test]
fn refusals_write_nothing() {
    let dir = tempfile::tempdir().expect("tmp");
    let out = dir.path().join("x.docx");
    assert_eq!(translate("scanned.pdf", None, None, "de", &out).expect_err("scan").code, "translate.no_text");
    assert_eq!(translate("five-pages.pdf", None, Some("en"), "en", &out).expect_err("same").code, "translate.same");
    assert_eq!(translate("five-pages.pdf", None, Some("en"), "ja", &out).expect_err("dir").code, "translate.direction");
    assert_eq!(
        translate("five-pages.pdf", None, None, "de", &dir.path().join("x.odt")).expect_err("ext").code,
        "translate.extension"
    );
    let e = translate("encrypted-rc4-user-password.pdf", None, None, "de", &out).expect_err("pw");
    assert_eq!(e.category, ErrorCategory::PasswordRequired);
    assert!(!out.exists());
}

/// Path drawing operators of a page (lines, rectangles, fills, strokes), counted with lopdf.
fn drawing_ops(doc: &lopdf::Document, page: u32) -> usize {
    let id = doc.get_pages()[&page];
    let content = doc.get_and_decode_page_content(id).expect("content");
    content
        .operations
        .iter()
        .filter(|o| matches!(o.operator.as_str(), "re" | "m" | "l" | "c" | "f" | "f*" | "S" | "B" | "b"))
        .count()
}

#[test]
fn pdf_output_keeps_the_layout_and_replaces_the_text() {
    let dir = tempfile::tempdir().expect("tmp");
    let out = dir.path().join("t.pdf");
    let r = translate("tables.pdf", None, Some("en"), "fr", &out).expect("translate");
    let src = lopdf::Document::load(fixture("tables.pdf")).expect("source");
    let doc = lopdf::Document::load(&out).expect("lopdf parses the translation");
    assert_eq!(doc.get_pages().len(), src.get_pages().len());
    // Drawings (the tables' rules and fills) are exactly as they were.
    for p in src.get_pages().keys() {
        assert_eq!(drawing_ops(&doc, *p), drawing_ops(&src, *p), "page {p} drawings");
    }
    let text = doc.extract_text(&[1]).expect("text");
    for v in ["007", "2026-03-12", "PWTABLES"] {
        assert!(text.contains(v), "{v} kept: {text}");
    }
    assert!(text.contains("trimestre"), "French text: {text}");
    let before = src.extract_text(&[1]).expect("text");
    assert!(before.contains("quarter") && !text.contains("quarter"), "English text replaced");
    assert!(r.warnings.iter().any(|w| w.contains("Machine translation from English to French")));

    // Columns stay columns: the first page's two halves keep their text apart.
    let out2 = dir.path().join("a.pdf");
    translate("article.pdf", Some("1"), Some("en"), "de", &out2).expect("article");
    let doc = lopdf::Document::load(&out2).expect("parse");
    let t = doc.extract_text(&[1]).expect("text");
    assert!(t.contains("PWARTICLE"), "name kept: {t}");
    // Page 2 was not asked for: unchanged.
    let src = lopdf::Document::load(fixture("article.pdf")).expect("src");
    assert_eq!(doc.extract_text(&[2]).expect("p2"), src.extract_text(&[2]).expect("p2"));
}

#[test]
fn bilingual_pdf_puts_each_page_next_to_its_translation() {
    let dir = tempfile::tempdir().expect("tmp");
    let out = dir.path().join("side.pdf");
    let o = pw_pdf::OpenOptions::default();
    // Pages 2 and 3 of the article: two sheets, each twice the page's width.
    translate_as("article.pdf", Some("2-3"), Some("en"), "de", &out, true).expect("bilingual");
    let info = pw_pdf::open_document(&out, &o).expect("open");
    let src = pw_pdf::open_document(&fixture("article.pdf"), &o).expect("src");
    assert_eq!(info.pages.len(), 2);
    for (sheet, page) in [(0u32, 1u32), (1, 2)] {
        let (s, p) = (&info.pages[sheet as usize], &src.pages[page as usize]);
        assert!((s.width_pt - 2.0 * p.width_pt).abs() < 0.5 && (s.height_pt - p.height_pt).abs() < 0.5, "sheet size");
        let half = |right: bool| {
            let t = pw_pdf::page_text_segments(&out, &o, sheet).expect("segments");
            let mid = s.width_pt / 2.0;
            t.segments
                .iter()
                .filter(|g| (g.left >= mid) == right)
                .map(|g| g.text.as_str())
                .collect::<Vec<_>>()
                .join(" ")
        };
        let (left, right) = (half(false), half(true));
        // The original's English on the left, the German on the right.
        let original = pw_pdf::page_text_segments(&fixture("article.pdf"), &o, page).expect("original");
        let lines: Vec<&str> =
            original.segments.iter().map(|g| g.text.trim()).filter(|t| t.len() > 20).take(4).collect();
        assert!(
            lines.iter().all(|l| left.contains(l)),
            "sheet {sheet} left half is the original: {lines:?} in {left:?}"
        );
        assert!(
            right.contains(" und ") || right.contains(" die ") || right.contains(" der "),
            "sheet {sheet} right half is German: {right:?}"
        );
        assert!(!right.contains("while upstream"), "sheet {sheet} right half is translated: {right:?}");
    }
    // Not for Word or text output.
    let e = translate_as("article.pdf", Some("1"), Some("en"), "de", &dir.path().join("x.docx"), true)
        .expect_err("refused");
    assert_eq!(e.code, "translate.bilingual");
}
