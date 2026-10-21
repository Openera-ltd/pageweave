//! Form data export (XFDF/JSON) and import against the real engine. Imported files are checked
//! independently with lopdf (fully qualified names built from the field tree, `/V` values).

use pw_core::formdata::{parse_form_data, DataValue, FormDataFormat};
use pw_core::ops::{FieldValue, FormValue, Operation, OperationRequest, OperationResult};
use pw_core::output::CollisionPolicy;
use pw_core::PwResult;
use pw_pdf::{execute, form_fields, FieldKind};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/pdf").join(name)
}

fn run(op: Operation) -> PwResult<OperationResult> {
    let mut req = OperationRequest::new(op);
    req.collision = CollisionPolicy::Fail;
    execute(&req, None)
}

fn hash(p: &Path) -> Vec<u8> {
    Sha256::digest(std::fs::read(p).expect("read")).to_vec()
}

/// name → /V as text, for every terminal field, walking /Kids and joining /T with dots.
fn lopdf_values(path: &Path) -> HashMap<String, String> {
    use lopdf::Object;
    let doc = lopdf::Document::load(path).expect("lopdf must parse the output");
    let catalog = doc.catalog().expect("catalog");
    let acro = doc.dereference(catalog.get(b"AcroForm").expect("AcroForm")).expect("deref").1.as_dict().expect("dict");
    let roots = acro.get(b"Fields").and_then(Object::as_array).expect("Fields").clone();
    let text = |o: &Object| match o {
        Object::String(s, _) if s.starts_with(&[0xFE, 0xFF]) => {
            let u: Vec<u16> = s[2..].chunks(2).map(|c| u16::from_be_bytes([c[0], *c.get(1).unwrap_or(&0)])).collect();
            String::from_utf16_lossy(&u)
        }
        Object::String(s, _) => s.iter().map(|&b| b as char).collect(),
        Object::Name(n) => String::from_utf8_lossy(n).into_owned(),
        _ => String::new(),
    };
    let mut out = HashMap::new();
    let mut stack: Vec<(lopdf::ObjectId, String)> =
        roots.iter().filter_map(|r| r.as_reference().ok()).map(|id| (id, String::new())).collect();
    while let Some((id, prefix)) = stack.pop() {
        let d = doc.get_dictionary(id).expect("field");
        let part = d.get(b"T").map(text).ok();
        let name = match (&part, prefix.is_empty()) {
            (Some(p), true) => p.clone(),
            (Some(p), false) => format!("{prefix}.{p}"),
            (None, _) => prefix.clone(),
        };
        let kids: Vec<lopdf::ObjectId> = d
            .get(b"Kids")
            .and_then(Object::as_array)
            .map(|k| k.iter().filter_map(|o| o.as_reference().ok()).collect())
            .unwrap_or_default();
        // Kids with their own /T are fields; otherwise they are this field's widgets.
        let field_kids: Vec<_> =
            kids.iter().filter(|k| doc.get_dictionary(**k).map(|kd| kd.has(b"T")).unwrap_or(false)).collect();
        if field_kids.is_empty() {
            out.insert(name, d.get(b"V").map(text).unwrap_or_default());
        } else {
            stack.extend(field_kids.into_iter().map(|k| (*k, name.clone())));
        }
    }
    out
}

#[test]
fn nested_fixture_fields_have_fully_qualified_names() {
    let info = form_fields(&fixture("form-nested.pdf"), None).expect("fields");
    let got: Vec<(&str, FieldKind, bool)> =
        info.fields.iter().map(|f| (f.name.as_str(), f.kind, f.read_only)).collect();
    assert_eq!(
        got,
        [
            ("applicant.name", FieldKind::Text, false),
            ("applicant.address.city", FieldKind::Text, false),
            ("ref_no", FieldKind::Text, true),
            ("colour", FieldKind::List, false),
            ("agree", FieldKind::Checkbox, false),
            ("submit", FieldKind::PushButton, false),
        ]
    );
}

#[test]
fn export_writes_nested_xfdf_with_unsaved_values_and_leaves_the_source_alone() {
    let dir = tempfile::tempdir().expect("tmp");
    let src = fixture("form-nested.pdf");
    let before = hash(&src);
    let out = dir.path().join("data.xfdf");
    let r = run(Operation::ExportFormData {
        input: src.clone(),
        format: FormDataFormat::Xfdf,
        values: vec![
            FieldValue { name: "applicant.address.city".into(), value: FormValue::Text("Zürich & <Basel>".into()) },
            FieldValue { name: "agree".into(), value: FormValue::Checked(true) },
        ],
        output: out.clone(),
    })
    .expect("export");
    assert_eq!(r.data["fields"], 5, "push button left out");
    assert_eq!(hash(&src), before, "source untouched");
    let text = std::fs::read_to_string(&out).expect("read");
    assert_eq!(text.matches("<field name=\"applicant\">").count(), 1, "{text}");
    assert!(text.contains("<f href=\"form-nested.pdf\"/>"), "{text}");
    assert!(!text.contains("submit"), "{text}");
    let parsed = parse_form_data(text.as_bytes()).expect("parse");
    let v: HashMap<&str, &DataValue> = parsed.entries.iter().map(|e| (e.name.as_str(), &e.value)).collect();
    assert_eq!(v["applicant.address.city"], &DataValue::Text("Zürich & <Basel>".into()));
    assert_eq!(v["agree"], &DataValue::Text("Yes".into()), "checked state written as its export value");
    assert_eq!(v["ref_no"], &DataValue::Text("R-1001".into()), "read-only data is exported");
    assert_eq!(v["applicant.name"], &DataValue::Text(String::new()));

    // Same request again: the existing file is never replaced silently.
    let again =
        run(Operation::ExportFormData { input: src, format: FormDataFormat::Xfdf, values: vec![], output: out });
    assert_eq!(again.expect_err("conflict").category, pw_core::ErrorCategory::OutputConflict);
}

#[test]
fn import_xfdf_fills_a_verified_copy_and_reports_what_it_skipped() {
    let dir = tempfile::tempdir().expect("tmp");
    let data = dir.path().join("in.xfdf");
    std::fs::write(
        &data,
        "<?xml version=\"1.0\"?><xfdf xmlns=\"http://ns.adobe.com/xfdf/\"><fields>\
         <field name=\"applicant\"><field name=\"name\"><value>Ada Lovelace</value></field>\
         <field name=\"address\"><field name=\"city\"><value>London</value></field></field></field>\
         <field name=\"ref_no\"><value>HACKED</value></field>\
         <field name=\"colour\"><value>Green</value></field>\
         <field name=\"agree\"><value>Yes</value></field>\
         <field name=\"not_here\"><value>x</value></field>\
         </fields></xfdf>",
    )
    .expect("write");
    let src = fixture("form-nested.pdf");
    let before = hash(&src);

    // Preview first (read-only).
    let preview = run(Operation::ReadFormData { input: src.clone(), data: data.clone() }).expect("read");
    let matched: Vec<&str> =
        preview.data["matched"].as_array().expect("matched").iter().map(|m| m["name"].as_str().unwrap_or("")).collect();
    assert_eq!(matched, ["applicant.name", "applicant.address.city", "colour", "agree"]);
    assert_eq!(preview.data["skipped"][0]["name"], "ref_no");
    assert_eq!(preview.data["unknown"][0], "not_here");
    assert!(preview.outputs.is_empty());

    let out = dir.path().join("filled.pdf");
    let r = run(Operation::ImportFormData { input: src.clone(), data, output: out.clone() }).expect("import");
    assert_eq!(r.data["filled"], 4);
    assert!(r.warnings.iter().any(|w| w.contains("1 name(s) in the file are not fields")), "{:?}", r.warnings);
    assert!(
        r.warnings.iter().any(|w| w.contains("“ref_no” was not filled") && w.contains("read-only")),
        "{:?}",
        r.warnings
    );
    assert_eq!(hash(&src), before, "source untouched");

    let v = lopdf_values(&out);
    assert_eq!(v["applicant.name"], "Ada Lovelace");
    assert_eq!(v["applicant.address.city"], "London");
    assert_eq!(v["colour"], "Green");
    assert_eq!(v["agree"], "Yes");
    assert_eq!(v["ref_no"], "R-1001", "read-only field kept");
}

#[test]
fn json_export_of_a_filled_form_imports_back_to_the_same_values() {
    let dir = tempfile::tempdir().expect("tmp");
    let filled = dir.path().join("filled.pdf");
    let t = |n: &str, v: &str| FieldValue { name: n.into(), value: FormValue::Text(v.into()) };
    run(Operation::FillForm {
        input: fixture("form-fields.pdf"),
        values: vec![
            t("full_name", "Grace Hopper"),
            FieldValue { name: "subscribe".into(), value: FormValue::Checked(true) },
            t("plan", "pro"),
            t("country", "Japan"),
            t("comments", "Line one\nLine two — ünïcödé"),
        ],
        output: filled.clone(),
    })
    .expect("fill");
    let data = dir.path().join("data.json");
    run(Operation::ExportFormData {
        input: filled.clone(),
        format: FormDataFormat::Json,
        values: vec![],
        output: data.clone(),
    })
    .expect("export");
    let json: serde_json::Value = serde_json::from_slice(&std::fs::read(&data).expect("read")).expect("json");
    assert_eq!(json["format"], "pageweave-form-data");
    assert_eq!(json["fields"][1]["value"], true, "checkbox as a boolean");

    let out = dir.path().join("again.pdf");
    let r =
        run(Operation::ImportFormData { input: fixture("form-fields.pdf"), data: data.clone(), output: out.clone() })
            .expect("import");
    assert!(r.warnings.is_empty(), "{:?}", r.warnings);
    let a = form_fields(&filled, None).expect("a");
    let b = form_fields(&out, None).expect("b");
    let values = |i: &pw_pdf::FormInfo| i.fields.iter().map(pw_pdf::current_value).collect::<Vec<_>>();
    assert_eq!(values(&a), values(&b));

    // Importing into the already-filled file changes nothing, so no file is written.
    let none = dir.path().join("none.pdf");
    let e = run(Operation::ImportFormData { input: filled, data, output: none.clone() }).expect_err("nothing");
    assert_eq!(e.code, "form_data.nothing");
    assert!(!none.exists());
}

#[test]
fn import_refusals_are_specific_and_write_nothing() {
    let dir = tempfile::tempdir().expect("tmp");
    let data = dir.path().join("d.json");
    std::fs::write(&data, br#"{"not_a_field": "x"}"#).expect("write");
    let out = dir.path().join("o.pdf");
    let code = |op| run(op).expect_err("must fail").code;
    assert_eq!(
        code(Operation::ImportFormData { input: fixture("form-fields.pdf"), data: data.clone(), output: out.clone() }),
        "form_data.nothing"
    );
    assert_eq!(
        code(Operation::ImportFormData { input: fixture("five-pages.pdf"), data: data.clone(), output: out.clone() }),
        "form_data.no_form"
    );
    assert_eq!(
        code(Operation::ReadFormData { input: fixture("form-fields.pdf"), data: dir.path().join("missing.xfdf") }),
        "io.not_found"
    );
    let bomb = dir.path().join("bomb.xfdf");
    std::fs::write(&bomb, "<!DOCTYPE x [<!ENTITY a \"a\">]><xfdf/>").expect("write");
    assert_eq!(code(Operation::ReadFormData { input: fixture("form-fields.pdf"), data: bomb }), "form_data.xfdf");
    assert_eq!(
        code(Operation::ExportFormData {
            input: fixture("five-pages.pdf"),
            format: FormDataFormat::Xfdf,
            values: vec![],
            output: dir.path().join("x.xfdf"),
        }),
        "form_data.no_form"
    );
    // The data file is an input: the filled copy may never overwrite it.
    std::fs::write(&data, br#"{"full_name": "x"}"#).expect("write");
    let mut req = OperationRequest::new(Operation::ImportFormData {
        input: fixture("form-fields.pdf"),
        data: data.clone(),
        output: data.clone(),
    });
    req.collision = CollisionPolicy::Replace;
    assert!(execute(&req, None).is_err(), "data file must not be replaced");
    assert_eq!(std::fs::read(&data).expect("read"), br#"{"full_name": "x"}"#);
    assert!(!out.exists());
}
