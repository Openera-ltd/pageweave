//! PDF → Word conversion against the real engine. Every output is read back independently here
//! (zip + XML parse, not the engine's own checker) and, with LibreOffice, opened and re-rendered
//! by a real word processor and compared with the original page by page.
//!
//! The LibreOffice round trip needs `soffice` on the PATH. Without it the test fails, unless
//! `PW_SKIP_SOFFICE=1` is set (Windows CI, where LibreOffice is not installed) — then it says
//! so loudly and does nothing.

use pw_core::ops::{Operation, OperationRequest, OperationResult};
use pw_core::output::CollisionPolicy;
use pw_core::{ErrorCategory, PwResult};
use pw_pdf::{execute, render_page_png, OpenOptions};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Instant;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/pdf").join(name)
}

fn convert_path(input: &Path, pages: Option<&str>, out: &Path) -> PwResult<OperationResult> {
    let mut req = OperationRequest::new(Operation::ConvertToDocx {
        input: input.into(),
        pages: pages.map(str::to_string),
        output: out.into(),
    });
    req.collision = CollisionPolicy::Fail;
    execute(&req, None)
}

fn convert(name: &str, out: &Path) -> OperationResult {
    convert_path(&fixture(name), None, out).unwrap_or_else(|e| panic!("{name}: {e:?}"))
}

/// What the package holds, read without the engine's checker.
struct Package {
    xml: String,
    rels: String,
    media: Vec<(String, Vec<u8>)>,
}

fn package(path: &Path) -> Package {
    let f = std::fs::File::open(path).expect("open");
    let mut z = zip::ZipArchive::new(f).expect("a zip package");
    let mut read = |name: &str| {
        let mut s = String::new();
        z.by_name(name).unwrap_or_else(|_| panic!("{name} present")).read_to_string(&mut s).expect("utf-8");
        s
    };
    let xml = read("word/document.xml");
    let rels = read("word/_rels/document.xml.rels");
    let _ = read("[Content_Types].xml");
    let names: Vec<String> =
        z.file_names().flatten().filter(|n| n.starts_with("word/media/")).map(|n| n.to_string()).collect();
    let media = names
        .into_iter()
        .map(|n| {
            let mut v = Vec::new();
            z.by_name(&n).expect("media").read_to_end(&mut v).expect("read");
            (n, v)
        })
        .collect();
    Package { xml, rels, media }
}

/// Text of `<w:t>` elements and tabs, paragraphs on their own lines (regex-free, tolerant reader).
fn docx_text(xml: &str) -> String {
    let mut out = String::new();
    let mut rest = xml;
    while let Some(i) = rest.find('<') {
        let tail = &rest[i..];
        let end = tail.find('>').expect("tag ends");
        let tag = &tail[..=end];
        rest = &tail[end + 1..];
        if tag.starts_with("<w:t>") || tag.starts_with("<w:t ") {
            let close = rest.find("</w:t>").expect("text closes");
            out.push_str(
                &rest[..close].replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&amp;", "&"),
            );
            rest = &rest[close..];
        } else if tag == "<w:tab/>" {
            out.push('\t');
        } else if tag == "</w:p>" {
            out.push('\n');
        }
    }
    out
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

fn count(hay: &str, needle: &str) -> usize {
    hay.matches(needle).count()
}

#[test]
fn every_character_of_the_pdf_is_in_the_word_file() {
    let dir = tempfile::tempdir().expect("tmp");
    for name in [
        "article.pdf",
        "five-pages.pdf",
        "search-terms.pdf",
        "uncompressed-report.pdf",
        "mixed-sizes-rotated.pdf",
        "tagged-accessible.pdf",
        "embedded-font.pdf",
        "form-fields.pdf",
        "compare-new.pdf",
        "sensitive-info.pdf",
    ] {
        let out = dir.path().join(name.replace(".pdf", ".docx"));
        let src_hash = Sha256::digest(std::fs::read(fixture(name)).expect("read"));
        let t = Instant::now();
        let r = convert(name, &out);
        eprintln!("{name}: {:?}, {} bytes", t.elapsed(), r.outputs[0].bytes);
        assert_eq!(Sha256::digest(std::fs::read(fixture(name)).expect("read")), src_hash, "{name}: source unchanged");
        let pkg = package(&out);
        let text = docx_text(&pkg.xml);
        let want = chars(&pdf_text(&fixture(name)).concat());
        assert_eq!(chars(&text), want, "{name}: the Word file holds exactly the PDF's characters");
        assert!(pkg.xml.contains("<w:sectPr>"), "{name}: sections");
    }
}

#[test]
fn structure_is_carried_over_as_editable_content() {
    let dir = tempfile::tempdir().expect("tmp");
    let out = dir.path().join("article.docx");
    let r = convert("article.pdf", &out);
    let pkg = package(&out);
    let x = &pkg.xml;
    // Three pages with their own sizes; page 3 is A4 landscape.
    assert!(x.contains("<w:pgSz w:w=\"12240\" w:h=\"15840\"/>"), "Letter");
    assert!(x.contains("<w:pgSz w:w=\"16840\" w:h=\"11900\" w:orient=\"landscape\"/>"), "A4 landscape");
    assert_eq!(count(x, "<w:type w:val=\"nextPage\"/>"), 3, "one new-page section per page");
    // Two columns under the full-width title, read column by column.
    assert!(x.contains("<w:cols w:num=\"2\" w:equalWidth=\"0\">"), "two columns");
    assert!(x.contains("<w:br w:type=\"column\"/>"), "column break between columns");
    let text = docx_text(x);
    let intro = text.find("1 Introduction").expect("heading 1");
    let methods = text.find("2 Methods").expect("heading 2");
    let dry = text.find("dry months").expect("column 1 text");
    assert!(intro < dry && dry < methods, "the first column comes before the second");
    // Headings use heading styles (navigable outline); bold and italic runs.
    assert_eq!(r.data["headings"], 5, "{}", r.data);
    assert!(count(x, "<w:pStyle w:val=\"Heading") >= 5);
    assert!(x.contains("<w:b/>") && x.contains("<w:i/>"), "bold and italic runs");
    assert!(x.contains("w:ascii=\"DejaVu Serif\""), "font family named as installed");
    // The title is centred, the justified paragraph justified.
    assert!(x.contains("<w:jc w:val=\"center\"/>"));
    assert!(x.contains("<w:jc w:val=\"both\"/>"));
    // Link on its word, external, http(s) only.
    assert_eq!(r.data["links"], 1);
    assert!(pkg.rels.contains("Target=\"https://rivers.example.org/data\" TargetMode=\"External\""));
    let link = x.find("<w:hyperlink").expect("hyperlink");
    assert!(x[link..].split("</w:hyperlink>").next().expect("part").contains("rivers.example.org"));
    // Superscript footnote mark; list items as their own paragraphs; table cells on tab stops.
    assert!(x.contains("<w:vertAlign w:val=\"superscript\"/>"));
    for item in ["• Flow rose", "• Sediment fell", "• Water quality met"] {
        assert!(text.lines().any(|l| l.starts_with(item)), "{item}");
    }
    // The ruled table is a Word table: 4 rows × 3 columns with borders, cell text in reading order.
    assert_eq!(r.data["tables"], 1, "{}", r.data);
    let tbl = &x[x.find("<w:tbl>").expect("table")..x.find("</w:tbl>").expect("table end")];
    assert_eq!(count(tbl, "<w:tr>"), 4);
    assert_eq!(count(tbl, "<w:gridCol "), 3);
    assert_eq!(count(tbl, "<w:tc>"), 12);
    assert!(tbl.contains("<w:insideH w:val=\"single\""), "borders");
    let cells: Vec<String> = tbl.split("<w:tc>").skip(1).map(docx_text).map(|t| t.trim().to_string()).collect();
    assert_eq!(cells[..6], ["Station", "Flow", "Sediment", "North", "12.4", "0.82"], "{cells:?}");
    // The picture, as stored pixels (lossless PNG for a grey image), inline in column 2.
    assert_eq!(r.data["pictures"], 1);
    assert_eq!(pkg.media.len(), 1);
    let img = image::load_from_memory(&pkg.media[0].1).expect("picture decodes");
    assert_eq!((img.width(), img.height()), (60, 20), "the image's own pixels, not a rendering");
    // The table's lines became its borders: nothing drawn is left out.
    assert_eq!(r.data["drawings_left_out"], 0, "{}", r.data);
    assert!(!r.warnings.iter().any(|w| w.contains("drawn line")), "{:?}", r.warnings);
    assert_eq!(r.data["pages_with_columns"], serde_json::json!([1]));
}

#[test]
fn pictures_keep_their_data_and_scans_are_reported() {
    let dir = tempfile::tempdir().expect("tmp");
    let out = dir.path().join("photos.docx");
    let r = convert("photo-heavy.pdf", &out);
    let pkg = package(&out);
    assert_eq!(r.data["pictures"], 5);
    // JPEG pictures are stored exactly as in the PDF.
    let jpegs: Vec<&(String, Vec<u8>)> = pkg.media.iter().filter(|(n, _)| n.ends_with(".jpeg")).collect();
    assert!(!jpegs.is_empty(), "JPEGs kept as JPEG");
    let pdf = std::fs::read(fixture("photo-heavy.pdf")).expect("read");
    for (n, j) in jpegs {
        assert!(pdf.windows(j.len().min(4096)).any(|w| w == &j[..j.len().min(4096)]), "{n} is the PDF's own JPEG data");
    }
    // Side-by-side pictures stay side by side (one row with a tab stop).
    assert!(pkg.xml.contains("<w:tabs><w:tab"), "row of pictures");

    let out = dir.path().join("scan.docx");
    let r = convert("scanned.pdf", &out);
    assert_eq!(r.data["pages_without_text"], serde_json::json!([1, 2, 3]));
    assert!(r.warnings.iter().any(|w| w.contains("Recognize text (OCR) first")), "{:?}", r.warnings);
    assert_eq!(r.data["pictures"], 3, "the scans are kept as pictures, not dropped");
}

#[test]
fn a_recognized_scan_gives_editable_text_without_the_scan_picture() {
    if pw_ocr::default_resource_dir().is_none() {
        panic!("OCR resources are required for this test (run the fetch scripts)");
    }
    let dir = tempfile::tempdir().expect("tmp");
    let searchable = dir.path().join("searchable.pdf");
    let mut req = OperationRequest::new(Operation::OcrPages {
        input: fixture("scanned.pdf"),
        pages: Some("1".into()),
        skip_text_pages: true,
        dpi: 300,
        output: searchable.clone(),
    });
    req.collision = CollisionPolicy::Fail;
    execute(&req, None).expect("ocr");
    let out = dir.path().join("scan.docx");
    let r = convert_path(&searchable, Some("1"), &out).expect("convert");
    assert_eq!(r.data["pages_from_recognized_text"], serde_json::json!([1]), "{}", r.data);
    assert_eq!(r.data["pictures"], 0, "the scan picture is left out");
    let text = docx_text(&package(&out).xml);
    assert_eq!(chars(&text), chars(&pdf_text(&searchable)[0]), "recognized text carried over");
    assert!(r.warnings.iter().any(|w| w.contains("are scans")), "{:?}", r.warnings);
}

#[test]
fn chosen_pages_only_and_failures_are_clear() {
    let dir = tempfile::tempdir().expect("tmp");
    let out = dir.path().join("two.docx");
    let r = convert_path(&fixture("five-pages.pdf"), Some("2-3"), &out).expect("pages");
    assert_eq!(r.data["pages"], 2);
    let text = docx_text(&package(&out).xml);
    let all = pdf_text(&fixture("five-pages.pdf"));
    assert_eq!(chars(&text), chars(&(all[1].clone() + &all[2])));

    // Wrong extension, existing output, password, damaged input, empty page selection.
    let e = convert_path(&fixture("five-pages.pdf"), None, &dir.path().join("x.doc")).expect_err("must fail");
    assert_eq!(e.code, "docx.extension");
    let e = convert_path(&fixture("five-pages.pdf"), None, &out).expect_err("must fail");
    assert_eq!(e.category, ErrorCategory::OutputConflict);
    let e = convert_path(&fixture("encrypted-rc4-user-password.pdf"), None, &dir.path().join("p.docx"))
        .expect_err("must fail");
    assert_eq!(e.category, ErrorCategory::PasswordRequired);
    let mut req = OperationRequest::new(Operation::ConvertToDocx {
        input: fixture("encrypted-rc4-user-password.pdf"),
        pages: None,
        output: dir.path().join("p.docx"),
    });
    req.password = Some("pw-user-test".into());
    let r = execute(&req, None).expect("with the password");
    assert!(r.warnings.iter().any(|w| w.contains("password protection does not carry over")), "{:?}", r.warnings);
    assert!(convert_path(&fixture("corrupt-truncated.pdf"), None, &dir.path().join("c.docx")).is_err());
    assert!(!dir.path().join("c.docx").exists());
    let e = convert_path(&fixture("five-pages.pdf"), Some("9"), &dir.path().join("n.docx")).expect_err("must fail");
    assert_eq!(e.category, ErrorCategory::InvalidInput);
    // Nothing staged is left behind.
    let leftovers: Vec<_> = std::fs::read_dir(dir.path())
        .expect("dir")
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with(".pw-staged"))
        .collect();
    assert!(leftovers.is_empty());
}

#[test]
fn cancelling_writes_nothing() {
    let dir = tempfile::tempdir().expect("tmp");
    let out = dir.path().join("c.docx");
    let jobs = std::sync::Arc::new(pw_core::job::JobManager::new(1));
    let req = OperationRequest::new(Operation::ConvertToDocx {
        input: fixture("twenty-pages.pdf"),
        pages: None,
        output: out.clone(),
    });
    // The job cancels itself before the engine starts, so the engine's own checks see it.
    let (tx, rx) = std::sync::mpsc::channel();
    let j = jobs.clone();
    let id = jobs.submit("convert_to_docx", move |ctx| {
        let me = rx.recv().expect("own id");
        j.cancel(me);
        execute(&req, Some(ctx)).map(|r| serde_json::to_value(r).unwrap_or_default())
    });
    tx.send(id).expect("send id");
    let s = loop {
        let s = jobs.wait(id, std::time::Duration::from_millis(100)).expect("job");
        if s.state.is_terminal() {
            break s;
        }
    };
    assert_eq!(s.state, pw_core::job::JobState::Cancelled, "{s:?}");
    assert!(!out.exists());
}

// ---- LibreOffice round trip: a real word processor opens and lays out every Word file.

fn soffice_available() -> bool {
    std::process::Command::new("soffice").arg("--version").output().map(|o| o.status.success()).unwrap_or(false)
}

/// Ink of a rendered page at 36 dpi: true where dark.
fn ink(path: &Path, page: u32) -> (Vec<bool>, u32, u32) {
    let (png, _, _) = render_page_png(path, &OpenOptions { password: None }, page, 0.5).expect("render");
    let img = image::load_from_memory(&png).expect("png").to_luma8();
    let (w, h) = img.dimensions();
    (img.pixels().map(|p| p.0[0] < 160).collect(), w, h)
}

/// Share of `a`'s ink with ink of `b` within `r` pixels.
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

fn words(s: &str) -> Vec<String> {
    s.split_whitespace().map(str::to_string).collect()
}

#[test]
fn libreoffice_opens_every_word_file_with_the_same_pages_text_and_layout() {
    if !soffice_available() {
        assert!(
            std::env::var("PW_SKIP_SOFFICE").as_deref() == Ok("1"),
            "LibreOffice (soffice) is required for the Word round-trip test; set PW_SKIP_SOFFICE=1 only where it cannot be installed"
        );
        eprintln!("SKIPPED: LibreOffice round trip (PW_SKIP_SOFFICE=1)");
        return;
    }
    // (fixture, minimum text similarity, minimum ink coverage both ways) — regression guards set
    // just under the values measured when this test was written (docs/benchmarks/DOCX_BASELINE.md).
    // Pages left out of the ink comparison are known, reported losses.
    let skip_ink: &[(&str, u32, &str)] =
        &[("photo-heavy.pdf", 4, "its only picture is a stamp comment, not carried over (reported)")];
    let cases: &[(&str, f64, f64)] = &[
        ("article.pdf", 0.99, 0.95),
        ("five-pages.pdf", 0.99, 0.90),
        ("uncompressed-report.pdf", 0.99, 0.90),
        ("photo-heavy.pdf", 0.99, 0.90),
        ("tagged-accessible.pdf", 0.99, 0.85),
        ("mixed-sizes-rotated.pdf", 0.99, 0.85),
        ("search-terms.pdf", 0.99, 0.90),
    ];
    let dir = tempfile::tempdir().expect("tmp");
    let profile = dir.path().join("lo-profile");
    let mut docx = Vec::new();
    let mut turned: HashMap<&str, Vec<u32>> = HashMap::new();
    for (name, _, _) in cases {
        let out = dir.path().join(name.replace(".pdf", ".docx"));
        let r = convert(name, &out);
        let t: Vec<u32> = serde_json::from_value(r.data["pages_turned_upright"].clone()).expect("list");
        turned.insert(name, t);
        docx.push(out);
    }
    let lo_out = dir.path().join("lo");
    let status = std::process::Command::new("soffice")
        .arg(format!("-env:UserInstallation=file://{}", profile.display()))
        .args(["--headless", "--norestore", "--convert-to", "pdf", "--outdir"])
        .arg(&lo_out)
        .args(&docx)
        .output()
        .expect("soffice runs");
    assert!(status.status.success(), "soffice: {}", String::from_utf8_lossy(&status.stderr));
    let mut report = Vec::new();
    let mut failures = Vec::new();
    for (name, min_text, min_ink) in cases {
        let back = lo_out.join(name);
        assert!(back.exists(), "LibreOffice opened and laid out {name}");
        let orig_text = pdf_text(&fixture(name));
        let back_text = pdf_text(&back);
        assert_eq!(back_text.len(), orig_text.len(), "{name}: same number of pages");
        // Turned pages read in a different order; their characters are checked elsewhere.
        let keep = |t: &[String]| {
            t.iter()
                .enumerate()
                .filter(|(i, _)| !turned[name].contains(&(*i as u32 + 1)))
                .map(|(_, s)| s.as_str())
                .collect::<Vec<_>>()
                .join(" ")
        };
        let (a, b) = (words(&keep(&orig_text)), words(&keep(&back_text)));
        let same: usize = similar::capture_diff_slices(similar::Algorithm::Myers, &a, &b)
            .iter()
            .map(|op| match op {
                similar::DiffOp::Equal { len, .. } => *len,
                _ => 0,
            })
            .sum();
        let ratio = 2.0 * same as f64 / (a.len() + b.len()).max(1) as f64;
        let mut worst = 1.0f64;
        for p in 0..orig_text.len() as u32 {
            let (ia, w, h) = ink(&fixture(name), p);
            let (ib, w2, h2) = ink(&back, p);
            if skip_ink.iter().any(|(n, pg, _)| n == name && *pg == p + 1) {
                continue;
            }
            if turned[name].contains(&(p + 1)) {
                // Laid out so its text reads upright: the page's sides swap; ink is not comparable.
                assert_eq!((w, h), (h2, w2), "{name} page {}: turned upright", p + 1);
                continue;
            }
            assert_eq!((w, h), (w2, h2), "{name} page {}: same page size", p + 1);
            let (fwd, back_cov) = (covered(&ia, &ib, w, h, 3), covered(&ib, &ia, w, h, 3));
            eprintln!("  {name} page {}: original ink kept {fwd:.3}, new ink on original {back_cov:.3}", p + 1);
            worst = worst.min(fwd).min(back_cov);
        }
        report.push(format!("{name}: words {ratio:.4}, worst page ink coverage {worst:.3}"));
        if ratio < *min_text || worst < *min_ink {
            failures.push(format!("{name}: words {ratio:.4} (min {min_text}), ink {worst:.3} (min {min_ink})"));
        }
    }
    eprintln!("{}", report.join("\n"));
    assert!(failures.is_empty(), "{failures:#?}");
}

#[test]
fn right_to_left_text_is_kept_in_reading_order_and_marked_right_to_left() {
    // Hebrew drawn glyph by glyph from right to left, as browsers write it (tools/fixtures/gen_rtl.py).
    let dir = tempfile::tempdir().expect("tmp");
    let out = dir.path().join("rtl.docx");
    convert("rtl-hebrew.pdf", &out);
    let p = package(&out);
    let paras: Vec<&str> = p.xml.split("<w:p>").skip(1).filter(|s| s.contains("<w:t")).collect();
    let text = |s: &str| docx_text(&format!("<w:p>{s}")).trim().to_string();
    let hebrew = paras.iter().find(|s| text(s).contains("שלום")).expect("the Hebrew paragraph");
    // One paragraph, words in reading order with their spaces, both lines joined.
    assert_eq!(text(hebrew), "שלום עולם, זהו מסמך לדוגמה השורה השנייה של הפסקה");
    assert!(hebrew.contains("<w:bidi/>") && hebrew.contains("<w:rtl/>"), "marked right-to-left");
    // Right-aligned: the paragraph's start in a right-to-left paragraph.
    assert!(hebrew.contains("<w:jc w:val=\"left\"/>"), "aligned to its start (the right)");
    let english = paras.iter().find(|s| text(s).starts_with("An English line")).expect("English paragraph");
    assert!(!english.contains("<w:bidi/>") && !english.contains("<w:rtl/>"));
}
