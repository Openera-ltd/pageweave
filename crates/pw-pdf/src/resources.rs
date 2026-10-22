//! Installed resources (PRD 18 "resource audit"): whether each local capability's files are
//! present and match the pinned checksums. Read-only: nothing is loaded, downloaded or changed.
//! Capability names are user-facing; file names are diagnostics only.

use crate::library_search_dirs;
use pdfium_render::prelude::Pdfium;
use pw_core::{PwError, PwResult};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::path::Path;

const PDFIUM_LOCK: &str = include_str!("../../../resources/pdfium.lock.json");

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceState {
    Ready,
    Missing,
    Damaged,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ResourceFile {
    pub name: String,
    pub state: ResourceState,
    pub bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Capability {
    /// `pdf_engine` or `text_recognition`.
    pub id: String,
    pub title: String,
    pub state: ResourceState,
    pub detail: String,
    /// Bytes on disk of the files found.
    pub bytes: u64,
    /// Folder the files were found in (diagnostics).
    pub location: Option<String>,
    /// File by file (diagnostics; not for normal UI).
    pub files: Vec<ResourceFile>,
    /// Needed for PageWeave to be complete. Optional ones (installed separately, such as the
    /// Office layout engine) do not make the report "not ready".
    pub required: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ResourceReport {
    pub capabilities: Vec<Capability>,
    /// Everything required is present and intact.
    pub all_ready: bool,
}

fn sha256(p: &Path) -> PwResult<String> {
    let bytes = std::fs::read(p).map_err(|e| PwError::from_io(&e, "resource file"))?;
    Ok(Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect())
}

fn summarize(
    id: &str,
    title: &str,
    files: Vec<ResourceFile>,
    location: Option<String>,
    missing_detail: &str,
) -> Capability {
    let bytes = files.iter().map(|f| f.bytes).sum();
    let missing = files.iter().filter(|f| f.state == ResourceState::Missing).count();
    let damaged = files.iter().filter(|f| f.state == ResourceState::Damaged).count();
    let (state, detail) = if files.is_empty() || missing == files.len() {
        (ResourceState::Missing, missing_detail.to_string())
    } else if damaged > 0 {
        (ResourceState::Damaged, format!("{damaged} file(s) don't match the versions this PageWeave was built with. Reinstall PageWeave to repair."))
    } else if missing > 0 {
        (ResourceState::Missing, format!("{missing} file(s) are missing. Reinstall PageWeave to repair."))
    } else {
        (ResourceState::Ready, "Installed and verified.".to_string())
    };
    Capability { id: id.into(), title: title.into(), state, detail, bytes, location, files, required: true }
}

fn pdf_engine() -> PwResult<Capability> {
    let plat = if cfg!(windows) { "win-x64" } else { "linux-x64" };
    let lock: serde_json::Value = serde_json::from_str(PDFIUM_LOCK)
        .map_err(|_| PwError::internal("pdfium.lock", "The PDF engine resource lock is unreadable."))?;
    let pinned = lock["artifacts"][plat]["library_sha256"].as_str().map(str::to_string);
    // The same search the engine itself uses, first match wins.
    let found = library_search_dirs(None)
        .into_iter()
        .map(|d| Pdfium::pdfium_platform_library_name_at_path(&d))
        .find(|p| p.is_file());
    let files = match &found {
        Some(p) => {
            let bytes = std::fs::metadata(p).map(|m| m.len()).unwrap_or(0);
            let ok = match &pinned {
                Some(sha) => sha256(p)? == *sha,
                None => true,
            };
            vec![ResourceFile {
                name: p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
                state: if ok { ResourceState::Ready } else { ResourceState::Damaged },
                bytes,
            }]
        }
        None => vec![],
    };
    Ok(summarize(
        "pdf_engine",
        "PDF engine",
        files,
        found.and_then(|p| p.parent().map(|d| d.display().to_string())),
        "The PDF engine is missing, so documents can't be opened. Reinstall PageWeave.",
    ))
}

fn text_recognition() -> PwResult<Capability> {
    let Some(dir) = pw_ocr::default_resource_dir() else {
        return Ok(summarize(
            "text_recognition",
            "Text recognition (OCR)",
            vec![],
            None,
            "Text recognition is not installed. Reinstall PageWeave to add it.",
        ));
    };
    let files = pw_ocr::check_files(&dir)?
        .into_iter()
        .map(|f| ResourceFile {
            name: f.name,
            state: match f.matches {
                None => ResourceState::Missing,
                Some(true) => ResourceState::Ready,
                Some(false) => ResourceState::Damaged,
            },
            bytes: f.bytes,
        })
        .collect();
    Ok(summarize(
        "text_recognition",
        "Text recognition (OCR)",
        files,
        Some(dir.display().to_string()),
        "Text recognition is not installed. Reinstall PageWeave to add it.",
    ))
}

/// Opening Office documents: LibreOffice installed on this computer (not bundled, D-019).
fn office_documents() -> Capability {
    match crate::office::find_office() {
        Some(e) => Capability {
            id: "office_documents".into(),
            title: "Office documents (optional)".into(),
            state: ResourceState::Ready,
            detail: format!("Word, Excel and PowerPoint files open through {} installed on this computer.", e.version),
            bytes: 0,
            location: e.program.parent().map(|d| d.display().to_string()),
            files: vec![],
            required: false,
        },
        None => Capability {
            id: "office_documents".into(),
            title: "Office documents (optional)".into(),
            state: ResourceState::Missing,
            detail: "To open Word, Excel and PowerPoint files, install LibreOffice (free). Everything else works without it.".into(),
            bytes: 0,
            location: None,
            files: vec![],
            required: false,
        },
    }
}

/// Check every installed resource against its pinned checksum.
pub fn resource_status() -> PwResult<ResourceReport> {
    let capabilities = vec![pdf_engine()?, text_recognition()?, office_documents()];
    let all_ready = capabilities.iter().filter(|c| c.required).all(|c| c.state == ResourceState::Ready);
    Ok(ResourceReport { capabilities, all_ready })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(state: ResourceState) -> ResourceFile {
        ResourceFile { name: "x".into(), state, bytes: 10 }
    }

    #[test]
    fn states_are_summarised_worst_first() {
        let c = summarize("a", "A", vec![file(ResourceState::Ready), file(ResourceState::Damaged)], None, "none");
        assert_eq!(c.state, ResourceState::Damaged);
        assert_eq!(c.bytes, 20);
        let c = summarize("a", "A", vec![file(ResourceState::Ready), file(ResourceState::Missing)], None, "none");
        assert_eq!(c.state, ResourceState::Missing);
        assert!(c.detail.contains("1 file(s) are missing"));
        let c = summarize("a", "A", vec![], None, "not installed");
        assert_eq!((c.state, c.detail.as_str()), (ResourceState::Missing, "not installed"));
        let c = summarize("a", "A", vec![file(ResourceState::Ready)], None, "none");
        assert_eq!(c.state, ResourceState::Ready);
    }

    #[test]
    fn this_build_finds_its_resources_intact() {
        // The development checkout (debug build) has both installed by the fetch scripts.
        let r = resource_status().expect("status");
        let pdf = &r.capabilities[0];
        assert_eq!(pdf.state, ResourceState::Ready, "{pdf:?}");
        if pw_ocr::default_resource_dir().is_some() {
            assert_eq!(r.capabilities[1].state, ResourceState::Ready, "{:?}", r.capabilities[1]);
            assert_eq!(r.capabilities[1].files.len(), 4);
        }
    }
}
