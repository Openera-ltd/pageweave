//! Durable job journal for restart recovery (PRD 04 "durable checkpoints and safe restart recovery",
//! PRD 05 "recovery journal").
//!
//! While a file-writing job runs, an entry describing it is kept on disk; it is removed when the job
//! reaches any terminal state (a drop guard also covers panics). An entry still present at startup
//! means the process that ran it ended abruptly. Recovery then removes that process's staged
//! (uncommitted) output files from the job's output folder and reports the job so the user can run
//! it again. Committed outputs are never touched.
//!
//! Each running instance holds an exclusive lock on `instance-<pid>.lock`, so recovery skips entries
//! whose process is still alive (two app windows sharing one data folder).
//!
//! Entries contain the operation (file paths, page ranges, annotation text) but never passwords,
//! which are not serialized. They live in the app's private data folder.

use crate::error::{PwError, PwResult};
use crate::ops::{Operation, OperationRequest};
use crate::output::{staged_prefix, write_atomic_bytes, CollisionPolicy};
use serde::{Deserialize, Serialize};
use std::fs::{self, File, TryLockError};
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JournalEntry {
    pub job_id: String,
    pub label: String,
    pub pid: u32,
    pub started_unix_ms: u64,
    pub operation: Operation,
    pub collision: CollisionPolicy,
}

/// A job found interrupted at startup.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecoveredJob {
    pub label: String,
    pub operation: Operation,
    pub collision: CollisionPolicy,
    /// Staged (uncommitted) files of the interrupted job that were removed.
    pub removed_partial_files: u32,
    /// Staged files that could not be removed (e.g. permission); left in place and reported.
    pub leftover_partial_files: Vec<PathBuf>,
}

pub struct Journal {
    dir: PathBuf,
    /// Held for the life of the journal: marks this process as alive to other instances.
    _instance_lock: Option<File>,
}

impl Journal {
    /// Open (creating) the journal folder and take this process's instance lock.
    pub fn open(dir: &Path) -> PwResult<Arc<Journal>> {
        fs::create_dir_all(dir).map_err(|e| PwError::from_io(&e, "job journal folder"))?;
        let lock_path = dir.join(format!("instance-{}.lock", std::process::id()));
        let lock = File::create(&lock_path).map_err(|e| PwError::from_io(&e, "job journal lock"))?;
        let held = match lock.try_lock() {
            Ok(()) => Some(lock),
            // Locking unsupported on this filesystem: recovery then treats every entry as possibly
            // live and leaves files alone (see `recover`).
            Err(_) => None,
        };
        Ok(Arc::new(Journal { dir: dir.to_path_buf(), _instance_lock: held }))
    }

    fn entry_path(&self, job_id: &str) -> PathBuf {
        self.dir.join(format!("job-{job_id}.json"))
    }

    /// Record that a job started. The returned guard removes the entry when dropped.
    pub fn begin(self: &Arc<Self>, job_id: &str, label: &str, request: &OperationRequest) -> PwResult<JournalGuard> {
        let entry = JournalEntry {
            job_id: job_id.to_string(),
            label: label.to_string(),
            pid: std::process::id(),
            started_unix_ms: now_ms(),
            operation: request.operation.clone(),
            collision: request.collision,
        };
        let bytes = serde_json::to_vec_pretty(&entry)
            .map_err(|_| PwError::internal("journal.encode", "Could not record the task."))?;
        write_atomic_bytes(&self.entry_path(job_id), CollisionPolicy::Replace, &[], &bytes, |_| Ok(()))?;
        Ok(JournalGuard { journal: self.clone(), job_id: job_id.to_string() })
    }

    fn finish(&self, job_id: &str) {
        let _ = fs::remove_file(self.entry_path(job_id));
    }

    /// Find jobs interrupted by an earlier process that has ended, clean their staged files and
    /// remove their entries. Corrupt entries are moved aside (`.corrupt`) rather than deleted.
    pub fn recover(&self) -> Vec<RecoveredJob> {
        let mut out = Vec::new();
        let Ok(rd) = fs::read_dir(&self.dir) else { return out };
        let me = std::process::id();
        for e in rd.flatten() {
            let path = e.path();
            let name = e.file_name().to_string_lossy().into_owned();
            if !(name.starts_with("job-") && name.ends_with(".json")) {
                continue;
            }
            let entry: JournalEntry = match fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok()) {
                Some(v) => v,
                None => {
                    let _ = fs::rename(&path, path.with_extension("json.corrupt"));
                    continue;
                }
            };
            if entry.pid == me || !self.process_ended(entry.pid) {
                continue;
            }
            let (removed, leftover) = match entry.operation.output_dir() {
                Some(dir) => remove_staged(&dir, entry.pid),
                None => (0, Vec::new()),
            };
            let _ = fs::remove_file(&path);
            out.push(RecoveredJob {
                label: entry.label,
                operation: entry.operation,
                collision: entry.collision,
                removed_partial_files: removed,
                leftover_partial_files: leftover,
            });
        }
        // Lock files of ended instances are no longer needed.
        if let Ok(rd) = fs::read_dir(&self.dir) {
            for e in rd.flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                if let Some(pid) = name
                    .strip_prefix("instance-")
                    .and_then(|r| r.strip_suffix(".lock"))
                    .and_then(|p| p.parse::<u32>().ok())
                {
                    if pid != me && self.process_ended(pid) {
                        let _ = fs::remove_file(e.path());
                    }
                }
            }
        }
        out
    }

    /// A process has ended when its instance lock can be taken (or never existed). If locking does
    /// not work here, assume it may be alive: never risk deleting a live job's files.
    fn process_ended(&self, pid: u32) -> bool {
        if self._instance_lock.is_none() {
            return false;
        }
        let p = self.dir.join(format!("instance-{pid}.lock"));
        let Ok(f) = File::options().read(true).write(true).open(&p) else { return true };
        match f.try_lock() {
            Ok(()) => true,
            Err(TryLockError::WouldBlock) => false,
            Err(TryLockError::Error(_)) => false,
        }
    }
}

/// Removes the journal entry when the job finishes, fails, is cancelled or panics.
pub struct JournalGuard {
    journal: Arc<Journal>,
    job_id: String,
}

impl Drop for JournalGuard {
    fn drop(&mut self) {
        self.journal.finish(&self.job_id);
    }
}

/// Remove staged files of process `pid` in `dir`. Returns (removed count, files that remain).
fn remove_staged(dir: &Path, pid: u32) -> (u32, Vec<PathBuf>) {
    let prefix = staged_prefix(pid);
    let mut removed = 0;
    let mut left = Vec::new();
    let Ok(rd) = fs::read_dir(dir) else { return (0, left) };
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if name.starts_with(&prefix) && name.ends_with(".tmp") && e.file_type().is_ok_and(|t| t.is_file()) {
            match fs::remove_file(e.path()) {
                Ok(()) => removed += 1,
                Err(_) => left.push(e.path()),
            }
        }
    }
    (removed, left)
}

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ops::SCHEMA_VERSION;

    fn request(out: &Path) -> OperationRequest {
        OperationRequest {
            schema_version: SCHEMA_VERSION,
            request_id: None,
            operation: Operation::ExtractPages { input: "in.pdf".into(), pages: "1".into(), output: out.to_path_buf() },
            collision: CollisionPolicy::Fail,
            password: Some("secret-pw".into()),
            new_password: Some("new-secret-pw".into()),
            new_owner_password: Some("owner-secret-pw".into()),
            certificate_password: Some("cert-secret-pw".into()),
            keep_protection: true,
        }
    }

    /// Simulate an entry left by a process that crashed: written by hand with a foreign pid.
    fn crashed_entry(journal_dir: &Path, pid: u32, out: &Path) {
        let entry = JournalEntry {
            job_id: "dead".into(),
            label: "extract_pages".into(),
            pid,
            started_unix_ms: 1,
            operation: request(out).operation,
            collision: CollisionPolicy::Fail,
        };
        fs::write(journal_dir.join("job-dead.json"), serde_json::to_vec(&entry).expect("json")).expect("write");
    }

    #[test]
    fn finished_jobs_leave_no_entry_and_passwords_are_never_written() {
        let d = tempfile::tempdir().expect("tmp");
        let j = Journal::open(&d.path().join("jobs")).expect("open");
        let g = j.begin("a1", "extract_pages", &request(&d.path().join("o.pdf"))).expect("begin");
        let text = fs::read_to_string(d.path().join("jobs/job-a1.json")).expect("entry");
        assert!(
            text.contains("extract_pages") && !text.contains("secret-pw"),
            "no password of any kind on disk: {text}"
        );
        drop(g);
        assert!(!d.path().join("jobs/job-a1.json").exists());
        assert!(j.recover().is_empty());
    }

    #[test]
    fn panicking_job_still_clears_its_entry() {
        let d = tempfile::tempdir().expect("tmp");
        let j = Journal::open(&d.path().join("jobs")).expect("open");
        let j2 = j.clone();
        let out = d.path().join("o.pdf");
        let r = std::panic::catch_unwind(move || {
            let _g = j2.begin("p1", "x", &request(&out)).expect("begin");
            panic!("boom");
        });
        assert!(r.is_err());
        assert!(!d.path().join("jobs/job-p1.json").exists());
    }

    #[test]
    fn recovery_removes_only_the_dead_process_staged_files() {
        let d = tempfile::tempdir().expect("tmp");
        let out_dir = d.path().join("out");
        fs::create_dir_all(&out_dir).expect("mkdir");
        let dead_pid = u32::MAX - 7; // no such process; no lock file
        fs::write(out_dir.join(format!("{}abc.tmp", staged_prefix(dead_pid))), b"partial").expect("w");
        fs::write(out_dir.join(format!("{}abc.tmp", staged_prefix(dead_pid - 1))), b"other").expect("w");
        fs::write(out_dir.join("kept.pdf"), b"committed").expect("w");
        let jdir = d.path().join("jobs");
        let j = Journal::open(&jdir).expect("open");
        crashed_entry(&jdir, dead_pid, &out_dir.join("o.pdf"));
        let rec = j.recover();
        assert_eq!(rec.len(), 1);
        assert_eq!(rec[0].removed_partial_files, 1);
        assert!(rec[0].leftover_partial_files.is_empty());
        assert!(out_dir.join("kept.pdf").exists(), "committed outputs untouched");
        assert!(
            out_dir.join(format!("{}abc.tmp", staged_prefix(dead_pid - 1))).exists(),
            "other processes' files untouched"
        );
        assert!(!jdir.join("job-dead.json").exists(), "entry consumed");
        assert!(j.recover().is_empty(), "reported once");
    }

    #[test]
    fn live_instances_are_left_alone() {
        let d = tempfile::tempdir().expect("tmp");
        let jdir = d.path().join("jobs");
        let out_dir = d.path().join("out");
        fs::create_dir_all(&out_dir).expect("mkdir");
        let j = Journal::open(&jdir).expect("open");
        // Another live instance: it holds its lock file.
        let other_pid = u32::MAX - 11;
        let held = File::create(jdir.join(format!("instance-{other_pid}.lock"))).expect("lock file");
        held.try_lock().expect("lock");
        fs::write(out_dir.join(format!("{}x.tmp", staged_prefix(other_pid))), b"live").expect("w");
        crashed_entry(&jdir, other_pid, &out_dir.join("o.pdf"));
        assert!(j.recover().is_empty());
        assert!(out_dir.join(format!("{}x.tmp", staged_prefix(other_pid))).exists());
        drop(held);
        let rec = j.recover();
        assert_eq!(rec.len(), 1, "recovered once that instance has ended");
        assert!(!jdir.join(format!("instance-{other_pid}.lock")).exists(), "stale lock cleaned");
    }

    #[test]
    fn corrupt_entries_are_set_aside_not_deleted() {
        let d = tempfile::tempdir().expect("tmp");
        let jdir = d.path().join("jobs");
        let j = Journal::open(&jdir).expect("open");
        fs::write(jdir.join("job-bad.json"), b"{not json").expect("w");
        assert!(j.recover().is_empty());
        assert!(jdir.join("job-bad.json.corrupt").exists());
    }
}
