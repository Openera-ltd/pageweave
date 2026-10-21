//! Accessibility check (PRD 05 "Compare and accessibility"): what can be checked automatically in
//! a PDF's structure — tags, language, title, alternate text, table headers, headings, form labels,
//! link descriptions, tab order, embedded fonts, a text layer — and what a person has to review.
//!
//! Read-only. Each check reports pass / fail / warning / not applicable, or "manual" for items no
//! program can judge (reading order, whether alternate text is meaningful, colour contrast). This
//! is not a PDF/UA or WCAG conformance claim.

use crate::document::{load, page_texts, OpenOptions};
use crate::engine;
use lopdf::{Dictionary, Document, Object, ObjectId};
use pdfium_render::prelude::*;
use pw_core::{ErrorCategory, PwError, PwResult};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::Path;

/// Upper bounds for hostile files.
const MAX_NODES: usize = 200_000;
const MAX_DEPTH: usize = 256;
/// Pages listed per check.
const MAX_PAGES_LISTED: usize = 50;
/// Names listed per check (fonts, fields).
const MAX_NAMES_LISTED: usize = 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    Pass,
    Fail,
    Warning,
    /// A person must review this; no automatic verdict.
    Manual,
    NotApplicable,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AccessibilityCheck {
    pub id: String,
    pub title: String,
    pub status: CheckStatus,
    pub detail: String,
    /// 0-based pages the finding concerns (first few).
    pub pages: Vec<u32>,
    /// Font or field names concerned (first few).
    pub names: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AccessibilityReport {
    pub tagged: bool,
    pub checks: Vec<AccessibilityCheck>,
    pub passed: usize,
    pub failed: usize,
    pub warnings: usize,
    pub manual: usize,
}

fn check(id: &str, title: &str, status: CheckStatus, detail: impl Into<String>) -> AccessibilityCheck {
    AccessibilityCheck {
        id: id.into(),
        title: title.into(),
        status,
        detail: detail.into(),
        pages: vec![],
        names: vec![],
    }
}

fn text(o: &Object) -> String {
    lopdf::decode_text_string(o).unwrap_or_else(|_| match o {
        Object::String(b, _) => b.iter().map(|&c| c as char).collect(),
        _ => String::new(),
    })
}

fn deref<'a>(doc: &'a Document, o: &'a Object) -> Option<&'a Object> {
    match o {
        Object::Reference(r) => doc.get_object(*r).ok(),
        other => Some(other),
    }
}

fn dict<'a>(doc: &'a Document, o: &'a Object) -> Option<&'a Dictionary> {
    deref(doc, o).and_then(|o| o.as_dict().ok())
}

fn get<'a>(doc: &'a Document, d: &'a Dictionary, key: &[u8]) -> Option<&'a Object> {
    d.get(key).ok().and_then(|o| deref(doc, o))
}

fn name(o: Option<&Object>) -> Option<String> {
    o.and_then(|o| o.as_name().ok()).map(|n| String::from_utf8_lossy(n).into_owned())
}

fn nonempty_text(doc: &Document, d: &Dictionary, key: &[u8]) -> bool {
    get(doc, d, key).map(|o| !text(o).trim().is_empty()).unwrap_or(false)
}

struct Elem {
    role: String,
    page: Option<u32>,
    alt: bool,
    /// Index of the parent element in the list, if any.
    parent: Option<usize>,
}

/// Every structure element (bounded), roles resolved through the role map.
fn struct_elems(doc: &Document, root: &Dictionary, page_index: &HashMap<ObjectId, u32>) -> (Vec<Elem>, bool) {
    let mut role_map: HashMap<String, String> = HashMap::new();
    if let Some(rm) = get(doc, root, b"RoleMap").and_then(|o| o.as_dict().ok()) {
        for (k, v) in rm.iter() {
            if let Some(n) = name(deref(doc, v)) {
                role_map.insert(String::from_utf8_lossy(k).into_owned(), n);
            }
        }
    }
    let resolve = |mut r: String| {
        for _ in 0..10 {
            match role_map.get(&r) {
                Some(n) if *n != r => r = n.clone(),
                _ => break,
            }
        }
        r
    };
    let mut out = Vec::new();
    let mut seen: HashSet<ObjectId> = HashSet::new();
    let mut truncated = false;
    // (kid object, parent element index, inherited page, depth)
    let mut stack: Vec<(Object, Option<usize>, Option<u32>, usize)> = Vec::new();
    if let Ok(k) = root.get(b"K") {
        stack.push((k.clone(), None, None, 0));
    }
    while let Some((o, parent, page, depth)) = stack.pop() {
        if out.len() >= MAX_NODES || depth > MAX_DEPTH {
            truncated = true;
            break;
        }
        let o = match &o {
            Object::Reference(r) => {
                if !seen.insert(*r) {
                    continue;
                }
                match doc.get_object(*r) {
                    Ok(x) => x.clone(),
                    Err(_) => continue,
                }
            }
            other => other.clone(),
        };
        match o {
            Object::Array(items) => {
                for i in items.into_iter().rev() {
                    stack.push((i, parent, page, depth + 1));
                }
            }
            Object::Dictionary(d) => {
                // Marked-content and object references are leaves.
                if matches!(name(d.get(b"Type").ok()).as_deref(), Some("MCR" | "OBJR")) {
                    continue;
                }
                let Some(role) = name(get(doc, &d, b"S")) else { continue };
                let pg = d
                    .get(b"Pg")
                    .ok()
                    .and_then(|p| p.as_reference().ok())
                    .and_then(|r| page_index.get(&r).copied())
                    .or(page);
                let alt = nonempty_text(doc, &d, b"Alt") || nonempty_text(doc, &d, b"ActualText");
                out.push(Elem { role: resolve(role), page: pg, alt, parent });
                let me = out.len() - 1;
                if let Ok(k) = d.get(b"K") {
                    stack.push((k.clone(), Some(me), pg, depth + 1));
                }
            }
            _ => {}
        }
    }
    (out, truncated)
}

fn pages_of<'a>(it: impl Iterator<Item = Option<u32>> + 'a) -> Vec<u32> {
    let set: BTreeSet<u32> = it.flatten().collect();
    set.into_iter().take(MAX_PAGES_LISTED).collect()
}

/// Whether a font dictionary carries its font program (Type 3 fonts are their own glyphs).
fn font_embedded(doc: &Document, f: &Dictionary) -> bool {
    let sub = name(get(doc, f, b"Subtype"));
    if sub.as_deref() == Some("Type3") {
        return true;
    }
    let desc_holder = if sub.as_deref() == Some("Type0") {
        get(doc, f, b"DescendantFonts")
            .and_then(|a| a.as_array().ok())
            .and_then(|a| a.first())
            .and_then(|d| dict(doc, d))
    } else {
        Some(f)
    };
    desc_holder
        .and_then(|d| get(doc, d, b"FontDescriptor"))
        .and_then(|o| o.as_dict().ok())
        .map(|fd| fd.has(b"FontFile") || fd.has(b"FontFile2") || fd.has(b"FontFile3"))
        .unwrap_or(false)
}

/// Fonts used by a page's resources and the form XObjects they draw (bounded depth).
fn page_fonts(
    doc: &Document,
    res: &Dictionary,
    depth: usize,
    out: &mut Vec<(String, bool)>,
    seen: &mut HashSet<Vec<u8>>,
) {
    if depth > 8 {
        return;
    }
    if let Some(fonts) = get(doc, res, b"Font").and_then(|o| o.as_dict().ok()) {
        for (_, f) in fonts.iter() {
            if let Some(fd) = dict(doc, f) {
                let base = name(get(doc, fd, b"BaseFont")).unwrap_or_else(|| "(unnamed font)".into());
                // Subset tags (ABCDEF+Name) are not part of the name people know.
                let shown = match base.split_once('+') {
                    Some((tag, rest)) if tag.len() == 6 && tag.chars().all(|c| c.is_ascii_uppercase()) => {
                        rest.to_string()
                    }
                    _ => base,
                };
                if seen.insert(shown.clone().into_bytes()) {
                    out.push((shown, font_embedded(doc, fd)));
                }
            }
        }
    }
    if let Some(xs) = get(doc, res, b"XObject").and_then(|o| o.as_dict().ok()) {
        for (_, x) in xs.iter() {
            let Some(Object::Stream(s)) = deref(doc, x) else { continue };
            if name(s.dict.get(b"Subtype").ok()).as_deref() == Some("Form") {
                if let Some(r) = get(doc, &s.dict, b"Resources").and_then(|o| o.as_dict().ok()) {
                    page_fonts(doc, r, depth + 1, out, seen);
                }
            }
        }
    }
}

fn manual_checks() -> Vec<AccessibilityCheck> {
    vec![
        check(
            "reading_order",
            "Reading order",
            CheckStatus::Manual,
            "Check that the content is read in a sensible order, for example with a screen reader.",
        ),
        check(
            "alt_text_quality",
            "Alternate text says what matters",
            CheckStatus::Manual,
            "Check that each picture's description tells a reader what it shows or means.",
        ),
        check(
            "contrast",
            "Colour contrast",
            CheckStatus::Manual,
            "Check that text stands out from its background (WCAG 2 asks for 4.5:1 for body text).",
        ),
        check(
            "color_only",
            "Meaning not by colour alone",
            CheckStatus::Manual,
            "Check that nothing is conveyed only by colour, such as red for overdue.",
        ),
    ]
}

pub fn check_accessibility(input: &Path, opts: &OpenOptions) -> PwResult<AccessibilityReport> {
    let pdfium = engine()?;
    let src = load(&pdfium, input, opts)?;
    let page_count = src.pages().len().max(0) as u32;
    let protected =
        !matches!(src.permissions().security_handler_revision(), Ok(PdfSecurityHandlerRevision::Unprotected));
    drop(src);
    drop(pdfium);
    // Structure is read with lopdf; protected files are read through PDFium's decryption.
    let bytes = if protected {
        crate::forms::save_without_security(input, opts.password)?
    } else {
        std::fs::read(input).map_err(|e| PwError::from_io(&e, "file"))?
    };
    let doc = Document::load_mem(&bytes).map_err(|_| {
        PwError::new(
            ErrorCategory::Corrupt,
            "accessibility.parse",
            "The document's structure could not be read for checking.",
        )
    })?;
    let catalog = doc.catalog().map_err(|_| {
        PwError::new(
            ErrorCategory::Corrupt,
            "accessibility.parse",
            "The document's structure could not be read for checking.",
        )
    })?;
    let pages = doc.get_pages();
    let page_index: HashMap<ObjectId, u32> = pages.iter().map(|(n, id)| (*id, n - 1)).collect();
    let mut checks = Vec::new();

    // Tags.
    let marked = get(&doc, catalog, b"MarkInfo")
        .and_then(|o| o.as_dict().ok())
        .and_then(|m| m.get(b"Marked").ok())
        .and_then(|b| b.as_bool().ok())
        .unwrap_or(false);
    let root = get(&doc, catalog, b"StructTreeRoot").and_then(|o| o.as_dict().ok());
    let tagged = marked && root.is_some();
    checks.push(if tagged {
        check(
            "tagged",
            "Tagged PDF",
            CheckStatus::Pass,
            "The document has a structure tree that assistive technology can follow.",
        )
    } else {
        check(
            "tagged",
            "Tagged PDF",
            CheckStatus::Fail,
            "The document is not tagged, so screen readers have to guess its structure and reading order.",
        )
    });

    // Title and language.
    let info_title = doc
        .trailer
        .get(b"Info")
        .ok()
        .and_then(|o| dict(&doc, o))
        .map(|i| nonempty_text(&doc, i, b"Title"))
        .unwrap_or(false);
    let xmp_title = get(&doc, catalog, b"Metadata")
        .and_then(|o| o.as_stream().ok())
        .and_then(|s| s.decompressed_content().ok().or_else(|| Some(s.content.clone())))
        .map(|x| {
            let x = String::from_utf8_lossy(&x).into_owned();
            x.find("<dc:title")
                .is_some_and(|i| x[i..].find("</dc:title>").is_some_and(|j| x[i..i + j].contains("<rdf:li")))
        })
        .unwrap_or(false);
    checks.push(if info_title || xmp_title {
        check("title", "Document title", CheckStatus::Pass, "The document has a title.")
    } else {
        check(
            "title",
            "Document title",
            CheckStatus::Fail,
            "The document has no title (set one in Edit document properties).",
        )
    });
    let shows_title = get(&doc, catalog, b"ViewerPreferences")
        .and_then(|o| o.as_dict().ok())
        .and_then(|v| v.get(b"DisplayDocTitle").ok())
        .and_then(|b| b.as_bool().ok())
        .unwrap_or(false);
    checks.push(if shows_title {
        check(
            "title_shown",
            "Title shown in the window",
            CheckStatus::Pass,
            "Readers show the title instead of the file name.",
        )
    } else {
        check(
            "title_shown",
            "Title shown in the window",
            CheckStatus::Warning,
            "Readers show the file name instead of the title (DisplayDocTitle is not set).",
        )
    });
    let lang = get(&doc, catalog, b"Lang").map(text).unwrap_or_default();
    checks.push(if lang.trim().is_empty() {
        check(
            "language",
            "Document language",
            CheckStatus::Fail,
            "No language is set, so screen readers may pronounce the text wrongly.",
        )
    } else {
        check("language", "Document language", CheckStatus::Pass, format!("Language: {}.", lang.trim()))
    });

    // A text layer on every page.
    let texts = page_texts(input, opts, &(0..page_count).collect::<Vec<_>>())?;
    let no_text: Vec<u32> = texts.iter().filter(|(_, t)| t.trim().is_empty()).map(|(p, _)| *p).collect();
    let mut c = if no_text.is_empty() {
        check(
            "text",
            "Text on every page",
            CheckStatus::Pass,
            "Every page has text that can be read aloud and searched.",
        )
    } else {
        check(
            "text",
            "Text on every page",
            CheckStatus::Fail,
            format!("{} page(s) have no text (likely scans or pictures of text); recognize text first.", no_text.len()),
        )
    };
    c.pages = no_text.into_iter().take(MAX_PAGES_LISTED).collect();
    checks.push(c);

    // Fonts.
    let mut fonts = Vec::new();
    let mut seen = HashSet::new();
    for id in pages.values() {
        if let Ok(pd) = doc.get_dictionary(*id) {
            if let Some(res) = get(&doc, pd, b"Resources").and_then(|o| o.as_dict().ok()) {
                page_fonts(&doc, res, 0, &mut fonts, &mut seen);
            }
        }
    }
    let missing: Vec<String> = fonts.iter().filter(|(_, e)| !e).map(|(n, _)| n.clone()).collect();
    let mut c = if fonts.is_empty() {
        check("fonts_embedded", "Fonts embedded", CheckStatus::NotApplicable, "The document uses no fonts.")
    } else if missing.is_empty() {
        check("fonts_embedded", "Fonts embedded", CheckStatus::Pass, "Every font is embedded.")
    } else {
        check(
            "fonts_embedded",
            "Fonts embedded",
            CheckStatus::Warning,
            format!("{} font(s) are not embedded; text may look or read differently elsewhere (PDF/UA requires embedded fonts).", missing.len()),
        )
    };
    c.names = missing.into_iter().take(MAX_NAMES_LISTED).collect();
    checks.push(c);

    // Structure-based checks.
    if let (true, Some(root)) = (tagged, root) {
        let (elems, truncated) = struct_elems(&doc, root, &page_index);
        let figures: Vec<&Elem> = elems.iter().filter(|e| e.role == "Figure").collect();
        let no_alt: Vec<&&Elem> = figures.iter().filter(|e| !e.alt).collect();
        let mut c = if figures.is_empty() {
            check("figures_alt", "Pictures have alternate text", CheckStatus::NotApplicable, "No tagged pictures.")
        } else if no_alt.is_empty() {
            check(
                "figures_alt",
                "Pictures have alternate text",
                CheckStatus::Pass,
                format!("All {} tagged picture(s) have alternate text.", figures.len()),
            )
        } else {
            check(
                "figures_alt",
                "Pictures have alternate text",
                CheckStatus::Fail,
                format!("{} of {} tagged picture(s) have no alternate text.", no_alt.len(), figures.len()),
            )
        };
        c.pages = pages_of(no_alt.iter().map(|e| e.page));
        checks.push(c);

        let tables: Vec<usize> = elems.iter().enumerate().filter(|(_, e)| e.role == "Table").map(|(i, _)| i).collect();
        let has_th = |t: usize| {
            elems.iter().any(|e| {
                if e.role != "TH" {
                    return false;
                }
                // Walk up to see whether this header belongs to table `t`.
                let mut p = e.parent;
                for _ in 0..MAX_DEPTH {
                    match p {
                        Some(i) if i == t => return true,
                        Some(i) => p = elems[i].parent,
                        None => return false,
                    }
                }
                false
            })
        };
        let headless: Vec<usize> = tables.iter().copied().filter(|t| !has_th(*t)).collect();
        let mut c = if tables.is_empty() {
            check("table_headers", "Tables have header cells", CheckStatus::NotApplicable, "No tagged tables.")
        } else if headless.is_empty() {
            check(
                "table_headers",
                "Tables have header cells",
                CheckStatus::Pass,
                format!("All {} table(s) have header cells.", tables.len()),
            )
        } else {
            check(
                "table_headers",
                "Tables have header cells",
                CheckStatus::Warning,
                format!(
                    "{} of {} table(s) have no header cells (TH), so cells can't be announced with their headings.",
                    headless.len(),
                    tables.len()
                ),
            )
        };
        c.pages = pages_of(headless.iter().map(|t| elems[*t].page));
        checks.push(c);

        let headings = elems
            .iter()
            .filter(|e| {
                e.role == "H" || (e.role.len() == 2 && e.role.starts_with('H') && e.role[1..].parse::<u8>().is_ok())
            })
            .count();
        checks.push(if headings > 0 {
            check("headings", "Headings", CheckStatus::Pass, format!("{headings} heading(s) tagged for navigation."))
        } else {
            check(
                "headings",
                "Headings",
                CheckStatus::Warning,
                "No headings are tagged, so readers can't jump between sections.",
            )
        });
        if truncated {
            checks.push(check(
                "structure_size",
                "Structure checked completely",
                CheckStatus::Warning,
                "The structure tree is very large or nested very deeply; only part of it was checked.",
            ));
        }
    } else {
        for (id, title) in [
            ("figures_alt", "Pictures have alternate text"),
            ("table_headers", "Tables have header cells"),
            ("headings", "Headings"),
        ] {
            checks.push(check(id, title, CheckStatus::NotApplicable, "Needs a tagged document."));
        }
    }

    // Annotations: form labels, link descriptions, tab order.
    let mut unlabeled: Vec<String> = Vec::new();
    let mut fields = 0usize;
    let mut links = 0usize;
    let mut undescribed: Vec<u32> = Vec::new();
    let mut no_tabs: Vec<u32> = Vec::new();
    for (n, id) in &pages {
        let Ok(pd) = doc.get_dictionary(*id) else { continue };
        let annots: Vec<&Dictionary> = get(&doc, pd, b"Annots")
            .and_then(|o| o.as_array().ok())
            .map(|a| a.iter().filter_map(|x| dict(&doc, x)).collect())
            .unwrap_or_default();
        let mut interactive = false;
        for a in annots.iter().take(MAX_NODES) {
            match name(get(&doc, a, b"Subtype")).as_deref() {
                Some("Widget") => {
                    interactive = true;
                    // The field is the widget itself or its parent (for kids without /T).
                    let field =
                        if a.has(b"T") { Some(*a) } else { get(&doc, a, b"Parent").and_then(|p| p.as_dict().ok()) };
                    let Some(f) = field else { continue };
                    let kind = name(get(&doc, f, b"FT"));
                    let push_button = kind.as_deref() == Some("Btn")
                        && get(&doc, f, b"Ff").and_then(|o| o.as_i64().ok()).is_some_and(|ff| ff & (1 << 16) != 0);
                    fields += 1;
                    if !nonempty_text(&doc, f, b"TU") && !push_button {
                        let nm = get(&doc, f, b"T").map(text).unwrap_or_else(|| "(unnamed)".into());
                        if !unlabeled.contains(&nm) {
                            unlabeled.push(nm);
                        }
                    }
                }
                Some("Link") => {
                    interactive = true;
                    links += 1;
                    if !nonempty_text(&doc, a, b"Contents") {
                        undescribed.push(n - 1);
                    }
                }
                Some("Popup") | None => {}
                Some(_) => interactive = true,
            }
        }
        if interactive && name(get(&doc, pd, b"Tabs")).as_deref() != Some("S") {
            no_tabs.push(n - 1);
        }
    }
    let mut c = if fields == 0 {
        check("form_labels", "Form fields are labelled", CheckStatus::NotApplicable, "No form fields.")
    } else if unlabeled.is_empty() {
        check(
            "form_labels",
            "Form fields are labelled",
            CheckStatus::Pass,
            "Every form field has a label (tooltip) readers announce.",
        )
    } else {
        check(
            "form_labels",
            "Form fields are labelled",
            CheckStatus::Fail,
            format!(
                "{} form field(s) have no label (tooltip), so readers announce only the field type.",
                unlabeled.len()
            ),
        )
    };
    c.names = unlabeled.into_iter().take(MAX_NAMES_LISTED).collect();
    checks.push(c);
    let mut c = if links == 0 {
        check("links_described", "Links are described", CheckStatus::NotApplicable, "No links.")
    } else if undescribed.is_empty() {
        check("links_described", "Links are described", CheckStatus::Pass, "Every link has a description.")
    } else {
        check(
            "links_described",
            "Links are described",
            CheckStatus::Warning,
            format!("{} of {links} link(s) have no description (Contents).", undescribed.len()),
        )
    };
    c.pages = pages_of(undescribed.into_iter().map(Some));
    checks.push(c);
    let mut c = if fields + links == 0 && no_tabs.is_empty() {
        check(
            "tab_order",
            "Tab order follows the structure",
            CheckStatus::NotApplicable,
            "No links, fields or comments.",
        )
    } else if no_tabs.is_empty() {
        check(
            "tab_order",
            "Tab order follows the structure",
            CheckStatus::Pass,
            "Pages with links, fields or comments use the structure order for Tab.",
        )
    } else {
        check(
            "tab_order",
            "Tab order follows the structure",
            CheckStatus::Warning,
            format!(
                "{} page(s) with links, fields or comments don't set the tab order to follow the structure.",
                no_tabs.len()
            ),
        )
    };
    c.pages = no_tabs.into_iter().take(MAX_PAGES_LISTED).collect();
    checks.push(c);

    // Bookmarks for long documents.
    let has_outline = get(&doc, catalog, b"Outlines").and_then(|o| o.as_dict().ok()).is_some_and(|o| o.has(b"First"));
    checks.push(if page_count <= 20 {
        check(
            "bookmarks",
            "Bookmarks for long documents",
            CheckStatus::NotApplicable,
            "The document has 20 pages or fewer.",
        )
    } else if has_outline {
        check("bookmarks", "Bookmarks for long documents", CheckStatus::Pass, "The document has bookmarks.")
    } else {
        check(
            "bookmarks",
            "Bookmarks for long documents",
            CheckStatus::Warning,
            format!("The document has {page_count} pages but no bookmarks to navigate them."),
        )
    });

    checks.extend(manual_checks());
    let count = |s: CheckStatus| checks.iter().filter(|c| c.status == s).count();
    Ok(AccessibilityReport {
        tagged,
        passed: count(CheckStatus::Pass),
        failed: count(CheckStatus::Fail),
        warnings: count(CheckStatus::Warning),
        manual: count(CheckStatus::Manual),
        checks,
    })
}
