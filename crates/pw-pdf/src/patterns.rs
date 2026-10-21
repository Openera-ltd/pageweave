//! Find sensitive text (e-mail addresses, phone numbers, card numbers, IBANs, US SSNs, or a custom
//! regular expression) with the exact boxes of every match, so it can be marked for redaction.
//!
//! Matching runs over each page's characters as PDFium reports them (generated spaces and line
//! breaks included), so a match never silently spans two lines unless the pattern allows it.
//! Checksums (Luhn, ISO 13616) and digit counts cut false positives; the user still reviews every
//! match before anything is marked.

use crate::document::{load, OpenOptions, SearchRect, MAX_SEARCH_HITS};
use crate::{engine, map_pdfium_error};
use pdfium_render::prelude::*;
use pw_core::ops::TextPattern;
use pw_core::range::parse_page_ranges;
use pw_core::{PwError, PwResult};
use regex::{Regex, RegexBuilder};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::OnceLock;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PatternHit {
    /// 0-based page index.
    pub page: u32,
    /// `email`, `phone`, `card_number`, `iban`, `us_ssn` or `custom`.
    pub kind: String,
    pub text: String,
    /// One box per line the match covers, PDF points (page space, origin bottom-left).
    pub rects: Vec<SearchRect>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PatternReport {
    pub hits: Vec<PatternHit>,
    pub truncated: bool,
}

fn kind_name(p: TextPattern) -> &'static str {
    match p {
        TextPattern::Email => "email",
        TextPattern::Phone => "phone",
        TextPattern::CardNumber => "card_number",
        TextPattern::Iban => "iban",
        TextPattern::UsSsn => "us_ssn",
    }
}

fn builtin(p: TextPattern) -> &'static Regex {
    static RE: OnceLock<[Regex; 5]> = OnceLock::new();
    let all = RE.get_or_init(|| {
        let r = |s: &str| Regex::new(s).unwrap_or_else(|e| panic!("built-in pattern {s}: {e}"));
        [
            r(r"(?i)[a-z0-9._%+\-]+@[a-z0-9\-]+(?:\.[a-z0-9\-]+)*\.[a-z]{2,24}"),
            // Separators are spaces (not line breaks), dots and dashes.
            r(r"\+?\d{10,15}|(?:\+\d{1,3}[ .\-]?)?(?:\(\d{1,5}\)[ .\-]?)?\d{2,5}(?:[ .\-]\d{2,5}){0,5}"),
            r(r"\d(?:[ \-]?\d){12,18}"),
            r(r"[A-Z]{2}\d{2}(?: ?[A-Z0-9]){11,30}"),
            r(r"\d{3}-\d{2}-\d{4}"),
        ]
    });
    &all[match p {
        TextPattern::Email => 0,
        TextPattern::Phone => 1,
        TextPattern::CardNumber => 2,
        TextPattern::Iban => 3,
        TextPattern::UsSsn => 4,
    }]
}

fn digits(s: &str) -> Vec<u32> {
    s.chars().filter_map(|c| c.to_digit(10)).collect()
}

fn luhn(d: &[u32]) -> bool {
    let sum: u32 = d
        .iter()
        .rev()
        .enumerate()
        .map(|(i, &x)| {
            if i % 2 == 1 {
                if x * 2 > 9 {
                    x * 2 - 9
                } else {
                    x * 2
                }
            } else {
                x
            }
        })
        .sum();
    sum % 10 == 0
}

fn iban_ok(s: &str) -> bool {
    let compact: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    if !(15..=34).contains(&compact.len()) {
        return false;
    }
    let (head, tail) = compact.split_at(4);
    let mut rem: u64 = 0;
    for c in tail.chars().chain(head.chars()) {
        let v = match c.to_digit(36) {
            Some(v) => v as u64,
            None => return false,
        };
        rem = if v >= 10 { (rem * 100 + v) % 97 } else { (rem * 10 + v) % 97 };
    }
    rem == 1
}

fn looks_like_date(s: &str) -> bool {
    static DATE: OnceLock<Regex> = OnceLock::new();
    DATE.get_or_init(|| {
        Regex::new(r"^(?:\d{4}[.\-]\d{1,2}[.\-]\d{1,2}|\d{1,2}[.\-]\d{1,2}[.\-]\d{2,4})$")
            .unwrap_or_else(|e| panic!("{e}"))
    })
    .is_match(s)
}

/// Whether a built-in match is a real one (checksums, digit counts, boundaries).
fn accept(p: TextPattern, m: &str, before: Option<char>, after: Option<char>) -> bool {
    let alnum = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric());
    let word_char = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric() || c == '_');
    match p {
        TextPattern::Email => !word_char(before) && !matches!(after, Some(c) if c.is_alphanumeric()),
        TextPattern::Phone => {
            let n = digits(m).len();
            (7..=15).contains(&n)
                && !alnum(before)
                && !alnum(after)
                && !matches!(before, Some('-' | '.' | '/'))
                && !looks_like_date(m)
                // Plain runs of digits without any phone punctuation are too often other numbers.
                && (m.contains(['+', '(', ' ', '-', '.']) || n >= 10)
        }
        TextPattern::CardNumber => {
            let d = digits(m);
            (13..=19).contains(&d.len()) && luhn(&d) && !alnum(before) && !alnum(after)
        }
        TextPattern::Iban => iban_ok(m) && !alnum(before) && !alnum(after),
        TextPattern::UsSsn => {
            let d = digits(m);
            let area = d[0] * 100 + d[1] * 10 + d[2];
            !alnum(before)
                && !alnum(after)
                && area != 0
                && area != 666
                && area < 900
                && (d[3], d[4]) != (0, 0)
                && d[5..].iter().any(|x| *x != 0)
        }
    }
}

/// Characters of a page with their boxes (None for generated characters without a box).
fn page_chars(page: &PdfPage) -> PwResult<(String, Vec<(usize, Option<SearchRect>)>)> {
    let text = page.text().map_err(|e| map_pdfium_error(&e, "text"))?;
    let mut s = String::new();
    // Byte offset in `s` → box of the character starting there.
    let mut boxes = Vec::new();
    for ch in text.chars().iter() {
        let c = ch.unicode_char().unwrap_or('\u{FFFD}');
        let rect = ch.loose_bounds().ok().and_then(|b| {
            let r = SearchRect {
                left: b.left().value,
                bottom: b.bottom().value,
                right: b.right().value,
                top: b.top().value,
            };
            (r.right > r.left && r.top > r.bottom).then_some(r)
        });
        boxes.push((s.len(), rect));
        s.push(c);
    }
    Ok((s, boxes))
}

/// Boxes of the characters in `start..end`, merged per line.
fn match_rects(boxes: &[(usize, Option<SearchRect>)], start: usize, end: usize) -> Vec<SearchRect> {
    let first = boxes.partition_point(|(o, _)| *o < start);
    let mut out: Vec<SearchRect> = Vec::new();
    for (_, r) in boxes[first..].iter().take_while(|(o, _)| *o < end) {
        let Some(r) = r else { continue };
        if let Some(last) = out.last_mut() {
            let overlap = last.top.min(r.top) - last.bottom.max(r.bottom);
            let h = (last.top - last.bottom).min(r.top - r.bottom);
            let close = r.left <= last.right + h * 2.0 && r.right >= last.left - h * 2.0;
            if overlap > h * 0.5 && close {
                last.left = last.left.min(r.left);
                last.right = last.right.max(r.right);
                last.bottom = last.bottom.min(r.bottom);
                last.top = last.top.max(r.top);
                continue;
            }
        }
        out.push(r.clone());
    }
    out
}

/// Compile a user's pattern with size limits (the regex engine is linear-time; no backtracking).
pub fn compile_custom(pattern: &str) -> PwResult<Regex> {
    RegexBuilder::new(pattern).size_limit(1 << 20).dfa_size_limit(4 << 20).nest_limit(64).build().map_err(|e| {
        let detail = match e {
            regex::Error::CompiledTooBig(_) => "it is too complex".to_string(),
            other => other.to_string().lines().last().unwrap_or("it is not valid").trim().to_string(),
        };
        PwError::invalid("pattern.invalid", format!("The pattern can't be used: {detail}."))
    })
}

pub fn find_patterns(
    input: &Path,
    opts: &OpenOptions,
    patterns: &[TextPattern],
    custom: Option<&str>,
    pages: Option<&str>,
    cancelled: &dyn Fn() -> bool,
    progress: &dyn Fn(u64, u64),
) -> PwResult<PatternReport> {
    let custom = custom.map(str::trim).filter(|c| !c.is_empty()).map(compile_custom).transpose()?;
    if patterns.is_empty() && custom.is_none() {
        return Err(PwError::invalid("pattern.none", "Choose what to look for."));
    }
    let pdfium = engine()?;
    let doc = load(&pdfium, input, opts)?;
    let count = doc.pages().len().max(0) as u32;
    let chosen: Vec<u32> = match pages {
        Some(spec) => parse_page_ranges(spec, count)?,
        None => (0..count).collect(),
    };
    // Earlier kinds win where matches overlap (an IBAN's digits are not also a phone number).
    let order =
        [TextPattern::Email, TextPattern::Iban, TextPattern::CardNumber, TextPattern::UsSsn, TextPattern::Phone];
    let mut hits = Vec::new();
    for (n, &pi) in chosen.iter().enumerate() {
        if cancelled() {
            return Err(PwError::cancelled());
        }
        progress(n as u64, chosen.len() as u64);
        let page = doc.pages().get(pi as PdfPageIndex).map_err(|e| map_pdfium_error(&e, "page"))?;
        let (text, boxes) = page_chars(&page)?;
        let mut taken: Vec<(usize, usize)> = Vec::new();
        let mut found: Vec<(usize, usize, &'static str)> = Vec::new();
        let free = |taken: &[(usize, usize)], s: usize, e: usize| taken.iter().all(|(a, b)| e <= *a || s >= *b);
        for p in order.iter().filter(|p| patterns.contains(p)) {
            for m in builtin(*p).find_iter(&text) {
                let before = text[..m.start()].chars().next_back();
                let after = text[m.end()..].chars().next();
                if accept(*p, m.as_str(), before, after) && free(&taken, m.start(), m.end()) {
                    taken.push((m.start(), m.end()));
                    found.push((m.start(), m.end(), kind_name(*p)));
                }
            }
        }
        if let Some(re) = &custom {
            for m in re.find_iter(&text).filter(|m| !m.as_str().trim().is_empty()) {
                if free(&taken, m.start(), m.end()) {
                    taken.push((m.start(), m.end()));
                    found.push((m.start(), m.end(), "custom"));
                }
            }
        }
        found.sort_by_key(|(s, _, _)| *s);
        for (s, e, kind) in found {
            let rects = match_rects(&boxes, s, e);
            if rects.is_empty() {
                continue; // nothing drawn to mark (generated characters only)
            }
            if hits.len() >= MAX_SEARCH_HITS {
                return Ok(PatternReport { hits, truncated: true });
            }
            hits.push(PatternHit { page: pi, kind: kind.to_string(), text: text[s..e].trim().to_string(), rects });
        }
    }
    progress(chosen.len() as u64, chosen.len() as u64);
    Ok(PatternReport { hits, truncated: false })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all(p: TextPattern, text: &str) -> Vec<String> {
        builtin(p)
            .find_iter(text)
            .filter(|m| accept(p, m.as_str(), text[..m.start()].chars().next_back(), text[m.end()..].chars().next()))
            .map(|m| m.as_str().to_string())
            .collect()
    }

    #[test]
    fn checksums_and_boundaries_cut_false_positives() {
        assert_eq!(all(TextPattern::Email, "mail ada.l+x@example.co.uk, not a@b or @x.com"), ["ada.l+x@example.co.uk"]);
        assert_eq!(
            all(TextPattern::Phone, "Call +44 20 7946 0958 or (555) 123-4567; ref 2026-10-21, order 1234, 0301234567"),
            ["+44 20 7946 0958", "(555) 123-4567", "0301234567"]
        );
        // 4111 1111 1111 1111 passes Luhn; the second differs in the last digit.
        assert_eq!(
            all(TextPattern::CardNumber, "4111 1111 1111 1111 and 4111-1111-1111-1112"),
            ["4111 1111 1111 1111"]
        );
        assert_eq!(
            all(TextPattern::Iban, "IBAN GB82 WEST 1234 5698 7654 32 / GB82 WEST 1234 5698 7654 33"),
            ["GB82 WEST 1234 5698 7654 32"]
        );
        assert_eq!(
            all(TextPattern::UsSsn, "123-45-6789 000-12-3456 666-12-3456 912-34-5678 123-00-4567 X123-45-6789"),
            ["123-45-6789"]
        );
        assert!(luhn(&digits("79927398713")));
        assert!(iban_ok("DE89370400440532013000"));
        assert!(!iban_ok("DE89370400440532013001"));
    }

    #[test]
    fn custom_patterns_are_bounded_and_explained() {
        assert!(compile_custom(r"INV-\d{6}").is_ok());
        let e = compile_custom("(unclosed").expect_err("invalid");
        assert_eq!(e.code, "pattern.invalid");
        assert!(e.message.starts_with("The pattern can't be used:"), "{}", e.message);
        // Huge counted repetition exceeds the compiled size limit instead of running away.
        assert_eq!(compile_custom(r"(\w{1000}){1000}").expect_err("too big").code, "pattern.invalid");
    }

    #[test]
    fn boxes_merge_per_line() {
        let b = |l: f32, bottom: f32| Some(SearchRect { left: l, bottom, right: l + 5.0, top: bottom + 10.0 });
        let boxes = vec![(0, b(0.0, 100.0)), (1, b(5.0, 100.0)), (2, None), (3, b(0.0, 80.0)), (4, b(5.0, 80.0))];
        let r = match_rects(&boxes, 0, 5);
        assert_eq!(r.len(), 2);
        assert_eq!((r[0].left, r[0].right, r[0].bottom), (0.0, 10.0, 100.0));
        assert_eq!(match_rects(&boxes, 3, 5).len(), 1);
    }
}
