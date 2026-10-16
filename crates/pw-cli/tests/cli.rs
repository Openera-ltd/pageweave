//! Runs the real `pageweave` binary (headless) and checks outputs, JSON envelopes and exit codes.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_pageweave"))
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/pdf").join(name)
}

fn json(out: &Output) -> serde_json::Value {
    serde_json::from_slice(&out.stdout)
        .unwrap_or_else(|e| panic!("stdout is not JSON ({e}): {}", String::from_utf8_lossy(&out.stdout)))
}

#[test]
fn inspect_json_envelope() {
    let out = bin().args(["--json", "inspect"]).arg(fixture("five-pages.pdf")).output().expect("run");
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let v = json(&out);
    assert_eq!(v["ok"], true);
    assert_eq!(v["result"]["data"]["page_count"], 5);
    assert_eq!(v["result"]["operation"], "inspect");
}

#[test]
fn human_output_and_exit_codes() {
    let out = bin().arg("inspect").arg(fixture("single-page.pdf")).output().expect("run");
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("Pages:      1"));

    let out = bin().arg("inspect").arg(fixture("not-a-pdf.pdf")).output().expect("run");
    assert_eq!(out.status.code(), Some(3), "unsupported format exit code");
    assert!(String::from_utf8_lossy(&out.stderr).contains("not a PDF"));

    let out = bin().args(["--json", "inspect"]).arg(fixture("missing.pdf")).output().expect("run");
    assert_eq!(out.status.code(), Some(7));
    let v = json(&out);
    assert_eq!(v["ok"], false);
    assert_eq!(v["error"]["category"], "not_found");

    let out = bin().arg("no-such-command").output().expect("run");
    assert_eq!(out.status.code(), Some(1), "usage error");
}

#[test]
fn extract_then_conflict_policies() {
    let dir = tempfile::tempdir().expect("tmp");
    let out_path = dir.path().join("x.pdf");
    let run = |extra: &[&str]| {
        bin()
            .args(extra)
            .args(["extract", "--pages", "2-3", "-o"])
            .arg(&out_path)
            .arg(fixture("five-pages.pdf"))
            .output()
            .expect("run")
    };
    let first = run(&[]);
    assert!(first.status.success(), "{}", String::from_utf8_lossy(&first.stderr));
    assert_eq!(lopdf::Document::load(&out_path).expect("parse").get_pages().len(), 2);

    let second = run(&[]);
    assert_eq!(second.status.code(), Some(8), "existing output must not be overwritten by default");

    let renamed = run(&["--json", "--on-conflict", "rename"]);
    assert!(renamed.status.success());
    let v = json(&renamed);
    assert!(v["result"]["outputs"][0]["path"].as_str().expect("path").ends_with("x (1).pdf"));
}

#[test]
fn invalid_range_reports_exit_2() {
    let dir = tempfile::tempdir().expect("tmp");
    let out = bin()
        .args(["--json", "delete", "--pages", "0", "-o"])
        .arg(dir.path().join("d.pdf"))
        .arg(fixture("five-pages.pdf"))
        .output()
        .expect("run");
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(json(&out)["error"]["code"], "range.zero");
}

#[test]
fn rotate_accepts_negative_degrees() {
    let dir = tempfile::tempdir().expect("tmp");
    let out = bin()
        .args(["rotate", "--degrees", "-90", "-o"])
        .arg(dir.path().join("r.pdf"))
        .arg(fixture("single-page.pdf"))
        .output()
        .expect("run");
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
}

#[test]
fn password_from_stdin_never_echoed() {
    let mut child = bin()
        .args(["--json", "--password-stdin", "inspect"])
        .arg(fixture("encrypted-rc4-user-password.pdf"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn");
    child.stdin.as_mut().expect("stdin").write_all(b"pw-user-test\n").expect("write");
    let out = child.wait_with_output().expect("wait");
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let all = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(!all.contains("pw-user-test"), "password leaked to output");
    assert_eq!(json(&out)["result"]["data"]["encrypted"], true);

    let out = bin().args(["--json", "inspect"]).arg(fixture("encrypted-rc4-user-password.pdf")).output().expect("run");
    assert_eq!(out.status.code(), Some(5));
}

#[test]
fn run_typed_json_request_from_stdin() {
    let dir = tempfile::tempdir().expect("tmp");
    let req = serde_json::json!({
        "schema_version": 1,
        "operation": {"op": "split", "input": fixture("five-pages.pdf"), "split": {"mode": "every_n", "n": 3}, "output_dir": dir.path()},
    });
    let mut child =
        bin().args(["--json", "run", "-"]).stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().expect("spawn");
    child.stdin.as_mut().expect("stdin").write_all(req.to_string().as_bytes()).expect("write");
    let out = child.wait_with_output().expect("wait");
    assert!(out.status.success());
    assert_eq!(json(&out)["result"]["data"]["parts"], 2);

    let mut child =
        bin().args(["--json", "run", "-"]).stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().expect("spawn");
    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(br#"{"schema_version":1,"operation":{"op":"shell","cmd":"rm -rf /"}}"#)
        .expect("write");
    let out = child.wait_with_output().expect("wait");
    assert_eq!(out.status.code(), Some(2), "unknown operations are rejected");
}

#[test]
fn capabilities_lists_operations() {
    let out = bin().args(["--json", "capabilities"]).output().expect("run");
    assert!(out.status.success());
    let v = json(&out);
    assert!(v["result"]["operations"].as_array().expect("ops").iter().any(|o| o == "merge"));
    assert!(v["result"]["operations"].as_array().expect("ops").iter().any(|o| o == "annotate"));
    assert_eq!(v["result"]["network"], "none");
}

#[test]
fn unicode_and_space_paths() {
    let dir = tempfile::tempdir().expect("tmp");
    let src = dir.path().join("Résumé – 文件 copy.pdf");
    std::fs::copy(fixture("five-pages.pdf"), &src).expect("copy");
    let out_path = dir.path().join("出力 out.pdf");
    let out = bin().args(["extract", "--pages", "1", "-o"]).arg(&out_path).arg(&src).output().expect("run");
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(out_path.is_file());
}
