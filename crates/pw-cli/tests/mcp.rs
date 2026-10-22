//! `pageweave mcp`: a real server process driven over stdio the way an MCP client does. Every tool
//! the server advertises is called here and its result checked (outputs with lopdf).

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{channel, Receiver};
use std::time::Duration;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/pdf").join(name)
}

struct Client {
    child: Child,
    stdin: Option<ChildStdin>,
    rx: Receiver<Value>,
    next: u64,
    /// Notifications seen while waiting for responses.
    notes: Vec<Value>,
    /// Responses that arrived while waiting for another id (concurrent calls finish in any order).
    early: Vec<Value>,
}

impl Client {
    fn start(roots: &[&Path], write: bool) -> Client {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_pageweave"));
        cmd.arg("mcp");
        for r in roots {
            cmd.arg("--root").arg(r);
        }
        if write {
            cmd.arg("--allow-write");
        }
        let mut child = cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().expect("spawn");
        let stdout = child.stdout.take().expect("stdout");
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                let v: Value =
                    serde_json::from_str(&line).unwrap_or_else(|e| panic!("server wrote non-JSON ({e}): {line}"));
                if tx.send(v).is_err() {
                    break;
                }
            }
        });
        let stdin = child.stdin.take();
        let mut c = Client { child, stdin, rx, next: 1, notes: vec![], early: vec![] };
        let r = c.request("initialize", json!({ "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": { "name": "test", "version": "1" } }));
        assert_eq!(r["result"]["protocolVersion"], "2025-06-18");
        assert_eq!(r["result"]["serverInfo"]["name"], "pageweave");
        c.notify("notifications/initialized", json!({}));
        c
    }

    fn raw(&mut self, line: &str) {
        let w = self.stdin.as_mut().expect("stdin open");
        w.write_all(line.as_bytes()).expect("write");
        w.write_all(b"\n").expect("write");
        w.flush().expect("flush");
    }

    fn notify(&mut self, method: &str, params: Value) {
        self.raw(&json!({ "jsonrpc": "2.0", "method": method, "params": params }).to_string());
    }

    fn send(&mut self, method: &str, params: Value) -> u64 {
        let id = self.next;
        self.next += 1;
        self.raw(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }).to_string());
        id
    }

    fn wait(&mut self, id: u64) -> Value {
        if let Some(i) = self.early.iter().position(|v| v.get("id") == Some(&json!(id))) {
            return self.early.remove(i);
        }
        loop {
            let v = self.rx.recv_timeout(Duration::from_secs(120)).expect("response in time");
            if v.get("id") == Some(&json!(id)) {
                return v;
            }
            if v.get("method").is_some() {
                self.notes.push(v);
            } else if v.get("id").is_some_and(|i| i.is_u64()) {
                self.early.push(v);
            } else {
                panic!("unexpected message {v}");
            }
        }
    }

    fn next_message(&mut self) -> Value {
        self.rx.recv_timeout(Duration::from_secs(120)).expect("message in time")
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.send(method, params);
        self.wait(id)
    }

    /// tools/call → (isError, structuredContent).
    fn call(&mut self, name: &str, args: Value) -> (bool, Value) {
        let r = self.request("tools/call", json!({ "name": name, "arguments": args }));
        let res = &r["result"];
        assert!(res["content"][0]["text"].is_string(), "text content always present: {r}");
        (res["isError"] == true, res["structuredContent"].clone())
    }

    fn ok(&mut self, name: &str, args: Value) -> Value {
        let (err, v) = self.call(name, args);
        assert!(!err, "{name} failed: {v}");
        v
    }

    fn err(&mut self, name: &str, args: Value) -> String {
        let (err, v) = self.call(name, args);
        assert!(err, "{name} should fail: {v}");
        v["error"]["code"].as_str().unwrap_or_default().to_string()
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        self.stdin.take();
        let _ = self.child.wait();
    }
}

fn copy_fixtures(dir: &Path, names: &[&str]) {
    for n in names {
        std::fs::copy(fixture(n), dir.join(n)).expect("copy");
    }
}

fn page_count(p: &Path) -> usize {
    lopdf::Document::load(p).expect("lopdf parses the output").get_pages().len()
}

fn tool_names(c: &mut Client) -> Vec<String> {
    let r = c.request("tools/list", json!({}));
    r["result"]["tools"]
        .as_array()
        .expect("tools")
        .iter()
        .map(|t| t["name"].as_str().unwrap_or("").to_string())
        .collect()
}

#[test]
fn read_only_server_lists_and_runs_read_tools_and_refuses_writing() {
    let root = tempfile::tempdir().expect("tmp");
    copy_fixtures(
        root.path(),
        &[
            "five-pages.pdf",
            "search-terms.pdf",
            "form-fields.pdf",
            "compare-old.pdf",
            "compare-new.pdf",
            "signed-rsa.pdf",
            "sensitive-info.pdf",
            "tagged-issues.pdf",
        ],
    );
    let mut c = Client::start(&[root.path()], false);

    let names = tool_names(&mut c);
    assert_eq!(
        names,
        [
            "list_capabilities",
            "inspect_document",
            "extract_text",
            "search_text",
            "list_form_fields",
            "compare_documents",
            "check_signatures",
            "check_accessibility",
            "find_sensitive_text"
        ],
        "write tools are not even listed"
    );
    let r = c.request("tools/list", json!({}));
    for t in r["result"]["tools"].as_array().expect("tools") {
        assert_eq!(t["inputSchema"]["type"], "object");
        assert_eq!(t["annotations"]["readOnlyHint"], true);
    }

    let caps = c.ok("list_capabilities", json!({}));
    assert_eq!(caps["can_write"], false);
    assert_eq!(caps["network"], "none");

    let info = c.ok("inspect_document", json!({ "path": "five-pages.pdf" }));
    assert_eq!(info["data"]["page_count"], 5);
    let text = c.ok("extract_text", json!({ "path": "five-pages.pdf", "pages": "2" }));
    assert!(text["data"]["pages"][0]["text"].as_str().unwrap_or("").contains("PWMARK002"), "{text}");
    let hits = c.ok("search_text", json!({ "path": "search-terms.pdf", "query": "alpha", "whole_word": true }));
    assert!(hits["data"]["count"].as_u64().unwrap_or(0) >= 3, "{hits}");
    let fields = c.ok("list_form_fields", json!({ "path": "form-fields.pdf" }));
    assert_eq!(fields["data"]["fields"][0]["name"], "full_name");
    let acc = c.ok("check_accessibility", json!({ "path": "tagged-issues.pdf" }));
    assert_eq!(acc["data"]["tagged"], true);
    assert!(acc["data"]["failed"].as_u64().unwrap_or(0) >= 3, "{acc}");
    let found = c.ok("find_sensitive_text", json!({ "path": "sensitive-info.pdf", "kinds": ["email", "iban"] }));
    let kinds: Vec<&str> =
        found["data"]["hits"].as_array().expect("hits").iter().map(|h| h["kind"].as_str().unwrap_or("")).collect();
    assert_eq!(kinds, ["email", "iban", "email"], "{found}");
    assert_eq!(
        c.err("find_sensitive_text", json!({ "path": "sensitive-info.pdf", "kinds": ["passwords"] })),
        "mcp.arguments"
    );
    assert_eq!(c.err("find_sensitive_text", json!({ "path": "sensitive-info.pdf" })), "pattern.none");
    let cmp = c.ok("compare_documents", json!({ "old": "compare-old.pdf", "new": "compare-new.pdf" }));
    assert!(cmp["data"].is_object(), "{cmp}");
    let sig = c.ok("check_signatures", json!({ "path": "signed-rsa.pdf" }));
    assert!(sig["data"]["signatures"].as_array().is_some_and(|s| !s.is_empty()), "{sig}");

    // Writing is refused even when asked for directly.
    assert_eq!(
        c.err("extract_pages", json!({ "path": "five-pages.pdf", "pages": "1", "output": "x.pdf" })),
        "mcp.write_not_allowed"
    );
    assert!(!root.path().join("x.pdf").exists());
}

#[test]
fn paths_outside_the_folders_and_bad_requests_are_refused() {
    let root = tempfile::tempdir().expect("tmp");
    let outside = tempfile::tempdir().expect("tmp");
    copy_fixtures(root.path(), &["five-pages.pdf"]);
    copy_fixtures(outside.path(), &["single-page.pdf"]);
    let mut c = Client::start(&[root.path()], true);

    let secret = outside.path().join("single-page.pdf");
    assert_eq!(c.err("inspect_document", json!({ "path": secret })), "mcp.path_denied");
    assert_eq!(c.err("inspect_document", json!({ "path": "../x/../../etc/passwd" })), "io.not_found");
    assert_eq!(
        c.err(
            "extract_pages",
            json!({ "path": "five-pages.pdf", "pages": "1", "output": outside.path().join("o.pdf") })
        ),
        "mcp.path_denied"
    );
    assert_eq!(
        c.err("extract_pages", json!({ "path": "five-pages.pdf", "pages": "1", "output": "five-pages.pdf" })),
        "output.exists"
    );
    assert_eq!(
        c.err("extract_pages", json!({ "path": "five-pages.pdf", "pages": "1", "output": "o.exe" })),
        "mcp.output_extension"
    );
    assert_eq!(c.err("inspect_document", json!({ "path": "five-pages.pdf", "shell": "rm -rf /" })), "mcp.arguments");
    assert_eq!(
        c.err("extract_pages", json!({ "path": "five-pages.pdf", "pages": "9", "output": "o.pdf" })),
        "range.out_of_bounds"
    );
    assert!(std::fs::read_dir(outside.path()).expect("ls").count() == 1, "nothing written outside");

    let r = c.request("tools/call", json!({ "name": "run_shell", "arguments": {} }));
    assert_eq!(r["error"]["code"], -32602);
    let r = c.request("no/such/method", json!({}));
    assert_eq!(r["error"]["code"], -32601);
    assert_eq!(c.request("ping", json!({}))["result"], json!({}));
    c.raw("{not json");
    assert_eq!(c.next_message()["error"]["code"], -32700);
    c.raw(&format!("{{\"jsonrpc\":\"2.0\",\"id\":99,\"method\":\"ping\",\"pad\":\"{}\"}}", "x".repeat(1_100_000)));
    assert_eq!(c.next_message()["error"]["code"], -32600, "oversized message refused");
    // Still serving afterwards.
    assert_eq!(c.ok("inspect_document", json!({ "path": "five-pages.pdf" }))["data"]["page_count"], 5);
}

#[test]
fn write_tools_create_verified_new_files_and_leave_sources_alone() {
    let root = tempfile::tempdir().expect("tmp");
    copy_fixtures(
        root.path(),
        &["five-pages.pdf", "twenty-pages.pdf", "form-fields.pdf", "uncompressed-report.pdf", "tables.pdf"],
    );
    std::fs::create_dir(root.path().join("parts")).expect("mkdir");
    let before = std::fs::read(root.path().join("five-pages.pdf")).expect("read");
    let mut c = Client::start(&[root.path()], true);
    let names = tool_names(&mut c);
    assert_eq!(names.len(), 23, "{names:?}");

    let r = c.ok("extract_pages", json!({ "path": "five-pages.pdf", "pages": "4, 2", "output": "ex.pdf" }));
    assert_eq!(r["outputs"][0]["pages"], 2);
    assert_eq!(page_count(&root.path().join("ex.pdf")), 2);
    c.ok("delete_pages", json!({ "path": "five-pages.pdf", "pages": "1-2", "output": "del.pdf" }));
    assert_eq!(page_count(&root.path().join("del.pdf")), 3);
    c.ok("rotate_pages", json!({ "path": "five-pages.pdf", "degrees": 90, "output": "rot.pdf" }));
    let rot = lopdf::Document::load(root.path().join("rot.pdf")).expect("rot");
    for (_, id) in rot.get_pages() {
        assert_eq!(rot.get_dictionary(id).expect("page").get(b"Rotate").and_then(|o| o.as_i64()).ok(), Some(90));
    }
    c.ok("merge_documents", json!({ "paths": ["five-pages.pdf", "ex.pdf"], "output": "merged.pdf" }));
    assert_eq!(page_count(&root.path().join("merged.pdf")), 7);
    let s = c.ok("split_document", json!({ "path": "twenty-pages.pdf", "every_n": 7, "output_dir": "parts" }));
    assert_eq!(s["outputs"].as_array().map(Vec::len), Some(3));
    assert_eq!(
        c.err(
            "split_document",
            json!({ "path": "twenty-pages.pdf", "every_n": 7, "max_bytes": 20000, "output_dir": "parts" })
        ),
        "mcp.split_mode"
    );
    c.ok("fill_form", json!({ "path": "form-fields.pdf", "values": { "full_name": "Ada", "subscribe": true }, "output": "filled.pdf" }));
    let f = c.ok("list_form_fields", json!({ "path": "filled.pdf" }));
    assert_eq!(f["data"]["fields"][0]["value"], "Ada");
    let o = c.ok("reduce_file_size", json!({ "path": "uncompressed-report.pdf", "output": "small.pdf" }));
    assert!(o["data"]["bytes_after"].as_u64() < o["data"]["bytes_before"].as_u64(), "{o}");
    c.ok("export_text", json!({ "path": "five-pages.pdf", "pages": "3", "output": "p3.txt" }));
    assert!(std::fs::read_to_string(root.path().join("p3.txt")).expect("txt").contains("PWMARK003"));
    assert_eq!(c.err("export_text", json!({ "path": "five-pages.pdf", "output": "p3.pdf" })), "mcp.output_extension");
    let w = c.ok("convert_to_docx", json!({ "path": "five-pages.pdf", "pages": "2-3", "output": "two.docx" }));
    assert_eq!(w["data"]["pages"], 2, "{w}");
    assert!(zip::ZipArchive::new(std::fs::File::open(root.path().join("two.docx")).expect("open")).is_ok());
    assert_eq!(
        c.err("convert_to_docx", json!({ "path": "five-pages.pdf", "output": "two.pdf" })),
        "mcp.output_extension"
    );
    let p = c.ok("convert_to_pptx", json!({ "path": "five-pages.pdf", "pages": "1", "output": "one.pptx" }));
    assert_eq!(p["data"]["pages"], 1, "{p}");
    std::fs::write(root.path().join("notes.txt"), "PWMCPOFFICE notes").expect("write");
    if pw_pdf::office::find_office().is_some() {
        let o = c.ok("convert_office_to_pdf", json!({ "path": "notes.txt", "output": "notes.pdf" }));
        assert_eq!(o["outputs"][0]["pages"], 1, "{o}");
    } else {
        assert_eq!(
            c.err("convert_office_to_pdf", json!({ "path": "notes.txt", "output": "notes.pdf" })),
            "office.not_installed"
        );
    }
    if pw_speech::engine_info().is_ok() {
        let a = c.ok(
            "read_aloud_to_wav",
            json!({ "path": "five-pages.pdf", "pages": "1", "rate": 1.5, "output": "p1.wav" }),
        );
        assert_eq!(a["data"]["pages"], 1, "{a}");
        assert!(std::fs::read(root.path().join("p1.wav")).expect("wav").starts_with(b"RIFF"));
    } else {
        assert!(std::env::var("PW_SKIP_SPEECH").as_deref() == Ok("1"), "a speech engine is required for this test");
    }
    assert_eq!(
        c.err("read_aloud_to_wav", json!({ "path": "five-pages.pdf", "output": "p1.mp3" })),
        "mcp.output_extension"
    );
    let t = c.ok("export_tables", json!({ "path": "tables.pdf", "output": "t.xlsx" }));
    assert_eq!(t["data"]["tables"], 2, "{t}");
    assert_eq!(c.err("export_tables", json!({ "path": "five-pages.pdf", "output": "n.xlsx" })), "tables.none");

    assert_eq!(std::fs::read(root.path().join("five-pages.pdf")).expect("read"), before, "source unchanged");
}

#[test]
fn recognize_text_reports_progress_and_stops_when_cancelled() {
    if pw_ocr::default_resource_dir().is_none() {
        assert!(std::env::var_os("CI").is_none(), "OCR resources missing in CI");
        eprintln!("OCR resources not installed; skipping");
        return;
    }
    let root = tempfile::tempdir().expect("tmp");
    copy_fixtures(root.path(), &["scanned.pdf"]);
    let mut c = Client::start(&[root.path()], true);

    // With a progress token: progress notifications, then a searchable copy.
    let id = c.send("tools/call", json!({ "name": "recognize_text", "arguments": { "path": "scanned.pdf", "output": "ocr.pdf" }, "_meta": { "progressToken": "p1" } }));
    let r = c.wait(id);
    assert_eq!(r["result"]["isError"], false, "{r}");
    let progress: Vec<&Value> = c.notes.iter().filter(|n| n["method"] == "notifications/progress").collect();
    assert!(!progress.is_empty(), "progress was reported");
    assert!(progress.iter().all(|p| p["params"]["progressToken"] == "p1"));
    let hits = c.ok("search_text", json!({ "path": "ocr.pdf", "query": "the" }));
    assert!(hits["data"]["count"].as_u64().unwrap_or(0) > 0, "OCR text is searchable: {hits}");

    // A larger job, cancelled after it starts: no output is left behind.
    let paths: Vec<&str> = std::iter::repeat_n("scanned.pdf", 6).collect();
    c.ok("merge_documents", json!({ "paths": paths, "output": "big.pdf" }));
    let id = c.send("tools/call", json!({ "name": "recognize_text", "arguments": { "path": "big.pdf", "output": "big-ocr.pdf" }, "_meta": { "progressToken": 7 } }));
    loop {
        let m = c.next_message();
        if m["method"] == "notifications/progress" {
            break;
        }
        assert_ne!(m.get("id"), Some(&json!(id)), "finished before it could be cancelled: {m}");
    }
    c.notify("notifications/cancelled", json!({ "requestId": id, "reason": "test" }));
    let r = c.wait(id);
    assert_eq!(r["result"]["isError"], true, "{r}");
    assert_eq!(r["result"]["structuredContent"]["error"]["category"], "cancelled");
    assert!(!root.path().join("big-ocr.pdf").exists(), "nothing committed after cancel");
}

#[test]
fn many_quick_calls_in_flight_all_get_their_results() {
    // Regression: finished calls used to prune every finished job, so a quick job that completed
    // before its own waiter looked was lost ("The operation was lost.").
    let root = tempfile::tempdir().expect("tmp");
    copy_fixtures(root.path(), &["five-pages.pdf", "form-fields.pdf"]);
    let mut c = Client::start(&[root.path()], false);
    for _round in 0..5 {
        let ids: Vec<u64> = (0..8)
            .map(|i| {
                let (name, path) = if i % 2 == 0 {
                    ("list_form_fields", "form-fields.pdf")
                } else {
                    ("inspect_document", "five-pages.pdf")
                };
                c.send("tools/call", json!({ "name": name, "arguments": { "path": path } }))
            })
            .collect();
        for id in ids {
            let r = c.wait(id);
            assert_eq!(r["result"]["isError"], false, "{r}");
        }
    }
}
