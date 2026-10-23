//! Page label text as PDF readers compute it (PDF 32000-1 §12.4.2), used to verify written labels.

use crate::ops::{LabelStyle, PageLabelRange};

fn roman(mut n: u32) -> String {
    const T: [(u32, &str); 13] = [
        (1000, "m"),
        (900, "cm"),
        (500, "d"),
        (400, "cd"),
        (100, "c"),
        (90, "xc"),
        (50, "l"),
        (40, "xl"),
        (10, "x"),
        (9, "ix"),
        (5, "v"),
        (4, "iv"),
        (1, "i"),
    ];
    let mut s = String::new();
    for (v, r) in T {
        while n >= v {
            s.push_str(r);
            n -= v;
        }
    }
    s
}

/// 1 → a, 26 → z, 27 → aa, 28 → bb, 53 → aaa (the letter repeated, as the PDF spec defines).
fn letters(n: u32) -> String {
    let n = n.max(1) - 1;
    let ch = (b'a' + (n % 26) as u8) as char;
    std::iter::repeat_n(ch, (n / 26 + 1) as usize).collect()
}

/// The label of 0-based `page`, or `None` when no range covers it (no labels at all).
pub fn label_for(ranges: &[PageLabelRange], page: u32) -> Option<String> {
    let r = ranges.iter().rev().find(|r| r.start <= page)?;
    let n = r.first.saturating_add(page - r.start);
    let num = match r.style {
        LabelStyle::Decimal => n.to_string(),
        LabelStyle::RomanLower => roman(n),
        LabelStyle::RomanUpper => roman(n).to_uppercase(),
        LabelStyle::LettersLower => letters(n),
        LabelStyle::LettersUpper => letters(n).to_uppercase(),
        LabelStyle::None => String::new(),
    };
    Some(format!("{}{num}", r.prefix))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(start: u32, style: LabelStyle, prefix: &str, first: u32) -> PageLabelRange {
        PageLabelRange { start, style, prefix: prefix.into(), first }
    }

    #[test]
    fn styles_follow_the_pdf_rules() {
        assert_eq!(roman(1994), "mcmxciv");
        assert_eq!(letters(1), "a");
        assert_eq!(letters(26), "z");
        assert_eq!(letters(27), "aa");
        assert_eq!(letters(28), "bb");
        assert_eq!(letters(53), "aaa");
        let ranges = [
            r(0, LabelStyle::RomanLower, "", 1),
            r(3, LabelStyle::Decimal, "", 1),
            r(6, LabelStyle::Decimal, "A-", 7),
            r(8, LabelStyle::None, "Back cover", 1),
            r(9, LabelStyle::LettersUpper, "", 26),
        ];
        let got: Vec<String> = (0..11).map(|p| label_for(&ranges, p).unwrap_or_default()).collect();
        assert_eq!(got, ["i", "ii", "iii", "1", "2", "3", "A-7", "A-8", "Back cover", "Z", "AA"]);
        assert_eq!(label_for(&[], 0), None);
        assert_eq!(label_for(&[r(0, LabelStyle::RomanUpper, "", 4)], 0).as_deref(), Some("IV"));
    }
}
