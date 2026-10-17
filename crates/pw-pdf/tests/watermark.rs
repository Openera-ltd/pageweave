//! Text watermarks against the real engine: placement, angle, opacity, layering and visibility.
use pw_core::ops::{Operation, OperationRequest, WatermarkLayer, WatermarkSpec};
use pw_pdf::{execute, render_page_png, search, OpenOptions};
use std::path::{Path, PathBuf};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/pdf").join(name)
}

fn spec(text: &str) -> WatermarkSpec {
    WatermarkSpec {
        text: text.into(),
        font_size: 72.0,
        color: "#808080".into(),
        opacity: 0.25,
        angle: 45.0,
        layer: WatermarkLayer::Over,
    }
}

fn run(
    input: &Path,
    output: &Path,
    pages: Option<&str>,
    w: WatermarkSpec,
) -> pw_core::PwResult<pw_core::ops::OperationResult> {
    execute(
        &OperationRequest::new(Operation::Watermark {
            input: input.into(),
            pages: pages.map(Into::into),
            watermark: w,
            output: output.into(),
        }),
        None,
    )
}

fn hit_rect(path: &Path, word: &str) -> Vec<(u32, f32, f32)> {
    let (hits, _) = search(path, &OpenOptions { password: None }, word, true, false, &|| false).expect("search");
    hits.iter()
        .map(|h| {
            let (mut l, mut b, mut r, mut t) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
            for q in &h.rects {
                l = l.min(q.left);
                b = b.min(q.bottom);
                r = r.max(q.right);
                t = t.max(q.top);
            }
            (h.page, r - l, t - b)
        })
        .collect()
}

#[test]
fn translucent_watermark_on_every_page() {
    let src = fixture("five-pages.pdf");
    let before = std::fs::read(&src).expect("read");
    let dir = tempfile::tempdir().expect("tmp");
    let out = dir.path().join("wm.pdf");
    let r = run(&src, &out, None, spec("DRAFT")).expect("watermark");
    assert!(r.warnings.is_empty(), "{:?}", r.warnings);
    assert_eq!(std::fs::read(&src).expect("read"), before, "source untouched");
    let hits = hit_rect(&out, "DRAFT");
    assert_eq!(hits.iter().map(|h| h.0).collect::<Vec<_>>(), vec![0, 1, 2, 3, 4]);
    // 45°: the text's bounding box is roughly square.
    let (_, w, h) = hits[0];
    assert!((w / h - 1.0).abs() < 0.3, "45° box {w}x{h}");
    // Translucent: the darkest pixel near the centre is a blend of grey on white, not solid grey.
    let (png, _, _) = render_page_png(&out, &OpenOptions { password: None }, 0, 1.0).expect("render");
    let img = image::load_from_memory(&png).expect("png").to_rgb8();
    let mut darkest = 255u8;
    for y in 300..500 {
        for x in 200..420 {
            darkest = darkest.min(img.get_pixel(x, y).0[0]);
        }
    }
    assert!((205..=240).contains(&darkest), "25% grey on white should be ~223, got {darkest}");
}

#[test]
fn angle_is_as_displayed_even_on_rotated_pages() {
    let src = fixture("mixed-sizes-rotated.pdf"); // page 3 has /Rotate 90
    let dir = tempfile::tempdir().expect("tmp");
    let out = dir.path().join("wm.pdf");
    let mut w = spec("STRAIGHT");
    w.angle = 0.0;
    run(&src, &out, None, w).expect("watermark");
    for (page, wd, ht) in hit_rect(&out, "STRAIGHT") {
        // Search rectangles are in unrotated page space: a horizontal (as displayed) line on the
        // /Rotate 90 page is vertical there.
        if page == 2 {
            assert!(ht > wd * 3.0, "page 3 ({wd}x{ht}) should be vertical in page space");
        } else {
            assert!(wd > ht * 3.0, "page {} ({wd}x{ht}) should be horizontal", page + 1);
        }
    }
}

#[test]
fn chosen_pages_only_and_hidden_behind_a_scan_is_reported() {
    let dir = tempfile::tempdir().expect("tmp");
    let out = dir.path().join("p2.pdf");
    run(&fixture("five-pages.pdf"), &out, Some("2, 4"), spec("COPY")).expect("watermark");
    assert_eq!(hit_rect(&out, "COPY").iter().map(|h| h.0).collect::<Vec<_>>(), vec![1, 3]);

    // A scanned page is one opaque image: behind it, the watermark cannot be seen — said, not hidden.
    let behind = dir.path().join("behind.pdf");
    let mut w = spec("SECRET");
    w.layer = WatermarkLayer::Behind;
    let r = run(&fixture("scanned.pdf"), &behind, Some("1"), w.clone()).expect("watermark");
    assert!(
        r.warnings.iter().any(|m| m.contains("cannot be seen on page 1") && m.contains("covers it")),
        "{:?}",
        r.warnings
    );
    let over = dir.path().join("over.pdf");
    w.layer = WatermarkLayer::Over;
    let r = run(&fixture("scanned.pdf"), &over, Some("1"), w).expect("watermark");
    assert!(!r.warnings.iter().any(|m| m.contains("cannot be seen")), "{:?}", r.warnings);
}

#[test]
fn invalid_watermarks_are_refused_before_writing() {
    let dir = tempfile::tempdir().expect("tmp");
    let out = dir.path().join("x.pdf");
    let src = fixture("five-pages.pdf");
    let cases: Vec<(WatermarkSpec, &str)> = vec![
        (spec("  "), "watermark.empty"),
        (WatermarkSpec { opacity: 0.0, ..spec("A") }, "watermark.opacity"),
        (WatermarkSpec { angle: 120.0, ..spec("A") }, "watermark.angle"),
        (WatermarkSpec { font_size: 2.0, ..spec("A") }, "watermark.size"),
        (WatermarkSpec { color: "red".into(), ..spec("A") }, "watermark.color"),
        (spec("机密"), "watermark.charset"),
    ];
    for (w, code) in cases {
        assert_eq!(run(&src, &out, None, w).expect_err(code).code, code);
    }
    assert!(!out.exists());
}
