//! Jobs in a worker process (D-015): results, progress, passwords, cancellation, and a crash in
//! the PDF engine that fails the job instead of the caller.

use pw_core::ops::{Operation, OperationRequest, RectPt, RedactArea};
use pw_core::ErrorCategory;
use pw_pdf::worker::{run_in_worker, WorkerEvent};
use std::path::{Path, PathBuf};

fn exe() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_pageweave"))
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures").join(name)
}

#[test]
fn a_job_runs_in_the_worker_with_its_password_and_returns_the_result() {
    let dir = tempfile::tempdir().expect("tmp");
    let out = dir.path().join("x.pdf");
    let mut req = OperationRequest::new(Operation::ExtractPages {
        input: fixture("pdf/encrypted-rc4-user-password.pdf"),
        pages: "2".into(),
        output: out.clone(),
    });
    req.password = Some("pw-user-test".into());
    let mut events = 0;
    let (pid, r) = run_in_worker(&exe(), &req, &|| false, &mut |_e| events += 1);
    assert!(pid.is_some());
    let r = r.expect("result");
    assert_eq!(r.operation, "extract_pages");
    assert!(out.exists());
    // Without the password the worker reports the typed error.
    let mut req2 = req.clone();
    req2.password = None;
    if let Operation::ExtractPages { output, .. } = &mut req2.operation {
        *output = dir.path().join("y.pdf");
    }
    let (_, e) = run_in_worker(&exe(), &req2, &|| false, &mut |_| {});
    assert_eq!(e.expect_err("needs password").category, ErrorCategory::PasswordRequired);
}

#[test]
fn an_engine_crash_fails_only_the_job() {
    let dir = tempfile::tempdir().expect("tmp");
    let out = dir.path().join("r.pdf");
    let req = OperationRequest::new(Operation::Redact {
        input: fixture("hostile/redact-generate-content-crash.pdf"),
        areas: vec![RedactArea { page: 0, rect: RectPt { left: 50.0, bottom: 600.0, right: 400.0, top: 760.0 } }],
        output: out.clone(),
    });
    let (_, r) = run_in_worker(&exe(), &req, &|| false, &mut |_| {});
    let e = r.expect_err("the worker crashed");
    assert_eq!(e.code, "engine.crashed");
    assert!(e.message.contains("nothing was saved"));
    assert!(!out.exists());
    // This process is still here and can run the next job.
    let (_, ok) = run_in_worker(
        &exe(),
        &OperationRequest::new(Operation::Inspect { input: fixture("pdf/five-pages.pdf") }),
        &|| false,
        &mut |_| {},
    );
    assert_eq!(ok.expect("inspect").data["page_count"], 5);
}

#[test]
fn progress_arrives_and_cancel_stops_the_worker() {
    if pw_ocr::default_resource_dir().is_none() {
        assert!(std::env::var_os("CI").is_none(), "OCR resources missing in CI");
        return;
    }
    let dir = tempfile::tempdir().expect("tmp");
    let out = dir.path().join("o.pdf");
    let req = OperationRequest::new(Operation::OcrPages {
        input: fixture("pdf/scanned.pdf"),
        pages: None,
        skip_text_pages: true,
        dpi: 200,
        output: out.clone(),
    });
    let seen = std::cell::Cell::new(false);
    let (_, r) = run_in_worker(&exe(), &req, &|| seen.get(), &mut |e| {
        if matches!(e, WorkerEvent::Progress(_)) {
            seen.set(true);
        }
    });
    assert!(seen.get(), "progress was reported");
    assert_eq!(r.expect_err("cancelled").category, ErrorCategory::Cancelled);
    assert!(!out.exists(), "nothing committed after cancel");
}

/// Same mutations as pw-pdf's robustness test, through write operations in the worker: every case
/// ends in a verified copy or a typed error (a native crash becomes `engine.crashed`), and no
/// output or staged file is left behind on failure.
#[test]
fn mutated_files_through_write_operations_in_the_worker() {
    use pw_core::ops::{LabelStyle, PageLabelRange, SanitizeOptions};
    let cases: usize = std::env::var("PW_FUZZ_CASES").ok().and_then(|v| v.parse().ok()).unwrap_or(2);
    let mut seed: u64 = std::env::var("PW_FUZZ_SEED").ok().and_then(|v| v.parse().ok()).unwrap_or(0xfeed_beef);
    let mut next = move || {
        seed ^= seed >> 12;
        seed ^= seed << 25;
        seed ^= seed >> 27;
        seed.wrapping_mul(0x2545_F491_4F6C_DD1D)
    };
    let dir = tempfile::tempdir().expect("tmp");
    let fixtures: Vec<PathBuf> =
        ["compare-old.pdf", "five-pages.pdf", "hidden-info.pdf", "form-fields.pdf", "redaction.pdf"]
            .iter()
            .map(|n| fixture(&format!("pdf/{n}")))
            .collect();
    let mut failures = Vec::new();
    let mut crashes = 0;
    for fx in &fixtures {
        let src = std::fs::read(fx).expect("read");
        for case in 0..cases {
            let mut b = src.clone();
            // Splice a chunk elsewhere (the mutation that found the redaction crash), plus a flip.
            let a = (next() as usize) % b.len();
            let len = ((next() as usize) % 512).min(b.len() - a);
            let chunk = b[a..a + len].to_vec();
            let at = (next() as usize) % b.len();
            b.splice(at..at, chunk);
            let i = (next() as usize) % b.len();
            b[i] ^= 1 << (next() % 8);
            let input = dir.path().join(format!("m{case}.pdf"));
            std::fs::write(&input, &b).expect("write");
            let out = dir.path().join(format!("out{case}.pdf"));
            let ops = vec![
                Operation::ExtractPages { input: input.clone(), pages: "1".into(), output: out.clone() },
                Operation::Sanitize { input: input.clone(), options: SanitizeOptions::default(), output: out.clone() },
                Operation::SetPageLabels {
                    input: input.clone(),
                    ranges: vec![PageLabelRange {
                        start: 0,
                        style: LabelStyle::Decimal,
                        prefix: String::new(),
                        first: 1,
                    }],
                    output: out.clone(),
                },
                Operation::Redact {
                    input: input.clone(),
                    areas: vec![RedactArea {
                        page: 0,
                        rect: RectPt { left: 50.0, bottom: 600.0, right: 400.0, top: 760.0 },
                    }],
                    output: out.clone(),
                },
            ];
            for op in ops {
                let id = op.id();
                let (_, r) = run_in_worker(&exe(), &OperationRequest::new(op), &|| false, &mut |_| {});
                match r {
                    Ok(_) => {
                        if lopdf::Document::load(&out).is_err() {
                            failures
                                .push(format!("{} case {case} {id}: committed output does not parse", fx.display()));
                        }
                    }
                    Err(e) => {
                        if e.code == "engine.crashed" {
                            crashes += 1;
                        }
                        if out.exists() {
                            failures.push(format!(
                                "{} case {case} {id}: failed ({}) but left an output",
                                fx.display(),
                                e.code
                            ));
                        }
                    }
                }
                let _ = std::fs::remove_file(&out);
            }
        }
    }
    let staged: Vec<_> = std::fs::read_dir(dir.path())
        .expect("ls")
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with(".pw-staged"))
        .collect();
    eprintln!("worker robustness: {crashes} engine crash(es) contained");
    assert!(staged.is_empty(), "staged leftovers");
    assert!(failures.is_empty(), "{failures:#?}");
}
