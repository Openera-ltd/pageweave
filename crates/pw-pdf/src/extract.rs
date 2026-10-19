//! Extract the images embedded in a document (PRD 01 "extract images"). JPEG images are saved
//! exactly as stored (no re-encoding); every other image is decoded to its own pixel size and saved
//! as PNG. An image used several times (a logo on every page) is saved once. Images inside form
//! XObjects are included. Every written file is read back and decoded before it counts.

use crate::document::{load, OpenOptions};
use crate::{engine, map_pdfium_error};
use pdfium_render::prelude::*;
use pw_core::output::{write_atomic_bytes, CollisionPolicy, CommittedOutput};
use pw_core::{ErrorCategory, PwError, PwResult};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::path::Path;

/// Nesting limit for form XObjects (hostile files).
const MAX_DEPTH: usize = 8;
/// Images extracted at most per run.
pub const MAX_EXTRACTED_IMAGES: usize = 5000;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ExtractedImage {
    /// 0-based page of its first use.
    pub page: u32,
    pub file: String,
    pub format: &'static str,
    pub width: u32,
    pub height: u32,
    /// Saved exactly as stored in the document.
    pub exact: bool,
}

enum Encoded {
    Jpeg(Vec<u8>),
    Png(Vec<u8>),
}

fn collect<'a>(objects: impl Iterator<Item = PdfPageObject<'a>>, depth: usize, out: &mut Vec<PdfPageImageObject<'a>>) {
    for o in objects {
        match o {
            PdfPageObject::Image(img) => out.push(img),
            PdfPageObject::XObjectForm(form) if depth < MAX_DEPTH => {
                let inner: Vec<PdfPageObject<'a>> = (0..form.len()).filter_map(|i| form.get(i).ok()).collect();
                collect(inner.into_iter(), depth + 1, out);
            }
            _ => {}
        }
    }
}

fn encode(img: &PdfPageImageObject) -> Option<(Encoded, u32, u32, bool)> {
    let filters: Vec<String> = img.filters().iter().map(|f| f.name().to_string()).collect();
    let rgb_or_gray = matches!(img.color_space(), Ok(PdfColorSpace::DeviceRGB | PdfColorSpace::DeviceGray));
    if filters == ["DCTDecode"] && rgb_or_gray {
        let raw = img.get_raw_image_data().ok()?;
        if let Ok(decoded) = image::load_from_memory_with_format(&raw, image::ImageFormat::Jpeg) {
            return Some((Encoded::Jpeg(raw), decoded.width(), decoded.height(), true));
        }
    }
    let decoded = img.get_raw_image().ok()?;
    let mut png = Vec::new();
    decoded.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).ok()?;
    Some((Encoded::Png(png), decoded.width(), decoded.height(), false))
}

/// Save every embedded image of the chosen (or all) pages into `output_dir`.
pub fn extract_images(
    input: &Path,
    opts: &OpenOptions,
    pages: Option<&[u32]>,
    output_dir: &Path,
    policy: CollisionPolicy,
    cancelled: &dyn Fn() -> bool,
    progress: &dyn Fn(u64, u64),
) -> PwResult<(Vec<CommittedOutput>, Vec<ExtractedImage>, Vec<String>)> {
    let pdfium = engine()?;
    let doc = load(&pdfium, input, opts)?;
    let n = doc.pages().len().max(0) as u32;
    let chosen: Vec<u32> = match pages {
        Some(p) => p.to_vec(),
        None => (0..n).collect(),
    };
    let stem = input.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "document".into());
    let width = n.to_string().len().max(3);
    let mut seen: HashSet<[u8; 32]> = HashSet::new();
    let mut written: Vec<CommittedOutput> = Vec::new();
    let mut report: Vec<ExtractedImage> = Vec::new();
    let mut unreadable = 0usize;
    let result = (|| -> PwResult<()> {
        for (k, &i) in chosen.iter().enumerate() {
            if cancelled() {
                return Err(PwError::cancelled());
            }
            let page = doc.pages().get(i as PdfPageIndex).map_err(|e| map_pdfium_error(&e, "page"))?;
            let mut images = Vec::new();
            collect(page.objects().iter(), 0, &mut images);
            let mut on_page = 0usize;
            for img in &images {
                let raw = img.get_raw_image_data().unwrap_or_default();
                let key: [u8; 32] = Sha256::digest(&raw).into();
                if !raw.is_empty() && !seen.insert(key) {
                    continue; // the same image again
                }
                let Some((encoded, w, h, exact)) = encode(img) else {
                    unreadable += 1;
                    continue;
                };
                if report.len() >= MAX_EXTRACTED_IMAGES {
                    return Err(PwError::new(
                        ErrorCategory::UnsupportedOperation,
                        "extract.too_many",
                        format!("More than {MAX_EXTRACTED_IMAGES} images: extract fewer pages at a time."),
                    ));
                }
                on_page += 1;
                let (bytes, ext, format, fmt) = match encoded {
                    Encoded::Jpeg(b) => (b, "jpg", "jpeg", image::ImageFormat::Jpeg),
                    Encoded::Png(b) => (b, "png", "png", image::ImageFormat::Png),
                };
                let name = output_dir.join(format!("{stem}_p{:0width$}_{on_page:02}.{ext}", i + 1));
                let c = write_atomic_bytes(&name, policy, &[input.to_path_buf()], &bytes, |staged| {
                    let back = std::fs::read(staged).map_err(|e| PwError::from_io(&e, "output file"))?;
                    match image::load_from_memory_with_format(&back, fmt) {
                        Ok(d) if back == bytes && d.width() == w && d.height() == h => Ok(()),
                        _ => Err(PwError::new(
                            ErrorCategory::ValidationFailed,
                            "output.verify",
                            "An extracted image failed verification and was not saved.",
                        )),
                    }
                })?;
                report.push(ExtractedImage {
                    page: i,
                    file: c.path.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default(),
                    format,
                    width: w,
                    height: h,
                    exact,
                });
                written.push(c);
            }
            progress(k as u64 + 1, chosen.len() as u64);
        }
        Ok(())
    })();
    if let Err(e) = result {
        for c in &written {
            let _ = std::fs::remove_file(&c.path);
        }
        return Err(e);
    }
    if written.is_empty() {
        return Err(PwError::new(
            ErrorCategory::UnsupportedOperation,
            "extract.none",
            if unreadable > 0 {
                "The images in these pages could not be read, so nothing was saved."
            } else {
                "These pages contain no embedded images, so nothing was saved."
            },
        )
        .with_action("To save whole pages as pictures, use Export pages as images."));
    }
    let mut warnings = Vec::new();
    if unreadable > 0 {
        warnings.push(format!("{unreadable} image(s) could not be read and were not saved."));
    }
    if report.iter().any(|r| !r.exact) {
        warnings.push(
            "Images that were not JPEG are saved as PNG at their own size; transparency masks are not applied.".into(),
        );
    }
    Ok((written, report, warnings))
}
