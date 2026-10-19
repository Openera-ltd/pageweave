//! Embedded files against the real engine.
use pw_core::ops::{Operation, OperationRequest};
use pw_core::ErrorCategory;
use pw_pdf::execute;
use std::path::{Path, PathBuf};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/pdf").join(name)
}

fn list(path: &Path) -> Vec<(String, u64)> {
    let r = execute(&OperationRequest::new(Operation::ListAttachments { input: path.into() }), None).expect("list");
    r.data["attachments"]
        .as_array()
        .expect("array")
        .iter()
        .map(|a| (a["name"].as_str().unwrap_or("").to_string(), a["bytes"].as_u64().unwrap_or(0)))
        .collect()
}

#[test]
fn list_and_save_an_embedded_file() {
    let src = fixture("hidden-info.pdf");
    assert_eq!(list(&src), vec![("secret.txt".to_string(), "Embedded PWATTACH-DOC".len() as u64)]);
    assert!(list(&fixture("five-pages.pdf")).is_empty());
    let dir = tempfile::tempdir().expect("tmp");
    let out = dir.path().join("secret.txt");
    let r = execute(
        &OperationRequest::new(Operation::ExtractAttachment { input: src.clone(), index: 0, output: out.clone() }),
        None,
    )
    .expect("extract");
    assert!(r.warnings.iter().any(|w| w.contains("trust")), "{:?}", r.warnings);
    assert_eq!(std::fs::read(&out).expect("read"), b"Embedded PWATTACH-DOC");
    let err = execute(
        &OperationRequest::new(Operation::ExtractAttachment { input: src, index: 5, output: dir.path().join("x") }),
        None,
    )
    .expect_err("bad index");
    assert_eq!(err.code, "attach.index");
}

#[test]
fn attach_files_into_a_copy_with_unique_names() {
    let src = fixture("hidden-info.pdf");
    let before = std::fs::read(&src).expect("read");
    let dir = tempfile::tempdir().expect("tmp");
    let a = dir.path().join("secret.txt"); // same name as the existing attachment
    let b = dir.path().join("data – 2026.csv");
    std::fs::write(&a, b"replacement?").expect("write");
    std::fs::write(&b, b"x,y\n1,2\n").expect("write");
    let out = dir.path().join("attached.pdf");
    let r = execute(
        &OperationRequest::new(Operation::AddAttachments {
            input: src.clone(),
            files: vec![a, b],
            output: out.clone(),
        }),
        None,
    )
    .expect("attach");
    assert!(r.warnings.iter().any(|w| w.contains("secret (2).txt")), "{:?}", r.warnings);
    assert_eq!(std::fs::read(&src).expect("read"), before, "source untouched");
    // PDF name trees are kept sorted by name, so look entries up by name.
    let names: Vec<String> = list(&out).into_iter().map(|(n, _)| n).collect();
    let mut sorted = names.clone();
    sorted.sort();
    assert_eq!(sorted, vec!["data – 2026.csv", "secret (2).txt", "secret.txt"]);
    let index = names.iter().position(|n| n == "data – 2026.csv").expect("csv") as u32;
    let saved = dir.path().join("back.csv");
    execute(&OperationRequest::new(Operation::ExtractAttachment { input: out, index, output: saved.clone() }), None)
        .expect("extract");
    assert_eq!(std::fs::read(&saved).expect("read"), b"x,y\n1,2\n");
}

#[test]
fn missing_or_wrong_files_write_nothing() {
    let dir = tempfile::tempdir().expect("tmp");
    let out = dir.path().join("o.pdf");
    let src = fixture("five-pages.pdf");
    let add = |files: Vec<PathBuf>| {
        execute(
            &OperationRequest::new(Operation::AddAttachments { input: src.clone(), files, output: out.clone() }),
            None,
        )
    };
    assert_eq!(add(vec![]).expect_err("none").code, "attach.none");
    assert_eq!(add(vec![dir.path().join("nope.txt")]).expect_err("missing").category, ErrorCategory::NotFound);
    assert_eq!(add(vec![dir.path().to_path_buf()]).expect_err("dir").code, "attach.not_file");
    // Attaching the output file itself is refused: it is an input.
    std::fs::write(&out, b"existing").expect("write");
    let err = execute(
        &OperationRequest::new(Operation::AddAttachments {
            input: src.clone(),
            files: vec![out.clone()],
            output: out.clone(),
        }),
        None,
    )
    .expect_err("output is an input");
    assert_ne!(err.category, ErrorCategory::Internal, "{err:?}");
    assert_eq!(std::fs::read(&out).expect("read"), b"existing");
}
