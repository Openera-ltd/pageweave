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

/// One word of a translation with the run (of its paragraph) whose style it takes.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct StyledWord {
    pub text: String,
    pub style: usize,
}

fn same_style(a: &Run, b: &Run) -> bool {
    a.font == b.font
        && a.bold == b.bold
        && a.italic == b.italic
        && a.color == b.color
        && a.link == b.link
        && a.vert == b.vert
}

/// The run whose style most of the paragraph's letters have.
fn main_run(p: &Paragraph) -> usize {
    let mut best = (0usize, 0usize);
    for (i, r) in p.runs.iter().enumerate() {
        let n: usize = p
            .runs
            .iter()
            .filter(|o| same_style(o, r))
            .map(|o| o.text.chars().filter(|c| c.is_alphanumeric()).count())
            .sum();
        if n > best.1 {
            best = (i, n);
        }
    }
    best.0
}

/// Runs set apart from the paragraph's main style (a bold word, a coloured phrase) whose own
/// translation is looked for in the paragraph's translation.
fn marked_runs(p: &Paragraph) -> Vec<(usize, String)> {
    let main = main_run(p);
    p.runs
        .iter()
        .enumerate()
        .filter(|(_, r)| !same_style(r, &p.runs[main]))
        .map(|(i, r)| (i, r.text.trim().to_string()))
        .filter(|(_, t)| t.chars().filter(|c| c.is_alphabetic()).count() >= 2)
        .collect()
}

fn norm(w: &str) -> String {
    w.trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase()
}

/// Split `out` into words, each styled like the main run unless it belongs to a marked run's
/// translation (or to the run's own text, kept as is). Returns the words and whether every marked
/// run was found.
pub(super) fn style_words(p: &Paragraph, out: &str, run_out: &HashMap<String, String>) -> (Vec<StyledWord>, bool) {
    let main = main_run(p);
    let mut words: Vec<StyledWord> =
        out.split_whitespace().map(|w| StyledWord { text: w.to_string(), style: main }).collect();
    let keys: Vec<String> = words.iter().map(|w| norm(&w.text)).collect();
    let mut taken = vec![false; words.len()];
    let mut all = true;
    for (ri, text) in marked_runs(p) {
        let mut cands: Vec<Vec<String>> = Vec::new();
        if let Some(t) = run_out.get(&text) {
            cands.push(t.split_whitespace().map(norm).filter(|w| !w.is_empty()).collect());
        }
        cands.push(text.split_whitespace().map(norm).filter(|w| !w.is_empty()).collect());
        let mut found = false;
        'c: for c in cands.iter().filter(|c| !c.is_empty()) {
            // Exact sequence first, then the longest part of it (translations reorder words).
            for len in (1..=c.len()).rev() {
                if len < c.len() && len * 2 < c.len() {
                    break;
                }
                for s in 0..=c.len() - len {
                    let part = &c[s..s + len];
                    for start in 0..keys.len().saturating_sub(len - 1) {
                        if (0..len).all(|k| !taken[start + k] && keys[start + k] == part[k]) {
                            for k in 0..len {
                                taken[start + k] = true;
                                words[start + k].style = ri;
                            }
                            found = true;
                            break 'c;
                        }
                    }
                }
            }
        }
        all &= found;
    }
    (words, all)
}

/// The text a styled translation stands for (for checks).
fn joined(words: &[StyledWord]) -> String {
    words.iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" ")
}

pub(super) fn paragraphs_mut(layouts: &mut [PageLayout]) -> Vec<&mut Paragraph> {
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
/// `output` ending in `.pdf` gets a copy of the PDF with the same layout, `.docx` a Word document,
/// `.txt` a text file.
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
    let pdf = output.extension().is_some_and(|e| e.eq_ignore_ascii_case("pdf"));
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
    // The units translated: paragraphs (Word, text) or blocks in place (PDF).
    let blocks = if pdf { Some(super::translate_pdf::placed(&layouts, &idx)) } else { None };
    let units: Vec<Paragraph> = match &blocks {
        Some(b) => b.iter().map(|p| p.para.clone()).collect(),
        None => paragraphs_mut(&mut layouts).into_iter().map(|p| p.clone()).collect(),
    };
    let texts: Vec<String> = units.iter().map(text_of).collect();
    if std::env::var_os("PW_DEBUG_UNITS").is_some() {
        for (u, t) in units.iter().zip(&texts) {
            eprintln!("UNIT {:?} {:?} lines={:?} | {}", u.rect, u.align, u.frame.map(|f| f.lines), t);
        }
    }
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
    // Marked runs (bold words, coloured phrases) are translated on their own too, to find them in
    // their paragraph's translation.
    let mut batch = texts.clone();
    let mut run_texts: Vec<String> = Vec::new();
    {
        let mut seen = std::collections::HashSet::new();
        for p in &units {
            for (_, t) in marked_runs(p) {
                if seen.insert(t.clone()) {
                    run_texts.push(t);
                }
            }
        }
    }
    batch.extend(run_texts.iter().cloned());
    let mut translated = pw_translate::translate_all(&source, target, &batch, cancelled, progress)?;
    let run_out: HashMap<String, String> = run_texts.into_iter().zip(translated.split_off(texts.len())).collect();

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
    // Words take the style of the run they come from (bold and coloured words found again).
    let styled: Vec<Option<Vec<StyledWord>>> = units
        .iter()
        .zip(texts.iter().zip(&translated))
        .map(|(p, (src, out))| {
            if p.runs.is_empty() || out.trim().is_empty() || out.trim() == src.trim() {
                return None;
            }
            let (w, all) = style_words(p, out, &run_out);
            if !all {
                report.simplified_paragraphs += 1;
            }
            Some(w)
        })
        .collect();
    report.paragraphs = translated.len();
    if cancelled() {
        return Err(PwError::cancelled());
    }
    if pdf {
        let blocks = blocks.unwrap_or_default();
        let (c, written) = write_pdf(input, opts, &blocks, &styled, output, policy, cancelled)?;
        report.millis = started.elapsed().as_millis() as u64;
        let mut warnings = notes(&report);
        if written.overflowing > 0 {
            warnings.push(format!(
                "{} paragraph{} did not fit in {} space even at a smaller size: check {}.",
                written.overflowing,
                if written.overflowing == 1 { "" } else { "s" },
                if written.overflowing == 1 { "its" } else { "their" },
                if written.overflowing == 1 { "it" } else { "them" }
            ));
        }
        if written.covered > 0 {
            warnings.push(
                "Recognized text over scanned pages was covered with white so the translation can be read.".into(),
            );
        }
        return Ok((c, report, warnings));
    }
    // Word and text: each paragraph's words, consecutive words of one style joined in one run.
    for (p, w) in paragraphs_mut(&mut layouts).into_iter().zip(&styled) {
        let Some(words) = w else { continue };
        let mut runs: Vec<Run> = Vec::new();
        let mut last: Option<usize> = None;
        for wd in words {
            if last == Some(wd.style) {
                if let Some(r) = runs.last_mut() {
                    r.text.push(' ');
                    r.text.push_str(&wd.text);
                }
            } else {
                if let Some(r) = runs.last_mut() {
                    r.text.push(' ');
                }
                runs.push(Run { text: wd.text.clone(), ..p.runs[wd.style].clone() });
                last = Some(wd.style);
            }
        }
        p.runs = runs;
        p.tabs.clear();
        p.boxes.clear();
    }
    let translated: Vec<String> = texts
        .iter()
        .zip(&translated)
        .zip(&styled)
        .map(|((src, out), w)| match w {
            Some(w) => joined(w),
            None if out.trim().is_empty() => src.clone(),
            None => out.clone(),
        })
        .collect();

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
    Ok((c, report.clone(), notes(&report)))
}

fn notes(report: &TranslateReport) -> Vec<String> {
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
    warnings
}

/// The PDF with every translated paragraph written in place, checked after saving.
#[allow(clippy::too_many_arguments)]
fn write_pdf(
    input: &Path,
    opts: &OpenOptions,
    placed: &[super::translate_pdf::Placed],
    styled: &[Option<Vec<StyledWord>>],
    output: &Path,
    policy: CollisionPolicy,
    cancelled: &dyn Fn() -> bool,
) -> PwResult<(CommittedOutput, super::translate_pdf::Written)> {
    let pdfium = engine()?;
    let mut doc = load(&pdfium, input, opts)?;
    if placed.len() != styled.len() {
        return Err(PwError::internal("translate.layout", "The page layout changed while translating."));
    }
    let written = super::translate_pdf::write_in_place(&mut doc, placed, styled, cancelled)?;
    let bytes = crate::edit::save_bytes(&doc)?;
    let pages = doc.pages().len().max(0) as u32;
    drop(doc);
    let c = write_atomic_bytes(output, policy, &[input.to_path_buf()], &bytes, |staged| {
        let back = pdfium.load_pdf_from_file(staged, opts.password).map_err(|_| {
            PwError::new(ErrorCategory::ValidationFailed, "output.verify", "The translated PDF could not be reopened.")
        })?;
        if back.pages().len().max(0) as u32 != pages {
            return Err(PwError::new(
                ErrorCategory::ValidationFailed,
                "output.verify",
                "The translated PDF lost pages.",
            ));
        }
        super::translate_pdf::verify_lines(&back, &written)
    })?;
    Ok((c, written))
}
