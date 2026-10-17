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
fn fields_lists_and_run_fills_a_form() {
    let out = bin().arg("fields").arg(fixture("form-fields.pdf")).output().expect("run");
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("full_name  (text)"), "{text}");
    assert!(text.contains("subscribe  (checkbox) = unchecked"), "{text}");
    assert!(text.contains("plan  (radio) options: basic pro"), "{text}");
    assert!(text.contains("options: Canada | Germany | Japan"), "{text}");

    let dir = tempfile::tempdir().expect("tmp");
    let filled = dir.path().join("filled.pdf");
    let req = serde_json::json!({
        "schema_version": 1,
        "operation": {"op": "fill_form", "input": fixture("form-fields.pdf"), "output": filled, "values": [
            {"name": "full_name", "value": "Grace Hopper"},
            {"name": "plan", "value": "basic"}
        ]}
    });
    let req_path = dir.path().join("req.json");
    std::fs::write(&req_path, req.to_string()).expect("write");
    let out = bin().args(["--json", "run", req_path.to_str().expect("utf8")]).output().expect("run");
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stdout));
    let out = bin().args(["fields", filled.to_str().expect("utf8")]).output().expect("run");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("full_name  (text) = \"Grace Hopper\""), "{text}");
    assert!(text.contains("options: [basic] pro"), "{text}");

    let out = bin().args(["--json", "fields"]).arg(fixture("five-pages.pdf")).output().expect("run");
    assert_eq!(json(&out)["result"]["data"]["fields"].as_array().map(Vec::len), Some(0));
}

#[test]
fn images_command_exports_pngs() {
    let dir = tempfile::tempdir().expect("tmp");
    let out = bin()
        .args(["--json", "images", "--pages", "1-2", "--dpi", "72", "--out-dir"])
        .arg(dir.path())
        .arg(fixture("five-pages.pdf"))
        .output()
        .expect("run");
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stdout));
    let v = json(&out);
    assert_eq!(v["result"]["outputs"].as_array().map(Vec::len), Some(2));
    assert!(dir.path().join("five-pages_p001.png").is_file());
    assert!(dir.path().join("five-pages_p002.png").is_file());
}

#[test]
fn redact_previews_then_writes_a_verified_copy() {
    let src = fixture("redaction.pdf");
    let out = bin().args(["redact", "--preview", "--area", "1:100,670,200,700"]).arg(&src).output().expect("run");
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(String::from_utf8_lossy(&out.stdout).contains("SECRET-4711"));

    let dir = tempfile::tempdir().expect("tmp");
    let red = dir.path().join("red.pdf");
    let out = bin()
        .args(["--json", "redact", "--area", "1:100,670,200,700", "-o"])
        .arg(&red)
        .arg(&src)
        .output()
        .expect("run");
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stdout));
    assert_eq!(json(&out)["result"]["operation"], "redact");
    let text = bin().args(["text", "--pages", "1"]).arg(&red).output().expect("run");
    let text = String::from_utf8_lossy(&text.stdout);
    assert!(!text.contains("SECRET") && text.contains("is closed"), "{text}");

    // Bad area syntax is a usage error naming the expected form; nothing is written.
    let bad = dir.path().join("bad.pdf");
    let out = bin().args(["redact", "--area", "1:1,2,3", "-o"]).arg(&bad).arg(&src).output().expect("run");
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("PAGE:LEFT,BOTTOM,RIGHT,TOP"));
    assert!(!bad.exists());
}

#[test]
fn ocr_makes_a_scanned_file_searchable() {
    if pw_ocr::default_resource_dir().is_none() {
        assert!(std::env::var_os("CI").is_none(), "OCR resources missing in CI");
        eprintln!("OCR resources not installed; skipping");
        return;
    }
    let dir = tempfile::tempdir().expect("tmp");
    let out = dir.path().join("searchable.pdf");
    let r = bin()
        .args(["--json", "ocr", "--pages", "1", "-o"])
        .arg(&out)
        .arg(fixture("scanned.pdf"))
        .output()
        .expect("run");
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stdout));
    assert_eq!(json(&r)["result"]["data"]["pages_processed"], 1);
    let found = bin().args(["--json", "search"]).arg(&out).arg("Description").output().expect("run");
    assert_eq!(json(&found)["result"]["data"]["count"], 1);
    let bad = bin()
        .args(["ocr", "--dpi", "20", "-o"])
        .arg(dir.path().join("x.pdf"))
        .arg(fixture("scanned.pdf"))
        .output()
        .expect("run");
    assert!(!bad.status.success());
}

#[test]
fn protect_reads_passwords_from_stdin_only() {
    let dir = tempfile::tempdir().expect("tmp");
    let out = dir.path().join("locked.pdf");
    let mut child = bin()
        .args(["--json", "protect", "--no-print", "--owner-password-stdin", "-o"])
        .arg(&out)
        .arg(fixture("five-pages.pdf"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn");
    child.stdin.take().expect("stdin").write_all(b"cli-open\ncli-owner\n").expect("write");
    let r = child.wait_with_output().expect("wait");
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stdout));
    assert!(!String::from_utf8_lossy(&r.stdout).contains("cli-open"), "passwords never echoed");
    // Needs the password now: inspect without it fails with the password exit code (5).
    let locked = bin().args(["inspect"]).arg(&out).output().expect("run");
    assert_eq!(locked.status.code(), Some(5));
    let mut child = bin()
        .args(["--json", "--password-stdin", "inspect"])
        .arg(&out)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn");
    child.stdin.take().expect("stdin").write_all(b"cli-open\n").expect("write");
    let r = child.wait_with_output().expect("wait");
    assert_eq!(json(&r)["result"]["data"]["encrypted"], true);
    // An empty new password is refused before anything is written.
    let bad = dir.path().join("bad.pdf");
    let mut child = bin()
        .args(["protect", "-o"])
        .arg(&bad)
        .arg(fixture("five-pages.pdf"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn");
    child.stdin.take().expect("stdin").write_all(b"\n").expect("write");
    let r = child.wait_with_output().expect("wait");
    assert_eq!(r.status.code(), Some(2));
    assert!(!bad.exists());
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

/// Run with `stdin_text` piped in; returns the output.
fn run_with_stdin(args: &[&str], paths: &[&Path], stdin_text: &str) -> Output {
    let mut cmd = bin();
    cmd.args(args);
    for p in paths {
        cmd.arg(p);
    }
    let mut child = cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().expect("spawn");
    child.stdin.take().expect("stdin").write_all(stdin_text.as_bytes()).expect("write");
    child.wait_with_output().expect("wait")
}

#[test]
fn unprotect_honours_owner_restrictions() {
    let dir = tempfile::tempdir().expect("tmp");
    let locked = dir.path().join("locked.pdf");
    let r = run_with_stdin(
        &["protect", "--no-modify", "--owner-password-stdin", "-o"],
        &[&locked, &fixture("five-pages.pdf")],
        "reader\nowner\n",
    );
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
    let open = dir.path().join("open.pdf");
    let r = run_with_stdin(&["--password-stdin", "unprotect", "-o"], &[&open, &locked], "reader\n");
    assert_eq!(r.status.code(), Some(6), "permission exit code: {}", String::from_utf8_lossy(&r.stderr));
    assert!(String::from_utf8_lossy(&r.stderr).contains("restricted"));
    assert!(!open.exists());
    let r = run_with_stdin(&["--password-stdin", "unprotect", "-o"], &[&open, &locked], "owner\n");
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
    let v = json(&bin().args(["--json", "inspect"]).arg(&open).output().expect("run"));
    assert_eq!(v["result"]["data"]["encrypted"], false);
}

#[test]
fn sanitize_inspects_and_cleans() {
    let v =
        json(&bin().args(["--json", "sanitize", "--inspect"]).arg(fixture("hidden-info.pdf")).output().expect("run"));
    assert_eq!(v["result"]["data"]["scripts"], 7);
    let dir = tempfile::tempdir().expect("tmp");
    let out = dir.path().join("clean.pdf");
    let r = bin()
        .args(["sanitize", "--keep-comments", "-o"])
        .arg(&out)
        .arg(fixture("hidden-info.pdf"))
        .output()
        .expect("run");
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
    let after = json(&bin().args(["--json", "sanitize", "--inspect"]).arg(&out).output().expect("run"));
    let d = &after["result"]["data"];
    assert_eq!(d["scripts"], 0);
    assert_eq!(d["comments"], 2, "kept on request (the hidden square went with its layer)");
    assert_eq!(d["earlier_versions"], false);
    // Without -o and without --inspect: an argument error (exit 1), nothing written.
    let r = bin().arg("sanitize").arg(fixture("hidden-info.pdf")).output().expect("run");
    assert_eq!(r.status.code(), Some(1));
}

#[test]
fn properties_set_keep_and_remove() {
    let dir = tempfile::tempdir().expect("tmp");
    let out = dir.path().join("p.pdf");
    let r = bin()
        .args(["properties", "--title", "CLI title", "--author", "", "-o"])
        .arg(&out)
        .arg(fixture("five-pages.pdf"))
        .output()
        .expect("run");
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
    let v = json(&bin().args(["--json", "inspect"]).arg(&out).output().expect("run"));
    assert_eq!(v["result"]["data"]["title"], "CLI title");
    assert_eq!(v["result"]["data"]["author"], serde_json::Value::Null);
    // Nothing to change: invalid input (exit 2), nothing written.
    let none = dir.path().join("none.pdf");
    let r = bin().args(["properties", "-o"]).arg(&none).arg(fixture("five-pages.pdf")).output().expect("run");
    assert_eq!(r.status.code(), Some(2));
    assert!(!none.exists());
}

#[test]
fn watermark_command() {
    let dir = tempfile::tempdir().expect("tmp");
    let out = dir.path().join("w.pdf");
    let r = bin()
        .args(["watermark", "--text", "DRAFT", "--pages", "2", "--angle", "-45", "--opacity", "40", "-o"])
        .arg(&out)
        .arg(fixture("five-pages.pdf"))
        .output()
        .expect("run");
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
    let v = json(&bin().args(["--json", "search"]).arg(&out).arg("DRAFT").output().expect("run"));
    assert_eq!(v["result"]["data"]["count"], 1);
    assert_eq!(v["result"]["data"]["hits"][0]["page"], 1);
    let bad = bin()
        .args(["watermark", "--text", "x", "--opacity", "0", "-o"])
        .arg(dir.path().join("b.pdf"))
        .arg(fixture("five-pages.pdf"))
        .output()
        .expect("run");
    assert_eq!(bad.status.code(), Some(2));
}

#[test]
fn crop_command_with_units() {
    let dir = tempfile::tempdir().expect("tmp");
    let out = dir.path().join("c.pdf");
    let r = bin()
        .args(["crop", "--top", "1", "--unit", "in", "-o"])
        .arg(&out)
        .arg(fixture("five-pages.pdf"))
        .output()
        .expect("run");
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
    assert!(String::from_utf8_lossy(&r.stderr).contains("hidden, not removed"));
    let v = json(&bin().args(["--json", "inspect"]).arg(&out).output().expect("run"));
    assert_eq!(v["result"]["data"]["pages"][0]["height_pt"], 720.0, "one inch off the top");
    let bad = bin()
        .args(["crop", "--top", "1", "--unit", "cm", "-o"])
        .arg(dir.path().join("b.pdf"))
        .arg(fixture("five-pages.pdf"))
        .output()
        .expect("run");
    assert_eq!(bad.status.code(), Some(2));
}

#[test]
fn attachments_list_save_and_add() {
    let r = bin().arg("attachments").arg(fixture("hidden-info.pdf")).output().expect("run");
    assert!(String::from_utf8_lossy(&r.stdout).contains("secret.txt"), "{}", String::from_utf8_lossy(&r.stdout));
    let dir = tempfile::tempdir().expect("tmp");
    let saved = dir.path().join("s.txt");
    let r = bin()
        .args(["attachments", "--save", "0", "-o"])
        .arg(&saved)
        .arg(fixture("hidden-info.pdf"))
        .output()
        .expect("run");
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
    assert_eq!(std::fs::read(&saved).expect("read"), b"Embedded PWATTACH-DOC");
    let out = dir.path().join("a.pdf");
    let r = bin()
        .args(["attachments", "--add"])
        .arg(&saved)
        .arg("-o")
        .arg(&out)
        .arg(fixture("five-pages.pdf"))
        .output()
        .expect("run");
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
    let v = json(&bin().args(["--json", "attachments"]).arg(&out).output().expect("run"));
    assert_eq!(v["result"]["data"]["attachments"][0]["name"], "s.txt");
    let r = bin().args(["attachments", "--save", "0"]).arg(fixture("hidden-info.pdf")).output().expect("run");
    assert_eq!(r.status.code(), Some(2), "needs -o");
}

#[test]
fn images_to_pdf_command() {
    let img = |n: &str| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/images").join(n);
    let dir = tempfile::tempdir().expect("tmp");
    let out = dir.path().join("i.pdf");
    let r = bin()
        .args(["images-to-pdf"])
        .arg(img("gray-300dpi.jpg"))
        .arg(img("alpha.png"))
        .args(["--page-size", "letter", "-o"])
        .arg(&out)
        .output()
        .expect("run");
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
    let v = json(&bin().args(["--json", "inspect"]).arg(&out).output().expect("run"));
    assert_eq!(v["result"]["data"]["page_count"], 2);
    assert_eq!(v["result"]["data"]["pages"][0]["width_pt"], 612.0);
    let bad = bin()
        .args(["images-to-pdf"])
        .arg(img("truncated.jpg"))
        .arg("-o")
        .arg(dir.path().join("b.pdf"))
        .output()
        .expect("run");
    assert_eq!(bad.status.code(), Some(4), "damaged input exit code: {}", String::from_utf8_lossy(&bad.stderr));
}
