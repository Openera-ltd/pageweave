//! Read-only engine calls in long-lived reader processes (decision D-018). The viewer renders
//! pages and reads text, fields, comments and the like constantly; running those calls in copies
//! of the executable (started with [`READER_FLAG`]) means a PDF engine crash on a damaged page
//! fails that one call instead of closing the app. A crashed reader is replaced on the next call;
//! a call that crashed a reader is not retried (so a bad page cannot crash readers in a loop).
//!
//! Protocol, one request at a time per reader:
//! - parent → child: one JSON line `{"call": ReadCall, "password": …}`.
//! - child → parent: one JSON line `{"ok": value}`, `{"error": PwError}` or `{"bytes": n}`
//!   followed by exactly `n` raw bytes (PNG images).

use crate::document::OpenOptions;
use pw_core::{ErrorCategory, PwError, PwResult};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc;
use std::sync::Mutex;
use std::time::Duration;

/// Command-line flag that turns an executable into a reader.
pub const READER_FLAG: &str = "--pw-reader";
const MAX_LINE: u64 = 64 * 1024 * 1024;
const MAX_BYTES: usize = 512 * 1024 * 1024;
/// A single read taking longer than this is treated as hung: the reader is replaced.
pub const CALL_TIMEOUT: Duration = Duration::from_secs(120);

/// One read-only engine call.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "call", rename_all = "snake_case")]
pub enum ReadCall {
    Open { path: PathBuf },
    Render { path: PathBuf, page: u32, scale: f32 },
    PageText { path: PathBuf, page: u32 },
    Search { path: PathBuf, query: String, match_case: bool, whole_word: bool },
    FormFields { path: PathBuf },
    RedactionPreview { path: PathBuf, areas: Vec<pw_core::ops::RedactArea> },
    ListAttachments { path: PathBuf },
    ListAnnotations { path: PathBuf },
    VerifySignatures { path: PathBuf, trusted: Vec<PathBuf> },
    PageObjects { path: PathBuf, page: u32 },
    PreviewEdits { path: PathBuf, page: u32, edits: Vec<pw_core::ops::PageEdit>, scale: f32 },
    SanitizeInspect { path: PathBuf },
}

impl ReadCall {
    /// What identifies this call for "it crashed a reader before" (path, kind, page).
    fn key(&self) -> String {
        let (kind, path, page): (&str, &Path, Option<u32>) = match self {
            ReadCall::Open { path } => ("open", path, None),
            ReadCall::Render { path, page, .. } => ("render", path, Some(*page)),
            ReadCall::PageText { path, page } => ("page_text", path, Some(*page)),
            ReadCall::Search { path, .. } => ("search", path, None),
            ReadCall::FormFields { path } => ("form_fields", path, None),
            ReadCall::RedactionPreview { path, .. } => ("redaction_preview", path, None),
            ReadCall::ListAttachments { path } => ("list_attachments", path, None),
            ReadCall::ListAnnotations { path } => ("list_annotations", path, None),
            ReadCall::VerifySignatures { path, .. } => ("verify_signatures", path, None),
            ReadCall::PageObjects { path, page } => ("page_objects", path, Some(*page)),
            ReadCall::PreviewEdits { path, page, .. } => ("preview_edits", path, Some(*page)),
            ReadCall::SanitizeInspect { path } => ("sanitize_inspect", path, None),
        };
        format!("{kind}\u{1}{}\u{1}{page:?}", path.display())
    }
}

/// The answer to a call: JSON data, or raw bytes for images.
#[derive(Debug, Clone, PartialEq)]
pub enum ReadReply {
    Json(Value),
    Bytes(Vec<u8>),
}

#[derive(Serialize, Deserialize)]
struct Request {
    call: ReadCall,
    #[serde(default)]
    password: Option<String>,
}

fn to_json<T: Serialize>(r: PwResult<T>) -> PwResult<ReadReply> {
    r.map(|v| ReadReply::Json(serde_json::to_value(v).unwrap_or_default()))
}

/// Run a call in this process (the reader's side, and the fallback when no pool is set up).
pub fn run_call(call: &ReadCall, password: Option<&str>) -> PwResult<ReadReply> {
    let opts = OpenOptions { password };
    match call {
        ReadCall::Open { path } => to_json(crate::open_document(path, &opts)),
        ReadCall::Render { path, page, scale } => {
            crate::render_page_png(path, &opts, *page, *scale).map(|(png, _, _)| ReadReply::Bytes(png))
        }
        ReadCall::PageText { path, page } => to_json(crate::page_text_segments(path, &opts, *page)),
        ReadCall::Search { path, query, match_case, whole_word } => {
            crate::search(path, &opts, query, *match_case, *whole_word, &|| false)
                .map(|(hits, truncated)| ReadReply::Json(json!({ "hits": hits, "truncated": truncated })))
        }
        ReadCall::FormFields { path } => to_json(crate::form_fields(path, password)),
        ReadCall::RedactionPreview { path, areas } => to_json(crate::redaction_preview(path, &opts, areas)),
        ReadCall::ListAttachments { path } => to_json(crate::list_attachments(path, &opts).map(|(l, _)| l)),
        ReadCall::ListAnnotations { path } => to_json(crate::list_annotations(path, &opts)),
        ReadCall::VerifySignatures { path, trusted } => {
            let anchors = crate::load_trusted_certificates(trusted)?;
            to_json(crate::verify_signatures(path, &opts, &anchors))
        }
        ReadCall::PageObjects { path, page } => to_json(crate::objects::editable_objects(path, &opts, *page)),
        ReadCall::PreviewEdits { path, page, edits, scale } => {
            crate::objects::preview_object_edits(path, &opts, *page, edits, *scale).map(ReadReply::Bytes)
        }
        ReadCall::SanitizeInspect { path } => to_json(crate::sanitize_inspect(path, &opts)),
    }
}

/// Debug builds only: `PW_READER_TEST=abort:<page>` or `hang:<page>` (or the same text in the file
/// `pageweave-reader-test` in the temporary folder, for apps started by a test driver) makes a
/// reader abort or hang on rendering that page, so the crash and timeout paths can be tested.
/// Release builds ignore both.
fn test_hooks(call: &ReadCall) {
    #[cfg(debug_assertions)]
    let hook = std::env::var("PW_READER_TEST").ok().or_else(|| {
        std::fs::read_to_string(std::env::temp_dir().join("pageweave-reader-test")).ok().map(|s| s.trim().to_string())
    });
    #[cfg(debug_assertions)]
    if let (Some(v), ReadCall::Render { page, .. }) = (hook, call) {
        match v.split_once(':') {
            Some(("abort", p)) if p.parse() == Ok(*page) => std::process::abort(),
            Some(("hang", p)) if p.parse() == Ok(*page) => std::thread::sleep(Duration::from_secs(3600)),
            _ => {}
        }
    }
    #[cfg(not(debug_assertions))]
    let _ = call;
}

fn read_line_bounded(r: &mut impl BufRead) -> Option<String> {
    let mut s = String::new();
    match r.take(MAX_LINE).read_line(&mut s) {
        Ok(0) | Err(_) => None,
        Ok(_) => Some(s),
    }
}

/// Reader side: answer requests from stdin until it closes.
pub fn run_reader() {
    let mut input = BufReader::new(std::io::stdin());
    let mut out = std::io::stdout().lock();
    while let Some(line) = read_line_bounded(&mut input) {
        let reply = match serde_json::from_str::<Request>(&line) {
            Ok(req) => {
                test_hooks(&req.call);
                run_call(&req.call, req.password.as_deref())
            }
            Err(_) => Err(PwError::internal("reader.input", "The request could not be read.")),
        };
        let ok = match reply {
            Ok(ReadReply::Json(v)) => writeln!(out, "{}", json!({ "ok": v })),
            Ok(ReadReply::Bytes(b)) => writeln!(out, "{}", json!({ "bytes": b.len() })).and_then(|_| out.write_all(&b)),
            Err(e) => writeln!(out, "{}", json!({ "error": e })),
        };
        if ok.and_then(|_| out.flush()).is_err() {
            return;
        }
    }
}

fn crashed() -> PwError {
    PwError::new(
        ErrorCategory::EngineFailure,
        "engine.crashed",
        "The PDF engine stopped unexpectedly while reading this file. The file may be damaged.",
    )
    .with_action("Try “Reduce file size” on the file, or open it in another program and save a copy.")
}

fn hung() -> PwError {
    PwError::new(
        ErrorCategory::Timeout,
        "engine.timeout",
        "Reading this part of the file took too long and was stopped. The file may be damaged.",
    )
}

/// Message from a reader: a header line and, for images, its bytes.
type Message = (Value, Option<Vec<u8>>);

struct Reader {
    child: Child,
    stdin: ChildStdin,
    rx: mpsc::Receiver<Message>,
}

impl Reader {
    fn start(exe: &Path, env: &[(String, String)]) -> PwResult<Reader> {
        let mut child = Command::new(exe)
            .arg(READER_FLAG)
            .envs(env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| {
                PwError::new(
                    ErrorCategory::Internal,
                    "reader.spawn",
                    format!("The document reader could not be started ({}).", e.kind()),
                )
            })?;
        let stdin = child.stdin.take().ok_or_else(crashed)?;
        let stdout = child.stdout.take().ok_or_else(crashed)?;
        let (tx, rx) = mpsc::channel::<Message>();
        std::thread::spawn(move || {
            let mut r = BufReader::new(stdout);
            while let Some(line) = read_line_bounded(&mut r) {
                let Ok(v) = serde_json::from_str::<Value>(&line) else { return };
                let bytes = match v.get("bytes").and_then(Value::as_u64) {
                    Some(n) if (n as usize) <= MAX_BYTES => {
                        let mut b = vec![0u8; n as usize];
                        if r.read_exact(&mut b).is_err() {
                            return;
                        }
                        Some(b)
                    }
                    Some(_) => return,
                    None => None,
                };
                if tx.send((v, bytes)).is_err() {
                    return;
                }
            }
        });
        Ok(Reader { child, stdin, rx })
    }

    fn stop(mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// A small pool of readers. Calls run in parallel up to the pool size.
pub struct ReaderPool {
    exe: PathBuf,
    slots: Vec<Mutex<Option<Reader>>>,
    /// Calls that crashed a reader; answered with the crash at once afterwards.
    crashed: Mutex<HashSet<String>>,
    timeout: Duration,
    env: Vec<(String, String)>,
}

impl ReaderPool {
    pub fn new(exe: PathBuf, size: usize) -> ReaderPool {
        ReaderPool::with_timeout(exe, size, CALL_TIMEOUT)
    }

    pub fn with_timeout(exe: PathBuf, size: usize, timeout: Duration) -> ReaderPool {
        ReaderPool {
            exe,
            slots: (0..size.clamp(1, 8)).map(|_| Mutex::new(None)).collect(),
            crashed: Mutex::new(HashSet::new()),
            timeout,
            env: Vec::new(),
        }
    }

    /// Extra environment for the readers (tests use it for the debug-build hooks).
    pub fn with_env(mut self, key: &str, value: &str) -> ReaderPool {
        self.env.push((key.into(), value.into()));
        self
    }

    /// Run `call` in a reader.
    pub fn call(&self, call: &ReadCall, password: Option<&str>) -> PwResult<ReadReply> {
        let key = call.key();
        if self.crashed.lock().unwrap_or_else(|p| p.into_inner()).contains(&key) {
            return Err(crashed());
        }
        // A free reader if there is one, else wait for the first.
        let slot = self
            .slots
            .iter()
            .find_map(|s| s.try_lock().ok())
            .unwrap_or_else(|| self.slots[0].lock().unwrap_or_else(|p| p.into_inner()));
        let mut slot = slot;
        // A reader that ended while idle (killed, out of memory) is replaced without blaming this call.
        if let Some(r) = slot.as_mut() {
            if !matches!(r.child.try_wait(), Ok(None)) {
                if let Some(r) = slot.take() {
                    r.stop();
                }
            }
        }
        if slot.is_none() {
            *slot = Some(Reader::start(&self.exe, &self.env)?);
        }
        let reader = slot.as_mut().expect("reader started");
        let line = serde_json::to_string(&Request { call: call.clone(), password: password.map(str::to_string) })
            .map_err(|_| PwError::internal("reader.request", "The request could not be prepared."))?;
        let sent = writeln!(reader.stdin, "{line}").and_then(|_| reader.stdin.flush());
        let answer = match sent {
            Ok(()) => reader.rx.recv_timeout(self.timeout),
            Err(_) => Err(mpsc::RecvTimeoutError::Disconnected),
        };
        match answer {
            Ok((v, bytes)) => {
                if let Some(b) = bytes {
                    return Ok(ReadReply::Bytes(b));
                }
                if let Some(ok) = v.get("ok") {
                    return Ok(ReadReply::Json(ok.clone()));
                }
                if let Some(e) = v.get("error") {
                    return Err(serde_json::from_value(e.clone()).unwrap_or_else(|_| crashed()));
                }
                // Not a reply this protocol knows: replace the reader.
                if let Some(r) = slot.take() {
                    r.stop();
                }
                Err(crashed())
            }
            Err(e) => {
                if let Some(r) = slot.take() {
                    r.stop();
                }
                self.crashed.lock().unwrap_or_else(|p| p.into_inner()).insert(key);
                Err(match e {
                    mpsc::RecvTimeoutError::Timeout => hung(),
                    mpsc::RecvTimeoutError::Disconnected => crashed(),
                })
            }
        }
    }

    /// Process ids of the running readers (diagnostics and tests).
    pub fn reader_pids(&self) -> Vec<u32> {
        self.slots.iter().filter_map(|s| s.lock().ok().and_then(|g| g.as_ref().map(|r| r.child.id()))).collect()
    }

    /// Forget which calls crashed (for example after a file was replaced).
    pub fn forget_crashes_for(&self, path: &Path) {
        let p = format!("\u{1}{}\u{1}", path.display());
        self.crashed.lock().unwrap_or_else(|x| x.into_inner()).retain(|k| !k.contains(&p));
    }
}

impl Drop for ReaderPool {
    fn drop(&mut self) {
        for s in &self.slots {
            if let Some(r) = s.lock().unwrap_or_else(|p| p.into_inner()).take() {
                r.stop();
            }
        }
    }
}
