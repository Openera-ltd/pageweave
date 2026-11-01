//! Translate a file line by line (benchmarks): `eval <src> <tgt> <in.txt> <out.txt>`.
//! Prints sentences per second.

fn main() {
    let a: Vec<String> = std::env::args().collect();
    if a.len() != 5 {
        eprintln!("usage: eval <src> <tgt> <in.txt> <out.txt>");
        std::process::exit(2);
    }
    // Release builds only look next to the executable: give the folders explicitly.
    let dir = std::path::PathBuf::from(std::env::var_os("PAGEWEAVE_TRANSLATE_DIR").expect("PAGEWEAVE_TRANSLATE_DIR"));
    let ocr = std::path::PathBuf::from(std::env::var_os("PAGEWEAVE_OCR_DIR").expect("PAGEWEAVE_OCR_DIR"));
    // PAGEWEAVE_EVAL_PAIR_DIR: a candidate model's folder, used without pinned checksums.
    let mut t = match std::env::var_os("PAGEWEAVE_EVAL_PAIR_DIR") {
        Some(d) => pw_translate::Translator::from_files(std::path::Path::new(&d), &ocr, &format!("{}-{}", a[1], a[2])),
        None => pw_translate::Translator::load_with_runtime(&dir, &ocr, &a[1], &a[2]),
    }
    .expect("load");
    // PAGEWEAVE_EVAL_BEAM: beam width (1 = greedy decoding, as before D-030).
    if let Some(b) = std::env::var("PAGEWEAVE_EVAL_BEAM").ok().and_then(|b| b.parse().ok()) {
        t.set_beam(b);
    }
    let input = std::fs::read_to_string(&a[3]).expect("read");
    let started = std::time::Instant::now();
    let mut out = String::new();
    let lines: Vec<&str> = input.lines().collect();
    for l in &lines {
        out.push_str(&t.text(l, &|| false, &|_, _| {}).expect("translate"));
        out.push('\n');
    }
    std::fs::write(&a[4], out).expect("write");
    let secs = started.elapsed().as_secs_f64();
    println!("{} lines in {:.1} s ({:.2} lines/s)", lines.len(), secs, lines.len() as f64 / secs);
}
