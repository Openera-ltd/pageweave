//! Default values of form fields (`/DV`, inherited down the field tree), used by "Reset form".
//! Read with lopdf from the file itself because PDFium does not expose them. Read-only and
//! bounded: a damaged or hostile tree yields fewer defaults, never an error.

use lopdf::{Dictionary, Document, Object, ObjectId};
use std::collections::{HashMap, HashSet};
use std::path::Path;

/// Deepest field tree walked, and most fields read (hostile files).
const MAX_DEPTH: usize = 32;
const MAX_FIELDS: usize = 20_000;
const MAX_DECOMPRESSED: usize = 256 * 1024 * 1024;

fn resolve<'a>(doc: &'a Document, o: &'a Object) -> Option<&'a Object> {
    match o {
        Object::Reference(id) => doc.get_object(*id).ok(),
        other => Some(other),
    }
}

fn dict<'a>(doc: &'a Document, o: &'a Object) -> Option<&'a Dictionary> {
    resolve(doc, o).and_then(|o| o.as_dict().ok())
}

/// A `/DV` as the value PageWeave shows: text for text and choice fields, the state name for
/// buttons, the first entry of a multi-selection.
fn value_text(doc: &Document, o: &Object) -> Option<String> {
    let o = resolve(doc, o)?;
    match o {
        Object::Name(n) => Some(String::from_utf8_lossy(n).into_owned()),
        Object::String(..) => lopdf::decode_text_string(o).ok(),
        Object::Array(a) => a.first().and_then(|f| value_text(doc, f)),
        _ => None,
    }
    .map(|v| v.replace("\r\n", "\n").replace('\r', "\n"))
}

struct Walk<'a> {
    doc: &'a Document,
    seen: HashSet<ObjectId>,
    out: HashMap<String, String>,
    count: usize,
}

impl Walk<'_> {
    fn field(&mut self, node: &Object, parent_name: &str, inherited: Option<String>, depth: usize) {
        if depth > MAX_DEPTH || self.count >= MAX_FIELDS {
            return;
        }
        if let Object::Reference(id) = node {
            if !self.seen.insert(*id) {
                return;
            }
        }
        let Some(d) = dict(self.doc, node) else { return };
        self.count += 1;
        let partial =
            d.get(b"T").ok().and_then(|t| resolve(self.doc, t)).and_then(|t| lopdf::decode_text_string(t).ok());
        let name = match (&partial, parent_name.is_empty()) {
            (Some(p), true) => p.clone(),
            (Some(p), false) => format!("{parent_name}.{p}"),
            (None, _) => parent_name.to_string(),
        };
        let dv = d.get(b"DV").ok().and_then(|v| value_text(self.doc, v)).or(inherited);
        let kids: Vec<Object> = d
            .get(b"Kids")
            .ok()
            .and_then(|k| resolve(self.doc, k))
            .and_then(|k| k.as_array().ok())
            .cloned()
            .unwrap_or_default();
        // A kid with its own /T is a child field; one without is a widget of this field.
        let child_fields: Vec<&Object> =
            kids.iter().filter(|k| dict(self.doc, k).is_some_and(|kd| kd.has(b"T"))).collect();
        if child_fields.is_empty() {
            if let (false, Some(v)) = (name.is_empty(), dv) {
                self.out.entry(name).or_insert(v);
            }
            return;
        }
        for k in child_fields {
            self.field(k, &name, dv.clone(), depth + 1);
        }
    }
}

/// Default value per fully qualified field name, for fields that have one.
pub fn field_defaults_from(doc: &Document) -> HashMap<String, String> {
    let mut w = Walk { doc, seen: HashSet::new(), out: HashMap::new(), count: 0 };
    let fields = doc
        .catalog()
        .ok()
        .and_then(|c| c.get(b"AcroForm").ok())
        .and_then(|a| dict(doc, a))
        .and_then(|a| a.get(b"Fields").ok())
        .and_then(|f| resolve(doc, f))
        .and_then(|f| f.as_array().ok())
        .cloned()
        .unwrap_or_default();
    for f in &fields {
        w.field(f, "", None, 0);
    }
    w.out
}

/// Defaults of the file at `path` (empty when it cannot be read, e.g. a wrong password).
pub fn field_defaults(path: &Path, password: Option<&str>) -> HashMap<String, String> {
    let Ok(raw) = std::fs::read(path) else { return HashMap::new() };
    // Bound stream decompression (hostile files); the field tree itself is small.
    let opts = || {
        let mut o = match password {
            Some(p) => lopdf::LoadOptions::with_password(p),
            None => lopdf::LoadOptions::with_max_decompressed_size(MAX_DECOMPRESSED),
        };
        o.max_decompressed_size = Some(MAX_DECOMPRESSED);
        o
    };
    let doc = Document::load_mem_with_options(&raw, opts()).or_else(|_| {
        let mut o = opts();
        o.password = None;
        Document::load_mem_with_options(&raw, o)
    });
    match doc {
        Ok(d) if !d.is_encrypted() => field_defaults_from(&d),
        _ => HashMap::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lopdf::dictionary;

    #[test]
    fn defaults_are_named_by_full_name_and_inherited() {
        let mut doc = Document::with_version("1.7");
        let widget = doc.add_object(dictionary! { "Type" => "Annot", "Subtype" => "Widget" });
        let city = doc
            .add_object(dictionary! { "T" => Object::string_literal("city"), "DV" => Object::string_literal("Oslo") });
        let zip = doc.add_object(dictionary! { "T" => Object::string_literal("zip") });
        let addr = doc.add_object(dictionary! {
            "T" => Object::string_literal("address"), "DV" => Object::string_literal("inherited"),
            "Kids" => vec![city.into(), zip.into()],
        });
        let agree = doc.add_object(
            dictionary! { "T" => Object::string_literal("agree"), "DV" => "Yes", "Kids" => vec![widget.into()] },
        );
        let pick = doc.add_object(dictionary! {
            "T" => Object::string_literal("pick"),
            "DV" => vec![Object::string_literal("b"), Object::string_literal("c")],
        });
        let none = doc.add_object(dictionary! { "T" => Object::string_literal("none") });
        let form = doc.add_object(
            dictionary! { "Fields" => vec![addr.into(), agree.into(), pick.into(), none.into(), addr.into()] },
        );
        let cat = doc.add_object(dictionary! { "Type" => "Catalog", "AcroForm" => form });
        doc.trailer.set("Root", cat);
        let d = field_defaults_from(&doc);
        assert_eq!(d.get("address.city").map(String::as_str), Some("Oslo"));
        assert_eq!(d.get("address.zip").map(String::as_str), Some("inherited"));
        assert_eq!(d.get("agree").map(String::as_str), Some("Yes"));
        assert_eq!(d.get("pick").map(String::as_str), Some("b"));
        assert!(!d.contains_key("none"));
        assert_eq!(d.len(), 4);
    }

    #[test]
    fn a_cyclic_tree_ends() {
        let mut doc = Document::with_version("1.7");
        let a_id = doc.new_object_id();
        let b = doc.add_object(dictionary! { "T" => Object::string_literal("b"), "Kids" => vec![a_id.into()] });
        doc.objects.insert(a_id, Object::Dictionary(dictionary! { "T" => Object::string_literal("a"), "DV" => Object::string_literal("x"), "Kids" => vec![b.into()] }));
        let form = doc.add_object(dictionary! { "Fields" => vec![a_id.into()] });
        let cat = doc.add_object(dictionary! { "Type" => "Catalog", "AcroForm" => form });
        doc.trailer.set("Root", cat);
        // b's kid is a again: the walk stops there instead of looping.
        let d = field_defaults_from(&doc);
        assert!(d.keys().all(|k| k.len() < 8), "{d:?}");
    }
}
