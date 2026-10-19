//! Parity harness: run the engine over every PNG in a directory and write raw results in the
//! benchmark cache format (ml/pwml/bench.py `--rescore`), so the Rust pipeline is scored by the
//! same scorer as the reference.
//!
//! Usage: cargo run --release -p pw-ocr --example ocr_dir -- IMAGE_DIR OUT_RAW_DIR [RESOURCE_DIR]
use std::path::PathBuf;
use std::time::Instant;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("usage: ocr_dir IMAGE_DIR OUT_RAW_DIR [RESOURCE_DIR]");
        std::process::exit(2);
    }
    let (input, out) = (PathBuf::from(&args[1]), PathBuf::from(&args[2]));
    let res = args.get(3).map(PathBuf::from).or_else(pw_ocr::default_resource_dir).ok_or("OCR resources not found")?;
    std::fs::create_dir_all(&out)?;
    let mut pages: Vec<PathBuf> = std::fs::read_dir(&input)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "png"))
        .collect();
    pages.sort();
    let load = |p: &PathBuf| -> Result<(usize, usize, Vec<u8>), Box<dyn std::error::Error>> {
        let img = image::open(p)?.to_rgb8();
        Ok((img.width() as usize, img.height() as usize, img.into_raw()))
    };
    let t0 = Instant::now();
    pw_ocr::with_engine(&res, |e| {
        let (w, h, px) = load(&pages[0]).map_err(|e| pw_core::PwError::internal("x", e.to_string()))?;
        e.recognize(w, h, &px, 3, &|| false).map(|_| ())
    })?;
    let warmup = t0.elapsed().as_secs_f64();
    std::fs::write(out.join("_run.json"), serde_json::json!({"info": {"engine": "pw-ocr (Rust, ONNX Runtime 1.31, official PP-OCRv6 small)"}, "warmup_s": warmup}).to_string())?;
    for p in &pages {
        let (w, h, px) = load(p)?;
        let t = Instant::now();
        let lines = pw_ocr::with_engine(&res, |e| e.recognize(w, h, &px, 3, &|| false))?;
        let secs = t.elapsed().as_secs_f64();
        let rows: Vec<_> =
            lines.iter().map(|l| serde_json::json!({"text": l.text, "poly": l.quad, "conf": l.score})).collect();
        let id = p.file_stem().and_then(|s| s.to_str()).unwrap_or("page");
        std::fs::write(
            out.join(format!("{id}.json")),
            serde_json::json!({"seconds": secs, "lines": rows}).to_string(),
        )?;
    }
    eprintln!("{} pages, warm-up {:.2}s", pages.len(), warmup);
    Ok(())
}
