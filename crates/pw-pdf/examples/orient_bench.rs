//! Orientation detection on corpus pages turned 0/90/180/270° (development benchmark; val only —
//! blind is scored once). Usage:
//!   cargo run -p pw-pdf --release --example orient_bench -- ml/out/corpus-v1/val
use image::imageops;

fn main() {
    let dir = std::env::args().nth(1).expect("corpus split directory");
    // Release builds only look next to the executable; the benchmark names the folder itself.
    let res = std::env::var_os("PAGEWEAVE_OCR_DIR")
        .map(std::path::PathBuf::from)
        .or_else(pw_ocr::default_resource_dir)
        .expect("OCR resources");
    let mut entries: Vec<_> = std::fs::read_dir(&dir).expect("dir").flatten().map(|e| e.path()).collect();
    entries.sort();
    let (mut right, mut wrong, mut undecided, mut n) = (0, 0, 0, 0);
    let mut ms_total = 0u128;
    pw_ocr::with_engine(&res, |engine| {
        for png in entries.iter().filter(|p| p.extension().is_some_and(|e| e == "png")) {
            let img = image::open(png).expect("png").into_rgb8();
            for r in [0u32, 90, 180, 270] {
                // Turn the upright page clockwise by r; the correction is (360 - r) % 360.
                let turned = match r {
                    90 => imageops::rotate90(&img),
                    180 => imageops::rotate180(&img),
                    270 => imageops::rotate270(&img),
                    _ => img.clone(),
                };
                let want = (360 - r) % 360;
                let t = std::time::Instant::now();
                let got = pw_pdf::orient::detect_orientation(engine, &turned, &|| false)?;

                ms_total += t.elapsed().as_millis();
                n += 1;
                match got {
                    Some(o) if o.turn == want => right += 1,
                    Some(o) => {
                        wrong += 1;
                        println!(
                            "WRONG {} turned {r}: got {} ({} vs {})",
                            png.display(),
                            o.turn,
                            o.quality,
                            o.runner_up
                        );
                    }
                    None => {
                        undecided += 1;
                        println!("UNDECIDED {} turned {r}", png.display());
                    }
                }
            }
        }
        Ok(())
    })
    .expect("bench");
    println!(
        "cases {n}: right {right}, wrong {wrong}, undecided {undecided}; {} ms per page",
        ms_total / n.max(1) as u128
    );
}
