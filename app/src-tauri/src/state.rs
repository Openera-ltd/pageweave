//! Application state shared by commands and the page protocol.

use pw_core::job::JobManager;
use pw_core::journal::{Journal, RecoveredJob};
use pw_core::log::{Level, LogConfig, Logger};
use pw_core::settings::{LoadOutcome, Settings, SettingsStore};
use pw_core::{fields, PwError, PwResult};
use pw_pdf::DocumentInfo;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

pub type DocId = u64;

/// A document the user has open. Holds the path and (in memory only) the password.
#[derive(Clone)]
pub struct DocEntry {
    pub path: PathBuf,
    pub password: Option<String>,
    pub info: DocumentInfo,
    /// Opened only to preview pages (e.g. for insertion); never shown as a tab.
    pub preview: bool,
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
    /// Durable record of running file-writing jobs; None if its folder could not be used (logged).
    pub journal: Option<Arc<Journal>>,
}

struct Inner {
    launch_files: Mutex<Vec<String>>,
    docs: Mutex<HashMap<DocId, DocEntry>>,
    next_id: Mutex<DocId>,
    settings: Mutex<Settings>,
    store: SettingsStore,
    /// Number of unsaved edits (pending annotations) the UI reports; closing the window is held
    /// back while it is non-zero so work is not lost silently.
    unsaved: AtomicU32,
    /// Startup milestones already recorded (each is logged once per run).
    marks: Mutex<HashSet<String>>,
    /// Jobs found interrupted at startup, handed to the UI once.
    recovered: Mutex<Vec<RecoveredJob>>,
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
        let (journal, recovered) = match Journal::open(&data_dir.join("jobs")) {
            Ok(j) => {
                let rec = j.recover();
                for r in &rec {
                    logger.log(
                        Level::Warn,
                        "job",
                        "recovered_interrupted",
                        None,
                        fields! {
                            "operation" => r.operation.id(),
                            "removed_partial_files" => r.removed_partial_files,
                            "leftover_partial_files" => r.leftover_partial_files.len(),
                        },
                    );
                }
                (Some(j), rec)
            }
            Err(e) => {
                logger.log(Level::Error, "job", "journal_unavailable", None, fields! {"error_code" => e.code});
                (None, Vec::new())
            }
        };
        // Bounded pool: leave headroom for the UI and rendering threads.
        let workers = std::thread::available_parallelism().map(|n| n.get().saturating_sub(1)).unwrap_or(2).clamp(1, 4);
        AppState {
            inner: Arc::new(Inner {
                launch_files: Mutex::new(Vec::new()),
                docs: Mutex::new(HashMap::new()),
                next_id: Mutex::new(1),
                settings: Mutex::new(settings),
                store,
                unsaved: AtomicU32::new(0),
                marks: Mutex::new(HashSet::new()),
                recovered: Mutex::new(recovered),
            }),
            jobs: Arc::new(JobManager::new(workers)),
            logger: Arc::new(logger),
            engine_dir,
            settings_outcome: outcome,
            log_dir,
            journal,
        }
    }

    pub fn set_unsaved(&self, count: u32) {
        self.inner.unsaved.store(count, Ordering::SeqCst);
    }

    /// Interrupted jobs found at startup (returned once).
    pub fn take_recovered(&self) -> Vec<RecoveredJob> {
        std::mem::take(&mut *self.inner.recovered.lock().unwrap_or_else(|p| p.into_inner()))
    }

    /// Record a startup milestone once. Returns false if it was already recorded.
    pub fn first_mark(&self, name: &str) -> bool {
        self.inner.marks.lock().unwrap_or_else(|p| p.into_inner()).insert(name.to_string())
    }

    pub fn unsaved(&self) -> u32 {
        self.inner.unsaved.load(Ordering::SeqCst)
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
        self.open_with(path, password, true)
    }

    /// Like `open`; `remember = false` keeps the file out of the recent list (e.g. a file opened
    /// only to preview pages for insertion).
    pub fn open_with(&self, path: PathBuf, password: Option<String>, remember: bool) -> PwResult<OpenedDoc> {
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
            .insert(id, DocEntry { path: canonical.clone(), password, info: info.clone(), preview: !remember });
        if remember {
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

    /// Documents shown as tabs, in the order they were opened (lets the UI recover after a reload).
    pub fn open_documents(&self) -> Vec<OpenedDoc> {
        let docs = self.inner.docs.lock().unwrap_or_else(|p| p.into_inner());
        let mut list: Vec<OpenedDoc> = docs
            .iter()
            .filter(|(_, d)| !d.preview)
            .map(|(id, d)| OpenedDoc {
                doc_id: *id,
                path: d.path.clone(),
                name: display_name(&d.path),
                info: d.info.clone(),
                already_open: true,
            })
            .collect();
        list.sort_by_key(|d| d.doc_id);
        list
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
    fn unsaved_count_guards_window_close() {
        let (s, _d) = state();
        assert!(!crate::close_needs_confirmation(&s));
        s.set_unsaved(3);
        assert!(crate::close_needs_confirmation(&s));
        assert!(crate::close_needs_confirmation(&s.clone()), "clones share the count");
        s.set_unsaved(0);
        assert!(!crate::close_needs_confirmation(&s));
    }

    #[test]
    fn interrupted_jobs_are_recovered_at_startup_and_reported_once() {
        let dir = tempfile::tempdir().expect("tmp");
        let out = dir.path().join("out");
        std::fs::create_dir_all(&out).expect("mkdir");
        let dead_pid = u32::MAX - 3;
        let staged = out.join(format!("{}zz.tmp", pw_core::output::staged_prefix(dead_pid)));
        std::fs::write(&staged, b"partial").expect("w");
        let jobs = dir.path().join("data/jobs");
        std::fs::create_dir_all(&jobs).expect("mkdir");
        let entry = serde_json::json!({
            "job_id": "j", "label": "extract_pages", "pid": dead_pid, "started_unix_ms": 1,
            "operation": {"op": "extract_pages", "input": "in.pdf", "pages": "1", "output": out.join("o.pdf")},
            "collision": "fail"
        });
        std::fs::write(jobs.join("job-j.json"), entry.to_string()).expect("w");
        let s = AppState::new(&dir.path().join("cfg"), &dir.path().join("data"), None);
        let rec = s.take_recovered();
        assert_eq!(rec.len(), 1);
        assert_eq!(rec[0].removed_partial_files, 1);
        assert!(!staged.exists());
        assert!(s.take_recovered().is_empty());
        let log = std::fs::read_to_string(dir.path().join("data/logs/pageweave.jsonl")).expect("log");
        assert!(log.contains("recovered_interrupted") && !log.contains("o.pdf"), "logged without paths");
    }

    #[test]
    fn startup_marks_are_recorded_once() {
        let (s, _d) = state();
        assert!(s.first_mark("first_page"));
        assert!(!s.first_mark("first_page"));
        assert!(s.first_mark("ui_ready"));
    }

    #[test]
    fn webview2_args_follow_the_standard_variable_and_keep_defaults() {
        assert_eq!(crate::webview2_browser_args(None), None);
        assert_eq!(crate::webview2_browser_args(Some("  ")), None);
        let a = crate::webview2_browser_args(Some("--remote-debugging-port=9222")).expect("args");
        assert!(a.starts_with("--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection"));
        assert!(a.ends_with(" --remote-debugging-port=9222"));
    }

    #[test]
    fn open_documents_lists_tabs_in_order_without_previews() {
        let (s, _d) = state();
        let a = s.open(fixture("five-pages.pdf"), None).expect("open");
        s.open_with(fixture("single-page.pdf"), None, false).expect("preview");
        let b = s.open(fixture("twenty-pages.pdf"), None).expect("open");
        let ids: Vec<_> = s.open_documents().iter().map(|d| d.doc_id).collect();
        assert_eq!(ids, vec![a.doc_id, b.doc_id]);
        s.close(a.doc_id);
        assert_eq!(s.open_documents().len(), 1);
    }

    #[test]
    fn preview_opens_stay_out_of_recent_files() {
        let (s, _d) = state();
        s.open_with(fixture("single-page.pdf"), None, false).expect("open");
        assert!(s.settings().recent_files.is_empty());
        s.open(fixture("five-pages.pdf"), None).expect("open");
        assert_eq!(s.settings().recent_files.len(), 1);
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
