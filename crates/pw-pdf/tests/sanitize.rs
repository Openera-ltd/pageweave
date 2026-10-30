//! Sanitization against the real engine. Every marker in the fixture is searched for in the whole
//! output file (decompressed) with lopdf, an independent parser, not only through PDFium.

use pw_core::ops::{Operation, OperationRequest, SanitizeOptions};
use pw_core::ErrorCategory;
use pw_pdf::{execute, open_document, OpenOptions};
use std::path::{Path, PathBuf};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/pdf").join(name)
}

/// Every object of the file, streams decompressed where possible, plus the trailer.
fn everything(path: &Path) -> Vec<u8> {
    let doc = lopdf::Document::load(path).expect("lopdf parses the output");
    let mut all = Vec::new();
    for obj in doc.objects.values() {
        match obj {
            lopdf::Object::Stream(s) => {
                all.extend(s.decompressed_content().unwrap_or_else(|_| s.content.clone()));
                all.extend(format!("{:?}", s.dict).into_bytes());
            }
            other => all.extend(format!("{other:?}").into_bytes()),
        }
    }
    all.extend(format!("{:?}", doc.trailer).into_bytes());
    all
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

fn raw_contains(path: &Path, needle: &[u8]) -> bool {
    contains(&std::fs::read(path).expect("read"), needle)
}

fn sanitize(input: &Path, output: &Path, options: SanitizeOptions) -> pw_core::PwResult<pw_core::ops::OperationResult> {
    execute(&OperationRequest::new(Operation::Sanitize { input: input.into(), options, output: output.into() }), None)
}

fn page_text(path: &Path, page: u32) -> String {
    let r = execute(
        &OperationRequest::new(Operation::ExtractText { input: path.into(), pages: Some(format!("{page}")) }),
        None,
    )
    .expect("text");
    r.data["pages"][0]["text"].as_str().unwrap_or_default().to_string()
}

const HIDDEN_IMAGE: fn() -> Vec<u8> = || (0..48u32).map(|i| ((i * 17 + 9) % 256) as u8).collect();

const REMOVED_BY_DEFAULT: &[&str] = &[
    "PWMETA-TITLE",
    "PWMETA-AUTHOR",
    "PWMETA-CUSTOM",
    "PWMETA-XMP",
    "PWPIECE",
    "PWTHUMB2",
    "PWJS-DOC",
    "PWJS-OPEN",
    "PWJS-PAGE",
    "PWJS-FIELD",
    "PWJS-OUTLINE",
    "PWLAUNCH",
    "PWXFA",
    "PWATTACH-DOC",
    "PWATTACH-ANNOT",
    "PWLAYER-HIDDEN",
    "PWLAYER-XOBJ",
    "PWLAYER-ANNOT",
    "PWCOMMENT",
    "PWHIGHLIGHT",
    "PWOLD-REVISION",
    "Draft notes",
];

#[test]
fn inspection_lists_each_kind_of_hidden_information() {
    let r = execute(&OperationRequest::new(Operation::SanitizeInspect { input: fixture("hidden-info.pdf") }), None)
        .expect("inspect");
    let f = &r.data;
    let meta: Vec<&str> = f["metadata"].as_array().expect("list").iter().filter_map(|v| v.as_str()).collect();
    for m in
        ["Title", "Author", "PDF producer", "Custom properties", "XMP metadata", "Application data", "Page thumbnails"]
    {
        assert!(meta.contains(&m), "{m} missing from {meta:?}");
    }
    // Open-action, document, page, field and outline scripts, the Launch link, and XFA.
    assert_eq!(f["scripts"], 7, "{f}");
    assert_eq!(f["attachments"], 2, "{f}");
    let names: Vec<&str> = f["attachment_names"].as_array().expect("list").iter().filter_map(|v| v.as_str()).collect();
    assert!(names.contains(&"secret.txt") && names.contains(&"notes.txt"), "{names:?}");
    assert_eq!(f["layers"], 2);
    assert_eq!(f["hidden_layers"], serde_json::json!(["Draft notes"]));
    assert_eq!(f["comments"], 3, "note, highlight and the hidden square: {f}");
    assert_eq!(f["form_fields"], 1);
    assert_eq!(f["web_links"], 1);
    assert_eq!(f["earlier_versions"], true);

    let clean = execute(&OperationRequest::new(Operation::SanitizeInspect { input: fixture("five-pages.pdf") }), None)
        .expect("inspect");
    assert_eq!(clean.data["scripts"], 0);
    assert_eq!(clean.data["earlier_versions"], false);
    assert_eq!(clean.data["hidden_layers"], serde_json::json!([]));
}

#[test]
fn default_cleaning_removes_hidden_information_and_keeps_the_document() {
    let src = fixture("hidden-info.pdf");
    let before = std::fs::read(&src).expect("read");
    let all_src = everything(&src);
    for m in REMOVED_BY_DEFAULT {
        assert!(contains(&all_src, m.as_bytes()) || raw_contains(&src, m.as_bytes()), "fixture lacks {m}");
    }
    let dir = tempfile::tempdir().expect("tmp");
    let out = dir.path().join("clean.pdf");
    let r = sanitize(&src, &out, SanitizeOptions::default()).expect("sanitize");
    assert_eq!(std::fs::read(&src).expect("read"), before, "source untouched");
    assert_eq!(r.outputs[0].pages, Some(2));
    assert_eq!(r.data["removed"]["form_fields"], 0, "not selected, so not reported");
    assert!(r.warnings.iter().any(|w| w.contains("PDF/A")), "{:?}", r.warnings);
    assert!(r.warnings.iter().any(|w| w.contains("hidden layers")), "{:?}", r.warnings);

    let all = everything(&out);
    for m in REMOVED_BY_DEFAULT {
        assert!(!contains(&all, m.as_bytes()), "{m} survived in the parsed objects");
        assert!(!raw_contains(&out, m.as_bytes()), "{m} survived in the file bytes");
    }
    assert!(!contains(&all, &HIDDEN_IMAGE()), "image drawn only in the hidden layer survived");
    for kept in ["PWBODY", "PWLAYER-SHOWN", "PWPAGE2", "PWWEB", "PWFORMVALUE", "Chapter one", "Chapter two"] {
        assert!(contains(&all, kept.as_bytes()), "{kept} should stay");
    }
    let t1 = page_text(&out, 1);
    assert!(t1.contains("PWBODY") && t1.contains("PWLAYER-SHOWN"), "{t1}");
    assert!(!t1.contains("PWLAYER-HIDDEN") && !t1.contains("PWLAYER-XOBJ"), "{t1}");
    assert_eq!(page_text(&out, 2).trim(), "Second page PWPAGE2");

    // Structure: one version, no layers left, the page link and the field stay, outline kept.
    let doc = lopdf::Document::load(&out).expect("lopdf");
    assert!(doc.trailer.get(b"Prev").is_err() && doc.trailer.get(b"Info").is_err());
    let root = doc.catalog().expect("catalog");
    assert!(root.get(b"OCProperties").is_err() && root.get(b"OpenAction").is_err() && root.get(b"Metadata").is_err());
    assert!(root.get(b"AcroForm").is_ok() && root.get(b"Outlines").is_ok());
    let p1 = *doc.get_pages().get(&1).expect("page 1");
    let annots = doc.get_page_annotations(p1).expect("annots");
    let mut kinds: Vec<String> = annots
        .iter()
        .filter_map(|a| {
            a.get(b"Subtype").ok().and_then(|s| s.as_name().ok()).map(|s| String::from_utf8_lossy(s).into())
        })
        .collect();
    kinds.sort();
    assert_eq!(kinds, vec!["Link", "Link", "Widget"], "web link, page link and the field");
    let info = open_document(&out, &OpenOptions { password: None }).expect("PDFium opens it");
    assert_eq!(info.page_count, 2);
    assert_eq!(info.bookmarks.len(), 2);
}

#[test]
fn every_option_also_removes_fields_and_web_links() {
    let src = fixture("hidden-info.pdf");
    let dir = tempfile::tempdir().expect("tmp");
    let out = dir.path().join("all.pdf");
    let all_opts = SanitizeOptions { form_fields: true, web_links: true, ..SanitizeOptions::default() };
    let r = sanitize(&src, &out, all_opts).expect("sanitize");
    assert_eq!(r.data["removed"]["form_fields"], 1);
    assert_eq!(r.data["removed"]["web_links"], 1);
    let all = everything(&out);
    for m in REMOVED_BY_DEFAULT.iter().chain(&["PWFORMVALUE", "PWWEB"]) {
        assert!(!contains(&all, m.as_bytes()), "{m} survived");
    }
    let doc = lopdf::Document::load(&out).expect("lopdf");
    assert!(doc.catalog().expect("catalog").get(b"AcroForm").is_err());
    let p1 = *doc.get_pages().get(&1).expect("page 1");
    let annots = doc.get_page_annotations(p1).unwrap_or_default();
    assert_eq!(annots.len(), 1, "only the page link stays");
    assert!(page_text(&out, 1).contains("PWBODY"));
}

#[test]
fn only_the_selected_kinds_are_removed() {
    let src = fixture("hidden-info.pdf");
    let dir = tempfile::tempdir().expect("tmp");
    let out = dir.path().join("meta.pdf");
    let only_meta = SanitizeOptions {
        metadata: true,
        scripts: false,
        attachments: false,
        hidden_layers: false,
        comments: false,
        form_fields: false,
        web_links: false,
    };
    sanitize(&src, &out, only_meta).expect("sanitize");
    let all = everything(&out);
    for gone in ["PWMETA-TITLE", "PWMETA-XMP", "PWPIECE", "PWTHUMB2", "PWOLD-REVISION"] {
        assert!(!contains(&all, gone.as_bytes()), "{gone} survived");
    }
    for kept in ["PWJS-DOC", "PWJS-FIELD", "PWATTACH-DOC", "PWATTACH-ANNOT", "PWLAYER-HIDDEN", "PWCOMMENT", "PWXFA"] {
        assert!(contains(&all, kept.as_bytes()), "{kept} was not selected and must stay");
    }
    let doc = lopdf::Document::load(&out).expect("lopdf");
    assert!(doc.catalog().expect("catalog").get(b"OCProperties").is_ok(), "layers untouched");

    // Layers only: hidden content goes, comments stay (the hidden square goes with its layer).
    let out2 = dir.path().join("layers.pdf");
    let only_layers = SanitizeOptions { metadata: false, hidden_layers: true, ..only_meta };
    sanitize(&src, &out2, only_layers).expect("sanitize");
    let all = everything(&out2);
    for gone in ["PWLAYER-HIDDEN", "PWLAYER-XOBJ", "PWLAYER-ANNOT", "Draft notes", "PWOLD-REVISION"] {
        assert!(!contains(&all, gone.as_bytes()), "{gone} survived");
    }
    for kept in ["PWLAYER-SHOWN", "PWCOMMENT", "PWMETA-TITLE", "PWJS-DOC"] {
        assert!(contains(&all, kept.as_bytes()), "{kept} must stay");
    }
}

#[test]
fn files_without_hidden_information_clean_to_an_equivalent_copy() {
    for name in ["five-pages.pdf", "mixed-sizes-rotated.pdf", "redaction-hard.pdf", "form-fields.pdf"] {
        let src = fixture(name);
        let dir = tempfile::tempdir().expect("tmp");
        let out = dir.path().join("clean.pdf");
        let all_opts = SanitizeOptions { form_fields: true, web_links: true, ..SanitizeOptions::default() };
        let r = sanitize(&src, &out, all_opts).unwrap_or_else(|e| panic!("{name}: {e:?}"));
        let n = open_document(&src, &OpenOptions { password: None }).expect("src").page_count;
        assert_eq!(r.outputs[0].pages, Some(n), "{name}");
        assert_eq!(page_text(&out, 1), page_text(&src, 1), "{name}: text unchanged");
    }
    // The earlier revision of the redaction fixture is gone.
    let dir = tempfile::tempdir().expect("tmp");
    let out = dir.path().join("rev.pdf");
    sanitize(&fixture("redaction-hard.pdf"), &out, SanitizeOptions::default()).expect("sanitize");
    assert!(!contains(&everything(&out), b"OLDREV-SECRET"));
}

#[test]
fn protected_corrupt_and_conflicting_requests_write_nothing() {
    let dir = tempfile::tempdir().expect("tmp");
    let out = dir.path().join("x.pdf");
    let mut req = OperationRequest::new(Operation::Sanitize {
        input: fixture("encrypted-rc4-user-password.pdf"),
        options: SanitizeOptions::default(),
        output: out.clone(),
    });
    req.password = Some("pw-user-test".into());
    let err = execute(&req, None).expect_err("protected");
    assert_eq!(err.code, "sanitize.protected");
    let err = sanitize(&fixture("corrupt-truncated.pdf"), &out, SanitizeOptions::default()).expect_err("corrupt");
    assert_eq!(err.category, ErrorCategory::Corrupt);
    assert!(!out.exists());

    std::fs::write(&out, b"existing").expect("write");
    let err = sanitize(&fixture("hidden-info.pdf"), &out, SanitizeOptions::default()).expect_err("exists");
    assert_eq!(err.category, ErrorCategory::OutputConflict);
    assert_eq!(std::fs::read(&out).expect("read"), b"existing");
    let entries: Vec<_> = std::fs::read_dir(dir.path()).expect("dir").collect();
    assert_eq!(entries.len(), 1, "no partial files left");
}

#[test]
fn usage_rights_permissions_are_not_taken_for_attachments() {
    // Government forms carry /Perms /UR3 whose parameters have an /EF permission list. It is not
    // an embedded file: inspection must not count it, and cleaning must not fail on it.
    let dir = tempfile::tempdir().expect("tmp");
    let src = fixture("form-xfa-hybrid.pdf");
    let found =
        execute(&OperationRequest::new(Operation::SanitizeInspect { input: src.clone() }), None).expect("inspect");
    assert_eq!(found.data["attachments"], 0, "{}", found.data);
    let out = dir.path().join("clean.pdf");
    sanitize(&src, &out, SanitizeOptions::default()).expect("cleaned and verified");
    assert!(out.exists());
}
