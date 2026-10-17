//! Reduce file size without changing the document (PRD 02 "compression"): streams stored without
//! compression or with weak Flate compression are recompressed, identical streams and fonts are
//! stored once, unused objects and earlier saved versions are dropped, and objects are packed into
//! compressed object streams. Nothing visible changes: images keep their exact data, and every
//! rewritten stream is decoded again and compared byte for byte before it is used.
//!
//! The copy is verified before commit: it reopens, every page renders the same with annotations and
//! form fields drawn, page text, annotation counts, bookmarks and attachments match, and it is
//! smaller than the original (otherwise nothing is saved and the reason is given).

use crate::document::OpenOptions;
use crate::engine;
use crate::properties::signature_warning;
use crate::sanitize::{has_earlier_versions, load_plain};
use lopdf::{Dictionary, Document, Object, ObjectId, Stream};
use pdfium_render::prelude::*;
use pw_core::output::{write_atomic_bytes, CollisionPolicy, CommittedOutput};
use pw_core::{ErrorCategory, PwError, PwResult};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::Read;
use std::path::Path;

/// Largest decoded stream considered (larger ones are left as they are).
const MAX_DECODED: usize = 256 * 1024 * 1024;
/// Longest side of the verification renders, in pixels.
const CHECK_PIXELS: f32 = 1000.0;
/// Rounding differences allowed per colour channel between the two renders.
const MAX_CHANNEL_DELTA: u8 = 2;
/// Passes of duplicate merging (merging fonts' files can make the font dictionaries identical).
const DEDUPE_PASSES: usize = 3;

/// What the rewrite changed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct OptimizeSummary {
    pub bytes_before: u64,
    pub bytes_after: u64,
    pub streams_recompressed: u32,
    pub duplicates_merged: u32,
    pub unused_removed: u32,
}

fn fail(what: &str) -> PwError {
    PwError::new(
        ErrorCategory::ValidationFailed,
        "output.verify",
        format!("The smaller copy failed verification ({what}) and was not saved."),
    )
    .with_action("Nothing was changed. Please report this file if it keeps happening.")
}

/// Strict zlib decode: any error, trailing garbage or excess size means "leave this stream alone".
fn inflate(data: &[u8]) -> Option<Vec<u8>> {
    let mut d = flate2::read::ZlibDecoder::new(data);
    let mut out = Vec::new();
    d.by_ref().take(MAX_DECODED as u64 + 1).read_to_end(&mut out).ok()?;
    if out.len() > MAX_DECODED {
        return None;
    }
    // The whole input must be one complete zlib stream (a few trailing whitespace bytes are common).
    let used = d.total_in() as usize;
    if data.get(used..).is_some_and(|rest| rest.iter().all(|b| b.is_ascii_whitespace())) {
        Some(out)
    } else {
        None
    }
}

fn deflate(data: &[u8]) -> Option<Vec<u8>> {
    use std::io::Write;
    let mut e = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::best());
    e.write_all(data).ok()?;
    e.finish().ok()
}

fn name_is(d: &Dictionary, key: &[u8], value: &[u8]) -> bool {
    d.get(key).ok().and_then(|o| o.as_name().ok()) == Some(value)
}

/// The stream's data when it is stored plainly or with Flate alone (no predictor), else `None`.
fn plain_data(s: &Stream) -> Option<Vec<u8>> {
    if s.dict.has(b"DecodeParms") || s.dict.has(b"F") {
        return None; // predictors / external file data: left exactly as they are
    }
    match s.dict.get(b"Filter").ok() {
        None => Some(s.content.clone()),
        Some(Object::Name(n)) if n == b"FlateDecode" => inflate(&s.content),
        Some(Object::Array(a)) if a.is_empty() => Some(s.content.clone()),
        Some(Object::Array(a)) if a.len() == 1 && a[0].as_name().ok() == Some(b"FlateDecode") => inflate(&s.content),
        _ => None,
    }
}

/// Recompress one stream if that makes it smaller; the new data is decoded again and compared.
fn recompress(s: &mut Stream) -> bool {
    // XMP metadata stays readable to tools that do not decode PDF streams (and PDF/A requires it).
    if name_is(&s.dict, b"Type", b"Metadata") || !s.allows_compression {
        return false;
    }
    let Some(data) = plain_data(s) else { return false };
    let Some(packed) = deflate(&data) else { return false };
    if packed.len() + 16 >= s.content.len() {
        return false;
    }
    if inflate(&packed).as_deref() != Some(data.as_slice()) {
        return false;
    }
    s.dict.set("Filter", Object::Name(b"FlateDecode".to_vec()));
    s.dict.remove(b"DecodeParms");
    s.set_content(packed);
    true
}

/// A canonical byte string for an object (dictionary keys sorted), for duplicate detection.
fn canon(o: &Object, out: &mut Vec<u8>) {
    match o {
        Object::Dictionary(d) => canon_dict(d, out),
        Object::Array(a) => {
            out.push(b'[');
            for x in a {
                canon(x, out);
                out.push(b' ');
            }
            out.push(b']');
        }
        Object::Stream(s) => {
            canon_dict(&s.dict, out);
            out.extend_from_slice(&Sha256::digest(&s.content));
        }
        other => out.extend_from_slice(format!("{other:?}").as_bytes()),
    }
}

fn canon_dict(d: &Dictionary, out: &mut Vec<u8>) {
    let mut keys: Vec<&Vec<u8>> = d.iter().map(|(k, _)| k).filter(|k| k.as_slice() != b"Length").collect();
    keys.sort();
    out.extend_from_slice(b"<<");
    for k in keys {
        out.push(b'/');
        out.extend_from_slice(k);
        out.push(b' ');
        if let Ok(v) = d.get(k) {
            canon(v, out);
        }
        out.push(b' ');
    }
    out.extend_from_slice(b">>");
}

/// Objects that may be shared when identical: streams (fonts, images, forms, colour profiles) and
/// font-level dictionaries. Never pages, annotations, fields or anything with an identity.
fn mergeable(o: &Object) -> bool {
    match o {
        Object::Stream(s) => !name_is(&s.dict, b"Type", b"Metadata") && !name_is(&s.dict, b"Type", b"XRef"),
        Object::Dictionary(d) => {
            name_is(d, b"Type", b"Font") || name_is(d, b"Type", b"FontDescriptor") || name_is(d, b"Type", b"ExtGState")
        }
        _ => false,
    }
}

fn redirect(o: &mut Object, map: &HashMap<ObjectId, ObjectId>) {
    match o {
        Object::Reference(id) => {
            if let Some(to) = map.get(id) {
                *id = *to;
            }
        }
        Object::Array(a) => a.iter_mut().for_each(|x| redirect(x, map)),
        Object::Dictionary(d) => d.iter_mut().for_each(|(_, v)| redirect(v, map)),
        Object::Stream(s) => s.dict.iter_mut().for_each(|(_, v)| redirect(v, map)),
        _ => {}
    }
}

/// Store identical objects once; returns how many copies were dropped.
fn merge_duplicates(doc: &mut Document) -> u32 {
    let mut merged = 0;
    for _ in 0..DEDUPE_PASSES {
        let mut first: HashMap<Vec<u8>, ObjectId> = HashMap::new();
        let mut map: HashMap<ObjectId, ObjectId> = HashMap::new();
        let mut ids: Vec<ObjectId> = doc.objects.keys().copied().collect();
        ids.sort_unstable();
        for id in ids {
            let Some(o) = doc.objects.get(&id) else { continue };
            if !mergeable(o) {
                continue;
            }
            let mut key = Vec::new();
            canon(o, &mut key);
            match first.get(&key) {
                Some(keep) => {
                    map.insert(id, *keep);
                }
                None => {
                    first.insert(key, id);
                }
            }
        }
        if map.is_empty() {
            break;
        }
        merged += map.len() as u32;
        for o in doc.objects.values_mut() {
            redirect(o, &map);
        }
        for (_, v) in doc.trailer.iter_mut() {
            redirect(v, &map);
        }
        for id in map.keys() {
            doc.objects.remove(id);
        }
    }
    merged
}

fn render(page: &PdfPage) -> PwResult<image::RgbaImage> {
    let longest = page.width().value.max(page.height().value).max(1.0);
    let config = PdfRenderConfig::new()
        .scale_page_by_factor((CHECK_PIXELS / longest).min(1.0))
        .render_form_data(true)
        .render_annotations(true);
    let bitmap = page.render_with_config(&config).map_err(|e| crate::map_pdfium_error(&e, "render"))?;
    Ok(bitmap.as_image().map_err(|e| crate::map_pdfium_error(&e, "render"))?.to_rgba8())
}

/// The copy is the same document: pages look and read the same; annotations, bookmarks and
/// attachments are all there.
fn check_same(
    pdfium: &Pdfium,
    source: &PdfDocument,
    bytes: &[u8],
    cancelled: &dyn Fn() -> bool,
    progress: &dyn Fn(u64, u64),
) -> PwResult<()> {
    let out = pdfium.load_pdf_from_byte_slice(bytes, None).map_err(|_| fail("it could not be reopened"))?;
    let n = source.pages().len();
    if out.pages().len() != n {
        return Err(fail("page count changed"));
    }
    if out.bookmarks().iter().count() != source.bookmarks().iter().count() {
        return Err(fail("bookmarks changed"));
    }
    if out.attachments().len() != source.attachments().len() {
        return Err(fail("attachments changed"));
    }
    for i in 0..n {
        if cancelled() {
            return Err(PwError::new(ErrorCategory::Cancelled, "job.cancelled", "Reducing the size was cancelled."));
        }
        let a = source.pages().get(i).map_err(|_| fail("page unreadable"))?;
        let b = out.pages().get(i).map_err(|_| fail("page unreadable"))?;
        if a.annotations().len() != b.annotations().len() {
            return Err(fail(&format!("page {} annotations changed", i + 1)));
        }
        let text = |p: &PdfPage| p.text().map(|t| t.all()).unwrap_or_default();
        if text(&a) != text(&b) {
            return Err(fail(&format!("page {} text changed", i + 1)));
        }
        let (ra, rb) = (render(&a)?, render(&b)?);
        let same = ra.dimensions() == rb.dimensions()
            && ra.pixels().zip(rb.pixels()).all(|(p, q)| (0..3).all(|c| p.0[c].abs_diff(q.0[c]) <= MAX_CHANNEL_DELTA));
        if !same {
            return Err(fail(&format!("page {} no longer looks the same", i + 1)));
        }
        progress(i as u64 + 1, n as u64);
    }
    Ok(())
}

pub fn optimize(
    input: &Path,
    opts: &OpenOptions,
    output: &Path,
    policy: CollisionPolicy,
    cancelled: &dyn Fn() -> bool,
    progress: &dyn Fn(u64, u64),
) -> PwResult<(CommittedOutput, OptimizeSummary, Vec<String>)> {
    let pdfium = engine()?;
    let (source, mut doc, raw) = load_plain(&pdfium, input, opts).map_err(|e| {
        if e.code == "sanitize.protected" {
            PwError::new(ErrorCategory::UnsupportedOperation, "optimize.protected", "This file is password protected.")
                .with_action("Remove the password first (Remove password…), then reduce the size of that copy.")
        } else {
            e
        }
    })?;
    let signatures = source.signatures().len();
    let earlier = has_earlier_versions(&raw);

    let mut summary = OptimizeSummary { bytes_before: raw.len() as u64, ..Default::default() };
    for o in doc.objects.values_mut() {
        if let Object::Stream(s) = o {
            if cancelled() {
                return Err(PwError::new(
                    ErrorCategory::Cancelled,
                    "job.cancelled",
                    "Reducing the size was cancelled.",
                ));
            }
            if recompress(s) {
                summary.streams_recompressed += 1;
            }
        }
    }
    summary.duplicates_merged = merge_duplicates(&mut doc);
    doc.trailer.remove(b"Prev");
    doc.trailer.remove(b"XRefStm");
    summary.unused_removed = doc.prune_objects().len() as u32;

    let mut bytes = Vec::new();
    let save =
        lopdf::SaveOptions::builder().use_object_streams(true).use_xref_streams(true).compression_level(9).build();
    doc.save_with_options(&mut bytes, save)
        .map_err(|_| PwError::internal("optimize.save", "The smaller copy could not be written."))?;
    summary.bytes_after = bytes.len() as u64;
    if summary.bytes_after >= summary.bytes_before {
        return Err(PwError::new(
            ErrorCategory::UnsupportedOperation,
            "optimize.not_smaller",
            "This file is already compact; a rewritten copy would not be smaller, so nothing was saved.",
        )
        .with_action("Images are kept exactly as they are; their quality is what decides the size of this file."));
    }
    if has_earlier_versions(&bytes) {
        return Err(fail("an earlier version remains"));
    }
    check_same(&pdfium, &source, &bytes, cancelled, progress)?;

    let protected_sources = [input.to_path_buf()];
    let c = write_atomic_bytes(output, policy, &protected_sources, &bytes, |staged| {
        let back = std::fs::read(staged).map_err(|e| PwError::from_io(&e, "output file"))?;
        if back == bytes {
            Ok(())
        } else {
            Err(fail("the written file differs"))
        }
    })?;
    let mut warnings = Vec::new();
    if signatures > 0 {
        warnings.push(signature_warning(signatures));
    }
    if earlier {
        warnings.push("Earlier saved versions of this file are not in the smaller copy.".into());
    }
    Ok((c, summary, warnings))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inflate_is_strict() {
        let packed = deflate(b"hello hello hello hello").expect("deflate");
        assert_eq!(inflate(&packed).as_deref(), Some(&b"hello hello hello hello"[..]));
        let mut trailing = packed.clone();
        trailing.extend_from_slice(b"\r\n");
        assert!(inflate(&trailing).is_some(), "trailing whitespace is fine");
        let mut junk = packed.clone();
        junk.extend_from_slice(b"xyz");
        assert!(inflate(&junk).is_none(), "trailing data is not");
        assert!(inflate(&packed[..packed.len() - 3]).is_none(), "truncated data is refused");
        assert!(inflate(b"not zlib").is_none());
    }

    #[test]
    fn recompress_only_when_lossless_and_smaller() {
        let text = b"BT /F1 12 Tf 72 720 Td (Hello) Tj ET\n".repeat(200);
        let mut plain = Stream::new(Dictionary::new(), text.clone());
        assert!(recompress(&mut plain));
        assert_eq!(plain_data(&plain).as_deref(), Some(text.as_slice()));

        let mut weak = Dictionary::new();
        weak.set("Filter", Object::Name(b"FlateDecode".to_vec()));
        let mut e = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::none());
        std::io::Write::write_all(&mut e, &text).expect("write");
        let mut weak = Stream::new(weak, e.finish().expect("finish"));
        assert!(recompress(&mut weak), "stored-only Flate is recompressed");
        assert_eq!(plain_data(&weak).as_deref(), Some(text.as_slice()));

        let mut dct = Dictionary::new();
        dct.set("Filter", Object::Name(b"DCTDecode".to_vec()));
        let mut dct = Stream::new(dct, vec![0xff; 4000]);
        assert!(!recompress(&mut dct), "image codecs are left alone");

        let mut pred = Dictionary::new();
        pred.set("Filter", Object::Name(b"FlateDecode".to_vec()));
        pred.set("DecodeParms", Object::Dictionary(Dictionary::from_iter(vec![("Predictor", Object::Integer(12))])));
        let mut pred = Stream::new(pred, deflate(&text).expect("deflate"));
        assert!(!recompress(&mut pred), "predictor streams are left alone");

        let mut meta = Dictionary::new();
        meta.set("Type", Object::Name(b"Metadata".to_vec()));
        let mut meta = Stream::new(meta, text.clone());
        assert!(!recompress(&mut meta), "XMP stays uncompressed");

        let mut corrupt = Dictionary::new();
        corrupt.set("Filter", Object::Name(b"FlateDecode".to_vec()));
        let mut bad = deflate(&text).expect("deflate");
        bad.truncate(bad.len() / 2);
        let before = bad.clone();
        let mut corrupt = Stream::new(corrupt, bad);
        assert!(!recompress(&mut corrupt), "damaged data is never re-encoded");
        assert_eq!(corrupt.content, before);
    }

    #[test]
    fn duplicates_are_merged_and_references_follow() {
        let mut doc = Document::with_version("1.7");
        let font = || Stream::new(Dictionary::from_iter(vec![("Length1", Object::Integer(3))]), b"FONTDATA".to_vec());
        let a = doc.add_object(Object::Stream(font()));
        let b = doc.add_object(Object::Stream(font()));
        let other = doc.add_object(Object::Stream(Stream::new(Dictionary::new(), b"OTHER".to_vec())));
        let fd = |f: ObjectId| {
            Object::Dictionary(Dictionary::from_iter(vec![
                ("Type", Object::Name(b"FontDescriptor".to_vec())),
                ("FontFile2", Object::Reference(f)),
            ]))
        };
        let da = doc.add_object(fd(a));
        let db = doc.add_object(fd(b));
        let annot = || Object::Dictionary(Dictionary::from_iter(vec![("Type", Object::Name(b"Annot".to_vec()))]));
        let n1 = doc.add_object(annot());
        let n2 = doc.add_object(annot());
        let holder = doc.add_object(Object::Array(vec![
            Object::Reference(da),
            Object::Reference(db),
            Object::Reference(other),
            Object::Reference(n1),
            Object::Reference(n2),
        ]));
        // Font file first, then (next pass) the now-identical descriptors.
        assert_eq!(merge_duplicates(&mut doc), 2);
        let refs: Vec<ObjectId> = doc
            .get_object(holder)
            .expect("holder")
            .as_array()
            .expect("array")
            .iter()
            .map(|o| o.as_reference().expect("ref"))
            .collect();
        assert_eq!(refs[0], refs[1], "descriptors merged");
        assert_eq!(refs[2], other);
        assert_ne!(refs[3], refs[4], "annotations keep their identity");
        assert!(doc.get_object(b).is_err() && doc.get_object(db).is_err());
    }
}
