//! Signer trust: chains from the signature's certificates to certificates the user trusts.
//! Identities: fixtures/keys (tools/fixtures/gen_test_identity.py; TEST-ONLY).
use pw_core::ops::{Operation, OperationRequest};
use pw_pdf::{execute, TrustStatus};
use std::path::{Path, PathBuf};

const PASSWORD: &str = "pageweave-test";

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/pdf").join(name)
}

fn key(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/keys").join(name)
}

fn signed(dir: &Path, identity: &str) -> PathBuf {
    let out = dir.join(format!("{identity}.pdf"));
    let mut req = OperationRequest::new(Operation::Sign {
        input: fixture("single-page.pdf"),
        certificate: key(identity),
        reason: None,
        location: None,
        placement: None,
        output: out.clone(),
    });
    req.certificate_password = Some(PASSWORD.into());
    execute(&req, None).expect("sign");
    out
}

fn trust(pdf: &Path, trusted: Vec<PathBuf>) -> (TrustStatus, serde_json::Value) {
    let r = execute(&OperationRequest::new(Operation::VerifySignatures { input: pdf.into(), trusted }), None)
        .expect("verify");
    let s = r.data["signatures"][0].clone();
    assert_eq!(s["status"], "valid", "integrity first: {s}");
    assert_eq!(s["revocation"], "not_checked");
    let t = match s["trust"].as_str().unwrap_or("") {
        "trusted" => TrustStatus::Trusted,
        "unknown_signer" => TrustStatus::UnknownSigner,
        "expired" => TrustStatus::Expired,
        "chain_invalid" => TrustStatus::ChainInvalid,
        _ => TrustStatus::NotChecked,
    };
    (t, s)
}

#[test]
fn trusted_only_through_a_verified_chain_to_a_chosen_certificate() {
    let dir = tempfile::tempdir().expect("tmp");
    let pdf = signed(dir.path(), "test-signer-rsa.p12");
    let (t, s) = trust(&pdf, vec![key("test-ca.der")]);
    assert_eq!(t, TrustStatus::Trusted, "{s}");
    assert_eq!(s["trusted_by"], "PageWeave Test CA");
    assert_eq!(s["chain"], serde_json::json!(["PageWeave Test Signer", "PageWeave Test CA"]));
    let (t, s) = trust(&pdf, vec![]);
    assert_eq!(t, TrustStatus::UnknownSigner, "{s}");
    assert!(s["trust_detail"].as_str().unwrap_or("").contains("root you have not chosen to trust"));
    // ECDSA signer, same CA.
    let e = signed(dir.path(), "test-signer-ecdsa.p12");
    assert_eq!(trust(&e, vec![key("test-ca.der")]).0, TrustStatus::Trusted);
}

#[test]
fn a_different_ca_with_the_same_name_is_not_trusted() {
    // signed-rsa.pdf was issued by another throwaway "PageWeave Test CA" (same name, other key).
    let (t, s) = trust(&fixture("signed-rsa.pdf"), vec![key("test-ca.der")]);
    assert_eq!(t, TrustStatus::UnknownSigner, "{s}");
}

#[test]
fn broken_and_expired_chains_are_reported() {
    let dir = tempfile::tempdir().expect("tmp");
    let not_ca = signed(dir.path(), "test-signer-not-ca.p12");
    let (t, s) = trust(&not_ca, vec![key("test-ca.der")]);
    assert_eq!(t, TrustStatus::ChainInvalid, "{s}");
    assert!(s["trust_detail"].as_str().unwrap_or("").contains("not a certificate authority"));
    let old = signed(dir.path(), "test-signer-old-issuer.p12");
    let (t, s) = trust(&old, vec![key("test-ca.der")]);
    assert_eq!(t, TrustStatus::Expired, "{s}");
    assert!(s["trust_detail"].as_str().unwrap_or("").contains("PageWeave Test Old Intermediate"));
}

#[test]
fn trusting_the_signer_directly_and_pem_bundles() {
    let dir = tempfile::tempdir().expect("tmp");
    let pdf = signed(dir.path(), "test-signer-rsa.p12");
    // The signer's own certificate, taken from its identity, as a PEM file (with another cert first).
    let store = p12_keystore::KeyStore::from_pkcs12(
        &std::fs::read(key("test-signer-rsa.p12")).expect("read"),
        PASSWORD,
        p12_keystore::Pkcs12ImportPolicy::Relaxed,
    )
    .expect("p12");
    let (_, chain) = store.private_key_chain().expect("chain");
    let pem = |der: &[u8]| {
        let b64 = base64_encode(der);
        let lines: Vec<String> = b64.as_bytes().chunks(64).map(|c| String::from_utf8_lossy(c).into_owned()).collect();
        format!("-----BEGIN CERTIFICATE-----\n{}\n-----END CERTIFICATE-----\n", lines.join("\n"))
    };
    let other = std::fs::read(key("test-ca.der")).expect("ca");
    // The signer's certificate alone is enough to trust it.
    let signer_only = dir.path().join("signer.pem");
    std::fs::write(&signer_only, pem(chain.certs()[0].as_der())).expect("write");
    let (t, s) = trust(&pdf, vec![signer_only]);
    assert_eq!(t, TrustStatus::Trusted, "{s}");
    assert_eq!(s["trusted_by"], "PageWeave Test Signer");
    // A PEM file may hold several certificates.
    let bundle = dir.path().join("bundle.pem");
    std::fs::write(&bundle, format!("{}{}", pem(&other), pem(chain.certs()[0].as_der()))).expect("write");
    assert_eq!(pw_pdf::load_trusted_certificates(&[bundle]).expect("bundle").len(), 2);
    let junk = dir.path().join("junk.cer");
    std::fs::write(&junk, b"not a certificate").expect("write");
    let e =
        execute(&OperationRequest::new(Operation::VerifySignatures { input: pdf.clone(), trusted: vec![junk] }), None)
            .expect_err("unreadable trust file");
    assert_eq!(e.code, "trust.unreadable");
}

fn base64_encode(d: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::new();
    for c in d.chunks(3) {
        let n =
            (u32::from(c[0]) << 16) | (u32::from(*c.get(1).unwrap_or(&0)) << 8) | u32::from(*c.get(2).unwrap_or(&0));
        for i in 0..4 {
            if i <= c.len() {
                s.push(T[((n >> (18 - 6 * i)) & 63) as usize] as char);
            } else {
                s.push('=');
            }
        }
    }
    s
}

#[test]
fn the_chain_offered_for_trust_is_signer_then_verified_issuers() {
    let dir = tempfile::tempdir().expect("tmp");
    let pdf = signed(dir.path(), "test-signer-rsa.p12");
    let chain = pw_pdf::signature_chain(&pdf, 0).expect("chain");
    let names: Vec<&str> = chain.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, vec!["PageWeave Test Signer", "PageWeave Test CA"]);
    assert!(!chain[0].is_ca && chain[1].is_ca && chain[1].self_signed);
    let ca = pw_pdf::parse_certificates(&std::fs::read(key("test-ca.der")).expect("ca"));
    assert_eq!(chain[1].fingerprint, pw_pdf::fingerprint(&ca[0]));
    assert_eq!(chain[1].fingerprint.len(), 32 * 3 - 1);
    assert!(pw_pdf::signature_chain(&fixture("five-pages.pdf"), 0).is_err());
}
