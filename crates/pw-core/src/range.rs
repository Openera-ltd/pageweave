//! Page-range syntax shared by every interface: `1-3, 5, 8-10`, `7-` (to last page), `last`.
//!
//! Pages are 1-based in user-facing syntax and returned 0-based. Rules (PRD 01 D / PRD 05):
//! - whitespace around tokens is allowed; empty tokens (`1,,2`, trailing comma) are rejected;
//! - zero, negative and non-numeric values are rejected;
//! - descending ranges (`5-3`) are rejected rather than silently reversed;
//! - out-of-range pages are rejected, naming the document's page count;
//! - repeated or overlapping pages are rejected by default, because for extract/delete/rotate a
//!   duplicate is almost always a typing mistake. Callers that need duplicates (e.g. page
//!   duplication) use [`RangeOptions::allow_duplicates`].

use crate::error::{PwError, PwResult};

#[derive(Debug, Clone, Copy, Default)]
pub struct RangeOptions {
    pub allow_duplicates: bool,
}

/// Parse `spec` against a document with `page_count` pages. Returns 0-based indices in the order given.
pub fn parse_page_ranges(spec: &str, page_count: u32) -> PwResult<Vec<u32>> {
    parse_page_ranges_with(spec, page_count, RangeOptions::default())
}

pub fn parse_page_ranges_with(spec: &str, page_count: u32, opts: RangeOptions) -> PwResult<Vec<u32>> {
    if page_count == 0 {
        return Err(PwError::invalid("range.no_pages", "The document has no pages."));
    }
    if spec.trim().is_empty() {
        return Err(PwError::invalid("range.empty", "Enter at least one page, for example 1-3, 5."));
    }
    if spec.len() > 4096 {
        return Err(PwError::invalid("range.too_long", "The page range is too long."));
    }
    let mut out: Vec<u32> = Vec::new();
    let mut seen = vec![false; page_count as usize];
    for raw in spec.split(',') {
        let token = raw.trim();
        if token.is_empty() {
            return Err(PwError::invalid("range.empty_item", "Remove the empty item between commas."));
        }
        let (start, end) = parse_token(token, page_count)?;
        for p in start..=end {
            let idx = (p - 1) as usize;
            if seen[idx] && !opts.allow_duplicates {
                return Err(PwError::invalid("range.duplicate", format!("Page {p} is listed more than once.")));
            }
            seen[idx] = true;
            out.push(p - 1);
        }
    }
    Ok(out)
}

fn parse_token(token: &str, page_count: u32) -> PwResult<(u32, u32)> {
    if let Some((a, b)) = token.split_once('-') {
        let a = a.trim();
        let b = b.trim();
        if a.is_empty() {
            // "-3" is a negative number or a missing start; both are errors.
            return Err(PwError::invalid(
                "range.missing_start",
                format!("“{token}” needs a starting page, for example 1-3."),
            ));
        }
        let start = parse_page(a, page_count)?;
        let end = if b.is_empty() { page_count } else { parse_page(b, page_count)? };
        if end < start {
            return Err(PwError::invalid(
                "range.descending",
                format!("“{token}” goes backwards. Write it as {end}-{start}."),
            ));
        }
        Ok((start, end))
    } else {
        let p = parse_page(token, page_count)?;
        Ok((p, p))
    }
}

fn parse_page(s: &str, page_count: u32) -> PwResult<u32> {
    if s.eq_ignore_ascii_case("last") || s.eq_ignore_ascii_case("end") {
        return Ok(page_count);
    }
    if !s.bytes().all(|b| b.is_ascii_digit()) {
        return Err(PwError::invalid("range.not_a_number", format!("“{s}” is not a page number.")));
    }
    let n: u64 = s.parse().map_err(|_| PwError::invalid("range.out_of_bounds", format!("Page {s} is too large.")))?;
    if n == 0 {
        return Err(PwError::invalid("range.zero", "Pages start at 1."));
    }
    if n > page_count as u64 {
        return Err(PwError::invalid(
            "range.out_of_bounds",
            format!("Page {n} does not exist; the document has {page_count} pages."),
        ));
    }
    Ok(n as u32)
}

/// Format 0-based indices compactly as 1-based ranges: `[0,1,2,4]` → `1-3, 5`.
pub fn format_page_ranges(indices: &[u32]) -> String {
    let mut parts = Vec::new();
    let mut i = 0;
    while i < indices.len() {
        let start = indices[i];
        let mut end = start;
        while i + 1 < indices.len() && indices[i + 1] == end + 1 {
            end += 1;
            i += 1;
        }
        parts.push(if start == end { format!("{}", start + 1) } else { format!("{}-{}", start + 1, end + 1) });
        i += 1;
    }
    parts.join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(spec: &str, n: u32) -> Vec<u32> {
        parse_page_ranges(spec, n).unwrap_or_else(|e| panic!("{spec:?} should parse: {e:?}"))
    }
    fn code(spec: &str, n: u32) -> String {
        parse_page_ranges(spec, n).expect_err(spec).code
    }

    #[test]
    fn valid_forms() {
        assert_eq!(ok("1-3,5,8-10", 10), vec![0, 1, 2, 4, 7, 8, 9]);
        assert_eq!(ok(" 2 ,  4 - 5 ", 5), vec![1, 3, 4]);
        assert_eq!(ok("7-", 9), vec![6, 7, 8]);
        assert_eq!(ok("last", 4), vec![3]);
        assert_eq!(ok("3-LAST", 4), vec![2, 3]);
        assert_eq!(ok("5,1", 5), vec![4, 0], "order given by the user is preserved");
        assert_eq!(ok("1-1", 1), vec![0]);
    }

    #[test]
    fn rejects_invalid_forms() {
        assert_eq!(code("", 5), "range.empty");
        assert_eq!(code("   ", 5), "range.empty");
        assert_eq!(code("1,,2", 5), "range.empty_item");
        assert_eq!(code("1,", 5), "range.empty_item");
        assert_eq!(code("0", 5), "range.zero");
        assert_eq!(code("0-2", 5), "range.zero");
        assert_eq!(code("-3", 5), "range.missing_start");
        assert_eq!(code("-", 5), "range.missing_start");
        assert_eq!(code("5-3", 5), "range.descending");
        assert_eq!(code("6", 5), "range.out_of_bounds");
        assert_eq!(code("2-6", 5), "range.out_of_bounds");
        assert_eq!(code("99999999999999999999", 5), "range.out_of_bounds");
        assert_eq!(code("a", 5), "range.not_a_number");
        assert_eq!(code("1.5", 5), "range.not_a_number");
        assert_eq!(code("+1", 5), "range.not_a_number");
        assert_eq!(code("1-2-3", 5), "range.not_a_number");
        assert_eq!(code("2,2", 5), "range.duplicate");
        assert_eq!(code("1-3,2-4", 5), "range.duplicate");
        assert_eq!(code("1", 0), "range.no_pages");
    }

    #[test]
    fn duplicates_allowed_when_requested() {
        let v = parse_page_ranges_with("1,1,2", 3, RangeOptions { allow_duplicates: true }).expect("parse");
        assert_eq!(v, vec![0, 0, 1]);
    }

    #[test]
    fn out_of_range_message_names_page_count() {
        let e = parse_page_ranges("9", 3).expect_err("out of range");
        assert!(e.message.contains("3 pages"), "{}", e.message);
    }

    #[test]
    fn format_round_trip() {
        assert_eq!(format_page_ranges(&[0, 1, 2, 4, 7, 8, 9]), "1-3, 5, 8-10");
        assert_eq!(format_page_ranges(&[]), "");
        for spec in ["1-3, 5, 8-10", "2", "1-10"] {
            assert_eq!(format_page_ranges(&ok(spec, 10)), spec);
        }
    }
}
