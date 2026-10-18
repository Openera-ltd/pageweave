//! Visual signatures (typed, drawn, picture) placed in a box, upright as the page is displayed.
use pw_core::ops::{Operation, OperationRequest, OperationResult, RectPt, SignatureMark, TypedStyle};
use pw_pdf::execute;
use std::path::{Path, PathBuf};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/pdf").join(name)
}

fn place(
    input: &Path,
    page: u32,
    rect: RectPt,
    mark: SignatureMark,
    output: &Path,
) -> pw_core::PwResult<OperationResult> {
    let req = OperationRequest::new(Operation::AddSignatureMark {
        input: input.into(),
        page,
        rect,
        mark,
        output: output.into(),
    });
    req.validate()?;
    execute(&req, None)
}

fn typed(text: &str) -> SignatureMark {
    SignatureMark::Typed { text: text.into(), style: TypedStyle::Formal, color: "#1a2b5c".into() }
}

/// The displayed page at 72 dpi, grey.
fn render(p: &Path, page: u32) -> image::GrayImage {
    let png = pw_pdf::render_page_png(p, &pw_pdf::OpenOptions { password: None }, page, 1.0).expect("render").0;
    image::load_from_memory(&png).expect("png").to_luma8()
}

/// Bounding box (x0, y0, x1, y1) of dark pixels inside a displayed area.
fn ink_bounds(img: &image::GrayImage, x0: u32, y0: u32, x1: u32, y1: u32) -> Option<(u32, u32, u32, u32)> {
    let mut b: Option<(u32, u32, u32, u32)> = None;
    for y in y0..y1.min(img.height()) {
        for x in x0..x1.min(img.width()) {
            if img.get_pixel(x, y).0[0] < 160 {
                b = Some(match b {
                    None => (x, y, x, y),
                    Some((a, c, d, e)) => (a.min(x), c.min(y), d.max(x), e.max(y)),
                });
            }
        }
    }
    b
}

#[test]
fn typed_signature_is_text_in_the_box_and_upright_on_a_rotated_page() {
    let dir = tempfile::tempdir().expect("tmp");
    let out = dir.path().join("typed.pdf");
    let src = fixture("five-pages.pdf");
    let before = std::fs::read(&src).expect("read");
    place(&src, 0, RectPt { left: 350.0, bottom: 40.0, right: 560.0, top: 100.0 }, typed("Ada Lovelace"), &out)
        .expect("place");
    assert_eq!(std::fs::read(&src).expect("read"), before, "source untouched");
    let t =
        execute(&OperationRequest::new(Operation::ExtractText { input: out.clone(), pages: Some("1".into()) }), None)
            .expect("text");
    assert!(t.data["pages"][0]["text"].as_str().unwrap_or_default().contains("Ada Lovelace"));
    // Page 3 of mixed-sizes-rotated.pdf is 612 × 792 with /Rotate 90: displayed 792 wide, 612 tall.
    // Unrotated box x 400–460, y 100–300 shows as x 100–300, y 400–460 (from the top).
    let rot = dir.path().join("rotated.pdf");
    place(
        &fixture("mixed-sizes-rotated.pdf"),
        2,
        RectPt { left: 400.0, bottom: 100.0, right: 460.0, top: 300.0 },
        typed("Ada Lovelace"),
        &rot,
    )
    .expect("place rotated");
    let img = render(&rot, 2);
    assert_eq!((img.width(), img.height()), (792, 612));
    let (x0, y0, x1, y1) = ink_bounds(&img, 100, 400, 300, 460).expect("ink in the displayed box");
    assert!(x1 - x0 > 3 * (y1 - y0), "a horizontal line of text as displayed: {x0},{y0}–{x1},{y1}");
}

#[test]
fn drawn_strokes_land_in_the_box_with_their_shape() {
    let dir = tempfile::tempdir().expect("tmp");
    let out = dir.path().join("drawn.pdf");
    // A wide zigzag across the middle of the box, and a dot.
    let zig: Vec<[f32; 2]> = (0..=10).map(|i| [0.05 + 0.09 * i as f32, if i % 2 == 0 { 0.3 } else { 0.7 }]).collect();
    let mark = SignatureMark::Drawn { strokes: vec![zig, vec![[0.95, 0.9]]], color: "#000000".into(), width: 2.0 };
    place(&fixture("single-page.pdf"), 0, RectPt { left: 100.0, bottom: 100.0, right: 300.0, top: 160.0 }, mark, &out)
        .expect("place");
    let img = render(&out, 0);
    let h = img.height();
    // Displayed box: x 100–300, y (from top) h-160 .. h-100.
    let (x0, y0, x1, y1) = ink_bounds(&img, 95, h - 165, 305, h - 95).expect("ink");
    assert!(x0 <= 112 && x1 >= 285, "spans the box width: {x0}..{x1}");
    assert!(y1 - y0 >= 20, "zigzag height");
    // Nothing drawn outside the box.
    assert!(ink_bounds(&img, 0, h - 95, img.width(), h).is_none(), "below the box stays clean");
}

#[test]
fn transparent_picture_keeps_the_page_visible_around_the_ink() {
    let dir = tempfile::tempdir().expect("tmp");
    // 300 × 100 transparent picture with an opaque dark band in the middle third.
    let mut pic = image::RgbaImage::from_pixel(300, 100, image::Rgba([0, 0, 0, 0]));
    for y in 40..60 {
        for x in 20..280 {
            pic.put_pixel(x, y, image::Rgba([20, 30, 90, 255]));
        }
    }
    let path = dir.path().join("sig.png");
    pic.save(&path).expect("save png");
    let out = dir.path().join("picture.pdf");
    // Over a blank area of the page: the transparent parts must stay white.
    place(
        &fixture("single-page.pdf"),
        0,
        RectPt { left: 100.0, bottom: 300.0, right: 400.0, top: 400.0 },
        SignatureMark::Image { path: path.clone() },
        &out,
    )
    .expect("place");
    let img = render(&out, 0);
    let h = img.height();
    let (_, y0, _, y1) = ink_bounds(&img, 95, h - 405, 405, h - 295).expect("band drawn");
    assert!((y1 - y0) <= 24, "only the opaque band is dark: {y0}..{y1}");
    // A transparent corner of the picture is still white.
    assert!(img.get_pixel(105, h - 395).0[0] > 240, "transparent area stays clear");
}

#[test]
fn refusals_write_nothing() {
    let dir = tempfile::tempdir().expect("tmp");
    let out = dir.path().join("o.pdf");
    let r = RectPt { left: 100.0, bottom: 100.0, right: 300.0, top: 160.0 };
    let code = |res: pw_core::PwResult<OperationResult>| res.expect_err("refused").code;
    assert_eq!(code(place(&fixture("five-pages.pdf"), 0, r, typed("李小龍"), &out)), "mark.text_charset");
    assert_eq!(code(place(&fixture("five-pages.pdf"), 0, r, typed("  "), &out)), "mark.text");
    assert_eq!(
        code(place(
            &fixture("five-pages.pdf"),
            0,
            r,
            SignatureMark::Drawn { strokes: vec![], color: "#000000".into(), width: 1.5 },
            &out
        )),
        "mark.empty"
    );
    assert_eq!(
        code(place(
            &fixture("five-pages.pdf"),
            0,
            r,
            SignatureMark::Drawn { strokes: vec![vec![[3.0, 0.5]]], color: "#000000".into(), width: 1.5 },
            &out
        )),
        "mark.points"
    );
    assert_eq!(code(place(&fixture("five-pages.pdf"), 9, r, typed("A"), &out)), "mark.page");
    assert_eq!(
        code(place(
            &fixture("five-pages.pdf"),
            0,
            RectPt { left: 0.0, bottom: 0.0, right: 4.0, top: 4.0 },
            typed("A"),
            &out
        )),
        "mark.box"
    );
    assert_eq!(
        code(place(&fixture("five-pages.pdf"), 0, r, SignatureMark::Image { path: fixture("five-pages.pdf") }, &out)),
        "mark.image_format"
    );
    assert!(place(
        &fixture("five-pages.pdf"),
        0,
        r,
        SignatureMark::Image { path: dir.path().join("missing.png") },
        &out
    )
    .is_err());
    assert!(!out.exists());
}

#[test]
fn signed_documents_are_warned_about() {
    let dir = tempfile::tempdir().expect("tmp");
    let out = dir.path().join("o.pdf");
    let r = place(
        &fixture("signed-rsa.pdf"),
        0,
        RectPt { left: 100.0, bottom: 100.0, right: 300.0, top: 160.0 },
        typed("Ada"),
        &out,
    )
    .expect("place");
    assert!(r.warnings.iter().any(|w| w.to_lowercase().contains("signature")), "{:?}", r.warnings);
}
