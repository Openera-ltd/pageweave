//! Read-aloud text against the real engine: reading order (columns one after another), word
//! positions in the page's own coordinates (they match search results, also on turned pages),
//! and WAV export with the installed speech engine.

use pw_core::ops::{Operation, OperationRequest};
use pw_core::output::CollisionPolicy;
use pw_core::ErrorCategory;
use pw_pdf::reading::{reading_text, ReadingPage};
use pw_pdf::{execute, search, OpenOptions, SearchRect};
use std::path::{Path, PathBuf};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/pdf").join(name)
}

fn word_rects(r: &ReadingPage, word: &str) -> Vec<SearchRect> {
    let chars: Vec<char> = r.text.chars().collect();
    r.words
        .iter()
        .find(|w| {
            chars[w.start as usize..w.end as usize].iter().collect::<String>().trim_end_matches(['.', ',']) == word
        })
        .unwrap_or_else(|| panic!("{word} in {}", r.text))
        .rects
        .clone()
}

fn overlaps(a: &SearchRect, b: &SearchRect) -> bool {
    let w = a.right.min(b.right) - a.left.max(b.left);
    let h = a.top.min(b.top) - a.bottom.max(b.bottom);
    w > 0.5 * (b.right - b.left) && h > 0.5 * (b.top - b.bottom)
}

#[test]
fn columns_are_read_in_order_and_words_are_where_search_finds_them() {
    let opts = OpenOptions { password: None };
    let r = reading_text(&fixture("article.pdf"), &opts, 0).expect("article");
    assert_eq!(r.page, 0);
    let t = &r.text;
    // Title, subtitle and heading come first; the first column is finished before the second.
    assert!(
        t.starts_with("Annual review of river systems PWARTICLE\nA field study of three catchments\n1 Introduction\n"),
        "{t}"
    );
    assert!(!t.contains("  ") && !t.contains("\n\n"));
    // Every word's offsets are inside the text and in order.
    let n = t.chars().count() as u32;
    assert!(r.words.windows(2).all(|w| w[0].end <= w[1].start) && r.words.iter().all(|w| w.end <= n));
    let marker = word_rects(&r, "PWARTICLE");
    let hits = search(&fixture("article.pdf"), &opts, "PWARTICLE", true, true, &|| false).expect("search").0;
    assert_eq!(marker.len(), 1);
    assert!(overlaps(&marker[0], &hits[0].rects[0]), "{marker:?} vs {:?}", hits[0].rects);

    // A page turned 90°: positions are in the page's own coordinates, like search results.
    let r = reading_text(&fixture("mixed-sizes-rotated.pdf"), &opts, 2).expect("turned");
    assert_eq!(
        r.text,
        "PageWeave fixture page 3\nMarker token PWMARK003\nThe quick brown fox jumps over the lazy dog."
    );
    let m = word_rects(&r, "PWMARK003");
    let hits = search(&fixture("mixed-sizes-rotated.pdf"), &opts, "PWMARK003", true, true, &|| false).expect("s").0;
    assert!(overlaps(&m[0], &hits[0].rects[0]), "{m:?} vs {:?}", hits[0].rects);

    // A table is read cell by cell.
    let r = reading_text(&fixture("tables.pdf"), &opts, 0).expect("tables");
    assert!(r.text.contains("Account\nOpened\nBalance\n007\nSavings\n2026-03-12\n1,234.50"), "{}", r.text);

    // Out of range and wrong password are typed errors.
    assert_eq!(reading_text(&fixture("article.pdf"), &opts, 99).expect_err("range").code, "text.page_out_of_bounds");
    let e = reading_text(&fixture("encrypted-rc4-user-password.pdf"), &opts, 0).expect_err("pw");
    assert_eq!(e.category, ErrorCategory::PasswordRequired);
}

fn speak(
    name: &str,
    pages: Option<&str>,
    rate: f32,
    output: &Path,
) -> pw_core::PwResult<pw_core::ops::OperationResult> {
    let mut req = OperationRequest::new(Operation::ReadAloudToWav {
        input: fixture(name),
        pages: pages.map(str::to_string),
        voice: None,
        rate,
        output: output.into(),
    });
    req.collision = CollisionPolicy::Fail;
    execute(&req, None)
}

fn speech_available() -> bool {
    match pw_speech::engine_info() {
        Ok(_) => true,
        Err(e) => {
            assert!(
                std::env::var("PW_SKIP_SPEECH").as_deref() == Ok("1"),
                "a speech engine is required ({}); set PW_SKIP_SPEECH=1 only where none can be installed",
                e.message
            );
            eprintln!("SKIPPED: speech checks (PW_SKIP_SPEECH=1)");
            false
        }
    }
}

#[test]
fn pages_read_aloud_into_a_verified_wav_and_refusals_write_nothing() {
    if !speech_available() {
        return;
    }
    let dir = tempfile::tempdir().expect("tmp");
    let out = dir.path().join("p3.wav");
    let r = speak("mixed-sizes-rotated.pdf", Some("3"), 1.0, &out).expect("speak");
    let (rate, samples) = pw_speech::wav::decode(&std::fs::read(&out).expect("wav")).expect("decode");
    assert_eq!(r.data["sample_rate"], rate);
    assert_eq!(r.data["sentences"], 3, "{}", r.data);
    let secs = samples.len() as f64 / rate as f64;
    // 14 words: a plausible speaking length, and not silence.
    assert!((2.0..15.0).contains(&secs), "{secs} s");
    let loud = samples.iter().filter(|s| s.unsigned_abs() > 1000).count();
    assert!(loud > samples.len() / 20, "audio is mostly silent");
    // Faster is shorter.
    let fast = dir.path().join("fast.wav");
    speak("mixed-sizes-rotated.pdf", Some("3"), 2.0, &fast).expect("fast");
    let (_, f) = pw_speech::wav::decode(&std::fs::read(&fast).expect("wav")).expect("decode");
    assert!((f.len() as f64) < 0.75 * samples.len() as f64, "{} vs {}", f.len(), samples.len());

    // Scans without text, a bad speed, a wrong name and an existing file are refused.
    let none = dir.path().join("none.wav");
    assert_eq!(speak("scanned.pdf", None, 1.0, &none).expect_err("no text").code, "speech.no_text");
    assert_eq!(speak("five-pages.pdf", None, 2.5, &none).expect_err("rate").code, "speech.rate");
    assert_eq!(
        speak("five-pages.pdf", None, 1.0, &dir.path().join("x.mp3")).expect_err("ext").code,
        "speech.extension"
    );
    assert_eq!(
        speak("mixed-sizes-rotated.pdf", Some("3"), 1.0, &out).expect_err("exists").category,
        ErrorCategory::OutputConflict
    );
    assert!(!none.exists());
    let mut req = OperationRequest::new(Operation::ReadAloudToWav {
        input: fixture("five-pages.pdf"),
        pages: None,
        voice: Some("no-such-voice".into()),
        rate: 1.0,
        output: none.clone(),
    });
    req.collision = CollisionPolicy::Fail;
    assert_eq!(execute(&req, None).expect_err("voice").code, "speech.voice");
    assert!(!none.exists());
}
