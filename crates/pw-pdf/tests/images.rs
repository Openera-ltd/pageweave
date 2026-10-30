//! Images to PDF against the real engine.
use pw_core::ops::{ImagePageSize, Operation, OperationRequest};
use pw_core::ErrorCategory;
use pw_pdf::{execute, open_document, render_page_png, OpenOptions};
use std::path::{Path, PathBuf};

fn img(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/images").join(name)
}

fn run(
    inputs: Vec<PathBuf>,
    size: ImagePageSize,
    margin: f32,
    out: &Path,
) -> pw_core::PwResult<pw_core::ops::OperationResult> {
    execute(
        &OperationRequest::new(Operation::ImagesToPdf { inputs, page_size: size, margin, output: out.into() }),
        None,
    )
}

fn opts() -> OpenOptions<'static> {
    OpenOptions { password: None }
}

#[test]
fn photos_keep_their_bytes_and_stand_upright() {
    let dir = tempfile::tempdir().expect("tmp");
    let out = dir.path().join("photos.pdf");
    let r =
        run(vec![img("photo-exif-rotate.jpg"), img("gray-300dpi.jpg"), img("alpha.png")], ImagePageSize::A4, 0.0, &out)
            .expect("convert");
    assert_eq!(r.outputs[0].pages, Some(3));
    let info = open_document(&out, &opts()).expect("open");
    // EXIF orientation 6: the 64x48 photo is shown turned, so as a portrait A4 page with /Rotate 90.
    assert_eq!(info.pages[0].rotation, 90);
    assert_eq!((info.pages[0].width_pt.round(), info.pages[0].height_pt.round()), (842.0, 595.0));
    assert_eq!(info.pages[1].rotation, 0);
    // The JPEG files are embedded byte for byte (independent parser).
    let doc = lopdf::Document::load(&out).expect("lopdf");
    for name in ["photo-exif-rotate.jpg", "gray-300dpi.jpg"] {
        let want = std::fs::read(img(name)).expect("read");
        assert!(
            doc.objects.values().any(|o| matches!(o, lopdf::Object::Stream(s) if s.content == want)),
            "{name} not embedded unchanged"
        );
    }
    // Shown upright: the band stored along the top is on the right after the 90° turn.
    let (png, w, h) = render_page_png(&out, &opts(), 0, 0.5).expect("render");
    let page = image::load_from_memory(&png).expect("png").to_rgb8();
    let (right, left) = (page.get_pixel(w - 10, h / 2).0, page.get_pixel(10, h / 2).0);
    assert!(right[2] > 150 && right[0] < 100, "right edge blue: {right:?}");
    assert!(left[0] > 150 && left[2] < 100, "left edge red: {left:?}");
    // PNG transparency shows the white page, not black.
    let (png, w, h) = render_page_png(&out, &opts(), 2, 0.5).expect("render");
    let page = image::load_from_memory(&png).expect("png").to_rgb8();
    let p = page.get_pixel(w * 3 / 4, h / 2).0;
    assert!(p.iter().all(|&c| c > 230), "transparent half renders white: {p:?}");
}

#[test]
fn natural_size_uses_the_recorded_resolution() {
    let dir = tempfile::tempdir().expect("tmp");
    let out = dir.path().join("n.pdf");
    run(vec![img("gray-300dpi.jpg"), img("alpha.png")], ImagePageSize::Image, 9.0, &out).expect("convert");
    let info = open_document(&out, &opts()).expect("open");
    // 30x60 px at 300 dpi = 7.2x14.4 pt, plus 9 pt margins; 40x60 at 150 dpi = 19.2x28.8 pt.
    let s =
        |i: usize| ((info.pages[i].width_pt * 10.0).round() / 10.0, (info.pages[i].height_pt * 10.0).round() / 10.0);
    assert_eq!(s(0), (25.2, 32.4));
    assert_eq!(s(1), (37.2, 46.8));
}

#[test]
fn damaged_and_unsupported_files_are_refused() {
    let dir = tempfile::tempdir().expect("tmp");
    let out = dir.path().join("x.pdf");
    let err = run(vec![img("alpha.png"), img("truncated.jpg")], ImagePageSize::A4, 0.0, &out).expect_err("truncated");
    assert_eq!(err.code, "images.damaged");
    assert!(err.message.contains("truncated.jpg"), "{}", err.message);
    let txt = dir.path().join("notes.jpg");
    std::fs::write(&txt, b"not an image").expect("write");
    assert_eq!(
        run(vec![txt], ImagePageSize::A4, 0.0, &out).expect_err("format").category,
        ErrorCategory::UnsupportedFormat
    );
    assert_eq!(run(vec![], ImagePageSize::A4, 0.0, &out).expect_err("none").code, "images.none");
    assert_eq!(run(vec![img("alpha.png")], ImagePageSize::A4, 500.0, &out).expect_err("margin").code, "images.margin");
    assert!(!out.exists());
}

#[test]
fn webp_tiff_gif_and_bmp_become_pages_and_a_multi_page_tiff_is_reported() {
    let dir = tempfile::tempdir().expect("tmp");
    // A 120 × 80 picture: left half blue, right half red.
    let pic =
        image::RgbImage::from_fn(
            120,
            80,
            |x, _| if x < 60 { image::Rgb([20, 60, 200]) } else { image::Rgb([210, 30, 40]) },
        );
    let mut inputs = Vec::new();
    for (name, fmt) in [
        ("p.webp", image::ImageFormat::WebP),
        ("p.tif", image::ImageFormat::Tiff),
        ("p.gif", image::ImageFormat::Gif),
        ("p.bmp", image::ImageFormat::Bmp),
    ] {
        let path = dir.path().join(name);
        image::DynamicImage::ImageRgb8(pic.clone()).save_with_format(&path, fmt).expect("encode");
        inputs.push(path);
    }
    // A two-page TIFF: only its first page is used, and the user is told.
    let multi = dir.path().join("two.tif");
    {
        let mut enc = tiff::encoder::TiffEncoder::new(std::fs::File::create(&multi).expect("create")).expect("tiff");
        for _ in 0..2 {
            enc.write_image::<tiff::encoder::colortype::RGB8>(120, 80, pic.as_raw()).expect("page");
        }
    }
    inputs.push(multi);
    let out = dir.path().join("all.pdf");
    let r = run(inputs, ImagePageSize::Image, 0.0, &out).expect("converted");
    assert_eq!(r.data["page_count"], 5);
    assert!(r.warnings.iter().any(|w| w.contains("first page of two.tif (2 pages)")), "{:?}", r.warnings);
    // Each page shows the picture: blue on the left, red on the right.
    for page in 0..5 {
        let (png, w, h) = render_page_png(&out, &opts(), page, 1.0).expect("render");
        let rendered = image::load_from_memory(&png).expect("png").to_rgb8();
        let (l, r) = (rendered.get_pixel(w / 4, h / 2), rendered.get_pixel(3 * w / 4, h / 2));
        assert!(l[2] > 150 && l[0] < 80 && r[0] > 150 && r[2] < 90, "page {page}: {l:?} {r:?}");
    }
}
