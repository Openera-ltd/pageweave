//! Editing existing text and pictures, against embedded-font.pdf
//! (tools/fixtures/gen_embedded_font.py): a subset-embedded DejaVu Sans line, a line split into one
//! object per word, a Helvetica line and a 120 × 80 picture.
use pw_core::ops::{Operation, OperationRequest, OperationResult, PageEdit, PageEdits};
use pw_pdf::objects::{editable_objects, EditableObjects};
use pw_pdf::{execute, OpenOptions};
use std::path::{Path, PathBuf};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/pdf").join(name)
}

fn list(p: &Path) -> EditableObjects {
    editable_objects(p, &OpenOptions { password: None }, 0).expect("list")
}

fn edit(input: &Path, edits: Vec<PageEdit>, output: &Path) -> pw_core::PwResult<OperationResult> {
    let req = OperationRequest::new(Operation::EditObjects {
        input: input.into(),
        pages: vec![PageEdits { page: 0, edits }],
        output: output.into(),
    });
    req.validate()?;
    execute(&req, None)
}

fn run<'a>(l: &'a EditableObjects, starts: &str) -> &'a pw_pdf::objects::EditableText {
    l.texts
        .iter()
        .find(|t| t.text.starts_with(starts))
        .unwrap_or_else(|| panic!("no run starting {starts:?} in {:?}", l.texts))
}

fn text_edit(objects: &[u32], text: &str) -> PageEdit {
    PageEdit::Text { objects: objects.to_vec(), text: text.into(), size: None, color: None, dx: 0.0, dy: 0.0 }
}

#[test]
fn listing_groups_word_objects_into_runs_and_finds_the_picture() {
    let l = list(&fixture("embedded-font.pdf"));
    let texts: Vec<(&str, usize, bool)> =
        l.texts.iter().map(|t| (t.text.as_str(), t.objects.len(), t.embedded)).collect();
    assert_eq!(
        texts,
        vec![
            ("Invoice 2026 total due", 1, true),
            ("Payment terms: 30 days", 4, true),
            ("Standard font line", 1, false)
        ]
    );
    assert!(run(&l, "Invoice").font.contains("DejaVuSans"));
    assert!((run(&l, "Invoice").size - 14.0).abs() < 0.01);
    assert_eq!(l.images.len(), 1);
    assert_eq!((l.images[0].width_px, l.images[0].height_px), (120, 80));
    assert!(l.texts.iter().all(|t| t.locked.is_none()));
}

#[test]
fn edits_within_the_font_keep_it_and_new_letters_get_a_standard_font_with_a_warning() {
    let dir = tempfile::tempdir().expect("tmp");
    let src = fixture("embedded-font.pdf");
    let before = std::fs::read(&src).expect("read");
    let l = list(&src);
    // Letters already in the subset font: kept, no warning.
    let kept = dir.path().join("kept.pdf");
    let r = edit(&src, vec![text_edit(&run(&l, "Invoice").objects, "Invoice 2026 total sum")], &kept).expect("edit");
    assert!(r.warnings.is_empty(), "{:?}", r.warnings);
    assert_eq!(std::fs::read(&src).expect("read"), before, "source untouched");
    let k = list(&kept);
    let edited = run(&k, "Invoice 2026 total sum");
    assert!(edited.font.contains("DejaVuSans") && edited.embedded, "font kept: {edited:?}");
    // New letters (Z, b): a standard sans font, said so.
    let subst = dir.path().join("subst.pdf");
    let r = edit(&src, vec![text_edit(&run(&l, "Invoice").objects, "Zebra billing")], &subst).expect("edit");
    assert!(r.warnings.iter().any(|w| w.contains("standard font")), "{:?}", r.warnings);
    let s = list(&subst);
    let z = run(&s, "Zebra billing");
    assert!(z.font.contains("Helvetica") && !z.embedded, "substituted: {z:?}");
    // The grouped line becomes one object holding the whole new line.
    let grouped = dir.path().join("grouped.pdf");
    edit(&src, vec![text_edit(&run(&l, "Payment").objects, "Payment terms: 60 days")], &grouped).expect("edit");
    let g = list(&grouped);
    assert_eq!(run(&g, "Payment").objects.len(), 1);
    assert_eq!(run(&g, "Payment").text, "Payment terms: 60 days");
    assert!(!g.texts.iter().any(|t| t.text.contains("30")), "old words removed");
}

#[test]
fn size_colour_and_moves_apply() {
    let dir = tempfile::tempdir().expect("tmp");
    let src = fixture("embedded-font.pdf");
    let l = list(&src);
    let out = dir.path().join("styled.pdf");
    let std_run = run(&l, "Standard");
    edit(
        &src,
        vec![PageEdit::Text {
            objects: std_run.objects.clone(),
            text: "Standard font line".into(),
            size: Some(18.0),
            color: Some("#c00000".into()),
            dx: 20.0,
            dy: -10.0,
        }],
        &out,
    )
    .expect("edit");
    let o = list(&out);
    let e = run(&o, "Standard");
    assert!((e.size - 18.0).abs() < 0.05, "size {}", e.size);
    assert_eq!(e.color, "#c00000");
    assert!(
        (e.rect.left - (std_run.rect.left + 20.0)).abs() < 0.5,
        "moved right: {} → {}",
        std_run.rect.left,
        e.rect.left
    );
}

#[test]
fn pictures_turn_replace_move_and_delete() {
    let dir = tempfile::tempdir().expect("tmp");
    let src = fixture("embedded-font.pdf");
    let l = list(&src);
    let img = l.images[0].object;
    let turned = dir.path().join("turned.pdf");
    edit(&src, vec![PageEdit::RotateImage { object: img, degrees: 90 }], &turned).expect("rotate");
    let t = list(&turned);
    assert_eq!((t.images[0].width_px, t.images[0].height_px), (80, 120), "pixels turned");
    assert!((t.images[0].rect.left - l.images[0].rect.left).abs() < 0.5, "same box");
    let pic = dir.path().join("new.png");
    image::RgbImage::from_pixel(50, 40, image::Rgb([10, 160, 60])).save(&pic).expect("png");
    let replaced = dir.path().join("replaced.pdf");
    edit(&src, vec![PageEdit::ReplaceImage { object: img, path: pic }], &replaced).expect("replace");
    assert_eq!((list(&replaced).images[0].width_px, list(&replaced).images[0].height_px), (50, 40));
    let moved = dir.path().join("moved.pdf");
    edit(&src, vec![PageEdit::MoveObjects { objects: vec![img], dx: 100.0, dy: 50.0 }], &moved).expect("move");
    let m = list(&moved);
    assert!(
        (m.images[0].rect.left - (l.images[0].rect.left + 100.0)).abs() < 0.5
            && (m.images[0].rect.bottom - (l.images[0].rect.bottom + 50.0)).abs() < 0.5
    );
    let gone = dir.path().join("gone.pdf");
    edit(&src, vec![PageEdit::DeleteObjects { objects: vec![img] }], &gone).expect("delete");
    assert!(list(&gone).images.is_empty());
}

#[test]
fn added_text_and_refusals() {
    let dir = tempfile::tempdir().expect("tmp");
    let src = fixture("embedded-font.pdf");
    let l = list(&src);
    let added = dir.path().join("added.pdf");
    edit(
        &src,
        vec![PageEdit::AddText {
            x: 72.0,
            y: 400.0,
            text: "Approved PWADDED".into(),
            size: 12.0,
            color: "#000000".into(),
        }],
        &added,
    )
    .expect("add");
    assert!(list(&added).texts.iter().any(|t| t.text == "Approved PWADDED"));
    let out = dir.path().join("o.pdf");
    let code = |r: pw_core::PwResult<OperationResult>| r.expect_err("refused").code;
    assert_eq!(code(edit(&src, vec![text_edit(&run(&l, "Invoice").objects, "請求書")], &out)), "edit.charset");
    assert_eq!(code(edit(&src, vec![text_edit(&[99], "x")], &out)), "edit.stale");
    assert_eq!(code(edit(&src, vec![PageEdit::DeleteObjects { objects: vec![99] }], &out)), "edit.stale");
    assert_eq!(code(edit(&src, vec![], &out)), "edit.nothing");
    assert_eq!(
        code(edit(&src, vec![PageEdit::RotateImage { object: l.images[0].object, degrees: 45 }], &out)),
        "edit.invalid"
    );
    assert!(!out.exists());
}

#[test]
fn object_edits_save_together_with_annotations() {
    use pw_core::ops::{AnnotationKind, AnnotationSpec, RectPt};
    let dir = tempfile::tempdir().expect("tmp");
    let src = fixture("embedded-font.pdf");
    let l = list(&src);
    let out = dir.path().join("both.pdf");
    let req = OperationRequest::new(Operation::ApplyEdits {
        input: src.clone(),
        form_values: vec![],
        annotations: vec![AnnotationSpec {
            page: 0,
            kind: AnnotationKind::Highlight,
            rects: vec![RectPt { left: 72.0, bottom: 638.0, right: 166.0, top: 650.0 }],
            color: "#ffd400".into(),
            contents: None,
        }],
        object_edits: vec![PageEdits {
            page: 0,
            edits: vec![text_edit(&run(&l, "Invoice").objects, "Invoice 2026 total sum")],
        }],
        output: out.clone(),
    });
    let r = execute(&req, None).expect("apply");
    assert_eq!(r.data["object_edits"], 1);
    assert!(list(&out).texts.iter().any(|t| t.text == "Invoice 2026 total sum"));
    let info = pw_pdf::open_document(&out, &OpenOptions { password: None }).expect("open");
    assert_eq!(info.page_count, 1);
    // No staged leftovers next to the output.
    let leftovers: Vec<_> = std::fs::read_dir(dir.path())
        .expect("dir")
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with(".pw-staged"))
        .collect();
    assert!(leftovers.is_empty());
    let doc = lopdf::Document::load(&out).expect("lopdf");
    let page = doc.get_pages()[&1];
    assert!(doc.get_dictionary(page).expect("page").get(b"Annots").is_ok(), "the highlight was added too");
}
