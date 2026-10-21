//! Accessibility check against fixtures built to pass every automated check, to fail the usual ones,
//! and against untagged, scanned, long and password-protected files.

use pw_core::ops::{Operation, OperationRequest};
use pw_pdf::{execute, AccessibilityReport, CheckStatus};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/pdf").join(name)
}

fn report_with(name: &str, password: Option<&str>) -> AccessibilityReport {
    let mut req = OperationRequest::new(Operation::CheckAccessibility { input: fixture(name) });
    req.password = password.map(str::to_string);
    serde_json::from_value(execute(&req, None).expect("check").data).expect("report")
}

fn statuses(r: &AccessibilityReport) -> HashMap<&str, CheckStatus> {
    r.checks.iter().map(|c| (c.id.as_str(), c.status)).collect()
}

#[test]
fn a_well_made_tagged_file_passes_every_automated_check() {
    let r = report_with("tagged-accessible.pdf", None);
    assert!(r.tagged);
    let automated: Vec<_> = r.checks.iter().filter(|c| c.status != CheckStatus::Manual).collect();
    for c in &automated {
        assert!(matches!(c.status, CheckStatus::Pass | CheckStatus::NotApplicable), "{c:?}");
    }
    let s = statuses(&r);
    for id in [
        "tagged",
        "title",
        "title_shown",
        "language",
        "text",
        "fonts_embedded",
        "figures_alt",
        "table_headers",
        "headings",
        "form_labels",
        "links_described",
        "tab_order",
    ] {
        assert_eq!(s[id], CheckStatus::Pass, "{id}");
    }
    assert_eq!(s["bookmarks"], CheckStatus::NotApplicable, "one page");
    assert_eq!((r.failed, r.warnings, r.manual), (0, 0, 4));
    assert!(r.checks.iter().any(|c| c.id == "language" && c.detail.contains("en-US")));
}

#[test]
fn the_usual_problems_are_each_reported_with_where() {
    let r = report_with("tagged-issues.pdf", None);
    assert!(r.tagged);
    let s = statuses(&r);
    assert_eq!(s["language"], CheckStatus::Fail);
    assert_eq!(s["title_shown"], CheckStatus::Warning);
    assert_eq!(s["figures_alt"], CheckStatus::Fail);
    assert_eq!(s["table_headers"], CheckStatus::Warning);
    assert_eq!(s["form_labels"], CheckStatus::Fail);
    assert_eq!(s["links_described"], CheckStatus::Warning);
    assert_eq!(s["tab_order"], CheckStatus::Warning);
    assert_eq!(s["fonts_embedded"], CheckStatus::Warning);
    assert_eq!(s["title"], CheckStatus::Pass);
    assert_eq!(s["headings"], CheckStatus::Pass);
    let by = |id: &str| r.checks.iter().find(|c| c.id == id).expect(id);
    assert_eq!(by("figures_alt").pages, [0]);
    assert_eq!(by("form_labels").names, ["comments"]);
    assert_eq!(by("fonts_embedded").names, ["Helvetica"]);
}

#[test]
fn untagged_scanned_and_long_files() {
    let s = statuses(&report_with("five-pages.pdf", None))
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect::<HashMap<_, _>>();
    assert_eq!(s["tagged"], CheckStatus::Fail);
    assert_eq!(s["figures_alt"], CheckStatus::NotApplicable, "needs tags");
    assert_eq!(s["form_labels"], CheckStatus::NotApplicable);
    let scanned = report_with("scanned.pdf", None);
    let text = scanned.checks.iter().find(|c| c.id == "text").expect("text");
    assert_eq!(text.status, CheckStatus::Fail);
    assert_eq!(text.pages, [0, 1, 2]);
    let long = statuses(&report_with("twenty-pages.pdf", None)).get("bookmarks").copied();
    assert_eq!(long, Some(CheckStatus::NotApplicable), "exactly 20 pages");
    // Form without labels in an untagged file: still checked.
    let form = report_with("form-fields.pdf", None);
    assert_eq!(statuses(&form)["form_labels"], CheckStatus::Pass, "the fixture's fields all have /TU");
}

#[test]
fn protected_files_are_checked_with_their_password() {
    let r = report_with("encrypted-rc4-user-password.pdf", Some("pw-user-test"));
    assert!(!r.checks.is_empty());
    assert_eq!(statuses(&r)["tagged"], CheckStatus::Fail);
    let mut req =
        OperationRequest::new(Operation::CheckAccessibility { input: fixture("encrypted-rc4-user-password.pdf") });
    req.password = None;
    assert_eq!(execute(&req, None).expect_err("needs password").category, pw_core::ErrorCategory::PasswordRequired);
}
