//! Hostile input: deterministic mutations of every fixture (truncations, byte flips, splices,
//! number and length tampering) fed to every read-only operation. Each must finish with a result
//! or a typed error — no panic, no hang. `PW_FUZZ_CASES` raises the number of cases per fixture
//! (default kept small for CI); `PW_FUZZ_SEED` changes the seed.

use pw_core::ops::{Operation, OperationRequest, TextPattern};
use pw_pdf::execute;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

fn fixtures() -> Vec<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/pdf");
    let mut v: Vec<PathBuf> = std::fs::read_dir(&dir)
        .expect("fixtures")
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "pdf"))
        // Large image fixtures make each case slow without exercising more parsing.
        .filter(|p| std::fs::metadata(p).map(|m| m.len() < 300_000).unwrap_or(false))
        .collect();
    v.sort();
    v
}

/// xorshift64*: small, deterministic, no dependency.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next() % n as u64) as usize
        }
    }
}

fn mutate(src: &[u8], rng: &mut Rng) -> Vec<u8> {
    let mut b = src.to_vec();
    match rng.below(6) {
        // Truncate anywhere.
        0 => b.truncate(rng.below(b.len().max(1))),
        // Flip a few random bytes.
        1 => {
            for _ in 0..1 + rng.below(16) {
                let i = rng.below(b.len());
                if i < b.len() {
                    b[i] ^= 1 << rng.below(8);
                }
            }
        }
        // Splice a chunk of the file somewhere else (duplicated objects, broken xref offsets).
        2 => {
            let a = rng.below(b.len());
            let len = rng.below(512).min(b.len() - a);
            let chunk = b[a..a + len].to_vec();
            let at = rng.below(b.len());
            b.splice(at..at, chunk);
        }
        // Tamper with a number (lengths, offsets, counts, object numbers).
        3 => {
            let digits: Vec<usize> = (0..b.len()).filter(|&i| b[i].is_ascii_digit()).collect();
            if !digits.is_empty() {
                let i = digits[rng.below(digits.len())];
                let huge = b"99999999999";
                let repl: &[u8] = match rng.below(4) {
                    0 => b"0",
                    1 => b"-1",
                    2 => huge,
                    _ => b"4294967296",
                };
                b.splice(i..i + 1, repl.iter().copied());
            }
        }
        // Replace a keyword with another (obj/endobj/stream/R/xref…).
        4 => {
            let words: [&[u8]; 8] = [b"obj", b"endobj", b"stream", b"endstream", b" R", b"xref", b"trailer", b"/Kids"];
            let from = words[rng.below(words.len())];
            let to = words[rng.below(words.len())];
            if let Some(pos) = b.windows(from.len()).position(|w| w == from) {
                b.splice(pos..pos + from.len(), to.iter().copied());
            }
        }
        // Zero a block.
        _ => {
            let a = rng.below(b.len());
            let len = rng.below(256).min(b.len() - a);
            for x in &mut b[a..a + len] {
                *x = 0;
            }
        }
    }
    b
}

fn read_only_ops(p: &Path) -> Vec<Operation> {
    let input = p.to_path_buf();
    vec![
        Operation::Inspect { input: input.clone() },
        Operation::ExtractText { input: input.clone(), pages: None },
        Operation::Search { input: input.clone(), query: "a".into(), match_case: false, whole_word: false },
        Operation::FormFields { input: input.clone() },
        Operation::ListAnnotations { input: input.clone() },
        Operation::ListAttachments { input: input.clone() },
        Operation::SanitizeInspect { input: input.clone() },
        Operation::CheckAccessibility { input: input.clone() },
        Operation::FindPatterns { input: input.clone(), patterns: vec![TextPattern::Email], custom: None, pages: None },
        Operation::VerifySignatures { input, trusted: vec![] },
    ]
}

#[test]
fn mutated_fixtures_never_panic_or_hang() {
    let cases: usize = std::env::var("PW_FUZZ_CASES").ok().and_then(|v| v.parse().ok()).unwrap_or(6);
    let seed: u64 = std::env::var("PW_FUZZ_SEED").ok().and_then(|v| v.parse().ok()).unwrap_or(0x5eed_1234_abcd);
    let dir = tempfile::tempdir().expect("tmp");
    let mut rng = Rng(seed);
    let mut failures = Vec::new();
    let mut runs = 0usize;
    let mut slowest = (Duration::ZERO, String::new());
    for fx in fixtures() {
        let src = std::fs::read(&fx).expect("read");
        if src.is_empty() {
            continue;
        }
        for case in 0..cases {
            let bytes = mutate(&src, &mut rng);
            let path = dir.path().join(format!("m{case}.pdf"));
            std::fs::write(&path, &bytes).expect("write");
            for op in read_only_ops(&path) {
                let id = op.id();
                let req = OperationRequest::new(op);
                let t = Instant::now();
                let r = catch_unwind(AssertUnwindSafe(|| execute(&req, None)));
                let took = t.elapsed();
                runs += 1;
                let what = format!(
                    "{} case {case} {id}",
                    fx.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
                );
                if took > slowest.0 {
                    slowest = (took, what.clone());
                }
                if r.is_err() {
                    // Keep the input for reproduction.
                    let keep = dir.path().join(format!("panic-{}.pdf", failures.len()));
                    let _ = std::fs::copy(&path, &keep);
                    failures.push(format!("{what}: panicked"));
                } else if took > Duration::from_secs(20) {
                    failures.push(format!("{what}: took {took:?}"));
                }
            }
        }
    }
    eprintln!("robustness: {runs} runs, slowest {:?} ({})", slowest.0, slowest.1);
    assert!(failures.is_empty(), "seed {seed:#x}: {failures:#?}");
}
