//! Form data export and import against a document's AcroForm: which field each entry of a form
//! data file belongs to, whether its value fits the field, and the export view of a form.
//!
//! The data file is untrusted. Nothing here touches the document; filling goes through
//! [`crate::forms::fill_form`], which checks every value against the form again before writing.

use crate::forms::{form_fields, FieldKind, FormField, FormInfo};
use pw_core::formdata::{
    parse_form_data, write_form_data, DataEntry, DataValue, ExportEntry, ExportValue, FormDataFormat,
    MAX_FORM_DATA_BYTES,
};
use pw_core::ops::{FieldValue, FormValue, MAX_FORM_TEXT_CHARS};
use pw_core::output::{write_atomic_bytes, CollisionPolicy, CommittedOutput};
use pw_core::{ErrorCategory, PwError, PwResult};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::Path;

/// A value from the data file that fits a field.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MatchedValue {
    pub name: String,
    pub label: Option<String>,
    pub kind: FieldKind,
    /// The value to fill.
    pub value: FormValue,
    /// What the document holds now.
    pub current: FormValue,
    /// False when the document already holds this value.
    pub changed: bool,
}

/// An entry of the data file that names a field of this form but cannot be used.
/// An empty value for a radio group or choice field: clear it when the document has a choice.
fn clear_if_chosen(f: &FormField) -> Option<FormValue> {
    match current_value(f) {
        FormValue::Text(c) if c.is_empty() => None,
        _ => Some(FormValue::Text(String::new())),
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SkippedEntry {
    pub name: String,
    pub reason: String,
}

/// How a form data file matches a document's form.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FormDataMatch {
    pub format: FormDataFormat,
    /// Entries in the file.
    pub entries: usize,
    pub matched: Vec<MatchedValue>,
    pub skipped: Vec<SkippedEntry>,
    /// Names in the file that this form does not have.
    pub unknown: Vec<String>,
}

impl FormDataMatch {
    /// Values that would change the document, as fill requests.
    pub fn changes(&self) -> Vec<FieldValue> {
        self.matched
            .iter()
            .filter(|m| m.changed)
            .map(|m| FieldValue { name: m.name.clone(), value: m.value.clone() })
            .collect()
    }
}

fn is_fillable(kind: FieldKind) -> bool {
    matches!(kind, FieldKind::Text | FieldKind::Checkbox | FieldKind::Radio | FieldKind::Combo | FieldKind::List)
}

/// The value a field holds in the document, in fill-request form.
pub fn current_value(f: &FormField) -> FormValue {
    match f.kind {
        FieldKind::Checkbox => FormValue::Checked(f.widgets.iter().any(|w| w.checked)),
        FieldKind::Radio => FormValue::Text(
            f.widgets.iter().find(|w| w.checked).and_then(|w| w.export_value.clone()).unwrap_or_default(),
        ),
        _ => FormValue::Text(f.value.clone().unwrap_or_default()),
    }
}

fn on_states(f: &FormField) -> impl Iterator<Item = &str> {
    f.widgets.iter().filter_map(|w| w.export_value.as_deref())
}

/// The fill value for one entry, or why it cannot be used.
fn convert(f: &FormField, value: &DataValue) -> Result<Option<FormValue>, String> {
    let single = match value {
        DataValue::Several(v) if v.len() == 1 => DataValue::Text(v[0].clone()),
        other => other.clone(),
    };
    match (f.kind, &single) {
        (_, DataValue::Missing) => Err("The file has no plain value for it.".into()),
        (_, DataValue::Several(_)) => Err("The file chooses several values; this field takes one.".into()),
        (FieldKind::Text, DataValue::Text(t)) => {
            if t.chars().count() > MAX_FORM_TEXT_CHARS {
                Err("The value is too long.".into())
            } else {
                Ok(Some(FormValue::Text(t.clone())))
            }
        }
        (FieldKind::Text | FieldKind::Combo | FieldKind::List, DataValue::Bool(_)) => {
            Err("The file gives true/false, but this field takes text.".into())
        }
        (FieldKind::Checkbox, DataValue::Bool(b)) => Ok(Some(FormValue::Checked(*b))),
        (FieldKind::Checkbox, DataValue::Text(t)) => {
            if on_states(f).any(|s| s == t) {
                Ok(Some(FormValue::Checked(true)))
            } else if ["off", "no", "false", ""].iter().any(|s| t.eq_ignore_ascii_case(s)) {
                Ok(Some(FormValue::Checked(false)))
            } else if ["yes", "on", "true"].iter().any(|s| t.eq_ignore_ascii_case(s)) {
                Ok(Some(FormValue::Checked(true)))
            } else {
                Err(format!("“{t}” is not a state of this checkbox."))
            }
        }
        (FieldKind::Radio, DataValue::Text(t)) => {
            if on_states(f).any(|s| s == t) {
                Ok(Some(FormValue::Text(t.clone())))
            } else if t.is_empty() || t.eq_ignore_ascii_case("off") {
                // Nothing chosen in the file: clear the document's choice, if it has one.
                Ok(clear_if_chosen(f))
            } else {
                Err(format!("“{t}” is not one of its options."))
            }
        }
        (FieldKind::Radio, DataValue::Bool(_)) => {
            Err("The file gives true/false, but this field takes one of its options.".into())
        }
        (FieldKind::Combo | FieldKind::List, DataValue::Text(t)) if t.is_empty() => Ok(clear_if_chosen(f)),
        (FieldKind::Combo, DataValue::Text(t)) if f.editable || f.options.iter().any(|o| o == t) => {
            Ok(Some(FormValue::Text(t.clone())))
        }
        (FieldKind::List, DataValue::Text(t)) if f.options.iter().any(|o| o == t) => {
            Ok(Some(FormValue::Text(t.clone())))
        }
        (FieldKind::Combo | FieldKind::List, DataValue::Text(t)) => Err(format!("“{t}” is not one of its choices.")),
        (FieldKind::PushButton | FieldKind::Signature | FieldKind::Unknown, _) => {
            Err("This kind of field can’t be filled.".into())
        }
    }
}

/// Match parsed entries to the form's fields. Pure: no document access.
pub fn match_entries(fields: &[FormField], format: FormDataFormat, entries: &[DataEntry]) -> FormDataMatch {
    let by_name: HashMap<&str, &FormField> = fields.iter().map(|f| (f.name.as_str(), f)).collect();
    let mut seen: HashSet<&str> = HashSet::new();
    let mut out = FormDataMatch { format, entries: entries.len(), matched: vec![], skipped: vec![], unknown: vec![] };
    for e in entries {
        let Some(f) = by_name.get(e.name.as_str()) else {
            if seen.insert(e.name.as_str()) {
                out.unknown.push(e.name.clone());
            }
            continue;
        };
        let skip = |reason: &str| SkippedEntry { name: e.name.clone(), reason: reason.to_string() };
        if !seen.insert(e.name.as_str()) {
            out.skipped.push(skip("The file lists this field more than once; the first value is used."));
            continue;
        }
        if f.read_only && is_fillable(f.kind) {
            out.skipped.push(skip("The field is read-only."));
            continue;
        }
        match convert(f, &e.value) {
            Ok(Some(value)) => {
                let current = current_value(f);
                out.matched.push(MatchedValue {
                    name: f.name.clone(),
                    label: f.label.clone(),
                    kind: f.kind,
                    changed: value != current,
                    value,
                    current,
                });
            }
            Ok(None) => {}
            Err(reason) => out.skipped.push(skip(&reason)),
        }
    }
    out
}

/// Read a form data file from disk, bounded by the size limit.
pub fn read_data_file(path: &Path) -> PwResult<Vec<u8>> {
    let file = std::fs::File::open(path).map_err(|e| PwError::from_io(&e, "form data file"))?;
    let mut bytes = Vec::new();
    file.take(MAX_FORM_DATA_BYTES + 1).read_to_end(&mut bytes).map_err(|e| PwError::from_io(&e, "form data file"))?;
    Ok(bytes)
}

fn refuse_xfa(info: &FormInfo) -> PwResult<()> {
    if info.xfa_only {
        return Err(PwError::new(
            ErrorCategory::UnsupportedOperation,
            "form.xfa",
            "This is a dynamic XFA form, which PageWeave cannot fill or export.",
        ));
    }
    Ok(())
}

/// Match a data file against the document's form (read-only).
pub fn read_form_data(input: &Path, password: Option<&str>, data: &Path) -> PwResult<FormDataMatch> {
    let parsed = parse_form_data(&read_data_file(data)?)?;
    let info = form_fields(input, password)?;
    refuse_xfa(&info)?;
    if !info.fields.iter().any(|f| is_fillable(f.kind)) {
        return Err(PwError::invalid("form_data.no_form", "This document has no form fields to fill."));
    }
    Ok(match_entries(&info.fields, parsed.format, &parsed.entries))
}

/// Fill the document from a data file into a verified new file.
pub fn import_form_data(
    input: &Path,
    password: Option<&str>,
    data: &Path,
    output: &Path,
    policy: CollisionPolicy,
) -> PwResult<(CommittedOutput, FormDataMatch, Vec<String>)> {
    let m = read_form_data(input, password, data)?;
    let changes = m.changes();
    if changes.is_empty() {
        let msg = if m.matched.is_empty() {
            "None of the values in the file can be used for this form."
        } else {
            "The form already holds every value in the file; nothing would change."
        };
        return Err(PwError::invalid("form_data.nothing", msg));
    }
    let protected = [input.to_path_buf(), data.to_path_buf()];
    let (committed, warnings) =
        crate::forms::apply_edits_protecting(input, password, &changes, &[], output, policy, &protected)?;
    Ok((committed, m, warnings))
}

/// What exporting would write for each field, with unsaved `values` taking the place of the
/// document's own. Push buttons, signature fields and unknown kinds are left out.
pub fn export_entries(info: &FormInfo, values: &[FieldValue]) -> PwResult<Vec<ExportEntry>> {
    let pending: HashMap<&str, &FormValue> = values.iter().map(|v| (v.name.as_str(), &v.value)).collect();
    let by_name: HashSet<&str> = info.fields.iter().map(|f| f.name.as_str()).collect();
    if let Some(v) = values.iter().find(|v| !by_name.contains(v.name.as_str())) {
        return Err(PwError::invalid("form.unknown_field", format!("The form has no field named “{}”.", v.name)));
    }
    let mut out = Vec::new();
    for f in info.fields.iter().filter(|f| is_fillable(f.kind)) {
        let value = pending.get(f.name.as_str()).map(|v| (*v).clone()).unwrap_or_else(|| current_value(f));
        let wrong =
            || PwError::invalid("form.value_type", format!("The value for “{}” does not fit the field.", f.name));
        let (kind, value) = match (f.kind, value) {
            (FieldKind::Checkbox, FormValue::Checked(checked)) => {
                ("checkbox", ExportValue::Checkbox { checked, on: on_states(f).next().unwrap_or("Yes").to_string() })
            }
            (FieldKind::Radio, FormValue::Text(t)) => {
                if !t.is_empty() && !on_states(f).any(|s| s == t) {
                    return Err(wrong());
                }
                ("radio", ExportValue::Radio(if t.is_empty() { None } else { Some(t) }))
            }
            (FieldKind::Text, FormValue::Text(t)) => ("text", ExportValue::Text(t)),
            (FieldKind::Combo, FormValue::Text(t)) => ("combo", ExportValue::Text(t)),
            (FieldKind::List, FormValue::Text(t)) => ("list", ExportValue::Text(t)),
            _ => return Err(wrong()),
        };
        out.push(ExportEntry { name: f.name.clone(), kind, value });
    }
    Ok(out)
}

/// Write the form's values (with unsaved `values` applied) to `output`; the written file is read
/// back and must hold exactly the intended entries before it is committed.
pub fn export_form_data(
    input: &Path,
    password: Option<&str>,
    format: FormDataFormat,
    values: &[FieldValue],
    output: &Path,
    policy: CollisionPolicy,
) -> PwResult<(CommittedOutput, usize, Vec<String>)> {
    let info = form_fields(input, password)?;
    refuse_xfa(&info)?;
    let entries = export_entries(&info, values)?;
    if entries.is_empty() {
        return Err(PwError::invalid("form_data.no_form", "This document has no form fields to export."));
    }
    let source = input.file_name().and_then(|n| n.to_str());
    let written = write_form_data(format, &entries, source);
    let expected: Vec<DataEntry> =
        entries.iter().map(|e| DataEntry { name: e.name.clone(), value: e.expected(format) }).collect();
    let committed = write_atomic_bytes(output, policy, &[input.to_path_buf()], &written.bytes, |staged| {
        let back = parse_form_data(&read_data_file(staged)?).map_err(|_| verify_failed())?;
        // XFDF groups names into a tree, so compare as sets of (name, value).
        let mut got: Vec<_> = back.entries.iter().map(|e| (&e.name, &e.value)).collect();
        let mut want: Vec<_> = expected.iter().map(|e| (&e.name, &e.value)).collect();
        got.sort_by(|a, b| a.0.cmp(b.0));
        want.sort_by(|a, b| a.0.cmp(b.0));
        if back.format != format || got != want {
            return Err(verify_failed());
        }
        Ok(())
    })?;
    Ok((committed, entries.len(), written.warnings))
}

fn verify_failed() -> PwError {
    PwError::new(
        ErrorCategory::ValidationFailed,
        "output.verify",
        "The form data file failed verification (it did not read back the same) and was not saved.",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::forms::FormWidget;
    use pw_core::ops::RectPt;

    fn widget(export: Option<&str>, checked: bool) -> FormWidget {
        FormWidget {
            page: 0,
            rect: RectPt { left: 0.0, bottom: 0.0, right: 1.0, top: 1.0 },
            export_value: export.map(str::to_string),
            checked,
        }
    }

    fn field(name: &str, kind: FieldKind) -> FormField {
        FormField {
            name: name.into(),
            label: None,
            kind,
            value: None,
            options: vec![],
            read_only: false,
            required: false,
            multiline: false,
            editable: false,
            default_value: None,
            widgets: vec![widget(None, false)],
        }
    }

    fn fields() -> Vec<FormField> {
        let mut cb = field("subscribe", FieldKind::Checkbox);
        cb.widgets = vec![widget(Some("On"), false)];
        let mut radio = field("plan", FieldKind::Radio);
        radio.widgets = vec![widget(Some("basic"), true), widget(Some("pro"), false)];
        let mut empty_radio = field("tier", FieldKind::Radio);
        empty_radio.widgets = vec![widget(Some("a"), false), widget(Some("b"), false)];
        let mut combo = field("country", FieldKind::Combo);
        combo.options = vec!["Canada".into(), "Japan".into()];
        combo.value = Some("Canada".into());
        let mut list = field("colors", FieldKind::List);
        list.options = vec!["Red".into(), "Blue".into()];
        let mut ro = field("ref_no", FieldKind::Text);
        ro.read_only = true;
        ro.value = Some("R-1".into());
        let mut name = field("full_name", FieldKind::Text);
        name.value = Some("Ada".into());
        vec![name, cb, radio, empty_radio, combo, list, ro, field("submit", FieldKind::PushButton)]
    }

    fn e(name: &str, value: DataValue) -> DataEntry {
        DataEntry { name: name.into(), value }
    }

    fn t(s: &str) -> DataValue {
        DataValue::Text(s.into())
    }

    #[test]
    fn values_are_matched_by_field_kind() {
        let m = match_entries(
            &fields(),
            FormDataFormat::Xfdf,
            &[
                e("full_name", t("Ada")),
                e("subscribe", t("Yes")),
                e("plan", t("pro")),
                e("tier", t("Off")),
                e("country", t("Japan")),
                e("colors", DataValue::Several(vec!["Blue".into()])),
                e("missing", t("x")),
                e("missing", t("y")),
            ],
        );
        let got: Vec<(&str, &FormValue, bool)> =
            m.matched.iter().map(|v| (v.name.as_str(), &v.value, v.changed)).collect();
        assert_eq!(
            got,
            [
                ("full_name", &FormValue::Text("Ada".into()), false),
                ("subscribe", &FormValue::Checked(true), true),
                ("plan", &FormValue::Text("pro".into()), true),
                ("country", &FormValue::Text("Japan".into()), true),
                ("colors", &FormValue::Text("Blue".into()), true),
            ]
        );
        assert!(m.skipped.is_empty(), "{:?}", m.skipped);
        assert_eq!(m.unknown, ["missing"], "unknown names listed once");
        assert_eq!(m.entries, 8);
        assert_eq!(m.changes().len(), 4, "unchanged values are not filled");
    }

    #[test]
    fn unusable_entries_say_why() {
        let m = match_entries(
            &fields(),
            FormDataFormat::Json,
            &[
                e("full_name", DataValue::Bool(true)),
                e("subscribe", t("maybe")),
                e("plan", t("enterprise")),
                e("country", t("Mars")),
                e("colors", DataValue::Several(vec!["Red".into(), "Blue".into()])),
                e("ref_no", t("R-2")),
                e("submit", t("x")),
                e("plan", DataValue::Missing),
                e("tier", DataValue::Bool(false)),
            ],
        );
        assert!(m.matched.is_empty(), "{:?}", m.matched);
        let reasons: Vec<(&str, &str)> = m.skipped.iter().map(|s| (s.name.as_str(), s.reason.as_str())).collect();
        assert_eq!(reasons.len(), 9, "{reasons:?}");
        assert!(reasons[0].1.contains("takes text"));
        assert!(reasons[1].1.contains("“maybe” is not a state"));
        assert!(reasons[2].1.contains("not one of its options"));
        assert!(reasons[3].1.contains("not one of its choices"));
        assert!(reasons[4].1.contains("several values"));
        assert!(reasons[5].1.contains("read-only"));
        assert!(reasons[6].1.contains("can’t be filled"));
        assert!(reasons[7].1.contains("more than once"));
        assert!(reasons[8].1.contains("true/false"));
    }

    #[test]
    fn empty_values_clear_a_chosen_radio_or_choice_and_change_nothing_otherwise() {
        let m = match_entries(
            &fields(),
            FormDataFormat::Xfdf,
            &[e("plan", t("Off")), e("tier", t("")), e("country", t("")), e("colors", t(""))],
        );
        assert!(m.skipped.is_empty(), "{:?}", m.skipped);
        let got: Vec<(&str, &FormValue)> = m.matched.iter().map(|v| (v.name.as_str(), &v.value)).collect();
        // "tier" and "colors" have nothing chosen: nothing to clear.
        assert_eq!(got, [("plan", &FormValue::Text(String::new())), ("country", &FormValue::Text(String::new()))]);
    }

    #[test]
    fn editable_combo_takes_any_text_and_export_uses_pending_values() {
        let mut fs = fields();
        fs[4].editable = true;
        let m = match_entries(&fs, FormDataFormat::Json, &[e("country", t("Mars"))]);
        assert_eq!(m.matched[0].value, FormValue::Text("Mars".into()));

        let info = FormInfo { fields: fs, has_xfa: false, xfa_only: false, truncated: false };
        let pending = [FieldValue { name: "subscribe".into(), value: FormValue::Checked(true) }];
        let out = export_entries(&info, &pending).expect("entries");
        let names: Vec<&str> = out.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(
            names,
            ["full_name", "subscribe", "plan", "tier", "country", "colors", "ref_no"],
            "push button left out"
        );
        assert_eq!(out[1].value, ExportValue::Checkbox { checked: true, on: "On".into() });
        assert_eq!(out[2].value, ExportValue::Radio(Some("basic".into())));
        assert_eq!(out[3].value, ExportValue::Radio(None));
        let bad = [FieldValue { name: "nope".into(), value: FormValue::Text("x".into()) }];
        assert_eq!(export_entries(&info, &bad).expect_err("unknown").code, "form.unknown_field");
        let wrong = [FieldValue { name: "plan".into(), value: FormValue::Text("zzz".into()) }];
        assert_eq!(export_entries(&info, &wrong).expect_err("wrong").code, "form.value_type");
    }
}
