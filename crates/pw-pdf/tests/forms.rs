//! AcroForm reading and filling against the real engine. Results are checked independently with
//! `lopdf` (field values, button states and regenerated appearance streams) and by rendering.

use pw_core::ops::{FieldValue, FormValue, Operation, OperationRequest};
use pw_core::output::CollisionPolicy;
use pw_core::ErrorCategory;
use pw_pdf::{execute, form_fields, render_page_png, FieldKind};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/pdf").join(name)
}

fn text(name: &str, v: &str) -> FieldValue {
    FieldValue { name: name.into(), value: FormValue::Text(v.into()) }
}

fn fill(values: Vec<FieldValue>, output: &Path) -> pw_core::PwResult<pw_core::ops::OperationResult> {
    let mut req =
        OperationRequest::new(Operation::FillForm { input: fixture("form-fields.pdf"), values, output: output.into() });
    req.collision = CollisionPolicy::Fail;
    execute(&req, None)
}

/// Decode a PDF text string (UTF-16BE with BOM, or PDFDocEncoding treated as Latin-1).
fn pdf_text(bytes: &[u8]) -> String {
    if bytes.starts_with(&[0xFE, 0xFF]) {
        let units: Vec<u16> =
            bytes[2..].chunks(2).map(|c| u16::from_be_bytes([c[0], *c.get(1).unwrap_or(&0)])).collect();
        String::from_utf16_lossy(&units)
    } else {
        bytes.iter().map(|&b| b as char).collect()
    }
}

/// Independent view of the form: name → (/V, [(/AS, decompressed /AP /N of the on/current state)]).
fn lopdf_fields(path: &Path) -> std::collections::HashMap<String, (String, Vec<(String, String)>)> {
    use lopdf::Object;
    let doc = lopdf::Document::load(path).expect("lopdf must parse the output");
    let catalog = doc.catalog().expect("catalog");
    let acro =
        doc.dereference(catalog.get(b"AcroForm").expect("AcroForm")).expect("deref").1.as_dict().expect("dict").clone();
    let fields = acro.get(b"Fields").and_then(Object::as_array).expect("Fields").clone();
    let mut out = std::collections::HashMap::new();
    let value_str = |o: &Object| match o {
        Object::String(s, _) => pdf_text(s),
        Object::Name(n) => String::from_utf8_lossy(n).into_owned(),
        _ => String::new(),
    };
    for f in fields {
        let id = f.as_reference().expect("ref");
        let d = doc.get_dictionary(id).expect("field");
        let name = d.get(b"T").map(value_str).unwrap_or_default();
        let v = d.get(b"V").map(value_str).unwrap_or_default();
        let widgets: Vec<lopdf::ObjectId> = match d.get(b"Kids").and_then(Object::as_array) {
            Ok(kids) => kids.iter().filter_map(|k| k.as_reference().ok()).collect(),
            Err(_) => vec![id],
        };
        let mut ws = Vec::new();
        for w in widgets {
            let wd = doc.get_dictionary(w).expect("widget");
            let state = wd.get(b"AS").map(value_str).unwrap_or_default();
            let normal = wd
                .get(b"AP")
                .ok()
                .and_then(|ap| doc.dereference(ap).ok())
                .and_then(|(_, o)| o.as_dict().ok().cloned())
                .and_then(|ap| ap.get(b"N").ok().cloned());
            let stream_id = match normal {
                Some(Object::Reference(r)) => Some(r),
                Some(Object::Dictionary(states)) => {
                    states.get(state.as_bytes()).ok().and_then(|o| o.as_reference().ok())
                }
                _ => None,
            };
            let content = stream_id
                .and_then(|r| doc.get_object(r).ok())
                .and_then(|o| o.as_stream().ok().cloned())
                .map(|mut s| {
                    let _ = s.decompress();
                    String::from_utf8_lossy(&s.content).into_owned()
                })
                .unwrap_or_default();
            ws.push((state, content));
        }
        out.insert(name, (v, ws));
    }
    out
}

#[test]
fn reads_every_field_of_the_fixture() {
    let info = form_fields(&fixture("form-fields.pdf"), None).expect("fields");
    assert!(!info.has_xfa);
    let names: Vec<&str> = info.fields.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, ["full_name", "subscribe", "plan", "country", "comments"]);
    let f = |n: &str| info.fields.iter().find(|f| f.name == n).expect(n);
    assert_eq!(f("full_name").kind, FieldKind::Text);
    assert_eq!(f("full_name").label.as_deref(), Some("Full name"), "/TU is the accessible label");
    assert!(!f("full_name").multiline);
    assert!(f("comments").multiline);
    assert_eq!(f("subscribe").kind, FieldKind::Checkbox);
    assert_eq!(f("subscribe").widgets[0].export_value.as_deref(), Some("On"), "on-state is /On, not /Yes");
    assert!(!f("subscribe").widgets[0].checked);
    assert_eq!(f("plan").kind, FieldKind::Radio);
    let exports: Vec<_> = f("plan").widgets.iter().map(|w| w.export_value.clone().unwrap_or_default()).collect();
    assert_eq!(exports, ["basic", "pro"]);
    assert_eq!(f("country").kind, FieldKind::Combo);
    assert_eq!(f("country").options, ["Canada", "Germany", "Japan"]);
    assert_eq!(f("country").value.as_deref(), Some("Canada"));
    let r = f("full_name").widgets[0].rect;
    assert_eq!((r.left, r.bottom, r.right, r.top), (150.0, 666.0, 400.0, 686.0));
    assert!(info.fields.iter().all(|f| f.widgets.iter().all(|w| w.page == 0)));
}

#[test]
fn documents_without_a_form_have_no_fields() {
    let info = form_fields(&fixture("five-pages.pdf"), None).expect("fields");
    assert!(info.fields.is_empty());
}

#[test]
fn fills_every_kind_and_regenerates_appearances() {
    let dir = tempfile::tempdir().expect("tmp");
    let out = dir.path().join("filled.pdf");
    let src_hash = Sha256::digest(std::fs::read(fixture("form-fields.pdf")).expect("read"));
    let r = fill(
        vec![
            text("full_name", "Ada Lovelace"),
            FieldValue { name: "subscribe".into(), value: FormValue::Checked(true) },
            text("plan", "pro"),
            text("country", "Japan"),
            text("comments", "Line one\nLine two – ünïcödé"),
        ],
        &out,
    )
    .expect("fill");
    assert_eq!(r.outputs.len(), 1);
    assert!(r.warnings.is_empty(), "{:?}", r.warnings);
    assert_eq!(Sha256::digest(std::fs::read(fixture("form-fields.pdf")).expect("read")), src_hash, "source untouched");

    // Engine view.
    let info = form_fields(&out, None).expect("reopen");
    let f = |n: &str| info.fields.iter().find(|f| f.name == n).expect(n).clone();
    assert_eq!(f("full_name").value.as_deref(), Some("Ada Lovelace"));
    assert!(f("subscribe").widgets[0].checked);
    assert_eq!(f("plan").widgets.iter().map(|w| w.checked).collect::<Vec<_>>(), [false, true]);
    assert_eq!(f("country").value.as_deref(), Some("Japan"));
    assert_eq!(f("comments").value.as_deref(), Some("Line one\nLine two – ünïcödé"));

    // Independent view: values, button states, and appearance streams that show the new values.
    let lf = lopdf_fields(&out);
    assert_eq!(lf["full_name"].0, "Ada Lovelace");
    assert!(lf["full_name"].1[0].1.contains("Ada"), "text appearance regenerated: {:?}", lf["full_name"].1[0].1);
    assert_eq!(lf["subscribe"].0, "On");
    assert_eq!(lf["subscribe"].1[0].0, "On", "checkbox appearance state switched to its own on-state");
    assert_eq!(lf["plan"].0, "pro");
    assert_eq!(lf["plan"].1.iter().map(|w| w.0.as_str()).collect::<Vec<_>>(), ["Off", "pro"]);
    assert_eq!(lf["country"].0, "Japan");
    assert!(lf["country"].1[0].1.contains("Japan"), "combo appearance regenerated");

    // Rendered: the name box now has ink where the original was blank.
    let ink = |path: &Path| {
        let (png, _, _) = render_page_png(path, &pw_pdf::OpenOptions { password: None }, 0, 1.0).expect("render");
        let img = image::load_from_memory(&png).expect("png").to_luma8();
        // full_name rect [150 666 400 686] at scale 1 → pixel rows 792-686..792-666.
        let mut dark = 0;
        for y in (792 - 684)..(792 - 668) {
            for x in 155..395 {
                if img.get_pixel(x, y).0[0] < 128 {
                    dark += 1;
                }
            }
        }
        dark
    };
    assert_eq!(ink(&fixture("form-fields.pdf")), 0, "fixture's name box starts empty");
    assert!(ink(&out) > 50, "filled name is drawn");
}

#[test]
fn clearing_and_unchecking_work() {
    let dir = tempfile::tempdir().expect("tmp");
    let first = dir.path().join("a.pdf");
    fill(
        vec![text("full_name", "Temp"), FieldValue { name: "subscribe".into(), value: FormValue::Checked(true) }],
        &first,
    )
    .expect("fill");
    let second = dir.path().join("b.pdf");
    let mut req = OperationRequest::new(Operation::FillForm {
        input: first.clone(),
        values: vec![text("full_name", ""), FieldValue { name: "subscribe".into(), value: FormValue::Checked(false) }],
        output: second.clone(),
    });
    req.collision = CollisionPolicy::Fail;
    execute(&req, None).expect("refill");
    let lf = lopdf_fields(&second);
    assert_eq!(lf["full_name"].0, "");
    assert_eq!(lf["subscribe"].1[0].0, "Off");
}

#[test]
fn invalid_requests_fail_before_anything_is_written() {
    let dir = tempfile::tempdir().expect("tmp");
    let out = dir.path().join("never.pdf");
    let code = |values| fill(values, &out).expect_err("must fail").code;
    assert_eq!(code(vec![text("no_such_field", "x")]), "form.unknown_field");
    assert_eq!(code(vec![text("subscribe", "yes")]), "form.value_type");
    assert_eq!(code(vec![FieldValue { name: "full_name".into(), value: FormValue::Checked(true) }]), "form.value_type");
    assert_eq!(code(vec![text("country", "Atlantis")]), "form.value_type");
    assert_eq!(code(vec![text("plan", "enterprise")]), "form.value_type");
    assert!(!out.exists());
    let leftovers: Vec<_> = std::fs::read_dir(dir.path()).expect("dir").collect();
    assert!(leftovers.is_empty(), "no staged files left behind");
    // Existing output is never replaced under the Fail policy.
    std::fs::write(&out, b"keep").expect("w");
    let e = fill(vec![text("full_name", "x")], &out).expect_err("conflict");
    assert_eq!(e.category, ErrorCategory::OutputConflict);
    assert_eq!(std::fs::read(&out).expect("r"), b"keep");
}

#[test]
fn form_work_and_rendering_can_run_concurrently() {
    let dir = tempfile::tempdir().expect("tmp");
    let dir_path = dir.path().to_path_buf();
    let mut handles = Vec::new();
    for t in 0..3 {
        let d = dir_path.clone();
        handles.push(std::thread::spawn(move || {
            for i in 0..5 {
                let out = d.join(format!("c-{t}-{i}.pdf"));
                fill(vec![text("full_name", &format!("T{t} {i}"))], &out).expect("fill");
                assert_eq!(
                    form_fields(&out, None).expect("read").fields[0].value.as_deref(),
                    Some(format!("T{t} {i}").as_str())
                );
            }
        }));
    }
    for _ in 0..3 {
        handles.push(std::thread::spawn(|| {
            for _ in 0..10 {
                render_page_png(&fixture("five-pages.pdf"), &pw_pdf::OpenOptions { password: None }, 0, 0.5)
                    .expect("render");
            }
        }));
    }
    for h in handles {
        h.join().expect("thread");
    }
}

#[test]
fn form_values_and_annotations_are_saved_together() {
    use pw_core::ops::{AnnotationKind, AnnotationSpec, RectPt};
    let dir = tempfile::tempdir().expect("tmp");
    let out = dir.path().join("both.pdf");
    let mut req = OperationRequest::new(Operation::ApplyEdits {
        input: fixture("form-fields.pdf"),
        form_values: vec![text("full_name", "Both Kinds")],
        annotations: vec![AnnotationSpec {
            page: 0,
            kind: AnnotationKind::Highlight,
            rects: vec![RectPt { left: 40.0, bottom: 735.0, right: 200.0, top: 750.0 }],
            color: "#ffd400".into(),
            contents: None,
        }],
        object_edits: vec![],
        annotation_changes: vec![],
        output: out.clone(),
    });
    req.collision = CollisionPolicy::Fail;
    let r = execute(&req, None).expect("apply");
    assert_eq!(r.operation, "apply_edits");
    let lf = lopdf_fields(&out);
    assert_eq!(lf["full_name"].0, "Both Kinds");
    let doc = lopdf::Document::load(&out).expect("lopdf");
    let page = *doc.get_pages().get(&1).expect("page 1");
    let annots = doc.get_page_annotations(page).expect("annots");
    let highlights =
        annots.iter().filter(|a| a.get(b"Subtype").and_then(|s| s.as_name()).is_ok_and(|n| n == b"Highlight")).count();
    assert_eq!(highlights, 1, "highlight added next to the filled widgets");
    // Annotations alone also work, and nothing at all is rejected.
    let only = dir.path().join("only-annots.pdf");
    let mut req = OperationRequest::new(Operation::ApplyEdits {
        input: fixture("single-page.pdf"),
        form_values: vec![],
        annotations: vec![AnnotationSpec {
            page: 0,
            kind: AnnotationKind::Underline,
            rects: vec![RectPt { left: 72.0, bottom: 700.0, right: 150.0, top: 712.0 }],
            color: "#1f6feb".into(),
            contents: None,
        }],
        object_edits: vec![],
        annotation_changes: vec![],
        output: only.clone(),
    });
    req.collision = CollisionPolicy::Fail;
    execute(&req, None).expect("annotations only");
    let empty = OperationRequest::new(Operation::ApplyEdits {
        input: fixture("single-page.pdf"),
        form_values: vec![],
        annotations: vec![],
        object_edits: vec![],
        annotation_changes: vec![],
        output: dir.path().join("x.pdf"),
    });
    assert_eq!(execute(&empty, None).expect_err("empty").code, "edits.empty");
}

/// D-006: `unsafe` is denied workspace-wide; only the forms module may opt in.
#[test]
fn unsafe_is_confined_to_the_forms_module() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut offenders = Vec::new();
    let mut stack = vec![root.join("crates"), root.join("app/src-tauri/src")];
    while let Some(dir) = stack.pop() {
        for e in std::fs::read_dir(&dir).expect("dir").flatten() {
            let p = e.path();
            if p.is_dir() {
                if !p.ends_with("target") {
                    stack.push(p);
                }
            } else if p.extension().is_some_and(|x| x == "rs") {
                let text = std::fs::read_to_string(&p).expect("read");
                let needle = ["allow(", "unsafe_code)"].concat();
                if text.contains(&needle) && !p.ends_with("pw-pdf/src/forms.rs") {
                    offenders.push(p.display().to_string());
                }
            }
        }
    }
    assert!(offenders.is_empty(), "unsafe opt-in outside forms.rs: {offenders:?}");
}
