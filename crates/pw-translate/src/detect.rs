//! Source-language detection for the languages PageWeave translates (en, de, fr, es, it, pt): share
//! of each language's most frequent function words among the text's words. Returns `None` when the
//! text is too short or no language clearly wins; the user then chooses (manual override always
//! available).

const WORDS: &[(&str, &[&str])] = &[
    (
        "en",
        &[
            "the", "and", "of", "to", "in", "is", "that", "it", "for", "with", "was", "on", "as", "are", "this", "be",
            "by", "not", "or", "have", "from", "at", "which", "but", "they", "were", "has", "their", "will", "would",
        ],
    ),
    (
        "de",
        &[
            "der", "die", "und", "das", "ist", "nicht", "den", "ein", "zu", "mit", "sich", "des", "auf", "für", "im",
            "dem", "eine", "auch", "es", "an", "als", "wird", "sind", "von", "bei", "einer", "nach", "wir", "werden",
            "oder",
        ],
    ),
    (
        "fr",
        &[
            "le", "la", "les", "de", "des", "et", "est", "un", "une", "du", "que", "qui", "dans", "pour", "pas", "sur",
            "au", "avec", "ce", "il", "sont", "par", "plus", "nous", "ne", "aux", "elle", "cette", "été", "mais",
        ],
    ),
    (
        "es",
        &[
            "el", "la", "de", "que", "y", "los", "las", "en", "un", "una", "es", "por", "con", "para", "no", "se",
            "del", "al", "lo", "como", "más", "pero", "sus", "su", "fue", "son", "este", "esta", "entre", "también",
        ],
    ),
    (
        "it",
        &[
            "il", "di", "che", "la", "e", "un", "una", "per", "non", "del", "della", "sono", "con", "le", "gli", "si",
            "da", "nel", "alla", "anche", "come", "più", "ma", "è", "dei", "questo", "questa", "delle", "lo", "ha",
        ],
    ),
    (
        "pt",
        &[
            "o", "os", "do", "da", "dos", "das", "e", "que", "em", "um", "uma", "para", "com", "não", "no", "na",
            "nos", "nas", "ao", "mais", "mas", "foi", "são", "seu", "sua", "também", "pelo", "pela", "isso", "está",
        ],
    ),
];

/// The most likely language of `text` and its share of function words, if clear.
pub fn detect(text: &str) -> Option<(&'static str, f32)> {
    let words: Vec<String> = text
        .split(|c: char| !c.is_alphabetic() && c != '\'' && c != '’')
        .filter(|w| !w.is_empty())
        .map(|w| w.to_lowercase().trim_matches(['\'', '’']).to_string())
        .take(5000)
        .collect();
    if words.len() < 4 {
        return None;
    }
    let mut scores: Vec<(&'static str, f32)> = WORDS
        .iter()
        .map(|(lang, list)| {
            (*lang, words.iter().filter(|w| list.contains(&w.as_str())).count() as f32 / words.len() as f32)
        })
        .collect();
    scores.sort_by(|a, b| b.1.total_cmp(&a.1));
    let (best, second) = (scores[0], scores[1]);
    (best.1 >= 0.12 && best.1 >= 1.5 * second.1).then_some(best)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn common_languages_are_told_apart_and_unclear_text_is_not_guessed() {
        let cases = [
            ("en", "The results of the study show that the river is cleaner than it was in the past."),
            ("de", "Die Ergebnisse der Studie zeigen, dass der Fluss sauberer ist als in der Vergangenheit."),
            ("fr", "Les résultats de l'étude montrent que la rivière est plus propre que dans le passé."),
            ("es", "Los resultados del estudio muestran que el río está más limpio que en el pasado."),
            ("it", "I risultati dello studio mostrano che il fiume è più pulito che in passato e anche oggi."),
            ("pt", "Os resultados do estudo mostram que o rio está mais limpo do que no passado e também hoje."),
        ];
        for (lang, text) in cases {
            assert_eq!(detect(text).map(|d| d.0), Some(lang), "{text}");
        }
        assert_eq!(detect("PWMARK 12345"), None);
        assert_eq!(detect("Invoice 2026-10-21 total 1,234.50 EUR ref ABC-77"), None);
        assert_eq!(detect(""), None);
    }
}
