//! Job lifecycle (PRD 04 "Job system", docs/ARCHITECTURE.md "Jobs and UI communication").
//!
//! Work runs on a bounded pool of worker threads, never the caller's (UI) thread. Cancellation is
//! cooperative: a job observes [`JobContext::check_cancelled`] at safe boundaries. Terminal state is
//! truthful — if a job finishes its commit after cancellation was requested, it is reported as
//! `Succeeded`, not `Cancelled`. Progress is either determinate (measured units) or explicitly
//! indeterminate; percentages are never invented.

use crate::error::{ErrorCategory, PwError, PwResult};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct JobId(pub Uuid);

impl std::fmt::Display for JobId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    Queued,
    Running,
    Cancelling,
    Cancelled,
    Succeeded,
    Failed,
}

impl JobState {
    pub fn is_terminal(self) -> bool {
        matches!(self, JobState::Cancelled | JobState::Succeeded | JobState::Failed)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Progress {
    /// Total unknown — UI must show an indeterminate indicator, never a fake percentage.
    Indeterminate,
    Determinate {
        done: u64,
        total: u64,
        unit: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobSnapshot {
    pub id: JobId,
    pub label: String,
    pub state: JobState,
    pub phase: String,
    pub progress: Progress,
    pub result: Option<serde_json::Value>,
    pub error: Option<PwError>,
    pub warnings: Vec<String>,
    pub elapsed_ms: u64,
}

/// Handed to the running job.
pub struct JobContext {
    id: JobId,
    cancel: Arc<AtomicBool>,
    shared: Arc<Shared>,
}

impl JobContext {
    pub fn id(&self) -> JobId {
        self.id
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancel.load(Ordering::Acquire)
    }

    /// Return `Err(Cancelled)` if cancellation was requested. Call at safe boundaries only.
    pub fn check_cancelled(&self) -> PwResult<()> {
        if self.is_cancelled() {
            Err(PwError::cancelled())
        } else {
            Ok(())
        }
    }

    pub fn set_phase(&self, phase: &str) {
        self.shared.update(self.id, |j| j.phase = phase.to_string());
    }

    pub fn set_progress(&self, done: u64, total: u64, unit: &str) {
        let done = done.min(total);
        self.shared.update(self.id, |j| j.progress = Progress::Determinate { done, total, unit: unit.to_string() });
    }

    pub fn set_indeterminate(&self) {
        self.shared.update(self.id, |j| j.progress = Progress::Indeterminate);
    }

    pub fn warn(&self, warning: &str) {
        self.shared.update(self.id, |j| j.warnings.push(warning.to_string()));
    }
}

type Work = Box<dyn FnOnce(&JobContext) -> PwResult<serde_json::Value> + Send + 'static>;
type Listener = Arc<dyn Fn(&JobSnapshot) + Send + Sync>;

struct JobEntry {
    snapshot: JobSnapshot,
    cancel: Arc<AtomicBool>,
    started: Option<Instant>,
    created: Instant,
    dedupe_key: Option<String>,
}

struct Queue {
    pending: VecDeque<(JobId, Work)>,
    shutdown: bool,
}

struct Shared {
    jobs: Mutex<HashMap<JobId, JobEntry>>,
    queue: Mutex<Queue>,
    wake: Condvar,
    changed: Condvar,
    listeners: Mutex<Vec<Listener>>,
}

impl Shared {
    fn update(&self, id: JobId, f: impl FnOnce(&mut JobSnapshot)) {
        let snap = {
            let mut jobs = self.jobs.lock().unwrap_or_else(|p| p.into_inner());
            let Some(entry) = jobs.get_mut(&id) else { return };
            f(&mut entry.snapshot);
            let base = entry.started.unwrap_or(entry.created);
            entry.snapshot.elapsed_ms = base.elapsed().as_millis() as u64;
            entry.snapshot.clone()
        };
        self.changed.notify_all();
        let listeners: Vec<Listener> = self.listeners.lock().unwrap_or_else(|p| p.into_inner()).clone();
        for l in listeners {
            l(&snap);
        }
    }
}

/// Bounded job manager. Dropping it stops workers after their current job.
pub struct JobManager {
    shared: Arc<Shared>,
    workers: Vec<thread::JoinHandle<()>>,
}

impl JobManager {
    pub fn new(max_concurrent: usize) -> Self {
        let max_concurrent = max_concurrent.clamp(1, 64);
        let shared = Arc::new(Shared {
            jobs: Mutex::new(HashMap::new()),
            queue: Mutex::new(Queue { pending: VecDeque::new(), shutdown: false }),
            wake: Condvar::new(),
            changed: Condvar::new(),
            listeners: Mutex::new(Vec::new()),
        });
        let workers = (0..max_concurrent)
            .map(|i| {
                let shared = Arc::clone(&shared);
                thread::Builder::new()
                    .name(format!("pw-worker-{i}"))
                    .spawn(move || worker_loop(shared))
                    .expect("spawn worker thread")
            })
            .collect();
        JobManager { shared, workers }
    }

    /// Register a listener called on every state/progress change (from worker threads).
    pub fn subscribe(&self, f: impl Fn(&JobSnapshot) + Send + Sync + 'static) {
        self.shared.listeners.lock().unwrap_or_else(|p| p.into_inner()).push(Arc::new(f));
    }

    pub fn submit(
        &self,
        label: &str,
        work: impl FnOnce(&JobContext) -> PwResult<serde_json::Value> + Send + 'static,
    ) -> JobId {
        self.submit_inner(label, None, Box::new(work)).0
    }

    /// Submit unless a non-terminal job with the same key exists (duplicate-click protection).
    /// Returns `(id, true)` for a new job or `(existing_id, false)`.
    pub fn submit_unique(
        &self,
        label: &str,
        key: &str,
        work: impl FnOnce(&JobContext) -> PwResult<serde_json::Value> + Send + 'static,
    ) -> (JobId, bool) {
        self.submit_inner(label, Some(key.to_string()), Box::new(work))
    }

    fn submit_inner(&self, label: &str, key: Option<String>, work: Work) -> (JobId, bool) {
        let id = JobId(Uuid::new_v4());
        {
            let mut jobs = self.shared.jobs.lock().unwrap_or_else(|p| p.into_inner());
            if let Some(k) = &key {
                if let Some((existing, _)) =
                    jobs.iter().find(|(_, e)| e.dedupe_key.as_deref() == Some(k) && !e.snapshot.state.is_terminal())
                {
                    return (*existing, false);
                }
            }
            jobs.insert(
                id,
                JobEntry {
                    snapshot: JobSnapshot {
                        id,
                        label: label.to_string(),
                        state: JobState::Queued,
                        phase: "queued".into(),
                        progress: Progress::Indeterminate,
                        result: None,
                        error: None,
                        warnings: Vec::new(),
                        elapsed_ms: 0,
                    },
                    cancel: Arc::new(AtomicBool::new(false)),
                    started: None,
                    created: Instant::now(),
                    dedupe_key: key,
                },
            );
        }
        self.shared.queue.lock().unwrap_or_else(|p| p.into_inner()).pending.push_back((id, work));
        self.shared.wake.notify_one();
        self.shared.update(id, |_| {});
        (id, true)
    }

    /// Request cancellation. Queued jobs are cancelled immediately; running jobs move to
    /// `Cancelling` until they reach a safe boundary. Returns false for unknown or finished jobs.
    pub fn cancel(&self, id: JobId) -> bool {
        let state = {
            let jobs = self.shared.jobs.lock().unwrap_or_else(|p| p.into_inner());
            match jobs.get(&id) {
                Some(e) if !e.snapshot.state.is_terminal() => {
                    e.cancel.store(true, Ordering::Release);
                    e.snapshot.state
                }
                _ => return false,
            }
        };
        if state == JobState::Queued {
            let removed = {
                let mut q = self.shared.queue.lock().unwrap_or_else(|p| p.into_inner());
                let before = q.pending.len();
                q.pending.retain(|(j, _)| *j != id);
                q.pending.len() != before
            };
            if removed {
                self.shared.update(id, |j| {
                    j.state = JobState::Cancelled;
                    j.phase = "cancelled".into();
                    j.error = Some(PwError::cancelled());
                });
                return true;
            }
        }
        self.shared.update(id, |j| {
            if !j.state.is_terminal() {
                j.state = JobState::Cancelling;
            }
        });
        true
    }

    pub fn get(&self, id: JobId) -> Option<JobSnapshot> {
        self.shared.jobs.lock().unwrap_or_else(|p| p.into_inner()).get(&id).map(|e| e.snapshot.clone())
    }

    pub fn list(&self) -> Vec<JobSnapshot> {
        let mut v: Vec<JobSnapshot> =
            self.shared.jobs.lock().unwrap_or_else(|p| p.into_inner()).values().map(|e| e.snapshot.clone()).collect();
        v.sort_by_key(|s| s.id.0);
        v
    }

    /// Block until the job is terminal or `timeout` elapses. For tests and the CLI — never call from a UI thread.
    pub fn wait(&self, id: JobId, timeout: Duration) -> Option<JobSnapshot> {
        let deadline = Instant::now() + timeout;
        let mut jobs = self.shared.jobs.lock().unwrap_or_else(|p| p.into_inner());
        loop {
            match jobs.get(&id) {
                None => return None,
                Some(e) if e.snapshot.state.is_terminal() => return Some(e.snapshot.clone()),
                Some(_) => {}
            }
            let now = Instant::now();
            if now >= deadline {
                return jobs.get(&id).map(|e| e.snapshot.clone());
            }
            jobs = self.shared.changed.wait_timeout(jobs, deadline - now).unwrap_or_else(|p| p.into_inner()).0;
        }
    }

    /// Remove finished jobs from the registry so it stays bounded in long sessions.
    pub fn prune_finished(&self) -> usize {
        let mut jobs = self.shared.jobs.lock().unwrap_or_else(|p| p.into_inner());
        let before = jobs.len();
        jobs.retain(|_, e| !e.snapshot.state.is_terminal());
        before - jobs.len()
    }

    /// Remove one finished job and return its final snapshot; a job still queued or running stays.
    /// Lets each caller clean up after itself without racing others (unlike [`Self::prune_finished`]).
    pub fn remove(&self, id: JobId) -> Option<JobSnapshot> {
        let mut jobs = self.shared.jobs.lock().unwrap_or_else(|p| p.into_inner());
        if jobs.get(&id).is_some_and(|e| e.snapshot.state.is_terminal()) {
            jobs.remove(&id).map(|e| e.snapshot)
        } else {
            None
        }
    }
}

impl Drop for JobManager {
    fn drop(&mut self) {
        {
            let mut q = self.shared.queue.lock().unwrap_or_else(|p| p.into_inner());
            q.shutdown = true;
        }
        let ids: Vec<JobId> = self.shared.jobs.lock().unwrap_or_else(|p| p.into_inner()).keys().copied().collect();
        for id in ids {
            self.cancel(id);
        }
        self.shared.wake.notify_all();
        for w in self.workers.drain(..) {
            let _ = w.join();
        }
    }
}

fn worker_loop(shared: Arc<Shared>) {
    loop {
        let (id, work) = {
            let mut q = shared.queue.lock().unwrap_or_else(|p| p.into_inner());
            loop {
                if q.shutdown {
                    return;
                }
                if let Some(item) = q.pending.pop_front() {
                    break item;
                }
                q = shared.wake.wait(q).unwrap_or_else(|p| p.into_inner());
            }
        };
        let cancel = {
            let mut jobs = shared.jobs.lock().unwrap_or_else(|p| p.into_inner());
            let Some(entry) = jobs.get_mut(&id) else { continue };
            entry.started = Some(Instant::now());
            Arc::clone(&entry.cancel)
        };
        if cancel.load(Ordering::Acquire) {
            shared.update(id, |j| {
                j.state = JobState::Cancelled;
                j.phase = "cancelled".into();
                j.error = Some(PwError::cancelled());
            });
            continue;
        }
        shared.update(id, |j| {
            // A cancel request that raced with dequeueing has already set Cancelling; keep it.
            if j.state == JobState::Queued {
                j.state = JobState::Running;
            }
            j.phase = "running".into();
        });
        let ctx = JobContext { id, cancel, shared: Arc::clone(&shared) };
        // A panicking job must not take down the worker or leave the job non-terminal.
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| work(&ctx)));
        shared.update(id, |j| match outcome {
            Ok(Ok(value)) => {
                j.state = JobState::Succeeded;
                j.phase = "done".into();
                j.result = Some(value);
                if let Progress::Determinate { total, done, .. } = &mut j.progress {
                    *done = *total;
                }
            }
            Ok(Err(e)) if e.category == ErrorCategory::Cancelled => {
                j.state = JobState::Cancelled;
                j.phase = "cancelled".into();
                j.error = Some(e);
            }
            Ok(Err(e)) => {
                j.state = JobState::Failed;
                j.phase = "failed".into();
                j.error = Some(e);
            }
            Err(_) => {
                j.state = JobState::Failed;
                j.phase = "failed".into();
                j.error = Some(PwError::internal("job.panic", "The operation stopped unexpectedly."));
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    use std::sync::mpsc;

    const T: Duration = Duration::from_secs(10);

    #[test]
    fn runs_and_succeeds_with_progress() {
        let jm = JobManager::new(2);
        let id = jm.submit("count", |ctx| {
            for i in 0..=4 {
                ctx.set_progress(i, 4, "pages");
            }
            Ok(serde_json::json!({"pages": 4}))
        });
        let s = jm.wait(id, T).expect("job");
        assert_eq!(s.state, JobState::Succeeded);
        assert_eq!(s.result.expect("result")["pages"], 4);
        assert_eq!(s.progress, Progress::Determinate { done: 4, total: 4, unit: "pages".into() });
    }

    #[test]
    fn failure_is_reported_with_category() {
        let jm = JobManager::new(1);
        let id = jm.submit("bad", |_| Err(PwError::invalid("x.bad", "bad input")));
        let s = jm.wait(id, T).expect("job");
        assert_eq!(s.state, JobState::Failed);
        assert_eq!(s.error.expect("error").code, "x.bad");
    }

    #[test]
    fn cancel_running_job_at_safe_boundary() {
        let jm = JobManager::new(1);
        let (started_tx, started_rx) = mpsc::channel();
        let id = jm.submit("long", move |ctx| {
            started_tx.send(()).ok();
            loop {
                ctx.check_cancelled()?;
                thread::sleep(Duration::from_millis(2));
            }
        });
        started_rx.recv_timeout(T).expect("started");
        assert!(jm.cancel(id));
        let s = jm.wait(id, T).expect("job");
        assert_eq!(s.state, JobState::Cancelled);
        assert!(!jm.cancel(id), "cancelling a finished job reports false");
    }

    #[test]
    fn remove_takes_only_that_finished_job() {
        let jm = JobManager::new(1);
        let (release_tx, release_rx) = mpsc::channel::<()>();
        let running = jm.submit("running", move |_| {
            release_rx.recv_timeout(T).ok();
            Ok(serde_json::Value::Null)
        });
        let done = jm.submit("done", |_| Ok(serde_json::json!(1)));
        let other = jm.submit("other", |_| Ok(serde_json::json!(2)));
        assert!(jm.remove(running).is_none(), "a running job is not removed");
        release_tx.send(()).ok();
        assert_eq!(jm.wait(done, T).expect("done").state, JobState::Succeeded);
        jm.wait(other, T);
        assert_eq!(jm.remove(done).and_then(|s| s.result), Some(serde_json::json!(1)));
        assert!(jm.get(done).is_none());
        assert!(jm.get(other).is_some(), "other finished jobs stay until their owner removes them");
    }

    #[test]
    fn cancel_queued_job_never_runs_it() {
        let jm = JobManager::new(1);
        let (release_tx, release_rx) = mpsc::channel::<()>();
        let blocker = jm.submit("blocker", move |_| {
            release_rx.recv_timeout(T).ok();
            Ok(serde_json::Value::Null)
        });
        let ran = Arc::new(AtomicBool::new(false));
        let ran2 = Arc::clone(&ran);
        let queued = jm.submit("queued", move |_| {
            ran2.store(true, Ordering::SeqCst);
            Ok(serde_json::Value::Null)
        });
        assert!(jm.cancel(queued));
        assert_eq!(jm.get(queued).expect("job").state, JobState::Cancelled);
        release_tx.send(()).ok();
        jm.wait(blocker, T);
        thread::sleep(Duration::from_millis(20));
        assert!(!ran.load(Ordering::SeqCst));
    }

    #[test]
    fn commit_after_cancel_request_reports_truthful_success() {
        let jm = JobManager::new(1);
        let (started_tx, started_rx) = mpsc::channel();
        let (go_tx, go_rx) = mpsc::channel::<()>();
        let id = jm.submit("commit", move |_ctx| {
            started_tx.send(()).ok();
            go_rx.recv_timeout(T).ok();
            // Commit already happened; the job does not observe cancellation afterwards.
            Ok(serde_json::json!("committed"))
        });
        started_rx.recv_timeout(T).expect("started");
        jm.cancel(id);
        assert_eq!(jm.get(id).expect("job").state, JobState::Cancelling);
        go_tx.send(()).ok();
        assert_eq!(jm.wait(id, T).expect("job").state, JobState::Succeeded);
    }

    #[test]
    fn concurrency_is_bounded() {
        let jm = JobManager::new(2);
        let active = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let ids: Vec<JobId> = (0..8)
            .map(|_| {
                let (a, p) = (Arc::clone(&active), Arc::clone(&peak));
                jm.submit("w", move |_| {
                    let now = a.fetch_add(1, Ordering::SeqCst) + 1;
                    p.fetch_max(now, Ordering::SeqCst);
                    thread::sleep(Duration::from_millis(15));
                    a.fetch_sub(1, Ordering::SeqCst);
                    Ok(serde_json::Value::Null)
                })
            })
            .collect();
        for id in ids {
            assert_eq!(jm.wait(id, T).expect("job").state, JobState::Succeeded);
        }
        assert!(peak.load(Ordering::SeqCst) <= 2);
    }

    #[test]
    fn duplicate_submission_returns_existing_job() {
        let jm = JobManager::new(1);
        let (go_tx, go_rx) = mpsc::channel::<()>();
        let (a, new_a) = jm.submit_unique("save", "save:doc1", move |_| {
            go_rx.recv_timeout(T).ok();
            Ok(serde_json::Value::Null)
        });
        let (b, new_b) = jm.submit_unique("save", "save:doc1", |_| Ok(serde_json::Value::Null));
        assert!(new_a && !new_b);
        assert_eq!(a, b);
        go_tx.send(()).ok();
        jm.wait(a, T);
        let (c, new_c) = jm.submit_unique("save", "save:doc1", |_| Ok(serde_json::Value::Null));
        assert!(new_c && c != a, "a finished job no longer blocks a new one");
        jm.wait(c, T);
    }

    #[test]
    fn panicking_job_fails_without_killing_worker() {
        let jm = JobManager::new(1);
        let id = jm.submit("panic", |_| panic!("boom"));
        assert_eq!(jm.wait(id, T).expect("job").state, JobState::Failed);
        let ok = jm.submit("after", |_| Ok(serde_json::Value::Null));
        assert_eq!(jm.wait(ok, T).expect("job").state, JobState::Succeeded);
    }

    #[test]
    fn listeners_see_terminal_state_and_prune_bounds_registry() {
        let jm = JobManager::new(1);
        let (tx, rx) = mpsc::channel();
        let tx = Mutex::new(tx);
        jm.subscribe(move |s| {
            if s.state.is_terminal() {
                tx.lock().expect("lock").send(s.state).ok();
            }
        });
        let id = jm.submit("x", |_| Ok(serde_json::Value::Null));
        assert_eq!(rx.recv_timeout(T).expect("event"), JobState::Succeeded);
        jm.wait(id, T);
        assert_eq!(jm.prune_finished(), 1);
        assert!(jm.list().is_empty());
    }
}
