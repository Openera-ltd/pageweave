//! Office documents → PDF (PRD 06 interchange: DOCX, XLSX, PPTX, ODT, ODS, ODP, RTF, TXT, HTML,
//! CSV), decision D-019. The layout engine is LibreOffice (MPL-2.0) when it is installed on this
//! computer; it is not bundled. Each conversion runs in a fresh, private LibreOffice profile with
//! macros switched off (security level "very high", macro execution disabled), on a copy of the
//! input in a private temporary folder, with a time limit and cancellation. The PDF it produces is
//! opened with PDFium and must have pages before it is written (atomically, never over the input).

use crate::document::{load, OpenOptions};
use crate::engine;
use pw_core::output::{write_atomic_bytes, CollisionPolicy, CommittedOutput};
use pw_core::{ErrorCategory, PwError, PwResult};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Extensions LibreOffice opens that PageWeave offers (lower case).
pub const OFFICE_EXTENSIONS: &[&str] = &[
    // Word
    "docx", "docm", "doc", "dotx", "dotm", "dot", "rtf", "txt", // Excel
    "xlsx", "xlsm", "xlsb", "xls", "xltx", "xltm", "xlt", "csv", // PowerPoint
    "pptx", "pptm", "ppt", "ppsx", "ppsm", "pps", "potx", "potm", "pot", // Visio and Publisher
    "vsdx", "vsd", "pub", // OpenDocument and web pages
    "odt", "ods", "odp", "odg", "html", "htm",
];
/// Inputs larger than this are refused.
const MAX_INPUT_BYTES: u64 = 512 * 1024 * 1024;
/// Explicit LibreOffice program (tests, unusual installs).
pub const SOFFICE_ENV: &str = "PAGEWEAVE_SOFFICE";
/// A conversion running longer than this is stopped.
pub const CONVERT_TIMEOUT: Duration = Duration::from_secs(300);

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OfficeEngine {
    pub program: PathBuf,
    /// e.g. "LibreOffice 24.2.7.2".
    pub version: String,
}

/// Where PageWeave keeps its own LibreOffice (installed with the full download or downloaded from
/// Settings): `%LOCALAPPDATA%\io.github.kuyamcliff.pageweave\office`.
pub fn office_dir() -> Option<PathBuf> {
    Some(pw_net::data_dir()?.join("office"))
}

/// The program inside an unpacked LibreOffice (an administrative image puts it under
/// `LibreOffice\program` or `program`).
fn program_in(dir: &Path) -> Vec<PathBuf> {
    let exe = if cfg!(windows) { "soffice.exe" } else { "soffice" };
    let mut v = vec![dir.join("program").join(exe)];
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            if e.path().is_dir() {
                v.push(e.path().join("program").join(exe));
            }
        }
    }
    v
}

fn candidates() -> Vec<PathBuf> {
    let mut v = Vec::new();
    // An explicit setting is the only place looked at (empty or missing: treated as not installed).
    if let Some(p) = std::env::var_os(SOFFICE_ENV) {
        v.push(PathBuf::from(p));
        return v;
    }
    // PageWeave's own copy first (the version it was tested with).
    if let Some(d) = office_dir() {
        v.extend(program_in(&d));
    }
    let exe = if cfg!(windows) { "soffice.exe" } else { "soffice" };
    if let Some(path) = std::env::var_os("PATH") {
        for d in std::env::split_paths(&path) {
            v.push(d.join(exe));
        }
    }
    if cfg!(windows) {
        for var in ["ProgramFiles", "ProgramFiles(x86)", "ProgramW6432"] {
            if let Some(d) = std::env::var_os(var) {
                v.push(PathBuf::from(d).join("LibreOffice").join("program").join(exe));
            }
        }
    } else {
        v.push(PathBuf::from("/usr/bin/soffice"));
        v.push(PathBuf::from("/usr/lib/libreoffice/program/soffice"));
        v.push(PathBuf::from("/opt/libreoffice/program/soffice"));
    }
    v
}

/// The installed LibreOffice, if any (its version is read from the program itself).
pub fn find_office() -> Option<OfficeEngine> {
    candidates()
        .into_iter()
        .filter(|p| p.is_file())
        .find_map(|p| version_of(&p).map(|version| OfficeEngine { program: p, version }))
}

/// "LibreOffice 24.2.7.2" from `program --version`, or None when it is not a working LibreOffice.
/// A candidate that does not start is skipped (it used to end the whole search).
fn version_of(program: &Path) -> Option<String> {
    let out = Command::new(program).arg("--version").stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    (out.status.success() && text.contains("LibreOffice"))
        .then(|| text.split_whitespace().take(2).collect::<Vec<_>>().join(" "))
}

fn not_installed() -> PwError {
    PwError::new(
        ErrorCategory::MissingResource,
        "office.not_installed",
        "Opening Word, Excel and PowerPoint files needs LibreOffice, which is not installed yet.",
    )
    .with_action(if cfg!(windows) {
        "Download LibreOffice once from Settings ▸ Resources (or with `pageweave office --install`), then try again."
    } else {
        "Install LibreOffice with your package manager, then try again."
    })
}

/// The pinned LibreOffice package (resources/office.lock.json).
#[derive(Debug, Clone, serde::Deserialize, Serialize)]
pub struct OfficePackage {
    pub version: String,
    pub url: String,
    pub sha256: String,
    pub bytes: u64,
}

pub fn office_package() -> PwResult<OfficePackage> {
    let v: serde_json::Value = serde_json::from_str(include_str!("../../../resources/office.lock.json"))
        .map_err(|_| PwError::internal("office.lock", "The Office package pin is damaged."))?;
    let w = &v["windows_x64"];
    Ok(OfficePackage {
        version: v["version"].as_str().unwrap_or_default().to_string(),
        url: w["url"].as_str().unwrap_or_default().to_string(),
        sha256: w["sha256"].as_str().unwrap_or_default().to_string(),
        bytes: w["bytes"].as_u64().unwrap_or(0),
    })
}

/// Download the pinned LibreOffice package (resumable, checked against its SHA-256) and unpack it
/// into [`office_dir`] with `msiexec /a` (files only: nothing is registered with Windows and no
/// administrator rights are needed). Windows only. Returns the program found.
pub fn install_office(cancelled: &dyn Fn() -> bool, progress: &dyn Fn(u64, u64)) -> PwResult<OfficeEngine> {
    if !cfg!(windows) {
        return Err(PwError::new(
            ErrorCategory::UnsupportedOperation,
            "office.install_platform",
            "PageWeave downloads LibreOffice on Windows only.",
        )
        .with_action("Install LibreOffice with your package manager."));
    }
    let pkg = office_package()?;
    let dir = office_dir().ok_or_else(|| {
        PwError::new(ErrorCategory::Permission, "office.no_folder", "There is no folder to install LibreOffice in.")
    })?;
    let parent = dir.parent().map(Path::to_path_buf).unwrap_or_else(|| dir.clone());
    std::fs::create_dir_all(&parent).map_err(|e| PwError::from_io(&e, "data folder"))?;
    let msi = parent.join(format!("LibreOffice_{}.msi", pkg.version));
    const WHAT: pw_net::What = pw_net::What {
        code: "office.download",
        noun: "LibreOffice",
        folder: "data folder",
        user_agent: "PageWeave Office download",
    };
    if !(msi.is_file() && pw_net::sha256_file(&msi, WHAT.folder)? == pkg.sha256) {
        pw_net::fetch(&WHAT, &pkg.url, &msi, pkg.bytes, &pkg.sha256, cancelled, &|b| progress(b, pkg.bytes))?;
    }
    progress(pkg.bytes, pkg.bytes);
    if cancelled() {
        return Err(PwError::cancelled());
    }
    // Unpack beside the final folder, then swap it in.
    let staging = parent.join("office.unpacking");
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging).map_err(|e| PwError::from_io(&e, "data folder"))?;
    let system = std::env::var_os("SystemRoot").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
    let msiexec = system.join("System32").join("msiexec.exe");
    let mut target = std::ffi::OsString::from("TARGETDIR=");
    target.push(staging.as_os_str());
    let mut child = Command::new(&msiexec)
        .arg("/a")
        .arg(&msi)
        .arg("/qn")
        .arg(target)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| PwError::from_io(&e, "Windows Installer"))?;
    let started = Instant::now();
    loop {
        if let Some(status) = child.try_wait().map_err(|e| PwError::from_io(&e, "Windows Installer"))? {
            if !status.success() {
                let _ = std::fs::remove_dir_all(&staging);
                return Err(PwError::new(
                    ErrorCategory::EngineFailure,
                    "office.unpack",
                    format!("LibreOffice could not be unpacked (Windows Installer ended with {status})."),
                )
                .with_action("Try again. If it keeps failing, install LibreOffice from libreoffice.org."));
            }
            break;
        }
        if cancelled() || started.elapsed() > Duration::from_secs(900) {
            let _ = child.kill();
            let _ = child.wait();
            let _ = std::fs::remove_dir_all(&staging);
            return Err(if cancelled() {
                PwError::cancelled()
            } else {
                PwError::new(
                    ErrorCategory::Timeout,
                    "office.unpack_timeout",
                    "Unpacking LibreOffice took too long and was stopped.",
                )
            });
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    // The unpacked program must run before it replaces anything.
    let found = program_in(&staging).into_iter().find(|p| p.is_file()).ok_or_else(|| {
        PwError::new(ErrorCategory::CorruptResource, "office.unpack", "The unpacked LibreOffice has no program.")
    })?;
    if version_of(&found).is_none() {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(PwError::new(
            ErrorCategory::CorruptResource,
            "office.unpack",
            "The unpacked LibreOffice does not start.",
        ));
    }
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::rename(&staging, &dir).map_err(|e| PwError::from_io(&e, "data folder"))?;
    let _ = std::fs::remove_file(&msi);
    find_office().ok_or_else(not_installed)
}

pub fn is_office_file(p: &Path) -> bool {
    p.extension().and_then(|e| e.to_str()).is_some_and(|e| OFFICE_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
}

/// LibreOffice settings for the private profile: macros never run, no update checks, no
/// "first start" dialogs, links in documents not updated.
const PROFILE_SETTINGS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<oor:items xmlns:oor="http://openoffice.org/2001/registry" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">
<item oor:path="/org.openoffice.Office.Common/Security/Scripting"><prop oor:name="MacroSecurityLevel" oor:op="fuse"><value>3</value></prop></item>
<item oor:path="/org.openoffice.Office.Common/Security/Scripting"><prop oor:name="DisableMacrosExecution" oor:op="fuse"><value>true</value></prop></item>
<item oor:path="/org.openoffice.Office.Common/Misc"><prop oor:name="FirstRun" oor:op="fuse"><value>false</value></prop></item>
<item oor:path="/org.openoffice.Office.Jobs/Jobs/org.openoffice.Office.Jobs:Job['UpdateCheck']/Arguments"><prop oor:name="AutoCheckEnabled" oor:op="fuse"><value>false</value></prop></item>
<item oor:path="/org.openoffice.Office.Writer/Content/Update"><prop oor:name="Link" oor:op="fuse"><value>2</value></prop></item>
<item oor:path="/org.openoffice.Office.Calc/Content/Update"><prop oor:name="Link" oor:op="fuse"><value>2</value></prop></item>
</oor:items>
"#;

fn file_url(p: &Path) -> String {
    let s = p.to_string_lossy().replace('\\', "/");
    let s = if s.starts_with('/') { s } else { format!("/{s}") };
    let mut out = String::from("file://");
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'/' | b'-' | b'_' | b'.' | b'~' | b':' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct OfficeReport {
    pub pages: u32,
    pub engine: String,
    pub millis: u64,
}

/// Convert `input` (an Office document) to a PDF at `output`.
/// The first 64 KB of a file.
fn read_head(path: &Path) -> std::io::Result<Vec<u8>> {
    use std::io::Read;
    let mut buf = Vec::with_capacity(65_536);
    std::fs::File::open(path)?.take(65_536).read_to_end(&mut buf)?;
    Ok(buf)
}

/// Whether the start of a file is what its extension says, so LibreOffice is never asked to
/// convert a renamed picture or random bytes (it would turn them into a PDF of noise and report
/// success). Office Open XML and OpenDocument are ZIP archives, older Office formats OLE
/// containers (old .doc/.xls files are sometimes RTF, HTML or XML instead), RTF starts with
/// `{\rtf`, and text formats must look like text.
fn content_matches(ext: &str, head: &[u8]) -> bool {
    const ZIP: &[u8] = b"PK\x03\x04";
    const OLE: &[u8] = &[0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];
    let markup = || {
        let t: Vec<u8> = head
            .iter()
            .skip_while(|b| matches!(b, 0xEF | 0xBB | 0xBF | b' ' | b'\t' | b'\r' | b'\n'))
            .take(1)
            .copied()
            .collect();
        t.first() == Some(&b'<')
    };
    let rtf = || head.starts_with(b"{\\rtf");
    match ext {
        "docx" | "docm" | "dotx" | "dotm" | "xlsx" | "xlsm" | "xlsb" | "xltx" | "xltm" | "pptx" | "pptm" | "ppsx"
        | "ppsm" | "potx" | "potm" | "vsdx" | "odt" | "ods" | "odp" | "odg" => head.starts_with(ZIP),
        "doc" | "dot" | "xls" | "xlt" => head.starts_with(OLE) || rtf() || markup(),
        "ppt" | "pps" | "pot" | "vsd" | "pub" => head.starts_with(OLE),
        "rtf" => rtf(),
        _ => looks_like_text(head),
    }
}

/// Text (any common encoding): few control characters other than tab and line ends. UTF-16 is
/// recognised by its byte-order mark (its NUL bytes would look binary).
fn looks_like_text(head: &[u8]) -> bool {
    if head.starts_with(&[0xFF, 0xFE]) || head.starts_with(&[0xFE, 0xFF]) {
        return true;
    }
    let control = head.iter().filter(|&&b| b < 0x20 && !matches!(b, b'\t' | b'\n' | b'\r' | 0x0C | 0x1B)).count();
    control * 100 <= head.len() * 2
}

pub fn convert_office_to_pdf(
    input: &Path,
    output: &Path,
    policy: CollisionPolicy,
    cancelled: &dyn Fn() -> bool,
    timeout: Duration,
) -> PwResult<(CommittedOutput, OfficeReport, Vec<String>)> {
    if !is_office_file(input) {
        return Err(PwError::new(
            ErrorCategory::UnsupportedFormat,
            "office.format",
            "This kind of file can't be converted. Word, Excel, PowerPoint, OpenDocument, RTF, text, CSV and HTML files can.",
        ));
    }
    let meta = std::fs::metadata(input).map_err(|e| PwError::from_io(&e, "file"))?;
    if meta.len() > MAX_INPUT_BYTES {
        return Err(PwError::invalid("office.too_large", "The file is larger than 512 MB."));
    }
    if meta.len() == 0 {
        return Err(PwError::new(ErrorCategory::UnsupportedFormat, "office.empty", "The file is empty."));
    }
    let ext = input.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    let head = read_head(input).map_err(|e| PwError::from_io(&e, "file"))?;
    if !content_matches(&ext, &head) {
        return Err(PwError::new(
            ErrorCategory::UnsupportedFormat,
            "office.mismatch",
            format!("This file is not a .{ext} document: its content does not match its name."),
        )
        .with_action("Check that it is the right file; a renamed picture or download error can look like this."));
    }
    let engine_info = find_office().ok_or_else(not_installed)?;
    let started = Instant::now();
    let work = tempfile::Builder::new()
        .prefix("pageweave-office-")
        .tempdir()
        .map_err(|e| PwError::from_io(&e, "temporary folder"))?;
    let profile = work.path().join("profile");
    std::fs::create_dir_all(profile.join("user")).map_err(|e| PwError::from_io(&e, "temporary folder"))?;
    std::fs::write(profile.join("user").join("registrymodifications.xcu"), PROFILE_SETTINGS)
        .map_err(|e| PwError::from_io(&e, "temporary folder"))?;
    // A copy with a plain name: LibreOffice never sees (or locks) the original.
    let ext = input.extension().and_then(|e| e.to_str()).unwrap_or("txt").to_ascii_lowercase();
    let copy = work.path().join(format!("document.{ext}"));
    std::fs::copy(input, &copy).map_err(|e| PwError::from_io(&e, "file"))?;
    let out_dir = work.path().join("out");
    std::fs::create_dir_all(&out_dir).map_err(|e| PwError::from_io(&e, "temporary folder"))?;
    let mut child = Command::new(&engine_info.program)
        .arg(format!("-env:UserInstallation={}", file_url(&profile)))
        .args([
            "--headless",
            "--invisible",
            "--norestore",
            "--nologo",
            "--nolockcheck",
            "--nodefault",
            "--convert-to",
            "pdf",
            "--outdir",
        ])
        .arg(&out_dir)
        .arg(&copy)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| {
            PwError::new(
                ErrorCategory::EngineFailure,
                "office.start",
                format!("LibreOffice could not be started ({}).", e.kind()),
            )
        })?;
    let status = loop {
        if cancelled() {
            let _ = child.kill();
            let _ = child.wait();
            return Err(PwError::cancelled());
        }
        if started.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            return Err(PwError::new(
                ErrorCategory::Timeout,
                "office.timeout",
                "Converting the file took too long and was stopped.",
            ));
        }
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) => std::thread::sleep(Duration::from_millis(100)),
            Err(e) => return Err(PwError::from_io(&e, "conversion")),
        }
    };
    let produced = out_dir.join("document.pdf");
    if !status.success() || !produced.is_file() {
        return Err(PwError::new(
            ErrorCategory::Corrupt,
            "office.failed",
            "LibreOffice could not convert this file. It may be damaged, password-protected or not what its name says.",
        ));
    }
    let bytes = std::fs::read(&produced).map_err(|e| PwError::from_io(&e, "converted file"))?;
    let pdfium = engine()?;
    let pages = {
        let d = pdfium.load_pdf_from_byte_slice(&bytes, None).map_err(|_| {
            PwError::new(
                ErrorCategory::ValidationFailed,
                "output.verify",
                "The converted PDF does not open and was not saved.",
            )
        })?;
        d.pages().len().max(0) as u32
    };
    if pages == 0 {
        return Err(PwError::new(
            ErrorCategory::ValidationFailed,
            "output.verify",
            "The converted PDF has no pages and was not saved.",
        ));
    }
    let c = write_atomic_bytes(output, policy, &[input.to_path_buf()], &bytes, |staged| {
        let back = std::fs::read(staged).map_err(|e| PwError::from_io(&e, "output file"))?;
        if back != bytes {
            return Err(PwError::new(
                ErrorCategory::ValidationFailed,
                "output.verify",
                "The written PDF differs and was not kept.",
            ));
        }
        // Reopened from disk with the normal loader, as the app will.
        let p = engine()?;
        load(&p, staged, &OpenOptions { password: None }).map(|_| ())
    })?;
    let report =
        OfficeReport { pages, engine: engine_info.version.clone(), millis: started.elapsed().as_millis() as u64 };
    let warnings = vec![format!(
        "Converted with {} installed on this computer; layout can differ from Microsoft Office where fonts differ.",
        engine_info.version
    )];
    Ok((c, report, warnings))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn office_files_by_extension() {
        assert!(is_office_file(Path::new("a/Report.DOCX")));
        assert!(is_office_file(Path::new("x.odp")));
        assert!(!is_office_file(Path::new("x.pdf")));
        assert!(!is_office_file(Path::new("x")));
        assert!(!is_office_file(Path::new("x.exe")));
    }

    #[test]
    fn content_must_match_the_name() {
        let zip = b"PK\x03\x04rest".as_slice();
        let ole = [0xD0u8, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1, 0, 0];
        let png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".as_slice();
        assert!(content_matches("docx", zip) && content_matches("odp", zip));
        assert!(!content_matches("docx", png) && !content_matches("xlsx", &ole));
        assert!(content_matches("doc", &ole) && content_matches("ppt", &ole));
        // Old .doc/.xls files are sometimes RTF, HTML or XML.
        assert!(content_matches("doc", b"{\\rtf1\\ansi") && content_matches("xls", b"\xEF\xBB\xBF<?xml version"));
        assert!(!content_matches("ppt", b"{\\rtf1"));
        assert!(content_matches("rtf", b"{\\rtf1\\ansi") && !content_matches("rtf", b"hello"));
        // Text: plain, Latin-1, UTF-16 with its mark; binary is not.
        assert!(content_matches("csv", b"a,b\r\n1,2\r\n") && content_matches("txt", b"caf\xe9\n"));
        assert!(content_matches("txt", &[0xFF, 0xFE, b'a', 0, b'b', 0]));
        assert!(!content_matches("txt", &[0u8, 1, 2, 3, 4, 5, 6, 7, 8, 0, 0, 0]) && !content_matches("html", png));
    }

    #[test]
    fn empty_and_mislabelled_files_are_refused_before_libreoffice_runs() {
        let dir = tempfile::tempdir().expect("tmp");
        let refuse = |name: &str, bytes: &[u8]| {
            let input = dir.path().join(name);
            std::fs::write(&input, bytes).expect("write");
            let out = dir.path().join(format!("{name}.pdf"));
            let e = convert_office_to_pdf(&input, &out, CollisionPolicy::Fail, &|| false, Duration::from_secs(5))
                .expect_err("refused");
            assert!(!out.exists());
            e.code
        };
        assert_eq!(refuse("empty.docx", b""), "office.empty");
        assert_eq!(refuse("picture.docx", b"\x89PNG\r\n\x1a\n"), "office.mismatch");
        assert_eq!(refuse("noise.rtf", &[0u8, 7, 200, 13, 0, 0, 9]), "office.mismatch");
    }

    #[test]
    fn profile_urls_are_escaped() {
        assert_eq!(file_url(Path::new("/tmp/a b/é")), "file:///tmp/a%20b/%C3%A9");
        assert_eq!(file_url(Path::new("C:\\Users\\x")), "file:///C:/Users/x");
    }
}
