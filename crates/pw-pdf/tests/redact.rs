//! Permanent redaction against the real engine. The output is examined with `lopdf` (an independent
//! parser) for any trace of the removed content, not only with PDFium.

use pw_core::ops::{Operation, OperationRequest, RectPt, RedactArea};
use pw_pdf::{execute, render_page_png, search, OpenOptions};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/pdf").join(name)
}

fn opts() -> OpenOptions<'static> {
    OpenOptions { password: None }
}

/// Rectangle of the first match of `text`, grown by `pad` points.
fn rect_of(path: &Path, text: &str, pad: f32) -> RedactArea {
    let (hits, _) = search(path, &opts(), text, true, false, &|| false).expect("search");
    let hit = hits.first().unwrap_or_else(|| panic!("{text} not found"));
    let r = &hit.rects[0];
    RedactArea {
        page: hit.page,
        rect: RectPt { left: r.left - pad, bottom: r.bottom - pad, right: r.right + pad, top: r.top + pad },
    }
}

fn area(page: u32, left: f32, bottom: f32, right: f32, top: f32) -> RedactArea {
    RedactArea { page, rect: RectPt { left, bottom, right, top } }
}

/// Every stream of the file, decompressed where lopdf can, concatenated with the other objects'
/// debug form: what an attacker with a PDF parser would see.
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

fn page_text(path: &Path, page: u32) -> String {
    let r = execute(
        &OperationRequest::new(Operation::ExtractText { input: path.into(), pages: Some(format!("{}", page + 1)) }),
        None,
    )
    .expect("text");
    r.data["pages"][0]["text"].as_str().unwrap_or_default().to_string()
}

fn pixel(path: &Path, page: u32, x_pt: f32, y_pt_from_top: f32) -> [u8; 3] {
    let (png, _, _) = render_page_png(path, &opts(), page, 1.0).expect("render");
    let img = image::load_from_memory(&png).expect("png").to_rgb8();
    img.get_pixel(x_pt as u32, y_pt_from_top as u32).0
}

#[test]
fn redaction_removes_content_and_keeps_the_rest() {
    let src = fixture("redaction.pdf");
    let before = Sha256::digest(std::fs::read(&src).expect("ok"));
    let dir = tempfile::tempdir().expect("ok");
    let out = dir.path().join("redacted.pdf");
    let areas = vec![
        rect_of(&src, "SECRET-4711", 0.5),
        rect_of(&src, "Fully hidden line PWHIDE", 1.0),
        area(0, 390.0, 290.0, 510.0, 545.0), // square, Im2, note
        area(0, 340.0, 380.0, 380.0, 480.0), // right part of Im1
        rect_of(&src, "Inside form", 0.5),   // form XObject: page becomes an image
        rect_of(&src, "PWROT", 0.5),
        rect_of(&src, "PWOFF", 0.5),
    ];
    assert_eq!(areas[4].page, 1);
    assert_eq!(areas[5].page, 2);
    assert_eq!(areas[6].page, 3);
    assert!(areas[6].rect.left > 100.0, "offset page: search reports user-space coordinates");

    // Preview: exactly the text that will go.
    let preview =
        execute(&OperationRequest::new(Operation::RedactionPreview { input: src.clone(), areas: areas.clone() }), None)
            .expect("preview");
    let pv = preview.data["areas"].as_array().expect("ok");
    let text_of = |i: usize| {
        pv.iter()
            .filter(|a| a["page"] == areas[i].page)
            .map(|a| a["text"].as_str().expect("ok").to_string())
            .collect::<Vec<_>>()
    };
    assert!(text_of(0).contains(&"SECRET-4711".to_string()), "{pv:?}");
    assert!(text_of(1).contains(&"Fully hidden line PWHIDE".to_string()), "{pv:?}");
    let a3 = pv
        .iter()
        .find(|a| a["page"] == 0 && a["images"] == 1 && a["annotations"] == 2)
        .unwrap_or_else(|| panic!("area 3 preview: {pv:?}"));
    assert!(a3["graphics"].as_u64().expect("ok") >= 2, "square and background: {a3:?}");
    assert_eq!(a3["graphics_kept"], 1, "the background crosses the edge and stays: {a3:?}");

    let r = execute(&OperationRequest::new(Operation::Redact { input: src.clone(), areas, output: out.clone() }), None)
        .expect("redact");
    assert_eq!(r.outputs[0].pages, Some(4));
    let warnings = r.warnings.join("\n");
    assert!(warnings.contains("Page 2 was replaced by an image"), "{warnings}");
    assert!(!warnings.contains("Page 1 "), "page 1 is redacted precisely: {warnings}");
    assert_eq!(Sha256::digest(std::fs::read(&src).expect("ok")), before, "source untouched");

    // Text: removed words gone, neighbours kept (PDFium extraction).
    let t1 = page_text(&out, 0);
    for keep in ["Name: Alice Example", "Account", "is closed", "Public footer text"] {
        assert!(t1.contains(keep), "kept {keep:?}: {t1:?}");
    }
    for gone in ["SECRET", "4711", "PWHIDE", "hidden"] {
        assert!(!t1.contains(gone), "removed {gone:?}: {t1:?}");
    }
    assert!(page_text(&out, 1).trim().is_empty(), "image page has no text");
    let t3 = page_text(&out, 2);
    assert!(t3.contains("Rotated page keeps this") && t3.contains("here") && !t3.contains("PWROT"), "{t3:?}");
    let t4 = page_text(&out, 3);
    assert!(t4.contains("Offset page stays") && t4.contains("secret") && !t4.contains("PWOFF"), "{t4:?}");

    // Independent parser over every object and stream: no trace of anything removed.
    let raw = everything(&out);
    for gone in [&b"SECRET"[..], b"PWHIDE", b"PWNOTE", b"PWLINK", b"PWTHUMB", b"PWFORMX", b"PWROT", b"PWOFF"] {
        assert!(!contains(&raw, gone), "{} found in the output", String::from_utf8_lossy(gone));
    }
    let im = |n: u32, k: u32, len: usize| (0..len).map(|i| ((i as u32 * n + k) % 256) as u8).collect::<Vec<u8>>();
    assert!(!contains(&raw, &im(7, 13, 192)), "image crossing an area: original pixels gone");
    assert!(!contains(&raw, &im(11, 5, 48)), "image inside an area: gone");
    assert!(contains(&raw, &im(3, 101, 48)), "image outside every area kept");
    let doc = lopdf::Document::load(&out).expect("ok");
    assert!(doc.trailer.get(b"Info").is_err() && doc.trailer.get(b"Prev").is_err());
    let root = doc.catalog().expect("ok");
    assert!(root.get(b"Outlines").is_err(), "bookmarks not carried over");
    // The link to page 2 survives and points into the document.
    let pages = doc.get_pages();
    let p1 = doc.get_dictionary(pages[&1]).expect("ok");
    let annots = p1.get(b"Annots").and_then(|a| a.as_array()).cloned().unwrap_or_default();
    assert_eq!(annots.len(), 1, "only the link outside the areas remains: {annots:?}");

    // Rendering: areas black, the rest as before.
    assert_eq!(pixel(&out, 0, 455.0, 792.0 - 310.0), [0, 0, 0], "area painted");
    assert_eq!(pixel(&src, 0, 120.0, 792.0 - 115.0), pixel(&out, 0, 120.0, 792.0 - 115.0), "shared image kept");
    let rot = doc.get_dictionary(pages[&3]).expect("ok").get(b"Rotate").and_then(|r| r.as_i64()).unwrap_or(0);
    assert_eq!(rot, 90, "rotation kept on a precisely redacted page");
}

#[test]
fn redaction_refuses_bad_requests_and_leaves_nothing() {
    let src = fixture("redaction.pdf");
    let dir = tempfile::tempdir().expect("ok");
    let out = dir.path().join("x.pdf");
    let err = execute(
        &OperationRequest::new(Operation::Redact {
            input: src.clone(),
            areas: vec![area(9, 0.0, 0.0, 10.0, 10.0)],
            output: out.clone(),
        }),
        None,
    )
    .expect_err("must fail");
    assert_eq!(err.code, "redact.page");
    let err = execute(
        &OperationRequest::new(Operation::Redact { input: src.clone(), areas: vec![], output: out.clone() }),
        None,
    )
    .expect_err("must fail");
    assert_eq!(err.code, "redact.empty");
    assert!(!out.exists());
    assert_eq!(std::fs::read_dir(dir.path()).expect("ok").count(), 0, "no staged files left");
}

#[test]
fn redacting_an_empty_area_still_paints_it_and_changes_nothing_else() {
    let src = fixture("single-page.pdf");
    let dir = tempfile::tempdir().expect("ok");
    let out = dir.path().join("o.pdf");
    let r = execute(
        &OperationRequest::new(Operation::Redact {
            input: src.clone(),
            areas: vec![area(0, 400.0, 100.0, 500.0, 150.0)],
            output: out.clone(),
        }),
        None,
    )
    .expect("redact");
    assert!(!r.warnings.iter().any(|w| w.contains("replaced by an image")), "{:?}", r.warnings);
    assert_eq!(page_text(&out, 0), page_text(&src, 0));
    assert_eq!(pixel(&out, 0, 450.0, 792.0 - 125.0), [0, 0, 0]);
}

#[test]
fn redaction_on_rotated_pages_forms_and_protected_files() {
    let dir = tempfile::tempdir().expect("ok");
    let run = |src: &Path, areas: Vec<RedactArea>, out: &Path, password: Option<&str>| {
        let mut req = OperationRequest::new(Operation::Redact { input: src.into(), areas, output: out.into() });
        req.password = password.map(Into::into);
        execute(&req, None)
    };

    // A word on a 90°-rotated page is removed precisely (no image fallback); its neighbours stay.
    let mixed = fixture("mixed-sizes-rotated.pdf");
    let out = dir.path().join("rot.pdf");
    let r = run(&mixed, vec![rect_of(&mixed, "PWMARK003", 0.5)], &out, None).expect("rotated");
    assert!(!r.warnings.iter().any(|w| w.contains("replaced by an image")), "{:?}", r.warnings);
    let t = page_text(&out, 2);
    assert!(!t.contains("PWMARK003") && t.contains("Marker token") && t.contains("lazy dog"), "{t:?}");

    // A form widget under an area is removed; the copy has no fillable fields.
    let form = fixture("form-fields.pdf");
    let info = pw_pdf::form_fields(&form, None).expect("fields");
    let w = &info.fields[0].widgets[0];
    let out = dir.path().join("form.pdf");
    let a = area(w.page, w.rect.left - 1.0, w.rect.bottom - 1.0, w.rect.right + 1.0, w.rect.top + 1.0);
    run(&form, vec![a], &out, None).expect("form");
    assert!(pw_pdf::form_fields(&out, None).expect("fields of copy").fields.is_empty());
    let doc = lopdf::Document::load(&out).expect("ok");
    let pages = doc.get_pages();
    let annots = doc
        .get_dictionary(pages[&1])
        .expect("ok")
        .get(b"Annots")
        .and_then(|a| a.as_array())
        .cloned()
        .unwrap_or_default();
    assert_eq!(annots.len(), info.fields.iter().map(|f| f.widgets.len()).sum::<usize>() - 1, "one widget removed");

    // Password-protected source: redacted copy is not protected, and says so.
    let enc = fixture("encrypted-rc4-user-password.pdf");
    let out = dir.path().join("enc.pdf");
    let r = run(&enc, vec![area(0, 60.0, 600.0, 400.0, 760.0)], &out, Some("pw-user-test")).expect("encrypted");
    assert!(r.warnings.iter().any(|w| w.contains("not password protected")), "{:?}", r.warnings);
}

#[test]
fn cancelled_redaction_writes_nothing() {
    let src = fixture("redaction.pdf");
    let dir = tempfile::tempdir().expect("ok");
    let out = dir.path().join("c.pdf");
    let err = pw_pdf::redact(&src, &opts(), &[rect_of(&src, "PWHIDE", 0.5)], &out, Default::default(), &|| true)
        .expect_err("must fail");
    assert_eq!(err.category, pw_core::ErrorCategory::Cancelled);
    assert_eq!(std::fs::read_dir(dir.path()).expect("ok").count(), 0);
}
