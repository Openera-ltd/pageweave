//! Missing and damaged OCR resources give specific, actionable errors (own process: the OCR engine
//! and ONNX Runtime environment are process-wide).
use pw_core::ops::{Operation, OperationRequest};
use pw_core::ErrorCategory;
use std::path::Path;

#[test]
fn missing_then_corrupt_resources_are_reported() {
    let Some(real) = pw_ocr::default_resource_dir() else {
        assert!(std::env::var_os("CI").is_none(), "OCR resources missing in CI (tools/fetch_ocr.py)");
        eprintln!("OCR resources not installed; skipping");
        return;
    };
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/pdf/scanned.pdf");
    let tmp = tempfile::tempdir().expect("ok");
    let res = tmp.path().join("ocr");
    std::fs::create_dir(&res).expect("ok");
    for f in [pw_ocr::DET_FILE, pw_ocr::REC_FILE, pw_ocr::RUNTIME_FILE] {
        std::fs::copy(real.join(f), res.join(f)).expect("ok");
    }
    // Only this test binary's OCR resource lookup reads the variable (debug builds only).
    std::env::set_var("PAGEWEAVE_OCR_DIR", &res);
    let run = |name: &str| execute_ocr(&fixture, &tmp.path().join(name));
    let e = run("a.pdf").expect_err("must fail");
    assert_eq!((e.category, e.code.as_str()), (ErrorCategory::MissingResource, "ocr.resource_missing"), "{e:?}");
    std::fs::write(res.join(pw_ocr::DICT_FILE), "not the dictionary\n").expect("ok");
    let e = run("b.pdf").expect_err("must fail");
    assert_eq!((e.category, e.code.as_str()), (ErrorCategory::CorruptResource, "ocr.resource_corrupt"), "{e:?}");
    assert!(!tmp.path().join("a.pdf").exists() && !tmp.path().join("b.pdf").exists());
}

fn execute_ocr(input: &Path, output: &Path) -> pw_core::PwResult<pw_core::ops::OperationResult> {
    pw_pdf::execute(
        &OperationRequest::new(Operation::OcrPages {
            input: input.into(),
            pages: Some("1".into()),
            skip_text_pages: true,
            dpi: 200,
            output: output.into(),
        }),
        None,
    )
}
