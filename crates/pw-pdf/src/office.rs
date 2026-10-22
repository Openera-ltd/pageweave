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
pub const OFFICE_EXTENSIONS: &[&str] =
    &["docx", "doc", "odt", "rtf", "txt", "xlsx", "xls", "ods", "csv", "pptx", "ppt", "odp", "html", "htm"];
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

fn candidates() -> Vec<PathBuf> {
    let mut v = Vec::new();
    // An explicit setting is the only place looked at (empty or missing: treated as not installed).
    if let Some(p) = std::env::var_os(SOFFICE_ENV) {
        v.push(PathBuf::from(p));
        return v;
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
    for p in candidates() {
        if !p.is_file() {
            continue;
        }
        let out = Command::new(&p).arg("--version").stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
        let text = String::from_utf8_lossy(&out.stdout);
        if out.status.success() && text.contains("LibreOffice") {
            let version = text.split_whitespace().take(2).collect::<Vec<_>>().join(" ");
            return Some(OfficeEngine { program: p, version });
        }
    }
    None
}

fn not_installed() -> PwError {
    PwError::new(
        ErrorCategory::MissingResource,
        "office.not_installed",
        "Opening Word, Excel and PowerPoint files needs LibreOffice, which is not installed on this computer.",
    )
    .with_action("Install LibreOffice (free, from libreoffice.org), then try again.")
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
    fn profile_urls_are_escaped() {
        assert_eq!(file_url(Path::new("/tmp/a b/é")), "file:///tmp/a%20b/%C3%A9");
        assert_eq!(file_url(Path::new("C:\\Users\\x")), "file:///C:/Users/x");
    }
}
