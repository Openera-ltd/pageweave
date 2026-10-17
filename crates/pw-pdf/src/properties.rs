//! Edit document properties (PRD 05 "metadata"): title, author, subject and keywords in a new file.
//! The document information dictionary is updated and, when the file carries XMP metadata, a fresh
//! XMP packet with the same values replaces it (keeping a PDF/A identification), so readers that
//! prefer XMP show the same thing. The copy is rewritten from scratch (no earlier versions keep the
//! old values) and verified before commit: PDFium reads back exactly the requested properties and
//! every page renders as before.

use crate::document::OpenOptions;
use crate::engine;
use crate::sanitize::{catalog, check_pages, dict_of, has_earlier_versions, load_plain, stream_data};
use lopdf::{Dictionary, Object, Stream};
use pdfium_render::prelude::*;
use pw_core::output::{write_atomic_bytes, CollisionPolicy, CommittedOutput};
use pw_core::{ErrorCategory, PwError, PwResult};
use std::path::Path;

/// Requested changes: `None` keeps a property, an empty string removes it.
#[derive(Debug, Clone, Default)]
pub struct PropertyChanges {
    pub title: Option<String>,
    pub author: Option<String>,
    pub subject: Option<String>,
    pub keywords: Option<String>,
}

fn fail(what: &str) -> PwError {
    PwError::new(
        ErrorCategory::ValidationFailed,
        "output.verify",
        format!("The copy with new properties failed verification ({what}) and was not saved."),
    )
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// `pdfaid:part` / `pdfaid:conformance` from an XMP packet (attribute or element form).
fn pdfa_id(xmp: &str) -> Option<(String, String)> {
    let find = |name: &str| -> Option<String> {
        let attr = format!("pdfaid:{name}=\"");
        if let Some(i) = xmp.find(&attr) {
            let rest = &xmp[i + attr.len()..];
            return rest.split('"').next().map(str::to_string);
        }
        let open = format!("<pdfaid:{name}>");
        let i = xmp.find(&open)?;
        let rest = &xmp[i + open.len()..];
        rest.split('<').next().map(|v| v.trim().to_string())
    };
    let part = find("part")?;
    part.chars().all(|c| c.is_ascii_digit()).then_some(())?;
    let conformance = find("conformance").unwrap_or_default();
    conformance.chars().all(|c| c.is_ascii_alphabetic()).then_some((part, conformance))
}

/// A minimal XMP packet carrying the document properties (and a PDF/A identification if given).
fn xmp_packet(
    title: &str,
    author: &str,
    subject: &str,
    keywords: &str,
    modified: &str,
    pdfa: Option<&(String, String)>,
) -> String {
    let mut d = String::new();
    let lang_alt = |tag: &str, v: &str| {
        format!("<{tag}><rdf:Alt><rdf:li xml:lang=\"x-default\">{}</rdf:li></rdf:Alt></{tag}>", xml_escape(v))
    };
    if !title.is_empty() {
        d += &lang_alt("dc:title", title);
    }
    if !author.is_empty() {
        d += &format!("<dc:creator><rdf:Seq><rdf:li>{}</rdf:li></rdf:Seq></dc:creator>", xml_escape(author));
    }
    if !subject.is_empty() {
        d += &lang_alt("dc:description", subject);
    }
    if !keywords.is_empty() {
        d += &format!("<pdf:Keywords>{}</pdf:Keywords>", xml_escape(keywords));
    }
    d += &format!("<xmp:ModifyDate>{modified}</xmp:ModifyDate><xmp:MetadataDate>{modified}</xmp:MetadataDate>");
    if let Some((part, conf)) = pdfa {
        d += &format!("<pdfaid:part>{part}</pdfaid:part>");
        if !conf.is_empty() {
            d += &format!("<pdfaid:conformance>{conf}</pdfaid:conformance>");
        }
    }
    format!(
        "<?xpacket begin=\"\u{feff}\" id=\"W5M0MpCehiHzreSzNTczkc9d\"?>\n<x:xmpmeta xmlns:x=\"adobe:ns:meta/\">\
<rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\"><rdf:Description rdf:about=\"\" \
xmlns:dc=\"http://purl.org/dc/elements/1.1/\" xmlns:pdf=\"http://ns.adobe.com/pdf/1.3/\" \
xmlns:xmp=\"http://ns.adobe.com/xap/1.0/\" xmlns:pdfaid=\"http://www.aiim.org/pdfa/ns/id/\">{d}</rdf:Description>\
</rdf:RDF></x:xmpmeta>\n<?xpacket end=\"w\"?>"
    )
}

/// Write a copy with the requested properties. Returns the file, its page count and warnings.
pub fn set_properties(
    input: &Path,
    opts: &OpenOptions,
    changes: &PropertyChanges,
    output: &Path,
    policy: CollisionPolicy,
    cancelled: &dyn Fn() -> bool,
    progress: &dyn Fn(u64, u64),
) -> PwResult<(CommittedOutput, u32, Vec<String>)> {
    let pdfium = engine()?;
    {
        let src = crate::document::load(&pdfium, input, opts)?;
        if crate::unprotect::is_protected(&src) {
            return Err(PwError::new(
                ErrorCategory::UnsupportedOperation,
                "properties.protected",
                "The properties of a password-protected file cannot be changed here.",
            )
            .with_action("Remove the password first (Remove password…), then change the properties of that copy."));
        }
    }
    let (source, mut doc, raw) = load_plain(&pdfium, input, opts).map_err(|e| {
        if e.code == "sanitize.protected" {
            PwError::new(
                ErrorCategory::UnsupportedOperation,
                "properties.protected",
                "This file is password protected.",
            )
        } else {
            e
        }
    })?;
    let signatures = source.signatures().len();
    let earlier = has_earlier_versions(&raw);

    // Current values (Info), then the requested changes.
    let mut info: Dictionary =
        doc.trailer.get(b"Info").ok().and_then(|o| dict_of(&doc, o)).cloned().unwrap_or_default();
    let current = |info: &Dictionary, k: &[u8]| {
        info.get(k).ok().map(|o| lopdf::decode_text_string(o).unwrap_or_default()).unwrap_or_default()
    };
    let mut want = Vec::new();
    for (key, change) in [
        (&b"Title"[..], &changes.title),
        (b"Author", &changes.author),
        (b"Subject", &changes.subject),
        (b"Keywords", &changes.keywords),
    ] {
        let value = match change {
            Some(v) => v.trim().to_string(),
            None => current(&info, key).trim().to_string(),
        };
        if value.is_empty() {
            info.remove(key);
        } else {
            info.set(key.to_vec(), lopdf::text_string(&value));
        }
        want.push(value);
    }
    let now = time::OffsetDateTime::now_utc();
    let pdf_date = format!(
        "D:{:04}{:02}{:02}{:02}{:02}{:02}Z",
        now.year(),
        u8::from(now.month()),
        now.day(),
        now.hour(),
        now.minute(),
        now.second()
    );
    info.set("ModDate", Object::string_literal(pdf_date));
    let info_id = doc.add_object(Object::Dictionary(info));
    doc.trailer.set("Info", Object::Reference(info_id));

    // XMP: replace with a packet carrying the same values.
    let mut xmp_replaced = false;
    let xmp_ref = catalog(&doc).and_then(|c| c.get(b"Metadata").ok()).and_then(|o| o.as_reference().ok());
    if let Some(id) = xmp_ref {
        let old = match doc.get_object(id) {
            Ok(Object::Stream(s)) => stream_data(s).map(|d| String::from_utf8_lossy(&d).into_owned()),
            _ => None,
        };
        if let Some(old) = old {
            let iso = format!(
                "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
                now.year(),
                u8::from(now.month()),
                now.day(),
                now.hour(),
                now.minute(),
                now.second()
            );
            let packet = xmp_packet(&want[0], &want[1], &want[2], &want[3], &iso, pdfa_id(&old).as_ref());
            let mut dict = Dictionary::new();
            dict.set("Type", Object::Name(b"Metadata".to_vec()));
            dict.set("Subtype", Object::Name(b"XML".to_vec()));
            doc.objects.insert(id, Object::Stream(Stream::new(dict, packet.into_bytes())));
            xmp_replaced = true;
        }
    }
    doc.trailer.remove(b"Prev");
    doc.trailer.remove(b"XRefStm");
    doc.prune_objects();
    let mut bytes = Vec::new();
    doc.save_to(&mut bytes).map_err(|_| PwError::internal("properties.save", "The copy could not be written."))?;

    let check = |bytes: &[u8]| -> PwResult<()> {
        let out = pdfium.load_pdf_from_byte_slice(bytes, None).map_err(|_| fail("it could not be reopened"))?;
        let meta = out.metadata();
        let got = |t: PdfDocumentMetadataTagType| meta.get(t).map(|v| v.value().trim().to_string()).unwrap_or_default();
        let read = [
            got(PdfDocumentMetadataTagType::Title),
            got(PdfDocumentMetadataTagType::Author),
            got(PdfDocumentMetadataTagType::Subject),
            got(PdfDocumentMetadataTagType::Keywords),
        ];
        if read.as_slice() != want.as_slice() {
            return Err(fail("the properties read back differ"));
        }
        if has_earlier_versions(bytes) {
            return Err(fail("an earlier version remains"));
        }
        Ok(())
    };
    check(&bytes)?;
    check_pages(&pdfium, &source, &bytes, cancelled, progress)?;
    let count = source.pages().len().max(0) as u32;
    let protected_sources = [input.to_path_buf()];
    let c = write_atomic_bytes(output, policy, &protected_sources, &bytes, |staged| {
        let back = std::fs::read(staged).map_err(|e| PwError::from_io(&e, "output file"))?;
        if back != bytes {
            return Err(fail("the written file differs"));
        }
        check(&back)
    })?;
    let mut warnings = Vec::new();
    if signatures > 0 {
        warnings.push(signature_warning(signatures));
    }
    if xmp_replaced {
        warnings.push("The XMP metadata was rewritten to match; other XMP entries were not kept.".into());
    }
    if earlier {
        warnings.push("Earlier saved versions of the file were not carried over, so the old values are gone.".into());
    }
    Ok((c, count, warnings))
}

/// Shown whenever a rewritten copy comes from a signed file.
pub(crate) fn signature_warning(n: u16) -> String {
    format!(
        "The original has {n} digital signature{}; {} not valid in this copy because its content was rewritten.",
        if n == 1 { "" } else { "s" },
        if n == 1 { "it is" } else { "they are" }
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pdfa_identification_is_found_in_both_forms() {
        assert_eq!(pdfa_id(r#"<x pdfaid:part="2" pdfaid:conformance="B"/>"#), Some(("2".into(), "B".into())));
        assert_eq!(
            pdfa_id("<pdfaid:part>3</pdfaid:part><pdfaid:conformance>U</pdfaid:conformance>"),
            Some(("3".into(), "U".into()))
        );
        assert_eq!(pdfa_id("<dc:title/>"), None);
        assert_eq!(pdfa_id(r#"pdfaid:part="2<script>""#), None, "only plain values are carried over");
    }

    #[test]
    fn xmp_values_are_escaped() {
        let x =
            xmp_packet("A & <B>", "Ann \"Q\"", "", "k1, k2", "2026-01-01T00:00:00Z", Some(&("2".into(), "B".into())));
        assert!(
            x.contains("A &amp; &lt;B&gt;")
                && x.contains("Ann &quot;Q&quot;")
                && x.contains("<pdfaid:part>2</pdfaid:part>")
        );
        assert!(!x.contains("dc:description"), "empty values are left out");
    }
}
