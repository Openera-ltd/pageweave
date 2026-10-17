//! Image extraction against photo-heavy.pdf (tools/fixtures/gen_photo_pdf.py): a JPEG used on two
//! pages, a CMYK JPEG, a soft-masked Flate image, a grey image inside a form XObject, and an image
//! only used by an annotation's appearance.
use pw_core::ops::{Operation, OperationRequest, OperationResult};
use pw_pdf::execute;
use std::path::{Path, PathBuf};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/pdf").join(name)
}

fn run(input: &Path, pages: Option<&str>, dir: &Path) -> pw_core::PwResult<OperationResult> {
    execute(
        &OperationRequest::new(Operation::ExtractImages {
            input: input.into(),
            pages: pages.map(Into::into),
            output_dir: dir.into(),
        }),
        None,
    )
}

#[test]
fn every_image_once_jpeg_exact_others_png() {
    let dir = tempfile::tempdir().expect("tmp");
    let r = run(&fixture("photo-heavy.pdf"), None, dir.path()).expect("extract");
    let images = r.data["images"].as_array().expect("images");
    let got: Vec<(String, u64, u64, bool)> = images
        .iter()
        .map(|i| {
            (
                i["file"].as_str().unwrap_or("").to_string(),
                i["width"].as_u64().unwrap_or(0),
                i["height"].as_u64().unwrap_or(0),
                i["exact"] == true,
            )
        })
        .collect();
    assert_eq!(
        got,
        vec![
            ("photo-heavy_p001_01.jpg".into(), 1500, 1000, true),
            ("photo-heavy_p002_01.png".into(), 600, 400, false),
            ("photo-heavy_p003_01.png".into(), 500, 500, false),
            ("photo-heavy_p003_02.png".into(), 700, 700, false),
        ],
        "the JPEG reused on page 2 is saved once; the form's image is included"
    );
    // Independent check: the JPEG file is byte for byte the stream data in the PDF.
    let doc = lopdf::Document::load(fixture("photo-heavy.pdf")).expect("lopdf");
    let stored = doc
        .objects
        .values()
        .find_map(|o| match o {
            lopdf::Object::Stream(s) if s.dict.get(b"Width").and_then(|w| w.as_i64()).ok() == Some(1500) => {
                Some(s.content.clone())
            }
            _ => None,
        })
        .expect("jpeg stream");
    assert_eq!(std::fs::read(dir.path().join("photo-heavy_p001_01.jpg")).expect("read"), stored);
    let png = image::open(dir.path().join("photo-heavy_p003_02.png")).expect("png");
    assert_eq!((png.width(), png.height()), (700, 700));
}

#[test]
fn chosen_pages_none_found_and_failures_leave_nothing() {
    let dir = tempfile::tempdir().expect("tmp");
    let r = run(&fixture("photo-heavy.pdf"), Some("2"), dir.path()).expect("extract");
    assert_eq!(r.data["images"].as_array().expect("images").len(), 2, "page 2: the JPEG and the CMYK image");
    let empty = tempfile::tempdir().expect("tmp");
    let e = run(&fixture("five-pages.pdf"), None, empty.path()).expect_err("no images");
    assert_eq!(e.code, "extract.none");
    assert!(run(&fixture("photo-heavy.pdf"), Some("9"), empty.path()).is_err());
    assert_eq!(std::fs::read_dir(empty.path()).expect("dir").count(), 0);
}
