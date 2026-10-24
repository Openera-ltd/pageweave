//! PageWeave desktop host. Owns application state and exposes a narrow, typed command surface to
//! the WebView UI. All document work runs on the job pool or blocking worker threads, never on the
//! UI/event-loop thread (CLAUDE.md §4.4).

pub mod pages;
pub mod state;

use pw_core::job::JobSnapshot;
use pw_core::log::Level;
use pw_core::ops::{OperationRequest, SCHEMA_VERSION};
use pw_core::settings::{LoadOutcome, Settings};
use pw_core::{fields, PwError};
use pw_pdf::service::ReadCall;
use serde::Serialize;
use state::{AppState, DocId, OpenedDoc};
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::time::Instant;
use tauri::{Emitter, Manager, State};

type CmdResult<T> = Result<T, PwError>;

/// Set as the first thing in `run()`; startup milestones are measured from here.
static HOST_START: OnceLock<Instant> = OnceLock::new();

/// Startup milestones the UI may report (anything else is rejected).
pub const TIMING_MARKS: [&str; 2] = ["ui_ready", "first_page"];

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> CmdResult<T> + Send + 'static) -> CmdResult<T> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|_| PwError::internal("worker.join", "A background task stopped unexpectedly."))?
}

#[derive(Serialize)]
pub struct AppInfo {
    version: &'static str,
    engine: &'static str,
    schema_version: u32,
    settings_status: LoadOutcome,
    engine_ready: bool,
    engine_error: Option<PwError>,
    platform: &'static str,
}

#[tauri::command]
fn app_info(state: State<'_, AppState>) -> AppInfo {
    let engine = pw_pdf::init(state.engine_dir.as_deref());
    AppInfo {
        version: env!("CARGO_PKG_VERSION"),
        engine: pw_pdf::engine_version(),
        schema_version: SCHEMA_VERSION,
        settings_status: state.settings_outcome.clone(),
        engine_ready: engine.is_ok(),
        engine_error: engine.err(),
        platform: std::env::consts::OS,
    }
}

#[tauri::command]
fn get_settings(state: State<'_, AppState>) -> Settings {
    state.settings()
}

/// Validate and persist settings. Invalid fields are rejected rather than silently changed.
#[tauri::command]
fn save_settings(state: State<'_, AppState>, settings: serde_json::Value) -> CmdResult<Settings> {
    let (s, bad) = Settings::from_value(&settings);
    if !bad.is_empty() {
        return Err(PwError::invalid("settings.invalid", format!("Invalid setting: {}.", bad.join(", "))));
    }
    state.save_settings(s)
}

#[tauri::command]
async fn open_document(
    state: State<'_, AppState>,
    path: PathBuf,
    password: Option<String>,
    remember: Option<bool>,
) -> CmdResult<OpenedDoc> {
    let st = state.inner().clone();
    blocking(move || st.open_with(path, password, remember.unwrap_or(true))).await
}

#[tauri::command]
fn close_document(state: State<'_, AppState>, doc_id: DocId) -> bool {
    state.close(doc_id)
}

/// Validate a page-range expression against an open document (single implementation in pw-core).
#[tauri::command]
fn validate_range(state: State<'_, AppState>, doc_id: DocId, spec: String) -> CmdResult<Vec<u32>> {
    let doc = state.doc(doc_id)?;
    pw_core::range::parse_page_ranges(&spec, doc.info.page_count)
}

#[tauri::command]
async fn search(
    state: State<'_, AppState>,
    doc_id: DocId,
    query: String,
    match_case: bool,
    whole_word: bool,
) -> CmdResult<serde_json::Value> {
    let doc = state.doc(doc_id)?;
    let st = state.inner().clone();
    blocking(move || {
        st.read_json(
            &ReadCall::Search { path: doc.path.clone(), query, match_case, whole_word },
            doc.password.as_deref(),
        )
    })
    .await
}

/// A read-only call for an open document, in a reader process (D-018), off the UI thread.
async fn read(
    state: &State<'_, AppState>,
    doc_id: DocId,
    call: impl FnOnce(PathBuf) -> ReadCall + Send + 'static,
) -> CmdResult<serde_json::Value> {
    let doc = state.doc(doc_id)?;
    let st = state.inner().clone();
    blocking(move || st.read_json(&call(doc.path.clone()), doc.password.as_deref())).await
}

/// Text runs with geometry for one page (selectable text layer). Runs on a worker thread.
#[tauri::command]
async fn page_text(state: State<'_, AppState>, doc_id: DocId, page: u32) -> CmdResult<serde_json::Value> {
    read(&state, doc_id, move |path| ReadCall::PageText { path, page }).await
}

/// AcroForm fields of an open document. Runs on a worker thread.
#[tauri::command]
async fn form_fields(state: State<'_, AppState>, doc_id: DocId) -> CmdResult<serde_json::Value> {
    read(&state, doc_id, |path| ReadCall::FormFields { path }).await
}

/// What redacting `areas` of an open document would remove (read-only). Runs on a worker thread.
#[tauri::command]
async fn redaction_preview(
    state: State<'_, AppState>,
    doc_id: DocId,
    areas: Vec<pw_core::ops::RedactArea>,
) -> CmdResult<serde_json::Value> {
    let doc = state.doc(doc_id)?;
    // Same validation as the engine operation (area count and geometry).
    OperationRequest::new(pw_core::ops::Operation::RedactionPreview { input: doc.path.clone(), areas: areas.clone() })
        .validate()?;
    read(&state, doc_id, move |path| ReadCall::RedactionPreview { path, areas }).await
}

/// Files embedded in an open document (read-only). Runs on a worker thread.
#[tauri::command]
async fn list_attachments(state: State<'_, AppState>, doc_id: DocId) -> CmdResult<serde_json::Value> {
    read(&state, doc_id, |path| ReadCall::ListAttachments { path }).await
}

/// Run one job in a worker copy of this executable (decision D-015), so a crash inside the PDF
/// engine fails this job instead of closing the app. Progress and cancellation pass through; a
/// crashed worker's staged files are removed at once.
fn run_job(
    request: &OperationRequest,
    ctx: &pw_core::job::JobContext,
) -> pw_core::PwResult<pw_core::ops::OperationResult> {
    let exe = std::env::current_exe().map_err(|e| pw_core::PwError::from_io(&e, "application file"))?;
    let (pid, result) = pw_pdf::worker::run_in_worker(&exe, request, &|| ctx.is_cancelled(), &mut |e| match e {
        pw_pdf::worker::WorkerEvent::Progress(pw_core::job::Progress::Determinate { done, total, unit }) => {
            ctx.set_progress(done, total, &unit)
        }
        pw_pdf::worker::WorkerEvent::Progress(pw_core::job::Progress::Indeterminate) => ctx.set_indeterminate(),
        pw_pdf::worker::WorkerEvent::Phase(p) => ctx.set_phase(&p),
    });
    if let (Some(pid), Err(_)) = (pid, &result) {
        if let Some(dir) = request.operation.output_dir() {
            // Only this worker's files: other jobs may be writing next to them.
            let prefix = pw_core::output::worker_staged_prefix(std::process::id(), pid);
            if let Ok(entries) = std::fs::read_dir(&dir) {
                for e in entries.flatten() {
                    if e.file_name().to_string_lossy().starts_with(&prefix) {
                        let p = e.path();
                        let _ = if p.is_dir() { std::fs::remove_dir_all(&p) } else { std::fs::remove_file(&p) };
                    }
                }
            }
        }
    }
    result
}

/// The language of the open document's first pages, from its reading-order text (None: unclear).
#[tauri::command]
async fn detect_language(state: State<'_, AppState>, doc_id: DocId) -> CmdResult<Option<String>> {
    let doc = state.doc(doc_id)?;
    let st = state.inner().clone();
    let pages = doc.info.page_count.min(3);
    blocking(move || {
        let mut text = String::new();
        for page in 0..pages {
            let v = st.read_json(&ReadCall::ReadingText { path: doc.path.clone(), page }, doc.password.as_deref())?;
            text.push_str(v["text"].as_str().unwrap_or(""));
            text.push('\n');
        }
        Ok(pw_translate::detect::detect(&text).map(|(l, _)| l.to_string()))
    })
    .await
}

/// Translation directions and whether each model is installed (D-022).
#[tauri::command]
async fn translation_directions() -> CmdResult<Vec<pw_translate::Direction>> {
    blocking(pw_translate::directions).await
}

/// The computer's speech engine and voices (read aloud, D-021).
#[tauri::command]
async fn speech_info() -> CmdResult<pw_speech::EngineInfo> {
    blocking(pw_speech::engine_info).await
}

/// One page's text in reading order with word positions (read aloud). Runs in a reader process.
#[tauri::command]
async fn reading_text(state: State<'_, AppState>, doc_id: DocId, page: u32) -> CmdResult<serde_json::Value> {
    read(&state, doc_id, move |path| ReadCall::ReadingText { path, page }).await
}

#[derive(Serialize)]
pub struct Spoken {
    /// A WAV file as a data URL, for an audio element.
    audio: String,
    duration_ms: u32,
    sentences: Vec<pw_speech::Mark>,
    words: Vec<pw_speech::Mark>,
    precision: pw_speech::Precision,
}

/// Speak a piece of text (a paragraph) with the system voice. No PDF is parsed here.
#[tauri::command]
async fn speak_text(text: String, voice: Option<String>, rate: f32) -> CmdResult<Spoken> {
    use base64::Engine as _;
    blocking(move || {
        let s = pw_speech::speak(&text, &pw_speech::SpeakOptions { voice, rate }, &|| false)?;
        let wav = pw_speech::wav::encode(s.sample_rate, &s.samples);
        Ok(Spoken {
            audio: format!("data:audio/wav;base64,{}", base64::engine::general_purpose::STANDARD.encode(wav)),
            duration_ms: s.duration_ms(),
            sentences: s.sentences,
            words: s.words,
            precision: s.precision,
        })
    })
    .await
}

/// The LibreOffice installed on this computer, for opening Office files (D-019).
#[tauri::command]
async fn office_engine() -> CmdResult<Option<String>> {
    blocking(|| Ok(pw_pdf::office::find_office().map(|e| e.version))).await
}

#[derive(Serialize)]
pub struct OfficePackageInfo {
    version: String,
    bytes: u64,
    /// PageWeave can download and unpack it here (Windows).
    can_install: bool,
}

/// The LibreOffice package PageWeave downloads when Office files should open.
#[tauri::command]
fn office_package() -> CmdResult<OfficePackageInfo> {
    let p = pw_pdf::office::office_package()?;
    Ok(OfficePackageInfo { version: p.version, bytes: p.bytes, can_install: cfg!(windows) })
}

/// Installed resources checked against their pinned checksums (read-only; reads ~60 MB once).
#[tauri::command]
async fn resource_status() -> CmdResult<pw_pdf::resources::ResourceReport> {
    blocking(pw_pdf::resources::resource_status).await
}

/// Comments and markup already in the open document (read-only).
#[tauri::command]
async fn list_annotations(state: State<'_, AppState>, doc_id: DocId) -> CmdResult<serde_json::Value> {
    read(&state, doc_id, |path| ReadCall::ListAnnotations { path }).await
}

/// The open document's digital signatures, checked against the trusted certificates (read-only).
/// Runs on a worker thread.
#[tauri::command]
async fn verify_signatures(state: State<'_, AppState>, doc_id: DocId) -> CmdResult<serde_json::Value> {
    let trusted = trusted_files(&state.trust_dir);
    read(&state, doc_id, move |path| ReadCall::VerifySignatures { path, trusted }).await
}

/// The editable text runs and pictures of one page of an open document (read-only).
#[tauri::command]
async fn page_objects(state: State<'_, AppState>, doc_id: DocId, page: u32) -> CmdResult<serde_json::Value> {
    read(&state, doc_id, move |path| ReadCall::PageObjects { path, page }).await
}

/// One page with pending object edits applied, as a PNG data URL (nothing is written).
#[tauri::command]
async fn preview_page_edits(
    state: State<'_, AppState>,
    doc_id: DocId,
    page: u32,
    edits: Vec<pw_core::ops::PageEdit>,
    scale_permille: u32,
) -> CmdResult<String> {
    use base64::Engine as _;
    let doc = state.doc(doc_id)?;
    let st = state.inner().clone();
    blocking(move || {
        let call =
            ReadCall::PreviewEdits { path: doc.path.clone(), page, edits, scale: scale_permille as f32 / 1000.0 };
        let png = st.read_bytes(&call, doc.password.as_deref())?;
        Ok(format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(png)))
    })
    .await
}

/// A certificate the user trusts.
#[derive(serde::Serialize)]
struct TrustedCert {
    fingerprint: String,
    name: String,
    subject: String,
    valid_to: String,
}

fn trusted_files(dir: &std::path::Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|rd| rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "der")).collect())
        .unwrap_or_default();
    files.sort();
    files
}

fn trusted_entry(der: &[u8]) -> Option<TrustedCert> {
    let c = pw_pdf::parse_certificates(der).into_iter().next()?;
    let chain_name = c.tbs_certificate.subject.to_string();
    let name = c
        .tbs_certificate
        .subject
        .0
        .iter()
        .flat_map(|rdn| rdn.0.iter())
        .find(|a| a.oid.to_string() == "2.5.4.3")
        .and_then(|a| a.value.decode_as::<String>().ok())
        .unwrap_or_else(|| chain_name.clone());
    Some(TrustedCert {
        fingerprint: pw_pdf::fingerprint(&c),
        name,
        subject: chain_name,
        valid_to: c.tbs_certificate.validity.not_after.to_date_time().to_string(),
    })
}

/// File name for a fingerprint ("AB:CD:…" → "ABCD….der"); None for anything else.
fn trust_file_name(fingerprint: &str) -> Option<String> {
    let hex: String = fingerprint.chars().filter(|c| *c != ':').collect();
    (hex.len() == 64 && hex.chars().all(|c| c.is_ascii_hexdigit())).then(|| format!("{}.der", hex.to_ascii_uppercase()))
}

/// Certificates the user trusts for signatures.
#[tauri::command]
async fn trusted_certificates(state: State<'_, AppState>) -> CmdResult<Vec<TrustedCert>> {
    let dir = state.trust_dir.clone();
    blocking(move || {
        Ok(trusted_files(&dir).iter().filter_map(|p| std::fs::read(p).ok()).filter_map(|d| trusted_entry(&d)).collect())
    })
    .await
}

/// The certificate chain of one signature of an open document (to choose what to trust).
#[tauri::command]
async fn signature_chain(
    state: State<'_, AppState>,
    doc_id: DocId,
    index: usize,
) -> CmdResult<Vec<pw_pdf::ChainCertificate>> {
    let doc = state.doc(doc_id)?;
    blocking(move || pw_pdf::signature_chain(&doc.path, index)).await
}

/// Trust one certificate of a signature's chain (position 0 = the signer).
#[tauri::command]
async fn trust_certificate(
    state: State<'_, AppState>,
    doc_id: DocId,
    index: usize,
    position: usize,
) -> CmdResult<TrustedCert> {
    let doc = state.doc(doc_id)?;
    let dir = state.trust_dir.clone();
    let logger = state.logger.clone();
    blocking(move || {
        let chain = pw_pdf::signature_chain(&doc.path, index)?;
        let c = chain
            .get(position)
            .ok_or_else(|| PwError::invalid("trust.position", "That certificate is not in the chain."))?;
        let file = trust_file_name(&c.fingerprint)
            .ok_or_else(|| PwError::internal("trust.fingerprint", "Bad fingerprint."))?;
        std::fs::create_dir_all(&dir).map_err(|e| PwError::from_io(&e, "trusted certificates folder"))?;
        let tmp = dir.join(format!("{file}.tmp"));
        std::fs::write(&tmp, &c.der).map_err(|e| PwError::from_io(&e, "trusted certificate"))?;
        std::fs::rename(&tmp, dir.join(&file)).map_err(|e| PwError::from_io(&e, "trusted certificate"))?;
        logger.log(Level::Info, "trust", "certificate_added", None, fields! {"is_ca" => c.is_ca});
        trusted_entry(&c.der)
            .ok_or_else(|| PwError::internal("trust.reread", "The certificate could not be read back."))
    })
    .await
}

/// Stop trusting a certificate.
#[tauri::command]
async fn remove_trusted_certificate(state: State<'_, AppState>, fingerprint: String) -> CmdResult<bool> {
    let file = trust_file_name(&fingerprint)
        .ok_or_else(|| PwError::invalid("trust.fingerprint", "Not a certificate fingerprint."))?;
    let path = state.trust_dir.join(file);
    let logger = state.logger.clone();
    blocking(move || match std::fs::remove_file(&path) {
        Ok(()) => {
            logger.log(Level::Info, "trust", "certificate_removed", None, fields! {});
            Ok(true)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(PwError::from_io(&e, "trusted certificate")),
    })
    .await
}

/// What hidden information an open document holds (read-only). Runs on a worker thread.
#[tauri::command]
async fn sanitize_inspect(state: State<'_, AppState>, doc_id: DocId) -> CmdResult<serde_json::Value> {
    read(&state, doc_id, |path| ReadCall::SanitizeInspect { path }).await
}

/// Start an operation as a job. When `doc_id` is given, its password is supplied by the host so
/// the UI never has to hold it. Returns the job ID; progress arrives as `job-update` events.
#[tauri::command]
fn start_operation(
    state: State<'_, AppState>,
    request: OperationRequest,
    doc_id: Option<DocId>,
) -> CmdResult<pw_core::job::JobId> {
    request.validate()?;
    let mut request = request;
    if let Some(id) = doc_id {
        request.password = state.doc(id)?.password.clone();
    }
    let label = request.operation.id().to_string();
    let key = serde_json::to_string(&request.operation).unwrap_or_default();
    let logger = state.logger.clone();
    let journal = state.journal.clone();
    let (id, _new) = state.jobs.submit_unique(&label, &key, move |ctx| {
        let started = std::time::Instant::now();
        // Kept until the job ends in any way; left behind only if the process dies mid-job.
        let _journal = match (&journal, request.operation.writes_output()) {
            (Some(j), true) => match j.begin(&ctx.id().to_string(), request.operation.id(), &request) {
                Ok(g) => Some(g),
                Err(e) => {
                    logger.log(
                        pw_core::log::Level::Warn,
                        "job",
                        "journal_write_failed",
                        Some(&ctx.id().to_string()),
                        fields! {"error_code" => e.code},
                    );
                    None
                }
            },
            _ => None,
        };
        let result = run_job(&request, ctx);
        logger.log(
            pw_core::log::Level::Info,
            "job",
            "operation.finished",
            Some(&ctx.id().to_string()),
            fields! {
                "operation" => request.operation.id(),
                "ok" => result.is_ok(),
                "error_code" => result.as_ref().err().map(|e| e.code.clone()),
                "ms" => started.elapsed().as_millis() as u64,
            },
        );
        result.map(|r| serde_json::to_value(r).unwrap_or_default())
    });
    Ok(id)
}

#[tauri::command]
fn cancel_job(state: State<'_, AppState>, job_id: pw_core::job::JobId) -> bool {
    state.jobs.cancel(job_id)
}

#[tauri::command]
fn list_jobs(state: State<'_, AppState>) -> Vec<JobSnapshot> {
    state.jobs.list()
}

/// Jobs that were interrupted when PageWeave last closed (consumed once).
#[tauri::command]
fn recovered_jobs(state: State<'_, AppState>) -> Vec<pw_core::journal::RecoveredJob> {
    state.take_recovered()
}

/// Documents currently open as tabs, so the UI can restore them after its page is reloaded.
#[tauri::command]
fn open_documents(state: State<'_, AppState>) -> Vec<OpenedDoc> {
    state.open_documents()
}

/// PDF paths passed on the command line (file association / "Open with") not yet handled by the UI.
#[tauri::command]
fn launch_files(state: State<'_, AppState>) -> Vec<String> {
    state.pending_launch_files()
}

/// The UI has handled one command-line file; it is not offered again (e.g. after a page reload).
#[tauri::command]
fn launch_file_done(state: State<'_, AppState>, path: String) {
    state.launch_file_done(&path);
}

#[tauri::command]
fn log_dir(state: State<'_, AppState>) -> String {
    state.log_dir.to_string_lossy().into_owned()
}

/// Record a startup milestone (milliseconds since the host started) in the diagnostic log, once per
/// run. Returns the elapsed time, or null when the milestone was already recorded.
#[tauri::command]
fn mark_timing(state: State<'_, AppState>, name: String) -> CmdResult<Option<u64>> {
    if !TIMING_MARKS.contains(&name.as_str()) {
        return Err(PwError::invalid("timing.unknown", "Unknown timing mark."));
    }
    if !state.first_mark(&name) {
        return Ok(None);
    }
    let ms = HOST_START.get().map(|t| t.elapsed().as_millis() as u64).unwrap_or(0);
    state.logger.log(
        pw_core::log::Level::Info,
        "app",
        "timing",
        None,
        fields! {"mark" => name, "since_start_ms" => ms},
    );
    Ok(Some(ms))
}

#[derive(Serialize)]
pub struct UpdateInfo {
    current: &'static str,
    /// This build carries the release key, so it can install updates itself.
    can_install: bool,
    /// Updates install on Windows; elsewhere the app only reports them.
    platform_supported: bool,
}

#[tauri::command]
fn update_info() -> UpdateInfo {
    UpdateInfo {
        current: env!("CARGO_PKG_VERSION"),
        can_install: pw_update::can_install(),
        platform_supported: cfg!(windows),
    }
}

/// Read this project's latest release information (nothing about the user is sent).
#[tauri::command]
async fn check_for_update(state: State<'_, AppState>) -> CmdResult<Option<pw_update::Update>> {
    let logger = state.logger.clone();
    blocking(move || {
        let r = pw_update::check(env!("CARGO_PKG_VERSION"));
        match &r {
            Ok(u) => logger.log(
                pw_core::log::Level::Info,
                "update",
                "checked",
                None,
                fields! {"available" => u.as_ref().map(|u| u.version.clone()).unwrap_or_default()},
            ),
            Err(e) => logger.log(
                pw_core::log::Level::Warn,
                "update",
                "check_failed",
                None,
                fields! {"code" => e.code.clone()},
            ),
        }
        r
    })
    .await
}

/// Run the downloaded, verified installer for `version` as an update and close PageWeave; the
/// installer starts it again. Refused while there are unsaved edits.
#[tauri::command]
async fn install_update(state: State<'_, AppState>, app: tauri::AppHandle, version: String) -> CmdResult<()> {
    if state.unsaved() > 0 {
        return Err(PwError::new(
            pw_core::ErrorCategory::OutputConflict,
            "update.unsaved",
            "Save or discard your unsaved changes before updating.",
        ));
    }
    if !cfg!(windows) {
        return Err(PwError::new(
            pw_core::ErrorCategory::UnsupportedOperation,
            "update.platform",
            "Updates install on Windows only.",
        ));
    }
    let v = version.clone();
    let (exe, args) = blocking(move || pw_update::prepared_installer(&v)).await?;
    state.logger.log(pw_core::log::Level::Info, "update", "install", None, fields! {"version" => version});
    std::process::Command::new(&exe).args(&args).spawn().map_err(|e| PwError::from_io(&e, "update installer"))?;
    app.exit(0);
    Ok(())
}

/// Open this project's releases page in the browser (a fixed address).
#[tauri::command]
fn open_releases_page(app: tauri::AppHandle) -> CmdResult<()> {
    use tauri_plugin_opener::OpenerExt;
    app.opener()
        .open_url("https://github.com/kuyamcliff/pageweave/releases/latest", None::<&str>)
        .map_err(|_| PwError::internal("open.url", "The releases page could not be opened."))
}

/// The UI reports how many unsaved edits exist so the window-close guard can hold back a close.
#[tauri::command]
fn set_unsaved_changes(state: State<'_, AppState>, count: u32) {
    state.set_unsaved(count);
}

/// Close the window after the user chose to discard unsaved edits.
#[tauri::command]
fn quit_app(state: State<'_, AppState>, window: tauri::Window) -> CmdResult<()> {
    state.set_unsaved(0);
    window.destroy().map_err(|_| PwError::internal("window.close", "The window could not be closed."))
}

/// wry's WebView2 defaults (see wry `create_environment`): kept whenever arguments are supplied.
const WRY_DEFAULT_WEBVIEW2_ARGS: &str =
    "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection --autoplay-policy=no-user-gesture-required";

/// WebView2 browser arguments for the main window. wry always passes its own arguments, and on
/// current WebView2 runtimes those take precedence over the standard `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS`
/// variable, so the variable (used by Edge WebDriver and other tooling) silently does nothing.
/// When it is set, append it to wry's defaults; when unset, return None and change nothing.
pub fn webview2_browser_args(env_value: Option<&str>) -> Option<String> {
    let extra = env_value.map(str::trim).filter(|v| !v.is_empty())?;
    Some(format!("{WRY_DEFAULT_WEBVIEW2_ARGS} {extra}"))
}

/// Whether a window close request must wait for the user (unsaved edits exist).
pub fn close_needs_confirmation(state: &AppState) -> bool {
    state.unsaved() > 0
}

pub fn run() {
    HOST_START.get_or_init(Instant::now);
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .register_asynchronous_uri_scheme_protocol("pwpage", |ctx, request, responder| {
            let state = ctx.app_handle().state::<AppState>().inner().clone();
            let uri = request.uri().clone();
            std::thread::spawn(move || responder.respond(pages::serve(&state, &uri)));
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if close_needs_confirmation(&window.state::<AppState>()) {
                    api.prevent_close();
                    let _ = window.emit("close-requested", ());
                }
            }
        })
        .setup(|app| {
            let config_dir = app.path().app_config_dir()?;
            let data_dir = app.path().app_local_data_dir()?;
            let engine_dir = app.path().resource_dir().ok();
            let mut state = AppState::new(&config_dir, &data_dir, engine_dir);
            // Page images and read-only calls run in reader processes (D-018): two, so the viewer
            // can render while another call is busy.
            if let Ok(exe) = std::env::current_exe() {
                state.readers = Some(Arc::new(pw_pdf::service::ReaderPool::new(exe, 2)));
            }
            let launch = state::launch_args(std::env::args_os().skip(1));
            let launch_count = launch.len();
            state.set_launch_files(launch);
            let handle = app.handle().clone();
            state.jobs.subscribe(move |snap| {
                let _ = handle.emit("job-update", snap);
            });
            state.logger.log(
                pw_core::log::Level::Info,
                "app",
                "startup",
                None,
                fields! {"version" => env!("CARGO_PKG_VERSION"), "launch_files" => launch_count},
            );
            app.manage(state);
            // Installers left from an update that has finished are removed.
            std::thread::spawn(|| pw_update::clean(env!("CARGO_PKG_VERSION")));
            // Translation models of earlier versions that no direction uses any more.
            std::thread::spawn(pw_translate::install::prune);
            // The main window is created here (config has `create: false`) so WebView2 arguments
            // can be supplied; everything else comes from tauri.conf.json.
            let config = app
                .config()
                .app
                .windows
                .iter()
                .find(|w| w.label == "main")
                .cloned()
                .ok_or("main window missing from tauri.conf.json")?;
            #[allow(unused_mut)]
            let mut builder = tauri::WebviewWindowBuilder::from_config(app.handle(), &config)?;
            #[cfg(windows)]
            if let Some(args) =
                webview2_browser_args(std::env::var("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS").ok().as_deref())
            {
                builder = builder.additional_browser_args(&args);
            }
            builder.build()?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            app_info,
            update_info,
            check_for_update,
            install_update,
            open_releases_page,
            office_engine,
            office_package,
            speech_info,
            translation_directions,
            detect_language,
            reading_text,
            speak_text,
            get_settings,
            save_settings,
            open_document,
            close_document,
            validate_range,
            search,
            page_text,
            start_operation,
            cancel_job,
            list_jobs,
            log_dir,
            launch_files,
            launch_file_done,
            set_unsaved_changes,
            quit_app,
            mark_timing,
            recovered_jobs,
            form_fields,
            redaction_preview,
            sanitize_inspect,
            list_attachments,
            list_annotations,
            resource_status,
            verify_signatures,
            trusted_certificates,
            page_objects,
            preview_page_edits,
            signature_chain,
            trust_certificate,
            remove_trusted_certificate,
            open_documents,
        ])
        .run(tauri::generate_context!())
        .expect("error while running PageWeave");
}
