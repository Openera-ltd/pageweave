// Hide the console window in release builds on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // Worker mode (decision D-015): the app runs each document job in a copy of itself, so a crash
    // inside the PDF engine fails that job instead of closing the window.
    if std::env::args_os().nth(1).is_some_and(|a| a == pw_pdf::worker::WORKER_FLAG) {
        // Same stack as the CLI (Windows gives the main thread 1 MB).
        let h = std::thread::Builder::new()
            .name("pageweave-worker".into())
            .stack_size(16 * 1024 * 1024)
            .spawn(pw_pdf::worker::run_worker);
        if let Ok(h) = h {
            let _ = h.join();
        }
        return;
    }
    // Reader mode (D-018): the viewer's page images and read-only calls, until stdin closes.
    if std::env::args_os().nth(1).is_some_and(|a| a == pw_pdf::service::READER_FLAG) {
        let h = std::thread::Builder::new()
            .name("pageweave-reader".into())
            .stack_size(16 * 1024 * 1024)
            .spawn(pw_pdf::service::run_reader);
        if let Ok(h) = h {
            let _ = h.join();
        }
        return;
    }
    pageweave_app::run()
}
