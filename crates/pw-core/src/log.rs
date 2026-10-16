//! Structured, privacy-safe JSONL logging (PRD 12 "Logging rules").
//!
//! Events carry an operation/job ID, a component, an event name and typed fields. There is no
//! free-form message parameter, so document text cannot be logged by accident; string fields are
//! scanned for secret-like tokens and paths are logged only through [`redact_path`]. Files rotate by
//! size and count. Logging failures (disk full, permissions) never propagate to the caller — they
//! are counted and the logger keeps working.

use serde::Serialize;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Level {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

/// Privacy-safe description of a path: extension plus a short stable hash of the full path.
/// Lets diagnostics correlate events about the same file without revealing names or folders.
pub fn redact_path(path: &Path) -> String {
    let digest = Sha256::digest(path.as_os_str().to_string_lossy().as_bytes());
    let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    format!(
        "<file:{}:{:02x}{:02x}{:02x}{:02x}>",
        if ext.is_empty() { "-" } else { &ext },
        digest[0],
        digest[1],
        digest[2],
        digest[3]
    )
}

/// Replace substrings that look like credentials. Defence in depth — callers must not pass secrets.
pub fn scrub(s: &str) -> String {
    const PREFIXES: [&str; 9] = ["github_pat_", "ghp_", "gho_", "ghs_", "KGAT_", "cfut_", "sk-", "xoxb-", "AKIA"];
    let mut out = String::with_capacity(s.len());
    for word in s.split_inclusive(|c: char| c.is_whitespace() || c == '"' || c == '\'' || c == '=') {
        let trimmed = word.trim_end_matches(|c: char| c.is_whitespace() || c == '"' || c == '\'' || c == '=');
        let looks_secret = PREFIXES.iter().any(|p| trimmed.starts_with(p) && trimmed.len() >= p.len() + 8)
            || trimmed.to_ascii_lowercase().starts_with("bearer")
            || (trimmed.len() >= 32 && trimmed.chars().all(|c| c.is_ascii_hexdigit()));
        if looks_secret {
            out.push_str("[REDACTED]");
            out.push_str(&word[trimmed.len()..]);
        } else {
            out.push_str(word);
        }
    }
    out
}

pub struct LogConfig {
    pub dir: PathBuf,
    pub file_stem: String,
    pub max_bytes: u64,
    pub max_files: usize,
    pub min_level: Level,
    pub build: String,
}

pub struct Logger {
    cfg: LogConfig,
    state: Mutex<Option<(File, u64)>>,
    dropped: AtomicU64,
}

impl Logger {
    pub fn new(cfg: LogConfig) -> Self {
        Logger { cfg, state: Mutex::new(None), dropped: AtomicU64::new(0) }
    }

    /// Number of events that could not be written (e.g. disk full). Exposed in diagnostics.
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }

    pub fn current_path(&self) -> PathBuf {
        self.cfg.dir.join(format!("{}.jsonl", self.cfg.file_stem))
    }

    fn rotated_path(&self, n: usize) -> PathBuf {
        self.cfg.dir.join(format!("{}.{n}.jsonl", self.cfg.file_stem))
    }

    pub fn log(&self, level: Level, component: &str, event: &str, op_id: Option<&str>, fields: Map<String, Value>) {
        if level < self.cfg.min_level {
            return;
        }
        let mut obj = Map::new();
        obj.insert("ts".into(), Value::String(OffsetDateTime::now_utc().format(&Rfc3339).unwrap_or_default()));
        obj.insert("level".into(), serde_json::to_value(level).unwrap_or(Value::Null));
        obj.insert("component".into(), Value::String(component.into()));
        obj.insert("event".into(), Value::String(event.into()));
        obj.insert("build".into(), Value::String(self.cfg.build.clone()));
        if let Some(id) = op_id {
            obj.insert("op".into(), Value::String(id.into()));
        }
        let mut clean = Map::new();
        for (k, v) in fields {
            clean.insert(k, scrub_value(v));
        }
        obj.insert("fields".into(), Value::Object(clean));
        let mut line = serde_json::to_vec(&Value::Object(obj)).unwrap_or_default();
        line.push(b'\n');
        if self.write_line(&line).is_err() {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn write_line(&self, line: &[u8]) -> std::io::Result<()> {
        let mut guard = self.state.lock().unwrap_or_else(|p| p.into_inner());
        if guard.is_none() {
            fs::create_dir_all(&self.cfg.dir)?;
            let f = OpenOptions::new().create(true).append(true).open(self.current_path())?;
            let len = f.metadata()?.len();
            *guard = Some((f, len));
        }
        let needs_rotate =
            guard.as_ref().map(|(_, len)| *len + line.len() as u64 > self.cfg.max_bytes).unwrap_or(false);
        if needs_rotate {
            *guard = None;
            self.rotate()?;
            let f = OpenOptions::new().create(true).append(true).open(self.current_path())?;
            *guard = Some((f, 0));
        }
        let (f, len) = guard.as_mut().ok_or_else(|| std::io::Error::other("log file unavailable"))?;
        f.write_all(line)?;
        *len += line.len() as u64;
        Ok(())
    }

    fn rotate(&self) -> std::io::Result<()> {
        if self.cfg.max_files <= 1 {
            return fs::remove_file(self.current_path()).or_else(ignore_not_found);
        }
        let oldest = self.rotated_path(self.cfg.max_files - 1);
        fs::remove_file(&oldest).or_else(ignore_not_found)?;
        for n in (1..self.cfg.max_files - 1).rev() {
            let from = self.rotated_path(n);
            if from.exists() {
                fs::rename(&from, self.rotated_path(n + 1))?;
            }
        }
        fs::rename(self.current_path(), self.rotated_path(1)).or_else(ignore_not_found)
    }
}

fn ignore_not_found(e: std::io::Error) -> std::io::Result<()> {
    if e.kind() == std::io::ErrorKind::NotFound {
        Ok(())
    } else {
        Err(e)
    }
}

fn scrub_value(v: Value) -> Value {
    match v {
        Value::String(s) => Value::String(scrub(&s)),
        Value::Array(a) => Value::Array(a.into_iter().map(scrub_value).collect()),
        Value::Object(o) => Value::Object(o.into_iter().map(|(k, v)| (k, scrub_value(v))).collect()),
        other => other,
    }
}

/// Build a field map: `fields!{"pages" => 3, "file" => redact_path(p)}`.
#[macro_export]
macro_rules! fields {
    ($($k:expr => $v:expr),* $(,)?) => {{
        #[allow(unused_mut)]
        let mut m = ::serde_json::Map::new();
        $( m.insert($k.to_string(), ::serde_json::json!($v)); )*
        m
    }};
}

#[cfg(test)]
mod tests {
    use super::*;

    fn logger(dir: &Path, max_bytes: u64, max_files: usize) -> Logger {
        Logger::new(LogConfig {
            dir: dir.to_path_buf(),
            file_stem: "pageweave".into(),
            max_bytes,
            max_files,
            min_level: Level::Info,
            build: "test".into(),
        })
    }

    #[test]
    fn writes_structured_jsonl() {
        let dir = tempfile::tempdir().expect("tempdir");
        let l = logger(dir.path(), 1 << 20, 3);
        l.log(Level::Info, "pdf", "open.done", Some("op-1"), fields! {"pages" => 3});
        l.log(Level::Debug, "pdf", "ignored", None, fields! {});
        let text = fs::read_to_string(l.current_path()).expect("read");
        let lines: Vec<Value> = text.lines().map(|s| serde_json::from_str(s).expect("json")).collect();
        assert_eq!(lines.len(), 1, "debug is below the INFO default");
        assert_eq!(lines[0]["event"], "open.done");
        assert_eq!(lines[0]["op"], "op-1");
        assert_eq!(lines[0]["fields"]["pages"], 3);
        assert!(lines[0]["ts"].as_str().expect("ts").ends_with('Z'));
    }

    #[test]
    fn secrets_are_scrubbed() {
        let dir = tempfile::tempdir().expect("tempdir");
        let l = logger(dir.path(), 1 << 20, 3);
        let fake_pat = format!("github_pat_{}", "A1b2C3d4E5f6G7h8");
        let fake_kaggle = format!("KGAT_{}", "0123456789abcdef");
        l.log(
            Level::Error,
            "net",
            "request.failed",
            None,
            fields! {"detail" => format!("auth={fake_pat} other token {fake_kaggle} Bearer xyz"), "nested" => [fake_kaggle.as_str()]},
        );
        let text = fs::read_to_string(l.current_path()).expect("read");
        assert!(!text.contains(&fake_pat), "{text}");
        assert!(!text.contains(&fake_kaggle), "{text}");
        assert!(text.contains("[REDACTED]"));
    }

    #[test]
    fn redacted_path_hides_name_but_is_stable() {
        let a = redact_path(Path::new("/home/alice/Medical Records/biopsy.PDF"));
        assert!(!a.contains("alice") && !a.contains("biopsy"), "{a}");
        assert!(a.starts_with("<file:pdf:"));
        assert_eq!(a, redact_path(Path::new("/home/alice/Medical Records/biopsy.PDF")));
        assert_ne!(a, redact_path(Path::new("/home/alice/other.pdf")));
    }

    #[test]
    fn rotates_by_size_and_bounds_file_count() {
        let dir = tempfile::tempdir().expect("tempdir");
        let l = logger(dir.path(), 400, 3);
        for i in 0..50 {
            l.log(Level::Info, "c", "e", None, fields! {"i" => i});
        }
        let mut names: Vec<String> = fs::read_dir(dir.path())
            .expect("dir")
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(names, vec!["pageweave.1.jsonl", "pageweave.2.jsonl", "pageweave.jsonl"]);
        for n in &names {
            assert!(fs::metadata(dir.path().join(n)).expect("meta").len() <= 400);
        }
        assert_eq!(l.dropped(), 0);
    }

    #[test]
    fn unwritable_location_counts_drops_instead_of_failing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let blocker = dir.path().join("not-a-dir");
        fs::write(&blocker, b"x").expect("seed");
        let l = logger(&blocker.join("logs"), 1 << 20, 2);
        l.log(Level::Info, "c", "e", None, fields! {});
        assert_eq!(l.dropped(), 1);
    }
}
