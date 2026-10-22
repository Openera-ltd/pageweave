//! Office documents → PDF with the installed LibreOffice (D-019): Word, Excel, PowerPoint and
//! text files convert with their text; macros never run; missing LibreOffice, timeouts,
//! cancellation and unsupported files are clear; originals are never changed.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_pageweave"))
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/pdf").join(name)
}

fn json(out: &Output) -> serde_json::Value {
    serde_json::from_slice(&out.stdout)
        .unwrap_or_else(|e| panic!("not JSON ({e}): {}", String::from_utf8_lossy(&out.stdout)))
}

fn office_available() -> bool {
    let ok = pw_pdf::office::find_office().is_some();
    if !ok {
        assert!(
            std::env::var("PW_SKIP_SOFFICE").as_deref() == Ok("1"),
            "LibreOffice is required; set PW_SKIP_SOFFICE=1 only where it cannot be installed"
        );
        eprintln!("SKIPPED: LibreOffice not installed (PW_SKIP_SOFFICE=1)");
    }
    ok
}

fn text_of(pdf: &Path) -> String {
    let t = bin().args(["--json", "text"]).arg(pdf).output().expect("run");
    json(&t)["result"]["data"]["pages"]
        .as_array()
        .expect("pages")
        .iter()
        .map(|p| p["text"].as_str().unwrap_or("").to_string())
        .collect::<Vec<_>>()
        .join("\n")
}

fn convert(input: &Path, out: &Path) -> Output {
    bin().args(["--json", "from-office"]).arg(input).arg("-o").arg(out).output().expect("run")
}

#[test]
fn word_excel_powerpoint_and_text_files_convert_with_their_text() {
    if !office_available() {
        return;
    }
    let dir = tempfile::tempdir().expect("tmp");
    // Office files made by PageWeave's own exports, plus plain text.
    let docx = dir.path().join("article.docx");
    assert!(bin()
        .arg("to-docx")
        .arg(fixture("article.pdf"))
        .arg("-o")
        .arg(&docx)
        .output()
        .expect("run")
        .status
        .success());
    let xlsx = dir.path().join("tables.xlsx");
    assert!(bin()
        .arg("tables")
        .arg(fixture("tables.pdf"))
        .arg("-o")
        .arg(&xlsx)
        .output()
        .expect("run")
        .status
        .success());
    let pptx = dir.path().join("slides.pptx");
    assert!(bin()
        .args(["to-pptx", "--pages", "1"])
        .arg(fixture("article.pdf"))
        .arg("-o")
        .arg(&pptx)
        .output()
        .expect("run")
        .status
        .success());
    let txt = dir.path().join("notes ünïcode.txt");
    std::fs::write(&txt, "Meeting notes PWOFFICETXT\nSecond line — with a dash\n").expect("write");
    for (src, needle) in [(&docx, "1 Introduction"), (&xlsx, "1,234.50"), (&pptx, "PWARTICLE"), (&txt, "PWOFFICETXT")] {
        let before = std::fs::read(src).expect("read");
        let out = dir.path().join(format!("{}.pdf", src.file_stem().expect("stem").to_string_lossy()));
        let r = convert(src, &out);
        assert!(r.status.success(), "{}: {}", src.display(), String::from_utf8_lossy(&r.stdout));
        let v = json(&r);
        assert!(v["result"]["data"]["pages"].as_u64().unwrap_or(0) >= 1);
        assert!(v["result"]["data"]["engine"].as_str().unwrap_or("").starts_with("LibreOffice"));
        assert!(text_of(&out).contains(needle), "{}: {needle}", src.display());
        assert_eq!(std::fs::read(src).expect("read"), before, "original unchanged");
    }
    // The Word round trip keeps every word of the article's first page.
    let back = text_of(&dir.path().join("article.pdf"));
    for w in ["Annual", "review", "rivers.example.org", "Station", "Discussion"] {
        assert!(back.contains(w), "{w}");
    }
}

/// An OpenDocument text whose Basic macro runs on open and would write `marker`.
fn macro_document(path: &Path, marker: &Path) {
    let f = std::fs::File::create(path).expect("create");
    let mut z = zip::ZipWriter::new(f);
    let stored = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    z.start_file("mimetype", stored).expect("zip");
    z.write_all(b"application/vnd.oasis.opendocument.text").expect("zip");
    let files = [
        ("META-INF/manifest.xml", r#"<?xml version="1.0" encoding="UTF-8"?><manifest:manifest xmlns:manifest="urn:oasis:names:tc:opendocument:xmlns:manifest:1.0" manifest:version="1.2"><manifest:file-entry manifest:full-path="/" manifest:media-type="application/vnd.oasis.opendocument.text"/><manifest:file-entry manifest:full-path="content.xml" manifest:media-type="text/xml"/><manifest:file-entry manifest:full-path="Basic/" manifest:media-type=""/><manifest:file-entry manifest:full-path="Basic/script-lc.xml" manifest:media-type="text/xml"/><manifest:file-entry manifest:full-path="Basic/Standard/" manifest:media-type=""/><manifest:file-entry manifest:full-path="Basic/Standard/script-lb.xml" manifest:media-type="text/xml"/><manifest:file-entry manifest:full-path="Basic/Standard/Module1.xml" manifest:media-type="text/xml"/></manifest:manifest>"#.to_string()),
        ("content.xml", r#"<?xml version="1.0" encoding="UTF-8"?><office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:script="urn:oasis:names:tc:opendocument:xmlns:script:1.0" xmlns:xlink="http://www.w3.org/1999/xlink" office:version="1.2"><office:scripts><office:event-listeners><script:event-listener script:language="ooo:script" script:event-name="office:load" xlink:href="vnd.sun.star.script:Standard.Module1.Main?language=Basic&amp;location=document" xlink:type="simple"/></office:event-listeners></office:scripts><office:body><office:text><text:p>Macro test PWMACRO</text:p></office:text></office:body></office:document-content>"#.to_string()),
        ("Basic/script-lc.xml", r#"<?xml version="1.0" encoding="UTF-8"?><!DOCTYPE library:libraries PUBLIC "-//OpenOffice.org//DTD OfficeDocument 1.0//EN" "libraries.dtd"><library:libraries xmlns:library="http://openoffice.org/2000/library" xmlns:xlink="http://www.w3.org/1999/xlink"><library:library library:name="Standard" library:link="false"/></library:libraries>"#.to_string()),
        ("Basic/Standard/script-lb.xml", r#"<?xml version="1.0" encoding="UTF-8"?><!DOCTYPE library:library PUBLIC "-//OpenOffice.org//DTD OfficeDocument 1.0//EN" "library.dtd"><library:library xmlns:library="http://openoffice.org/2000/library" library:name="Standard" library:readonly="false" library:passwordprotected="false"><library:element library:name="Module1"/></library:library>"#.to_string()),
        ("Basic/Standard/Module1.xml", format!(r#"<?xml version="1.0" encoding="UTF-8"?><!DOCTYPE script:module PUBLIC "-//OpenOffice.org//DTD OfficeDocument 1.0//EN" "module.dtd"><script:module xmlns:script="http://openoffice.org/2000/script" script:name="Module1" script:language="StarBasic">Sub Main
  Open "{}" For Output As #1
  Print #1, "macro ran"
  Close #1
End Sub</script:module>"#, marker.display())),
    ];
    let o = zip::write::SimpleFileOptions::default();
    for (name, body) in files {
        z.start_file(name, o).expect("zip");
        z.write_all(body.as_bytes()).expect("zip");
    }
    z.finish().expect("zip");
}

/// Limit of this test: on LibreOffice 24.2 headless conversion this document's macro did not run
/// even in a profile with macro security switched off, so passing does not by itself prove the
/// hardening works; it guards against a macro ever running. The private profile (security level
/// "very high", macro execution disabled) is applied to every conversion regardless (D-019).
#[test]
fn macros_never_run() {
    if !office_available() {
        return;
    }
    let dir = tempfile::tempdir().expect("tmp");
    let marker = dir.path().join("macro-ran.txt");
    let odt = dir.path().join("macro.odt");
    macro_document(&odt, &marker);
    let out = dir.path().join("macro.pdf");
    let r = convert(&odt, &out);
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stdout));
    assert!(text_of(&out).contains("PWMACRO"));
    assert!(!marker.exists(), "the document's macro must not run");
}

#[test]
fn failures_are_clear_and_write_nothing() {
    let dir = tempfile::tempdir().expect("tmp");
    let txt = dir.path().join("a.txt");
    std::fs::write(&txt, "hello").expect("write");
    // Not an Office file.
    let r = convert(&fixture("five-pages.pdf"), &dir.path().join("x.pdf"));
    assert_eq!(json(&r)["error"]["code"], "office.format");
    // Wrong output name.
    let r =
        bin().args(["--json", "from-office"]).arg(&txt).arg("-o").arg(dir.path().join("x.docx")).output().expect("run");
    assert_eq!(json(&r)["error"]["code"], "office.extension");
    // LibreOffice missing: a clear reason and what to do.
    let r = bin()
        .env("PAGEWEAVE_SOFFICE", dir.path().join("no-such-soffice"))
        .args(["--json", "from-office"])
        .arg(&txt)
        .arg("-o")
        .arg(dir.path().join("m.pdf"))
        .output()
        .expect("run");
    let v = json(&r);
    assert_eq!(v["error"]["code"], "office.not_installed");
    assert!(v["error"]["action"].as_str().unwrap_or("").contains("LibreOffice"));
    assert!(!dir.path().join("m.pdf").exists());
    if !office_available() {
        return;
    }
    // A damaged document.
    let bad = dir.path().join("broken.docx");
    std::fs::write(&bad, b"PK\x03\x04 not really a document").expect("write");
    let r = convert(&bad, &dir.path().join("broken.pdf"));
    assert!(!r.status.success());
    assert!(!dir.path().join("broken.pdf").exists());
    // Time limit and cancellation (library API).
    let e = pw_pdf::office::convert_office_to_pdf(
        &txt,
        &dir.path().join("t.pdf"),
        pw_core::output::CollisionPolicy::Fail,
        &|| false,
        std::time::Duration::from_millis(1),
    )
    .expect_err("too slow");
    assert_eq!(e.code, "office.timeout");
    let e = pw_pdf::office::convert_office_to_pdf(
        &txt,
        &dir.path().join("c.pdf"),
        pw_core::output::CollisionPolicy::Fail,
        &|| true,
        pw_pdf::office::CONVERT_TIMEOUT,
    )
    .expect_err("cancelled");
    assert_eq!(e.category, pw_core::ErrorCategory::Cancelled);
    assert!(!dir.path().join("t.pdf").exists() && !dir.path().join("c.pdf").exists());
}
