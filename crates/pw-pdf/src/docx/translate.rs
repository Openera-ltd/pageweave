//! Translate a document (PRD 08, decision D-022): the pages are laid out as for Word export
//! (reading order, columns, headings, tables, pictures), every paragraph and table cell is
//! translated with the local models, and the result is written as a Word document with the same
//! structure, or as a text file. The engine lock is released while translating. A report says how
//! many numbers, dates, codes and addresses the translation did not keep exactly, and where mixed
//! formatting inside a paragraph had to become one style.

use super::layout::{Item, PageLayout, Paragraph, Run};
use super::{analyze, fail, reader, writer, Analysis};
use crate::document::{load, OpenOptions};
use crate::engine;
use pdfium_render::prelude::*;
use pw_core::output::{write_atomic_bytes, CollisionPolicy, CommittedOutput};
use pw_core::range::parse_page_ranges;
use pw_core::{ErrorCategory, PwError, PwResult};
use serde::Serialize;
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct TranslateReport {
    pub pages: u32,
    pub source: String,
    pub target: String,
    /// The source language was detected (not given).
    pub detected: bool,
    /// Translated through English (no direct model for the pair).
    pub via_english: bool,
    pub paragraphs: usize,
    pub tables: usize,
    pub pictures: usize,
    /// Numbers, dates, codes, web and mail addresses in the source, and how many the
    /// translation kept unchanged.
    pub protected_total: usize,
    pub protected_kept: usize,
    /// Up to 20 of those not kept, for review.
    pub protected_missing: Vec<String>,
    /// Paragraphs whose mixed formatting (a bold word, a link) became one style.
    pub simplified_paragraphs: usize,
    pub millis: u64,
}

fn paragraphs_mut(layouts: &mut [PageLayout]) -> Vec<&mut Paragraph> {
    let mut out = Vec::new();
    for l in layouts.iter_mut() {
        for b in &mut l.bands {
            for c in &mut b.columns {
                for it in &mut c.items {
                    match it {
                        Item::Para(p) => out.push(p),
                        Item::Table { table, .. } => {
                            for cell in &mut table.cells {
                                out.extend(cell.paragraphs.iter_mut());
                            }
                        }
                        Item::Pictures { .. } => {}
                    }
                }
            }
        }
    }
    out
}

fn text_of(p: &Paragraph) -> String {
    p.runs.iter().map(|r| r.text.as_str()).collect::<String>().replace('\t', " ")
}

/// Translate `input` (all pages or `pages`) into `target`. `source` is detected when absent.
/// `output` ending in `.docx` gets a Word document, `.txt` a text file.
#[allow(clippy::too_many_arguments)]
pub fn translate_document(
    input: &Path,
    opts: &OpenOptions,
    pages: Option<&str>,
    source: Option<&str>,
    target: &str,
    output: &Path,
    policy: CollisionPolicy,
    cancelled: &dyn Fn() -> bool,
    progress: &dyn Fn(u64, u64),
) -> PwResult<(CommittedOutput, TranslateReport, Vec<String>)> {
    let started = std::time::Instant::now();
    let word = output.extension().is_some_and(|e| e.eq_ignore_ascii_case("docx"));
    // Layout first, under the engine lock; translating (much slower) happens after it is released.
    let (mut layouts, media, links, title, idx) = {
        let pdfium = engine()?;
        let doc = load(&pdfium, input, opts)?;
        let n = doc.pages().len().max(0) as u32;
        let idx: Vec<u32> = match pages {
            Some(spec) => parse_page_ranges(spec, n)?,
            None => (0..n).collect(),
        };
        if idx.is_empty() {
            return Err(PwError::invalid("translate.no_pages", "Choose at least one page."));
        }
        let title = doc
            .metadata()
            .get(PdfDocumentMetadataTagType::Title)
            .map(|t| t.value().trim().to_string())
            .filter(|t| !t.is_empty());
        let Analysis { layouts, media, links, .. } = analyze(&doc, &idx, cancelled, &|_, _| {})?;
        (layouts, media, links, title, idx)
    };
    let texts: Vec<String> = paragraphs_mut(&mut layouts).into_iter().map(|p| text_of(p)).collect();
    let all: String = texts.join("\n");
    if !all.chars().any(char::is_alphabetic) {
        return Err(PwError::new(
            ErrorCategory::UnsupportedOperation,
            "translate.no_text",
            "These pages have no text to translate.",
        )
        .with_action("If they are scans, recognize their text first (Make searchable)."));
    }
    let (source, detected) = match source {
        Some(s) => (s.to_string(), false),
        None => match pw_translate::detect::detect(&all) {
            Some((lang, _)) => (lang.to_string(), true),
            None => {
                return Err(PwError::new(
                    ErrorCategory::InvalidInput,
                    "translate.detect",
                    "The document's language could not be recognized.",
                )
                .with_action("Choose the language it is written in."))
            }
        },
    };
    let steps = pw_translate::route(&source, target)?;
    let translated = pw_translate::translate_all(&source, target, &texts, cancelled, progress)?;

    let mut report = TranslateReport {
        pages: idx.len() as u32,
        source: source.clone(),
        target: target.to_string(),
        detected,
        via_english: steps.len() > 1,
        ..Default::default()
    };
    for (src, out) in texts.iter().zip(&translated) {
        let tokens = pw_translate::protected_tokens(src);
        let missing = pw_translate::missing_tokens(src, out);
        report.protected_total += tokens.len();
        report.protected_kept += tokens.len() - missing.len();
        for m in missing {
            if report.protected_missing.len() < 20 && !report.protected_missing.contains(&m) {
                report.protected_missing.push(m);
            }
        }
    }
    // Each paragraph takes its translation in the style of its first run (or the common style).
    for (p, text) in paragraphs_mut(&mut layouts).into_iter().zip(&translated) {
        let Some(first) = p.runs.first().cloned() else { continue };
        let same = p.runs.iter().all(|r| {
            r.font == first.font
                && r.bold == first.bold
                && r.italic == first.italic
                && r.color == first.color
                && r.link == first.link
                && r.vert == first.vert
        });
        if !same {
            report.simplified_paragraphs += 1;
        }
        p.runs = vec![Run { text: text.clone(), link: if same { first.link } else { None }, ..first }];
        p.tabs.clear();
        p.boxes.clear();
    }
    report.paragraphs = translated.len();
    if cancelled() {
        return Err(PwError::cancelled());
    }

    let (bytes, c) = if word {
        let bytes = writer::write_docx(&writer::DocxInput {
            pages: &layouts,
            media: &media,
            links: &links,
            title: title.as_deref(),
        })?;
        let expected: HashMap<char, usize> = layouts.iter().fold(HashMap::new(), |mut m, l| {
            for (ch, n) in super::layout::emitted_chars(l) {
                *m.entry(ch).or_default() += n;
            }
            m
        });
        let sections: usize = layouts.iter().map(|l| l.bands.len().max(1)).sum();
        let verify = |b: &[u8]| -> PwResult<reader::DocxCheck> {
            let c = reader::check_docx(b)?;
            if c.sections != sections {
                return Err(fail("its sections do not match the pages"));
            }
            if c.chars != expected {
                return Err(fail("translated text is missing or changed"));
            }
            Ok(c)
        };
        let check = verify(&bytes)?;
        report.tables = check.tables;
        report.pictures = check.pictures;
        let c = write_atomic_bytes(output, policy, &[input.to_path_buf()], &bytes, |staged| {
            let back = std::fs::read(staged).map_err(|e| PwError::from_io(&e, "output file"))?;
            if back != bytes {
                return Err(fail("the written file differs"));
            }
            verify(&back).map(|_| ())
        })?;
        (bytes, c)
    } else {
        // Text: a heading per page, paragraphs on their own lines, Windows line endings.
        let mut body = String::new();
        let mut k = 0usize;
        for (pi, l) in layouts.iter().enumerate() {
            if pi > 0 {
                body.push_str("\r\n\x0c\r\n");
            }
            body.push_str(&format!("--- Page {} ---\r\n", idx[pi] + 1));
            let count = {
                let mut l2 = l.clone();
                paragraphs_mut(std::slice::from_mut(&mut l2)).len()
            };
            for t in &translated[k..k + count] {
                body.push_str(&t.replace('\n', "\r\n"));
                body.push_str("\r\n");
            }
            k += count;
        }
        let bytes = body.into_bytes();
        let c = write_atomic_bytes(output, policy, &[input.to_path_buf()], &bytes, |staged| {
            let back = std::fs::read(staged).map_err(|e| PwError::from_io(&e, "output file"))?;
            if back == bytes && std::str::from_utf8(&back).is_ok() {
                Ok(())
            } else {
                Err(PwError::new(
                    ErrorCategory::ValidationFailed,
                    "output.verify",
                    "The text file failed verification.",
                ))
            }
        })?;
        (bytes, c)
    };
    let _ = bytes;
    report.millis = started.elapsed().as_millis() as u64;
    let mut warnings = vec![format!(
        "Machine translation from {} to {}{}: review it before relying on it.",
        pw_translate::language_name(&report.source),
        pw_translate::language_name(&report.target),
        if report.via_english { " (through English)" } else { "" }
    )];
    if report.protected_kept < report.protected_total {
        warnings.push(format!(
            "{} of {} numbers, dates, codes or addresses are not written exactly as in the original (for example {}): check them.",
            report.protected_total - report.protected_kept,
            report.protected_total,
            report.protected_missing.iter().take(3).cloned().collect::<Vec<_>>().join(", ")
        ));
    }
    if report.simplified_paragraphs > 0 {
        warnings.push(format!(
            "{} paragraph{} with mixed formatting (such as a bold word or a link) now use one style.",
            report.simplified_paragraphs,
            if report.simplified_paragraphs == 1 { "" } else { "s" }
        ));
    }
    Ok((c, report, warnings))
}
