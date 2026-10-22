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
    let ops = v["result"]["operations"].as_array().expect("ops");
    for id in ["set_bookmarks", "export_text", "redact", "export_form_data", "read_form_data", "import_form_data"] {
        assert!(ops.iter().any(|o| o == id), "{id} listed");
    }
}

#[test]
fn optimize_reports_sizes_and_refuses_a_second_pass() {
    let dir = tempfile::tempdir().expect("tmp");
    let out = dir.path().join("small.pdf");
    let src = fixture("uncompressed-report.pdf");
    let r = bin().args(["--json", "optimize", "-o"]).arg(&out).arg(&src).output().expect("run");
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
    let v = json(&r);
    let before = v["result"]["data"]["bytes_before"].as_u64().expect("before");
    let after = v["result"]["data"]["bytes_after"].as_u64().expect("after");
    assert_eq!(before, std::fs::metadata(&src).expect("meta").len());
    assert_eq!(after, std::fs::metadata(&out).expect("meta").len());
    assert!(after * 3 < before, "{before} -> {after}");
    let again =
        bin().args(["--json", "optimize", "-o"]).arg(dir.path().join("again.pdf")).arg(&out).output().expect("run");
    assert!(!again.status.success());
    assert_eq!(json(&again)["error"]["code"], "optimize.not_smaller");
    assert!(!dir.path().join("again.pdf").exists());
}

#[test]
fn optimize_image_flags_resample_and_bad_values_are_usage_errors() {
    let dir = tempfile::tempdir().expect("tmp");
    let out = dir.path().join("photos.pdf");
    let r = bin()
        .args(["--json", "optimize", "--image-dpi", "150", "--image-quality", "70", "-o"])
        .arg(&out)
        .arg(fixture("photo-heavy.pdf"))
        .output()
        .expect("run");
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
    assert_eq!(json(&r)["result"]["data"]["images_reduced"], 3);
    let bad = bin()
        .args(["--json", "optimize", "--image-dpi", "5000", "-o"])
        .arg(dir.path().join("x.pdf"))
        .arg(fixture("photo-heavy.pdf"))
        .output()
        .expect("run");
    assert_eq!(bad.status.code(), Some(2), "out-of-range resolution is invalid input");
    assert_eq!(json(&bad)["error"]["code"], "optimize.dpi");
}

#[test]
fn compare_lists_differences_in_both_forms() {
    let (old, new) = (fixture("compare-old.pdf"), fixture("compare-new.pdf"));
    let r = bin().args(["--json", "compare"]).arg(&old).arg(&new).output().expect("run");
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
    let v = json(&r);
    assert_eq!(v["result"]["operation"], "compare");
    assert_eq!(v["result"]["data"]["summary"]["pages_inserted"], 1);
    assert!(v["result"]["outputs"].as_array().expect("outputs").is_empty(), "read-only");
    let human = bin().arg("compare").arg(&old).arg(&new).output().expect("run");
    let text = String::from_utf8_lossy(&human.stdout);
    assert!(text.contains("replaced: \"quarterly\" -> \"annual\""), "{text}");
    assert!(text.contains("page 3 -> -: deleted") && text.contains("page - -> 3: inserted"), "{text}");
    assert!(text.contains("Title: \"Report v1\" -> \"Report v2\""), "{text}");
}

#[test]
fn signatures_reports_status_and_says_what_is_not_trusted() {
    let r = bin().args(["--json", "signatures"]).arg(fixture("signed-tampered.pdf")).output().expect("run");
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
    let v = json(&r);
    assert_eq!(v["result"]["data"]["signatures"][0]["status"], "invalid");
    assert_eq!(v["result"]["data"]["signatures"][0]["trust"], "not_checked");
    let human = bin().arg("signatures").arg(fixture("signed-rsa.pdf")).output().expect("run");
    let text = String::from_utf8_lossy(&human.stdout);
    assert!(text.contains("Signature 1 (Approval): VALID") && text.contains("Signer: PageWeave Test Signer"), "{text}");
    assert!(text.contains("Trust: unknown signer") && text.contains("Revocation"), "{text}");
    let none = bin().arg("signatures").arg(fixture("five-pages.pdf")).output().expect("run");
    assert!(String::from_utf8_lossy(&none.stdout).contains("No digital signatures."));
}

#[test]
fn deskew_straightens_and_reports_each_page() {
    let dir = tempfile::tempdir().expect("tmp");
    let out = dir.path().join("straight.pdf");
    let r = bin()
        .args(["--json", "deskew", "--pages", "1,5", "-o"])
        .arg(&out)
        .arg(fixture("skewed-scan.pdf"))
        .output()
        .expect("run");
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
    let v = json(&r);
    assert_eq!(v["result"]["data"]["straightened"], 1);
    assert_eq!(v["result"]["data"]["pages"][1]["action"], "has_text");
    assert!(out.exists());
}

#[test]
fn blank_pages_lists_pages_and_how_to_remove_them() {
    let r = bin().arg("blank-pages").arg(fixture("blank-pages.pdf")).output().expect("run");
    let text = String::from_utf8_lossy(&r.stdout);
    assert!(text.contains("Blank pages: 2, 3") && text.contains("delete --pages \"2,3\""), "{text}");
    let bad = bin().args(["blank-pages", "--max-ink", "50"]).arg(fixture("blank-pages.pdf")).output().expect("run");
    assert_eq!(bad.status.code(), Some(2));
}

#[test]
fn extract_images_writes_files_and_reports_none() {
    let dir = tempfile::tempdir().expect("tmp");
    let r = bin()
        .args(["--json", "extract-images", "-o"])
        .arg(dir.path())
        .arg(fixture("photo-heavy.pdf"))
        .output()
        .expect("run");
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
    assert_eq!(json(&r)["result"]["outputs"].as_array().expect("outputs").len(), 4);
    let none = bin()
        .args(["--json", "extract-images", "-o"])
        .arg(dir.path())
        .arg(fixture("five-pages.pdf"))
        .output()
        .expect("run");
    assert_eq!(json(&none)["error"]["code"], "extract.none");
}

#[test]
fn orient_turns_a_page_and_explains_nothing_to_do() {
    let dir = tempfile::tempdir().expect("tmp");
    let out = dir.path().join("upright.pdf");
    let r = bin()
        .args(["--json", "orient", "--pages", "2", "-o"])
        .arg(&out)
        .arg(fixture("turned-scan.pdf"))
        .output()
        .expect("run");
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
    assert_eq!(json(&r)["result"]["data"]["pages"][0]["turn"], 180);
    let none = bin()
        .args(["--json", "orient", "--pages", "5", "-o"])
        .arg(dir.path().join("n.pdf"))
        .arg(fixture("turned-scan.pdf"))
        .output()
        .expect("run");
    assert_eq!(json(&none)["error"]["code"], "orient.nothing");
}

#[test]
fn objects_lists_runs_and_edit_applies_a_json_edits_file() {
    let dir = tempfile::tempdir().expect("tmp");
    let src = fixture("embedded-font.pdf");
    let r = bin().args(["--json", "objects", "--page", "1"]).arg(&src).output().expect("run");
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
    let v = json(&r);
    let texts = v["result"]["data"]["texts"].as_array().expect("texts").clone();
    let invoice = texts.iter().find(|t| t["text"] == "Invoice 2026 total due").expect("run listed");
    let edits = dir.path().join("edits.json");
    std::fs::write(
        &edits,
        serde_json::json!([{ "page": 0, "edits": [
            { "kind": "text", "objects": invoice["objects"], "text": "Invoice 2026 total sum", "dx": 0, "dy": 0 },
            { "kind": "add_text", "x": 72, "y": 400, "text": "PWCLI added" }
        ]}])
        .to_string(),
    )
    .expect("write");
    let out = dir.path().join("edited.pdf");
    let r = bin().args(["--json", "edit", "--edits"]).arg(&edits).arg("-o").arg(&out).arg(&src).output().expect("run");
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stdout));
    let t = bin().args(["--json", "text"]).arg(&out).output().expect("run");
    let text = json(&t)["result"]["data"]["pages"][0]["text"].as_str().expect("text").to_string();
    assert!(text.contains("Invoice 2026 total sum") && text.contains("PWCLI added"), "{text}");
    // Page 0 and a malformed edits file are refused with codes.
    let zero = bin().args(["--json", "objects", "--page", "0"]).arg(&src).output().expect("run");
    assert_eq!(json(&zero)["error"]["code"], "page.range");
    std::fs::write(&edits, "[{\"page\": 0, \"edits\": [{\"kind\": \"bogus\"}]}]").expect("write");
    let bad = bin()
        .args(["--json", "edit", "--edits"])
        .arg(&edits)
        .arg("-o")
        .arg(dir.path().join("x.pdf"))
        .arg(&src)
        .output()
        .expect("run");
    assert_eq!(json(&bad)["error"]["code"], "edit.invalid");
}

#[test]
fn sign_reads_the_password_from_stdin_and_reports_the_signer() {
    use std::io::Write;
    let dir = tempfile::tempdir().expect("tmp");
    let out = dir.path().join("signed.pdf");
    let key = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/keys/test-signer-rsa.p12");
    let run = |pw: &str, out: &Path| {
        let mut c = bin()
            .args(["sign", "--reason", "OK", "--visible", "1,350,40,560,100", "--certificate"])
            .arg(&key)
            .arg("-o")
            .arg(out)
            .arg(fixture("five-pages.pdf"))
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("spawn");
        writeln!(c.stdin.take().expect("stdin"), "{pw}").expect("write");
        c.wait_with_output().expect("wait")
    };
    let r = run("pageweave-test", &out);
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
    assert!(String::from_utf8_lossy(&r.stdout).contains("Signed as PageWeave Test Signer"));
    let bad = run("nope", &dir.path().join("bad.pdf"));
    assert!(!bad.status.success());
    assert!(!dir.path().join("bad.pdf").exists());
    let v = bin().args(["--json", "signatures"]).arg(&out).output().expect("run");
    assert_eq!(json(&v)["result"]["data"]["signatures"][0]["status"], "valid");
}

#[test]
fn signature_mark_places_typed_text() {
    let dir = tempfile::tempdir().expect("tmp");
    let out = dir.path().join("o.pdf");
    let r = bin()
        .args(["--json", "signature-mark", "--box", "350,40,560,100", "--text", "Ada Lovelace", "-o"])
        .arg(&out)
        .arg(fixture("five-pages.pdf"))
        .output()
        .expect("run");
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
    assert_eq!(json(&r)["result"]["data"]["kind"], "typed");
    let both = bin()
        .args(["signature-mark", "--box", "1,2,3,4", "-o", "x.pdf"])
        .arg(fixture("five-pages.pdf"))
        .output()
        .expect("run");
    assert!(!both.status.success());
}

#[test]
fn signatures_with_trust_report_trusted_signers() {
    use std::io::Write;
    let dir = tempfile::tempdir().expect("tmp");
    let out = dir.path().join("signed.pdf");
    let keys = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/keys");
    let mut c = bin()
        .args(["sign", "--certificate"])
        .arg(keys.join("test-signer-rsa.p12"))
        .arg("-o")
        .arg(&out)
        .arg(fixture("five-pages.pdf"))
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .spawn()
        .expect("spawn");
    writeln!(c.stdin.take().expect("stdin"), "pageweave-test").expect("write");
    assert!(c.wait().expect("wait").success());
    let trusted =
        bin().args(["--json", "signatures", "--trust"]).arg(keys.join("test-ca.der")).arg(&out).output().expect("run");
    assert_eq!(json(&trusted)["result"]["data"]["signatures"][0]["trust"], "trusted");
    let human = bin().arg("signatures").arg(&out).output().expect("run");
    assert!(String::from_utf8_lossy(&human.stdout).contains("Trust: unknown signer"));
}

/// Windows gives the main thread a 1 MB stack; a debug build's argument parsing outgrew it
/// (2026-10-18, every CLI test failed on Windows only). Emulate that limit on Unix.
#[cfg(unix)]
#[test]
fn runs_with_a_one_megabyte_main_thread_stack() {
    let bin = env!("CARGO_BIN_EXE_pageweave");
    let r = std::process::Command::new("sh")
        .args(["-c", &format!("ulimit -s 1024 && exec '{bin}' capabilities")])
        .output()
        .expect("run");
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
}

#[test]
fn text_saves_a_file_with_output_and_prints_without() {
    let dir = tempfile::tempdir().expect("tmp");
    let out = dir.path().join("five.txt");
    let r = bin()
        .args(["--json", "text", "--pages", "2-3", "-o"])
        .arg(&out)
        .arg(fixture("five-pages.pdf"))
        .output()
        .expect("run");
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
    assert_eq!(json(&r)["result"]["operation"], "export_text");
    let text = std::fs::read_to_string(&out).expect("read");
    assert!(
        text.contains("--- Page 2 ---\r\n") && text.contains("PWMARK003") && !text.contains("PWMARK001"),
        "{text:?}"
    );
    // The same command again refuses to overwrite (exit 2 = invalid input / conflict) and leaves the file.
    let again = bin().args(["text", "-o"]).arg(&out).arg(fixture("five-pages.pdf")).output().expect("run");
    assert!(!again.status.success());
    assert_eq!(std::fs::read_to_string(&out).expect("read"), text);
    let printed = bin().args(["text", "--pages", "1"]).arg(fixture("five-pages.pdf")).output().expect("run");
    assert!(String::from_utf8_lossy(&printed.stdout).contains("PWMARK001"));
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

#[test]
fn form_export_and_import_round_trip_through_files() {
    let dir = tempfile::tempdir().expect("tmp");
    // A .json name picks JSON without --format.
    let data = dir.path().join("data.json");
    let r =
        bin().args(["--json", "form-export", "-o"]).arg(&data).arg(fixture("form-nested.pdf")).output().expect("run");
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
    assert_eq!(json(&r)["result"]["data"]["format"], "json");
    let mut v: serde_json::Value = serde_json::from_slice(&std::fs::read(&data).expect("read")).expect("json");
    v["fields"][0]["value"] = "Ada".into();
    v["fields"][1]["value"] = "Paris".into();
    std::fs::write(&data, serde_json::to_vec(&v).expect("ser")).expect("write");

    let r = bin()
        .args(["--json", "form-import", "--preview"])
        .arg(fixture("form-nested.pdf"))
        .arg(&data)
        .output()
        .expect("run");
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
    let p = json(&r);
    assert_eq!(p["result"]["operation"], "read_form_data");
    let changed: Vec<&str> = p["result"]["data"]["matched"]
        .as_array()
        .expect("matched")
        .iter()
        .filter(|m| m["changed"] == true)
        .map(|m| m["name"].as_str().unwrap_or(""))
        .collect();
    assert_eq!(changed, ["applicant.name", "applicant.address.city"]);

    let out = dir.path().join("filled.pdf");
    let r = bin()
        .args(["--json", "form-import", "-o"])
        .arg(&out)
        .arg(fixture("form-nested.pdf"))
        .arg(&data)
        .output()
        .expect("run");
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
    assert_eq!(json(&r)["result"]["data"]["filled"], 2);
    let fields = bin().args(["--json", "fields"]).arg(&out).output().expect("run");
    let f = json(&fields);
    let names: Vec<(&str, &str)> = f["result"]["data"]["fields"]
        .as_array()
        .expect("fields")
        .iter()
        .take(2)
        .map(|x| (x["name"].as_str().unwrap_or(""), x["value"].as_str().unwrap_or("")))
        .collect();
    assert_eq!(names, [("applicant.name", "Ada"), ("applicant.address.city", "Paris")]);

    // XFDF by default; neither -o nor --preview is a usage error.
    let x = dir.path().join("data.xfdf");
    let r = bin().args(["form-export", "-o"]).arg(&x).arg(&out).output().expect("run");
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
    assert!(std::fs::read_to_string(&x).expect("read").contains("<value>Paris</value>"));
    let r = bin().arg("form-import").arg(fixture("form-nested.pdf")).arg(&x).output().expect("run");
    assert_eq!(r.status.code(), Some(1), "usage error");
    // A document without a form is refused with invalid input (exit 2).
    let r = bin()
        .args(["form-export", "-o"])
        .arg(dir.path().join("n.xfdf"))
        .arg(fixture("five-pages.pdf"))
        .output()
        .expect("run");
    assert_eq!(r.status.code(), Some(2), "{}", String::from_utf8_lossy(&r.stderr));
}

#[test]
fn split_by_size_and_by_bookmarks_from_the_cli() {
    let dir = tempfile::tempdir().expect("tmp");
    let r = bin()
        .args(["--json", "split", "--max-size", "300KB", "--out-dir"])
        .arg(dir.path())
        .arg(fixture("photo-heavy.pdf"))
        .output()
        .expect("run");
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
    let v = json(&r);
    let outs = v["result"]["outputs"].as_array().expect("outputs");
    assert!(outs.len() >= 2);
    for o in outs {
        let path = o["path"].as_str().expect("path");
        let size = std::fs::metadata(path).expect("meta").len();
        assert!(size <= 300 * 1024 || o["pages"] == 1, "{path}: {size}");
    }
    let bm = tempfile::tempdir().expect("tmp");
    let r = bin()
        .args(["split", "--by-bookmarks", "1", "--out-dir"])
        .arg(bm.path())
        .arg(fixture("five-pages.pdf"))
        .output()
        .expect("run");
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
    assert!(bm.path().join("five-pages_03_Chapter 3.pdf").exists());
    let r = bin()
        .args(["split", "--max-size", "lots", "--out-dir"])
        .arg(bm.path())
        .arg(fixture("five-pages.pdf"))
        .output()
        .expect("run");
    assert_eq!(r.status.code(), Some(2), "bad size is invalid input");
}

#[test]
fn patterns_lists_sensitive_text_with_positions() {
    let r =
        bin().args(["patterns", "--kinds", "email,card,ssn"]).arg(fixture("sensitive-info.pdf")).output().expect("run");
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
    let out = String::from_utf8_lossy(&r.stdout);
    assert!(out.contains("page 1  email        ada.lovelace@example.org"), "{out}");
    assert!(out.contains("4111 1111 1111 1111") && !out.contains("1112"), "{out}");
    assert!(out.contains("page 2  email") && out.contains("4 match(es)"), "{out}");
    let r = bin()
        .args(["--json", "patterns", "--custom", r"invoice \d+"])
        .arg(fixture("sensitive-info.pdf"))
        .output()
        .expect("run");
    let v = json(&r);
    assert_eq!(v["result"]["data"]["hits"][0]["kind"], "custom");
    assert!(v["result"]["data"]["hits"][0]["rects"][0]["left"].is_number());
    let r = bin().args(["patterns"]).arg(fixture("sensitive-info.pdf")).output().expect("run");
    assert_eq!(r.status.code(), Some(2), "nothing to look for is invalid input");
}

#[test]
fn accessibility_reports_checks_and_review_items() {
    let r = bin().args(["accessibility"]).arg(fixture("tagged-issues.pdf")).output().expect("run");
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
    let out = String::from_utf8_lossy(&r.stdout);
    assert!(out.contains("FAIL    Document language"), "{out}");
    assert!(out.contains("FAIL    Pictures have alternate text") && out.contains("pages: 1"), "{out}");
    assert!(out.contains("REVIEW  Reading order"), "{out}");
    assert!(out.contains("not a PDF/UA or WCAG verdict"), "{out}");
    let r = bin().args(["--json", "accessibility"]).arg(fixture("tagged-accessible.pdf")).output().expect("run");
    assert_eq!(json(&r)["result"]["data"]["failed"], 0);
}

#[test]
fn labels_set_and_remove() {
    let dir = tempfile::tempdir().expect("tmp");
    let out = dir.path().join("l.pdf");
    let r = bin()
        .args(["labels", "--range", "1:roman-lower", "--range", "3:decimal:1:Ch-", "-o"])
        .arg(&out)
        .arg(fixture("five-pages.pdf"))
        .output()
        .expect("run");
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
    let info = json(&bin().args(["--json", "inspect"]).arg(&out).output().expect("run"));
    let labels: Vec<&str> = info["result"]["data"]["pages"]
        .as_array()
        .expect("pages")
        .iter()
        .map(|p| p["label"].as_str().unwrap_or(""))
        .collect();
    assert_eq!(labels, ["i", "ii", "Ch-1", "Ch-2", "Ch-3"]);
    let out2 = dir.path().join("n.pdf");
    let r = bin().args(["labels", "--remove", "-o"]).arg(&out2).arg(&out).output().expect("run");
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
    let r = bin()
        .args(["labels", "--range", "0:decimal", "-o"])
        .arg(dir.path().join("x.pdf"))
        .arg(&out)
        .output()
        .expect("run");
    assert_eq!(r.status.code(), Some(2), "page 0 is invalid input");
}

#[test]
fn annotations_list_delete_and_retext() {
    let r = bin().args(["annotations"]).arg(fixture("hidden-info.pdf")).output().expect("run");
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
    let out = String::from_utf8_lossy(&r.stdout);
    assert!(
        out.contains("page 1  #0   Text") && out.contains("Reviewer note PWCOMMENT") && out.contains("4 annotation(s)"),
        "{out}"
    );
    let dir = tempfile::tempdir().expect("tmp");
    let copy = dir.path().join("c.pdf");
    let r = bin()
        .args(["annotations", "--delete", "1:0:Text", "--set-text", "1:2:Highlight=Seen", "-o"])
        .arg(&copy)
        .arg(fixture("hidden-info.pdf"))
        .output()
        .expect("run");
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
    let r = bin().args(["annotations"]).arg(&copy).output().expect("run");
    let out = String::from_utf8_lossy(&r.stdout);
    assert!(
        !out.contains("PWCOMMENT") && out.contains("Highlight       Seen") && out.contains("3 annotation(s)"),
        "{out}"
    );
    let r = bin()
        .args(["annotations", "--delete", "1:0", "-o"])
        .arg(dir.path().join("x.pdf"))
        .arg(fixture("hidden-info.pdf"))
        .output()
        .expect("run");
    assert_eq!(r.status.code(), Some(2), "missing type is invalid input");
}

#[test]
fn resources_reports_each_capability() {
    let r = bin().args(["--json", "resources"]).output().expect("run");
    let v = json(&r);
    let caps = v["result"]["capabilities"].as_array().expect("caps");
    assert_eq!(caps[0]["id"], "pdf_engine");
    assert_eq!(caps[0]["state"], "ready", "{v}");
    assert_eq!(caps[1]["id"], "text_recognition");
    let ready = v["result"]["all_ready"] == true;
    assert_eq!(r.status.success(), ready, "exit code follows readiness");
    let human = bin().arg("resources").output().expect("run");
    assert!(String::from_utf8_lossy(&human.stdout).contains("READY    PDF engine: Installed and verified."));
}

#[test]
fn to_docx_writes_a_word_file_and_reports_what_was_kept() {
    let dir = tempfile::tempdir().expect("tmp");
    let out = dir.path().join("article.docx");
    let r = bin()
        .args(["--json", "to-docx", "--pages", "1-2"])
        .arg(fixture("article.pdf"))
        .arg("-o")
        .arg(&out)
        .output()
        .expect("run");
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stdout));
    let v = json(&r);
    assert_eq!(v["result"]["operation"], "convert_to_docx");
    assert_eq!(v["result"]["data"]["pages"], 2);
    assert_eq!(v["result"]["data"]["links"], 1);
    assert_eq!(v["result"]["data"]["tables"], 1, "the ruled table on page 2");
    let z = zip::ZipArchive::new(std::fs::File::open(&out).expect("open")).expect("a Word package");
    assert!(z.file_names().flatten().any(|n| n == "word/document.xml"));
    // Existing output → conflict (exit 5); wrong extension → invalid input (exit 2).
    let again =
        bin().args(["--json", "to-docx"]).arg(fixture("article.pdf")).arg("-o").arg(&out).output().expect("run");
    assert_eq!(json(&again)["error"]["category"], "output_conflict");
    let bad = bin()
        .args(["to-docx"])
        .arg(fixture("article.pdf"))
        .arg("-o")
        .arg(dir.path().join("a.doc"))
        .output()
        .expect("run");
    assert_eq!(bad.status.code(), Some(2));
}

#[test]
fn tables_export_to_xlsx_and_csv() {
    let dir = tempfile::tempdir().expect("tmp");
    let out = dir.path().join("t.xlsx");
    let r = bin().arg("tables").arg(fixture("tables.pdf")).arg("-o").arg(&out).output().expect("run");
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
    let text = String::from_utf8_lossy(&r.stdout);
    assert!(
        text.contains("Page 1 table 1: 4 rows × 4 columns") && text.contains("Page 2 table 1: 3 rows × 2 columns"),
        "{text}"
    );
    let csv = bin()
        .args(["--json", "tables", "--format", "csv"])
        .arg(fixture("tables.pdf"))
        .arg("-o")
        .arg(dir.path())
        .output()
        .expect("run");
    assert_eq!(json(&csv)["result"]["outputs"].as_array().map(Vec::len), Some(2));
    let none = bin()
        .args(["--json", "tables"])
        .arg(fixture("five-pages.pdf"))
        .arg("-o")
        .arg(dir.path().join("n.xlsx"))
        .output()
        .expect("run");
    assert_eq!(json(&none)["error"]["code"], "tables.none");
}
