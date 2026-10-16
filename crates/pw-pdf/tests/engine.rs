//! Real-engine integration tests (PRD 04 "Architecture acceptance": real engines, not mocks).
//! Outputs are re-parsed with `lopdf`, an independent PDF parser, so the engine does not grade itself.

use pw_core::ops::{Operation, OperationRequest, SplitMode};
use pw_core::output::CollisionPolicy;
use pw_core::ErrorCategory;
use pw_pdf::{execute, open_document, render_page_png, search, OpenOptions};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/pdf").join(name)
}

fn sha(path: &Path) -> Vec<u8> {
    Sha256::digest(std::fs::read(path).expect("read")).to_vec()
}

/// Independent check: page count, page text markers (via lopdf) and /Rotate values.
fn independent(path: &Path) -> (usize, Vec<String>, Vec<i64>) {
    let doc = lopdf::Document::load(path).expect("lopdf must parse the output");
    let pages = doc.get_pages();
    let mut texts = Vec::new();
    let mut rots = Vec::new();
    for (num, id) in &pages {
        texts.push(doc.extract_text(&[*num]).unwrap_or_default());
        let rot = doc
            .get_object(*id)
            .and_then(|o| o.as_dict().cloned())
            .ok()
            .and_then(|d| d.get(b"Rotate").ok().and_then(|r| r.as_i64().ok()))
            .unwrap_or(0);
        rots.push(rot);
    }
    (pages.len(), texts, rots)
}

fn run(op: Operation) -> pw_core::PwResult<pw_core::ops::OperationResult> {
    execute(&OperationRequest::new(op), None)
}

fn opts() -> OpenOptions<'static> {
    OpenOptions::default()
}

#[test]
fn inspect_reports_structure() {
    let info = open_document(&fixture("five-pages.pdf"), &opts()).expect("open");
    assert_eq!(info.page_count, 5);
    assert_eq!(info.title.as_deref(), Some("Five pages"));
    assert_eq!(info.pdf_version, "1.7");
    assert!(!info.encrypted);
    assert!(!info.has_xfa);
    assert_eq!(info.bookmarks.len(), 5);
    assert_eq!(info.bookmarks[2].title, "Chapter 3");
    assert_eq!(info.bookmarks[2].page, Some(2));
    assert_eq!(info.sha256.len(), 64);

    let mixed = open_document(&fixture("mixed-sizes-rotated.pdf"), &opts()).expect("open");
    assert_eq!((mixed.pages[0].width_pt, mixed.pages[0].height_pt), (612.0, 792.0));
    assert_eq!((mixed.pages[1].width_pt, mixed.pages[1].height_pt), (595.0, 842.0));
    assert_eq!(mixed.pages[2].rotation, 90);
    // Regression: sizes are reported unrotated (MediaBox 612×792), not PDFium's rotated 792×612.
    assert_eq!((mixed.pages[2].width_pt, mixed.pages[2].height_pt), (612.0, 792.0));
}

#[test]
fn bad_inputs_map_to_specific_errors() {
    let cases = [
        ("empty.pdf", ErrorCategory::Corrupt, "pdf.empty"),
        ("not-a-pdf.pdf", ErrorCategory::UnsupportedFormat, "pdf.not_pdf"),
        ("corrupt-truncated.pdf", ErrorCategory::Corrupt, ""),
        ("does-not-exist.pdf", ErrorCategory::NotFound, "io.not_found"),
    ];
    for (name, cat, code) in cases {
        match open_document(&fixture(name), &opts()) {
            Err(e) => {
                assert_eq!(e.category, cat, "{name}: {e:?}");
                if !code.is_empty() {
                    assert_eq!(e.code, code, "{name}");
                }
            }
            // PDFium can sometimes recover truncated files by rebuilding the xref; that is acceptable
            // only if what it recovered is internally consistent.
            Ok(info) if name == "corrupt-truncated.pdf" => assert!(info.page_count <= 5),
            Ok(_) => panic!("{name} should fail"),
        }
    }
}

#[test]
fn javascript_open_action_is_not_executed_and_document_opens() {
    // The pinned engine is built without V8; opening must succeed without running anything.
    let info = open_document(&fixture("open-action-javascript.pdf"), &opts()).expect("open");
    assert_eq!(info.page_count, 1);
}

#[test]
fn search_case_and_whole_word() {
    let p = fixture("search-terms.pdf");
    let never = || false;
    let (hits, truncated) = search(&p, &opts(), "alpha", false, false, &never).expect("search");
    assert!(!truncated);
    // "Alpha", "alpha", "ALPHA", "alpha"(bet) on page 1 and "alpha" on page 3.
    assert_eq!(hits.len(), 5, "{hits:?}");
    assert_eq!(hits.iter().filter(|h| h.page == 2).count(), 1);
    assert!(hits.iter().all(|h| h.page != 1));
    assert!(hits.iter().all(|h| !h.rects.is_empty() && h.rects[0].right > h.rects[0].left));

    let (case, _) = search(&p, &opts(), "alpha", true, false, &never).expect("search");
    assert_eq!(case.len(), 3, "{case:?}");
    let (word, _) = search(&p, &opts(), "alpha", false, true, &never).expect("search");
    assert_eq!(word.len(), 4, "whole word excludes 'alphabet': {word:?}");
    let (none, _) = search(&p, &opts(), "zzzz", false, false, &never).expect("search");
    assert!(none.is_empty());
    let cancelled = || true;
    assert_eq!(
        search(&p, &opts(), "alpha", false, false, &cancelled).expect_err("cancel").category,
        ErrorCategory::Cancelled
    );
}

#[test]
fn render_produces_png_of_expected_size() {
    let (png, w, h) = render_page_png(&fixture("single-page.pdf"), &opts(), 0, 1.0).expect("render");
    assert!(png.starts_with(b"\x89PNG"));
    assert_eq!((w, h), (612, 792));
    let img = image::load_from_memory(&png).expect("decode");
    assert_eq!((img.width(), img.height()), (612, 792));
    // Page is not blank: some pixels are dark (text was drawn).
    let dark = img.to_luma8().pixels().filter(|p| p.0[0] < 128).count();
    assert!(dark > 100, "expected rendered text, found {dark} dark pixels");

    let (_, rw, rh) = render_page_png(&fixture("mixed-sizes-rotated.pdf"), &opts(), 2, 0.5).expect("render rotated");
    assert_eq!((rw, rh), (396, 306), "rotated page renders landscape");
    assert_eq!(
        render_page_png(&fixture("single-page.pdf"), &opts(), 1, 1.0).expect_err("oob").code,
        "render.page_out_of_bounds"
    );
    assert_eq!(
        render_page_png(&fixture("single-page.pdf"), &opts(), 0, 500.0).expect_err("huge").code,
        "render.too_large"
    );
}

#[test]
fn extract_pages_in_user_order_preserves_source() {
    let dir = tempfile::tempdir().expect("tmp");
    let src = fixture("five-pages.pdf");
    let before = sha(&src);
    let out = dir.path().join("extract.pdf");
    let r = run(Operation::ExtractPages { input: src.clone(), pages: "4, 1-2".into(), output: out.clone() })
        .expect("extract");
    assert_eq!(r.outputs[0].pages, Some(3));
    let (n, texts, _) = independent(&out);
    assert_eq!(n, 3);
    assert!(texts[0].contains("PWMARK004"), "{texts:?}");
    assert!(texts[1].contains("PWMARK001"));
    assert!(texts[2].contains("PWMARK002"));
    assert_eq!(sha(&src), before, "source must be unchanged");
}

#[test]
fn delete_pages_and_refuse_deleting_all() {
    let dir = tempfile::tempdir().expect("tmp");
    let src = fixture("five-pages.pdf");
    let out = dir.path().join("deleted.pdf");
    run(Operation::DeletePages { input: src.clone(), pages: "2,4".into(), output: out.clone() }).expect("delete");
    let (n, texts, _) = independent(&out);
    assert_eq!(n, 3);
    assert!(texts[0].contains("PWMARK001") && texts[1].contains("PWMARK003") && texts[2].contains("PWMARK005"));
    let e = run(Operation::DeletePages { input: src, pages: "1-5".into(), output: dir.path().join("x.pdf") })
        .expect_err("all pages");
    assert_eq!(e.code, "delete.all_pages");
    assert!(!dir.path().join("x.pdf").exists());
}

#[test]
fn rotate_is_relative_and_verified_independently() {
    let dir = tempfile::tempdir().expect("tmp");
    let src = fixture("mixed-sizes-rotated.pdf");
    let out = dir.path().join("rot.pdf");
    run(Operation::RotatePages { input: src.clone(), pages: Some("1,3".into()), degrees: 90, output: out.clone() })
        .expect("rotate");
    let (_, _, rots) = independent(&out);
    assert_eq!(rots, vec![90, 0, 180], "page 3 was already at 90");
    let out2 = dir.path().join("rot2.pdf");
    run(Operation::RotatePages { input: out.clone(), pages: None, degrees: -90, output: out2.clone() })
        .expect("rotate back");
    assert_eq!(independent(&out2).2, vec![0, 270, 90]);
}

#[test]
fn merge_in_order_with_progress() {
    let dir = tempfile::tempdir().expect("tmp");
    let out = dir.path().join("merged.pdf");
    let r = run(Operation::Merge {
        inputs: vec![fixture("single-page.pdf"), fixture("five-pages.pdf"), fixture("mixed-sizes-rotated.pdf")],
        output: out.clone(),
    })
    .expect("merge");
    assert_eq!(r.outputs[0].pages, Some(9));
    assert!(r.warnings.iter().any(|w| w.contains("Bookmarks")), "bookmark loss must be disclosed");
    let (n, texts, rots) = independent(&out);
    assert_eq!(n, 9);
    assert!(texts[1].contains("PWMARK001") && texts[5].contains("PWMARK005"));
    assert_eq!(rots[8], 90, "intrinsic rotation survives the merge");
}

#[test]
fn merge_with_bad_input_names_the_file_position_and_writes_nothing() {
    let dir = tempfile::tempdir().expect("tmp");
    let out = dir.path().join("merged.pdf");
    let e = run(Operation::Merge {
        inputs: vec![fixture("single-page.pdf"), fixture("not-a-pdf.pdf")],
        output: out.clone(),
    })
    .expect_err("bad input");
    assert!(e.message.starts_with("File 2 of 2"), "{}", e.message);
    assert!(!out.exists());
}

#[test]
fn split_every_n_and_by_ranges() {
    let dir = tempfile::tempdir().expect("tmp");
    let r = run(Operation::Split {
        input: fixture("five-pages.pdf"),
        split: SplitMode::EveryN { n: 2 },
        output_dir: dir.path().to_path_buf(),
    })
    .expect("split");
    let names: Vec<String> =
        r.outputs.iter().map(|o| o.path.file_name().expect("name").to_string_lossy().into_owned()).collect();
    assert_eq!(names, vec!["five-pages_part01.pdf", "five-pages_part02.pdf", "five-pages_part03.pdf"]);
    assert_eq!(independent(&r.outputs[2].path).0, 1);

    let dir2 = tempfile::tempdir().expect("tmp");
    let r = run(Operation::Split {
        input: fixture("five-pages.pdf"),
        split: SplitMode::Ranges { ranges: vec!["1-2".into(), "5".into()] },
        output_dir: dir2.path().to_path_buf(),
    })
    .expect("split ranges");
    assert_eq!(r.outputs.len(), 2);
    assert!(independent(&r.outputs[1].path).1[0].contains("PWMARK005"));
}

#[test]
fn split_is_all_or_nothing_on_conflict() {
    let dir = tempfile::tempdir().expect("tmp");
    std::fs::write(dir.path().join("five-pages_part02.pdf"), b"existing").expect("seed");
    let e = run(Operation::Split {
        input: fixture("five-pages.pdf"),
        split: SplitMode::EveryN { n: 2 },
        output_dir: dir.path().to_path_buf(),
    })
    .expect_err("conflict on part 2");
    assert_eq!(e.category, ErrorCategory::OutputConflict);
    assert!(!dir.path().join("five-pages_part01.pdf").exists(), "part 1 must be rolled back");
    assert_eq!(std::fs::read(dir.path().join("five-pages_part02.pdf")).expect("read"), b"existing");
}

#[test]
fn output_onto_source_is_refused_even_with_replace() {
    let dir = tempfile::tempdir().expect("tmp");
    let src = dir.path().join("doc.pdf");
    std::fs::copy(fixture("five-pages.pdf"), &src).expect("copy");
    let before = sha(&src);
    let mut req = OperationRequest::new(Operation::RotatePages {
        input: src.clone(),
        pages: None,
        degrees: 90,
        output: src.clone(),
    });
    req.collision = CollisionPolicy::Replace;
    let e = execute(&req, None).expect_err("must refuse");
    assert_eq!(e.code, "output.would_overwrite_source");
    assert_eq!(sha(&src), before);
}

#[test]
fn range_errors_surface_before_any_write() {
    let dir = tempfile::tempdir().expect("tmp");
    let out = dir.path().join("o.pdf");
    let e = run(Operation::ExtractPages { input: fixture("five-pages.pdf"), pages: "3-9".into(), output: out.clone() })
        .expect_err("oob");
    assert_eq!(e.code, "range.out_of_bounds");
    assert!(e.message.contains("5 pages"));
    assert!(!out.exists());
}

#[test]
fn extract_text_by_page() {
    let r = run(Operation::ExtractText { input: fixture("five-pages.pdf"), pages: Some("2".into()) }).expect("text");
    let pages = r.data["pages"].as_array().expect("pages");
    assert_eq!(pages.len(), 1);
    assert_eq!(pages[0]["page"], 2);
    assert!(pages[0]["text"].as_str().expect("text").contains("PWMARK002"));
}

#[test]
fn render_operation_writes_validated_png() {
    let dir = tempfile::tempdir().expect("tmp");
    let out = dir.path().join("p1.png");
    let r = run(Operation::Render { input: fixture("single-page.pdf"), page: 1, scale: 0.5, output: out.clone() })
        .expect("render");
    assert_eq!(r.data["width"], 306);
    assert!(std::fs::read(&out).expect("read").starts_with(b"\x89PNG"));
}

#[test]
fn concurrent_operations_from_job_workers() {
    use pw_core::job::{JobManager, JobState};
    use std::time::Duration;
    let jm = JobManager::new(4);
    let dir = tempfile::tempdir().expect("tmp");
    let ids: Vec<_> = (0..8)
        .map(|i| {
            let out = dir.path().join(format!("r{i}.pdf"));
            let req = OperationRequest::new(Operation::RotatePages {
                input: fixture("twenty-pages.pdf"),
                pages: None,
                degrees: 90,
                output: out,
            });
            jm.submit("rotate", move |ctx| {
                execute(&req, Some(ctx)).map(|r| serde_json::to_value(r).unwrap_or_default())
            })
        })
        .collect();
    for id in ids {
        let s = jm.wait(id, Duration::from_secs(60)).expect("job");
        assert_eq!(s.state, JobState::Succeeded, "{:?}", s.error);
    }
    for i in 0..8 {
        assert_eq!(independent(&dir.path().join(format!("r{i}.pdf"))).0, 20);
    }
}

const USER_PW: &str = "pw-user-test";

#[test]
fn encrypted_document_requires_password() {
    let p = fixture("encrypted-rc4-user-password.pdf");
    let e = open_document(&p, &opts()).expect_err("needs password");
    assert_eq!(e.category, ErrorCategory::PasswordRequired);
    let e = open_document(&p, &OpenOptions { password: Some("wrong") }).expect_err("wrong password");
    assert_eq!(e.category, ErrorCategory::PasswordRequired);
    let info = open_document(&p, &OpenOptions { password: Some(USER_PW) }).expect("open with password");
    assert_eq!(info.page_count, 5);
    assert!(info.encrypted);
}

/// Probe: does an output derived from an encrypted file stay encrypted? Records actual behaviour.
#[test]
fn protection_status_of_outputs_from_encrypted_input() {
    let dir = tempfile::tempdir().expect("tmp");
    let src = fixture("encrypted-rc4-user-password.pdf");
    for (name, op) in [
        (
            "rotate",
            Operation::RotatePages { input: src.clone(), pages: None, degrees: 90, output: dir.path().join("rot.pdf") },
        ),
        (
            "extract",
            Operation::ExtractPages { input: src.clone(), pages: "1-2".into(), output: dir.path().join("ext.pdf") },
        ),
    ] {
        let mut req = OperationRequest::new(op);
        req.password = Some(USER_PW.into());
        let r = execute(&req, None).unwrap_or_else(|e| panic!("{name}: {e:?}"));
        let out = &r.outputs[0].path;
        let reopened_without_pw = open_document(out, &opts());
        eprintln!("{name}: output opens without password = {}", reopened_without_pw.is_ok());
        if reopened_without_pw.is_ok() {
            assert!(
                r.warnings.iter().any(|w| w.contains("not password protected")),
                "{name}: protection removal must be disclosed, warnings={:?}",
                r.warnings
            );
        }
    }
}

#[test]
fn page_text_segments_have_text_and_geometry() {
    let t = pw_pdf::page_text_segments(&fixture("five-pages.pdf"), &opts(), 1).expect("segments");
    let all: String = t.segments.iter().map(|s| s.text.as_str()).collect::<Vec<_>>().join("\n");
    assert!(all.contains("PWMARK002"), "{all}");
    assert!(!t.truncated);
    for s in &t.segments {
        assert!(s.right > s.left && s.top > s.bottom, "{s:?}");
        assert!(s.left >= 0.0 && s.right <= 612.0 && s.bottom >= 0.0 && s.top <= 792.0, "inside the page: {s:?}");
    }
    // Lines appear top to bottom in this simple single-column fixture.
    let tops: Vec<f32> = t.segments.iter().map(|s| s.top).collect();
    assert!(tops.windows(2).all(|w| w[0] >= w[1]), "{tops:?}");
    assert_eq!(
        pw_pdf::page_text_segments(&fixture("five-pages.pdf"), &opts(), 9).expect_err("oob").code,
        "text.page_out_of_bounds"
    );
}

fn marker_rect(path: &Path, page: u32, needle: &str) -> pw_core::ops::RectPt {
    let t = pw_pdf::page_text_segments(path, &opts(), page).expect("segments");
    let s = t.segments.iter().find(|s| s.text.contains(needle)).expect("run with marker");
    pw_core::ops::RectPt { left: s.left, bottom: s.bottom, right: s.right, top: s.top }
}

#[test]
fn annotate_adds_verified_markup_and_note_without_touching_source() {
    use pw_core::ops::{AnnotationKind, AnnotationSpec};
    let dir = tempfile::tempdir().expect("tmp");
    let src = fixture("five-pages.pdf");
    let before = sha(&src);
    let rect = marker_rect(&src, 1, "PWMARK002");
    let out = dir.path().join("annotated.pdf");
    let spec = |kind, color: &str, contents: Option<&str>| AnnotationSpec {
        page: 1,
        kind,
        rects: vec![rect],
        color: color.into(),
        contents: contents.map(Into::into),
    };
    let r = run(Operation::Annotate {
        input: src.clone(),
        annotations: vec![
            spec(AnnotationKind::Highlight, "#ffd400", None),
            spec(AnnotationKind::Underline, "#2f6fde", None),
            spec(AnnotationKind::Note, "#ff8800", Some("Check this marker")),
        ],
        output: out.clone(),
    })
    .expect("annotate");
    assert_eq!(r.data["annotations"], 3);
    assert_eq!(sha(&src), before, "source unchanged");

    // Independent parser: page 2 carries exactly these annotations with geometry and colour.
    let doc = lopdf::Document::load(&out).expect("lopdf parses output");
    let pages = doc.get_pages();
    let page2 = doc.get_object(pages[&2]).and_then(|o| o.as_dict()).expect("page 2 dict");
    let annots = page2.get(b"Annots").and_then(|a| a.as_array()).expect("/Annots");
    assert_eq!(annots.len(), 3);
    let mut subtypes = Vec::new();
    for a in annots {
        let d = doc.dereference(a).expect("deref").1.as_dict().expect("annot dict").clone();
        let st = d
            .get(b"Subtype")
            .and_then(|s| s.as_name())
            .map(|n| String::from_utf8_lossy(n).into_owned())
            .expect("subtype");
        if st == "Highlight" {
            let q = d.get(b"QuadPoints").and_then(|q| q.as_array()).expect("quad points");
            assert_eq!(q.len(), 8);
            let c = d.get(b"C").and_then(|c| c.as_array()).expect("colour");
            let red = c[0].as_float().expect("red");
            assert!((red - 1.0).abs() < 0.01, "highlight colour red channel {red}");
        }
        if st == "Text" {
            let contents = d
                .get(b"Contents")
                .and_then(|c| c.as_str())
                .map(|b| String::from_utf8_lossy(b).into_owned())
                .unwrap_or_default();
            assert!(contents.contains("Check this marker"), "note contents: {contents:?}");
        }
        subtypes.push(st);
    }
    subtypes.sort();
    assert_eq!(subtypes, vec!["Highlight", "Text", "Underline"]);
    for (n, id) in &pages {
        if *n != 2 {
            let d = doc.get_object(*id).and_then(|o| o.as_dict()).expect("page");
            assert!(d.get(b"Annots").is_err(), "page {n} untouched");
        }
    }

    // Rendered by the engine: highlight colour appears inside the marked rectangle.
    let (png, w, h) = render_page_png(&out, &opts(), 1, 1.0).expect("render");
    let img = image::load_from_memory(&png).expect("decode").to_rgb8();
    assert_eq!((w, h), (612, 792));
    let (x0, x1) = (rect.left as u32 + 2, rect.right as u32 - 2);
    let (y0, y1) = ((792.0 - rect.top) as u32 + 2, (792.0 - rect.bottom) as u32 - 2);
    let yellowish = (y0..y1)
        .flat_map(|y| (x0..x1).map(move |x| (x, y)))
        .filter(|&(x, y)| {
            let p = img.get_pixel(x, y);
            p[0] > 200 && p[1] > 170 && p[2] < 120
        })
        .count();
    assert!(yellowish > 50, "highlight is drawn ({yellowish} highlight-coloured pixels)");
}

#[test]
fn annotate_rejects_bad_page_and_writes_nothing() {
    use pw_core::ops::{AnnotationKind, AnnotationSpec, RectPt};
    let dir = tempfile::tempdir().expect("tmp");
    let out = dir.path().join("a.pdf");
    let e = run(Operation::Annotate {
        input: fixture("single-page.pdf"),
        annotations: vec![AnnotationSpec {
            page: 3,
            kind: AnnotationKind::Highlight,
            rects: vec![RectPt { left: 1.0, bottom: 1.0, right: 9.0, top: 9.0 }],
            color: "#ffd400".into(),
            contents: None,
        }],
        output: out.clone(),
    })
    .expect_err("page out of range");
    assert_eq!(e.code, "annotate.page");
    assert!(!out.exists());
}
