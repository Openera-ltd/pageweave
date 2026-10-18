//! Signing with a certificate. Identities: fixtures/keys (tools/fixtures/gen_test_identity.py,
//! TEST-ONLY keys, password "pageweave-test"). Every signed file is checked by PageWeave's own
//! verifier and independently with `openssl cms -verify` against the test CA.
use pw_core::ops::{Operation, OperationRequest, OperationResult, SignPlacement};
use pw_pdf::execute;
use std::path::{Path, PathBuf};
use std::process::Command;

const PASSWORD: &str = "pageweave-test";

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/pdf").join(name)
}

fn key(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/keys").join(name)
}

fn sign(
    input: &Path,
    identity: &str,
    password: &str,
    placement: Option<SignPlacement>,
    output: &Path,
) -> pw_core::PwResult<OperationResult> {
    let mut req = OperationRequest::new(Operation::Sign {
        input: input.into(),
        certificate: key(identity),
        reason: Some("Approved".into()),
        location: Some("Test lab".into()),
        placement,
        output: output.into(),
    });
    req.certificate_password = Some(password.into());
    req.validate()?;
    execute(&req, None)
}

fn statuses(p: &Path) -> Vec<(String, bool)> {
    let r = execute(&OperationRequest::new(Operation::VerifySignatures { input: p.into() }), None).expect("verify");
    r.data["signatures"]
        .as_array()
        .expect("signatures")
        .iter()
        .map(|s| (s["status"].as_str().unwrap_or("?").to_string(), s["covers_whole_file"] == true))
        .collect()
}

/// Independent check of the last signature with OpenSSL, chained to the test CA.
fn openssl_verifies(pdf: &Path, work: &Path) {
    let bytes = std::fs::read(pdf).expect("read");
    let text = String::from_utf8_lossy(&bytes);
    let at = text.rfind("/ByteRange").expect("byte range");
    let open = at + text[at..].find('[').expect("[");
    let close = open + text[open..].find(']').expect("]");
    let n: Vec<usize> = text[open + 1..close].split_whitespace().map(|v| v.parse().expect("int")).collect();
    let signed = [&bytes[n[0]..n[0] + n[1]], &bytes[n[2]..n[2] + n[3]]].concat();
    let hex = String::from_utf8_lossy(&bytes[n[0] + n[1] + 1..n[2] - 1]).to_string();
    let der: Vec<u8> =
        (0..hex.len()).step_by(2).map(|i| u8::from_str_radix(&hex[i..i + 2], 16).expect("hex")).collect();
    // The reserved space is zero-padded after the DER.
    let len =
        4 + match der[1] {
            0x82 => (der[2] as usize) << 8 | der[3] as usize,
            0x83 => (der[2] as usize) << 16 | (der[3] as usize) << 8 | der[4] as usize,
            other => panic!("unexpected length form {other:#x}"),
        } + usize::from(der[1] == 0x83);
    std::fs::write(work.join("sig.p7s"), &der[..len]).expect("write");
    std::fs::write(work.join("signed.bin"), signed).expect("write");
    let ca = work.join("ca.pem");
    let x = Command::new("openssl")
        .args(["x509", "-inform", "DER", "-in"])
        .arg(key("test-ca.der"))
        .arg("-out")
        .arg(&ca)
        .status();
    let Ok(status) = x else {
        // OpenSSL is part of the CI images (and Git for Windows); require it unless explicitly waived.
        assert!(
            std::env::var_os("PAGEWEAVE_NO_OPENSSL").is_some(),
            "openssl is required for the independent signature check"
        );
        eprintln!("note: independent OpenSSL check waived by PAGEWEAVE_NO_OPENSSL");
        return;
    };
    assert!(status.success());
    let out = Command::new("openssl")
        .args(["cms", "-verify", "-inform", "DER", "-binary", "-purpose", "any", "-in"])
        .arg(work.join("sig.p7s"))
        .arg("-content")
        .arg(work.join("signed.bin"))
        .arg("-CAfile")
        .arg(&ca)
        .args(["-out"])
        .arg(work.join("ignored.out"))
        .output()
        .expect("openssl");
    assert!(out.status.success(), "openssl: {}", String::from_utf8_lossy(&out.stderr));
}

#[test]
fn rsa_visible_signature_is_valid_independently_and_keeps_the_original() {
    let dir = tempfile::tempdir().expect("tmp");
    let src = fixture("five-pages.pdf");
    let before = std::fs::read(&src).expect("read");
    let out = dir.path().join("signed.pdf");
    let r = sign(
        &src,
        "test-signer-rsa.p12",
        PASSWORD,
        Some(SignPlacement { page: 0, rect: [350.0, 40.0, 560.0, 100.0] }),
        &out,
    )
    .expect("sign");
    assert_eq!(r.data["signer"], "PageWeave Test Signer");
    assert_eq!(r.data["algorithm"], "RSA with SHA-256");
    assert_eq!(std::fs::read(&src).expect("read"), before, "source untouched");
    let signed = std::fs::read(&out).expect("read");
    assert!(signed.starts_with(&before), "incremental update: the original bytes come first");
    assert_eq!(statuses(&out), vec![("valid".to_string(), true)]);
    openssl_verifies(&out, dir.path());
    // The visible box is a widget on page 1 with an appearance.
    let doc = lopdf::Document::load(&out).expect("lopdf");
    let page = doc.get_pages()[&1];
    let annots =
        doc.get_dictionary(page).expect("page").get(b"Annots").expect("annots").as_array().expect("array").clone();
    let w = doc.get_dictionary(annots.last().expect("widget").as_reference().expect("ref")).expect("widget");
    assert!(w.get(b"AP").is_ok());
    let appended = &signed[before.len()..];
    assert!(appended.windows(5).any(|w| w == b"/Prev"), "the appended revision points back to the original");
}

#[test]
fn ecdsa_legacy_and_xref_stream_files_sign_invisibly() {
    let dir = tempfile::tempdir().expect("tmp");
    let e = dir.path().join("ecdsa.pdf");
    let r = sign(&fixture("single-page.pdf"), "test-signer-ecdsa.p12", PASSWORD, None, &e).expect("ecdsa");
    assert_eq!(r.data["algorithm"], "ECDSA P-256 with SHA-256");
    assert_eq!(r.data["visible"], false);
    assert_eq!(statuses(&e), vec![("valid".to_string(), true)]);
    openssl_verifies(&e, dir.path());
    // A PKCS#12 encrypted the old way (3DES) still opens.
    let l = dir.path().join("legacy.pdf");
    sign(&fixture("single-page.pdf"), "test-signer-rsa-legacy.p12", PASSWORD, None, &l).expect("legacy");
    assert_eq!(statuses(&l), vec![("valid".to_string(), true)]);
    // A file with object streams and a cross-reference stream (as Reduce file size writes).
    let small = dir.path().join("small.pdf");
    execute(
        &OperationRequest::new(Operation::Optimize {
            input: fixture("five-pages.pdf"),
            images: None,
            output: small.clone(),
        }),
        None,
    )
    .expect("optimize");
    assert!(std::fs::read(&small).expect("read").windows(10).any(|w| w == b"/Type/XRef" || w == b"/Type /XRe"));
    let x = dir.path().join("xref-stream-signed.pdf");
    sign(
        &small,
        "test-signer-rsa.p12",
        PASSWORD,
        Some(SignPlacement { page: 2, rect: [72.0, 72.0, 272.0, 132.0] }),
        &x,
    )
    .expect("sign");
    assert_eq!(statuses(&x), vec![("valid".to_string(), true)]);
    openssl_verifies(&x, dir.path());
}

#[test]
fn earlier_signatures_stay_valid_when_signing_again() {
    let dir = tempfile::tempdir().expect("tmp");
    // A file signed by another tool (Python cryptography fixtures), then signed by PageWeave twice.
    let once = dir.path().join("once.pdf");
    sign(&fixture("signed-rsa.pdf"), "test-signer-ecdsa.p12", PASSWORD, None, &once).expect("second signature");
    let s = statuses(&once);
    assert_eq!(s.len(), 2);
    assert_eq!(
        s[0],
        ("valid".to_string(), false),
        "the earlier signature is valid and no longer covers the whole file"
    );
    assert_eq!(s[1], ("valid".to_string(), true));
    let twice = dir.path().join("twice.pdf");
    let r = sign(&once, "test-signer-rsa.p12", PASSWORD, None, &twice).expect("third signature");
    assert_eq!(r.data["field"], "Signature2", "a new, unused field name");
    assert!(statuses(&twice).iter().all(|(st, _)| st == "valid"));
}

#[test]
fn refusals_write_nothing() {
    let dir = tempfile::tempdir().expect("tmp");
    let out = dir.path().join("o.pdf");
    let code = |r: pw_core::PwResult<OperationResult>| r.expect_err("refused").code;
    assert_eq!(
        code(sign(&fixture("five-pages.pdf"), "test-signer-rsa.p12", "wrong", None, &out)),
        "sign.wrong_password"
    );
    assert_eq!(
        code(sign(&fixture("five-pages.pdf"), "test-signer-expired.p12", PASSWORD, None, &out)),
        "sign.cert_expired"
    );
    assert_eq!(code(sign(&fixture("five-pages.pdf"), "test-ca.der", PASSWORD, None, &out)), "sign.identity_unreadable");
    assert_eq!(
        code(sign(
            &fixture("five-pages.pdf"),
            "test-signer-rsa.p12",
            PASSWORD,
            Some(SignPlacement { page: 9, rect: [0.0, 0.0, 100.0, 50.0] }),
            &out
        )),
        "sign.page"
    );
    assert_eq!(
        code(sign(
            &fixture("five-pages.pdf"),
            "test-signer-rsa.p12",
            PASSWORD,
            Some(SignPlacement { page: 0, rect: [0.0, 0.0, 5.0, 5.0] }),
            &out
        )),
        "sign.box"
    );
    assert!(sign(&fixture("encrypted-rc4-user-password.pdf"), "test-signer-rsa.p12", PASSWORD, None, &out).is_err());
    assert!(sign(&fixture("corrupt-truncated.pdf"), "test-signer-rsa.p12", PASSWORD, None, &out).is_err());
    // No password at all is refused before anything is read.
    let mut req = OperationRequest::new(Operation::Sign {
        input: fixture("five-pages.pdf"),
        certificate: key("test-signer-rsa.p12"),
        reason: None,
        location: None,
        placement: None,
        output: out.clone(),
    });
    assert_eq!(req.validate().expect_err("no password").code, "sign.password_missing");
    // And the password never appears in the serialized request (job journal, logs).
    req.certificate_password = Some(PASSWORD.into());
    assert!(!serde_json::to_string(&req).expect("json").contains(PASSWORD));
    assert_eq!(std::fs::read_dir(dir.path()).expect("dir").count(), 0);
}
