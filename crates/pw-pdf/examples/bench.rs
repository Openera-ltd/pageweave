//! Engine performance baseline (PRD 12). Prints one JSON object with p50/p95 timings and peak RSS.
//!
//! Usage: BENCH_PDFIUM_DIR=third_party/pdfium/linux-x64/lib cargo run --release -p pw-pdf --example bench -- <small.pdf> <large.pdf> <out-dir> [iterations]
//! Numbers are environment-specific; record the machine with every result. Not a pass/fail test.

use pw_core::ops::{Operation, OperationRequest};
use pw_pdf::{execute, open_document, render_page_png, search, OpenOptions};
use std::path::PathBuf;
use std::time::Instant;

fn pct(mut v: Vec<f64>, p: f64) -> f64 {
    v.sort_by(|a, b| a.total_cmp(b));
    let i = ((v.len() as f64 - 1.0) * p).round() as usize;
    v[i.min(v.len() - 1)]
}

fn measure(n: usize, mut f: impl FnMut()) -> serde_json::Value {
    let mut t = Vec::with_capacity(n);
    for _ in 0..n {
        let s = Instant::now();
        f();
        t.push(s.elapsed().as_secs_f64() * 1000.0);
    }
    serde_json::json!({ "p50_ms": (pct(t.clone(), 0.5) * 100.0).round() / 100.0, "p95_ms": (pct(t.clone(), 0.95) * 100.0).round() / 100.0, "n": n })
}

fn peak_rss_kb() -> Option<u64> {
    let s = std::fs::read_to_string("/proc/self/status").ok()?;
    s.lines().find(|l| l.starts_with("VmHWM:"))?.split_whitespace().nth(1)?.parse().ok()
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let small = PathBuf::from(&args[1]);
    let large = PathBuf::from(&args[2]);
    let out = PathBuf::from(&args[3]);
    let n: usize = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(20);
    let o = OpenOptions::default();
    let init = Instant::now();
    let dir = std::env::var_os("BENCH_PDFIUM_DIR").map(PathBuf::from);
    pw_pdf::init(dir.as_deref()).expect("pdfium (set BENCH_PDFIUM_DIR in release builds)");
    let init_ms = init.elapsed().as_secs_f64() * 1000.0;
    let large_pages = open_document(&large, &o).expect("open large").page_count;
    let mut k = 0u32;
    let result = serde_json::json!({
        "engine": pw_pdf::engine_version(),
        "engine_init_ms": (init_ms * 100.0).round() / 100.0,
        "large_pages": large_pages,
        "open_inspect_small": measure(n, || { open_document(&small, &o).expect("open"); }),
        "open_inspect_large": measure(n.min(10), || { open_document(&large, &o).expect("open"); }),
        "render_first_page_scale_1_5": measure(n, || { render_page_png(&small, &o, 0, 1.5).expect("render"); }),
        "render_first_page_scale_3": measure(n, || { render_page_png(&small, &o, 0, 3.0).expect("render"); }),
        "render_last_page_large_scale_1_5": measure(n, || { render_page_png(&large, &o, large_pages - 1, 1.5).expect("render"); }),
        "search_large": measure(n.min(10), || { search(&large, &o, "PWMARK", false, false, &|| false).expect("search"); }),
        "extract_100_pages_from_large": measure(n.min(10), || {
            k += 1;
            let mut r = OperationRequest::new(Operation::ExtractPages { input: large.clone(), pages: "1-100".into(), output: out.join(format!("x{k}.pdf")) });
            r.collision = pw_core::output::CollisionPolicy::Replace;
            execute(&r, None).expect("extract");
        }),
        "peak_rss_kb": peak_rss_kb(),
    });
    println!("{}", serde_json::to_string_pretty(&result).unwrap_or_default());
}
