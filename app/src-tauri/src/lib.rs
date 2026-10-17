//! PageWeave desktop host. Owns application state and exposes a narrow, typed command surface to
//! the WebView UI. All document work runs on the job pool or blocking worker threads, never on the
//! UI/event-loop thread (CLAUDE.md §4.4).

pub mod pages;
pub mod state;

use pw_core::job::JobSnapshot;
use pw_core::ops::{OperationRequest, SCHEMA_VERSION};
use pw_core::settings::{LoadOutcome, Settings};
use pw_core::{fields, PwError};
use serde::Serialize;
use state::{AppState, DocId, OpenedDoc};
use std::path::PathBuf;
use std::sync::OnceLock;
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
    blocking(move || {
        let opts = pw_pdf::OpenOptions { password: doc.password.as_deref() };
        let (hits, truncated) = pw_pdf::search(&doc.path, &opts, &query, match_case, whole_word, &|| false)?;
        Ok(serde_json::json!({ "hits": hits, "truncated": truncated }))
    })
    .await
}

/// Text runs with geometry for one page (selectable text layer). Runs on a worker thread.
#[tauri::command]
async fn page_text(state: State<'_, AppState>, doc_id: DocId, page: u32) -> CmdResult<pw_pdf::PageText> {
    let doc = state.doc(doc_id)?;
    blocking(move || {
        pw_pdf::page_text_segments(&doc.path, &pw_pdf::OpenOptions { password: doc.password.as_deref() }, page)
    })
    .await
}

/// AcroForm fields of an open document. Runs on a worker thread.
#[tauri::command]
async fn form_fields(state: State<'_, AppState>, doc_id: DocId) -> CmdResult<pw_pdf::FormInfo> {
    let doc = state.doc(doc_id)?;
    blocking(move || pw_pdf::form_fields(&doc.path, doc.password.as_deref())).await
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
        let result = pw_pdf::execute(&request, Some(ctx));
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

/// PDF paths passed on the command line (file association / "Open with"), consumed once.
#[tauri::command]
fn launch_files(state: State<'_, AppState>) -> Vec<String> {
    state.take_launch_files()
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
            let state = AppState::new(&config_dir, &data_dir, engine_dir);
            state.set_launch_files(state::launch_args(std::env::args_os().skip(1)));
            let handle = app.handle().clone();
            state.jobs.subscribe(move |snap| {
                let _ = handle.emit("job-update", snap);
            });
            state.logger.log(
                pw_core::log::Level::Info,
                "app",
                "startup",
                None,
                fields! {"version" => env!("CARGO_PKG_VERSION")},
            );
            app.manage(state);
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
            set_unsaved_changes,
            quit_app,
            mark_timing,
            recovered_jobs,
            form_fields,
        ])
        .run(tauri::generate_context!())
        .expect("error while running PageWeave");
}
