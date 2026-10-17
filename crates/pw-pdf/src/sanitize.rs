//! Sanitize (PRD 05 "Passwords, redaction and sanitization"): a new copy without the chosen kinds
//! of hidden information — metadata, scripts and actions, attachments, hidden layers, comments,
//! form fields, web links. The copy is rewritten from scratch by lopdf, so earlier saved versions
//! (incremental updates) never carry over. Before commit it is verified two ways: an independent
//! walk of the written file must find none of the removed kinds anywhere, and every page must
//! render (PDFium, without annotations) exactly as in the original, so removal cannot have changed
//! what a page shows.
//!
//! Not detected (stated in the UI): text drawn in white or hidden behind other content, and content
//! outside the visible page area.

use crate::content_ops::{tokenize, RawOp};
use crate::document::{load, OpenOptions};
use crate::edit::save_bytes;
use crate::{engine, map_pdfium_error};
use image::RgbaImage;
use lopdf::{Dictionary, Document, Object, ObjectId, Stream};
use pdfium_render::prelude::*;
use pw_core::ops::SanitizeOptions;
use pw_core::output::{write_atomic_bytes, CollisionPolicy, CommittedOutput};
use pw_core::{ErrorCategory, PwError, PwResult};
use serde::Serialize;
use std::collections::HashSet;
use std::path::Path;

/// Markup and comment annotations ("Comments"), including pending redaction marks and popups.
const COMMENT_TYPES: &[&[u8]] = &[
    b"Text",
    b"FreeText",
    b"Line",
    b"Square",
    b"Circle",
    b"Polygon",
    b"PolyLine",
    b"Highlight",
    b"Underline",
    b"Squiggly",
    b"StrikeOut",
    b"Caret",
    b"Ink",
    b"Popup",
    b"Stamp",
    b"Redact",
];
/// Annotations that carry embedded files or media ("Attachments").
const MEDIA_TYPES: &[&[u8]] = &[b"FileAttachment", b"Sound", b"Movie", b"Screen", b"RichMedia", b"3D"];
/// Action types (PDF 2.0, 12.6.4); used to recognise action dictionaries without /Type.
const ACTION_TYPES: &[&[u8]] = &[
    b"GoTo",
    b"GoToR",
    b"GoToE",
    b"GoToDp",
    b"Launch",
    b"Thread",
    b"URI",
    b"Sound",
    b"Movie",
    b"Hide",
    b"Named",
    b"SubmitForm",
    b"ResetForm",
    b"ImportData",
    b"JavaScript",
    b"SetOCGState",
    b"Rendition",
    b"Trans",
    b"GoTo3DView",
    b"RichMediaExecute",
];
/// Named actions that only navigate.
const SAFE_NAMED: &[&[u8]] = &[b"NextPage", b"PrevPage", b"FirstPage", b"LastPage"];
/// Arrays whose entries are a list (a dead reference is dropped rather than replaced by null).
const LIST_KEYS: &[&[u8]] = &[b"Annots", b"Fields", b"Kids", b"K", b"Next", b"AF", b"CO", b"OCGs", b"Order"];
/// Decompressed size limit per content stream (hostile files).
const MAX_STREAM_BYTES: usize = 256 << 20;
/// Longest side of a verification render, in pixels.
const CHECK_PIXELS: f32 = 1000.0;
/// Per-channel difference tolerated between the original and cleaned renders.
const MAX_CHANNEL_DELTA: u8 = 2;
const MAX_NAMES: usize = 20;
const MAX_DEPTH: u32 = 64;

/// What hidden information a file holds, per category (shown before cleaning, and in the result as
/// what was removed).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct SanitizeFindings {
    /// Kinds of metadata present ("Title", "Author", "XMP metadata", "Page thumbnails", …).
    pub metadata: Vec<String>,
    /// Scripts and actions that run code, launch, open other files, submit or import data.
    pub scripts: u32,
    /// Embedded files and media items.
    pub attachments: u32,
    /// Names of embedded files (at most 20).
    pub attachment_names: Vec<String>,
    /// All layers, and the names of hidden ones.
    pub layers: u32,
    pub hidden_layers: Vec<String>,
    pub comments: u32,
    pub form_fields: u32,
    pub web_links: u32,
    /// The file holds earlier saved versions (incremental updates).
    pub earlier_versions: bool,
}

impl SanitizeFindings {
    /// Only the categories `o` removes.
    fn selected(mut self, o: &SanitizeOptions) -> Self {
        if !o.metadata {
            self.metadata.clear();
        }
        if !o.scripts {
            self.scripts = 0;
        }
        if !o.attachments {
            self.attachments = 0;
            self.attachment_names.clear();
        }
        if !o.hidden_layers {
            self.layers = 0;
            self.hidden_layers.clear();
        }
        if !o.comments {
            self.comments = 0;
        }
        if !o.form_fields {
            self.form_fields = 0;
        }
        if !o.web_links {
            self.web_links = 0;
        }
        self
    }
}

fn fail(what: &str) -> PwError {
    PwError::new(
        ErrorCategory::ValidationFailed,
        "output.verify",
        format!("The cleaned copy failed verification ({what}) and was not saved."),
    )
}

fn protected() -> PwError {
    PwError::new(
        ErrorCategory::UnsupportedOperation,
        "sanitize.protected",
        "Password-protected files cannot be cleaned.",
    )
    .with_action("Remove the password first (Remove password…), then clean that copy.")
}

fn unreadable() -> PwError {
    PwError::new(
        ErrorCategory::Corrupt,
        "sanitize.content",
        "Some page content could not be read, so hidden layers cannot be removed safely. Try again without removing hidden layers.",
    )
}

// ---- small lopdf helpers --------------------------------------------------------------------

fn name<'a>(d: &'a Dictionary, key: &[u8]) -> Option<&'a [u8]> {
    d.get(key).ok().and_then(|o| o.as_name().ok())
}

fn dict_of<'a>(doc: &'a Document, o: &'a Object) -> Option<&'a Dictionary> {
    match doc.dereference(o).ok()?.1 {
        Object::Dictionary(d) => Some(d),
        Object::Stream(s) => Some(&s.dict),
        _ => None,
    }
}

fn dict_ref(o: &Object) -> Option<&Dictionary> {
    match o {
        Object::Dictionary(d) => Some(d),
        Object::Stream(s) => Some(&s.dict),
        _ => None,
    }
}

fn catalog(doc: &Document) -> Option<&Dictionary> {
    doc.trailer.get(b"Root").ok().and_then(|r| dict_of(doc, r))
}

fn refs(o: Option<&Object>) -> Vec<ObjectId> {
    match o {
        Some(Object::Reference(r)) => vec![*r],
        Some(Object::Array(a)) => a.iter().filter_map(|x| x.as_reference().ok()).collect(),
        _ => Vec::new(),
    }
}

fn text(o: &Object) -> String {
    lopdf::decode_text_string(o).unwrap_or_else(|_| match o {
        Object::String(b, _) => b.iter().map(|&c| c as char).collect(),
        _ => String::new(),
    })
}

/// Name token from a content stream (`/Name#20x`) as a dictionary key.
fn decode_name(tok: &[u8]) -> Option<Vec<u8>> {
    let n = tok.strip_prefix(b"/")?;
    let mut out = Vec::with_capacity(n.len());
    let mut i = 0;
    while i < n.len() {
        if n[i] == b'#' {
            let hex = n.get(i + 1..i + 3).and_then(|h| std::str::from_utf8(h).ok());
            if let Some(v) = hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(n[i]);
        i += 1;
    }
    Some(out)
}

fn stream_data(s: &Stream) -> Option<Vec<u8>> {
    if s.dict.has(b"Filter") {
        s.decompressed_content_with_limit(MAX_STREAM_BYTES).ok()
    } else {
        Some(s.content.clone())
    }
}

// ---- classification -------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Act {
    Navigation,
    Web,
    Embedded,
    Layer,
    Unsafe,
}

/// The kind of action `d` is, if it is an action dictionary.
fn action_class(d: &Dictionary) -> Option<Act> {
    let s = name(d, b"S")?;
    match name(d, b"Type") {
        Some(b"Action") => {}
        Some(_) => return None,
        None if !ACTION_TYPES.contains(&s) || d.has(b"K") => return None,
        None => {}
    }
    Some(match s {
        b"GoTo" => Act::Navigation,
        b"Named" if name(d, b"N").is_some_and(|n| SAFE_NAMED.contains(&n)) => Act::Navigation,
        b"URI" => Act::Web,
        b"GoToE" => Act::Embedded,
        b"SetOCGState" => Act::Layer,
        _ => Act::Unsafe,
    })
}

fn action_removed(a: Act, o: &SanitizeOptions) -> bool {
    match a {
        Act::Navigation => false,
        Act::Web => o.web_links,
        Act::Embedded => o.scripts || o.attachments,
        Act::Layer => o.scripts || o.hidden_layers,
        Act::Unsafe => o.scripts,
    }
}

/// Subtype of an annotation dictionary.
fn annot_subtype(d: &Dictionary) -> Option<&[u8]> {
    let st = name(d, b"Subtype")?;
    let annot = match name(d, b"Type") {
        Some(t) => t == b"Annot",
        None => d.has(b"Rect"),
    };
    annot.then_some(st)
}

fn annot_removed(st: &[u8], o: &SanitizeOptions) -> bool {
    (o.comments && COMMENT_TYPES.contains(&st))
        || (o.attachments && MEDIA_TYPES.contains(&st))
        || (o.form_fields && st == b"Widget")
}

// ---- layers (optional content) --------------------------------------------------------------

#[derive(Default)]
struct Layers {
    /// Groups hidden in the default configuration.
    off: HashSet<ObjectId>,
    all: Vec<(ObjectId, String)>,
}

fn read_layers(doc: &Document) -> Layers {
    let Some(ocp) = catalog(doc).and_then(|c| c.get(b"OCProperties").ok()).and_then(|o| dict_of(doc, o)) else {
        return Layers::default();
    };
    let all_ids = refs(ocp.get(b"OCGs").ok().map(|o| doc.dereference(o).map(|x| x.1).unwrap_or(o)));
    let d = ocp.get(b"D").ok().and_then(|o| dict_of(doc, o));
    let list = |k: &[u8]| -> HashSet<ObjectId> {
        refs(d.and_then(|d| d.get(k).ok()).map(|o| doc.dereference(o).map(|x| x.1).unwrap_or(o))).into_iter().collect()
    };
    let base_off = d.and_then(|d| name(d, b"BaseState")) == Some(b"OFF");
    let on = list(b"ON");
    let off = if base_off { all_ids.iter().copied().filter(|g| !on.contains(g)).collect() } else { list(b"OFF") };
    let all = all_ids
        .iter()
        .map(|&id| {
            let n = doc.get_dictionary(id).ok().and_then(|g| g.get(b"Name").ok()).map(text).unwrap_or_default();
            (id, n)
        })
        .collect();
    Layers { off, all }
}

/// Whether content marked with the optional-content group or membership dictionary `o` is shown.
/// Unknown forms count as visible (content is kept; the render check guards the result).
fn oc_visible(doc: &Document, layers: &Layers, o: &Object, depth: u32) -> bool {
    if depth > 16 {
        return true;
    }
    let (id, d) = match o {
        Object::Reference(id) => (Some(*id), doc.get_object(*id).ok().and_then(dict_ref)),
        other => (None, dict_ref(other)),
    };
    let Some(d) = d else { return true };
    match name(d, b"Type") {
        Some(b"OCG") => id.is_none_or(|id| !layers.off.contains(&id)),
        Some(b"OCMD") => {
            if let Ok(ve) = d.get(b"VE") {
                return ve_visible(doc, layers, ve, depth + 1);
            }
            let groups: Vec<bool> = match d.get(b"OCGs") {
                Ok(Object::Array(a)) => a.iter().map(|g| oc_visible(doc, layers, g, depth + 1)).collect(),
                Ok(Object::Null) | Err(_) => Vec::new(),
                Ok(g) => vec![oc_visible(doc, layers, g, depth + 1)],
            };
            if groups.is_empty() {
                return true;
            }
            match name(d, b"P").unwrap_or(b"AnyOn") {
                b"AllOn" => groups.iter().all(|&v| v),
                b"AnyOff" => groups.iter().any(|&v| !v),
                b"AllOff" => groups.iter().all(|&v| !v),
                _ => groups.iter().any(|&v| v),
            }
        }
        _ => true,
    }
}

fn ve_visible(doc: &Document, layers: &Layers, o: &Object, depth: u32) -> bool {
    if depth > 16 {
        return true;
    }
    match o {
        Object::Array(a) => {
            let args: Vec<bool> = a.iter().skip(1).map(|x| ve_visible(doc, layers, x, depth + 1)).collect();
            match a.first().and_then(|x| x.as_name().ok()) {
                Some(b"And") => args.iter().all(|&v| v),
                Some(b"Or") => args.iter().any(|&v| v),
                Some(b"Not") => !args.first().copied().unwrap_or(false),
                _ => true,
            }
        }
        other => oc_visible(doc, layers, other, depth + 1),
    }
}

// ---- content rewriting for hidden layers ----------------------------------------------------

struct Rewritten {
    data: Vec<u8>,
    changed: bool,
    /// XObject names the rewritten stream still draws.
    used: HashSet<Vec<u8>>,
    removed: u32,
}

/// Painting operators: replaced by `n` inside hidden content so clipping paths still apply.
fn is_paint(op: &[u8]) -> bool {
    matches!(op, b"S" | b"s" | b"f" | b"F" | b"f*" | b"B" | b"B*" | b"b" | b"b*")
}

/// Remove content marked as hidden optional content (and draws of hidden XObjects) from one stream;
/// visible optional-content markers become plain marked content. Graphics-state operators inside
/// hidden content are kept, so the state of the content that follows is unchanged.
fn rewrite_content(
    doc: &Document,
    layers: &Layers,
    data: &[u8],
    res: Option<&Dictionary>,
) -> Result<Rewritten, crate::content_ops::Unreadable> {
    let ops: Vec<RawOp> = tokenize(data)?;
    let sub = |key: &[u8]| res.and_then(|r| r.get(key).ok()).and_then(|o| dict_of(doc, o));
    let props = sub(b"Properties");
    let xobjects = sub(b"XObject");
    let prop_visible = |tok: Option<&&[u8]>| -> bool {
        let Some(key) = tok.and_then(|t| decode_name(t)) else { return true };
        props.and_then(|p| p.get(&key).ok()).is_none_or(|o| oc_visible(doc, layers, o, 0))
    };
    let xobject_hidden = |key: &[u8]| -> bool {
        xobjects
            .and_then(|x| x.get(key).ok())
            .and_then(|o| dict_of(doc, o))
            .and_then(|d| d.get(b"OC").ok())
            .is_some_and(|oc| !oc_visible(doc, layers, oc, 0))
    };
    #[derive(PartialEq)]
    enum Mc {
        Hidden,
        Other,
    }
    let mut stack: Vec<Mc> = Vec::new();
    let mut out = Vec::with_capacity(data.len());
    let mut last = 0usize;
    let mut changed = false;
    let mut removed = 0u32;
    let mut used = HashSet::new();
    for op in &ops {
        let hidden = stack.contains(&Mc::Hidden);
        let rep: Option<Vec<u8>> = match op.operator {
            b"BDC" => {
                let is_oc = op.operands.first() == Some(&&b"/OC"[..]);
                if hidden {
                    stack.push(Mc::Other);
                    Some(Vec::new())
                } else if is_oc && !prop_visible(op.operands.get(1)) {
                    stack.push(Mc::Hidden);
                    removed += 1;
                    Some(Vec::new())
                } else if is_oc {
                    stack.push(Mc::Other);
                    Some(b"/OC BMC".to_vec())
                } else {
                    stack.push(Mc::Other);
                    None
                }
            }
            b"BMC" => {
                stack.push(Mc::Other);
                hidden.then(Vec::new)
            }
            b"EMC" => {
                stack.pop();
                hidden.then(Vec::new)
            }
            _ if hidden => match op.operator {
                b"Tj" | b"TJ" | b"Do" | b"sh" | b"BI" => Some(Vec::new()),
                b"'" => Some(b"T*".to_vec()),
                b"\"" => match (op.operands.first(), op.operands.get(1)) {
                    (Some(aw), Some(ac)) => Some([*aw, b" Tw ", *ac, b" Tc T*"].concat()),
                    _ => Some(b"T*".to_vec()),
                },
                o if is_paint(o) => Some(b"n".to_vec()),
                _ => None,
            },
            b"Do" => {
                let key = op.operands.first().and_then(|t| decode_name(t)).unwrap_or_default();
                if xobject_hidden(&key) {
                    removed += 1;
                    Some(Vec::new())
                } else {
                    used.insert(key);
                    None
                }
            }
            _ => None,
        };
        if let Some(r) = rep {
            out.extend_from_slice(&data[last..op.start]);
            out.push(b' ');
            out.extend_from_slice(&r);
            out.push(b' ');
            last = op.end;
            changed = true;
        }
    }
    out.extend_from_slice(&data[last..]);
    Ok(Rewritten { data: out, changed, used, removed })
}

/// Effective resources of a page (inherited from the page tree when absent).
fn page_resources(doc: &Document, page: ObjectId) -> Option<Dictionary> {
    let mut id = page;
    for _ in 0..MAX_DEPTH {
        let d = doc.get_dictionary(id).ok()?;
        if let Ok(r) = d.get(b"Resources") {
            return dict_of(doc, r).cloned();
        }
        id = d.get(b"Parent").ok()?.as_reference().ok()?;
    }
    None
}

/// `res` with only the XObjects in `used`.
fn filtered(doc: &Document, res: &Dictionary, used: &HashSet<Vec<u8>>) -> Dictionary {
    let mut res = res.clone();
    if let Some(xo) = res.get(b"XObject").ok().and_then(|o| dict_of(doc, o)).cloned() {
        let mut kept = Dictionary::new();
        for (k, v) in xo.iter() {
            if used.contains(k) {
                kept.set(k.clone(), v.clone());
            }
        }
        res.set("XObject", Object::Dictionary(kept));
    }
    res
}

/// Form XObjects and tiling patterns a resource dictionary refers to.
fn child_streams(doc: &Document, res: &Dictionary) -> Vec<ObjectId> {
    let mut out = Vec::new();
    for key in [&b"XObject"[..], b"Pattern"] {
        let Some(d) = res.get(key).ok().and_then(|o| dict_of(doc, o)) else { continue };
        for (_, v) in d.iter() {
            let Ok(id) = v.as_reference() else { continue };
            let Ok(Object::Stream(s)) = doc.get_object(id) else { continue };
            let form = name(&s.dict, b"Subtype") == Some(b"Form");
            let tiling = s.dict.get(b"PatternType").and_then(Object::as_i64).ok() == Some(1);
            if form || tiling {
                out.push(id);
            }
        }
    }
    out
}

/// Appearance streams of an annotation.
fn appearance_streams(doc: &Document, annot: &Dictionary) -> Vec<ObjectId> {
    let mut out = Vec::new();
    let Some(ap) = annot.get(b"AP").ok().and_then(|o| dict_of(doc, o)) else { return out };
    for key in [&b"N"[..], b"R", b"D"] {
        match ap.get(key) {
            Ok(Object::Reference(id)) => match doc.get_object(*id) {
                Ok(Object::Stream(_)) => out.push(*id),
                Ok(Object::Dictionary(states)) => out.extend(states.iter().filter_map(|(_, v)| v.as_reference().ok())),
                _ => {}
            },
            Ok(Object::Dictionary(states)) => out.extend(states.iter().filter_map(|(_, v)| v.as_reference().ok())),
            _ => {}
        }
    }
    out
}

/// Remove hidden optional content everywhere. Returns objects to delete (layer groups, hidden
/// XObjects and annotations) and the number of hidden items removed.
fn strip_layers(doc: &mut Document, layers: &Layers) -> PwResult<(HashSet<ObjectId>, u32)> {
    let mut delete = HashSet::new();
    let mut removed = 0u32;
    // Pages: one combined content stream each.
    let pages: Vec<ObjectId> = doc.get_pages().values().copied().collect();
    let mut queue: Vec<(ObjectId, Option<Dictionary>)> = Vec::new();
    for page in pages {
        let res = page_resources(doc, page);
        let mut data = Vec::new();
        for id in doc.get_page_contents(page) {
            let Ok(Object::Stream(s)) = doc.get_object(id) else { continue };
            data.extend(stream_data(s).ok_or_else(unreadable)?);
            data.push(b'\n');
        }
        let r = rewrite_content(doc, layers, &data, res.as_ref()).map_err(|_| unreadable())?;
        removed += r.removed;
        if let Some(res) = &res {
            queue.extend(child_streams(doc, res).into_iter().map(|id| (id, Some(res.clone()))));
        }
        let annots: Vec<ObjectId> = doc
            .get_dictionary(page)
            .ok()
            .and_then(|p| p.get(b"Annots").ok())
            .map(|a| refs(Some(doc.dereference(a).map(|x| x.1).unwrap_or(a))))
            .unwrap_or_default();
        for a in annots {
            let Ok(ad) = doc.get_dictionary(a) else { continue };
            if ad.get(b"OC").is_ok_and(|oc| !oc_visible(doc, layers, oc, 0)) {
                delete.insert(a);
                removed += 1;
                continue;
            }
            queue.extend(appearance_streams(doc, ad).into_iter().map(|id| (id, None)));
        }
        let new_res = res.as_ref().map(|res| filtered(doc, res, &r.used));
        let stream = doc.add_object(Stream::new(Dictionary::new(), r.data));
        if let Ok(Object::Stream(s)) = doc.get_object_mut(stream) {
            let _ = s.compress();
        }
        let pd = doc.get_dictionary_mut(page).map_err(|_| unreadable())?;
        pd.set("Contents", Object::Reference(stream));
        if let Some(nr) = new_res {
            pd.set("Resources", Object::Dictionary(nr));
        }
    }
    // Form XObjects, tiling patterns and appearance streams, each once.
    let mut seen = HashSet::new();
    while let Some((id, parent_res)) = queue.pop() {
        if !seen.insert(id) || seen.len() > 100_000 {
            continue;
        }
        let (data, own_res, hidden) = {
            let Ok(Object::Stream(s)) = doc.get_object(id) else { continue };
            let own = s.dict.get(b"Resources").ok().and_then(|o| dict_of(doc, o)).cloned();
            let hidden = s.dict.get(b"OC").is_ok_and(|oc| !oc_visible(doc, layers, oc, 0));
            (stream_data(s).ok_or_else(unreadable)?, own, hidden)
        };
        if hidden {
            delete.insert(id);
            removed += 1;
            continue;
        }
        let res = own_res.clone().or(parent_res);
        if let Some(res) = &res {
            queue.extend(child_streams(doc, res).into_iter().map(|c| (c, Some(res.clone()))));
        }
        let r = rewrite_content(doc, layers, &data, res.as_ref()).map_err(|_| unreadable())?;
        removed += r.removed;
        let new_res = own_res.as_ref().map(|res| filtered(doc, res, &r.used));
        if let Ok(Object::Stream(s)) = doc.get_object_mut(id) {
            if r.changed {
                s.set_plain_content(r.data);
                let _ = s.compress();
            }
            if let Some(nr) = new_res {
                s.dict.set("Resources", Object::Dictionary(nr));
            }
        }
    }
    // Every layer group and membership dictionary goes; references to them are scrubbed later.
    for (id, obj) in &doc.objects {
        if let Some(d) = dict_ref(obj) {
            if matches!(name(d, b"Type"), Some(b"OCG") | Some(b"OCMD")) {
                delete.insert(*id);
            }
        }
    }
    // Hidden XObjects that no page draws are still in the file until pruned; delete them outright.
    for (id, obj) in &doc.objects {
        if let Object::Stream(s) = obj {
            if s.dict.get(b"OC").is_ok_and(|oc| !oc_visible(doc, layers, oc, 0)) {
                delete.insert(*id);
            }
        }
    }
    Ok((delete, removed))
}

// ---- survey ---------------------------------------------------------------------------------

/// Earlier saved versions: more than one end-of-file marker, unless the file is linearized (which
/// has two cross-reference sections in one version).
fn has_earlier_versions(raw: &[u8]) -> bool {
    let eofs = raw.windows(5).filter(|w| w == b"%%EOF").count();
    let head = &raw[..raw.len().min(1024)];
    let linearized = head.windows(11).any(|w| w == b"/Linearized");
    eofs > if linearized { 2 } else { 1 }
}

/// Visit every dictionary in the file (top-level objects, nested values and the trailer).
fn each_dict(doc: &Document, mut f: impl FnMut(&Dictionary)) {
    fn walk(o: &Object, f: &mut dyn FnMut(&Dictionary), depth: u32) {
        if depth > MAX_DEPTH {
            return;
        }
        match o {
            Object::Dictionary(d) => {
                f(d);
                d.iter().for_each(|(_, v)| walk(v, f, depth + 1));
            }
            Object::Stream(s) => {
                f(&s.dict);
                s.dict.iter().for_each(|(_, v)| walk(v, f, depth + 1));
            }
            Object::Array(a) => a.iter().for_each(|v| walk(v, f, depth + 1)),
            _ => {}
        }
    }
    for obj in doc.objects.values() {
        walk(obj, &mut f, 0);
    }
    for (_, v) in doc.trailer.iter() {
        walk(v, &mut f, 0);
    }
}

fn survey(doc: &Document, raw: &[u8]) -> SanitizeFindings {
    let mut f = SanitizeFindings { earlier_versions: has_earlier_versions(raw), ..Default::default() };
    if let Some(info) = doc.trailer.get(b"Info").ok().and_then(|o| dict_of(doc, o)) {
        let mut custom = false;
        for (k, _) in info.iter() {
            let label = match k.as_slice() {
                b"Title" => "Title",
                b"Author" => "Author",
                b"Subject" => "Subject",
                b"Keywords" => "Keywords",
                b"Creator" => "Creating application",
                b"Producer" => "PDF producer",
                b"CreationDate" => "Creation date",
                b"ModDate" => "Modification date",
                b"Trapped" => continue,
                _ => {
                    custom = true;
                    continue;
                }
            };
            f.metadata.push(label.into());
        }
        if custom {
            f.metadata.push("Custom properties".into());
        }
    }
    let (mut xmp, mut app_data, mut thumbs, mut xfa) = (false, false, false, false);
    let mut names = Vec::new();
    each_dict(doc, |d| {
        xmp |= name(d, b"Type") == Some(b"Metadata") || d.has(b"Metadata");
        app_data |= d.has(b"PieceInfo");
        thumbs |= d.has(b"Thumb");
        xfa |= d.has(b"XFA");
        match action_class(d) {
            Some(Act::Unsafe | Act::Embedded | Act::Layer) => f.scripts += 1,
            Some(Act::Web) => f.web_links += 1,
            _ => {}
        }
        if d.has(b"EF") {
            f.attachments += 1;
            let n = d.get(b"UF").or_else(|_| d.get(b"F")).map(text).unwrap_or_default();
            if names.len() < MAX_NAMES {
                names.push(n);
            }
        }
        if let Some(st) = annot_subtype(d) {
            if st == b"Widget" {
                f.form_fields += 1;
            } else if COMMENT_TYPES.contains(&st) && st != b"Popup" {
                f.comments += 1;
            } else if MEDIA_TYPES.contains(&st) && st != b"FileAttachment" {
                f.attachments += 1;
            }
        }
    });
    if xmp {
        f.metadata.push("XMP metadata".into());
    }
    if app_data {
        f.metadata.push("Application data".into());
    }
    if thumbs {
        f.metadata.push("Page thumbnails".into());
    }
    if xfa {
        f.scripts += 1;
    }
    f.attachment_names = names;
    let layers = read_layers(doc);
    f.layers = layers.all.len() as u32;
    f.hidden_layers = layers.all.iter().filter(|(id, _)| layers.off.contains(id)).map(|(_, n)| n.clone()).collect();
    f
}

// ---- removal --------------------------------------------------------------------------------

/// Remove keys and inline actions/annotations of the selected kinds from one object, recursively.
fn clean(o: &mut Object, opts: &SanitizeOptions, depth: u32) {
    if depth > MAX_DEPTH {
        return;
    }
    let d = match o {
        Object::Dictionary(d) => d,
        Object::Stream(s) => &mut s.dict,
        Object::Array(a) => {
            a.iter_mut().for_each(|v| clean(v, opts, depth + 1));
            return;
        }
        _ => return,
    };
    let mut drop: Vec<&[u8]> = Vec::new();
    if opts.scripts {
        drop.extend([&b"AA"[..], b"JS", b"XFA", b"NeedsRendering", b"JavaScript"]);
    }
    if opts.metadata {
        drop.extend([&b"Metadata"[..], b"PieceInfo", b"LastModified", b"Thumb"]);
    }
    if opts.attachments {
        drop.extend([&b"EmbeddedFiles"[..], b"AF", b"Collection"]);
    }
    if opts.hidden_layers {
        drop.extend([&b"OCProperties"[..], b"OC"]);
    }
    if opts.form_fields {
        drop.push(b"AcroForm");
    }
    for k in drop {
        d.remove(k);
    }
    let removed_inline = |v: &Object| dict_ref(v).and_then(action_class).is_some_and(|a| action_removed(a, opts));
    for k in [&b"A"[..], b"OpenAction", b"Next"] {
        if d.get(k).is_ok_and(removed_inline) {
            d.remove(k);
        }
    }
    if let Ok(Object::Array(next)) = d.get_mut(b"Next") {
        next.retain(|v| !removed_inline(v));
    }
    if let Ok(Object::Array(annots)) = d.get_mut(b"Annots") {
        annots.retain(|v| !dict_ref(v).and_then(annot_subtype).is_some_and(|st| annot_removed(st, opts)));
    }
    for (_, v) in d.iter_mut() {
        clean(v, opts, depth + 1);
    }
}

/// Whether `o` points at a deleted object (directly, or as an object-reference dictionary).
fn is_dead(o: &Object, dead: &HashSet<ObjectId>) -> bool {
    match o {
        Object::Reference(r) => dead.contains(r),
        Object::Dictionary(d) if name(d, b"Type") == Some(b"OBJR") => {
            d.get(b"Obj").and_then(Object::as_reference).is_ok_and(|r| dead.contains(&r))
        }
        _ => false,
    }
}

/// Drop every reference to a deleted object: dictionary entries and list entries go, other array
/// entries become null.
fn scrub(o: &mut Object, dead: &HashSet<ObjectId>, key: Option<&[u8]>, depth: u32) {
    if depth > MAX_DEPTH {
        return;
    }
    match o {
        Object::Array(a) => {
            if key.is_some_and(|k| LIST_KEYS.contains(&k)) {
                a.retain(|v| !is_dead(v, dead));
            } else {
                for v in a.iter_mut() {
                    if is_dead(v, dead) {
                        *v = Object::Null;
                    }
                }
            }
            a.iter_mut().for_each(|v| scrub(v, dead, None, depth + 1));
        }
        Object::Dictionary(d) => scrub_dict(d, dead, depth),
        Object::Stream(s) => scrub_dict(&mut s.dict, dead, depth),
        _ => {}
    }
}

fn scrub_dict(d: &mut Dictionary, dead: &HashSet<ObjectId>, depth: u32) {
    let gone: Vec<Vec<u8>> = d.iter().filter(|(_, v)| is_dead(v, dead)).map(|(k, _)| k.clone()).collect();
    for k in gone {
        d.remove(&k);
    }
    for (k, v) in d.iter_mut() {
        scrub(v, dead, Some(k.as_slice()), depth + 1);
    }
}

/// Top-level objects of the selected kinds.
fn doomed(doc: &Document, opts: &SanitizeOptions) -> HashSet<ObjectId> {
    let mut dead = HashSet::new();
    for (id, obj) in &doc.objects {
        let Some(d) = dict_ref(obj) else { continue };
        let ty = name(d, b"Type");
        let gone = action_class(d).is_some_and(|a| action_removed(a, opts))
            || annot_subtype(d).is_some_and(|st| annot_removed(st, opts))
            || (opts.metadata && ty == Some(b"Metadata"))
            || (opts.attachments && (ty == Some(b"EmbeddedFile") || d.has(b"EF")))
            || (opts.form_fields && d.has(b"FT"));
        if gone {
            dead.insert(*id);
        }
    }
    if opts.scripts {
        if let Some(acro) = catalog(doc).and_then(|c| c.get(b"AcroForm").ok()).and_then(|o| dict_of(doc, o)) {
            dead.extend(refs(acro.get(b"XFA").ok()));
        }
    }
    dead
}

/// Objects left without purpose by the removal: popups of removed annotations, and links whose
/// action was removed.
fn orphans(doc: &Document, dead: &HashSet<ObjectId>) -> HashSet<ObjectId> {
    let mut out = HashSet::new();
    for (id, obj) in &doc.objects {
        if dead.contains(id) {
            continue;
        }
        let Some(d) = dict_ref(obj) else { continue };
        match annot_subtype(d) {
            Some(b"Popup") if d.get(b"Parent").is_ok_and(|p| is_dead(p, dead)) => {
                out.insert(*id);
            }
            Some(b"Link") => {
                let action = d.get(b"A").is_ok_and(|a| !is_dead(a, dead));
                if !action && !d.has(b"Dest") {
                    out.insert(*id);
                }
            }
            _ => {}
        }
    }
    out
}

/// Apply `opts` to the document in place. Returns the number of hidden layer items removed.
fn apply(doc: &mut Document, opts: &SanitizeOptions) -> PwResult<u32> {
    let mut dead = doomed(doc, opts);
    let mut layer_items = 0;
    if opts.hidden_layers && catalog(doc).is_some_and(|c| c.has(b"OCProperties")) {
        let layers = read_layers(doc);
        let (more, n) = strip_layers(doc, &layers)?;
        dead.extend(more);
        layer_items = n;
    }
    for obj in doc.objects.values_mut() {
        clean(obj, opts, 0);
    }
    loop {
        let extra = orphans(doc, &dead);
        if extra.is_empty() {
            break;
        }
        dead.extend(extra);
    }
    for id in &dead {
        doc.objects.remove(id);
    }
    for obj in doc.objects.values_mut() {
        scrub(obj, &dead, None, 0);
    }
    let mut trailer = std::mem::take(&mut doc.trailer);
    scrub_dict(&mut trailer, &dead, 0);
    for k in [&b"Prev"[..], b"XRefStm"] {
        trailer.remove(k);
    }
    if opts.metadata {
        trailer.remove(b"Info");
    }
    doc.trailer = trailer;
    doc.prune_objects();
    Ok(layer_items)
}

// ---- verification ---------------------------------------------------------------------------

/// Independent whole-file check of the written copy: one version, no encryption, and none of the
/// removed kinds anywhere — keys, objects, actions, annotations, and optional-content markers in any
/// content stream.
fn check_file(bytes: &[u8], opts: &SanitizeOptions) -> PwResult<()> {
    let doc = Document::load_mem(bytes).map_err(|_| fail("it could not be re-read"))?;
    if doc.trailer.has(b"Prev") || doc.trailer.has(b"Encrypt") || has_earlier_versions(bytes) {
        return Err(fail("an earlier version remains"));
    }
    if opts.metadata && doc.trailer.has(b"Info") {
        return Err(fail("document properties remain"));
    }
    let mut problem: Option<&'static str> = None;
    each_dict(&doc, |d| {
        let ty = name(d, b"Type");
        let has = |k: &[u8]| d.has(k);
        let act = action_class(d);
        let st = annot_subtype(d);
        let p = if opts.metadata && (has(b"Metadata") || has(b"PieceInfo") || has(b"Thumb") || ty == Some(b"Metadata"))
        {
            Some("metadata remains")
        } else if opts.scripts && (has(b"AA") || has(b"JS") || has(b"XFA") || has(b"JavaScript")) {
            Some("a script remains")
        } else if act.is_some_and(|a| action_removed(a, opts)) {
            Some("an action remains")
        } else if opts.attachments
            && (has(b"EF") || has(b"EmbeddedFiles") || has(b"AF") || has(b"Collection") || ty == Some(b"EmbeddedFile"))
        {
            Some("an attachment remains")
        } else if st.is_some_and(|s| annot_removed(s, opts)) {
            Some("a removed annotation remains")
        } else if opts.form_fields && (has(b"AcroForm") || has(b"FT")) {
            Some("a form field remains")
        } else if opts.hidden_layers && (has(b"OCProperties") || has(b"OC") || matches!(ty, Some(b"OCG" | b"OCMD"))) {
            Some("a layer remains")
        } else {
            None
        };
        problem = problem.or(p);
    });
    if let Some(p) = problem {
        return Err(fail(p));
    }
    if opts.hidden_layers {
        // Every content-like stream, decoded by lopdf's own parser: no optional-content marker.
        for obj in doc.objects.values() {
            let Object::Stream(s) = obj else { continue };
            let sub = name(&s.dict, b"Subtype");
            let content_like = matches!(sub, None | Some(b"Form"))
                && matches!(name(&s.dict, b"Type"), None | Some(b"XObject") | Some(b"Pattern"))
                && !s.dict.has(b"Length1")
                && !s.dict.has(b"N");
            if !content_like {
                continue;
            }
            let Some(data) = stream_data(s) else { continue };
            let Ok(content) = lopdf::content::Content::decode(&data) else { continue };
            let oc = content
                .operations
                .iter()
                .any(|op| op.operator == "BDC" && op.operands.first().and_then(|o| o.as_name().ok()) == Some(b"OC"));
            if oc {
                return Err(fail("layered content remains"));
            }
        }
    }
    Ok(())
}

fn render(page: &PdfPage) -> PwResult<RgbaImage> {
    let longest = page.width().value.max(page.height().value).max(1.0);
    let config = PdfRenderConfig::new()
        .scale_page_by_factor((CHECK_PIXELS / longest).min(1.0))
        .render_form_data(false)
        .render_annotations(false);
    let bitmap = page.render_with_config(&config).map_err(|e| map_pdfium_error(&e, "render"))?;
    Ok(bitmap.as_image().map_err(|e| map_pdfium_error(&e, "render"))?.to_rgba8())
}

/// Every page of the copy looks exactly like the original's page (annotations not drawn).
fn check_pages(
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
    for i in 0..n {
        if cancelled() {
            return Err(PwError::new(ErrorCategory::Cancelled, "job.cancelled", "Cleaning was cancelled."));
        }
        let a = render(&source.pages().get(i).map_err(|_| fail("page unreadable"))?)?;
        let b = render(&out.pages().get(i).map_err(|_| fail("page unreadable"))?)?;
        let same = a.dimensions() == b.dimensions()
            && a.pixels().zip(b.pixels()).all(|(p, q)| (0..3).all(|c| p.0[c].abs_diff(q.0[c]) <= MAX_CHANNEL_DELTA));
        if !same {
            return Err(fail(&format!("page {} no longer looks the same", i + 1)));
        }
        progress(i as u64 + 1, n as u64);
    }
    Ok(())
}

// ---- entry points ---------------------------------------------------------------------------

/// The source as a lopdf document (normalised by PDFium when lopdf cannot parse it) and its bytes.
fn load_plain<'p>(
    pdfium: &'p Pdfium,
    input: &Path,
    opts: &OpenOptions,
) -> PwResult<(PdfDocument<'p>, Document, Vec<u8>)> {
    let src = load(pdfium, input, opts)?;
    if !matches!(src.permissions().security_handler_revision(), Ok(PdfSecurityHandlerRevision::Unprotected)) {
        return Err(protected());
    }
    let raw = std::fs::read(input).map_err(|e| PwError::from_io(&e, "file"))?;
    let doc = match Document::load_mem(&raw) {
        Ok(d) => d,
        Err(_) => Document::load_mem(&save_bytes(&src)?).map_err(|_| {
            PwError::new(ErrorCategory::Corrupt, "sanitize.parse", "This file could not be prepared for cleaning.")
        })?,
    };
    if doc.was_encrypted() || doc.is_encrypted() {
        return Err(protected());
    }
    Ok((src, doc, raw))
}

/// What hidden information the file holds (read-only).
pub fn sanitize_inspect(input: &Path, opts: &OpenOptions) -> PwResult<SanitizeFindings> {
    let pdfium = engine()?;
    let (_src, doc, raw) = load_plain(&pdfium, input, opts)?;
    Ok(survey(&doc, &raw))
}

/// Write a verified copy without the kinds of hidden information `options` selects. Returns the
/// committed file, its page count, what was removed, and warnings.
#[allow(clippy::too_many_arguments)]
pub fn sanitize(
    input: &Path,
    opts: &OpenOptions,
    options: &SanitizeOptions,
    output: &Path,
    policy: CollisionPolicy,
    cancelled: &dyn Fn() -> bool,
    progress: &dyn Fn(u64, u64),
) -> PwResult<(CommittedOutput, u32, SanitizeFindings, Vec<String>)> {
    let pdfium = engine()?;
    let (source, mut doc, raw) = load_plain(&pdfium, input, opts)?;
    let found = survey(&doc, &raw);
    let pdfa = options.metadata
        && doc.objects.values().any(|o| match o {
            Object::Stream(s) if name(&s.dict, b"Type") == Some(b"Metadata") => {
                stream_data(s).is_some_and(|d| d.windows(7).any(|w| w == b"pdfaid:"))
            }
            _ => false,
        });
    let field_actions = options.scripts && !options.form_fields && {
        let mut any = false;
        each_dict(&doc, |d| {
            let field = d.has(b"FT") || annot_subtype(d) == Some(b"Widget");
            let unsafe_action = |k: &[u8]| {
                d.get(k)
                    .ok()
                    .and_then(|a| dict_of(&doc, a))
                    .and_then(action_class)
                    .is_some_and(|a| a != Act::Navigation)
            };
            any |= field && (d.has(b"AA") || unsafe_action(b"A"));
        });
        any
    };
    let layer_items = apply(&mut doc, options)?;
    let mut bytes = Vec::new();
    doc.save_to(&mut bytes)
        .map_err(|_| PwError::internal("sanitize.save", "The cleaned copy could not be written."))?;
    check_file(&bytes, options)?;
    check_pages(&pdfium, &source, &bytes, cancelled, progress)?;
    let count = source.pages().len().max(0) as u32;
    let protected_sources = [input.to_path_buf()];
    let c = write_atomic_bytes(output, policy, &protected_sources, &bytes, |staged| {
        let back = std::fs::read(staged).map_err(|e| PwError::from_io(&e, "output file"))?;
        if back != bytes {
            return Err(fail("the written file differs"));
        }
        check_file(&back, options)
    })?;
    let mut warnings = Vec::new();
    if pdfa {
        warnings.push("The copy no longer declares PDF/A conformance, because that is stated in its metadata.".into());
    }
    if options.hidden_layers && found.layers > 0 {
        warnings.push(if layer_items > 0 {
            "Content in hidden layers was removed; the visible layers are now ordinary page content.".to_string()
        } else {
            "The layers are now ordinary page content.".to_string()
        });
    }
    if field_actions {
        warnings.push("Form fields stay, but buttons that ran scripts or submitted data no longer do anything.".into());
    }
    Ok((c, count, found.selected(options), warnings))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dict(pairs: Vec<(&str, Object)>) -> Dictionary {
        let mut d = Dictionary::new();
        for (k, v) in pairs {
            d.set(k, v);
        }
        d
    }

    #[test]
    fn actions_are_classified_by_type() {
        let js = dict(vec![("S", Object::Name(b"JavaScript".to_vec())), ("JS", Object::string_literal("x"))]);
        assert_eq!(action_class(&js), Some(Act::Unsafe));
        let next = dict(vec![("S", Object::Name(b"Named".to_vec())), ("N", Object::Name(b"NextPage".to_vec()))]);
        assert_eq!(action_class(&next), Some(Act::Navigation));
        let print = dict(vec![("S", Object::Name(b"Named".to_vec())), ("N", Object::Name(b"Print".to_vec()))]);
        assert_eq!(action_class(&print), Some(Act::Unsafe));
        // Not actions: a border style, a structure element, a transition.
        let border = dict(vec![("S", Object::Name(b"D".to_vec())), ("W", Object::Integer(1))]);
        assert_eq!(action_class(&border), None);
        let elem = dict(vec![("S", Object::Name(b"Sound".to_vec())), ("K", Object::Integer(0))]);
        assert_eq!(action_class(&elem), None);
        let trans = dict(vec![("Type", Object::Name(b"Trans".to_vec())), ("S", Object::Name(b"Launch".to_vec()))]);
        assert_eq!(action_class(&trans), None);
        let o = SanitizeOptions { scripts: false, ..SanitizeOptions::default() };
        assert!(!action_removed(Act::Unsafe, &o));
        assert!(action_removed(Act::Embedded, &o), "embedded-file actions go with the attachments");
        assert!(!action_removed(Act::Web, &SanitizeOptions::default()));
    }

    #[test]
    fn layer_membership_follows_the_policy() {
        let mut doc = Document::with_version("1.7");
        let on = doc.add_object(dict(vec![("Type", Object::Name(b"OCG".to_vec()))]));
        let off = doc.add_object(dict(vec![("Type", Object::Name(b"OCG".to_vec()))]));
        let layers = Layers { off: HashSet::from([off]), all: vec![] };
        let md = |p: &str| {
            Object::Dictionary(dict(vec![
                ("Type", Object::Name(b"OCMD".to_vec())),
                ("OCGs", Object::Array(vec![Object::Reference(on), Object::Reference(off)])),
                ("P", Object::Name(p.as_bytes().to_vec())),
            ]))
        };
        assert!(oc_visible(&doc, &layers, &Object::Reference(on), 0));
        assert!(!oc_visible(&doc, &layers, &Object::Reference(off), 0));
        assert!(oc_visible(&doc, &layers, &md("AnyOn"), 0));
        assert!(!oc_visible(&doc, &layers, &md("AllOn"), 0));
        assert!(oc_visible(&doc, &layers, &md("AnyOff"), 0));
        assert!(!oc_visible(&doc, &layers, &md("AllOff"), 0));
        let ve = Object::Dictionary(dict(vec![
            ("Type", Object::Name(b"OCMD".to_vec())),
            (
                "VE",
                Object::Array(vec![
                    Object::Name(b"And".to_vec()),
                    Object::Reference(on),
                    Object::Array(vec![Object::Name(b"Not".to_vec()), Object::Reference(off)]),
                ]),
            ),
        ]));
        assert!(oc_visible(&doc, &layers, &ve, 0));
    }

    #[test]
    fn hidden_content_is_cut_and_visible_markers_kept() {
        let mut doc = Document::with_version("1.7");
        let hidden = doc.add_object(dict(vec![("Type", Object::Name(b"OCG".to_vec()))]));
        let shown = doc.add_object(dict(vec![("Type", Object::Name(b"OCG".to_vec()))]));
        let layers = Layers { off: HashSet::from([hidden]), all: vec![] };
        let res = dict(vec![(
            "Properties",
            Object::Dictionary(dict(vec![("H", Object::Reference(hidden)), ("V", Object::Reference(shown))])),
        )]);
        let src = b"q 0 0 1 rg /OC /H BDC BT /F1 9 Tf 1 2 Td (SECRET) Tj (a) ' 1 2 (b) \" ET 0 0 9 9 re f /Im1 Do EMC \
                    /OC /V BDC BT (shown) Tj ET EMC Q";
        let r = rewrite_content(&doc, &layers, src, Some(&res)).expect("ok");
        let out = String::from_utf8_lossy(&r.data).to_string();
        assert!(
            !out.contains("SECRET") && !out.contains("(a)") && !out.contains("(b)") && !out.contains("Im1"),
            "{out}"
        );
        assert!(out.contains("0 0 9 9 re  n") && out.contains("1 Tw 2 Tc T*") && out.contains("0 0 1 rg"), "{out}");
        assert!(out.contains("/OC BMC") && out.contains("(shown) Tj") && !out.contains("/OC /V"), "{out}");
        assert_eq!(out.matches("EMC").count(), 1, "the hidden section's EMC goes too: {out}");
        assert_eq!(r.removed, 1);
        assert!(r.used.is_empty());
    }

    fn fixture(n: &str) -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/pdf").join(n)
    }

    const NONE: SanitizeOptions = SanitizeOptions {
        metadata: false,
        scripts: false,
        attachments: false,
        hidden_layers: false,
        comments: false,
        form_fields: false,
        web_links: false,
    };

    /// Each category's check catches leftovers of exactly that category: a copy cleaned of
    /// everything else must fail the check for the one category left in.
    #[test]
    fn file_check_catches_each_category_left_behind() {
        let raw = std::fs::read(fixture("hidden-info.pdf")).expect("read");
        let only = |f: fn(&mut SanitizeOptions)| {
            let mut o = NONE;
            f(&mut o);
            o
        };
        type Pick = fn(&mut SanitizeOptions);
        let cases: [(&str, Pick); 7] = [
            ("metadata", |o| o.metadata = true),
            ("script", |o| o.scripts = true),
            ("attachment", |o| o.attachments = true),
            ("layer", |o| o.hidden_layers = true),
            ("annotation", |o| o.comments = true),
            ("form field", |o| o.form_fields = true),
            ("action", |o| o.web_links = true),
        ];
        for (what, set) in cases {
            let target = only(set);
            let all = SanitizeOptions {
                metadata: true,
                scripts: true,
                attachments: true,
                hidden_layers: true,
                comments: true,
                form_fields: true,
                web_links: true,
            };
            // Everything removed except the target category.
            let mut rest = all;
            rest.metadata &= !target.metadata;
            rest.scripts &= !target.scripts;
            rest.attachments &= !target.attachments;
            rest.hidden_layers &= !target.hidden_layers;
            rest.comments &= !target.comments;
            rest.form_fields &= !target.form_fields;
            rest.web_links &= !target.web_links;
            if target.web_links || target.form_fields {
                // Links and fields also carry scripts; keep those categories consistent.
                rest.scripts = false;
            }
            let mut doc = Document::load_mem(&raw).expect("lopdf");
            apply(&mut doc, &rest).expect("apply");
            let mut bytes = Vec::new();
            doc.save_to(&mut bytes).expect("save");
            check_file(&bytes, &rest).unwrap_or_else(|e| panic!("{what}: the rest must pass: {e:?}"));
            let err = check_file(&bytes, &target).expect_err(what);
            assert!(err.message.contains("remain"), "{what}: {}", err.message);
        }
        // The untouched file fails for its earlier version alone.
        let err = check_file(&raw, &NONE).expect_err("earlier version");
        assert!(err.message.contains("earlier version"), "{}", err.message);
    }

    /// The render check sees content change, and PDFium honours hidden layers: making the hidden
    /// layer visible (instead of removing it) is caught.
    #[test]
    fn page_check_catches_changed_and_revealed_content() {
        let pdfium = engine().expect("engine");
        let path = fixture("hidden-info.pdf");
        let source = load(&pdfium, &path, &OpenOptions { password: None }).expect("load");
        let raw = std::fs::read(&path).expect("read");
        let mut doc = Document::load_mem(&raw).expect("lopdf");
        let root = doc.trailer.get(b"Root").and_then(Object::as_reference).expect("root");
        doc.get_dictionary_mut(root).expect("catalog").remove(b"OCProperties");
        let mut revealed = Vec::new();
        doc.save_to(&mut revealed).expect("save");
        let err = check_pages(&pdfium, &source, &revealed, &|| false, &|_, _| {}).expect_err("revealed layer");
        assert!(err.message.contains("page 1 no longer looks the same"), "{}", err.message);

        let mut doc = Document::load_mem(&raw).expect("lopdf");
        let p2 = *doc.get_pages().get(&2).expect("page 2");
        doc.change_page_content(p2, b"BT ET".to_vec()).expect("change");
        let mut changed = Vec::new();
        doc.save_to(&mut changed).expect("save");
        let err = check_pages(&pdfium, &source, &changed, &|| false, &|_, _| {}).expect_err("changed page");
        assert!(err.message.contains("page 2"), "{}", err.message);
        let err = check_pages(&pdfium, &source, &raw, &|| true, &|_, _| {}).expect_err("cancelled");
        assert_eq!(err.category, ErrorCategory::Cancelled);
    }

    #[test]
    fn earlier_versions_are_detected() {
        assert!(!has_earlier_versions(b"%PDF-1.7\n...%%EOF\n"));
        assert!(has_earlier_versions(b"%PDF-1.7\n...%%EOF\n...%%EOF\n"));
        assert!(!has_earlier_versions(b"%PDF-1.7\n1 0 obj << /Linearized 1 >> ...%%EOF\n...%%EOF\n"));
    }
}
