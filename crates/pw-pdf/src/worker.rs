//! Running one operation in a separate process (decision D-015). PDFium can abort the whole
//! process on some damaged files (an internal check fails; reproduced by the robustness tests), so
//! the app runs jobs in a child process: a crash then fails one job instead of closing the app.
//!
//! Protocol over the child's stdin/stdout, one JSON value per line:
//! - parent → child: the request (as serialized, which never includes passwords), then a secrets
//!   line `{"password":…,"new_password":…,"new_owner_password":…,"certificate_password":…}`,
//!   then optionally `"cancel"`. Closing stdin also cancels (the parent went away).
//! - child → parent: `{"progress":{"done":…,"total":…,"unit":…}}` / `{"phase":…}` while running,
//!   then exactly one `{"result":OperationResult}` or `{"error":PwError}`.

use crate::execute;
use pw_core::job::{JobManager, JobState, Progress};
use pw_core::ops::{OperationRequest, OperationResult};
use pw_core::{ErrorCategory, PwError, PwResult};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::Duration;

/// Command-line flag that turns an executable into a worker.
pub const WORKER_FLAG: &str = "--pw-worker";
/// Longest line accepted from either side.
const MAX_LINE: u64 = 64 * 1024 * 1024;

#[derive(Default, Serialize, Deserialize)]
struct Secrets {
    password: Option<String>,
    new_password: Option<String>,
    new_owner_password: Option<String>,
    certificate_password: Option<String>,
}

fn read_line_bounded(r: &mut impl BufRead) -> Option<String> {
    let mut s = String::new();
    match r.take(MAX_LINE).read_line(&mut s) {
        Ok(0) | Err(_) => None,
        Ok(_) => Some(s),
    }
}

/// Child side: read one request from stdin, run it, report on stdout, return.
pub fn run_worker() {
    let mut input = BufReader::new(std::io::stdin());
    let out = std::io::stdout();
    let send = |v: serde_json::Value| {
        let mut o = out.lock();
        let _ = writeln!(o, "{v}");
        let _ = o.flush();
    };
    let req_line = read_line_bounded(&mut input);
    let secrets_line = read_line_bounded(&mut input);
    let (Some(req_line), Some(secrets_line)) = (req_line, secrets_line) else {
        send(json!({ "error": PwError::internal("worker.input", "The task could not be started (no request).") }));
        return;
    };
    let mut req: OperationRequest = match serde_json::from_str(&req_line) {
        Ok(r) => r,
        Err(_) => {
            send(json!({ "error": PwError::internal("worker.input", "The task request could not be read.") }));
            return;
        }
    };
    let secrets: Secrets = serde_json::from_str(&secrets_line).unwrap_or_default();
    req.password = secrets.password;
    req.new_password = secrets.new_password;
    req.new_owner_password = secrets.new_owner_password;
    req.certificate_password = secrets.certificate_password;

    let jobs = Arc::new(JobManager::new(1));
    let id = jobs.submit(req.operation.id(), move |ctx| {
        execute(&req, Some(ctx)).map(|r| serde_json::to_value(r).unwrap_or_default())
    });
    // "cancel" or end of input (the parent went away) cancels the job.
    {
        let jobs = jobs.clone();
        std::thread::spawn(move || loop {
            match read_line_bounded(&mut input) {
                Some(l) if l.trim() == "\"cancel\"" || l.trim() == "cancel" => {
                    jobs.cancel(id);
                }
                Some(_) => {}
                None => {
                    jobs.cancel(id);
                    return;
                }
            }
        });
    }
    let mut last: Option<(Progress, String)> = None;
    loop {
        let Some(s) = jobs.wait(id, Duration::from_millis(150)) else {
            send(json!({ "error": PwError::internal("worker.lost", "The task was lost.") }));
            return;
        };
        let now = (s.progress.clone(), s.phase.clone());
        if last.as_ref() != Some(&now) {
            if last.as_ref().map(|l| &l.1) != Some(&s.phase) {
                send(json!({ "phase": s.phase }));
            }
            if let Progress::Determinate { .. } = s.progress {
                send(json!({ "progress": s.progress }));
            }
            last = Some(now);
        }
        match s.state {
            JobState::Succeeded => {
                send(json!({ "result": s.result.unwrap_or_default() }));
                return;
            }
            JobState::Failed => {
                send(
                    json!({ "error": s.error.unwrap_or_else(|| PwError::internal("worker.failed", "The task failed.")) }),
                );
                return;
            }
            JobState::Cancelled => {
                send(json!({ "error": PwError::cancelled() }));
                return;
            }
            _ => {}
        }
    }
}

/// What the parent hears from a running worker.
pub enum WorkerEvent {
    Progress(Progress),
    Phase(String),
}

fn crashed() -> PwError {
    PwError::new(
        ErrorCategory::EngineFailure,
        "engine.crashed",
        "The PDF engine stopped unexpectedly while working on this file. The file may be damaged; nothing was saved.",
    )
    .with_action(
        "Try “Reduce file size” or another tool on the file first, or open it in another program and save a copy.",
    )
}

/// Parent side: run `req` in `exe WORKER_FLAG`. `on_event` receives progress; `cancelled` is polled
/// and, once true, the worker is asked to stop (and killed if it does not within a few seconds).
/// Returns the worker's pid with the outcome, so the caller can clean up its staged files.
pub fn run_in_worker(
    exe: &std::path::Path,
    req: &OperationRequest,
    cancelled: &dyn Fn() -> bool,
    on_event: &mut dyn FnMut(WorkerEvent),
) -> (Option<u32>, PwResult<OperationResult>) {
    let mut child = match Command::new(exe)
        .arg(WORKER_FLAG)
        .env(pw_core::output::STAGE_OWNER_ENV, std::process::id().to_string())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            return (
                None,
                Err(PwError::new(
                    ErrorCategory::Internal,
                    "worker.spawn",
                    format!("The task could not be started ({}).", e.kind()),
                )),
            )
        }
    };
    let pid = child.id();
    let result = talk(&mut child, req, cancelled, on_event);
    let _ = child.kill();
    let _ = child.wait();
    (Some(pid), result)
}

fn talk(
    child: &mut Child,
    req: &OperationRequest,
    cancelled: &dyn Fn() -> bool,
    on_event: &mut dyn FnMut(WorkerEvent),
) -> PwResult<OperationResult> {
    let secrets = Secrets {
        password: req.password.clone(),
        new_password: req.new_password.clone(),
        new_owner_password: req.new_owner_password.clone(),
        certificate_password: req.certificate_password.clone(),
    };
    let mut stdin: ChildStdin = child.stdin.take().ok_or_else(crashed)?;
    let line = serde_json::to_string(req)
        .map_err(|_| PwError::internal("worker.request", "The task could not be prepared."))?;
    let sec = serde_json::to_string(&secrets).unwrap_or_else(|_| "{}".into());
    if writeln!(stdin, "{line}").and_then(|_| writeln!(stdin, "{sec}")).and_then(|_| stdin.flush()).is_err() {
        return Err(crashed());
    }
    // Lines are read on a thread so cancellation can be checked while the worker is quiet.
    let stdout = child.stdout.take().ok_or_else(crashed)?;
    let (tx, rx) = mpsc::channel::<String>();
    std::thread::spawn(move || {
        let mut r = BufReader::new(stdout);
        while let Some(l) = read_line_bounded(&mut r) {
            if tx.send(l).is_err() {
                return;
            }
        }
    });
    let cancel_sent = AtomicBool::new(false);
    let mut cancel_deadline: Option<std::time::Instant> = None;
    loop {
        if cancelled() && !cancel_sent.swap(true, Ordering::SeqCst) {
            let _ = writeln!(stdin, "\"cancel\"");
            let _ = stdin.flush();
            cancel_deadline = Some(std::time::Instant::now() + Duration::from_secs(10));
        }
        if cancel_deadline.is_some_and(|d| std::time::Instant::now() > d) {
            return Err(PwError::cancelled());
        }
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(l) => {
                let v: serde_json::Value = match serde_json::from_str(&l) {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                if let Some(p) = v.get("progress") {
                    if let Ok(p) = serde_json::from_value::<Progress>(p.clone()) {
                        on_event(WorkerEvent::Progress(p));
                    }
                } else if let Some(p) = v.get("phase").and_then(|p| p.as_str()) {
                    on_event(WorkerEvent::Phase(p.to_string()));
                } else if let Some(r) = v.get("result") {
                    return serde_json::from_value(r.clone()).map_err(|_| crashed());
                } else if let Some(e) = v.get("error") {
                    return Err(serde_json::from_value(e.clone()).unwrap_or_else(|_| crashed()));
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            // The worker's output ended without a result: it crashed or was killed.
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err(if cancel_sent.load(Ordering::SeqCst) { PwError::cancelled() } else { crashed() });
            }
        }
    }
}
