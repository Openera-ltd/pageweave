//! Reduce file size against the real engine; results checked independently (lopdf structure,
//! PDFium text and form fields) on top of the engine's own render comparison.
use pw_core::ops::{Operation, OperationRequest, OperationResult};
use pw_pdf::{execute, form_fields};
use std::path::{Path, PathBuf};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/pdf").join(name)
}

fn run(op: Operation) -> pw_core::PwResult<OperationResult> {
    execute(&OperationRequest::new(op), None)
}

fn optimize(input: &Path, output: &Path) -> pw_core::PwResult<OperationResult> {
    run(Operation::Optimize { input: input.into(), images: None, output: output.into() })
}

fn texts(path: &Path) -> Vec<String> {
    let r = run(Operation::ExtractText { input: path.into(), pages: None }).expect("text");
    r.data["pages"]
        .as_array()
        .expect("pages")
        .iter()
        .map(|p| p["text"].as_str().unwrap_or_default().to_string())
        .collect()
}

/// A copy of `src` with every stream stored uncompressed and no object streams (a bloated file).
fn bloated(src: &Path, out: &Path) {
    let mut doc = lopdf::Document::load(src).expect("load");
    doc.decompress();
    doc.save(out).expect("save");
}

/// A three-page text-heavy PDF with its content streams stored uncompressed.
fn text_heavy(out: &Path) {
    use lopdf::{dictionary, Document, Object, Stream};
    let mut doc = Document::with_version("1.7");
    let font = doc.add_object(dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica" });
    let pages_id = doc.new_object_id();
    let mut kids = Vec::new();
    for p in 0..3 {
        let mut ops = String::from("BT /F1 9 Tf 11 TL 40 760 Td\n");
        for l in 0..60 {
            ops.push_str(&format!(
                "(Page {} line {l}: the quick brown fox jumps over the lazy dog PWLINE{p}x{l}) Tj T*\n",
                p + 1
            ));
        }
        ops.push_str("ET\n");
        let content = doc.add_object(Stream::new(dictionary! {}, ops.into_bytes()));
        kids.push(Object::Reference(doc.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
            "Contents" => content,
            "Resources" => dictionary! { "Font" => dictionary! { "F1" => font } },
        })));
    }
    doc.objects.insert(pages_id, Object::Dictionary(dictionary! { "Type" => "Pages", "Kids" => kids, "Count" => 3 }));
    let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    doc.trailer.set("Root", catalog);
    doc.save(out).expect("save");
}

#[test]
fn uncompressed_file_shrinks_with_the_same_text_and_pages() {
    let dir = tempfile::tempdir().expect("tmp");
    let fat = dir.path().join("fat.pdf");
    text_heavy(&fat);
    let before = std::fs::read(&fat).expect("read");
    let out = dir.path().join("small.pdf");
    let r = optimize(&fat, &out).expect("optimize");
    assert_eq!(std::fs::read(&fat).expect("read"), before, "source untouched");
    let after = std::fs::metadata(&out).expect("meta").len();
    assert_eq!(r.data["bytes_before"], before.len() as u64);
    assert_eq!(r.data["bytes_after"], after);
    assert!(after * 4 < before.len() as u64, "at least 75% smaller: {} -> {after}", before.len());
    assert_eq!(r.data["streams_recompressed"], 3, "{}", r.data);
    let t = texts(&out);
    assert_eq!(t, texts(&fat));
    assert!(t[2].contains("PWLINE2x59"), "last line of the last page");
    // Independent structure check: every stream is compressed now, objects are packed.
    let doc = lopdf::Document::load(&out).expect("lopdf");
    for (id, o) in &doc.objects {
        if let lopdf::Object::Stream(s) = o {
            // The writer's own (small) cross-reference stream is not document content.
            let xref = s.dict.get(b"Type").ok().and_then(|t| t.as_name().ok()) == Some(b"XRef");
            assert!(xref || s.dict.has(b"Filter"), "stream {id:?} left uncompressed: {:?}", s.dict);
        }
    }
    let raw = std::fs::read(&out).expect("read");
    assert!(raw.windows(7).any(|w| w == b"/ObjStm"), "object streams used");
}

#[test]
fn small_fixtures_shrink_from_packing_alone() {
    let dir = tempfile::tempdir().expect("tmp");
    let fat = dir.path().join("fat.pdf");
    bloated(&fixture("twenty-pages.pdf"), &fat);
    let out = dir.path().join("small.pdf");
    let r = optimize(&fat, &out).expect("optimize");
    assert!(r.data["bytes_after"].as_u64() < r.data["bytes_before"].as_u64());
    assert_eq!(texts(&out), texts(&fat));
}

#[test]
fn merged_duplicates_are_stored_once() {
    let dir = tempfile::tempdir().expect("tmp");
    let merged = dir.path().join("merged.pdf");
    let five = fixture("five-pages.pdf");
    run(Operation::Merge { inputs: vec![five.clone(), five.clone(), five], output: merged.clone() }).expect("merge");
    let out = dir.path().join("small.pdf");
    let r = optimize(&merged, &out).expect("optimize");
    assert!(r.data["duplicates_merged"].as_u64().expect("n") >= 2, "the three copies' fonts are shared: {}", r.data);
    assert_eq!(texts(&out), texts(&merged));
}

#[test]
fn form_fields_annotations_and_bookmarks_survive() {
    let dir = tempfile::tempdir().expect("tmp");
    for name in ["form-fields.pdf", "hidden-info.pdf"] {
        let fat = dir.path().join(format!("fat-{name}"));
        bloated(&fixture(name), &fat);
        let out = dir.path().join(format!("small-{name}"));
        optimize(&fat, &out).unwrap_or_else(|e| panic!("{name}: {e:?}"));
        let a = form_fields(&fat, None).expect("fields");
        let b = form_fields(&out, None).expect("fields");
        let key =
            |f: &pw_pdf::FormInfo| f.fields.iter().map(|x| format!("{}={:?}", x.name, x.value)).collect::<Vec<_>>();
        assert_eq!(key(&a), key(&b), "{name}: fields and values");
        assert_eq!(texts(&out), texts(&fat), "{name}: text");
    }
}

#[test]
fn already_compact_file_is_refused_without_writing() {
    let dir = tempfile::tempdir().expect("tmp");
    let fat = dir.path().join("fat.pdf");
    bloated(&fixture("five-pages.pdf"), &fat);
    let once = dir.path().join("once.pdf");
    optimize(&fat, &once).expect("first pass");
    let twice = dir.path().join("twice.pdf");
    let e = optimize(&once, &twice).expect_err("nothing left to gain");
    assert_eq!(e.code, "optimize.not_smaller");
    assert!(!twice.exists());
}

#[test]
fn protected_corrupt_and_existing_outputs_write_nothing() {
    let dir = tempfile::tempdir().expect("tmp");
    let out = dir.path().join("o.pdf");
    let mut req = OperationRequest::new(Operation::Optimize {
        input: fixture("encrypted-rc4-user-password.pdf"),
        images: None,
        output: out.clone(),
    });
    req.password = Some("pw-user-test".into());
    let e = execute(&req, None).expect_err("protected");
    assert_eq!(e.code, "optimize.protected");
    assert!(optimize(&fixture("corrupt-truncated.pdf"), &out).is_err());
    assert!(!out.exists());
    let fat = dir.path().join("fat.pdf");
    bloated(&fixture("single-page.pdf"), &fat);
    std::fs::write(&out, b"keep").expect("write");
    assert!(optimize(&fat, &out).is_err(), "default policy never overwrites");
    assert_eq!(std::fs::read(&out).expect("read"), b"keep");
}

#[test]
fn signed_files_warn() {
    let dir = tempfile::tempdir().expect("tmp");
    let fat = dir.path().join("fat.pdf");
    bloated(&fixture("signature-structure.pdf"), &fat);
    let r = optimize(&fat, &dir.path().join("o.pdf")).expect("optimize");
    assert!(r.warnings.iter().any(|w| w.contains("signature")), "{:?}", r.warnings);
}

#[test]
fn cancelled_at_any_point_writes_nothing() {
    let dir = tempfile::tempdir().expect("tmp");
    let out = dir.path().join("o.pdf");
    let src = fixture("uncompressed-report.pdf");
    let opts = pw_pdf::OpenOptions { password: None };
    // Cancel at once (during recompression) and after a few checks (during page verification).
    for after in [0, 6] {
        let calls = std::cell::Cell::new(0);
        let cancelled = || {
            calls.set(calls.get() + 1);
            calls.get() > after
        };
        let e = pw_pdf::optimize(&src, &opts, None, &out, Default::default(), &cancelled, &|_, _| {})
            .expect_err("cancelled");
        assert_eq!(e.category, pw_core::ErrorCategory::Cancelled, "after {after}");
        assert_eq!(std::fs::read_dir(dir.path()).expect("dir").count(), 0, "nothing written");
    }
}

/// (width, height, filter, colour space, hash of the image data) of every image in the file, sorted.
fn images_in(path: &Path) -> Vec<(i64, i64, String, String, String)> {
    use sha2::{Digest, Sha256};
    let doc = lopdf::Document::load(path).expect("lopdf");
    let mut v: Vec<_> = doc
        .objects
        .values()
        .filter_map(|o| match o {
            lopdf::Object::Stream(s)
                if s.dict.get(b"Subtype").ok().and_then(|t| t.as_name().ok()) == Some(b"Image") =>
            {
                let n = |k: &[u8]| s.dict.get(k).ok().and_then(|o| o.as_i64().ok()).unwrap_or(0);
                let name = |k: &[u8]| {
                    s.dict.get(k).ok().and_then(|o| o.as_name().ok()).map(|n| String::from_utf8_lossy(n).into_owned())
                };
                // Pixel data as decoded (Flate re-compression changes bytes, not pixels); JPEG data raw.
                let data = if name(b"Filter").as_deref() == Some("FlateDecode") {
                    s.decompressed_content().expect("decode")
                } else {
                    s.content.clone()
                };
                let hash = format!("{:x}", Sha256::digest(&data));
                Some((
                    n(b"Width"),
                    n(b"Height"),
                    name(b"Filter").unwrap_or_default(),
                    name(b"ColorSpace").unwrap_or_default(),
                    hash,
                ))
            }
            _ => None,
        })
        .collect();
    v.sort();
    v
}

fn optimize_images(input: &Path, output: &Path, dpi: u32, quality: u8) -> pw_core::PwResult<OperationResult> {
    run(Operation::Optimize {
        input: input.into(),
        images: Some(pw_core::ops::ImageReduce { max_dpi: dpi, quality }),
        output: output.into(),
    })
}

#[test]
fn images_shown_above_the_target_are_resampled_and_the_rest_kept() {
    let src = fixture("photo-heavy.pdf");
    let dir = tempfile::tempdir().expect("tmp");
    let out = dir.path().join("small.pdf");
    let r = optimize_images(&src, &out, 150, 75).expect("optimize");
    assert_eq!(r.data["images_reduced"], 3, "{}", r.data);
    assert_eq!(r.data["images_kept"], 1, "the CMYK image is not supported: {}", r.data);
    let (before, after) = (r.data["bytes_before"].as_u64().expect("b"), r.data["bytes_after"].as_u64().expect("a"));
    assert!(after * 5 < before * 2, "at least 60% smaller: {before} -> {after}");
    assert_eq!(texts(&out), texts(&src));

    let old = images_in(&src);
    let new = images_in(&out);
    let dims = |v: &[(i64, i64, String, String, String)]| v.iter().map(|i| (i.0, i.1)).collect::<Vec<_>>();
    // 1500x1000 shown at 5x3.33 in -> 750x500; 500x500 with alpha at 2 in -> 300x300 (and its mask);
    // 700x700 gray drawn through a form scaled to 2 in -> 300x300; CMYK 600x400 and the annotation's
    // 800x800 image unchanged.
    let mut want = vec![(300, 300), (300, 300), (300, 300), (600, 400), (750, 500), (800, 800)];
    want.sort();
    assert_eq!(dims(&new), want, "{new:?}");
    for kept in old.iter().filter(|i| (i.0, i.1) == (600, 400) || (i.0, i.1) == (800, 800)) {
        assert!(new.contains(kept), "image {}x{} kept byte for byte", kept.0, kept.1);
    }
    for changed in new.iter().filter(|i| i.0 == 300 || i.0 == 750) {
        let mask = changed.3 == "DeviceGray" && changed.2 == "FlateDecode";
        assert!(mask || changed.2 == "DCTDecode", "{changed:?}");
    }
}

#[test]
fn lossless_mode_never_touches_images_and_a_second_pass_explains() {
    let src = fixture("photo-heavy.pdf");
    let dir = tempfile::tempdir().expect("tmp");
    let out = dir.path().join("lossless.pdf");
    let r = optimize(&src, &out).expect("optimize");
    assert_eq!(r.data["images_reduced"], 0);
    let hashes = |p: &Path| images_in(p).into_iter().map(|i| i.4).collect::<Vec<_>>();
    assert_eq!(hashes(&out), hashes(&src), "image data unchanged (decoded)");

    let small = dir.path().join("small.pdf");
    optimize_images(&src, &small, 150, 75).expect("first");
    let again = dir.path().join("again.pdf");
    let e = optimize_images(&small, &again, 150, 75).expect_err("nothing left");
    assert_eq!(e.code, "optimize.not_smaller");
    assert!(e.action.contains("resolution"), "{e:?}");
    assert!(!again.exists());
}

#[test]
fn out_of_range_image_settings_are_refused() {
    let dir = tempfile::tempdir().expect("tmp");
    let out = dir.path().join("o.pdf");
    for (dpi, q) in [(10, 75), (5000, 75), (150, 5), (150, 100)] {
        assert!(optimize_images(&fixture("photo-heavy.pdf"), &out, dpi, q).is_err(), "{dpi} dpi, quality {q}");
    }
    assert!(!out.exists());
}
