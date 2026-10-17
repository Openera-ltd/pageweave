//! Removing password protection against the real engine; owner restrictions are honoured.
use pw_core::ops::{Operation, OperationRequest, ProtectPermissions};
use pw_core::ErrorCategory;
use pw_pdf::{execute, open_document, OpenOptions};
use std::path::{Path, PathBuf};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/pdf").join(name)
}

const RC4_PW: &str = "pw-user-test";

fn unprotect(input: &Path, output: &Path, pw: Option<&str>) -> pw_core::PwResult<pw_core::ops::OperationResult> {
    let mut req = OperationRequest::new(Operation::Unprotect { input: input.into(), output: output.into() });
    req.password = pw.map(Into::into);
    execute(&req, None)
}

fn protect(
    input: &Path,
    output: &Path,
    user: &str,
    owner: Option<&str>,
    perms: ProtectPermissions,
    current: Option<&str>,
) {
    let mut req =
        OperationRequest::new(Operation::Protect { input: input.into(), permissions: perms, output: output.into() });
    req.new_password = Some(user.into());
    req.new_owner_password = owner.map(Into::into);
    req.password = current.map(Into::into);
    execute(&req, None).expect("protect");
}

fn text(path: &Path, pw: Option<&str>, page: u32) -> String {
    let mut req = OperationRequest::new(Operation::ExtractText { input: path.into(), pages: Some(format!("{page}")) });
    req.password = pw.map(Into::into);
    execute(&req, None).expect("text").data["pages"][0]["text"].as_str().unwrap_or_default().to_string()
}

#[test]
fn unprotected_copy_opens_without_a_password_and_keeps_content() {
    let src = fixture("encrypted-rc4-user-password.pdf");
    let before = std::fs::read(&src).expect("read");
    let dir = tempfile::tempdir().expect("tmp");
    let out = dir.path().join("open.pdf");
    let r = unprotect(&src, &out, Some(RC4_PW)).expect("unprotect");
    assert_eq!(r.outputs[0].pages, Some(5));
    assert_eq!(std::fs::read(&src).expect("read"), before, "source untouched");
    let info = open_document(&out, &OpenOptions { password: None }).expect("opens without a password");
    assert!(!info.encrypted);
    assert_eq!(info.page_count, 5);
    for p in 1..=5 {
        assert_eq!(text(&out, None, p), text(&src, Some(RC4_PW), p));
    }
    let doc = lopdf::Document::load(&out).expect("independent parser");
    assert!(doc.trailer.get(b"Encrypt").is_err());
}

#[test]
fn bookmarks_and_properties_survive_removing_and_changing_a_password() {
    let src = fixture("five-pages.pdf");
    let dir = tempfile::tempdir().expect("tmp");
    let a = dir.path().join("a.pdf");
    protect(&src, &a, "first", None, ProtectPermissions::default(), None);
    // Re-protecting a protected file keeps bookmarks and title (PDFium decrypts the whole document).
    let b = dir.path().join("b.pdf");
    protect(&a, &b, "second", None, ProtectPermissions::default(), Some("first"));
    let info = open_document(&b, &OpenOptions { password: Some("second") }).expect("new password");
    assert_eq!(info.bookmarks.len(), 5);
    assert_eq!(info.title.as_deref(), Some("Five pages"));
    assert!(open_document(&b, &OpenOptions { password: Some("first") }).is_err(), "old password no longer works");
    // Removing the password keeps them too.
    let c = dir.path().join("c.pdf");
    unprotect(&b, &c, Some("second")).expect("unprotect");
    let info = open_document(&c, &OpenOptions { password: None }).expect("open");
    assert_eq!(info.bookmarks.len(), 5);
    assert_eq!(info.title.as_deref(), Some("Five pages"));
}

#[test]
fn owner_restrictions_need_the_permissions_password() {
    let src = fixture("five-pages.pdf");
    let dir = tempfile::tempdir().expect("tmp");
    let locked = dir.path().join("restricted.pdf");
    let perms = ProtectPermissions { print: false, modify: false, ..ProtectPermissions::default() };
    protect(&src, &locked, "reader", Some("owner"), perms, None);

    let out = dir.path().join("out.pdf");
    let err = unprotect(&locked, &out, Some("reader")).expect_err("restricted");
    assert_eq!(err.code, "pdf.owner_restricted");
    assert_eq!(err.category, ErrorCategory::Permission);
    assert!(!out.exists());
    // Re-protecting with the open password would lift the restrictions too: refused.
    let mut req = OperationRequest::new(Operation::Protect {
        input: locked.clone(),
        permissions: ProtectPermissions::default(),
        output: out.clone(),
    });
    req.new_password = Some("mine".into());
    req.password = Some("reader".into());
    assert_eq!(execute(&req, None).expect_err("restricted").code, "pdf.owner_restricted");
    assert!(!out.exists());
    // The permissions password grants it.
    unprotect(&locked, &out, Some("owner")).expect("owner may remove protection");
    assert!(!open_document(&out, &OpenOptions { password: None }).expect("open").encrypted);
}

#[test]
fn unprotected_wrong_password_and_conflicts_write_nothing() {
    let dir = tempfile::tempdir().expect("tmp");
    let out = dir.path().join("x.pdf");
    let err = unprotect(&fixture("five-pages.pdf"), &out, None).expect_err("not protected");
    assert_eq!(err.code, "unprotect.not_protected");
    let err = unprotect(&fixture("encrypted-rc4-user-password.pdf"), &out, Some("wrong")).expect_err("wrong");
    assert_eq!(err.category, ErrorCategory::PasswordRequired);
    assert!(!out.exists());
    std::fs::write(&out, b"keep").expect("write");
    let err = unprotect(&fixture("encrypted-rc4-user-password.pdf"), &out, Some(RC4_PW)).expect_err("exists");
    assert_eq!(err.category, ErrorCategory::OutputConflict);
    assert_eq!(std::fs::read(&out).expect("read"), b"keep");
}
