//! Design Reconstruction Studio against the real recognition engine: the poster fixture is rebuilt
//! and measured (wording, boxes, colours, editable share, look of the export), edited and exported
//! again; bad requests are refused. Skipped with a message when the OCR resources are missing.
use base64::Engine as _;
use pw_core::output::CollisionPolicy;
use pw_pdf::studio::{export_design, read_design, reconstruct_design, save_design, Design, DesignFormat, ObjectKind};
use pw_pdf::{render_page_png, OpenOptions};
use std::path::{Path, PathBuf};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/pdf").join(name)
}

fn have_resources() -> bool {
    let ok = pw_ocr::default_resource_dir().is_some();
    assert!(ok || std::env::var_os("CI").is_none(), "OCR resources missing in CI (tools/fetch_ocr.py)");
    if !ok {
        eprintln!("OCR resources not installed (python3 tools/fetch_ocr.py); skipping");
    }
    ok
}

fn never() -> bool {
    false
}

/// The poster's known layout in PDF points (left, bottom, right, top), from gen_fixtures.py.
const BANNER: [f32; 4] = [0.0, 400.0, 400.0, 500.0];
const RULE: [f32; 4] = [40.0, 292.0, 360.0, 295.0];
const BUTTON: [f32; 4] = [40.0, 230.0, 190.0, 270.0];
const PICTURE: [f32; 4] = [250.0, 170.0, 360.0, 280.0];
const LINES: [&str; 6] = [
    "SPRING FAIR",
    "Saturday 14 May",
    "Music, food and crafts",
    "Riverside Park, 10:00 to 18:00",
    "Free entry",
    "Bring a friend",
];

/// Overlap (intersection over union) of a design object with a box in points.
fn iou(d: &Design, o: &pw_pdf::studio::DesignObject, b: [f32; 4]) -> f32 {
    let k = d.pt_per_px;
    let page_h = d.height as f32 * k;
    let (ax0, ay0, ax1, ay1) = (o.x * k, page_h - (o.y + o.h) * k, (o.x + o.w) * k, page_h - o.y * k);
    let iw = (ax1.min(b[2]) - ax0.max(b[0])).max(0.0);
    let ih = (ay1.min(b[3]) - ay0.max(b[1])).max(0.0);
    let inter = iw * ih;
    inter / ((ax1 - ax0) * (ay1 - ay0) + (b[2] - b[0]) * (b[3] - b[1]) - inter)
}

fn best(d: &Design, kind: ObjectKind, b: [f32; 4]) -> (&pw_pdf::studio::DesignObject, f32) {
    d.objects
        .iter()
        .filter(|o| o.kind == kind)
        .map(|o| (o, iou(d, o, b)))
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .expect("an object of that kind")
}

fn words(s: &str) -> Vec<String> {
    s.split_whitespace()
        .map(|w| w.trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase())
        .filter(|w| !w.is_empty())
        .collect()
}

/// Mean absolute difference per channel (0–255) between the design's source and a render of `pdf`.
fn difference(d: &Design, pdf: &Path) -> f32 {
    let src =
        image::load_from_memory(&base64::engine::general_purpose::STANDARD.decode(&d.source_png).expect("test value"))
            .expect("test value")
            .to_rgb8();
    let scale = 1.0 / d.pt_per_px;
    let (png, _, _) = render_page_png(pdf, &OpenOptions::default(), 0, scale).expect("test value");
    let out = image::load_from_memory(&png).expect("test value").to_rgb8();
    let (w, h) = (src.width().min(out.width()), src.height().min(out.height()));
    let mut sum = 0u64;
    for y in 0..h {
        for x in 0..w {
            let (a, b) = (src.get_pixel(x, y), out.get_pixel(x, y));
            sum += (0..3).map(|k| (a[k] as i32 - b[k] as i32).unsigned_abs() as u64).sum::<u64>();
        }
    }
    sum as f32 / (w * h * 3) as f32
}

#[test]
fn poster_is_rebuilt_as_editable_objects_and_exports_verify() {
    if !have_resources() {
        return;
    }
    let dir = tempfile::tempdir().expect("test value");
    let project = dir.path().join("poster.pwdesign");
    let (c, s) =
        reconstruct_design(&fixture("poster.pdf"), &OpenOptions::default(), 0, &project, CollisionPolicy::Fail, &never)
            .expect("reconstruct");
    assert_eq!(c.path, project);
    let d = read_design(&project).expect("test value");
    eprintln!("summary: {s:?}");
    for o in &d.objects {
        eprintln!(
            "{:?} {:<32} x{:.0} y{:.0} w{:.0} h{:.0} conf {:.2} {:?} {:?} size {:?} bold {}",
            o.kind, o.name, o.x, o.y, o.w, o.h, o.confidence, o.color, o.fill, o.font_size, o.bold
        );
    }

    // Wording: every expected word recognised (word recall), nothing invented beyond a stray word.
    let got: Vec<String> = d
        .objects
        .iter()
        .filter(|o| o.kind == ObjectKind::Text)
        .flat_map(|o| words(o.text.as_deref().expect("test value")))
        .collect();
    let want: Vec<String> = LINES.iter().flat_map(|l| words(l)).collect();
    let found = want.iter().filter(|w| got.contains(w)).count();
    let recall = found as f32 / want.len() as f32;
    let extra = got.iter().filter(|w| !want.contains(w)).count();
    eprintln!("word recall {recall:.3} ({found}/{}), extra words {extra}", want.len());
    assert!(recall >= 0.95, "word recall {recall}");
    assert!(extra <= 1, "invented words: {got:?}");

    // Boxes and colours.
    let (banner, i1) = best(&d, ObjectKind::Shape, BANNER);
    let (rule, i2) = best(&d, ObjectKind::Shape, RULE);
    let (button, i3) = best(&d, ObjectKind::Shape, BUTTON);
    let (picture, i4) = best(&d, ObjectKind::Image, PICTURE);
    eprintln!("IoU banner {i1:.3} rule {i2:.3} button {i3:.3} picture {i4:.3}");
    assert!(i1 > 0.95 && i3 > 0.95 && i4 > 0.9, "boxes {i1} {i3} {i4}");
    assert!(i2 > 0.6, "thin rule {i2}");
    let near = |hex: &str, want: [i32; 3]| {
        let v = u32::from_str_radix(&hex[1..], 16).expect("test value");
        let c = [(v >> 16) as i32 & 255, (v >> 8) as i32 & 255, v as i32 & 255];
        (0..3).all(|k| (c[k] - want[k]).abs() <= 12)
    };
    assert!(near(banner.fill.as_deref().expect("test value"), [31, 59, 122]), "banner {:?}", banner.fill);
    assert!(near(rule.fill.as_deref().expect("test value"), [204, 26, 26]), "rule {:?}", rule.fill);
    assert!(near(button.fill.as_deref().expect("test value"), [242, 140, 26]), "button {:?}", button.fill);
    assert!(picture.note.is_some(), "pictures are marked as kept, not interpreted");
    assert!(near(&d.background, [250, 245, 230]), "background {}", d.background);

    let text = |needle: &str| {
        d.objects
            .iter()
            .find(|o| {
                o.kind == ObjectKind::Text && o.text.as_deref().expect("test value").to_lowercase().contains(needle)
            })
            .unwrap_or_else(|| panic!("text {needle}"))
    };
    let (title, heading, body, entry) = (text("spring"), text("saturday"), text("music"), text("free"));
    assert!(
        near(title.color.as_deref().expect("test value"), [255, 255, 255])
            && near(entry.color.as_deref().expect("test value"), [255, 255, 255])
    );
    assert!(near(heading.color.as_deref().expect("test value"), [0, 0, 0]));
    assert!(near(body.color.as_deref().expect("test value"), [64, 64, 64]), "body {:?}", body.color);
    assert!(title.bold && heading.bold && entry.bold && !body.bold, "weights");
    // Sizes in points within 12 % of the fixture's (32, 18, 12, 16).
    for (o, pt) in [(title, 32.0), (heading, 18.0), (body, 12.0), (entry, 16.0)] {
        let got = o.font_size.expect("test value") * d.pt_per_px;
        assert!((got - pt).abs() / pt < 0.12, "{} size {got} pt, want {pt}", o.name);
    }
    // Objects back to front: text above the boxes it sits on.
    let pos = |id: &str| d.objects.iter().position(|o| o.id == id).expect("test value");
    assert!(pos(&title.id) > pos(&banner.id) && pos(&entry.id) > pos(&button.id));
    assert!(s.editable_ratio >= 0.85, "editable ratio {}", s.editable_ratio);

    // Exports read back; the PDF looks like the source.
    let svg = dir.path().join("poster.svg");
    export_design(&project, DesignFormat::Svg, &svg, CollisionPolicy::Fail).expect("test value");
    let svg_text = std::fs::read_to_string(&svg).expect("test value");
    assert!(svg_text.contains(">SPRING FAIR</text>") || svg_text.contains("SPRING"), "title is SVG text");
    let pdf = dir.path().join("poster.pdf");
    let (_, warnings) = export_design(&project, DesignFormat::Pdf, &pdf, CollisionPolicy::Fail).expect("test value");
    assert!(warnings.is_empty(), "{warnings:?}");
    let diff = difference(&d, &pdf);
    eprintln!("mean pixel difference of the PDF export: {diff:.2} / 255");
    assert!(diff < 6.0, "export looks different: {diff}");
    // Exports never replace without being asked.
    assert!(export_design(&project, DesignFormat::Pdf, &pdf, CollisionPolicy::Fail).is_err());

    // After an edit: new wording in the export, old gone; a hidden object leaves the export.
    let mut edited = d.clone();
    let t = edited.objects.iter_mut().find(|o| o.id == title.id).expect("test value");
    t.text = Some("SUMMER FAIR".into());
    edited.objects.iter_mut().find(|o| o.id == picture.id).expect("test value").visible = false;
    let project2 = dir.path().join("edited.pwdesign");
    save_design(&edited, &project2, CollisionPolicy::Fail, &[]).expect("test value");
    let pdf2 = dir.path().join("edited.pdf");
    export_design(&project2, DesignFormat::Pdf, &pdf2, CollisionPolicy::Fail).expect("test value");
    let r = pw_pdf::execute(
        &pw_core::ops::OperationRequest::new(pw_core::ops::Operation::ExtractText { input: pdf2.clone(), pages: None }),
        None,
    )
    .expect("test value");
    let page_text = r.data["pages"][0]["text"].as_str().expect("test value").to_string();
    assert!(page_text.contains("SUMMER FAIR") && !page_text.contains("SPRING"), "{page_text}");
    let svg2 = dir.path().join("edited.svg");
    export_design(&project2, DesignFormat::Svg, &svg2, CollisionPolicy::Fail).expect("test value");
    assert!(!std::fs::read_to_string(&svg2).expect("test value").contains("<image"), "hidden picture left out");
}

#[test]
fn a_png_is_rebuilt_too() {
    if !have_resources() {
        return;
    }
    let dir = tempfile::tempdir().expect("test value");
    let (png, _, _) = render_page_png(&fixture("poster.pdf"), &OpenOptions::default(), 0, 2.0).expect("test value");
    let src = dir.path().join("flyer.png");
    std::fs::write(&src, png).expect("test value");
    let out = dir.path().join("flyer.pwdesign");
    let (_, s) =
        reconstruct_design(&src, &OpenOptions::default(), 0, &out, CollisionPolicy::Fail, &never).expect("test value");
    let d = read_design(&out).expect("test value");
    assert_eq!((d.width, d.height), (800, 1000));
    assert_eq!(d.source_name, "flyer.png");
    assert!(s.text >= 6 && s.shapes >= 3 && s.images >= 1, "{s:?}");
    assert!((d.pt_per_px - 0.75).abs() < 1e-6);
}

fn code<T>(r: Result<T, pw_core::PwError>) -> String {
    r.err().map(|e| e.code.to_string()).unwrap_or_default()
}

#[test]
fn bad_requests_are_refused() {
    let dir = tempfile::tempdir().expect("test value");
    let opts = OpenOptions::default();
    // Wrong project extension, not an image, missing page.
    assert_eq!(
        code(reconstruct_design(
            &fixture("poster.pdf"),
            &opts,
            0,
            &dir.path().join("x.json"),
            CollisionPolicy::Fail,
            &never
        )),
        "design.extension"
    );
    assert_eq!(
        code(reconstruct_design(
            &fixture("not-a-pdf.pdf"),
            &opts,
            0,
            &dir.path().join("x.pwdesign"),
            CollisionPolicy::Fail,
            &never
        )),
        "design.format"
    );
    if have_resources() {
        assert_eq!(
            code(reconstruct_design(
                &fixture("poster.pdf"),
                &opts,
                3,
                &dir.path().join("x.pwdesign"),
                CollisionPolicy::Fail,
                &never
            )),
            "design.page"
        );
    }
    // A file that is not a design, and a design with a hostile picture.
    let fake = dir.path().join("fake.pwdesign");
    std::fs::write(&fake, b"{\"hello\": 1}").expect("test value");
    assert_eq!(code(read_design(&fake)), "design.unreadable");
    let hostile = dir.path().join("hostile.pwdesign");
    std::fs::write(
        &hostile,
        br##"{"version":1,"width":10,"height":10,"pt_per_px":1,"background":"#ffffff","source_png":"","source_name":"",
            "objects":[{"id":"p","name":"p","kind":"image","x":0,"y":0,"w":5,"h":5,"png":"PHN2Zz48L3N2Zz4="}]}"##,
    )
    .expect("test value");
    assert_eq!(code(read_design(&hostile)), "design.png");
    assert_eq!(
        code(export_design(&hostile, DesignFormat::Svg, &dir.path().join("h.svg"), CollisionPolicy::Fail)),
        "design.png"
    );
    assert!(!dir.path().join("h.svg").exists());
    // Export extension must match the format.
    assert_eq!(
        code(export_design(&fake, DesignFormat::Pdf, &dir.path().join("x.svg"), CollisionPolicy::Fail)),
        "design.extension"
    );
}

#[test]
fn rotation_and_groups_survive_export() {
    let dir = tempfile::tempdir().expect("test value");
    let bar = pw_pdf::studio::DesignObject {
        id: "bar".into(),
        name: "Bar".into(),
        kind: ObjectKind::Shape,
        x: 50.0,
        y: 90.0,
        w: 100.0,
        h: 20.0,
        visible: true,
        locked: false,
        confidence: 1.0,
        note: None,
        text: None,
        color: None,
        font_size: None,
        bold: false,
        baseline: None,
        fill: Some("#000000".into()),
        png: None,
        group: Some("g1".into()),
        rotation: 90.0,
        ellipse: false,
        radius: 0.0,
    };
    let d = Design {
        version: 1,
        width: 200,
        height: 200,
        pt_per_px: 1.0,
        background: "#ffffff".into(),
        source_png: String::new(),
        source_name: "t".into(),
        objects: vec![bar],
    };
    let project = dir.path().join("r.pwdesign");
    save_design(&d, &project, CollisionPolicy::Fail, &[]).expect("test value");
    assert_eq!(read_design(&project).expect("test value").objects[0].group.as_deref(), Some("g1"));
    let svg = dir.path().join("r.svg");
    export_design(&project, DesignFormat::Svg, &svg, CollisionPolicy::Fail).expect("test value");
    assert!(std::fs::read_to_string(&svg).expect("test value").contains("transform=\"rotate(90 100 100)\""));
    let pdf = dir.path().join("r.pdf");
    export_design(&project, DesignFormat::Pdf, &pdf, CollisionPolicy::Fail).expect("test value");
    // The 100 × 20 bar turned a quarter around its centre (100, 100) is 20 wide and 100 tall.
    let (png, _, _) = render_page_png(&pdf, &OpenOptions::default(), 0, 1.0).expect("test value");
    let img = image::load_from_memory(&png).expect("test value").to_luma8();
    let dark: Vec<(u32, u32)> = img.enumerate_pixels().filter(|p| p.2[0] < 100).map(|p| (p.0, p.1)).collect();
    let (x0, x1) = (dark.iter().map(|p| p.0).min().expect("ink"), dark.iter().map(|p| p.0).max().expect("ink"));
    let (y0, y1) = (dark.iter().map(|p| p.1).min().expect("ink"), dark.iter().map(|p| p.1).max().expect("ink"));
    assert!((x1 - x0 + 1).abs_diff(20) <= 2 && (y1 - y0 + 1).abs_diff(100) <= 2, "box {x0}..{x1} × {y0}..{y1}");
    assert!((x0 + x1) / 2 >= 98 && (x0 + x1) / 2 <= 102 && (y0 + y1) / 2 >= 98 && (y0 + y1) / 2 <= 102, "centred");
    // Out-of-range turns and empty group ids are refused.
    let mut bad = d.clone();
    bad.objects[0].rotation = 1000.0;
    assert_eq!(code(save_design(&bad, &dir.path().join("b.pwdesign"), CollisionPolicy::Fail, &[])), "design.invalid");
    bad.objects[0].rotation = 0.0;
    bad.objects[0].group = Some(String::new());
    assert_eq!(code(save_design(&bad, &dir.path().join("c.pwdesign"), CollisionPolicy::Fail, &[])), "design.invalid");
}

#[test]
fn circles_ellipses_and_rounded_rectangles_become_vectors() {
    if !have_resources() {
        return;
    }
    let dir = tempfile::tempdir().expect("test value");
    let project = dir.path().join("badges.pwdesign");
    let (_, s) =
        reconstruct_design(&fixture("badges.pdf"), &OpenOptions::default(), 0, &project, CollisionPolicy::Fail, &never)
            .expect("reconstruct");
    let d = read_design(&project).expect("test value");
    for o in &d.objects {
        eprintln!(
            "{:?} {} x{:.0} y{:.0} w{:.0} h{:.0} ellipse {} radius {} conf {:.3} {:?}",
            o.kind, o.name, o.x, o.y, o.w, o.h, o.ellipse, o.radius, o.confidence, o.text
        );
    }
    assert_eq!(s.images, 0, "every one-colour shape is a vector: {s:?}");
    // Boxes in points from gen_fixtures.py (left, bottom, right, top).
    let (circle, i1) = best(&d, ObjectKind::Shape, [20.0, 100.0, 100.0, 180.0]);
    let (oval, i2) = best(&d, ObjectKind::Shape, [130.0, 120.0, 270.0, 180.0]);
    let (button, i3) = best(&d, ObjectKind::Shape, [20.0, 30.0, 160.0, 80.0]);
    let (plain, i4) = best(&d, ObjectKind::Shape, [190.0, 30.0, 280.0, 90.0]);
    assert!(i1 > 0.95 && i2 > 0.95 && i3 > 0.95 && i4 > 0.95, "boxes {i1} {i2} {i3} {i4}");
    assert!(circle.ellipse && oval.ellipse, "circle and oval are ellipses");
    assert!(!button.ellipse && !plain.ellipse);
    let want_r = 16.0 / d.pt_per_px;
    assert!((button.radius - want_r).abs() / want_r < 0.15, "button radius {} px, want {want_r}", button.radius);
    assert_eq!(plain.radius, 0.0, "square corners stay square");
    let sale = d.objects.iter().find(|o| o.kind == ObjectKind::Text).expect("wording");
    assert_eq!(sale.text.as_deref(), Some("SALE 50%"));
    assert_eq!(
        sale.color.as_deref().map(|c| c.to_lowercase()).as_deref().map(|c| c >= "#f0f0f0"),
        Some(true),
        "white wording"
    );
    // Exports: SVG uses ellipse and rounded rect; the PDF looks like the source.
    let svg = dir.path().join("b.svg");
    export_design(&project, DesignFormat::Svg, &svg, CollisionPolicy::Fail).expect("svg");
    let text = std::fs::read_to_string(&svg).expect("test value");
    assert_eq!(text.matches("<ellipse ").count(), 2);
    assert!(text.contains(" rx=\""), "rounded corners in the SVG");
    let pdf = dir.path().join("b.pdf");
    export_design(&project, DesignFormat::Pdf, &pdf, CollisionPolicy::Fail).expect("pdf");
    let diff = difference(&d, &pdf);
    eprintln!("mean pixel difference {diff:.2}");
    assert!(diff < 4.0, "export looks different: {diff}");
}
