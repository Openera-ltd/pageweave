//! PDF → PowerPoint against the real engine: slides read back independently here (zip + text of
//! every <a:t>), and with LibreOffice the presentation is laid out again and compared with the
//! original page by page.

use pw_core::ops::{Operation, OperationRequest, OperationResult};
use pw_core::output::CollisionPolicy;
use pw_core::{ErrorCategory, PwResult};
use pw_pdf::{execute, render_page_png, OpenOptions};
use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/pdf").join(name)
}

fn convert(input: &Path, pages: Option<&str>, out: &Path) -> PwResult<OperationResult> {
    let mut req = OperationRequest::new(Operation::ConvertToPptx {
        input: input.into(),
        pages: pages.map(str::to_string),
        output: out.into(),
    });
    req.collision = CollisionPolicy::Fail;
    execute(&req, None)
}

fn slides(path: &Path) -> Vec<String> {
    let mut z = zip::ZipArchive::new(std::fs::File::open(path).expect("open")).expect("zip");
    let mut out = Vec::new();
    for i in 1.. {
        let mut s = String::new();
        match z.by_name(&format!("ppt/slides/slide{i}.xml")) {
            Ok(mut f) => f.read_to_string(&mut s).expect("utf-8"),
            Err(_) => break,
        };
        out.push(s);
    }
    out
}

fn text_of(xml: &str) -> String {
    let mut t = String::new();
    for part in xml.split("<a:t>").skip(1) {
        let end = part.find("</a:t>").expect("text closes");
        t.push_str(
            &part[..end].replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&amp;", "&"),
        );
        t.push(' ');
    }
    t
}

fn chars(s: &str) -> HashMap<char, usize> {
    let mut m = HashMap::new();
    for c in s.chars().filter(|c| !c.is_whitespace()) {
        *m.entry(c).or_default() += 1;
    }
    m
}

fn pdf_text(path: &Path) -> Vec<String> {
    let r = execute(&OperationRequest::new(Operation::ExtractText { input: path.into(), pages: None }), None)
        .expect("text");
    r.data["pages"].as_array().expect("pages").iter().map(|p| p["text"].as_str().unwrap_or("").to_string()).collect()
}

#[test]
fn one_slide_per_page_with_all_text_tables_pictures_and_links() {
    let dir = tempfile::tempdir().expect("tmp");
    let out = dir.path().join("article.pptx");
    let r = convert(&fixture("article.pdf"), None, &out).expect("convert");
    let s = slides(&out);
    assert_eq!(s.len(), 3);
    let pdf = pdf_text(&fixture("article.pdf"));
    for (i, x) in s.iter().enumerate() {
        assert_eq!(chars(&text_of(x)), chars(&pdf[i]), "slide {} holds its page's characters", i + 1);
    }
    assert!(s[1].contains("<a:tbl>") && s[1].matches("<a:tr ").count() == 4, "the ruled table is a table");
    assert!(s[0].contains("<p:pic>"), "the picture");
    assert!(s[0].contains("<a:hlinkClick r:id="), "the link");
    assert!(s[0].contains("b=\"1\"") && s[0].contains("i=\"1\""), "bold and italic");
    assert!(s[0].contains("typeface=\"DejaVu Serif\""));
    assert_eq!(
        (r.data["tables"].as_u64(), r.data["pictures"].as_u64(), r.data["links"].as_u64()),
        (Some(1), Some(1), Some(1))
    );
    // Slides take the first page's size; page 3 (A4 landscape) is scaled and the report says so.
    let mut z = zip::ZipArchive::new(std::fs::File::open(&out).expect("open")).expect("zip");
    let mut pres = String::new();
    z.by_name("ppt/presentation.xml").expect("presentation").read_to_string(&mut pres).expect("utf-8");
    assert!(pres.contains("<p:sldSz cx=\"7772400\" cy=\"10058400\"/>"), "Letter");
    assert!(r.warnings.iter().any(|w| w.contains("scaled to the slide size")), "{:?}", r.warnings);
}

#[test]
fn errors_are_clear_and_nothing_is_left_behind() {
    let dir = tempfile::tempdir().expect("tmp");
    let e = convert(&fixture("five-pages.pdf"), None, &dir.path().join("x.ppt")).expect_err("ext");
    assert_eq!(e.code, "pptx.extension");
    let e = convert(&fixture("encrypted-rc4-user-password.pdf"), None, &dir.path().join("p.pptx")).expect_err("pw");
    assert_eq!(e.category, ErrorCategory::PasswordRequired);
    assert!(convert(&fixture("corrupt-truncated.pdf"), None, &dir.path().join("c.pptx")).is_err());
    let two = dir.path().join("two.pptx");
    convert(&fixture("five-pages.pdf"), Some("2,4"), &two).expect("pages");
    assert_eq!(slides(&two).len(), 2);
    assert_eq!(
        convert(&fixture("five-pages.pdf"), None, &two).expect_err("exists").category,
        ErrorCategory::OutputConflict
    );
    let left: Vec<_> = std::fs::read_dir(dir.path())
        .expect("dir")
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with(".pw-staged"))
        .collect();
    assert!(left.is_empty());
}

fn ink(path: &Path, page: u32) -> (Vec<bool>, u32, u32) {
    let (png, _, _) = render_page_png(path, &OpenOptions { password: None }, page, 0.5).expect("render");
    let img = image::load_from_memory(&png).expect("png").to_luma8();
    let (w, h) = img.dimensions();
    (img.pixels().map(|p| p.0[0] < 160).collect(), w, h)
}

fn covered(a: &[bool], b: &[bool], w: u32, h: u32, r: i32) -> f64 {
    let (mut hit, mut total) = (0u64, 0u64);
    for y in 0..h as i32 {
        for x in 0..w as i32 {
            if !a[(y as u32 * w + x as u32) as usize] {
                continue;
            }
            total += 1;
            'near: for dy in -r..=r {
                for dx in -r..=r {
                    let (xx, yy) = (x + dx, y + dy);
                    if xx >= 0 && yy >= 0 && xx < w as i32 && yy < h as i32 && b[(yy as u32 * w + xx as u32) as usize] {
                        hit += 1;
                        break 'near;
                    }
                }
            }
        }
    }
    if total == 0 {
        1.0
    } else {
        hit as f64 / total as f64
    }
}

#[test]
fn libreoffice_lays_out_the_slides_like_the_pages() {
    let available =
        std::process::Command::new("soffice").arg("--version").output().map(|o| o.status.success()).unwrap_or(false);
    if !available {
        assert!(
            std::env::var("PW_SKIP_SOFFICE").as_deref() == Ok("1"),
            "LibreOffice (soffice) is required; set PW_SKIP_SOFFICE=1 only where it cannot be installed"
        );
        eprintln!("SKIPPED: LibreOffice slide round trip (PW_SKIP_SOFFICE=1)");
        return;
    }
    let dir = tempfile::tempdir().expect("tmp");
    let mut report = Vec::new();
    // (fixture, pages, minimum ink coverage both ways) — guards just under the measured values.
    for (name, pages, min_ink) in [
        ("article.pdf", "1-2", 0.92),
        ("photo-heavy.pdf", "1-3", 0.97),
        ("tagged-accessible.pdf", "1", 0.97),
        ("tables.pdf", "1-2", 0.97),
    ] {
        let out = dir.path().join(name.replace(".pdf", ".pptx"));
        convert(&fixture(name), Some(pages), &out).expect("convert");
        let st = std::process::Command::new("soffice")
            .arg(format!("-env:UserInstallation=file://{}", dir.path().join("lo-profile").display()))
            .args(["--headless", "--norestore", "--convert-to", "pdf", "--outdir"])
            .arg(dir.path().join("lo"))
            .arg(&out)
            .output()
            .expect("soffice");
        let back = dir.path().join("lo").join(name);
        assert!(
            back.exists(),
            "LibreOffice laid out {name} (is Impress installed?): {}",
            String::from_utf8_lossy(&st.stderr)
        );
        let n = pages.split('-').map(|p| p.parse::<u32>().expect("page")).collect::<Vec<_>>();
        let (first, last) = (n[0], *n.last().expect("page"));
        let back_text = pdf_text(&back);
        assert_eq!(back_text.len() as u32, last - first + 1, "{name}: one page per slide");
        let mut worst = 1.0f64;
        for (k, p) in (first - 1..last).enumerate() {
            let (a, w, h) = ink(&fixture(name), p);
            let (b, w2, h2) = ink(&back, k as u32);
            assert_eq!((w, h), (w2, h2), "{name} page {}: slide size", p + 1);
            worst = worst.min(covered(&a, &b, w, h, 3)).min(covered(&b, &a, w, h, 3));
            let orig = pdf_text(&fixture(name));
            assert_eq!(chars(&back_text[k]), chars(&orig[p as usize]), "{name} page {}: same characters", p + 1);
        }
        report.push(format!("{name} pages {pages}: worst ink coverage {worst:.3}"));
        assert!(worst >= min_ink, "{name}: ink {worst:.3} < {min_ink}");
    }
    eprintln!("{}", report.join("\n"));
}
