//! Text helpers shared by speech and translation.

/// Sentences of `text` as character ranges (whitespace around them excluded). A sentence ends at
/// `.`, `!`, `?`, `…` (and their full-width forms) followed by a space or the end, or at a line end.
/// Common abbreviations ("Mr.", "e.g.") and decimal numbers do not end a sentence.
pub fn sentences(text: &str) -> Vec<(usize, usize)> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut start: Option<usize> = None;
    let push = |out: &mut Vec<(usize, usize)>, s: usize, e: usize, chars: &[char]| {
        let mut e = e;
        while e > s && chars[e - 1].is_whitespace() {
            e -= 1;
        }
        if e > s {
            out.push((s, e));
        }
    };
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if start.is_none() {
            if !c.is_whitespace() {
                start = Some(i);
            }
            i += 1;
            continue;
        }
        let s = start.expect("start");
        if c == '\n' {
            push(&mut out, s, i, &chars);
            start = None;
            i += 1;
            continue;
        }
        if matches!(c, '.' | '!' | '?' | '…' | '。' | '！' | '？') {
            // Closing quotes and brackets belong to the sentence.
            let mut j = i + 1;
            while j < chars.len() && matches!(chars[j], '"' | '\'' | '”' | '’' | ')' | ']' | '»' | '.' | '!' | '?')
            {
                j += 1;
            }
            let at_end = j >= chars.len() || chars[j].is_whitespace() || matches!(c, '。' | '！' | '？');
            if at_end && !(c == '.' && is_abbreviation(&chars[s..i])) {
                push(&mut out, s, j, &chars);
                start = None;
                i = j;
                continue;
            }
        }
        i += 1;
    }
    if let Some(s) = start {
        push(&mut out, s, chars.len(), &chars);
    }
    out
}

fn is_abbreviation(before: &[char]) -> bool {
    let word: String =
        before.iter().rev().take_while(|c| !c.is_whitespace()).collect::<Vec<_>>().into_iter().rev().collect();
    let w = word.trim_start_matches(['(', '"', '\'']).to_ascii_lowercase();
    const ABBR: &[&str] =
        &["mr", "mrs", "ms", "dr", "prof", "sr", "jr", "st", "vs", "etc", "e.g", "i.e", "fig", "approx", "dept"];
    ABBR.contains(&w.as_str()) || (w.chars().count() == 1 && w.chars().all(|c| c.is_alphabetic()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cut(t: &str) -> Vec<String> {
        let c: Vec<char> = t.chars().collect();
        sentences(t).into_iter().map(|(s, e)| c[s..e].iter().collect()).collect()
    }

    #[test]
    fn sentences_split_at_ends_and_lines_not_abbreviations_or_numbers() {
        assert_eq!(
            cut("Hello world. It costs 12.50 dollars, Mr. Smith!  Really? Yes…\nA heading\n\n  Last one"),
            vec!["Hello world.", "It costs 12.50 dollars, Mr. Smith!", "Really?", "Yes…", "A heading", "Last one"]
        );
        assert_eq!(cut("See e.g. the table. Fig. 3 shows it."), vec!["See e.g. the table.", "Fig. 3 shows it."]);
        assert_eq!(cut("He said \"stop.\" Then left."), vec!["He said \"stop.\"", "Then left."]);
        assert_eq!(cut("日本語です。次の文。"), vec!["日本語です。", "次の文。"]);
        assert!(cut("   \n \n").is_empty());
        assert_eq!(cut("J. R. Tolkien wrote it."), vec!["J. R. Tolkien wrote it."]);
    }
}
