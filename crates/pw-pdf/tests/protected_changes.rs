//! Changes to password-protected files (D-010, D-042): allowed when the file's permissions allow
//! them, refused otherwise; new PDFs keep the open password and the source's restrictions.
use pw_core::ops::{BookmarkSpec, Operation, OperationRequest, OperationResult, ProtectPermissions};
use pw_core::PwResult;
use pw_pdf::{execute, open_document, OpenOptions};
use std::path::{Path, PathBuf};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/pdf").join(name)
}

fn protect(input: &Path, output: &Path, user: &str, owner: Option<&str>, perms: ProtectPermissions) {
    let mut req =
        OperationRequest::new(Operation::Protect { input: input.into(), permissions: perms, output: output.into() });
    req.new_password = Some(user.into());
    req.new_owner_password = owner.map(Into::into);
    execute(&req, None).expect("protect");
}

fn run(op: Operation, password: &str, keep: bool) -> PwResult<OperationResult> {
    let mut req = OperationRequest::new(op);
    req.password = Some(password.into());
    req.keep_protection = keep;
    execute(&req, None)
}

fn title(op_out: &Path, password: &str) -> Option<String> {
    open_document(op_out, &OpenOptions { password: Some(password) }).expect("open").title
}

fn properties(input: &Path, out: &Path, title: &str) -> Operation {
    Operation::SetProperties {
        input: input.into(),
        title: Some(title.into()),
        author: None,
        subject: None,
        keywords: None,
        output: out.into(),
    }
}

#[test]
fn an_unrestricted_protected_file_is_changed_and_stays_protected() {
    let dir = tempfile::tempdir().expect("tmp");
    let src = dir.path().join("p.pdf");
    protect(&fixture("five-pages.pdf"), &src, "open-pw", None, ProtectPermissions::default());
    // Properties: changed, and the copy needs the same password.
    let out = dir.path().join("props.pdf");
    let r = run(properties(&src, &out, "Changed title"), "open-pw", true).expect("properties");
    assert!(r.warnings.iter().any(|w| w.contains("same open password")), "{:?}", r.warnings);
    assert_eq!(open_document(&out, &OpenOptions::default()).expect_err("protected").code, "pdf.password");
    assert_eq!(title(&out, "open-pw").as_deref(), Some("Changed title"));
    // Bookmarks.
    let out = dir.path().join("marks.pdf");
    run(
        Operation::SetBookmarks {
            input: src.clone(),
            bookmarks: vec![BookmarkSpec { title: "Start".into(), page: 0, depth: 0 }],
            output: out.clone(),
        },
        "open-pw",
        true,
    )
    .expect("bookmarks");
    let info = open_document(&out, &OpenOptions { password: Some("open-pw") }).expect("open");
    assert_eq!(info.bookmarks.iter().map(|b| b.title.as_str()).collect::<Vec<_>>(), ["Start"]);
    // Without keep_protection (the API default) the copy is plain and the warning says so.
    let out = dir.path().join("plain.pdf");
    let r = run(properties(&src, &out, "Plain"), "open-pw", false).expect("properties");
    assert!(r.warnings.iter().any(|w| w.contains("not password protected")), "{:?}", r.warnings);
    assert!(open_document(&out, &OpenOptions::default()).is_ok());
}

#[test]
fn a_restricted_file_refuses_changes_its_owner_did_not_allow_and_copies_keep_restrictions() {
    let dir = tempfile::tempdir().expect("tmp");
    let src = dir.path().join("r.pdf");
    let perms = ProtectPermissions { print: false, modify: false, ..ProtectPermissions::default() };
    protect(&fixture("five-pages.pdf"), &src, "reader", Some("owner"), perms);
    // With the open password only: changing is refused, nothing written.
    let out = dir.path().join("x.pdf");
    let e = run(properties(&src, &out, "Nope"), "reader", true).expect_err("restricted");
    assert_eq!(e.code, "pdf.owner_restricted");
    assert!(!out.exists());
    // With the permissions password: allowed.
    let out = dir.path().join("owner.pdf");
    run(properties(&src, &out, "By the owner"), "owner", true).expect("owner may change it");
    assert_eq!(title(&out, "owner").as_deref(), Some("By the owner"));
    // Pages taken out with the open password keep the password and the restrictions (no printing,
    // no changes), so the copy does not lift what the owner set.
    let pages = dir.path().join("pages.pdf");
    let r =
        run(Operation::ExtractPages { input: src.clone(), pages: "1-2".into(), output: pages.clone() }, "reader", true)
            .expect("extract");
    assert!(r.warnings.iter().any(|w| w.contains("same restrictions")), "{:?}", r.warnings);
    let again = dir.path().join("again.pdf");
    assert_eq!(
        run(properties(&pages, &again, "Nope"), "reader", true).expect_err("still restricted").code,
        "pdf.owner_restricted"
    );
    let unprotect = run(Operation::Unprotect { input: pages, output: dir.path().join("u.pdf") }, "reader", false);
    assert_eq!(unprotect.expect_err("restrictions carried over").code, "pdf.owner_restricted");
}

#[test]
fn remove_password_is_not_protected_again_by_keep_protection() {
    // The app sets keep_protection on every request for a protected document; Remove password
    // must still give a copy without a password.
    let dir = tempfile::tempdir().expect("tmp");
    let src = dir.path().join("p.pdf");
    protect(&fixture("five-pages.pdf"), &src, "open-pw", None, ProtectPermissions::default());
    let out = dir.path().join("open.pdf");
    run(Operation::Unprotect { input: src, output: out.clone() }, "open-pw", true).expect("unprotect");
    assert!(open_document(&out, &OpenOptions::default()).is_ok(), "opens without a password");
}
