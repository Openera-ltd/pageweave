//! Signature verification against fixtures signed by an independent tool
//! (tools/fixtures/gen_signed.py; cross-checked with `openssl cms -verify` when generated).
use pw_pdf::{verify_signatures, OpenOptions, SignatureStatus};
use std::path::{Path, PathBuf};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/pdf").join(name)
}

fn check(name: &str) -> pw_pdf::SignatureReport {
    verify_signatures(&fixture(name), &OpenOptions { password: None }).expect("verify")
}

#[test]
fn untouched_rsa_and_ecdsa_signatures_are_valid() {
    for (name, alg) in
        [("signed-rsa.pdf", "RSA PKCS#1 v1.5 with SHA-256"), ("signed-ecdsa.pdf", "ECDSA P-256 with SHA-256")]
    {
        let r = check(name);
        assert_eq!(r.signatures.len(), 1, "{name}");
        let s = &r.signatures[0];
        assert_eq!(s.status, SignatureStatus::Valid, "{name}: {}", s.detail);
        assert_eq!(s.signature_algorithm.as_deref(), Some(alg));
        assert_eq!(s.digest_algorithm.as_deref(), Some("SHA-256"));
        assert_eq!(s.signer.as_deref(), Some("PageWeave Test Signer"));
        assert!(s.issuer.as_deref().unwrap_or_default().contains("PageWeave Test CA"), "{:?}", s.issuer);
        assert!(!s.self_signed);
        assert!(s.covers_whole_file && !s.changed_after);
        assert_eq!((s.field.as_deref(), s.page), (Some("Approval"), Some(0)));
        assert_eq!((s.reason.as_deref(), s.location.as_deref()), (Some("Approval"), Some("Test lab")));
        assert!(s.signing_time.as_deref().unwrap_or_default().starts_with("20"), "{:?}", s.signing_time);
        assert_eq!(s.certificate_valid_from.as_deref(), Some("2026-01-01T00:00:00Z"));
        assert_eq!(s.trust, "not_checked");
        assert!(r.trust_note.contains("does not yet check whether the signer is trusted"));
    }
}

#[test]
fn changes_after_signing_are_reported_and_tampering_is_invalid() {
    let edited = &check("signed-then-edited.pdf").signatures[0];
    assert_eq!(edited.status, SignatureStatus::Valid, "the signed revision itself is intact");
    assert!(edited.changed_after && !edited.covers_whole_file);

    let tampered = &check("signed-tampered.pdf").signatures[0];
    assert_eq!(tampered.status, SignatureStatus::Invalid);
    assert!(tampered.detail.contains("changed after it was signed"), "{}", tampered.detail);
}

#[test]
fn placeholder_signatures_are_unreadable_and_unsigned_files_have_none() {
    let r = check("signature-structure.pdf");
    assert_eq!(r.signatures.len(), 1);
    assert_ne!(r.signatures[0].status, SignatureStatus::Valid, "a placeholder is never valid");
    assert!(check("five-pages.pdf").signatures.is_empty());
    assert!(verify_signatures(&fixture("not-a-pdf.pdf"), &OpenOptions { password: None }).is_err());
}

#[test]
fn a_flipped_byte_in_the_signature_itself_is_caught() {
    // Change one hex digit inside the CMS signature value (near the end of the blob, which holds
    // the signer's signature bytes) while keeping the file structure intact.
    let raw = std::fs::read(fixture("signed-rsa.pdf")).expect("read");
    let start = raw.windows(11).position(|w| w == b"/Contents <").expect("contents") + 11;
    let end = start + raw[start..].iter().position(|&b| b == b'>').expect("end");
    let hex = &raw[start..end];
    let used = hex.len() - hex.iter().rev().take_while(|&&b| b == b'0').count();
    let mut bad = raw.clone();
    let i = start + used - 10;
    bad[i] = if bad[i] == b'a' { b'b' } else { b'a' };
    let dir = tempfile::tempdir().expect("tmp");
    let p = dir.path().join("bad-sig.pdf");
    std::fs::write(&p, bad).expect("write");
    let s = &verify_signatures(&p, &OpenOptions { password: None }).expect("verify").signatures[0];
    // The content digest still matches; only the public-key check can reject this.
    assert_eq!(s.status, SignatureStatus::Invalid, "{}", s.detail);
    assert!(s.detail.contains("does not match the signer's certificate"), "{}", s.detail);
}
