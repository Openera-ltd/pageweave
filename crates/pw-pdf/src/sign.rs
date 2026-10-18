//! Sign a document with a certificate (PRD 05 "certificate workflows"): a PKCS#12 identity
//! (.pfx/.p12) is opened with its password, and a detached CMS signature (adbe.pkcs7.detached,
//! SHA-256/384) is added in an incremental update, so earlier revisions — and earlier signatures —
//! stay byte for byte as they were. The signature is invisible or shown in a box on a page.
//!
//! Private keys never leave memory and are dropped when signing ends; RSA and ECDSA signing use
//! `ring` (constant-time). The new file is verified with PageWeave's own signature checker before
//! it is written: the new signature must be valid and every signature that was valid before must
//! still be valid. Like verification, signing says nothing about trust.

use crate::signatures::{verify_bytes, SignatureStatus};
use cms::cert::{CertificateChoices, IssuerAndSerialNumber};
use cms::content_info::{CmsVersion, ContentInfo};
use cms::signed_data::{
    CertificateSet, EncapsulatedContentInfo, SignedData, SignerIdentifier, SignerInfo, SignerInfos,
};
use der::asn1::{ObjectIdentifier as Oid, OctetString, SetOfVec, UtcTime};
use der::{Any, Decode, Encode};
use lopdf::{dictionary, Dictionary, Document, IncrementalDocument, Object, ObjectId, StringFormat};
use pw_core::output::{write_atomic_bytes, CollisionPolicy, CommittedOutput};
use pw_core::{ErrorCategory, PwError, PwResult};
use ring::rand::SystemRandom;
use ring::signature::{self as rsig, KeyPair};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256, Sha384};
use std::path::Path;
use x509_cert::attr::Attribute;
use x509_cert::spki::AlgorithmIdentifierOwned;
use x509_cert::Certificate;

const OID_DATA: &str = "1.2.840.113549.1.7.1";
const OID_SIGNED_DATA: &str = "1.2.840.113549.1.7.2";
const OID_CONTENT_TYPE: &str = "1.2.840.113549.1.9.3";
const OID_MESSAGE_DIGEST: &str = "1.2.840.113549.1.9.4";
const OID_SIGNING_TIME: &str = "1.2.840.113549.1.9.5";
const OID_SHA256: &str = "2.16.840.1.101.3.4.2.1";
const OID_SHA384: &str = "2.16.840.1.101.3.4.2.2";
const OID_RSA: &str = "1.2.840.113549.1.1.1";
const OID_SHA256_RSA: &str = "1.2.840.113549.1.1.11";
const OID_EC: &str = "1.2.840.10045.2.1";
const OID_P256: &str = "1.2.840.10045.3.1.7";
const OID_P384: &str = "1.3.132.0.34";
const OID_ECDSA_SHA256: &str = "1.2.840.10045.4.3.2";
const OID_ECDSA_SHA384: &str = "1.2.840.10045.4.3.3";

/// Largest identity file read (a PKCS#12 with a long chain is a few tens of KB).
const MAX_IDENTITY_BYTES: u64 = 1024 * 1024;

/// Where a visible signature is drawn: page (0-based) and a box in that page's unrotated PDF space.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SignaturePlacement {
    pub page: u32,
    /// [x1, y1, x2, y2] in points.
    pub rect: [f32; 4],
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SignResult {
    pub field: String,
    pub signer: String,
    pub issuer: String,
    pub certificate_valid_to: String,
    pub algorithm: &'static str,
    pub visible: bool,
}

fn err(code: &'static str, msg: impl Into<String>) -> PwError {
    PwError::new(ErrorCategory::InvalidInput, code, msg.into())
}

fn oid(s: &str) -> Oid {
    Oid::new_unwrap(s)
}

fn alg(o: &str, null_params: bool) -> AlgorithmIdentifierOwned {
    AlgorithmIdentifierOwned { oid: oid(o), parameters: null_params.then(|| Any::from(der::asn1::Null)) }
}

/// Common name of a certificate subject or issuer, or the whole name.
fn common_name(name: &x509_cert::name::Name) -> String {
    for rdn in name.0.iter() {
        for atv in rdn.0.iter() {
            if atv.oid.to_string() == "2.5.4.3" {
                if let Ok(s) = atv.value.decode_as::<String>() {
                    return s;
                }
                if let Ok(s) = atv.value.decode_as::<der::asn1::PrintableStringRef>() {
                    return s.to_string();
                }
            }
        }
    }
    name.to_string()
}

enum Key {
    Rsa(rsig::RsaKeyPair),
    Ecdsa(rsig::EcdsaKeyPair, bool), // true = P-384
}

impl Key {
    fn public_key(&self) -> &[u8] {
        match self {
            Key::Rsa(k) => k.public_key().as_ref(),
            Key::Ecdsa(k, _) => k.public_key().as_ref(),
        }
    }
    fn p384(&self) -> bool {
        matches!(self, Key::Ecdsa(_, true))
    }
    fn sign(&self, msg: &[u8]) -> PwResult<Vec<u8>> {
        let rng = SystemRandom::new();
        let failed = || PwError::internal("sign.failed", "The signature could not be made with this key.");
        match self {
            Key::Rsa(k) => {
                let mut sig = vec![0u8; k.public().modulus_len()];
                k.sign(&rsig::RSA_PKCS1_SHA256, &rng, msg, &mut sig).map_err(|_| failed())?;
                Ok(sig)
            }
            Key::Ecdsa(k, _) => Ok(k.sign(&rng, msg).map_err(|_| failed())?.as_ref().to_vec()),
        }
    }
}

struct Identity {
    key: Key,
    cert: Certificate,
    chain: Vec<Certificate>,
}

/// Open a PKCS#12 identity and pick the certificate that belongs to its private key.
fn load_identity(path: &Path, password: &str, now: time::OffsetDateTime) -> PwResult<Identity> {
    let meta = std::fs::metadata(path).map_err(|e| PwError::from_io(&e, "certificate file"))?;
    if meta.len() > MAX_IDENTITY_BYTES {
        return Err(err("sign.identity_unreadable", "This file is too large to be a certificate file."));
    }
    let data = std::fs::read(path).map_err(|e| PwError::from_io(&e, "certificate file"))?;
    let store = p12_keystore::KeyStore::from_pkcs12(&data, password, p12_keystore::Pkcs12ImportPolicy::Relaxed)
        .map_err(|e| {
            let text = e.to_string().to_lowercase();
            if text.contains("mac") || text.contains("password") || text.contains("decrypt") {
                err("sign.wrong_password", "The certificate password is not correct.")
            } else {
                err(
                    "sign.identity_unreadable",
                    "This certificate file could not be read. Use a .pfx or .p12 file that contains a private key.",
                )
            }
        })?;
    let (_, chain) = store
        .private_key_chain()
        .ok_or_else(|| err("sign.no_private_key", "This certificate file has no private key, so it cannot sign."))?;
    let key_der = chain.key().as_der();
    let certs: Vec<Certificate> = chain.certs().iter().filter_map(|c| Certificate::from_der(c.as_der()).ok()).collect();
    if certs.is_empty() {
        return Err(err("sign.no_certificate", "This certificate file contains a key but no certificate."));
    }
    // Key type from the certificates' public keys; the signer is the certificate matching the key.
    let rng = SystemRandom::new();
    let mut key = None;
    let mut signer = None;
    for (i, c) in certs.iter().enumerate() {
        let spki = &c.tbs_certificate.subject_public_key_info;
        let pk = spki.subject_public_key.raw_bytes();
        let candidate = match spki.algorithm.oid.to_string().as_str() {
            OID_RSA => rsig::RsaKeyPair::from_pkcs8(key_der).ok().map(Key::Rsa),
            OID_EC => {
                let curve =
                    spki.algorithm.parameters.as_ref().and_then(|p| p.decode_as::<Oid>().ok()).map(|o| o.to_string());
                let (a, p384) = match curve.as_deref() {
                    Some(OID_P256) => (&rsig::ECDSA_P256_SHA256_ASN1_SIGNING, false),
                    Some(OID_P384) => (&rsig::ECDSA_P384_SHA384_ASN1_SIGNING, true),
                    _ => continue,
                };
                rsig::EcdsaKeyPair::from_pkcs8(a, key_der, &rng).ok().map(|k| Key::Ecdsa(k, p384))
            }
            _ => None,
        };
        if let Some(k) = candidate {
            let matches = match &k {
                Key::Rsa(_) => {
                    // SPKI holds RSAPublicKey DER, which is what ring reports.
                    k.public_key() == pk
                }
                Key::Ecdsa(..) => k.public_key() == pk,
            };
            if matches {
                key = Some(k);
                signer = Some(i);
                break;
            }
        }
    }
    let (Some(key), Some(si)) = (key, signer) else {
        return Err(err(
            "sign.unsupported_key",
            "This certificate's key cannot be used: PageWeave signs with RSA (2048–8192 bits) and ECDSA P-256/P-384 keys.",
        ));
    };
    let cert = certs[si].clone();
    let v = &cert.tbs_certificate.validity;
    let (from, to) =
        (v.not_before.to_unix_duration().as_secs() as i64, v.not_after.to_unix_duration().as_secs() as i64);
    let t = now.unix_timestamp();
    let day = |s: i64| time::OffsetDateTime::from_unix_timestamp(s).map(|d| d.date().to_string()).unwrap_or_default();
    if t > to {
        return Err(err(
            "sign.cert_expired",
            format!("This certificate expired on {}. Use a current certificate.", day(to)),
        ));
    }
    if t < from {
        return Err(err("sign.cert_not_yet_valid", format!("This certificate is only valid from {}.", day(from))));
    }
    // Key usage, when stated, must allow signing.
    if let Some(exts) = &cert.tbs_certificate.extensions {
        for e in exts.iter().filter(|e| e.extn_id.to_string() == "2.5.29.15") {
            if let Ok(bits) = der::asn1::BitString::from_der(e.extn_value.as_bytes()) {
                let b = bits.raw_bytes().first().copied().unwrap_or(0);
                // digitalSignature (bit 0) or nonRepudiation (bit 1).
                if b & 0xC0 == 0 {
                    return Err(err("sign.key_usage", "This certificate is not meant for signing documents."));
                }
            }
        }
    }
    let chain = certs.into_iter().enumerate().filter(|(i, _)| *i != si).map(|(_, c)| c).collect();
    Ok(Identity { key, cert, chain })
}

/// Detached CMS SignedData over `digest`.
fn build_cms(id: &Identity, digest: &[u8], when: time::OffsetDateTime) -> PwResult<Vec<u8>> {
    let enc = |e: der::Error| PwError::internal("sign.encode", format!("The signature could not be encoded ({e})."));
    let p384 = id.key.p384();
    let digest_alg = alg(if p384 { OID_SHA384 } else { OID_SHA256 }, false);
    let sig_alg = match &id.key {
        Key::Rsa(_) => alg(OID_SHA256_RSA, true),
        Key::Ecdsa(_, false) => alg(OID_ECDSA_SHA256, false),
        Key::Ecdsa(_, true) => alg(OID_ECDSA_SHA384, false),
    };
    let attr = |o: &str, v: Any| -> PwResult<Attribute> {
        Ok(Attribute { oid: oid(o), values: SetOfVec::try_from(vec![v]).map_err(enc)? })
    };
    let secs = when.unix_timestamp().max(0) as u64;
    let utc = UtcTime::from_unix_duration(std::time::Duration::from_secs(secs)).map_err(enc)?;
    let attrs = vec![
        attr(OID_CONTENT_TYPE, Any::encode_from(&oid(OID_DATA)).map_err(enc)?)?,
        attr(OID_SIGNING_TIME, Any::encode_from(&utc).map_err(enc)?)?,
        attr(OID_MESSAGE_DIGEST, Any::encode_from(&OctetString::new(digest.to_vec()).map_err(enc)?).map_err(enc)?)?,
    ];
    let signed_attrs: SetOfVec<Attribute> = SetOfVec::try_from(attrs).map_err(enc)?;
    // The signature covers the DER encoding of the attributes as a SET (RFC 5652 5.4).
    let to_sign = signed_attrs.to_der().map_err(enc)?;
    let signature = id.key.sign(&to_sign)?;
    let si = SignerInfo {
        version: CmsVersion::V1,
        sid: SignerIdentifier::IssuerAndSerialNumber(IssuerAndSerialNumber {
            issuer: id.cert.tbs_certificate.issuer.clone(),
            serial_number: id.cert.tbs_certificate.serial_number.clone(),
        }),
        digest_alg: digest_alg.clone(),
        signed_attrs: Some(signed_attrs),
        signature_algorithm: sig_alg,
        signature: OctetString::new(signature).map_err(enc)?,
        unsigned_attrs: None,
    };
    let mut certs = vec![CertificateChoices::Certificate(id.cert.clone())];
    certs.extend(id.chain.iter().cloned().map(CertificateChoices::Certificate));
    let sd = SignedData {
        version: CmsVersion::V1,
        digest_algorithms: SetOfVec::try_from(vec![digest_alg]).map_err(enc)?,
        encap_content_info: EncapsulatedContentInfo { econtent_type: oid(OID_DATA), econtent: None },
        certificates: Some(CertificateSet(SetOfVec::try_from(certs).map_err(enc)?)),
        crls: None,
        signer_infos: SignerInfos(SetOfVec::try_from(vec![si]).map_err(enc)?),
    };
    let ci = ContentInfo { content_type: oid(OID_SIGNED_DATA), content: Any::encode_from(&sd).map_err(enc)? };
    ci.to_der().map_err(enc)
}

/// PDF text string: Latin-1 when possible, otherwise UTF-16BE with a BOM.
fn pdf_text(s: &str) -> Object {
    if s.chars().all(|c| (c as u32) < 256) {
        Object::String(s.chars().map(|c| c as u8).collect(), StringFormat::Literal)
    } else {
        let mut b = vec![0xFE, 0xFF];
        for u in s.encode_utf16() {
            b.extend(u.to_be_bytes());
        }
        Object::String(b, StringFormat::Literal)
    }
}

fn pdf_date(t: time::OffsetDateTime) -> String {
    format!(
        "D:{:04}{:02}{:02}{:02}{:02}{:02}Z",
        t.year(),
        u8::from(t.month()),
        t.day(),
        t.hour(),
        t.minute(),
        t.second()
    )
}

/// Content of a visible signature box: three lines of Helvetica fitted to the box.
fn appearance(w: f32, h: f32, lines: &[String]) -> Vec<u8> {
    let esc = |s: &str| -> String {
        s.chars()
            .map(|c| if (c as u32) < 256 { c } else { '?' })
            .flat_map(|c| match c {
                '(' | ')' | '\\' => vec!['\\', c],
                _ => vec![c],
            })
            .collect()
    };
    let n = lines.len().max(1) as f32;
    let longest = lines.iter().map(|l| l.chars().count()).max().unwrap_or(1).max(1) as f32;
    // Helvetica averages ~0.5 em per character.
    let size = (h / (n * 1.25)).min(w / (longest * 0.52)).clamp(3.0, 14.0);
    let mut out = format!(
        "q 0.35 0.45 0.75 RG 0.75 w 0.5 0.5 {:.2} {:.2} re S Q\nBT /Helv {size:.2} Tf 0.1 0.15 0.35 rg\n",
        w - 1.0,
        h - 1.0
    );
    let top = h - (h - n * size * 1.25) / 2.0 - size;
    for (i, l) in lines.iter().enumerate() {
        let y = top - i as f32 * size * 1.25;
        let line = esc(l);
        let x = 4.0f32.min(w * 0.05);
        out.push_str(&format!("1 0 0 1 {x:.2} {y:.2} Tm ({line}) Tj\n"));
    }
    out.push_str("ET\n");
    // Latin-1 bytes for the content stream.
    out.chars().map(|c| c as u32 as u8).collect()
}

fn reference_or_inline<'a>(doc: &'a Document, o: &'a Object) -> Option<(Option<ObjectId>, &'a Object)> {
    match o {
        Object::Reference(id) => doc.get_object(*id).ok().map(|x| (Some(*id), x)),
        other => Some((None, other)),
    }
}

/// Sign `input` into `output`.
#[allow(clippy::too_many_arguments)]
pub fn sign_document(
    input: &Path,
    certificate: &Path,
    password: &str,
    reason: Option<&str>,
    location: Option<&str>,
    placement: Option<SignaturePlacement>,
    output: &Path,
    policy: CollisionPolicy,
) -> PwResult<(CommittedOutput, SignResult, Vec<String>)> {
    let now = time::OffsetDateTime::now_utc();
    let id = load_identity(certificate, password, now)?;
    let raw = std::fs::read(input).map_err(|e| PwError::from_io(&e, "file"))?;
    let prev = Document::load_mem(&raw).map_err(|_| {
        PwError::new(
            ErrorCategory::UnsupportedOperation,
            "sign.structure",
            "This file's structure cannot be updated in place, so it cannot be signed.",
        )
        .with_action("Save a copy with Reduce file size (or Organize pages) and sign that copy.")
    })?;
    if prev.is_encrypted() {
        return Err(PwError::new(
            ErrorCategory::UnsupportedOperation,
            "sign.protected",
            "This file is password protected.",
        )
        .with_action("Remove the password first (Remove password…), then sign that copy."));
    }
    // Signatures that are valid now must stay valid.
    let before = verify_bytes(&raw)?;
    let valid_before = before.signatures.iter().filter(|s| s.status == SignatureStatus::Valid).count();

    let pages = prev.get_pages();
    let page_no = placement.map(|p| p.page).unwrap_or(0);
    let &page_id = pages.get(&(page_no + 1)).ok_or_else(|| err("sign.page", "That page does not exist."))?;
    let catalog_id = prev
        .trailer
        .get(b"Root")
        .and_then(Object::as_reference)
        .map_err(|_| PwError::internal("sign.structure", "The document catalog is missing."))?;

    // A field name not used yet.
    let existing: Vec<String> = before.signatures.iter().filter_map(|s| s.field.clone()).collect();
    let field = (1..).map(|n| format!("Signature{n}")).find(|n| !existing.contains(n)).unwrap_or_default();

    let signer = common_name(&id.cert.tbs_certificate.subject);
    let issuer = common_name(&id.cert.tbs_certificate.issuer);
    let mut inc = IncrementalDocument::create_from(raw.clone(), prev);

    // Placeholders sized for the CMS: certificates + signature + attributes, with room to spare.
    let cert_bytes: usize =
        std::iter::once(&id.cert).chain(id.chain.iter()).filter_map(|c| c.to_der().ok()).map(|d| d.len()).sum();
    let contents_len = (cert_bytes + 4096).next_power_of_two().max(8192);
    const BR: i64 = 9_999_999_999;
    let mut sig = Dictionary::new();
    sig.set("Type", Object::Name(b"Sig".to_vec()));
    sig.set("Filter", Object::Name(b"Adobe.PPKLite".to_vec()));
    sig.set("SubFilter", Object::Name(b"adbe.pkcs7.detached".to_vec()));
    sig.set("ByteRange", Object::Array(vec![0.into(), BR.into(), BR.into(), BR.into()]));
    sig.set("Contents", Object::String(vec![0u8; contents_len], StringFormat::Hexadecimal));
    sig.set("M", Object::string_literal(pdf_date(now)));
    sig.set("Name", pdf_text(&signer));
    if let Some(r) = reason.filter(|r| !r.trim().is_empty()) {
        sig.set("Reason", pdf_text(r.trim()));
    }
    if let Some(l) = location.filter(|l| !l.trim().is_empty()) {
        sig.set("Location", pdf_text(l.trim()));
    }
    let sig_id = inc.new_document.add_object(Object::Dictionary(sig));

    let mut widget = Dictionary::new();
    widget.set("Type", Object::Name(b"Annot".to_vec()));
    widget.set("Subtype", Object::Name(b"Widget".to_vec()));
    widget.set("FT", Object::Name(b"Sig".to_vec()));
    widget.set("T", pdf_text(&field));
    widget.set("V", Object::Reference(sig_id));
    widget.set("F", 132); // print + locked
    widget.set("P", Object::Reference(page_id));
    let mut warnings = Vec::new();
    match placement {
        Some(p) => {
            let [x1, y1, x2, y2] = p.rect;
            let (l, b, r, t) = (x1.min(x2), y1.min(y2), x1.max(x2), y1.max(y2));
            if !(r - l >= 20.0 && t - b >= 10.0) || ![l, b, r, t].iter().all(|v| v.is_finite()) {
                return Err(err("sign.box", "Make the signature box at least 20 × 10 points."));
            }
            let mut lines = vec![format!("Digitally signed by {signer}"), format!("Date: {}", now.date())];
            if let Some(r) = reason.filter(|r| !r.trim().is_empty()) {
                lines.push(format!("Reason: {}", r.trim()));
            }
            if lines.iter().any(|l| l.chars().any(|c| (c as u32) >= 256)) {
                warnings.push("Characters outside Latin-1 are shown as \"?\" in the visible signature box (the signature itself keeps the full name).".into());
            }
            let (w, h) = (r - l, t - b);
            let font = inc.new_document.add_object(dictionary! {
                "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica", "Encoding" => "WinAnsiEncoding"
            });
            let ap = lopdf::Stream::new(
                dictionary! {
                    "Type" => "XObject", "Subtype" => "Form",
                    "BBox" => vec![0.into(), 0.into(), w.into(), h.into()],
                    "Resources" => dictionary! { "Font" => dictionary! { "Helv" => font } },
                },
                appearance(w, h, &lines),
            );
            let ap_id = inc.new_document.add_object(ap);
            widget.set("Rect", Object::Array(vec![l.into(), b.into(), r.into(), t.into()]));
            widget.set("AP", dictionary! { "N" => ap_id });
        }
        None => widget.set("Rect", Object::Array(vec![0.into(), 0.into(), 0.into(), 0.into()])),
    }
    let widget_id = inc.new_document.add_object(Object::Dictionary(widget));

    // The page lists the widget.
    let structure = || PwError::internal("sign.structure", "The page or form structure could not be updated.");
    let annots = inc.get_prev_documents().get_dictionary(page_id).ok().and_then(|d| d.get(b"Annots").ok()).cloned();
    match annots.as_ref().and_then(|a| reference_or_inline(inc.get_prev_documents(), a).map(|(i, o)| (i, o.clone()))) {
        Some((Some(arr_id), Object::Array(mut a))) => {
            a.push(Object::Reference(widget_id));
            inc.new_document.set_object(arr_id, Object::Array(a));
        }
        other => {
            let mut a = match other {
                Some((None, Object::Array(a))) => a,
                _ => Vec::new(),
            };
            a.push(Object::Reference(widget_id));
            inc.opt_clone_object_to_new_document(page_id).map_err(|_| structure())?;
            inc.new_document.get_dictionary_mut(page_id).map_err(|_| structure())?.set("Annots", Object::Array(a));
        }
    }
    // The form lists the field and says the file holds signatures (SigFlags 3: signatures exist,
    // append only).
    let acro = inc.get_prev_documents().get_dictionary(catalog_id).ok().and_then(|c| c.get(b"AcroForm").ok()).cloned();
    let update_form = |form: &mut Dictionary, inc: &mut IncrementalDocument| -> PwResult<()> {
        let fields = form.get(b"Fields").ok().cloned();
        match fields {
            Some(Object::Reference(fid)) => {
                let mut a =
                    inc.get_prev_documents().get_object(fid).and_then(Object::as_array).cloned().unwrap_or_default();
                a.push(Object::Reference(widget_id));
                inc.new_document.set_object(fid, Object::Array(a));
            }
            Some(Object::Array(mut a)) => {
                a.push(Object::Reference(widget_id));
                form.set("Fields", Object::Array(a));
            }
            _ => form.set("Fields", Object::Array(vec![Object::Reference(widget_id)])),
        }
        form.set("SigFlags", 3);
        Ok(())
    };
    match acro {
        Some(Object::Reference(form_id)) => {
            let mut form = inc.get_prev_documents().get_dictionary(form_id).map_err(|_| structure())?.clone();
            update_form(&mut form, &mut inc)?;
            inc.new_document.set_object(form_id, Object::Dictionary(form));
        }
        other => {
            let mut form = match other {
                Some(Object::Dictionary(d)) => d,
                _ => Dictionary::new(),
            };
            update_form(&mut form, &mut inc)?;
            inc.opt_clone_object_to_new_document(catalog_id).map_err(|_| structure())?;
            inc.new_document
                .get_dictionary_mut(catalog_id)
                .map_err(|_| structure())?
                .set("AcroForm", Object::Dictionary(form));
        }
    }

    let mut bytes = Vec::new();
    inc.save_to(&mut bytes).map_err(|_| PwError::internal("sign.save", "The signed file could not be written."))?;

    // Locate the placeholders in the appended part and fill in the byte range.
    let tail = raw.len();
    let find = |hay: &[u8], needle: &[u8], from: usize| {
        hay[from..].windows(needle.len()).position(|w| w == needle).map(|p| p + from)
    };
    let zeros = format!("<{}>", "0".repeat(contents_len * 2));
    let c_start = find(&bytes, zeros.as_bytes(), tail)
        .ok_or_else(|| PwError::internal("sign.placeholder", "The signature space was not found."))?;
    let c_end = c_start + zeros.len();
    let br_text = format!("{BR} {BR} {BR}");
    let br_at = find(&bytes, br_text.as_bytes(), tail)
        .ok_or_else(|| PwError::internal("sign.placeholder", "The byte range was not found."))?;
    let actual = format!("{} {} {}", c_start, c_end, bytes.len() - c_end);
    let padded = format!("{actual:<width$}", width = br_text.len());
    bytes[br_at..br_at + br_text.len()].copy_from_slice(padded.as_bytes());

    let digest: Vec<u8> = if id.key.p384() {
        let mut h = Sha384::new();
        h.update(&bytes[..c_start]);
        h.update(&bytes[c_end..]);
        h.finalize().to_vec()
    } else {
        let mut h = Sha256::new();
        h.update(&bytes[..c_start]);
        h.update(&bytes[c_end..]);
        h.finalize().to_vec()
    };
    let cms = build_cms(&id, &digest, now)?;
    if cms.len() > contents_len {
        return Err(PwError::internal("sign.too_large", "The signature did not fit the space reserved for it."));
    }
    let hex: String = cms.iter().map(|b| format!("{b:02X}")).collect();
    bytes[c_start + 1..c_start + 1 + hex.len()].copy_from_slice(hex.as_bytes());

    // Verify before writing: our signature valid and covering the whole file; earlier ones intact.
    let fail = |what: String| {
        PwError::new(
            ErrorCategory::ValidationFailed,
            "output.verify",
            format!("The signed file failed verification ({what}) and was not saved."),
        )
    };
    let after = verify_bytes(&bytes)?;
    let ours = after
        .signatures
        .iter()
        .find(|s| s.field.as_deref() == Some(field.as_str()))
        .ok_or_else(|| fail("the new signature was not found".into()))?;
    if ours.status != SignatureStatus::Valid || !ours.covers_whole_file {
        return Err(fail(format!(
            "the new signature is {}: {}",
            serde_json::to_string(&ours.status).unwrap_or_default(),
            ours.detail
        )));
    }
    let valid_after = after.signatures.iter().filter(|s| s.status == SignatureStatus::Valid).count();
    if valid_after != valid_before + 1 {
        return Err(fail("an earlier signature is no longer valid".into()));
    }
    if !bytes.starts_with(&raw) {
        return Err(fail("the earlier revision changed".into()));
    }
    let protected = [input.to_path_buf(), certificate.to_path_buf()];
    let c = write_atomic_bytes(output, policy, &protected, &bytes, |staged| {
        let back = std::fs::read(staged).map_err(|e| PwError::from_io(&e, "output file"))?;
        if back == bytes {
            Ok(())
        } else {
            Err(fail("the written file differs".into()))
        }
    })?;
    if id.cert.tbs_certificate.issuer == id.cert.tbs_certificate.subject {
        warnings.push(
            "This certificate is self-signed: readers will show the signer as unknown unless they choose to trust it."
                .into(),
        );
    }
    let algorithm = match &id.key {
        Key::Rsa(_) => "RSA with SHA-256",
        Key::Ecdsa(_, false) => "ECDSA P-256 with SHA-256",
        Key::Ecdsa(_, true) => "ECDSA P-384 with SHA-384",
    };
    let valid_to = time::OffsetDateTime::from_unix_timestamp(
        id.cert.tbs_certificate.validity.not_after.to_unix_duration().as_secs() as i64,
    )
    .map(|d| d.date().to_string())
    .unwrap_or_default();
    Ok((
        c,
        SignResult { field, signer, issuer, certificate_valid_to: valid_to, algorithm, visible: placement.is_some() },
        warnings,
    ))
}
