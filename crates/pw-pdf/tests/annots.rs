//! Existing annotations: listed, deleted (with pop-ups) and re-texted in a verified copy; checked
//! independently with lopdf.

use pw_core::ops::{
    AnnotationChange, AnnotationKind, AnnotationSpec, Operation, OperationRequest, OperationResult, RectPt,
};
use pw_core::output::CollisionPolicy;
use pw_core::PwResult;
use pw_pdf::{execute, list_annotations, OpenOptions};
use std::path::{Path, PathBuf};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/pdf").join(name)
}

fn change(index: u32, subtype: &str, delete: bool, text: Option<&str>) -> AnnotationChange {
    AnnotationChange { page: 0, index, subtype: subtype.into(), delete, text: text.map(str::to_string) }
}

fn apply(
    input: PathBuf,
    changes: Vec<AnnotationChange>,
    annotations: Vec<AnnotationSpec>,
    output: &Path,
) -> PwResult<OperationResult> {
    let mut req = OperationRequest::new(Operation::ApplyEdits {
        input,
        form_values: vec![],
        annotations,
        object_edits: vec![],
        annotation_changes: changes,
        output: output.into(),
    });
    req.collision = CollisionPolicy::Fail;
    execute(&req, None)
}

/// (subtype, contents) of page 1's annotations, read with lopdf.
fn lopdf_annots(p: &Path) -> Vec<(String, String)> {
    let doc = lopdf::Document::load(p).expect("lopdf");
    let page = *doc.get_pages().get(&1).expect("page");
    let pd = doc.get_dictionary(page).expect("page dict");
    let arr = match pd.get(b"Annots").expect("Annots") {
        lopdf::Object::Reference(r) => doc.get_object(*r).expect("arr").as_array().expect("arr").clone(),
        o => o.as_array().expect("arr").clone(),
    };
    arr.iter()
        .map(|o| {
            let d = doc.dereference(o).expect("deref").1.as_dict().expect("dict");
            let st =
                String::from_utf8_lossy(d.get(b"Subtype").and_then(lopdf::Object::as_name).unwrap_or(b"")).into_owned();
            let c = d.get(b"Contents").ok().and_then(|c| lopdf::decode_text_string(c).ok()).unwrap_or_default();
            (st, c)
        })
        .collect()
}

#[test]
fn lists_comments_but_not_popups_fields_or_links() {
    let l = list_annotations(&fixture("hidden-info.pdf"), &OpenOptions::default()).expect("list");
    let got: Vec<(u32, &str)> = l.annotations.iter().map(|a| (a.index, a.subtype.as_str())).collect();
    assert_eq!(got, [(0, "Text"), (2, "Highlight"), (6, "FileAttachment"), (7, "Square")]);
    assert_eq!(l.annotations[0].text.as_deref(), Some("Reviewer note PWCOMMENT"));
    assert!(l.annotations.iter().all(|a| a.text_editable));
    assert!(l.annotations[0].rect.right > l.annotations[0].rect.left);
}

#[test]
fn delete_with_popup_and_retext_in_a_verified_copy_with_a_new_highlight() {
    let dir = tempfile::tempdir().expect("tmp");
    let src = fixture("hidden-info.pdf");
    let before = std::fs::read(&src).expect("read");
    let out = dir.path().join("edited.pdf");
    let r = apply(
        src.clone(),
        vec![change(0, "Text", true, None), change(2, "Highlight", false, Some("Checked by QA"))],
        vec![AnnotationSpec {
            page: 0,
            kind: AnnotationKind::Underline,
            rects: vec![RectPt { left: 72.0, bottom: 700.0, right: 200.0, top: 712.0 }],
            color: "#2f6fde".into(),
            contents: None,
        }],
        &out,
    )
    .expect("apply");
    assert_eq!(r.data["annotation_changes"], 2);
    assert!(r.warnings.iter().any(|w| w.contains("2 annotation(s) were removed")), "{:?}", r.warnings);
    assert_eq!(std::fs::read(&src).expect("read"), before, "source unchanged");
    let got = lopdf_annots(&out);
    let subtypes: Vec<&str> = got.iter().map(|(s, _)| s.as_str()).collect();
    assert_eq!(subtypes, ["Highlight", "Link", "Link", "Link", "FileAttachment", "Square", "Widget", "Underline"]);
    assert_eq!(got[0].1, "Checked by QA");
    let raw = String::from_utf8_lossy(&std::fs::read(&out).expect("read")).into_owned();
    assert!(!raw.contains("PWCOMM"), "the deleted note's text is gone from the file");
    // No staged leftovers.
    assert!(std::fs::read_dir(dir.path())
        .expect("ls")
        .flatten()
        .all(|e| !e.file_name().to_string_lossy().starts_with(".pw-staged")));
}

#[test]
fn stale_or_impossible_changes_are_refused_and_write_nothing() {
    let dir = tempfile::tempdir().expect("tmp");
    let out = dir.path().join("x.pdf");
    let code =
        |c: Vec<AnnotationChange>| apply(fixture("hidden-info.pdf"), c, vec![], &out).expect_err("must fail").code;
    assert_eq!(code(vec![change(0, "Highlight", true, None)]), "annotation.changed", "subtype mismatch");
    assert_eq!(code(vec![change(99, "Text", true, None)]), "annotation.changed", "index out of range");
    assert_eq!(code(vec![change(0, "Text", false, None)]), "annotation.no_change");
    assert_eq!(code(vec![change(0, "Text", true, None), change(0, "Text", true, None)]), "annotation.duplicate");
    let mut req = OperationRequest::new(Operation::ApplyEdits {
        input: fixture("encrypted-rc4-user-password.pdf"),
        form_values: vec![],
        annotations: vec![],
        object_edits: vec![],
        annotation_changes: vec![change(0, "Text", true, None)],
        output: out.clone(),
    });
    req.password = Some("pw-user-test".into());
    assert_eq!(execute(&req, None).expect_err("protected").code, "annotation.protected");
    assert!(!out.exists());
}
