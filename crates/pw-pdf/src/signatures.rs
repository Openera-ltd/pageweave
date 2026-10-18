//! Digital signature verification (PRD 05 "signatures"; launch checklist "signature trust statuses
//! honest and tested"). For each signature: the signed byte ranges are hashed, the CMS signer's
//! signed attributes are checked against that hash, and the signer's signature over them is
//! verified with the certificate's public key (RSA PKCS#1 v1.5, ECDSA P-256/P-384). Changes made
//! after a signature (later revisions) are reported.
//!
//! Trust: a valid signature's certificate chain is built from the certificates in the signature to
//! a certificate the user chose to trust (PageWeave's trusted-certificates list, or `--trust` on the
//! CLI); every link's signature, validity period and CA constraint is checked. The operating
//! system's root store is not used. Revocation (CRL/OCSP) and timestamps are not checked, and every
//! result says so. RSA verification is implemented here with a public-key modpow (decision D-011)
//! rather than a crate with an open advisory; only public operations exist in this module.

use lopdf::{Dictionary, Document, Object, ObjectId};
use num_bigint::BigUint;
use pw_core::{ErrorCategory, PwError, PwResult};
use serde::Serialize;
use sha1::Sha1;
use sha2::{Digest, Sha256, Sha384, Sha512};
use std::collections::HashMap;
use std::path::Path;

use cms::content_info::ContentInfo;
use cms::signed_data::{SignedData, SignerIdentifier, SignerInfo};
use der::asn1::{ObjectIdentifier as Oid, OctetString};
use der::{Decode, Encode};
use x509_cert::Certificate;

/// What PageWeave can say about one signature.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SignatureStatus {
    /// The signed content is unchanged and the signature matches the signer's certificate.
    Valid,
    /// The signed content changed, or the signature does not match the certificate.
    Invalid,
    /// A kind of signature this version cannot check.
    Unsupported,
    /// The signature data could not be read.
    Unreadable,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SignatureInfo {
    pub field: Option<String>,
    /// 0-based page of the signature field's widget, when it has one.
    pub page: Option<u32>,
    pub status: SignatureStatus,
    /// Why, in one sentence.
    pub detail: String,
    /// The signer's common name (or full subject when there is none).
    pub signer: Option<String>,
    pub signer_subject: Option<String>,
    pub issuer: Option<String>,
    pub certificate_valid_from: Option<String>,
    pub certificate_valid_to: Option<String>,
    pub self_signed: bool,
    pub signing_time: Option<String>,
    pub reason: Option<String>,
    pub location: Option<String>,
    pub sub_filter: Option<String>,
    pub digest_algorithm: Option<String>,
    pub signature_algorithm: Option<String>,
    /// The signature covers the file as it was when signed, and nothing was added afterwards.
    pub covers_whole_file: bool,
    /// Bytes were added after the signed revision (later edits, other signatures, annotations).
    pub changed_after: bool,
    pub has_timestamp: bool,
    /// Whether the signer chains to a certificate the user trusts (only for valid signatures).
    pub trust: TrustStatus,
    /// One sentence on the trust result.
    pub trust_detail: String,
    /// The trusted certificate the chain ends at, when trusted.
    pub trusted_by: Option<String>,
    /// Names along the chain, signer first.
    pub chain: Vec<String>,
    /// Always "not_checked": certificate revocation (CRL/OCSP) needs the network and is not done.
    pub revocation: &'static str,
}

/// What PageWeave can say about the signer of an integrity-valid signature.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TrustStatus {
    /// The signature itself is not valid (or could not be checked), so trust is not evaluated.
    NotChecked,
    /// The chain reaches a certificate the user trusts and every link checks out.
    Trusted,
    /// No chain to a trusted certificate: the signer is unknown.
    UnknownSigner,
    /// A certificate in the chain has expired (or is not valid yet).
    Expired,
    /// The chain is broken: an issuer's signature does not match, or an issuer is not a CA.
    ChainInvalid,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SignatureReport {
    pub signatures: Vec<SignatureInfo>,
    /// Shown with every report: what "valid" does and does not mean here.
    pub trust_note: &'static str,
}

pub const TRUST_NOTE: &str = "PageWeave checks that signed content is unchanged and matches the signer's certificate, and whether the signer's certificate chains to one you chose to trust. Revocation (whether a certificate was withdrawn) and timestamps are not checked.";

/// Nesting limit for certificate chains.
const MAX_CHAIN: usize = 8;

/// Read trusted certificates (DER, or PEM with one or more certificates) from files.
pub fn load_trusted_certificates(paths: &[std::path::PathBuf]) -> PwResult<Vec<Certificate>> {
    let mut out = Vec::new();
    for p in paths {
        let bytes = std::fs::read(p).map_err(|e| PwError::from_io(&e, "trusted certificate"))?;
        let found = parse_certificates(&bytes);
        if found.is_empty() {
            return Err(PwError::new(
                ErrorCategory::InvalidInput,
                "trust.unreadable",
                format!(
                    "{} is not a certificate (DER or PEM).",
                    p.file_name().map(|f| f.to_string_lossy()).unwrap_or_default()
                ),
            ));
        }
        out.extend(found);
    }
    Ok(out)
}

/// Certificates in DER or PEM bytes.
pub fn parse_certificates(bytes: &[u8]) -> Vec<Certificate> {
    if let Ok(c) = Certificate::from_der(bytes) {
        return vec![c];
    }
    let text = String::from_utf8_lossy(bytes);
    let mut out = Vec::new();
    let (begin, end) = ("-----BEGIN CERTIFICATE-----", "-----END CERTIFICATE-----");
    let mut rest = text.as_ref();
    while let Some(i) = rest.find(begin) {
        let after = &rest[i + begin.len()..];
        let Some(j) = after.find(end) else { break };
        let b64: String = after[..j].chars().filter(|c| !c.is_whitespace()).collect();
        if let Some(der) = base64_decode(&b64) {
            if let Ok(c) = Certificate::from_der(&der) {
                out.push(c);
            }
        }
        rest = &after[j + end.len()..];
    }
    out
}

fn base64_decode(s: &str) -> Option<Vec<u8>> {
    let val = |c: u8| -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        } as u32)
    };
    let bytes = s.trim_end_matches('=').as_bytes();
    let mut out = Vec::with_capacity(bytes.len() * 3 / 4);
    for chunk in bytes.chunks(4) {
        let mut acc = 0u32;
        for (i, &c) in chunk.iter().enumerate() {
            acc |= val(c)? << (18 - 6 * i);
        }
        let n = chunk.len();
        if n < 2 {
            return None;
        }
        out.push((acc >> 16) as u8);
        if n > 2 {
            out.push((acc >> 8) as u8);
        }
        if n > 3 {
            out.push(acc as u8);
        }
    }
    Some(out)
}

/// SHA-256 fingerprint of a certificate, as uppercase hex pairs separated by colons.
pub fn fingerprint(c: &Certificate) -> String {
    let der = c.to_der().unwrap_or_default();
    Sha256::digest(&der).iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(":")
}

/// Does `issuer`'s key verify `cert`'s signature?
fn issued_by(cert: &Certificate, issuer: &Certificate) -> bool {
    if cert.tbs_certificate.issuer != issuer.tbs_certificate.subject {
        return false;
    }
    let Ok(tbs) = cert.tbs_certificate.to_der() else { return false };
    let sig = cert.signature.raw_bytes();
    let spki = &issuer.tbs_certificate.subject_public_key_info;
    let key = spki.subject_public_key.raw_bytes();
    let hash = match cert.signature_algorithm.oid.to_string().as_str() {
        "1.2.840.113549.1.1.5" => Hash::Sha1,
        "1.2.840.113549.1.1.11" | "1.2.840.10045.4.3.2" => Hash::Sha256,
        "1.2.840.113549.1.1.12" | "1.2.840.10045.4.3.3" => Hash::Sha384,
        "1.2.840.113549.1.1.13" | "1.2.840.10045.4.3.4" => Hash::Sha512,
        _ => return false,
    };
    let digest = hash.digest(&[&tbs]);
    match spki.algorithm.oid.to_string().as_str() {
        "1.2.840.113549.1.1.1" => {
            cert.signature_algorithm.oid.to_string().starts_with("1.2.840.113549.1.1.")
                && rsa_pkcs1_verify(key, hash, &digest, sig)
        }
        "1.2.840.10045.2.1" => {
            let curve = spki
                .algorithm
                .parameters
                .as_ref()
                .and_then(|p| p.decode_as::<Oid>().ok())
                .map(|o| o.to_string())
                .unwrap_or_default();
            cert.signature_algorithm.oid.to_string().starts_with("1.2.840.10045.4.3.")
                && ecdsa_verify(&curve, key, &digest, sig)
        }
        _ => false,
    }
}

fn is_ca(c: &Certificate) -> bool {
    c.tbs_certificate.extensions.as_ref().is_some_and(|exts| {
        exts.iter().any(|e| {
            e.extn_id.to_string() == "2.5.29.19"
                && x509_cert::ext::pkix::BasicConstraints::from_der(e.extn_value.as_bytes()).is_ok_and(|b| b.ca)
        })
    })
}

fn same(a: &Certificate, b: &Certificate) -> bool {
    a.tbs_certificate.subject == b.tbs_certificate.subject
        && a.tbs_certificate.subject_public_key_info == b.tbs_certificate.subject_public_key_info
}

fn label(c: &Certificate) -> String {
    common_name(&c.tbs_certificate.subject).unwrap_or_else(|| c.tbs_certificate.subject.to_string())
}

/// Build and check the chain from `signer` to a trusted certificate, at time `now` (Unix seconds).
fn evaluate_trust(
    signer: &Certificate,
    pool: &[Certificate],
    anchors: &[Certificate],
    now: i64,
) -> (TrustStatus, String, Option<String>, Vec<String>) {
    let mut chain = vec![label(signer)];
    let valid_at = |c: &Certificate| {
        let v = &c.tbs_certificate.validity;
        let (from, to) =
            (v.not_before.to_unix_duration().as_secs() as i64, v.not_after.to_unix_duration().as_secs() as i64);
        from <= now && now <= to
    };
    let mut current = signer;
    for depth in 0..MAX_CHAIN {
        if !valid_at(current) {
            let to = current.tbs_certificate.validity.not_after.to_date_time();
            return (
                TrustStatus::Expired,
                format!("The certificate of {} is not valid today (valid to {}).", label(current), to),
                None,
                chain,
            );
        }
        if let Some(a) = anchors.iter().find(|a| same(a, current)) {
            return (
                TrustStatus::Trusted,
                format!("The signer's certificate chains to {}, which you trust.", label(a)),
                Some(label(a)),
                chain,
            );
        }
        let issuer = pool.iter().chain(anchors.iter()).find(|c| !same(c, current) && issued_by(current, c));
        let Some(issuer) = issuer else {
            let self_signed = issued_by(current, current);
            let what = if depth == 0 && self_signed {
                "The signer's certificate is self-signed and you have not chosen to trust it."
            } else if self_signed {
                "The signer's certificate chain ends at a root you have not chosen to trust."
            } else {
                "The signer's certificate does not chain to a certificate you trust (its issuer is not included)."
            };
            return (TrustStatus::UnknownSigner, what.into(), None, chain);
        };
        if !is_ca(issuer) {
            return (
                TrustStatus::ChainInvalid,
                format!("{} issued a certificate but is not a certificate authority.", label(issuer)),
                None,
                chain,
            );
        }
        chain.push(label(issuer));
        current = issuer;
    }
    (TrustStatus::ChainInvalid, "The certificate chain is too long to check.".into(), None, chain)
}

// ---- small DER helpers -------------------------------------------------------------------------

/// Length of the DER element at the start of `b` (header + content), if well formed.
fn der_len(b: &[u8]) -> Option<usize> {
    let first = *b.get(1)?;
    if first < 0x80 {
        return Some(2 + usize::from(first));
    }
    let n = usize::from(first & 0x7f);
    if n == 0 || n > 4 {
        return None; // indefinite (BER) or absurd lengths
    }
    let mut len = 0usize;
    for i in 0..n {
        len = (len << 8) | usize::from(*b.get(2 + i)?);
    }
    Some(2 + n + len)
}

/// Splits one TLV off `b`: (tag, content, rest).
fn tlv(b: &[u8]) -> Option<(u8, &[u8], &[u8])> {
    let total = der_len(b)?;
    let header = total - {
        let first = b[1];
        if first < 0x80 {
            usize::from(first)
        } else {
            let n = usize::from(first & 0x7f);
            let mut len = 0usize;
            for i in 0..n {
                len = (len << 8) | usize::from(b[2 + i]);
            }
            len
        }
    };
    Some((b[0], b.get(header..total)?, b.get(total..)?))
}

// ---- algorithms --------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Hash {
    Sha1,
    Sha256,
    Sha384,
    Sha512,
}

impl Hash {
    fn from_oid(oid: &Oid) -> Option<Hash> {
        Some(match oid.to_string().as_str() {
            "1.3.14.3.2.26" => Hash::Sha1,
            "2.16.840.1.101.3.4.2.1" => Hash::Sha256,
            "2.16.840.1.101.3.4.2.2" => Hash::Sha384,
            "2.16.840.1.101.3.4.2.3" => Hash::Sha512,
            _ => return None,
        })
    }

    fn name(self) -> &'static str {
        match self {
            Hash::Sha1 => "SHA-1",
            Hash::Sha256 => "SHA-256",
            Hash::Sha384 => "SHA-384",
            Hash::Sha512 => "SHA-512",
        }
    }

    fn digest(self, parts: &[&[u8]]) -> Vec<u8> {
        fn run<D: Digest>(parts: &[&[u8]]) -> Vec<u8> {
            let mut h = D::new();
            for p in parts {
                h.update(p);
            }
            h.finalize().to_vec()
        }
        match self {
            Hash::Sha1 => run::<Sha1>(parts),
            Hash::Sha256 => run::<Sha256>(parts),
            Hash::Sha384 => run::<Sha384>(parts),
            Hash::Sha512 => run::<Sha512>(parts),
        }
    }

    /// DER DigestInfo prefix (RFC 8017 §9.2 note 1), with the NULL parameters.
    fn digest_info_prefix(self) -> &'static [u8] {
        match self {
            Hash::Sha1 => &[0x30, 0x21, 0x30, 0x09, 0x06, 0x05, 0x2b, 0x0e, 0x03, 0x02, 0x1a, 0x05, 0x00, 0x04, 0x14],
            Hash::Sha256 => &[
                0x30, 0x31, 0x30, 0x0d, 0x06, 0x09, 0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02, 0x01, 0x05, 0x00,
                0x04, 0x20,
            ],
            Hash::Sha384 => &[
                0x30, 0x41, 0x30, 0x0d, 0x06, 0x09, 0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02, 0x02, 0x05, 0x00,
                0x04, 0x30,
            ],
            Hash::Sha512 => &[
                0x30, 0x51, 0x30, 0x0d, 0x06, 0x09, 0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02, 0x03, 0x05, 0x00,
                0x04, 0x40,
            ],
        }
    }
}

/// RSASSA-PKCS1-v1_5 verification (RFC 8017 §8.2.2) with the public key only.
fn rsa_pkcs1_verify(spki_key: &[u8], hash: Hash, digest: &[u8], signature: &[u8]) -> bool {
    // RSAPublicKey ::= SEQUENCE { modulus INTEGER, publicExponent INTEGER }
    let Some((0x30, body, _)) = tlv(spki_key) else { return false };
    let Some((0x02, n, rest)) = tlv(body) else { return false };
    let Some((0x02, e, _)) = tlv(rest) else { return false };
    let n = BigUint::from_bytes_be(n);
    let e = BigUint::from_bytes_be(e);
    let k = n.to_bytes_be().len();
    if signature.len() != k || k < 64 {
        return false;
    }
    let s = BigUint::from_bytes_be(signature);
    if s >= n {
        return false;
    }
    let m = s.modpow(&e, &n).to_bytes_be();
    let mut em = vec![0u8; k - m.len()];
    em.extend_from_slice(&m);
    // EM = 0x00 0x01 PS(0xff, at least 8) 0x00 T, with T = DigestInfo; also accept T without the
    // NULL parameters, which some signers emit.
    let with_null: Vec<u8> = [hash.digest_info_prefix(), digest].concat();
    let without_null = {
        let p = hash.digest_info_prefix();
        let mut t = vec![0x30, p[1] - 2, 0x30, p[3] - 2];
        t.extend_from_slice(&p[4..p[3] as usize + 2]);
        t.extend_from_slice(&p[p.len() - 2..]);
        t.extend_from_slice(digest);
        t
    };
    [with_null, without_null].iter().any(|t| {
        if k < t.len() + 11 {
            return false;
        }
        let ps = k - t.len() - 3;
        em[0] == 0 && em[1] == 1 && em[2..2 + ps].iter().all(|&b| b == 0xff) && em[2 + ps] == 0 && em[3 + ps..] == t[..]
    })
}

fn ecdsa_verify(curve: &str, point: &[u8], digest: &[u8], signature: &[u8]) -> bool {
    use p256::ecdsa::signature::hazmat::PrehashVerifier;
    match curve {
        "1.2.840.10045.3.1.7" => {
            let (Ok(key), Ok(sig)) =
                (p256::ecdsa::VerifyingKey::from_sec1_bytes(point), p256::ecdsa::Signature::from_der(signature))
            else {
                return false;
            };
            key.verify_prehash(digest, &sig).is_ok()
        }
        "1.3.132.0.34" => {
            let (Ok(key), Ok(sig)) =
                (p384::ecdsa::VerifyingKey::from_sec1_bytes(point), p384::ecdsa::Signature::from_der(signature))
            else {
                return false;
            };
            key.verify_prehash(digest, &sig).is_ok()
        }
        _ => false,
    }
}

// ---- certificate helpers -----------------------------------------------------------------------

fn common_name(name: &x509_cert::name::Name) -> Option<String> {
    const CN: &str = "2.5.4.3";
    for rdn in name.0.iter() {
        for atv in rdn.0.iter() {
            if atv.oid.to_string() == CN {
                let v = &atv.value;
                if let Ok(s) = v.decode_as::<der::asn1::Utf8StringRef>() {
                    return Some(s.as_str().to_string());
                }
                if let Ok(s) = v.decode_as::<der::asn1::PrintableStringRef>() {
                    return Some(s.as_str().to_string());
                }
                if let Ok(s) = v.decode_as::<der::asn1::Ia5StringRef>() {
                    return Some(s.as_str().to_string());
                }
            }
        }
    }
    None
}

fn signer_certificate<'a>(sd: &'a SignedData, si: &SignerInfo) -> Option<&'a Certificate> {
    let certs = sd.certificates.as_ref()?;
    certs.0.iter().find_map(|c| {
        let cms::cert::CertificateChoices::Certificate(cert) = c else { return None };
        let tbs = &cert.tbs_certificate;
        let hit = match &si.sid {
            SignerIdentifier::IssuerAndSerialNumber(ias) => {
                tbs.issuer == ias.issuer && tbs.serial_number == ias.serial_number
            }
            SignerIdentifier::SubjectKeyIdentifier(ski) => tbs.extensions.iter().flatten().any(|ext| {
                ext.extn_id.to_string() == "2.5.29.14"
                    && OctetString::from_der(ext.extn_value.as_bytes()).is_ok_and(|v| v.as_bytes() == ski.0.as_bytes())
            }),
        };
        hit.then_some(cert)
    })
}

fn attribute<'a>(si: &'a SignerInfo, oid: &str) -> Option<&'a der::Any> {
    si.signed_attrs.as_ref()?.iter().find(|a| a.oid.to_string() == oid).and_then(|a| a.values.iter().next())
}

// ---- reading signatures out of the file --------------------------------------------------------

fn text(o: Option<&Object>) -> Option<String> {
    o.and_then(|o| lopdf::decode_text_string(o).ok()).map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

fn ints(o: Option<&Object>) -> Option<Vec<i64>> {
    o?.as_array().ok()?.iter().map(|x| x.as_i64().ok()).collect()
}

/// Signature dictionaries with the name of the field and the page of its widget.
fn signature_dicts(doc: &Document) -> Vec<(ObjectId, Option<String>, Option<u32>)> {
    let page_of: HashMap<ObjectId, u32> = doc.get_pages().into_iter().map(|(n, id)| (id, n - 1)).collect();
    let mut by_sig: HashMap<ObjectId, (Option<String>, Option<u32>)> = HashMap::new();
    // Widgets / fields whose value is a signature dictionary.
    for o in doc.objects.values() {
        let Object::Dictionary(d) = o else { continue };
        let Ok(v) = d.get(b"V").and_then(|v| v.as_reference()) else { continue };
        if !is_sig(doc, v) {
            continue;
        }
        let page = d.get(b"P").and_then(|p| p.as_reference()).ok().and_then(|p| page_of.get(&p).copied());
        let entry = by_sig.entry(v).or_insert((None, None));
        entry.0 = entry.0.take().or_else(|| text(d.get(b"T").ok()));
        entry.1 = entry.1.or(page);
    }
    // Signature dictionaries no field points to are still reported.
    for (id, o) in doc.objects.iter() {
        if matches!(o, Object::Dictionary(_)) && is_sig(doc, *id) {
            by_sig.entry(*id).or_insert((None, None));
        }
    }
    let mut out: Vec<(ObjectId, Option<String>, Option<u32>)> =
        by_sig.into_iter().map(|(id, (f, p))| (id, f, p)).collect();
    // In file order of their signed ranges (first signature first).
    out.sort_by_key(|(id, _, _)| {
        doc.get_dictionary(*id)
            .ok()
            .and_then(|d| ints(d.get(b"ByteRange").ok()))
            .and_then(|r| r.get(1).copied())
            .unwrap_or(i64::MAX)
    });
    out
}

fn is_sig(doc: &Document, id: ObjectId) -> bool {
    doc.get_dictionary(id)
        .is_ok_and(|d| d.has(b"ByteRange") && d.has(b"Contents") && (d.has(b"Filter") || d.has(b"SubFilter")))
}

// ---- verification ------------------------------------------------------------------------------

struct Outcome {
    status: SignatureStatus,
    detail: String,
}

fn outcome(status: SignatureStatus, detail: impl Into<String>) -> Outcome {
    Outcome { status, detail: detail.into() }
}

fn check(raw: &[u8], d: &Dictionary, info: &mut SignatureInfo, anchors: &[Certificate]) -> Outcome {
    let sub = info.sub_filter.clone().unwrap_or_default();
    match sub.as_str() {
        "adbe.pkcs7.detached" | "ETSI.CAdES.detached" | "adbe.pkcs7.sha1" => {}
        "ETSI.RFC3161" => {
            return outcome(
                SignatureStatus::Unsupported,
                "This is a document timestamp; timestamps cannot be checked yet.",
            )
        }
        "" => return outcome(SignatureStatus::Unreadable, "The signature does not say what kind of signature it is."),
        other => {
            return outcome(SignatureStatus::Unsupported, format!("Signatures of type {other} cannot be checked yet."))
        }
    }
    // The byte ranges: two pieces around the /Contents hex string, which must be exactly the gap.
    let Some(r) = ints(d.get(b"ByteRange").ok()).filter(|r| r.len() == 4 && r.iter().all(|&x| x >= 0)) else {
        return outcome(SignatureStatus::Unreadable, "The signed byte ranges are missing or malformed.");
    };
    let (a, b, c, e) = (r[0] as usize, r[1] as usize, r[2] as usize, r[3] as usize);
    let ok_ranges = a == 0 && b <= c && c.checked_add(e).is_some_and(|end| end <= raw.len());
    if !ok_ranges || raw.get(b) != Some(&b'<') || raw.get(c.wrapping_sub(1)) != Some(&b'>') {
        return outcome(SignatureStatus::Invalid, "The signed byte ranges do not match the file.");
    }
    let tail_is_blank = |from: usize| raw[from..].iter().all(|x| x.is_ascii_whitespace());
    info.changed_after = !tail_is_blank(c + e);
    info.covers_whole_file = !info.changed_after;
    let signed: [&[u8]; 2] = [&raw[a..a + b], &raw[c..c + e]];

    // The CMS blob: hex in the file, zero-padded; DER only (BER is reported as unreadable).
    let hex: Vec<u8> = raw[b + 1..c - 1].iter().copied().filter(|x| !x.is_ascii_whitespace()).collect();
    let bytes: Option<Vec<u8>> =
        hex.chunks(2).map(|p| std::str::from_utf8(p).ok().and_then(|s| u8::from_str_radix(s, 16).ok())).collect();
    let Some(bytes) = bytes else {
        return outcome(SignatureStatus::Unreadable, "The signature data is not valid hexadecimal.");
    };
    let Some(len) = der_len(&bytes).filter(|&l| l <= bytes.len() && bytes[0] == 0x30) else {
        return outcome(SignatureStatus::Unreadable, "The signature data could not be read (not DER-encoded CMS).");
    };
    let Ok(ci) = ContentInfo::from_der(&bytes[..len]) else {
        return outcome(SignatureStatus::Unreadable, "The signature data could not be read (not DER-encoded CMS).");
    };
    if ci.content_type.to_string() != "1.2.840.113549.1.7.2" {
        return outcome(SignatureStatus::Unreadable, "The signature data is not a CMS signed-data structure.");
    }
    let Ok(sd) = ci.content.decode_as::<SignedData>() else {
        return outcome(SignatureStatus::Unreadable, "The signature data could not be read.");
    };
    let Some(si) = sd.signer_infos.0.iter().next() else {
        return outcome(SignatureStatus::Unreadable, "The signature names no signer.");
    };
    info.has_timestamp =
        si.unsigned_attrs.as_ref().is_some_and(|u| u.iter().any(|a| a.oid.to_string() == "1.2.840.113549.1.9.16.2.14"));

    let cert = signer_certificate(&sd, si);
    if let Some(cert) = cert {
        let tbs = &cert.tbs_certificate;
        info.signer = common_name(&tbs.subject).or_else(|| Some(tbs.subject.to_string()));
        info.signer_subject = Some(tbs.subject.to_string());
        info.issuer = common_name(&tbs.issuer).or_else(|| Some(tbs.issuer.to_string()));
        info.self_signed = tbs.subject == tbs.issuer;
        info.certificate_valid_from = Some(tbs.validity.not_before.to_date_time().to_string());
        info.certificate_valid_to = Some(tbs.validity.not_after.to_date_time().to_string());
    }
    if let Some(t) = attribute(si, "1.2.840.113549.1.9.5")
        .and_then(|v| v.to_der().ok())
        .and_then(|d| x509_cert::time::Time::from_der(&d).ok())
    {
        info.signing_time = Some(t.to_date_time().to_string());
    }

    let Some(hash) = Hash::from_oid(&si.digest_alg.oid) else {
        return outcome(
            SignatureStatus::Unsupported,
            format!("The digest algorithm {} cannot be checked yet.", si.digest_alg.oid),
        );
    };
    info.digest_algorithm = Some(hash.name().into());

    // What the signer's attributes vouch for: the byte ranges (detached), or for adbe.pkcs7.sha1
    // the encapsulated SHA-1 of the byte ranges.
    let content_digest = match (&sd.encap_content_info.econtent, sub.as_str()) {
        (None, "adbe.pkcs7.detached" | "ETSI.CAdES.detached") => hash.digest(&signed),
        (Some(any), "adbe.pkcs7.sha1") => {
            let Ok(inner) = any.decode_as::<OctetString>() else {
                return outcome(SignatureStatus::Unreadable, "The signed content could not be read.");
            };
            if inner.as_bytes() != Hash::Sha1.digest(&signed).as_slice() {
                return outcome(
                    SignatureStatus::Invalid,
                    "The document was changed after it was signed: the signed content no longer matches.",
                );
            }
            hash.digest(&[inner.as_bytes()])
        }
        _ => return outcome(SignatureStatus::Unsupported, "This signature layout cannot be checked yet."),
    };

    // With signed attributes, their messageDigest must match and the signature covers them (DER SET).
    let to_verify = match &si.signed_attrs {
        Some(attrs) => {
            let Some(md) = attribute(si, "1.2.840.113549.1.9.4").and_then(|v| v.decode_as::<OctetString>().ok()) else {
                return outcome(SignatureStatus::Unreadable, "The signature's message digest is missing.");
            };
            if md.as_bytes() != content_digest.as_slice() {
                return outcome(
                    SignatureStatus::Invalid,
                    "The document was changed after it was signed: the signed content no longer matches.",
                );
            }
            let Ok(der) = attrs.to_der() else {
                return outcome(SignatureStatus::Unreadable, "The signed attributes could not be read.");
            };
            hash.digest(&[&der])
        }
        None => content_digest,
    };

    let Some(cert) = cert else {
        return outcome(SignatureStatus::Unreadable, "The signer's certificate is not included in the signature.");
    };
    let spki = &cert.tbs_certificate.subject_public_key_info;
    let key = spki.subject_public_key.raw_bytes();
    let sig_alg = si.signature_algorithm.oid.to_string();
    let key_alg = spki.algorithm.oid.to_string();
    let sig = si.signature.as_bytes();
    let verified = match (key_alg.as_str(), sig_alg.as_str()) {
        (
            "1.2.840.113549.1.1.1",
            "1.2.840.113549.1.1.1"
            | "1.2.840.113549.1.1.5"
            | "1.2.840.113549.1.1.11"
            | "1.2.840.113549.1.1.12"
            | "1.2.840.113549.1.1.13",
        ) => {
            info.signature_algorithm = Some(format!("RSA PKCS#1 v1.5 with {}", hash.name()));
            rsa_pkcs1_verify(key, hash, &to_verify, sig)
        }
        (
            "1.2.840.10045.2.1",
            "1.2.840.10045.2.1"
            | "1.2.840.10045.4.1"
            | "1.2.840.10045.4.3.2"
            | "1.2.840.10045.4.3.3"
            | "1.2.840.10045.4.3.4",
        ) => {
            let curve = spki
                .algorithm
                .parameters
                .as_ref()
                .and_then(|p| p.decode_as::<Oid>().ok())
                .map(|o| o.to_string())
                .unwrap_or_default();
            let name = match curve.as_str() {
                "1.2.840.10045.3.1.7" => "P-256",
                "1.3.132.0.34" => "P-384",
                _ => {
                    return outcome(
                        SignatureStatus::Unsupported,
                        "Signatures on this elliptic curve cannot be checked yet.",
                    )
                }
            };
            info.signature_algorithm = Some(format!("ECDSA {name} with {}", hash.name()));
            ecdsa_verify(&curve, key, &to_verify, sig)
        }
        ("1.2.840.113549.1.1.1", "1.2.840.113549.1.1.10") | (_, "1.2.840.113549.1.1.10") => {
            return outcome(SignatureStatus::Unsupported, "RSA-PSS signatures cannot be checked yet.");
        }
        _ => {
            return outcome(
                SignatureStatus::Unsupported,
                format!("The signature algorithm {sig_alg} cannot be checked yet."),
            )
        }
    };
    if verified {
        let pool: Vec<Certificate> = sd
            .certificates
            .as_ref()
            .map(|set| {
                set.0
                    .iter()
                    .filter_map(|c| match c {
                        cms::cert::CertificateChoices::Certificate(c) => Some(c.clone()),
                        _ => None,
                    })
                    .collect()
            })
            .unwrap_or_default();
        let now = time::OffsetDateTime::now_utc().unix_timestamp();
        let (t, detail, by, chain) = evaluate_trust(cert, &pool, anchors, now);
        info.trust = t;
        info.trust_detail = detail;
        info.trusted_by = by;
        info.chain = chain;
        outcome(
            SignatureStatus::Valid,
            "The signed content is unchanged and the signature matches the signer's certificate.",
        )
    } else {
        outcome(SignatureStatus::Invalid, "The signature does not match the signer's certificate.")
    }
}

/// One certificate of a signature's chain, for choosing what to trust.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ChainCertificate {
    pub name: String,
    pub subject: String,
    pub fingerprint: String,
    pub valid_to: String,
    pub is_ca: bool,
    pub self_signed: bool,
    #[serde(skip)]
    pub der: Vec<u8>,
}

/// The certificates of signature `index` (in report order): the signer first, then each issuer
/// found among the certificates the signature carries (verified link by link).
pub fn signature_chain(input: &Path, index: usize) -> PwResult<Vec<ChainCertificate>> {
    let raw = std::fs::read(input).map_err(|e| PwError::from_io(&e, "file"))?;
    let doc = Document::load_mem(&raw).map_err(|_| {
        PwError::new(
            ErrorCategory::UnsupportedOperation,
            "signatures.unreadable",
            "The file's structure could not be read.",
        )
    })?;
    let missing = || {
        PwError::new(
            ErrorCategory::InvalidInput,
            "trust.no_certificate",
            "That signature carries no readable certificate.",
        )
    };
    let (id, _, _) = signature_dicts(&doc).into_iter().nth(index).ok_or_else(missing)?;
    let d = doc.get_dictionary(id).map_err(|_| missing())?;
    let contents = match d.get(b"Contents") {
        Ok(Object::String(b, _)) => b.clone(),
        _ => return Err(missing()),
    };
    let len = der_len(&contents).filter(|&l| l <= contents.len()).ok_or_else(missing)?;
    let ci = ContentInfo::from_der(&contents[..len]).map_err(|_| missing())?;
    let sd = ci.content.decode_as::<SignedData>().map_err(|_| missing())?;
    let si = sd.signer_infos.0.iter().next().ok_or_else(missing)?;
    let signer = signer_certificate(&sd, si).ok_or_else(missing)?.clone();
    let pool: Vec<Certificate> = sd
        .certificates
        .as_ref()
        .map(|set| {
            set.0
                .iter()
                .filter_map(|c| match c {
                    cms::cert::CertificateChoices::Certificate(c) => Some(c.clone()),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default();
    let mut chain = vec![signer];
    while chain.len() < MAX_CHAIN {
        let last = chain.last().cloned().ok_or_else(missing)?;
        match pool.iter().find(|c| !same(c, &last) && issued_by(&last, c)) {
            Some(c) if !chain.iter().any(|x| same(x, c)) => chain.push(c.clone()),
            _ => break,
        }
    }
    Ok(chain
        .into_iter()
        .map(|c| ChainCertificate {
            name: label(&c),
            subject: c.tbs_certificate.subject.to_string(),
            fingerprint: fingerprint(&c),
            valid_to: c.tbs_certificate.validity.not_after.to_date_time().to_string(),
            is_ca: is_ca(&c),
            self_signed: issued_by(&c, &c),
            der: c.to_der().unwrap_or_default(),
        })
        .collect())
}

/// Every signature in the file, checked (read-only). Protected files are read with `password`.
pub fn verify_signatures(
    input: &Path,
    opts: &crate::OpenOptions,
    anchors: &[Certificate],
) -> PwResult<SignatureReport> {
    let raw = std::fs::read(input).map_err(|e| PwError::from_io(&e, "file"))?;
    // Structure through lopdf (PDFium normalises files lopdf cannot parse, but a normalised copy has
    // different bytes, so signatures in such files are reported as unreadable instead).
    let doc = match Document::load_mem(&raw) {
        Ok(d) => d,
        Err(_) => {
            // Opening through the engine distinguishes "not a PDF / damaged" from "unusual structure".
            crate::open_document(input, opts)?;
            return Ok(SignatureReport { signatures: Vec::new(), trust_note: TRUST_NOTE });
        }
    };
    verify_loaded(&raw, &doc, anchors)
}

/// Check the signatures of a file held in memory (used to verify a newly signed file before it is
/// written).
pub(crate) fn verify_bytes(raw: &[u8]) -> PwResult<SignatureReport> {
    let doc = Document::load_mem(raw).map_err(|_| {
        PwError::new(
            ErrorCategory::ValidationFailed,
            "signatures.unreadable",
            "The file's structure could not be read.",
        )
    })?;
    verify_loaded(raw, &doc, &[])
}

fn verify_loaded(raw: &[u8], doc: &Document, anchors: &[Certificate]) -> PwResult<SignatureReport> {
    if doc.is_encrypted() {
        return Err(PwError::new(
            ErrorCategory::UnsupportedOperation,
            "signatures.protected",
            "Signatures in password-protected files cannot be checked yet.",
        ));
    }
    let mut out = Vec::new();
    for (id, field, page) in signature_dicts(doc) {
        let Ok(d) = doc.get_dictionary(id) else { continue };
        let mut info = SignatureInfo {
            field,
            page,
            status: SignatureStatus::Unreadable,
            detail: String::new(),
            signer: text(d.get(b"Name").ok()),
            signer_subject: None,
            issuer: None,
            certificate_valid_from: None,
            certificate_valid_to: None,
            self_signed: false,
            signing_time: text(d.get(b"M").ok()),
            reason: text(d.get(b"Reason").ok()),
            location: text(d.get(b"Location").ok()),
            sub_filter: d
                .get(b"SubFilter")
                .ok()
                .and_then(|o| o.as_name().ok())
                .map(|n| String::from_utf8_lossy(n).into_owned()),
            digest_algorithm: None,
            signature_algorithm: None,
            covers_whole_file: false,
            changed_after: false,
            has_timestamp: false,
            trust: TrustStatus::NotChecked,
            trust_detail: "Trust is checked only for valid signatures.".into(),
            trusted_by: None,
            chain: Vec::new(),
            revocation: "not_checked",
        };
        let o = check(raw, d, &mut info, anchors);
        info.status = o.status;
        info.detail = o.detail;
        out.push(info);
    }
    Ok(SignatureReport { signatures: out, trust_note: TRUST_NOTE })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn der_lengths() {
        assert_eq!(der_len(&[0x30, 0x03, 1, 2, 3]), Some(5));
        assert_eq!(der_len(&[0x30, 0x81, 0x80]), Some(3 + 128));
        assert_eq!(der_len(&[0x30, 0x82, 0x01, 0x00]), Some(4 + 256));
        assert_eq!(der_len(&[0x30, 0x80]), None, "indefinite length (BER) is refused");
        let (tag, body, rest) = tlv(&[0x02, 0x01, 0x05, 0xff]).expect("tlv");
        assert_eq!((tag, body, rest), (0x02, &[0x05][..], &[0xff][..]));
    }

    #[test]
    fn rsa_verification_follows_pkcs1_v15() {
        // The padding and DigestInfo checks with e = 1 (so m = s): EM must be 00 01 FF.. 00 DigestInfo H.
        // Real keys are covered by the signed fixtures in tests/signatures.rs.
        let digest = Hash::Sha256.digest(&[b"hello"]);
        let k = 128;
        let t = [Hash::Sha256.digest_info_prefix(), &digest[..]].concat();
        let mut em = vec![0x00, 0x01];
        em.extend(std::iter::repeat_n(0xff, k - t.len() - 3));
        em.push(0x00);
        em.extend_from_slice(&t);
        // n slightly above EM so s = EM < n; e = 1.
        let mut n = em.clone();
        n[0] = 0x7f;
        let key = {
            let int = |v: &[u8]| {
                let mut b = vec![0x02];
                let mut v = v.to_vec();
                if v[0] & 0x80 != 0 {
                    v.insert(0, 0);
                }
                b.push(0x81);
                b.push(v.len() as u8);
                b.extend(v);
                b
            };
            let mut body = int(&n);
            body.extend([0x02, 0x01, 0x01]);
            let mut s = vec![0x30, 0x81, body.len() as u8];
            s.extend(body);
            s
        };
        assert!(rsa_pkcs1_verify(&key, Hash::Sha256, &digest, &em));
        let other = Hash::Sha256.digest(&[b"hellO"]);
        assert!(!rsa_pkcs1_verify(&key, Hash::Sha256, &other, &em), "different digest");
        let mut short_padding = em.clone();
        short_padding[3] = 0x00;
        assert!(!rsa_pkcs1_verify(&key, Hash::Sha256, &digest, &short_padding), "bad padding");
        assert!(!rsa_pkcs1_verify(&key, Hash::Sha256, &digest, &em[1..]), "wrong length");
    }
}
