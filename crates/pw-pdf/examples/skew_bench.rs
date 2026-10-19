//! Skew measurement against corpus ground truth (development benchmark; val split only — the blind
//! split is reported once, never tuned on). Usage:
//!   cargo run -p pw-pdf --release --example skew_bench -- ml/out/corpus-v1/val
use image::imageops::FilterType;
use std::path::Path;

fn main() {
    let dir = std::env::args().nth(1).expect("corpus split directory");
    let mut rows = Vec::new();
    let mut entries: Vec<_> = std::fs::read_dir(&dir).expect("dir").flatten().map(|e| e.path()).collect();
    entries.sort();
    for json in entries.iter().filter(|p| p.extension().is_some_and(|e| e == "json")) {
        let truth: serde_json::Value = serde_json::from_slice(&std::fs::read(json).expect("read")).expect("json");
        let want = truth["params"]["rotate_deg"].as_f64().unwrap_or(0.0) as f32;
        let png = json.with_extension("png");
        if !Path::new(&png).exists() {
            continue;
        }
        let img = image::open(&png).expect("png").into_luma8();
        // Comparable to the engine's page render (longest side ~1400 px).
        let k = 1400.0 / img.width().max(img.height()) as f32;
        let small = image::imageops::resize(
            &img,
            (img.width() as f32 * k) as u32,
            (img.height() as f32 * k) as u32,
            FilterType::Triangle,
        );
        let t = std::time::Instant::now();
        let got = pw_pdf::deskew::measure_skew(&small);
        let ms = t.elapsed().as_millis();
        let angle = got.map(|s| s.angle).unwrap_or(f32::NAN);
        println!(
            "{} want {want:+.2} got {angle:+.2} conf {:.2} {ms} ms",
            json.file_stem().unwrap_or_default().to_string_lossy(),
            got.map(|s| s.confidence).unwrap_or(0.0)
        );
        rows.push((want, got));
    }
    let skewed: Vec<_> = rows.iter().filter(|r| r.0 != 0.0).collect();
    let straight: Vec<_> = rows.iter().filter(|r| r.0 == 0.0).collect();
    let err: Vec<f32> = skewed.iter().map(|r| r.1.map(|s| (s.angle - r.0).abs()).unwrap_or(f32::INFINITY)).collect();
    let max_err = err.iter().cloned().fold(0.0, f32::max);
    let mean_err = err.iter().sum::<f32>() / err.len().max(1) as f32;
    let false_pos =
        straight.iter().filter(|r| r.1.is_some_and(|s| s.angle.abs() >= pw_pdf::deskew::STRAIGHT_DEG)).count();
    println!(
        "skewed pages {}: mean abs error {mean_err:.3}°, max {max_err:.3}°; straight pages {}: {} would be turned",
        skewed.len(),
        straight.len(),
        false_pos
    );
}
