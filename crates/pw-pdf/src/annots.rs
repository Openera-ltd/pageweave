//! Annotations already in a file: list them, delete them (with their pop-ups), change their
//! comment text or colour, and move them. Changes are made with lopdf in a rewritten copy and
//! checked before they are used: the copy re-parses, every touched page keeps exactly the expected
//! annotations in order, changed texts, colours and places read back, PDFium opens it with the
//! expected counts, and page content renders as before.

use crate::document::OpenOptions;
use crate::engine;
use crate::properties::signature_warning;
use crate::sanitize::{check_pages, dict_of, load_plain};
use lopdf::{Dictionary, Document, Object, ObjectId};
use pdfium_render::prelude::*;
use pw_core::ops::{AnnotationChange, RectPt};
use pw_core::{ErrorCategory, PwError, PwResult};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::path::Path;

/// Annotations listed per document (hostile files).
const MAX_LISTED: usize = 20_000;

/// Kinds that are not comments: pop-ups belong to another annotation, widgets are form fields,
/// links are navigation.
const NOT_COMMENTS: &[&str] = &["Popup", "Widget", "Link"];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExistingAnnotation {
    pub page: u32,
    /// Position in the page's annotation list (what changes refer to).
    pub index: u32,
    pub subtype: String,
    pub rect: RectPt,
    pub text: Option<String>,
    pub author: Option<String>,
    /// `YYYY-MM-DD HH:MM` when the file records it.
    pub modified: Option<String>,
    /// `#rrggbb` when the annotation has an RGB colour.
    pub color: Option<String>,
    /// Whether its comment text can be changed here (text boxes draw their text; not yet).
    pub text_editable: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnnotationList {
    pub annotations: Vec<ExistingAnnotation>,
    pub truncated: bool,
}

fn text(o: &Object) -> String {
    lopdf::decode_text_string(o).unwrap_or_else(|_| match o {
        Object::String(b, _) => b.iter().map(|&c| c as char).collect(),
        _ => String::new(),
    })
}

fn num(o: &Object) -> Option<f32> {
    match o {
        Object::Integer(i) => Some(*i as f32),
        Object::Real(r) => Some(*r),
        _ => None,
    }
}

fn pdf_date(s: &str) -> Option<String> {
    let d = s.trim().strip_prefix("D:").unwrap_or(s.trim());
    let digits: String = d.chars().take_while(|c| c.is_ascii_digit()).collect();
    (digits.len() >= 12).then(|| {
        format!("{}-{}-{} {}:{}", &digits[0..4], &digits[4..6], &digits[6..8], &digits[8..10], &digits[10..12])
    })
}

fn protected() -> PwError {
    PwError::new(
        ErrorCategory::UnsupportedOperation,
        "annotation.protected",
        "Annotations in a password-protected file can't be changed here.",
    )
    .with_action("Remove the password first (Remove password…), then change that copy.")
}

/// The page's annotation entries (references or inline dictionaries) and where the array lives.
fn annots_of(doc: &Document, page: ObjectId) -> (Vec<Object>, Option<ObjectId>) {
    let Ok(pd) = doc.get_dictionary(page) else { return (vec![], None) };
    match pd.get(b"Annots") {
        Ok(Object::Reference(r)) => match doc.get_object(*r) {
            Ok(Object::Array(a)) => (a.clone(), Some(*r)),
            _ => (vec![], None),
        },
        Ok(Object::Array(a)) => (a.clone(), None),
        _ => (vec![], None),
    }
}

fn describe(doc: &Document, page: u32, index: u32, d: &Dictionary) -> Option<ExistingAnnotation> {
    let subtype =
        d.get(b"Subtype").ok().and_then(|o| o.as_name().ok()).map(|n| String::from_utf8_lossy(n).into_owned())?;
    if NOT_COMMENTS.contains(&subtype.as_str()) {
        return None;
    }
    let r: Vec<f32> = d
        .get(b"Rect")
        .ok()
        .and_then(|o| doc.dereference(o).ok().map(|x| x.1))
        .and_then(|o| o.as_array().ok())
        .map(|a| a.iter().filter_map(num).collect())
        .unwrap_or_default();
    let rect = if r.len() == 4 {
        RectPt { left: r[0].min(r[2]), bottom: r[1].min(r[3]), right: r[0].max(r[2]), top: r[1].max(r[3]) }
    } else {
        RectPt { left: 0.0, bottom: 0.0, right: 0.0, top: 0.0 }
    };
    let get_text = |k: &[u8]| d.get(k).ok().map(text).map(|t| t.trim().to_string()).filter(|t| !t.is_empty());
    let color = d.get(b"C").ok().and_then(|o| o.as_array().ok()).and_then(|a| {
        let v: Vec<f32> = a.iter().filter_map(num).collect();
        (v.len() == 3).then(|| {
            let c = |x: f32| (x.clamp(0.0, 1.0) * 255.0).round() as u8;
            format!("#{:02x}{:02x}{:02x}", c(v[0]), c(v[1]), c(v[2]))
        })
    });
    Some(ExistingAnnotation {
        page,
        index,
        text_editable: subtype != "FreeText",
        subtype,
        rect,
        text: get_text(b"Contents"),
        author: get_text(b"T"),
        modified: d.get(b"M").ok().map(text).and_then(|s| pdf_date(&s)),
        color,
    })
}

/// The annotations in `input` that are comments (read-only).
pub fn list_annotations(input: &Path, opts: &OpenOptions) -> PwResult<AnnotationList> {
    let pdfium = engine()?;
    // Listing is read-only: a protected file is read with the password it was opened with.
    let (_src, doc, _raw) = crate::sanitize::load_readable(&pdfium, input, opts).map_err(|e| {
        if e.code == "sanitize.protected" {
            protected()
        } else {
            e
        }
    })?;
    let mut out = Vec::new();
    for (n, pid) in doc.get_pages() {
        let (entries, _) = annots_of(&doc, pid);
        for (i, e) in entries.iter().enumerate() {
            if out.len() >= MAX_LISTED {
                return Ok(AnnotationList { annotations: out, truncated: true });
            }
            if let Some(d) = dict_of(&doc, e) {
                if let Some(a) = describe(&doc, n - 1, i as u32, d) {
                    out.push(a);
                }
            }
        }
    }
    Ok(AnnotationList { annotations: out, truncated: false })
}

fn fail(what: &str) -> PwError {
    PwError::new(
        ErrorCategory::ValidationFailed,
        "output.verify",
        format!("The copy with annotation changes failed verification ({what}) and was not saved."),
    )
}

/// What a touched page must hold afterwards: subtypes in order, and changed texts by new position.
struct PageExpect {
    subtypes: Vec<String>,
    texts: Vec<(usize, String)>,
    colors: Vec<(usize, [f32; 3])>,
    rects: Vec<(usize, [f32; 4])>,
}

/// Geometry entries given in page coordinates (x, y pairs; /InkList holds arrays of them). The
/// stored appearance is drawn into /Rect, so it moves with it.
const GEOMETRY: &[&[u8]] = &[b"Rect", b"QuadPoints", b"Vertices", b"L", b"CL", b"InkList"];

/// `o` (resolved) with every x, y pair mapped by `f`; `None` when it is not numbers.
fn mapped(doc: &Document, o: &Object, f: &dyn Fn(f32, f32) -> (f32, f32)) -> Option<Object> {
    let o = doc.dereference(o).ok()?.1;
    let a = o.as_array().ok()?;
    if a.iter().all(|v| num(v).is_some()) {
        let v: Vec<f32> = a.iter().filter_map(num).collect();
        let pts = v.chunks_exact(2).flat_map(|p| {
            let (x, y) = f(p[0], p[1]);
            [Object::Real(x), Object::Real(y)]
        });
        return Some(Object::Array(pts.collect()));
    }
    a.iter().map(|v| mapped(doc, v, f)).collect::<Option<Vec<_>>>().map(Object::Array)
}

/// The geometry entries of `entry` fitted from its /Rect into `to` (each point scaled with the box).
fn fitted_geometry(doc: &Document, entry: &Object, from: [f32; 4], to: [f32; 4]) -> Vec<(&'static [u8], Object)> {
    let Some(d) = dict_of(doc, entry) else { return vec![] };
    let sx = (to[2] - to[0]) / (from[2] - from[0]).max(0.01);
    let sy = (to[3] - to[1]) / (from[3] - from[1]).max(0.01);
    let f = move |x: f32, y: f32| (to[0] + (x - from[0]) * sx, to[1] + (y - from[1]) * sy);
    let mut out: Vec<(&'static [u8], Object)> = GEOMETRY
        .iter()
        .filter(|k| **k != b"Rect")
        .filter_map(|k| d.get(k).ok().and_then(|o| mapped(doc, o, &f)).map(|o| (*k, o)))
        .collect();
    out.push((b"Rect", Object::Array(to.iter().map(|&v| Object::Real(v)).collect())));
    out
}

/// The moved geometry entries of the annotation `entry` (to set on its dictionary).
fn moved_geometry(doc: &Document, entry: &Object, dx: f32, dy: f32) -> Vec<(&'static [u8], Object)> {
    let Some(d) = dict_of(doc, entry) else { return vec![] };
    let f = move |x: f32, y: f32| (x + dx, y + dy);
    GEOMETRY.iter().filter_map(|k| d.get(k).ok().and_then(|o| mapped(doc, o, &f)).map(|o| (*k, o))).collect()
}

fn rect_of(doc: &Document, entry: &Object) -> Option<[f32; 4]> {
    let r: Vec<f32> = dict_of(doc, entry)?
        .get(b"Rect")
        .ok()
        .and_then(|o| doc.dereference(o).ok().map(|x| x.1))
        .and_then(|o| o.as_array().ok())
        .map(|a| a.iter().filter_map(num).collect())?;
    (r.len() == 4).then(|| [r[0].min(r[2]), r[1].min(r[3]), r[0].max(r[2]), r[1].max(r[3])])
}

fn edit_entry(doc: &mut Document, entry: &mut Object, f: impl FnOnce(&mut Dictionary)) -> PwResult<()> {
    match entry {
        Object::Reference(r) => f(doc.get_dictionary_mut(*r).map_err(|_| changed())?),
        Object::Dictionary(d) => f(d),
        _ => return Err(changed()),
    }
    Ok(())
}

fn subtype_of(doc: &Document, o: &Object) -> String {
    dict_of(doc, o)
        .and_then(|d| d.get(b"Subtype").ok())
        .and_then(|n| n.as_name().ok())
        .map(|n| String::from_utf8_lossy(n).into_owned())
        .unwrap_or_default()
}

/// Apply `changes` to a rewritten copy of `input`; returns its verified bytes and warnings.
pub(crate) fn apply_annotation_changes(
    input: &Path,
    opts: &OpenOptions,
    changes: &[AnnotationChange],
    cancelled: &dyn Fn() -> bool,
) -> PwResult<(Vec<u8>, Vec<String>)> {
    let pdfium = engine()?;
    let (source, mut doc, _raw) =
        load_plain(&pdfium, input, opts).map_err(|e| if e.code == "sanitize.protected" { protected() } else { e })?;
    let pages = doc.get_pages();
    let mut by_page: BTreeMap<u32, Vec<&AnnotationChange>> = BTreeMap::new();
    for c in changes {
        by_page.entry(c.page).or_default().push(c);
    }
    let now = time::OffsetDateTime::now_utc();
    let stamp = format!(
        "D:{:04}{:02}{:02}{:02}{:02}{:02}Z",
        now.year(),
        u8::from(now.month()),
        now.day(),
        now.hour(),
        now.minute(),
        now.second()
    );
    let mut expect: BTreeMap<u32, PageExpect> = BTreeMap::new();
    let mut removed_total = 0usize;
    for (page, list) in &by_page {
        let pid = *pages.get(&(page + 1)).ok_or_else(|| {
            PwError::invalid("annotation.page", format!("Page {} does not exist in this document.", page + 1))
        })?;
        let (mut entries, array_obj) = annots_of(&doc, pid);
        let mut remove: HashSet<usize> = HashSet::new();
        let mut new_text: BTreeMap<usize, String> = BTreeMap::new();
        let mut new_color: BTreeMap<usize, [f32; 3]> = BTreeMap::new();
        let mut new_rect: BTreeMap<usize, [f32; 4]> = BTreeMap::new();
        for c in list {
            let i = c.index as usize;
            let entry = entries.get(i).cloned().ok_or_else(changed)?;
            if subtype_of(&doc, &entry) != c.subtype {
                return Err(changed());
            }
            if c.delete {
                remove.insert(i);
                // Its pop-up goes with it.
                let popup =
                    dict_of(&doc, &entry).and_then(|d| d.get(b"Popup").ok()).and_then(|p| p.as_reference().ok());
                if let Some(p) = popup {
                    if let Some(pi) = entries.iter().position(|e| e.as_reference().ok() == Some(p)) {
                        remove.insert(pi);
                    }
                }
            }
            if let (false, Some(col)) = (c.delete, &c.color) {
                let v = u32::from_str_radix(col.trim_start_matches('#'), 16).map_err(|_| changed())?;
                let rgb =
                    [(v >> 16) as u8, (v >> 8) as u8, v as u8].map(|b| (b as f32 / 255.0 * 1000.0).round() / 1000.0);
                // The stored appearance carries the old colour: dropping it makes viewers draw the
                // annotation from /C (PDFium and Acrobat regenerate it for these types).
                let set = |d: &mut Dictionary| {
                    d.set("C", Object::Array(rgb.iter().map(|&x| Object::Real(x)).collect()));
                    d.remove(b"AP");
                    d.set("M", Object::string_literal(stamp.clone()));
                };
                edit_entry(&mut doc, &mut entries[i], set)?;
                new_color.insert(i, rgb);
            }
            if let (false, Some([dx, dy])) = (c.delete, c.offset) {
                let before = rect_of(&doc, &entries[i]).ok_or_else(changed)?;
                let geometry = moved_geometry(&doc, &entries[i], dx, dy);
                let stamp = stamp.clone();
                edit_entry(&mut doc, &mut entries[i], |d| {
                    for (k, v) in geometry {
                        d.set(k, v);
                    }
                    d.set("M", Object::string_literal(stamp));
                })?;
                new_rect.insert(i, [before[0] + dx, before[1] + dy, before[2] + dx, before[3] + dy]);
                // Its pop-up (where the comment opens) moves along.
                let popup =
                    dict_of(&doc, &entries[i]).and_then(|d| d.get(b"Popup").ok()).and_then(|p| p.as_reference().ok());
                if let Some(p) = popup {
                    let geometry = moved_geometry(&doc, &Object::Reference(p), dx, dy);
                    if let Ok(pd) = doc.get_dictionary_mut(p) {
                        for (k, v) in geometry {
                            pd.set(k, v);
                        }
                    }
                }
            }
            if let (false, Some(r)) = (c.delete, &c.rect) {
                let from = rect_of(&doc, &entries[i]).ok_or_else(changed)?;
                let to = [r.left, r.bottom, r.right, r.top];
                let geometry = fitted_geometry(&doc, &entries[i], from, to);
                let stamp = stamp.clone();
                // Shapes and drawings are redrawn from their scaled geometry (a stored appearance
                // would keep the old line widths stretched); a stamp's picture is stretched into /Rect.
                let redraw = c.subtype != "Stamp";
                edit_entry(&mut doc, &mut entries[i], |d| {
                    for (k, v) in geometry {
                        d.set(k, v);
                    }
                    if redraw {
                        d.remove(b"AP");
                    }
                    d.set("M", Object::string_literal(stamp));
                })?;
                new_rect.insert(i, to);
            }
            if let (false, Some(t)) = (c.delete, &c.text) {
                if c.subtype == "FreeText" {
                    return Err(PwError::invalid(
                        "annotation.freetext",
                        "The text of a text box can't be changed here yet (it is drawn on the page).",
                    ));
                }
                let set = |d: &mut Dictionary| {
                    if t.trim().is_empty() {
                        d.remove(b"Contents");
                    } else {
                        d.set("Contents", lopdf::text_string(t));
                    }
                    d.set("M", Object::string_literal(stamp.clone()));
                };
                edit_entry(&mut doc, &mut entries[i], set)?;
                new_text.insert(i, t.trim().to_string());
            }
        }
        let kept: Vec<(usize, Object)> = entries.into_iter().enumerate().filter(|(i, _)| !remove.contains(i)).collect();
        removed_total += remove.len();
        let subtypes = kept.iter().map(|(_, o)| subtype_of(&doc, o)).collect();
        let texts = kept
            .iter()
            .enumerate()
            .filter_map(|(new_i, (old_i, _))| new_text.get(old_i).map(|t| (new_i, t.clone())))
            .collect();
        let colors = kept
            .iter()
            .enumerate()
            .filter_map(|(new_i, (old_i, _))| new_color.get(old_i).map(|c| (new_i, *c)))
            .collect();
        let rects = kept
            .iter()
            .enumerate()
            .filter_map(|(new_i, (old_i, _))| new_rect.get(old_i).map(|r| (new_i, *r)))
            .collect();
        expect.insert(*page, PageExpect { subtypes, texts, colors, rects });
        let arr = Object::Array(kept.into_iter().map(|(_, o)| o).collect());
        match array_obj {
            Some(id) => {
                doc.objects.insert(id, arr);
            }
            None => {
                let pd = doc.get_dictionary_mut(pid).map_err(|_| changed())?;
                pd.set("Annots", arr);
            }
        }
    }
    doc.trailer.remove(b"Prev");
    doc.trailer.remove(b"XRefStm");
    doc.prune_objects();
    let mut bytes = Vec::new();
    doc.save_to(&mut bytes).map_err(|_| PwError::internal("annotation.save", "The copy could not be written."))?;

    // Independent read-back of every touched page.
    let back = Document::load_mem(&bytes).map_err(|_| fail("it could not be re-read"))?;
    let back_pages = back.get_pages();
    for (page, want) in &expect {
        let pid = *back_pages.get(&(page + 1)).ok_or_else(|| fail("a page is missing"))?;
        let (entries, _) = annots_of(&back, pid);
        let got: Vec<String> = entries.iter().map(|o| subtype_of(&back, o)).collect();
        if got != want.subtypes {
            return Err(fail(&format!("page {} has different annotations than intended", page + 1)));
        }
        for (i, t) in &want.texts {
            let have = dict_of(&back, &entries[*i]).and_then(|d| d.get(b"Contents").ok()).map(text).unwrap_or_default();
            if have.trim() != t {
                return Err(fail(&format!("a comment on page {} did not take its new text", page + 1)));
            }
        }
        for (i, want_c) in &want.colors {
            let have: Vec<f32> = dict_of(&back, &entries[*i])
                .and_then(|d| d.get(b"C").ok())
                .and_then(|c| c.as_array().ok())
                .map(|a| a.iter().filter_map(|v| v.as_float().ok()).collect())
                .unwrap_or_default();
            if have.len() != 3 || have.iter().zip(want_c).any(|(a, b)| (a - b).abs() > 0.002) {
                return Err(fail(&format!("an annotation on page {} did not take its new colour", page + 1)));
            }
        }
        for (i, want_r) in &want.rects {
            let ok =
                rect_of(&back, &entries[*i]).is_some_and(|r| r.iter().zip(want_r).all(|(a, b)| (a - b).abs() < 0.01));
            if !ok {
                return Err(fail(&format!("an annotation on page {} did not move where intended", page + 1)));
            }
        }
    }
    let reopened = pdfium.load_pdf_from_byte_slice(&bytes, None).map_err(|_| fail("it could not be reopened"))?;
    for (page, want) in &expect {
        let n = reopened.pages().get(*page as PdfPageIndex).map(|p| p.annotations().len()).unwrap_or(0);
        if n != want.subtypes.len() {
            return Err(fail(&format!("page {} has {n} annotations, expected {}", page + 1, want.subtypes.len())));
        }
    }
    drop(reopened);
    check_pages(&pdfium, &source, &bytes, cancelled, &|_, _| {})?;
    let mut warnings = Vec::new();
    let signatures = source.signatures().len();
    if signatures > 0 {
        warnings.push(signature_warning(signatures));
    }
    if removed_total > 0 {
        warnings.push(format!("{removed_total} annotation(s) were removed (pop-ups included)."));
    }
    Ok((bytes, warnings))
}

fn changed() -> PwError {
    PwError::new(
        ErrorCategory::ValidationFailed,
        "annotation.changed",
        "An annotation is not where it was when the list was made; reopen the document and try again.",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drawing_strokes_scale_with_their_box() {
        let doc = Document::with_version("1.7");
        let mut d = Dictionary::new();
        d.set("Rect", Object::Array(vec![10.into(), 10.into(), 30.into(), 20.into()]));
        let stroke = |p: &[i64]| Object::Array(p.iter().map(|&v| v.into()).collect());
        d.set("InkList", Object::Array(vec![stroke(&[10, 10, 30, 20]), stroke(&[20, 15])]));
        let got = fitted_geometry(&doc, &Object::Dictionary(d), [10.0, 10.0, 30.0, 20.0], [100.0, 0.0, 140.0, 40.0]);
        let ink = got.iter().find(|(k, _)| *k == b"InkList").map(|(_, o)| o.clone()).expect("ink");
        let pts: Vec<Vec<f32>> = ink
            .as_array()
            .expect("strokes")
            .iter()
            .map(|s| s.as_array().expect("stroke").iter().filter_map(num).collect())
            .collect();
        assert_eq!(pts, vec![vec![100.0, 0.0, 140.0, 40.0], vec![120.0, 20.0]]);
        let rect = got.iter().find(|(k, _)| *k == b"Rect").map(|(_, o)| o.clone()).expect("rect");
        assert_eq!(
            rect.as_array().expect("rect").iter().filter_map(num).collect::<Vec<_>>(),
            [100.0, 0.0, 140.0, 40.0]
        );
    }
}
