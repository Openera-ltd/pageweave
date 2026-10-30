//! AcroForm reading and filling through PDFium's own form filler.
//!
//! Values are entered the way a user would — focus a field, select its text and replace it; click
//! a checkbox or radio button; select a list entry — so PDFium regenerates each widget's appearance
//! stream. Writing `/V` directly would leave stale appearances that most viewers keep showing.
//!
//! pdfium-render 0.9.4 does not expose its form-fill handle or raw bindings, so this module resolves
//! the few functions it needs from the already-loaded PDFium library. Every call happens while
//! holding the exclusive engine lock (`engine_exclusive`), so nothing else uses PDFium meanwhile.
//! Each document opened here is loaded from memory and fully torn down before the lock is released.

#![allow(unsafe_code)] // D-006: the only module with direct PDFium calls.

use crate::document::{load, preflight, OpenOptions};
use crate::edit::{apply_annotations, protection_warnings, save_bytes, verify_annotation_counts};
use crate::{engine, engine_exclusive, map_pdfium_error};
use pdfium_render::prelude::{
    FPDF_ANNOTATION, FPDF_BOOL, FPDF_DOCUMENT, FPDF_DWORD, FPDF_FILEWRITE, FPDF_FORMFILLINFO, FPDF_FORMHANDLE,
    FPDF_PAGE, FPDF_WCHAR, FPDF_WIDESTRING, FS_RECTF,
};
use pw_core::ops::{AnnotationSpec, FieldValue, FormValue, RectPt};
use pw_core::output::{write_atomic_bytes, CollisionPolicy, CommittedOutput};
use pw_core::{ErrorCategory, PwError, PwResult};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::ffi::{c_char, c_int, c_ulong, c_void, CString};
use std::path::Path;
use std::sync::OnceLock;

/// Upper bound on widgets inspected per document (hostile files).
const MAX_WIDGETS: usize = 20_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldKind {
    Text,
    Checkbox,
    Radio,
    Combo,
    List,
    PushButton,
    Signature,
    Unknown,
}

/// One on-page appearance of a field.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FormWidget {
    pub page: u32,
    /// PDF points, unrotated page space.
    pub rect: RectPt,
    /// For checkboxes and radio buttons: the value this widget represents when on.
    pub export_value: Option<String>,
    pub checked: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FormField {
    pub name: String,
    /// The field's user-facing name (`/TU`), when the document provides one.
    pub label: Option<String>,
    pub kind: FieldKind,
    /// Current value (`Off` for unchecked buttons, the export value when checked).
    pub value: Option<String>,
    /// Choices of combo and list boxes, in document order.
    pub options: Vec<String>,
    pub read_only: bool,
    pub required: bool,
    pub multiline: bool,
    /// Combo box that also accepts typed text.
    pub editable: bool,
    /// The value "Reset form" restores (`/DV`), when the document sets one.
    #[serde(default)]
    pub default_value: Option<String>,
    pub widgets: Vec<FormWidget>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FormInfo {
    pub fields: Vec<FormField>,
    /// The document carries an XFA form (D-038).
    pub has_xfa: bool,
    /// Only XFA: a dynamic XFA form, or XFA without standard fields. These cannot be filled by this
    /// engine build (no XFA). A hybrid form (XFA plus a complete standard form, as on most
    /// government forms) is filled through its standard fields instead.
    #[serde(default)]
    pub xfa_only: bool,
    pub truncated: bool,
}

// FPDF_FORMFIELD_* and FPDF_ANNOT_WIDGET from fpdf_formfill.h / fpdf_annot.h.
const FIELD_PUSHBUTTON: c_int = 1;
const FIELD_CHECKBOX: c_int = 2;
const FIELD_RADIOBUTTON: c_int = 3;
const FIELD_COMBOBOX: c_int = 4;
const FIELD_LISTBOX: c_int = 5;
const FIELD_TEXTFIELD: c_int = 6;
const FIELD_SIGNATURE: c_int = 7;
const ANNOT_WIDGET: c_int = 20;
// Field flags (PDF 32000-1:2008 tables 221, 226, 228).
const FLAG_READ_ONLY: c_int = 1;
const FLAG_REQUIRED: c_int = 1 << 1;
const FLAG_MULTILINE: c_int = 1 << 12;
const FLAG_EDIT: c_int = 1 << 18;
// FPDF_GetLastError
const ERR_PASSWORD: c_ulong = 4;
const ERR_SECURITY: c_ulong = 5;

macro_rules! api {
    ($($name:ident: fn($($arg:ty),*) $(-> $ret:ty)?;)*) => {
        #[allow(non_snake_case)]
        struct Api {
            _lib: libloading::Library,
            $($name: unsafe extern "C" fn($($arg),*) $(-> $ret)?,)*
        }
        impl Api {
            #[allow(non_snake_case)]
            fn load(path: &Path) -> Result<Api, String> {
                // SAFETY: the same library pdfium-render already loaded and initialized; loading it
                // again only increments its reference count.
                let lib = unsafe { libloading::Library::new(path) }.map_err(|e| e.to_string())?;
                $(
                    // SAFETY: the declared signature matches the PDFium chromium/7881 header.
                    let $name = unsafe {
                        *lib.get::<unsafe extern "C" fn($($arg),*) $(-> $ret)?>(concat!(stringify!($name), "\0").as_bytes())
                            .map_err(|e| format!("{}: {e}", stringify!($name)))?
                    };
                )*
                Ok(Api { _lib: lib, $($name,)* })
            }
        }
    };
}

api! {
    FPDF_LoadMemDocument64: fn(*const c_void, usize, *const c_char) -> FPDF_DOCUMENT;
    FPDF_GetLastError: fn() -> c_ulong;
    FPDF_CloseDocument: fn(FPDF_DOCUMENT);
    FPDF_GetPageCount: fn(FPDF_DOCUMENT) -> c_int;
    FPDF_LoadPage: fn(FPDF_DOCUMENT, c_int) -> FPDF_PAGE;
    FPDF_ClosePage: fn(FPDF_PAGE);
    FPDF_SaveAsCopy: fn(FPDF_DOCUMENT, *mut FPDF_FILEWRITE, FPDF_DWORD) -> FPDF_BOOL;
    FPDF_GetDocPermissions: fn(FPDF_DOCUMENT) -> c_ulong;
    FPDF_GetFormType: fn(FPDF_DOCUMENT) -> c_int;
    FPDFDOC_InitFormFillEnvironment: fn(FPDF_DOCUMENT, *mut FPDF_FORMFILLINFO) -> FPDF_FORMHANDLE;
    FPDFDOC_ExitFormFillEnvironment: fn(FPDF_FORMHANDLE);
    FORM_OnAfterLoadPage: fn(FPDF_PAGE, FPDF_FORMHANDLE);
    FORM_OnBeforeClosePage: fn(FPDF_PAGE, FPDF_FORMHANDLE);
    FORM_SetFocusedAnnot: fn(FPDF_FORMHANDLE, FPDF_ANNOTATION) -> FPDF_BOOL;
    FORM_SelectAllText: fn(FPDF_FORMHANDLE, FPDF_PAGE) -> FPDF_BOOL;
    FORM_ReplaceSelection: fn(FPDF_FORMHANDLE, FPDF_PAGE, FPDF_WIDESTRING);
    FORM_ForceToKillFocus: fn(FPDF_FORMHANDLE) -> FPDF_BOOL;
    FORM_OnMouseMove: fn(FPDF_FORMHANDLE, FPDF_PAGE, c_int, f64, f64) -> FPDF_BOOL;
    FORM_OnLButtonDown: fn(FPDF_FORMHANDLE, FPDF_PAGE, c_int, f64, f64) -> FPDF_BOOL;
    FORM_OnLButtonUp: fn(FPDF_FORMHANDLE, FPDF_PAGE, c_int, f64, f64) -> FPDF_BOOL;
    FORM_SetIndexSelected: fn(FPDF_FORMHANDLE, FPDF_PAGE, c_int, FPDF_BOOL) -> FPDF_BOOL;
    FPDFPage_GetAnnotCount: fn(FPDF_PAGE) -> c_int;
    FPDFPage_GetAnnot: fn(FPDF_PAGE, c_int) -> FPDF_ANNOTATION;
    FPDFPage_CloseAnnot: fn(FPDF_ANNOTATION);
    FPDFAnnot_GetSubtype: fn(FPDF_ANNOTATION) -> c_int;
    FPDFAnnot_GetRect: fn(FPDF_ANNOTATION, *mut FS_RECTF) -> FPDF_BOOL;
    FPDFAnnot_GetFormFieldName: fn(FPDF_FORMHANDLE, FPDF_ANNOTATION, *mut FPDF_WCHAR, c_ulong) -> c_ulong;
    FPDFAnnot_GetFormFieldAlternateName: fn(FPDF_FORMHANDLE, FPDF_ANNOTATION, *mut FPDF_WCHAR, c_ulong) -> c_ulong;
    FPDFAnnot_GetFormFieldType: fn(FPDF_FORMHANDLE, FPDF_ANNOTATION) -> c_int;
    FPDFAnnot_GetFormFieldFlags: fn(FPDF_FORMHANDLE, FPDF_ANNOTATION) -> c_int;
    FPDFAnnot_GetFormFieldValue: fn(FPDF_FORMHANDLE, FPDF_ANNOTATION, *mut FPDF_WCHAR, c_ulong) -> c_ulong;
    FPDFAnnot_GetFormFieldExportValue: fn(FPDF_FORMHANDLE, FPDF_ANNOTATION, *mut FPDF_WCHAR, c_ulong) -> c_ulong;
    FPDFAnnot_IsChecked: fn(FPDF_FORMHANDLE, FPDF_ANNOTATION) -> FPDF_BOOL;
    FPDFAnnot_GetOptionCount: fn(FPDF_FORMHANDLE, FPDF_ANNOTATION) -> c_int;
    FPDFAnnot_GetOptionLabel: fn(FPDF_FORMHANDLE, FPDF_ANNOTATION, c_int, *mut FPDF_WCHAR, c_ulong) -> c_ulong;
}

fn api(path: &Path) -> PwResult<&'static Api> {
    static API: OnceLock<Result<Api, String>> = OnceLock::new();
    API.get_or_init(|| Api::load(path)).as_ref().map_err(|_| {
        PwError::new(ErrorCategory::EngineFailure, "pdfium.forms_unavailable", "Form filling is not available.")
            .with_action("Reinstall PageWeave to repair the PDF engine.")
    })
}

/// Why the last FPDF_LoadMemDocument64 failed.
fn open_error(api: &Api) -> PwError {
    // SAFETY: plain getter.
    match unsafe { (api.FPDF_GetLastError)() } {
        ERR_PASSWORD => {
            PwError::new(ErrorCategory::PasswordRequired, "pdf.password", "This document is password protected.")
        }
        ERR_SECURITY => PwError::new(
            ErrorCategory::UnsupportedFormat,
            "pdf.security_handler",
            "This document uses an unsupported security handler.",
        ),
        _ => PwError::new(ErrorCategory::Corrupt, "pdf.format", "The file is damaged or is not a valid PDF."),
    }
}

/// PDFium writer collecting the saved file in memory.
#[repr(C)]
struct Writer {
    base: FPDF_FILEWRITE,
    out: Vec<u8>,
}

unsafe extern "C" fn write_block(this: *mut FPDF_FILEWRITE, data: *const c_void, size: c_ulong) -> c_int {
    // SAFETY: `this` is the `base` field of a live Writer (repr(C), first field).
    let w = unsafe { &mut *(this as *mut Writer) };
    if size > 0 {
        let chunk = unsafe { std::slice::from_raw_parts(data as *const u8, size as usize) };
        w.out.extend_from_slice(chunk);
    }
    1
}

impl Writer {
    fn new() -> Writer {
        Writer { base: FPDF_FILEWRITE { version: 1, WriteBlock: Some(write_block) }, out: Vec::new() }
    }
}

/// Permission bits that carry a meaning (print, modify, copy, annotate, fill forms, accessibility,
/// assemble, high-quality print). PDFium reports all bits when the owner password was given.
const PERMISSION_BITS: c_ulong = 0xF3C;
/// fpdf_save.h: save without the document's encryption.
const FPDF_REMOVE_SECURITY: FPDF_DWORD = 3;

/// The document saved by PDFium without its encryption: everything else (bookmarks, properties,
/// forms, attachments) is kept. Refused when the file restricts what its users may do and was
/// opened without its permissions (owner) password — removing the protection would lift
/// restrictions its owner set.
pub(crate) fn save_without_security(path: &Path, password: Option<&str>) -> PwResult<Vec<u8>> {
    let (_guard, lib) = engine_exclusive()?;
    let api = api(lib)?;
    preflight(path)?;
    let bytes = std::fs::read(path).map_err(|e| PwError::from_io(&e, "file"))?;
    let pw = password
        .map(CString::new)
        .transpose()
        .map_err(|_| PwError::invalid("pdf.password_nul", "The password is not valid."))?;
    // SAFETY: `bytes` outlives the document, which is closed below before they are dropped.
    let doc = unsafe {
        (api.FPDF_LoadMemDocument64)(
            bytes.as_ptr().cast(),
            bytes.len(),
            pw.as_ref().map_or(std::ptr::null(), |p| p.as_ptr()),
        )
    };
    if doc.is_null() {
        return Err(open_error(api));
    }
    // SAFETY: valid document handle.
    let perms = unsafe { (api.FPDF_GetDocPermissions)(doc) };
    let result = if perms & PERMISSION_BITS != PERMISSION_BITS {
        Err(PwError::new(
            ErrorCategory::Permission,
            "pdf.owner_restricted",
            "The owner of this file restricted what can be done with it, so its protection cannot be changed.",
        )
        .with_action("Open the file with its permissions password, or ask its owner for an unrestricted copy."))
    } else {
        let mut w = Writer::new();
        // SAFETY: valid document; `w` outlives the call.
        if unsafe { (api.FPDF_SaveAsCopy)(doc, &mut w.base, FPDF_REMOVE_SECURITY) } == 0 {
            Err(fail("pdf.save", "The file could not be saved without its protection."))
        } else {
            Ok(w.out)
        }
    };
    // SAFETY: the handle is not used afterwards.
    unsafe { (api.FPDF_CloseDocument)(doc) };
    result
}

fn fail(code: &str, msg: impl Into<String>) -> PwError {
    PwError::new(ErrorCategory::EngineFailure, code, msg)
}

/// A document opened for form work: owns the bytes, the document, the form environment and the
/// pages loaded so far, and tears them down in the required order on drop.
struct FormDoc<'a> {
    api: &'a Api,
    _bytes: Vec<u8>,
    doc: FPDF_DOCUMENT,
    form: FPDF_FORMHANDLE,
    _info: Box<FPDF_FORMFILLINFO>,
    pages: Vec<FPDF_PAGE>,
}

impl<'a> FormDoc<'a> {
    fn open(api: &'a Api, path: &Path, password: Option<&str>) -> PwResult<FormDoc<'a>> {
        preflight(path)?;
        let bytes = std::fs::read(path).map_err(|e| PwError::from_io(&e, "file"))?;
        let pw = password
            .map(CString::new)
            .transpose()
            .map_err(|_| PwError::invalid("pdf.password_nul", "The password is not valid."))?;
        // SAFETY: `bytes` outlives the document (owned by FormDoc, released after FPDF_CloseDocument).
        let doc = unsafe {
            (api.FPDF_LoadMemDocument64)(
                bytes.as_ptr().cast(),
                bytes.len(),
                pw.as_ref().map_or(std::ptr::null(), |p| p.as_ptr()),
            )
        };
        if doc.is_null() {
            return Err(open_error(api));
        }
        // SAFETY: an all-None callback table is valid (PDFium checks each callback before use); it is
        // boxed so its address stays fixed until FPDFDOC_ExitFormFillEnvironment.
        let mut info: Box<FPDF_FORMFILLINFO> = Box::new(unsafe { std::mem::zeroed() });
        info.version = 1;
        let form = unsafe { (api.FPDFDOC_InitFormFillEnvironment)(doc, &mut *info) };
        let mut fd = FormDoc { api, _bytes: bytes, doc, form, _info: info, pages: Vec::new() };
        if fd.form.is_null() {
            return Err(fail("form.init", "The document's form could not be prepared."));
        }
        // SAFETY: valid document handle.
        let n = unsafe { (api.FPDF_GetPageCount)(fd.doc) }.max(0);
        for i in 0..n {
            // SAFETY: index in range; pages are closed in Drop.
            let page = unsafe { (api.FPDF_LoadPage)(fd.doc, i) };
            if page.is_null() {
                return Err(fail("form.page", "A page of the document could not be read."));
            }
            unsafe { (api.FORM_OnAfterLoadPage)(page, fd.form) };
            fd.pages.push(page);
        }
        Ok(fd)
    }

    fn has_xfa(&self) -> bool {
        // FORMTYPE_XFA_FULL = 2, FORMTYPE_XFA_FOREGROUND = 3 (reported even by non-XFA builds).
        // SAFETY: valid document handle.
        matches!(unsafe { (self.api.FPDF_GetFormType)(self.doc) }, 2 | 3)
    }

    /// A dynamic XFA form: its standard fields (if any) are not the form people see.
    fn is_full_xfa(&self) -> bool {
        // SAFETY: valid document handle.
        unsafe { (self.api.FPDF_GetFormType)(self.doc) == 2 }
    }

    /// Visit every widget annotation; the handle is valid only during the callback.
    fn for_each_widget(&self, mut f: impl FnMut(u32, FPDF_PAGE, FPDF_ANNOTATION) -> PwResult<bool>) -> PwResult<bool> {
        let mut seen = 0usize;
        for (pi, &page) in self.pages.iter().enumerate() {
            // SAFETY: page handles are valid until Drop.
            let count = unsafe { (self.api.FPDFPage_GetAnnotCount)(page) };
            for ai in 0..count.max(0) {
                let annot = unsafe { (self.api.FPDFPage_GetAnnot)(page, ai) };
                if annot.is_null() {
                    continue;
                }
                let is_widget = unsafe { (self.api.FPDFAnnot_GetSubtype)(annot) } == ANNOT_WIDGET;
                let r = if is_widget {
                    seen += 1;
                    if seen > MAX_WIDGETS {
                        unsafe { (self.api.FPDFPage_CloseAnnot)(annot) };
                        return Ok(true);
                    }
                    f(pi as u32, page, annot)
                } else {
                    Ok(true)
                };
                unsafe { (self.api.FPDFPage_CloseAnnot)(annot) };
                if !r? {
                    return Ok(false);
                }
            }
        }
        Ok(false)
    }

    fn wide(&self, getter: impl Fn(*mut FPDF_WCHAR, c_ulong) -> c_ulong) -> Option<String> {
        let n = getter(std::ptr::null_mut(), 0);
        if n < 2 {
            return None;
        }
        let mut buf = vec![0u16; (n as usize).div_ceil(2)];
        let got = getter(buf.as_mut_ptr(), (buf.len() * 2) as c_ulong);
        if got < 2 {
            return None;
        }
        let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        Some(String::from_utf16_lossy(&buf[..len]))
    }

    fn name(&self, annot: FPDF_ANNOTATION) -> Option<String> {
        // SAFETY: buffer pointer/length pairs come from `wide`.
        self.wide(|b, l| unsafe { (self.api.FPDFAnnot_GetFormFieldName)(self.form, annot, b, l) })
    }
    /// Field value with line breaks normalized to `\n` (PDFium stores CR LF for multiline text).
    fn label(&self, annot: FPDF_ANNOTATION) -> Option<String> {
        self.wide(|b, l| unsafe { (self.api.FPDFAnnot_GetFormFieldAlternateName)(self.form, annot, b, l) })
            .filter(|s| !s.trim().is_empty())
    }
    fn value(&self, annot: FPDF_ANNOTATION) -> Option<String> {
        self.wide(|b, l| unsafe { (self.api.FPDFAnnot_GetFormFieldValue)(self.form, annot, b, l) })
            .map(|v| v.replace("\r\n", "\n").replace('\r', "\n"))
    }
    fn export_value(&self, annot: FPDF_ANNOTATION) -> Option<String> {
        self.wide(|b, l| unsafe { (self.api.FPDFAnnot_GetFormFieldExportValue)(self.form, annot, b, l) })
    }
    fn kind(&self, annot: FPDF_ANNOTATION) -> FieldKind {
        match unsafe { (self.api.FPDFAnnot_GetFormFieldType)(self.form, annot) } {
            FIELD_TEXTFIELD => FieldKind::Text,
            FIELD_CHECKBOX => FieldKind::Checkbox,
            FIELD_RADIOBUTTON => FieldKind::Radio,
            FIELD_COMBOBOX => FieldKind::Combo,
            FIELD_LISTBOX => FieldKind::List,
            FIELD_PUSHBUTTON => FieldKind::PushButton,
            FIELD_SIGNATURE => FieldKind::Signature,
            _ => FieldKind::Unknown,
        }
    }
    fn flags(&self, annot: FPDF_ANNOTATION) -> c_int {
        unsafe { (self.api.FPDFAnnot_GetFormFieldFlags)(self.form, annot) }
    }
    fn checked(&self, annot: FPDF_ANNOTATION) -> bool {
        unsafe { (self.api.FPDFAnnot_IsChecked)(self.form, annot) != 0 }
    }
    fn options(&self, annot: FPDF_ANNOTATION) -> Vec<String> {
        let n = unsafe { (self.api.FPDFAnnot_GetOptionCount)(self.form, annot) }.clamp(0, 10_000);
        (0..n)
            .map(|i| {
                self.wide(|b, l| unsafe { (self.api.FPDFAnnot_GetOptionLabel)(self.form, annot, i, b, l) })
                    .unwrap_or_default()
            })
            .collect()
    }
    fn rect(&self, annot: FPDF_ANNOTATION) -> Option<RectPt> {
        let mut r = FS_RECTF { left: 0.0, top: 0.0, right: 0.0, bottom: 0.0 };
        if unsafe { (self.api.FPDFAnnot_GetRect)(annot, &mut r) } == 0 {
            return None;
        }
        // PDF rectangles may be stored with any corner order.
        Some(RectPt {
            left: r.left.min(r.right),
            bottom: r.bottom.min(r.top),
            right: r.left.max(r.right),
            top: r.bottom.max(r.top),
        })
    }

    fn fields(&self) -> PwResult<(Vec<FormField>, bool)> {
        let mut order: Vec<String> = Vec::new();
        let mut by_name: HashMap<String, FormField> = HashMap::new();
        let truncated = self.for_each_widget(|page, _p, annot| {
            let Some(name) = self.name(annot) else { return Ok(true) };
            let kind = self.kind(annot);
            let flags = self.flags(annot);
            let widget = FormWidget {
                page,
                rect: self.rect(annot).unwrap_or(RectPt { left: 0.0, bottom: 0.0, right: 0.0, top: 0.0 }),
                export_value: matches!(kind, FieldKind::Checkbox | FieldKind::Radio)
                    .then(|| self.export_value(annot))
                    .flatten(),
                checked: matches!(kind, FieldKind::Checkbox | FieldKind::Radio) && self.checked(annot),
            };
            let entry = by_name.entry(name.clone()).or_insert_with(|| {
                order.push(name.clone());
                FormField {
                    name: name.clone(),
                    label: self.label(annot),
                    kind,
                    value: self.value(annot),
                    options: if matches!(kind, FieldKind::Combo | FieldKind::List) {
                        self.options(annot)
                    } else {
                        Vec::new()
                    },
                    read_only: flags & FLAG_READ_ONLY != 0,
                    required: flags & FLAG_REQUIRED != 0,
                    multiline: kind == FieldKind::Text && flags & FLAG_MULTILINE != 0,
                    editable: kind == FieldKind::Combo && flags & FLAG_EDIT != 0,
                    default_value: None,
                    widgets: Vec::new(),
                }
            });
            entry.widgets.push(widget);
            Ok(true)
        })?;
        Ok((order.into_iter().filter_map(|n| by_name.remove(&n)).collect(), truncated))
    }

    fn click(&self, page: FPDF_PAGE, annot: FPDF_ANNOTATION) {
        if let Some(r) = self.rect(annot) {
            let (x, y) = (f64::from((r.left + r.right) / 2.0), f64::from((r.bottom + r.top) / 2.0));
            // SAFETY: valid handles; coordinates in page space.
            unsafe {
                (self.api.FORM_OnMouseMove)(self.form, page, 0, x, y);
                (self.api.FORM_OnLButtonDown)(self.form, page, 0, x, y);
                (self.api.FORM_OnLButtonUp)(self.form, page, 0, x, y);
                (self.api.FORM_ForceToKillFocus)(self.form);
            }
        }
    }

    fn replace_text(&self, page: FPDF_PAGE, annot: FPDF_ANNOTATION, text: &str) -> bool {
        let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
        // SAFETY: valid handles; `wide` is NUL-terminated and outlives the call.
        unsafe {
            if (self.api.FORM_SetFocusedAnnot)(self.form, annot) == 0 {
                return false;
            }
            (self.api.FORM_SelectAllText)(self.form, page);
            (self.api.FORM_ReplaceSelection)(self.form, page, wide.as_ptr());
            (self.api.FORM_ForceToKillFocus)(self.form) != 0
        }
    }

    fn select_index(&self, page: FPDF_PAGE, annot: FPDF_ANNOTATION, index: usize, multi_options: usize) -> bool {
        // SAFETY: valid handles; index within the option count.
        unsafe {
            if (self.api.FORM_SetFocusedAnnot)(self.form, annot) == 0 {
                return false;
            }
            // List boxes may allow several selections: clear the others first.
            for i in 0..multi_options {
                if i != index {
                    (self.api.FORM_SetIndexSelected)(self.form, page, i as c_int, 0);
                }
            }
            let ok = (self.api.FORM_SetIndexSelected)(self.form, page, index as c_int, 1) != 0;
            (self.api.FORM_ForceToKillFocus)(self.form);
            ok
        }
    }

    fn save(&self) -> PwResult<Vec<u8>> {
        let mut w = Writer::new();
        // Leave the form environment's focus so every pending edit is committed to the document.
        unsafe { (self.api.FORM_ForceToKillFocus)(self.form) };
        // SAFETY: valid document; `w` outlives the call. Flags 0 = full (non-incremental) save.
        if unsafe { (self.api.FPDF_SaveAsCopy)(self.doc, &mut w.base, 0) } == 0 {
            return Err(fail("form.save", "The filled form could not be saved."));
        }
        Ok(w.out)
    }
}

impl Drop for FormDoc<'_> {
    fn drop(&mut self) {
        // SAFETY: teardown in the order PDFium requires: pages (with form notification), form
        // environment, document. Handles are not used afterwards.
        unsafe {
            for &page in &self.pages {
                if !self.form.is_null() {
                    (self.api.FORM_OnBeforeClosePage)(page, self.form);
                }
                (self.api.FPDF_ClosePage)(page);
            }
            if !self.form.is_null() {
                (self.api.FPDFDOC_ExitFormFillEnvironment)(self.form);
            }
            (self.api.FPDF_CloseDocument)(self.doc);
        }
    }
}

/// Fields of the document's AcroForm (empty when it has none).
pub fn form_fields(path: &Path, password: Option<&str>) -> PwResult<FormInfo> {
    let (_exclusive, lib) = engine_exclusive()?;
    let api = api(lib)?;
    let fd = FormDoc::open(api, path, password)?;
    let (mut fields, truncated) = fd.fields()?;
    let has_xfa = fd.has_xfa();
    let xfa_only = fd.is_full_xfa() || (has_xfa && fields.is_empty());
    drop(fd);
    if !fields.is_empty() {
        let mut defaults = crate::formdefaults::field_defaults(path, password);
        for f in &mut fields {
            f.default_value = defaults.remove(&f.name);
        }
    }
    Ok(FormInfo { fields, has_xfa, xfa_only, truncated })
}

/// Fill the form in memory (exclusive engine access, released before returning) and return the
/// saved bytes. Every request is checked against the form before anything changes.
fn fill_bytes(input: &Path, password: Option<&str>, values: &[FieldValue]) -> PwResult<Vec<u8>> {
    let (_exclusive, lib) = engine_exclusive()?;
    let api = api(lib)?;
    let fd = FormDoc::open(api, input, password)?;
    let (fields, _) = fd.fields()?;
    let hybrid = fd.has_xfa();
    if fd.is_full_xfa() || (hybrid && fields.is_empty()) {
        return Err(PwError::new(
            ErrorCategory::UnsupportedOperation,
            "form.xfa",
            "This is a dynamic XFA form, which PageWeave cannot fill.",
        ));
    }
    let by_name: HashMap<&str, &FormField> = fields.iter().map(|f| (f.name.as_str(), f)).collect();
    for v in values {
        let f = by_name.get(v.name.as_str()).ok_or_else(|| {
            PwError::invalid("form.unknown_field", format!("The form has no field named “{}”.", v.name))
        })?;
        check_value(f, &v.value)?;
    }
    let wanted: HashMap<&str, &FormValue> = values.iter().map(|v| (v.name.as_str(), &v.value)).collect();
    let mut done: HashSet<String> = HashSet::new();
    let mut failed: Option<String> = None;
    fd.for_each_widget(|_pi, page, annot| {
        let Some(name) = fd.name(annot) else { return Ok(true) };
        let Some(value) = wanted.get(name.as_str()) else { return Ok(true) };
        let field = by_name[name.as_str()];
        let ok = match (field.kind, value) {
            (FieldKind::Text, FormValue::Text(t)) => done.contains(&name) || fd.replace_text(page, annot, t),
            (FieldKind::Checkbox, FormValue::Checked(want)) => {
                if !done.contains(&name) && fd.checked(annot) != *want {
                    fd.click(page, annot);
                }
                true
            }
            (FieldKind::Radio, FormValue::Text(want)) => {
                if fd.export_value(annot).as_deref() == Some(want.as_str()) && !fd.checked(annot) {
                    fd.click(page, annot);
                }
                true
            }
            (FieldKind::Combo | FieldKind::List, FormValue::Text(t)) => {
                done.contains(&name)
                    || match field.options.iter().position(|o| o == t) {
                        Some(i) => fd.select_index(
                            page,
                            annot,
                            i,
                            if field.kind == FieldKind::List { field.options.len() } else { 0 },
                        ),
                        None => fd.replace_text(page, annot, t), // editable combo (checked above)
                    }
            }
            _ => false,
        };
        if !ok {
            failed = Some(name.clone());
            return Ok(false);
        }
        done.insert(name);
        Ok(true)
    })?;
    if let Some(name) = failed {
        return Err(fail("form.fill", format!("The field “{name}” could not be filled.")));
    }
    let bytes = fd.save()?;
    drop(fd);
    if hybrid {
        drop_xfa(&bytes)
    } else {
        Ok(bytes)
    }
}

/// A filled hybrid form without its XFA layer (D-038): the XFA data still holds the old values,
/// and XFA-aware viewers would show those instead of the standard fields that were filled.
fn drop_xfa(bytes: &[u8]) -> PwResult<Vec<u8>> {
    let broken = || fail("form.save", "The filled form could not be saved.");
    let mut doc = lopdf::Document::load_mem(bytes).map_err(|_| broken())?;
    let root = doc.trailer.get(b"Root").and_then(|o| o.as_reference()).map_err(|_| broken())?;
    let acro = {
        let catalog = doc.get_object_mut(root).and_then(|o| o.as_dict_mut()).map_err(|_| broken())?;
        catalog.remove(b"NeedsRendering");
        catalog.get(b"AcroForm").ok().cloned()
    };
    match acro {
        Some(lopdf::Object::Reference(id)) => {
            if let Ok(d) = doc.get_object_mut(id).and_then(|o| o.as_dict_mut()) {
                d.remove(b"XFA");
            }
        }
        Some(lopdf::Object::Dictionary(mut d)) => {
            d.remove(b"XFA");
            let catalog = doc.get_object_mut(root).and_then(|o| o.as_dict_mut()).map_err(|_| broken())?;
            catalog.set("AcroForm", lopdf::Object::Dictionary(d));
        }
        _ => {}
    }
    doc.prune_objects();
    let mut out = Vec::new();
    doc.save_to(&mut out).map_err(|_| broken())?;
    Ok(out)
}

/// Apply `values` to a copy of `input` and commit it to `output`. The source is not modified. The
/// staged copy is reopened and every requested value is checked before it is committed.
pub fn fill_form(
    input: &Path,
    password: Option<&str>,
    values: &[FieldValue],
    output: &Path,
    policy: CollisionPolicy,
) -> PwResult<(CommittedOutput, Vec<String>)> {
    apply_edits(input, password, values, &[], output, policy)
}

/// Apply form values and annotations to one copy of `input`, then verify both in the staged output
/// (field values re-read; annotation counts per touched page) before committing it atomically.
pub fn apply_edits(
    input: &Path,
    password: Option<&str>,
    values: &[FieldValue],
    annotations: &[AnnotationSpec],
    output: &Path,
    policy: CollisionPolicy,
) -> PwResult<(CommittedOutput, Vec<String>)> {
    apply_edits_protecting(input, password, values, annotations, output, policy, &[input.to_path_buf()])
}

/// [`apply_edits`] where `protected` lists every file the output may never replace (the input and
/// any other file the operation read, such as a form data file).
pub(crate) fn apply_edits_protecting(
    input: &Path,
    password: Option<&str>,
    values: &[FieldValue],
    annotations: &[AnnotationSpec],
    output: &Path,
    policy: CollisionPolicy,
    protected: &[std::path::PathBuf],
) -> PwResult<(CommittedOutput, Vec<String>)> {
    let filled = if values.is_empty() { None } else { Some(fill_bytes(input, password, values)?) };
    let (bytes, expected) = if annotations.is_empty() {
        match filled {
            Some(b) => (b, Default::default()),
            None => return Err(PwError::invalid("edits.empty", "There are no changes to save.")),
        }
    } else {
        let pdfium = engine()?;
        let doc = match &filled {
            Some(b) => pdfium.load_pdf_from_byte_slice(b, password).map_err(|e| map_pdfium_error(&e, "open"))?,
            None => load(&pdfium, input, &OpenOptions { password })?,
        };
        let expected = apply_annotations(&doc, annotations)?;
        (save_bytes(&doc)?, expected)
    };

    let mut out_encrypted = false;
    let committed = write_atomic_bytes(output, policy, protected, &bytes, |staged| {
        // Form check first: it needs exclusive engine access, so no shared access may be held here.
        if !values.is_empty() {
            let info = match form_fields(staged, None) {
                Ok(i) => i,
                Err(e) if e.category == ErrorCategory::PasswordRequired && password.is_some() => {
                    out_encrypted = true;
                    form_fields(staged, password).map_err(|_| verify_failed("could not be reopened"))?
                }
                Err(_) => return Err(verify_failed("could not be reopened")),
            };
            let by_name: HashMap<&str, &FormField> = info.fields.iter().map(|f| (f.name.as_str(), f)).collect();
            for v in values {
                let f = by_name.get(v.name.as_str()).ok_or_else(|| verify_failed("a field is missing"))?;
                if !value_matches(f, &v.value) {
                    return Err(verify_failed(&format!("“{}” did not take the new value", v.name)));
                }
            }
        }
        if !annotations.is_empty() {
            let pdfium = engine()?;
            if password.is_some() && pdfium.load_pdf_from_file(staged, None).is_err() {
                out_encrypted = true;
            }
            verify_annotation_counts(&pdfium, staged, password, &expected)?;
        }
        Ok(())
    })?;
    Ok((committed, protection_warnings(password.is_some(), out_encrypted)))
}

fn verify_failed(what: &str) -> PwError {
    PwError::new(
        ErrorCategory::ValidationFailed,
        "output.verify",
        format!("The output failed verification ({what}) and was not saved."),
    )
}

fn check_value(f: &FormField, v: &FormValue) -> PwResult<()> {
    let wrong = |what: &str| PwError::invalid("form.value_type", format!("“{}” {what}.", f.name));
    if f.read_only {
        return Err(PwError::invalid("form.read_only", format!("“{}” is read-only.", f.name)));
    }
    match (f.kind, v) {
        (FieldKind::Text, FormValue::Text(_)) => Ok(()),
        (FieldKind::Checkbox, FormValue::Checked(_)) => Ok(()),
        (FieldKind::Radio, FormValue::Text(t)) => {
            if f.widgets.iter().any(|w| w.export_value.as_deref() == Some(t.as_str())) {
                Ok(())
            } else {
                Err(wrong("has no such option"))
            }
        }
        (FieldKind::Combo, FormValue::Text(t)) if f.editable || f.options.iter().any(|o| o == t) => Ok(()),
        (FieldKind::List, FormValue::Text(t)) if f.options.iter().any(|o| o == t) => Ok(()),
        (FieldKind::Combo | FieldKind::List, FormValue::Text(_)) => Err(wrong("has no such option")),
        (FieldKind::Checkbox, _) => Err(wrong("needs true or false")),
        (FieldKind::Text | FieldKind::Radio | FieldKind::Combo | FieldKind::List, _) => Err(wrong("needs text")),
        (FieldKind::PushButton | FieldKind::Signature | FieldKind::Unknown, _) => {
            Err(PwError::invalid("form.unsupported_field", format!("“{}” cannot be filled.", f.name)))
        }
    }
}

fn value_matches(f: &FormField, v: &FormValue) -> bool {
    match (f.kind, v) {
        (FieldKind::Checkbox, FormValue::Checked(want)) => f.widgets.iter().any(|w| w.checked) == *want,
        (FieldKind::Radio, FormValue::Text(t)) => {
            f.widgets.iter().any(|w| w.checked && w.export_value.as_deref() == Some(t.as_str()))
        }
        (_, FormValue::Text(t)) => f.value.as_deref().unwrap_or("") == t,
        _ => false,
    }
}
