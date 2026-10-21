//! Form data files: writing and reading XFDF and PageWeave's JSON form data.
//!
//! Engine-independent. Reading treats the file as untrusted input: it is size-, count- and
//! depth-limited, XML document type declarations are refused (so no external or custom entities
//! are ever resolved), and only the five predefined XML entities and character references are
//! accepted. Matching values to a document's fields happens in the PDF engine, which knows the form.

use crate::{PwError, PwResult};
use quick_xml::events::Event;
use quick_xml::{Reader, XmlVersion};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

/// Largest form data file read (bytes).
pub const MAX_FORM_DATA_BYTES: u64 = 8 * 1024 * 1024;
/// Most field entries read from one file.
pub const MAX_FORM_DATA_ENTRIES: usize = 20_000;
/// Deepest `<field>` nesting accepted in XFDF.
const MAX_XFDF_DEPTH: usize = 64;
/// Longest field name accepted (characters).
const MAX_NAME_CHARS: usize = 2_000;
/// Identifies PageWeave's JSON form data.
pub const JSON_FORMAT_ID: &str = "pageweave-form-data";
const XFDF_NS: &str = "http://ns.adobe.com/xfdf/";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FormDataFormat {
    /// XML Forms Data Format (ISO 19444-1), read and written by most PDF applications.
    Xfdf,
    /// PageWeave's JSON form data (also accepts a plain `{"name": value}` object).
    Json,
}

impl FormDataFormat {
    pub fn extension(self) -> &'static str {
        match self {
            FormDataFormat::Xfdf => "xfdf",
            FormDataFormat::Json => "json",
        }
    }
}

/// A value read from a form data file, before it is matched to a field.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type", content = "value")]
pub enum DataValue {
    Text(String),
    /// JSON true/false (checkboxes).
    Bool(bool),
    /// Several values for one field (multi-select lists).
    Several(Vec<String>),
    /// No usable value: JSON null, or an XFDF field holding only rich text.
    Missing,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DataEntry {
    /// Fully qualified field name (`address.city`).
    pub name: String,
    pub value: DataValue,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParsedFormData {
    pub format: FormDataFormat,
    pub entries: Vec<DataEntry>,
}

/// A field's value as written to a form data file.
#[derive(Debug, Clone, PartialEq)]
pub enum ExportValue {
    Text(String),
    /// `on` is the export value used for the checked state in XFDF.
    Checkbox {
        checked: bool,
        on: String,
    },
    /// The selected option's export value; `None` when nothing is selected.
    Radio(Option<String>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExportEntry {
    pub name: String,
    /// Field kind for readers of the JSON file (`text`, `checkbox`, …); not used on import.
    pub kind: &'static str,
    pub value: ExportValue,
}

impl ExportEntry {
    /// The value an import of the written file must read back, per format.
    pub fn expected(&self, format: FormDataFormat) -> DataValue {
        match (&self.value, format) {
            (ExportValue::Text(t), _) => DataValue::Text(xml_safe(t).0),
            (ExportValue::Checkbox { checked, .. }, FormDataFormat::Json) => DataValue::Bool(*checked),
            (ExportValue::Checkbox { checked, on }, FormDataFormat::Xfdf) => {
                DataValue::Text(if *checked { xml_safe(on).0 } else { "Off".into() })
            }
            (ExportValue::Radio(v), FormDataFormat::Json) => match v {
                Some(s) => DataValue::Text(s.clone()),
                None => DataValue::Missing,
            },
            (ExportValue::Radio(v), FormDataFormat::Xfdf) => {
                DataValue::Text(v.as_deref().map(|s| xml_safe(s).0).unwrap_or_else(|| "Off".into()))
            }
        }
    }
}

/// Serialized file plus notes about anything that could not be written exactly.
#[derive(Debug, Clone, PartialEq)]
pub struct WrittenFormData {
    pub bytes: Vec<u8>,
    pub warnings: Vec<String>,
}

fn invalid(code: &str, msg: impl Into<String>) -> PwError {
    PwError::invalid(code, msg)
}

/// Whether XML 1.0 can hold the character (production `Char`).
fn xml_char(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\r' | '\u{20}'..='\u{D7FF}' | '\u{E000}'..='\u{FFFD}' | '\u{10000}'..)
}

/// The text with characters XML cannot hold removed, and how many were removed.
fn xml_safe(s: &str) -> (String, usize) {
    let kept: String = s.chars().filter(|c| xml_char(*c)).collect();
    let removed = s.chars().count() - kept.chars().count();
    (kept, removed)
}

fn escape_xml(s: &str, out: &mut String) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            // Carriage returns would be normalised away by any XML reader.
            '\r' => out.push_str("&#13;"),
            c => out.push(c),
        }
    }
}

/// Write entries in the chosen format. `source` is the PDF's file name (XFDF `<f href>`, JSON `source`).
pub fn write_form_data(format: FormDataFormat, entries: &[ExportEntry], source: Option<&str>) -> WrittenFormData {
    match format {
        FormDataFormat::Json => write_json(entries, source),
        FormDataFormat::Xfdf => write_xfdf(entries, source),
    }
}

fn write_json(entries: &[ExportEntry], source: Option<&str>) -> WrittenFormData {
    let fields: Vec<Value> = entries
        .iter()
        .map(|e| {
            let value = match &e.value {
                ExportValue::Text(t) => json!(t),
                ExportValue::Checkbox { checked, .. } => json!(checked),
                ExportValue::Radio(v) => json!(v),
            };
            json!({ "name": e.name, "kind": e.kind, "value": value })
        })
        .collect();
    let mut doc = Map::new();
    doc.insert("format".into(), json!(JSON_FORMAT_ID));
    doc.insert("version".into(), json!(1));
    if let Some(s) = source {
        doc.insert("source".into(), json!(s));
    }
    doc.insert("fields".into(), Value::Array(fields));
    let mut bytes = serde_json::to_vec_pretty(&Value::Object(doc)).unwrap_or_default();
    bytes.push(b'\n');
    WrittenFormData { bytes, warnings: vec![] }
}

/// A tree of field name parts, keeping first-seen order.
#[derive(Default)]
struct Node {
    children: Vec<(String, Node)>,
    value: Option<String>,
}

impl Node {
    fn child(&mut self, part: &str) -> &mut Node {
        let i = match self.children.iter().position(|(n, _)| n == part) {
            Some(i) => i,
            None => {
                self.children.push((part.to_string(), Node::default()));
                self.children.len() - 1
            }
        };
        &mut self.children[i].1
    }

    fn write(&self, indent: usize, out: &mut String) {
        for (name, node) in &self.children {
            let pad = "  ".repeat(indent);
            out.push_str(&pad);
            out.push_str("<field name=\"");
            escape_xml(name, out);
            out.push_str("\">");
            if let Some(v) = &node.value {
                out.push_str("<value>");
                escape_xml(v, out);
                out.push_str("</value>");
            }
            if !node.children.is_empty() {
                out.push('\n');
                node.write(indent + 1, out);
                out.push_str(&pad);
            }
            out.push_str("</field>\n");
        }
    }
}

fn write_xfdf(entries: &[ExportEntry], source: Option<&str>) -> WrittenFormData {
    let mut root = Node::default();
    let mut warnings = Vec::new();
    for e in entries {
        let text = match &e.value {
            ExportValue::Text(t) => t.clone(),
            ExportValue::Checkbox { checked: true, on } => on.clone(),
            ExportValue::Checkbox { checked: false, .. } => "Off".into(),
            ExportValue::Radio(v) => v.clone().unwrap_or_else(|| "Off".into()),
        };
        let (safe, removed) = xml_safe(&text);
        if removed > 0 {
            warnings.push(format!("“{}”: {removed} control character(s) that XFDF cannot hold were left out.", e.name));
        }
        let (name, _) = xml_safe(&e.name);
        let mut node = &mut root;
        for part in name.split('.') {
            node = node.child(part);
        }
        node.value = Some(safe);
    }
    let mut out = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    out.push_str("<xfdf xmlns=\"http://ns.adobe.com/xfdf/\" xml:space=\"preserve\">\n");
    if let Some(s) = source {
        out.push_str("  <f href=\"");
        escape_xml(&xml_safe(s).0, &mut out);
        out.push_str("\"/>\n");
    }
    out.push_str("  <fields>\n");
    root.write(2, &mut out);
    out.push_str("  </fields>\n</xfdf>\n");
    WrittenFormData { bytes: out.into_bytes(), warnings }
}

/// Read a form data file's bytes (XFDF or JSON, detected from the content).
pub fn parse_form_data(bytes: &[u8]) -> PwResult<ParsedFormData> {
    if bytes.len() as u64 > MAX_FORM_DATA_BYTES {
        return Err(invalid("form_data.too_large", "The form data file is too large (over 8 MB)."));
    }
    let text = std::str::from_utf8(bytes).map_err(|_| {
        invalid("form_data.encoding", "The form data file is not UTF-8 text. Save it as UTF-8 and try again.")
    })?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let trimmed = text.trim_start();
    let entries = if trimmed.starts_with('{') {
        (FormDataFormat::Json, parse_json(trimmed)?)
    } else if trimmed.starts_with('<') {
        (FormDataFormat::Xfdf, parse_xfdf(trimmed)?)
    } else if trimmed.starts_with("%FDF") {
        return Err(invalid(
            "form_data.fdf",
            "FDF files can’t be read yet. Export the data as XFDF from the other application instead.",
        ));
    } else {
        return Err(invalid("form_data.format", "This isn’t an XFDF or JSON form data file."));
    };
    let (format, entries) = entries;
    if entries.len() > MAX_FORM_DATA_ENTRIES {
        return Err(invalid("form_data.too_many", "The form data file has too many fields."));
    }
    for e in &entries {
        if e.name.trim().is_empty() {
            return Err(invalid("form_data.no_name", "A field in the form data file has no name."));
        }
        if e.name.chars().count() > MAX_NAME_CHARS {
            return Err(invalid("form_data.long_name", "A field name in the form data file is too long."));
        }
    }
    Ok(ParsedFormData { format, entries })
}

fn json_value(v: &Value) -> Option<DataValue> {
    Some(match v {
        Value::String(s) => DataValue::Text(s.clone()),
        Value::Bool(b) => DataValue::Bool(*b),
        // A number keeps its written form ("42", "3.5").
        Value::Number(n) => DataValue::Text(n.to_string()),
        Value::Null => DataValue::Missing,
        Value::Array(items) => {
            let mut out = Vec::with_capacity(items.len());
            for i in items {
                match i {
                    Value::String(s) => out.push(s.clone()),
                    Value::Number(n) => out.push(n.to_string()),
                    _ => return None,
                }
            }
            DataValue::Several(out)
        }
        Value::Object(_) => return None,
    })
}

fn parse_json(text: &str) -> PwResult<Vec<DataEntry>> {
    let bad = || invalid("form_data.json", "The JSON form data file could not be read (it is not valid JSON).");
    let doc: Value = serde_json::from_str(text).map_err(|_| bad())?;
    let Value::Object(obj) = doc else { return Err(bad()) };
    let unusable = |name: &str| {
        invalid("form_data.json_value", format!("The value for “{name}” must be text, a number, true/false or a list."))
    };
    if obj.get("format").and_then(Value::as_str) == Some(JSON_FORMAT_ID) {
        match obj.get("version").and_then(Value::as_u64) {
            Some(1) => {}
            _ => {
                return Err(invalid(
                    "form_data.version",
                    "This form data file was written by a newer PageWeave and can’t be read here.",
                ))
            }
        }
        let Some(Value::Array(fields)) = obj.get("fields") else {
            return Err(invalid("form_data.json_fields", "The form data file has no field list."));
        };
        if fields.len() > MAX_FORM_DATA_ENTRIES {
            return Err(invalid("form_data.too_many", "The form data file has too many fields."));
        }
        let mut out = Vec::with_capacity(fields.len());
        for f in fields {
            let name = f
                .get("name")
                .and_then(Value::as_str)
                .ok_or_else(|| invalid("form_data.no_name", "A field in the form data file has no name."))?;
            let value = json_value(f.get("value").unwrap_or(&Value::Null)).ok_or_else(|| unusable(name))?;
            out.push(DataEntry { name: name.to_string(), value });
        }
        Ok(out)
    } else {
        // A plain object of field name → value.
        if obj.len() > MAX_FORM_DATA_ENTRIES {
            return Err(invalid("form_data.too_many", "The form data file has too many fields."));
        }
        obj.iter()
            .map(|(name, v)| Ok(DataEntry { name: name.clone(), value: json_value(v).ok_or_else(|| unusable(name))? }))
            .collect()
    }
}

struct Open {
    /// Fully qualified name of a `<field>`.
    name: String,
    values: Vec<String>,
    rich_text: bool,
}

fn parse_xfdf(text: &str) -> PwResult<Vec<DataEntry>> {
    let bad = |detail: &str| invalid("form_data.xfdf", format!("The XFDF file could not be read ({detail})."));
    let mut reader = Reader::from_str(text);
    reader.config_mut().check_end_names = true;
    // Element stack: local names of every open element.
    let mut elements: Vec<String> = Vec::new();
    let mut fields: Vec<Open> = Vec::new();
    let mut value_text: Option<String> = None;
    let mut out: Vec<DataEntry> = Vec::new();
    let mut saw_root = false;

    let mut start = |e: &quick_xml::events::BytesStart,
                     elements: &mut Vec<String>,
                     fields: &mut Vec<Open>,
                     value_text: &mut Option<String>,
                     empty: bool|
     -> PwResult<()> {
        let local = e.local_name().as_ref().to_string();
        if elements.is_empty() {
            if local != "xfdf" {
                return Err(bad("it has no <xfdf> root element"));
            }
            saw_root = true;
            let ns = e.attributes().flatten().find(|a| a.key.as_ref() == "xmlns");
            if let Some(a) = ns {
                let v = a.normalized_value(XmlVersion::Implicit1_0).map_err(|_| bad("bad namespace"))?;
                if v != XFDF_NS {
                    return Err(bad("it is not in the XFDF namespace"));
                }
            }
        }
        // Fields count only inside <fields> (or nested fields), not in annotations.
        let in_fields = elements.last().is_some_and(|p| p == "fields" || p == "field");
        if local == "field" && in_fields {
            if fields.len() >= MAX_XFDF_DEPTH {
                return Err(bad("fields are nested too deeply"));
            }
            let mut name = None;
            for a in e.attributes() {
                let a = a.map_err(|_| bad("a malformed attribute"))?;
                if a.key.local_name().as_ref() == "name" {
                    name = Some(
                        a.normalized_value(XmlVersion::Implicit1_0)
                            .map_err(|_| bad("an unknown entity in a field name"))?
                            .into_owned(),
                    );
                }
            }
            let part = name.ok_or_else(|| bad("a <field> has no name"))?;
            let full = match fields.last() {
                Some(parent) => format!("{}.{part}", parent.name),
                None => part,
            };
            if empty {
                // <field name="x"/> carries no value; nothing to record.
            } else {
                fields.push(Open { name: full, values: vec![], rich_text: false });
            }
        } else if (local == "value" || local == "value-richtext")
            && elements.last().is_some_and(|p| p == "field")
            && !fields.is_empty()
        {
            if local == "value" {
                if empty {
                    if let Some(f) = fields.last_mut() {
                        f.values.push(String::new());
                    }
                } else {
                    *value_text = Some(String::new());
                }
            } else if let Some(f) = fields.last_mut() {
                f.rich_text = true;
            }
        }
        if !empty {
            if elements.len() >= MAX_XFDF_DEPTH * 2 + 8 {
                return Err(bad("elements are nested too deeply"));
            }
            elements.push(local);
        }
        Ok(())
    };

    loop {
        let event = reader.read_event().map_err(|_| bad("it is not well-formed XML"))?;
        match event {
            Event::Eof => break,
            Event::DocType(_) => {
                return Err(bad("it contains a document type declaration, which is not accepted"));
            }
            Event::Start(e) => start(&e, &mut elements, &mut fields, &mut value_text, false)?,
            Event::Empty(e) => start(&e, &mut elements, &mut fields, &mut value_text, true)?,
            Event::End(_) => {
                let local = elements.pop().unwrap_or_default();
                if local == "value" && value_text.is_some() && elements.last().is_some_and(|p| p == "field") {
                    if let (Some(t), Some(f)) = (value_text.take(), fields.last_mut()) {
                        f.values.push(t);
                    }
                } else if local == "field" && elements.last().is_some_and(|p| p == "fields" || p == "field") {
                    if let Some(f) = fields.pop() {
                        let value = match f.values.len() {
                            0 if f.rich_text => Some(DataValue::Missing),
                            0 => None, // a grouping element (only nested fields)
                            1 => f.values.into_iter().next().map(DataValue::Text),
                            _ => Some(DataValue::Several(f.values)),
                        };
                        if let Some(value) = value {
                            if out.len() >= MAX_FORM_DATA_ENTRIES {
                                return Err(invalid("form_data.too_many", "The form data file has too many fields."));
                            }
                            out.push(DataEntry { name: f.name, value });
                        }
                    }
                }
            }
            Event::Text(t) => {
                if let Some(v) = value_text.as_mut() {
                    v.push_str(&t.xml10_content());
                }
            }
            Event::CData(t) => {
                if let Some(v) = value_text.as_mut() {
                    v.push_str(&t.xml10_content());
                }
            }
            Event::GeneralRef(r) => {
                let ch = match r.resolve_char_ref() {
                    Ok(Some(c)) => c,
                    Ok(None) => match r.xml10_content().as_ref() {
                        "amp" => '&',
                        "lt" => '<',
                        "gt" => '>',
                        "quot" => '"',
                        "apos" => '\'',
                        _ => return Err(bad("it refers to an unknown entity")),
                    },
                    Err(_) => return Err(bad("it has an invalid character reference")),
                };
                if let Some(v) = value_text.as_mut() {
                    v.push(ch);
                }
            }
            Event::Decl(_) | Event::PI(_) | Event::Comment(_) => {}
        }
        if let Some(v) = &value_text {
            if v.len() as u64 > MAX_FORM_DATA_BYTES {
                return Err(bad("a value is too long"));
            }
        }
    }
    if !saw_root {
        return Err(bad("it has no <xfdf> root element"));
    }
    if !elements.is_empty() {
        return Err(bad("it ends early"));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, kind: &'static str, value: ExportValue) -> ExportEntry {
        ExportEntry { name: name.into(), kind, value }
    }

    fn sample() -> Vec<ExportEntry> {
        vec![
            entry("applicant.name", "text", ExportValue::Text("Ada <Lovelace> & \"Co\"".into())),
            entry("applicant.address.city", "text", ExportValue::Text("Zürich\r\nline 2\ttab".into())),
            entry("subscribe", "checkbox", ExportValue::Checkbox { checked: true, on: "On".into() }),
            entry("agree", "checkbox", ExportValue::Checkbox { checked: false, on: "Yes".into() }),
            entry("plan", "radio", ExportValue::Radio(Some("pro".into()))),
            entry("tier", "radio", ExportValue::Radio(None)),
            entry("notes", "text", ExportValue::Text(String::new())),
        ]
    }

    #[test]
    fn xfdf_round_trips_nested_names_and_special_characters() {
        let entries = sample();
        let w = write_form_data(FormDataFormat::Xfdf, &entries, Some("a&b.pdf"));
        assert!(w.warnings.is_empty());
        let text = String::from_utf8(w.bytes.clone()).expect("utf8");
        assert!(text.contains("<f href=\"a&amp;b.pdf\"/>"), "{text}");
        // Nested, not dotted: applicant appears once with both children inside it.
        assert_eq!(text.matches("<field name=\"applicant\">").count(), 1, "{text}");
        assert!(text.contains("<field name=\"address\">"));
        let parsed = parse_form_data(&w.bytes).expect("parse");
        assert_eq!(parsed.format, FormDataFormat::Xfdf);
        let want: Vec<DataEntry> = entries
            .iter()
            .map(|e| DataEntry { name: e.name.clone(), value: e.expected(FormDataFormat::Xfdf) })
            .collect();
        // Order of the parsed file follows the tree (applicant.* first), which is the input order here.
        assert_eq!(parsed.entries, want);
        assert_eq!(parsed.entries[1].value, DataValue::Text("Zürich\r\nline 2\ttab".into()), "CR survives");
        assert_eq!(parsed.entries[2].value, DataValue::Text("On".into()));
        assert_eq!(parsed.entries[3].value, DataValue::Text("Off".into()));
        assert_eq!(parsed.entries[5].value, DataValue::Text("Off".into()));
    }

    #[test]
    fn json_round_trips_with_booleans_and_null() {
        let entries = sample();
        let w = write_form_data(FormDataFormat::Json, &entries, Some("form.pdf"));
        let v: Value = serde_json::from_slice(&w.bytes).expect("json");
        assert_eq!(v["format"], JSON_FORMAT_ID);
        assert_eq!(v["source"], "form.pdf");
        assert_eq!(v["fields"][2]["kind"], "checkbox");
        let parsed = parse_form_data(&w.bytes).expect("parse");
        assert_eq!(parsed.format, FormDataFormat::Json);
        let want: Vec<DataEntry> = entries
            .iter()
            .map(|e| DataEntry { name: e.name.clone(), value: e.expected(FormDataFormat::Json) })
            .collect();
        assert_eq!(parsed.entries, want);
        assert_eq!(parsed.entries[2].value, DataValue::Bool(true));
        assert_eq!(parsed.entries[5].value, DataValue::Missing);
    }

    #[test]
    fn control_characters_are_left_out_of_xfdf_with_a_warning() {
        let e = [entry("x", "text", ExportValue::Text("a\u{0}b\u{1b}c".into()))];
        let w = write_form_data(FormDataFormat::Xfdf, &e, None);
        assert_eq!(w.warnings.len(), 1, "{:?}", w.warnings);
        assert!(w.warnings[0].contains("2 control"), "{:?}", w.warnings);
        let parsed = parse_form_data(&w.bytes).expect("parse");
        assert_eq!(parsed.entries[0].value, DataValue::Text("abc".into()));
        assert_eq!(e[0].expected(FormDataFormat::Xfdf), DataValue::Text("abc".into()));
        // JSON keeps them.
        let j = parse_form_data(&write_form_data(FormDataFormat::Json, &e, None).bytes).expect("json");
        assert_eq!(j.entries[0].value, DataValue::Text("a\u{0}b\u{1b}c".into()));
    }

    #[test]
    fn reads_xfdf_written_by_other_applications() {
        let xfdf = "\u{feff}<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
            <xfdf xmlns=\"http://ns.adobe.com/xfdf/\" xml:space=\"preserve\">\n\
            <f href=\"form.pdf\"/><ids original=\"AB\" modified=\"CD\"/>\n\
            <!-- comment -->\n\
            <fields>\n\
              <field name=\"full_name\"><value>Jos&#233; &amp; Ana</value></field>\n\
              <field name=\"addr\"><field name=\"zip\"><value><![CDATA[<1234>]]></value></field></field>\n\
              <field name=\"colors\"><value>Red</value><value>Blue</value></field>\n\
              <field name=\"rich\"><value-richtext><body><p>x</p></body></value-richtext></field>\n\
              <field name=\"empty\"><value/></field>\n\
              <field name=\"nothing\"/>\n\
            </fields>\n\
            <annots><field name=\"not_a_field\"><value>no</value></field></annots>\n\
            </xfdf>";
        let p = parse_form_data(xfdf.as_bytes()).expect("parse");
        let got: Vec<(&str, &DataValue)> = p.entries.iter().map(|e| (e.name.as_str(), &e.value)).collect();
        assert_eq!(
            got,
            [
                ("full_name", &DataValue::Text("José & Ana".into())),
                ("addr.zip", &DataValue::Text("<1234>".into())),
                ("colors", &DataValue::Several(vec!["Red".into(), "Blue".into()])),
                ("rich", &DataValue::Missing),
                ("empty", &DataValue::Text(String::new())),
            ]
        );
    }

    #[test]
    fn plain_json_objects_are_accepted() {
        let p = parse_form_data(br#"{"a": "x", "b": true, "c": 42, "d": ["r", 2], "e": null}"#).expect("parse");
        let got: Vec<(&str, &DataValue)> = p.entries.iter().map(|e| (e.name.as_str(), &e.value)).collect();
        assert_eq!(
            got,
            [
                ("a", &DataValue::Text("x".into())),
                ("b", &DataValue::Bool(true)),
                ("c", &DataValue::Text("42".into())),
                ("d", &DataValue::Several(vec!["r".into(), "2".into()])),
                ("e", &DataValue::Missing),
            ]
        );
    }

    #[test]
    fn hostile_or_wrong_files_are_refused_with_specific_codes() {
        let code = |b: &[u8]| parse_form_data(b).expect_err("must fail").code;
        // Entity expansion and external entities: any DOCTYPE is refused.
        let bomb = "<?xml version=\"1.0\"?><!DOCTYPE xfdf [<!ENTITY a \"aaaa\"><!ENTITY b \"&a;&a;&a;\">]>\
            <xfdf xmlns=\"http://ns.adobe.com/xfdf/\"><fields><field name=\"x\"><value>&b;</value></field></fields></xfdf>";
        assert_eq!(code(bomb.as_bytes()), "form_data.xfdf");
        let ext = "<!DOCTYPE xfdf SYSTEM \"file:///etc/passwd\"><xfdf/>";
        assert_eq!(code(ext.as_bytes()), "form_data.xfdf");
        // Undeclared entity without a DOCTYPE.
        let undeclared = "<xfdf><fields><field name=\"x\"><value>&secret;</value></field></fields></xfdf>";
        assert_eq!(code(undeclared.as_bytes()), "form_data.xfdf");
        assert_eq!(code(b"<html><body/></html>"), "form_data.xfdf");
        assert_eq!(code(b"<xfdf xmlns=\"urn:other\"/>"), "form_data.xfdf");
        assert_eq!(code(b"<xfdf><fields><field name=\"x\"><value>a</field></fields></xfdf>"), "form_data.xfdf");
        assert_eq!(code(b"<xfdf><fields><field name=\"x\"><value>a</value>"), "form_data.xfdf");
        assert_eq!(code(b"<xfdf><fields><field><value>a</value></field></fields></xfdf>"), "form_data.xfdf");
        assert_eq!(code(b"%FDF-1.2\n1 0 obj"), "form_data.fdf");
        assert_eq!(code(b"hello"), "form_data.format");
        assert_eq!(code(b""), "form_data.format");
        assert_eq!(code(&[0xff, 0xfe, b'<', 0]), "form_data.encoding");
        assert_eq!(code(b"{\"a\": "), "form_data.json");
        assert_eq!(code(b"{\"a\": {\"b\": 1}}"), "form_data.json_value");
        assert_eq!(code(b"{\"format\": \"pageweave-form-data\", \"version\": 2, \"fields\": []}"), "form_data.version");
        assert_eq!(
            code(b"{\"format\": \"pageweave-form-data\", \"version\": 1, \"fields\": [{\"value\": 1}]}"),
            "form_data.no_name"
        );
        assert_eq!(code(b"{\"\": \"x\"}"), "form_data.no_name");
        let deep = format!(
            "<xfdf><fields>{}<value>x</value>{}</fields></xfdf>",
            "<field name=\"a\">".repeat(70),
            "</field>".repeat(70)
        );
        assert_eq!(code(deep.as_bytes()), "form_data.xfdf");
        let big = vec![b' '; (MAX_FORM_DATA_BYTES + 1) as usize];
        assert_eq!(code(&big), "form_data.too_large");
        let many: String = (0..=MAX_FORM_DATA_ENTRIES).map(|i| format!("\"f{i}\":1,")).collect();
        assert_eq!(code(format!("{{{}\"z\":1}}", many).as_bytes()), "form_data.too_many");
    }
}
