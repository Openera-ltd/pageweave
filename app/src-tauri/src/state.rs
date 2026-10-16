//! Application state shared by commands and the page protocol.

use pw_core::job::JobManager;
use pw_core::log::{Level, LogConfig, Logger};
use pw_core::settings::{LoadOutcome, Settings, SettingsStore};
use pw_core::{fields, PwError, PwResult};
use pw_pdf::DocumentInfo;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

pub type DocId = u64;

/// A document the user has open. Holds the path and (in memory only) the password.
#[derive(Clone)]
pub struct DocEntry {
    pub path: PathBuf,
    pub password: Option<String>,
    pub info: DocumentInfo,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenedDoc {
    pub doc_id: DocId,
    pub path: PathBuf,
    pub name: String,
    pub info: DocumentInfo,
    /// True when the file was already open; the UI should switch to the existing tab.
    pub already_open: bool,
}

pub const MAX_OPEN_DOCUMENTS: usize = 64;

#[derive(Clone)]
pub struct AppState {
    inner: Arc<Inner>,
    pub jobs: Arc<JobManager>,
    pub logger: Arc<Logger>,
    pub engine_dir: Option<PathBuf>,
    pub settings_outcome: LoadOutcome,
    pub log_dir: PathBuf,
}

struct Inner {
    launch_files: Mutex<Vec<String>>,
    docs: Mutex<HashMap<DocId, DocEntry>>,
    next_id: Mutex<DocId>,
    settings: Mutex<Settings>,
    store: SettingsStore,
}

impl AppState {
    pub fn new(config_dir: &Path, data_dir: &Path, engine_dir: Option<PathBuf>) -> Self {
        let store = SettingsStore::new(config_dir.join("settings.json"));
        let (settings, outcome) = store.load();
        let log_dir = data_dir.join("logs");
        let logger = Logger::new(LogConfig {
            dir: log_dir.clone(),
            file_stem: "pageweave".into(),
            max_bytes: 2 * 1024 * 1024,
            max_files: 5,
            min_level: Level::Info,
            build: env!("CARGO_PKG_VERSION").into(),
        });
        // Bounded pool: leave headroom for the UI and rendering threads.
        let workers = std::thread::available_parallelism().map(|n| n.get().saturating_sub(1)).unwrap_or(2).clamp(1, 4);
        AppState {
            inner: Arc::new(Inner {
                launch_files: Mutex::new(Vec::new()),
                docs: Mutex::new(HashMap::new()),
                next_id: Mutex::new(1),
                settings: Mutex::new(settings),
                store,
            }),
            jobs: Arc::new(JobManager::new(workers)),
            logger: Arc::new(logger),
            engine_dir,
            settings_outcome: outcome,
            log_dir,
        }
    }

    pub fn settings(&self) -> Settings {
        self.inner.settings.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }

    pub fn save_settings(&self, s: Settings) -> PwResult<Settings> {
        self.inner.store.save(&s)?;
        *self.inner.settings.lock().unwrap_or_else(|p| p.into_inner()) = s.clone();
        Ok(s)
    }

    pub fn doc(&self, id: DocId) -> PwResult<DocEntry> {
        self.inner
            .docs
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(&id)
            .cloned()
            .ok_or_else(|| PwError::invalid("doc.not_open", "That document is no longer open."))
    }

    /// Open (or re-focus) a document. Runs on a worker thread.
    pub fn open(&self, path: PathBuf, password: Option<String>) -> PwResult<OpenedDoc> {
        pw_pdf::init(self.engine_dir.as_deref())?;
        let canonical = path.canonicalize().map_err(|e| PwError::from_io(&e, "file"))?;
        {
            let docs = self.inner.docs.lock().unwrap_or_else(|p| p.into_inner());
            if let Some((id, d)) = docs.iter().find(|(_, d)| d.path == canonical) {
                return Ok(OpenedDoc {
                    doc_id: *id,
                    path: d.path.clone(),
                    name: display_name(&d.path),
                    info: d.info.clone(),
                    already_open: true,
                });
            }
            if docs.len() >= MAX_OPEN_DOCUMENTS {
                return Err(PwError::invalid(
                    "doc.too_many",
                    format!("Close a tab first; at most {MAX_OPEN_DOCUMENTS} documents can be open."),
                ));
            }
        }
        let started = std::time::Instant::now();
        let info = pw_pdf::open_document(&canonical, &pw_pdf::OpenOptions { password: password.as_deref() });
        self.logger.log(
            Level::Info,
            "doc",
            "open",
            None,
            fields! {
                "file" => pw_core::log::redact_path(&canonical),
                "ok" => info.is_ok(),
                "error_code" => info.as_ref().err().map(|e| e.code.clone()),
                "pages" => info.as_ref().ok().map(|i| i.page_count),
                "ms" => started.elapsed().as_millis() as u64,
            },
        );
        let info = info?;
        let id = {
            let mut next = self.inner.next_id.lock().unwrap_or_else(|p| p.into_inner());
            let id = *next;
            *next += 1;
            id
        };
        self.inner
            .docs
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(id, DocEntry { path: canonical.clone(), password, info: info.clone() });
        {
            let mut s = self.inner.settings.lock().unwrap_or_else(|p| p.into_inner());
            s.touch_recent(&canonical.to_string_lossy());
            let snapshot = s.clone();
            drop(s);
            // Recent-files persistence failure must not fail the open; it is logged instead.
            if let Err(e) = self.inner.store.save(&snapshot) {
                self.logger.log(Level::Warn, "settings", "recent.save_failed", None, fields! {"code" => e.code});
            }
        }
        Ok(OpenedDoc { doc_id: id, name: display_name(&canonical), path: canonical, info, already_open: false })
    }

    pub fn set_launch_files(&self, files: Vec<String>) {
        *self.inner.launch_files.lock().unwrap_or_else(|p| p.into_inner()) = files;
    }

    /// Returns command-line files once, so a UI reload does not reopen them.
    pub fn take_launch_files(&self) -> Vec<String> {
        std::mem::take(&mut *self.inner.launch_files.lock().unwrap_or_else(|p| p.into_inner()))
    }

    pub fn close(&self, id: DocId) -> bool {
        self.inner.docs.lock().unwrap_or_else(|p| p.into_inner()).remove(&id).is_some()
    }
}

/// File arguments from the command line: options (leading `-`) are ignored; at most 32 files.
pub fn launch_args(args: impl Iterator<Item = std::ffi::OsString>) -> Vec<String> {
    args.map(PathBuf::from)
        .filter(|p| !p.to_string_lossy().starts_with('-') && p.is_file())
        .take(32)
        .map(|p| p.to_string_lossy().into_owned())
        .collect()
}

fn display_name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "Document".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/pdf").join(name)
    }

    fn state() -> (AppState, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("tmp");
        (AppState::new(&dir.path().join("cfg"), &dir.path().join("data"), None), dir)
    }

    #[test]
    fn open_twice_returns_existing_tab() {
        let (s, _d) = state();
        let a = s.open(fixture("five-pages.pdf"), None).expect("open");
        assert!(!a.already_open);
        let b = s.open(fixture("five-pages.pdf"), None).expect("reopen");
        assert!(b.already_open);
        assert_eq!(a.doc_id, b.doc_id);
        assert_eq!(s.settings().recent_files.len(), 1);
    }

    #[test]
    fn launch_args_keep_existing_files_only_and_are_taken_once() {
        let (s, _d) = state();
        let f = fixture("single-page.pdf");
        let args = vec!["--flag".into(), f.clone().into_os_string(), "/no/such/file.pdf".into()];
        let files = launch_args(args.into_iter());
        assert_eq!(files, vec![f.to_string_lossy().into_owned()]);
        s.set_launch_files(files);
        assert_eq!(s.take_launch_files().len(), 1);
        assert!(s.take_launch_files().is_empty());
    }

    #[test]
    fn close_and_lookup() {
        let (s, _d) = state();
        let a = s.open(fixture("single-page.pdf"), None).expect("open");
        assert!(s.doc(a.doc_id).is_ok());
        assert!(s.close(a.doc_id));
        assert!(!s.close(a.doc_id));
        assert_eq!(s.doc(a.doc_id).err().map(|e| e.code), Some("doc.not_open".into()));
    }

    #[test]
    fn open_failures_are_logged_without_paths() {
        let (s, d) = state();
        let e = s.open(fixture("not-a-pdf.pdf"), None).expect_err("bad");
        assert_eq!(e.code, "pdf.not_pdf");
        let log = std::fs::read_to_string(d.path().join("data/logs/pageweave.jsonl")).expect("log");
        assert!(log.contains("pdf.not_pdf"));
        assert!(!log.contains("not-a-pdf"), "file names must not be logged: {log}");
    }

    #[test]
    fn password_is_kept_for_the_session_only() {
        let (s, d) = state();
        let a = s.open(fixture("encrypted-rc4-user-password.pdf"), Some("pw-user-test".into())).expect("open");
        assert_eq!(s.doc(a.doc_id).expect("doc").password.as_deref(), Some("pw-user-test"));
        let settings = std::fs::read_to_string(d.path().join("cfg/settings.json")).expect("settings");
        assert!(!settings.contains("pw-user-test"));
    }
}
