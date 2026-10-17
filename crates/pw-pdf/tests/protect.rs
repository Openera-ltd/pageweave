//! Password protection against the real engine, checked independently with lopdf.
use pw_core::ops::{Operation, OperationRequest, ProtectPermissions};
use pw_core::ErrorCategory;
use pw_pdf::{execute, open_document, OpenOptions};
use std::path::{Path, PathBuf};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/pdf").join(name)
}

fn protect(
    input: &Path,
    output: &Path,
    user: &str,
    owner: Option<&str>,
    perms: ProtectPermissions,
    current: Option<&str>,
) -> pw_core::PwResult<pw_core::ops::OperationResult> {
    let mut req =
        OperationRequest::new(Operation::Protect { input: input.into(), permissions: perms, output: output.into() });
    req.new_password = Some(user.into());
    req.new_owner_password = owner.map(Into::into);
    req.password = current.map(Into::into);
    execute(&req, None)
}

fn text(path: &Path, pw: Option<&str>, page: u32) -> String {
    let mut req =
        OperationRequest::new(Operation::ExtractText { input: path.into(), pages: Some(format!("{}", page + 1)) });
    req.password = pw.map(Into::into);
    execute(&req, None).expect("text").data["pages"][0]["text"].as_str().unwrap_or_default().to_string()
}

#[test]
fn protected_copy_needs_the_password_and_keeps_content() {
    let src = fixture("five-pages.pdf");
    let before = std::fs::read(&src).expect("read");
    let dir = tempfile::tempdir().expect("tmp");
    let out = dir.path().join("locked.pdf");
    let perms = ProtectPermissions { print: false, copy: false, ..ProtectPermissions::default() };
    let r = protect(&src, &out, "open-1 ünï", Some("owner-1"), perms, None).expect("protect");
    assert_eq!(r.outputs[0].pages, Some(5));
    assert!(r.warnings.iter().any(|w| w.contains("not absolute security")), "{:?}", r.warnings);
    assert_eq!(std::fs::read(&src).expect("read"), before, "source untouched");

    let err = open_document(&out, &OpenOptions { password: None }).expect_err("needs a password");
    assert_eq!(err.category, ErrorCategory::PasswordRequired);
    let err = open_document(&out, &OpenOptions { password: Some("wrong") }).expect_err("wrong password");
    assert_eq!(err.category, ErrorCategory::PasswordRequired);
    for pw in ["open-1 ünï", "owner-1"] {
        let info = open_document(&out, &OpenOptions { password: Some(pw) }).expect("opens");
        assert_eq!(info.page_count, 5);
        assert!(info.encrypted);
    }
    assert_eq!(text(&out, Some("open-1 ünï"), 3), text(&src, None, 3));

    // Independent parser: AES-256 (V5/R6), print and copy bits cleared, others set; decrypts.
    let mut doc = lopdf::Document::load(&out).expect("lopdf");
    let enc = doc
        .trailer
        .get(b"Encrypt")
        .and_then(|o| o.as_reference())
        .and_then(|id| doc.get_dictionary(id))
        .expect("Encrypt")
        .clone();
    assert_eq!(enc.get(b"V").and_then(|v| v.as_i64()).ok(), Some(5));
    assert_eq!(enc.get(b"R").and_then(|v| v.as_i64()).ok(), Some(6));
    let p = enc.get(b"P").and_then(|v| v.as_i64()).expect("P") as u32;
    assert_eq!(p & (1 << 2), 0, "print off");
    assert_eq!(p & (1 << 4), 0, "copy off");
    assert_ne!(p & (1 << 3), 0, "modify on");
    assert_ne!(p & (1 << 9), 0, "accessibility extraction stays on");
    assert!(doc.clone().decrypt("wrong").is_err());
    doc.decrypt("open-1 ünï").expect("decrypts with the open password");
    // The plaintext is not readable in the file without decrypting.
    assert!(!std::fs::read(&out).expect("read").windows(9).any(|w| w == b"PWMARK004"));
}

#[test]
fn a_protected_source_gets_the_new_password_only() {
    let src = fixture("encrypted-rc4-user-password.pdf");
    let dir = tempfile::tempdir().expect("tmp");
    let out = dir.path().join("re.pdf");
    protect(&src, &out, "new-pw", None, ProtectPermissions::default(), Some("pw-user-test")).expect("re-protect");
    assert!(
        open_document(&out, &OpenOptions { password: Some("pw-user-test") }).is_err(),
        "old password no longer opens it"
    );
    assert!(open_document(&out, &OpenOptions { password: Some("new-pw") }).is_ok());
    assert_eq!(text(&out, Some("new-pw"), 0), text(&src, Some("pw-user-test"), 0));
}

#[test]
fn bad_requests_write_nothing() {
    let dir = tempfile::tempdir().expect("tmp");
    let out = dir.path().join("x.pdf");
    let e =
        protect(&fixture("five-pages.pdf"), &out, "", None, ProtectPermissions::default(), None).expect_err("empty");
    assert_eq!(e.code, "protect.password_empty");
    let e = protect(&fixture("five-pages.pdf"), &out, "a", Some("a"), ProtectPermissions::default(), None)
        .expect_err("same");
    assert_eq!(e.code, "protect.same_passwords");
    let e = protect(
        &fixture("encrypted-rc4-user-password.pdf"),
        &out,
        "a",
        None,
        ProtectPermissions::default(),
        Some("wrong"),
    )
    .expect_err("wrong current");
    assert_eq!(e.category, ErrorCategory::PasswordRequired);
    assert_eq!(std::fs::read_dir(dir.path()).expect("dir").count(), 0);
}
