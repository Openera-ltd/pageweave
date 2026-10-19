//! Page orientation for scans (PRD 06 "detect orientation"). Upright text reads with high
//! confidence as horizontal lines; the recogniser also reads tall (sideways) boxes by turning their
//! crops, so only horizontal lines count. The page is read in all four quarter turns and a turn is
//! chosen only when it reads clearly better than every other (docs/benchmarks/ORIENTATION.md). The
//! correction is lossless (the page's /Rotate).

use image::{imageops, RgbImage};
use pw_core::PwResult;

/// Longest side of the image recognised for orientation, in pixels.
pub const ORIENT_PIXELS: u32 = 1000;
/// The chosen turn must read at least this much better than the other candidate.
const MIN_MARGIN: f32 = 1.5;
/// ... and amount to at least this many confidently read characters.
const MIN_QUALITY: f32 = 20.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Orientation {
    /// Clockwise quarter turn (0, 90, 180, 270) that makes the text upright as displayed.
    pub turn: u32,
    /// Horizontal-line quality of the chosen turn and of the other candidate.
    pub quality: f32,
    pub runner_up: f32,
}

/// Confidence-weighted characters on horizontal lines, and on tall lines.
fn read(engine: &mut pw_ocr::OcrEngine, img: &RgbImage, cancelled: &dyn Fn() -> bool) -> PwResult<(f32, f32)> {
    let lines = engine.recognize(img.width() as usize, img.height() as usize, img.as_raw(), 3, cancelled)?;
    let (mut horizontal, mut tall) = (0f32, 0f32);
    for l in &lines {
        let d = |a: [f32; 2], b: [f32; 2]| (a[0] - b[0]).hypot(a[1] - b[1]);
        let (w, h) = (d(l.quad[0], l.quad[1]), d(l.quad[1], l.quad[2]));
        let q = l.score * l.text.chars().filter(|c| !c.is_whitespace()).count() as f32;
        if w >= h {
            horizontal += q;
        } else {
            tall += q;
        }
    }
    Ok((horizontal, tall))
}

fn turned(img: &RgbImage, turn: u32) -> RgbImage {
    match turn {
        90 => imageops::rotate90(img),
        180 => imageops::rotate180(img),
        270 => imageops::rotate270(img),
        _ => img.clone(),
    }
}

/// The upright turn of a page image, or `None` when it cannot be told with confidence.
pub fn detect_orientation(
    engine: &mut pw_ocr::OcrEngine,
    img: &RgbImage,
    cancelled: &dyn Fn() -> bool,
) -> PwResult<Option<Orientation>> {
    let k = ORIENT_PIXELS as f32 / img.width().max(img.height()).max(1) as f32;
    let small = if k < 1.0 {
        imageops::resize(
            img,
            (img.width() as f32 * k) as u32,
            (img.height() as f32 * k) as u32,
            imageops::FilterType::Triangle,
        )
    } else {
        img.clone()
    };
    // All four turns are read: shortcuts from fewer readings were wrong on skewed pages, where text a
    // quarter turn off still reads partly (val corpus, docs/benchmarks/ORIENTATION.md).
    let mut q = [0f32; 4];
    for (i, t) in [0u32, 90, 180, 270].into_iter().enumerate() {
        q[i] = read(engine, &turned(&small, t), cancelled)?.0;
    }
    let mut order = [0usize, 1, 2, 3];
    order.sort_by(|&x, &y| q[y].total_cmp(&q[x]));
    let (turn, best, other) = (order[0] as u32 * 90, q[order[0]], q[order[1]]);
    Ok((best >= MIN_QUALITY && best >= other * MIN_MARGIN).then_some(Orientation {
        turn,
        quality: best,
        runner_up: other,
    }))
}

// ---- operation ------------------------------------------------------------------------------

use crate::document::OpenOptions;
use crate::sanitize::load_plain;
use crate::{engine, map_pdfium_error};
use pdfium_render::prelude::*;
use pw_core::output::{write_atomic_bytes, CollisionPolicy, CommittedOutput};
use pw_core::{ErrorCategory, PwError};
use serde::Serialize;
use std::path::Path;

/// Pages with more visible text characters than this are born-digital: left as they are.
const MAX_VISIBLE_CHARS: usize = 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OrientAction {
    /// Turned upright by `turn` degrees clockwise.
    Turned,
    /// Already upright.
    Upright,
    /// Has visible text of its own: a born-digital page, left as it is.
    HasText,
    /// No clear answer (too little text, a picture, or similar readings): left as it is.
    Undecided,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OrientPage {
    /// 0-based page index.
    pub page: u32,
    pub action: OrientAction,
    /// Clockwise turn applied (degrees).
    pub turn: u32,
}

fn fail(what: &str) -> PwError {
    PwError::new(
        ErrorCategory::ValidationFailed,
        "output.verify",
        format!("The turned copy failed verification ({what}) and was not saved."),
    )
}

fn render_rgb(page: &PdfPage) -> PwResult<RgbImage> {
    let longest = page.width().value.max(page.height().value).max(1.0);
    let config = PdfRenderConfig::new()
        .scale_page_by_factor(ORIENT_PIXELS as f32 / longest)
        .render_form_data(false)
        .render_annotations(false);
    let bitmap = page.render_with_config(&config).map_err(|e| map_pdfium_error(&e, "render"))?;
    Ok(bitmap.as_image().map_err(|e| map_pdfium_error(&e, "render"))?.to_rgb8())
}

fn rotation_degrees(r: PdfPageRenderRotation) -> u32 {
    match r {
        PdfPageRenderRotation::None => 0,
        PdfPageRenderRotation::Degrees90 => 90,
        PdfPageRenderRotation::Degrees180 => 180,
        PdfPageRenderRotation::Degrees270 => 270,
    }
}

/// Turn the chosen (or all) scanned pages upright in a new file, by changing each page's /Rotate.
pub fn fix_orientation(
    input: &Path,
    opts: &OpenOptions,
    pages: Option<&[u32]>,
    output: &Path,
    policy: CollisionPolicy,
    cancelled: &dyn Fn() -> bool,
    progress: &dyn Fn(u64, u64),
) -> PwResult<(CommittedOutput, Vec<OrientPage>, Vec<String>)> {
    let resources = pw_ocr::default_resource_dir().ok_or_else(|| {
        PwError::new(
            ErrorCategory::MissingResource,
            "ocr.resource_missing",
            "Text recognition is not installed, and it is needed to tell which way pages face.",
        )
        .with_action("Reinstall PageWeave or run the resource setup.")
    })?;
    let pdfium = engine()?;
    let (source, mut doc, _raw) = load_plain(&pdfium, input, opts).map_err(|e| {
        if e.code == "sanitize.protected" {
            PwError::new(ErrorCategory::UnsupportedOperation, "orient.protected", "This file is password protected.")
                .with_action("Remove the password first (Remove password…), then turn that copy.")
        } else {
            e
        }
    })?;
    let n = source.pages().len().max(0) as u32;
    let chosen: Vec<u32> = match pages {
        Some(p) => p.to_vec(),
        None => (0..n).collect(),
    };
    let page_ids = doc.get_pages();
    let mut report = Vec::new();
    let mut planned: Vec<(u32, u32)> = Vec::new(); // (page, new /Rotate)
    for (k, &i) in chosen.iter().enumerate() {
        if cancelled() {
            return Err(PwError::cancelled());
        }
        let page = source.pages().get(i as PdfPageIndex).map_err(|e| map_pdfium_error(&e, "page"))?;
        progress(k as u64, chosen.len() as u64);
        if crate::deskew::visible_chars(&page) > MAX_VISIBLE_CHARS {
            report.push(OrientPage { page: i, action: OrientAction::HasText, turn: 0 });
            continue;
        }
        let img = render_rgb(&page)?;
        let found = pw_ocr::with_engine(&resources, |e| detect_orientation(e, &img, cancelled))?;
        match found {
            None => report.push(OrientPage { page: i, action: OrientAction::Undecided, turn: 0 }),
            Some(o) if o.turn == 0 => report.push(OrientPage { page: i, action: OrientAction::Upright, turn: 0 }),
            Some(o) => {
                let current = page.rotation().map(rotation_degrees).unwrap_or(0);
                let new = (current + o.turn) % 360;
                let Some(&id) = page_ids.get(&(i + 1)) else { return Err(fail("page missing")) };
                doc.get_dictionary_mut(id).map_err(|_| fail("page unreadable"))?.set("Rotate", i64::from(new));
                planned.push((i, new));
                report.push(OrientPage { page: i, action: OrientAction::Turned, turn: o.turn });
            }
        }
    }
    progress(chosen.len() as u64, chosen.len() as u64);
    if planned.is_empty() {
        return Err(PwError::new(
            ErrorCategory::UnsupportedOperation,
            "orient.nothing",
            "No page needed turning, so nothing was saved.",
        )
        .with_action("Pages with text of their own, pages already upright and pages without enough text to tell are left as they are."));
    }
    doc.trailer.remove(b"Prev");
    doc.trailer.remove(b"XRefStm");
    let mut bytes = Vec::new();
    doc.save_to(&mut bytes).map_err(|_| PwError::internal("orient.save", "The turned copy could not be written."))?;
    // Verify: same page count, every turned page has its new rotation, the others kept theirs.
    {
        let out = pdfium.load_pdf_from_byte_slice(&bytes, None).map_err(|_| fail("it could not be reopened"))?;
        if out.pages().len() as u32 != n {
            return Err(fail("page count changed"));
        }
        for i in 0..n {
            let want = match planned.iter().find(|p| p.0 == i) {
                Some(&(_, r)) => r,
                None => source
                    .pages()
                    .get(i as PdfPageIndex)
                    .ok()
                    .and_then(|p| p.rotation().ok())
                    .map(rotation_degrees)
                    .unwrap_or(0),
            };
            let got = out.pages().get(i as PdfPageIndex).ok().and_then(|p| p.rotation().ok()).map(rotation_degrees);
            if got != Some(want) {
                return Err(fail(&format!("page {} has the wrong rotation", i + 1)));
            }
        }
    }
    let c = write_atomic_bytes(output, policy, &[input.to_path_buf()], &bytes, |staged| {
        let back = std::fs::read(staged).map_err(|e| PwError::from_io(&e, "output file"))?;
        if back == bytes {
            Ok(())
        } else {
            Err(fail("the written file differs"))
        }
    })?;
    let mut warnings = Vec::new();
    let signatures = source.signatures().len();
    if signatures > 0 {
        warnings.push(crate::properties::signature_warning(signatures));
    }
    Ok((c, report, warnings))
}
