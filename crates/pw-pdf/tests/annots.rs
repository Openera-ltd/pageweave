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
    AnnotationChange {
        page: 0,
        index,
        subtype: subtype.into(),
        delete,
        text: text.map(str::to_string),
        color: None,
        offset: None,
        rect: None,
    }
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

#[test]
fn colours_of_markup_and_shapes_change_and_are_drawn_in_the_new_colour() {
    let dir = tempfile::tempdir().expect("tmp");
    let out = dir.path().join("recoloured.pdf");
    let recolour = |index, subtype: &str, color: &str| AnnotationChange {
        color: Some(color.into()),
        ..change(index, subtype, false, None)
    };
    apply(
        fixture("hidden-info.pdf"),
        vec![recolour(2, "Highlight", "#00ff00"), recolour(7, "Square", "#0000ff")],
        vec![],
        &out,
    )
    .expect("recolour");
    let l = list_annotations(&out, &OpenOptions::default()).expect("list");
    let col = |i: u32| l.annotations.iter().find(|a| a.index == i).and_then(|a| a.color.clone());
    assert_eq!((col(2).as_deref(), col(7).as_deref()), (Some("#00ff00"), Some("#0000ff")));
    // Drawn in the new colours (the stored appearance with the old one was dropped).
    let (png, _, h) = pw_pdf::render_page_png(&out, &OpenOptions::default(), 0, 1.0).expect("render");
    let img = image::load_from_memory(&png).expect("png").to_rgb8();
    let at = |x: f32, y: f32| *img.get_pixel(x as u32, h - y as u32);
    let hl = at(80.0, 745.0); // inside the highlight (72..200 × 736..754)
    assert!(hl[1] > 150 && hl[0] < 100, "highlight is green now (was yellow): {hl:?}");
    // The square (300..360 × 500..540) has a blue outline now, no red left.
    let (mut blue, mut red) = (0, 0);
    for y in (h - 542)..(h - 498) {
        for x in 298..362 {
            let p = img.get_pixel(x, y);
            blue += (p[2] > 150 && p[0] < 100) as u32;
            red += (p[0] > 150 && p[2] < 100) as u32;
        }
    }
    assert!(blue > 100 && red == 0, "square outline blue {blue} px, red {red} px");
    // Kinds that are not drawn from the colour are refused before anything is written.
    let other = dir.path().join("other.pdf");
    let e = apply(fixture("hidden-info.pdf"), vec![recolour(6, "FileAttachment", "#00ff00")], vec![], &other)
        .expect_err("refused");
    assert_eq!(e.code, "annotation.color_kind");
    let e = apply(fixture("hidden-info.pdf"), vec![recolour(2, "Highlight", "green")], vec![], &other)
        .expect_err("refused");
    assert_eq!(e.code, "annotation.color");
    assert!(!other.exists());
}

#[test]
fn annotations_move_with_their_appearance_and_bad_moves_are_refused() {
    let dir = tempfile::tempdir().expect("tmp");
    let out = dir.path().join("moved.pdf");
    let shift = |index, subtype: &str, dx: f32, dy: f32| AnnotationChange {
        offset: Some([dx, dy]),
        ..change(index, subtype, false, None)
    };
    let before = list_annotations(&fixture("hidden-info.pdf"), &OpenOptions::default()).expect("list");
    apply(
        fixture("hidden-info.pdf"),
        vec![shift(2, "Highlight", 0.0, -100.0), shift(7, "Square", 50.0, 50.0)],
        vec![],
        &out,
    )
    .expect("move");
    let after = list_annotations(&out, &OpenOptions::default()).expect("list");
    let rect = |l: &pw_pdf::AnnotationList, i: u32| {
        l.annotations.iter().find(|a| a.index == i).map(|a| a.rect).expect("annotation")
    };
    let (h0, h1) = (rect(&before, 2), rect(&after, 2));
    assert!((h1.bottom - (h0.bottom - 100.0)).abs() < 0.01 && (h1.left - h0.left).abs() < 0.01, "{h0:?} → {h1:?}");
    let (s0, s1) = (rect(&before, 7), rect(&after, 7));
    assert!((s1.left - (s0.left + 50.0)).abs() < 0.01 && (s1.top - (s0.top + 50.0)).abs() < 0.01, "{s0:?} → {s1:?}");
    // Drawn at the new place only: the highlight's yellow is 100 pt lower, none left where it was.
    let (png, _, h) = pw_pdf::render_page_png(&out, &OpenOptions::default(), 0, 1.0).expect("render");
    let img = image::load_from_memory(&png).expect("png").to_rgb8();
    let yellow = |y0: u32| {
        let mut n = 0;
        for y in (h - y0 - 16)..(h - y0 - 2) {
            for x in 76..196 {
                let p = img.get_pixel(x, y);
                n += (p[0] > 200 && p[1] > 180 && p[2] < 120) as u32;
            }
        }
        n
    };
    assert!(yellow(636) > 300 && yellow(736) == 0, "yellow at new place {}, at old place {}", yellow(636), yellow(736));
    let (mut red_new, mut red_old) = (0, 0);
    for y in 0..h {
        for x in 0..img.width() {
            let p = img.get_pixel(x, y);
            if p[0] > 150 && p[1] < 100 && p[2] < 100 {
                let py = (h - y) as f32;
                let fx = x as f32;
                red_new += ((348.0..=412.0).contains(&fx) && (548.0..=592.0).contains(&py)) as u32;
                red_old += ((298.0..348.0).contains(&fx) && (498.0..548.0).contains(&py)) as u32;
            }
        }
    }
    assert!(
        red_new > 100 && red_old == 0,
        "square drawn at its new place ({red_new} px) and not the old ({red_old} px)"
    );
    // Nothing to move, or too far: refused before anything is written.
    let other = dir.path().join("other.pdf");
    for (dx, dy) in [(0.0, 0.0), (f32::NAN, 1.0), (20_000.0, 0.0)] {
        let e = apply(fixture("hidden-info.pdf"), vec![shift(2, "Highlight", dx, dy)], vec![], &other)
            .expect_err("refused");
        assert_eq!(e.code, "annotation.offset");
    }
    assert!(!other.exists());
}

#[test]
fn shapes_resize_into_a_new_box_and_other_kinds_are_refused() {
    let dir = tempfile::tempdir().expect("tmp");
    let out = dir.path().join("resized.pdf");
    let to = RectPt { left: 300.0, bottom: 500.0, right: 420.0, top: 580.0 };
    let resize = |index, subtype: &str, rect: RectPt| AnnotationChange {
        rect: Some(rect),
        ..change(index, subtype, false, None)
    };
    apply(fixture("hidden-info.pdf"), vec![resize(7, "Square", to)], vec![], &out).expect("resize");
    let l = list_annotations(&out, &OpenOptions::default()).expect("list");
    let r = l.annotations.iter().find(|a| a.index == 7).map(|a| a.rect).expect("square");
    assert!((r.right - 420.0).abs() < 0.01 && (r.top - 580.0).abs() < 0.01, "{r:?}");
    // Redrawn at the new size: the outline is at the new right and top edges, not the old ones.
    let (png, _, h) = pw_pdf::render_page_png(&out, &OpenOptions::default(), 0, 1.0).expect("render");
    let img = image::load_from_memory(&png).expect("png").to_rgb8();
    let red = |x0: u32, x1: u32, y0: u32, y1: u32| {
        let mut n = 0;
        for y in (h - y1)..(h - y0) {
            for x in x0..x1 {
                let p = img.get_pixel(x, y);
                n += (p[0] > 150 && p[1] < 100 && p[2] < 100) as u32;
            }
        }
        n
    };
    assert!(red(415, 425, 510, 570) > 20, "new right edge drawn: {}", red(415, 425, 510, 570));
    assert!(red(310, 410, 575, 585) > 20, "new top edge drawn: {}", red(310, 410, 575, 585));
    assert_eq!(red(355, 365, 510, 530), 0, "old right edge gone");
    // Refused before anything is written: a kind that is not a shape, a box too small, a move as well.
    let other = dir.path().join("other.pdf");
    let e = apply(fixture("hidden-info.pdf"), vec![resize(2, "Highlight", to)], vec![], &other).expect_err("refused");
    assert_eq!(e.code, "annotation.resize_kind");
    let tiny = RectPt { left: 300.0, bottom: 500.0, right: 300.5, top: 580.0 };
    let e = apply(fixture("hidden-info.pdf"), vec![resize(7, "Square", tiny)], vec![], &other).expect_err("refused");
    assert_eq!(e.code, "annotation.rect");
    let both = AnnotationChange { offset: Some([5.0, 5.0]), ..resize(7, "Square", to) };
    let e = apply(fixture("hidden-info.pdf"), vec![both], vec![], &other).expect_err("refused");
    assert_eq!(e.code, "annotation.rect");
    assert!(!other.exists());
}

#[test]
fn comments_of_a_password_protected_file_are_listed_with_its_password() {
    let dir = tempfile::tempdir().expect("tmp");
    let protected = dir.path().join("p.pdf");
    let mut req = OperationRequest::new(Operation::Protect {
        input: fixture("hidden-info.pdf"),
        permissions: Default::default(),
        output: protected.clone(),
    });
    req.new_password = Some("list-pw".into());
    execute(&req, None).expect("protect");
    let plain = list_annotations(&fixture("hidden-info.pdf"), &OpenOptions::default()).expect("plain");
    let listed =
        list_annotations(&protected, &OpenOptions { password: Some("list-pw") }).expect("listed with the password");
    let texts = |l: &pw_pdf::AnnotationList| {
        l.annotations.iter().map(|a| (a.subtype.clone(), a.text.clone())).collect::<Vec<_>>()
    };
    assert_eq!(texts(&listed), texts(&plain));
    // Inspecting hidden information is read-only too.
    let found = pw_pdf::sanitize_inspect(&protected, &OpenOptions { password: Some("list-pw") }).expect("inspect");
    assert_eq!(
        found.comments,
        pw_pdf::sanitize_inspect(&fixture("hidden-info.pdf"), &OpenOptions::default()).expect("plain").comments
    );
    // Without the password the file does not open at all.
    assert_eq!(list_annotations(&protected, &OpenOptions::default()).expect_err("password").code, "pdf.password");
}
