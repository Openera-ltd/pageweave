//! Opt-in image downsampling for "Reduce file size": images shown at more than the chosen
//! resolution are resampled to it and stored as JPEG at the chosen quality.
//!
//! The size an image is shown at comes from the page content: the transformation in effect at each
//! `Do` (through nested form XObjects and their matrices) maps the image's unit square onto the page,
//! so its side lengths are the displayed width and height. The largest placement wins. An image is
//! only changed when every use of it is known: images reachable from annotation appearances,
//! patterns, Type 3 glyphs, or pages whose content cannot be read are left as they are, as are
//! images without a supported layout (8-bit gray/RGB, no decode array, no colour-key/stencil mask,
//! an optional unshared 8-bit gray soft mask without /Matte).

use crate::content_ops::tokenize;
use crate::sanitize::{decode_name, stream_data};
use image::imageops::FilterType;
use image::{GrayImage, RgbImage};
use lopdf::{Dictionary, Document, Object, ObjectId, Stream};
use pw_core::{ErrorCategory, PwError, PwResult};
use std::collections::{BTreeSet, HashMap, HashSet};

/// Nesting limit for form XObjects (hostile files).
const MAX_FORM_DEPTH: usize = 12;
/// Images are only resampled when that removes a meaningful share of their pixels.
const MIN_SCALE_GAIN: f32 = 0.8;
/// Largest mean difference (0–255) between a re-encoded image and the resampled pixels. Measured on
/// photo-heavy.pdf: at most 8.6 at the lowest allowed setting (36 dpi, quality 30), under 1.4 at
/// 150 dpi / quality 75; layout mistakes (garbled rows, swapped channels) are far above.
const MAX_JPEG_MEAN_ERROR: f64 = 20.0;
/// Largest decoded image considered (pixels × components).
const MAX_IMAGE_BYTES: usize = 512 * 1024 * 1024;

type Mat = [f32; 6];
const IDENTITY: Mat = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

/// `m` applied first, then `n` (PDF row-vector convention: `cm` sets CTM = M × CTM).
fn mul(m: &Mat, n: &Mat) -> Mat {
    [
        m[0] * n[0] + m[1] * n[2],
        m[0] * n[1] + m[1] * n[3],
        m[2] * n[0] + m[3] * n[2],
        m[2] * n[1] + m[3] * n[3],
        m[4] * n[0] + m[5] * n[2] + n[4],
        m[4] * n[1] + m[5] * n[3] + n[5],
    ]
}

fn num(tok: &[u8]) -> Option<f32> {
    std::str::from_utf8(tok).ok()?.parse::<f32>().ok().filter(|v| v.is_finite())
}

fn matrix_of(o: Option<&Object>) -> Mat {
    let Some(Object::Array(a)) = o else { return IDENTITY };
    if a.len() != 6 {
        return IDENTITY;
    }
    let mut m = IDENTITY;
    for (i, v) in a.iter().enumerate() {
        m[i] = v.as_float().ok().or_else(|| v.as_i64().ok().map(|x| x as f32)).unwrap_or(m[i]);
    }
    m
}

fn deref<'a>(doc: &'a Document, o: &'a Object) -> &'a Object {
    match o {
        Object::Reference(id) => doc.get_object(*id).unwrap_or(o),
        _ => o,
    }
}

fn name_is(d: &Dictionary, key: &[u8], value: &[u8]) -> bool {
    d.get(key).ok().and_then(|o| o.as_name().ok()) == Some(value)
}

fn is_image(doc: &Document, id: ObjectId) -> bool {
    matches!(doc.get_object(id), Ok(Object::Stream(s)) if name_is(&s.dict, b"Subtype", b"Image"))
}

/// Where each image is shown: largest displayed width and height (points) and the pages (1-based).
#[derive(Debug, Default, Clone)]
pub(crate) struct Placement {
    pub width_pt: f32,
    pub height_pt: f32,
    pub pages: BTreeSet<u32>,
}

struct Scan<'a> {
    doc: &'a Document,
    placed: HashMap<ObjectId, Placement>,
    /// Images whose uses are not all known (left as they are).
    unknown: HashSet<ObjectId>,
}

impl Scan<'_> {
    fn xobject(&self, resources: &[&Dictionary], name: &[u8]) -> Option<ObjectId> {
        resources.iter().find_map(|r| {
            let xo = r.get(b"XObject").ok().map(|o| deref(self.doc, o))?.as_dict().ok()?;
            xo.get(name).ok()?.as_reference().ok()
        })
    }

    /// Mark every image reachable from `o` (by references, any depth) as having unknown uses.
    fn taint(&mut self, o: &Object, seen: &mut HashSet<ObjectId>) {
        match o {
            Object::Reference(id) => {
                if !seen.insert(*id) {
                    return;
                }
                if is_image(self.doc, *id) {
                    self.unknown.insert(*id);
                }
                if let Ok(t) = self.doc.get_object(*id) {
                    self.taint(t, seen);
                }
            }
            Object::Array(a) => a.iter().for_each(|x| self.taint(x, seen)),
            Object::Dictionary(d) => {
                for (k, v) in d.iter() {
                    // Never walk up to the page tree or across to other annotations.
                    if k.as_slice() != b"P" && k.as_slice() != b"Parent" {
                        self.taint(v, seen);
                    }
                }
            }
            Object::Stream(s) => self.taint(&Object::Dictionary(s.dict.clone()), seen),
            _ => {}
        }
    }

    fn content(&mut self, data: &[u8], resources: &[&Dictionary], ctm: Mat, page: u32, depth: usize) -> bool {
        let Ok(ops) = tokenize(data) else { return false };
        let mut stack: Vec<Mat> = Vec::new();
        let mut m = ctm;
        let mut ok = true;
        for op in ops {
            match op.operator {
                b"q" => stack.push(m),
                b"Q" => m = stack.pop().unwrap_or(ctm),
                b"cm" if op.operands.len() == 6 => {
                    let v: Vec<f32> = op.operands.iter().filter_map(|t| num(t)).collect();
                    if v.len() != 6 {
                        return false;
                    }
                    m = mul(&[v[0], v[1], v[2], v[3], v[4], v[5]], &m);
                }
                b"Do" => {
                    let Some(id) =
                        op.operands.first().and_then(|t| decode_name(t)).and_then(|n| self.xobject(resources, &n))
                    else {
                        continue;
                    };
                    match self.doc.get_object(id) {
                        Ok(Object::Stream(s)) if name_is(&s.dict, b"Subtype", b"Image") => {
                            let p = self.placed.entry(id).or_default();
                            p.width_pt = p.width_pt.max(m[0].hypot(m[1]));
                            p.height_pt = p.height_pt.max(m[2].hypot(m[3]));
                            p.pages.insert(page);
                        }
                        Ok(Object::Stream(s)) if name_is(&s.dict, b"Subtype", b"Form") => {
                            let inner = mul(&matrix_of(s.dict.get(b"Matrix").ok()), &m);
                            let own = s
                                .dict
                                .get(b"Resources")
                                .ok()
                                .map(|o| deref(self.doc, o))
                                .and_then(|o| o.as_dict().ok());
                            let res: Vec<&Dictionary> = match own {
                                Some(d) => vec![d],
                                None => resources.to_vec(),
                            };
                            let readable = depth < MAX_FORM_DEPTH
                                && stream_data(s).is_some_and(|d| self.content(&d, &res, inner, page, depth + 1));
                            if !readable {
                                ok = false;
                                let mut seen = HashSet::new();
                                self.taint(&Object::Reference(id), &mut seen);
                            }
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }
        ok
    }
}

/// Every image's largest placement, and the images whose uses are not all known.
pub(crate) fn placements(doc: &Document) -> (HashMap<ObjectId, Placement>, HashSet<ObjectId>) {
    let mut scan = Scan { doc, placed: HashMap::new(), unknown: HashSet::new() };
    for (n, page_id) in doc.get_pages() {
        let (direct, refs) = doc.get_page_resources(page_id).unwrap_or((None, Vec::new()));
        let mut res: Vec<&Dictionary> = direct.into_iter().collect();
        res.extend(refs.iter().filter_map(|id| doc.get_dictionary(*id).ok()));
        let mut data = Vec::new();
        let mut readable = true;
        for id in doc.get_page_contents(page_id) {
            match doc.get_object(id) {
                Ok(Object::Stream(s)) => match stream_data(s) {
                    Some(d) => {
                        data.extend_from_slice(&d);
                        data.push(b'\n');
                    }
                    None => readable = false,
                },
                _ => readable = false,
            }
        }
        if !(readable && scan.content(&data, &res, IDENTITY, n, 0)) {
            let mut seen = HashSet::new();
            for r in &res {
                if let Ok(xo) = r.get(b"XObject") {
                    scan.taint(xo, &mut seen);
                }
            }
        }
        // Annotation appearances: placement not tracked.
        if let Ok(page) = doc.get_dictionary(page_id) {
            if let Ok(annots) = page.get(b"Annots") {
                let mut seen = HashSet::new();
                scan.taint(annots, &mut seen);
            }
        }
    }
    // Patterns and Type 3 glyph procedures: placement not tracked.
    let mut seen = HashSet::new();
    let roots: Vec<ObjectId> = doc
        .objects
        .iter()
        .filter(|(_, o)| {
            let d = match o {
                Object::Dictionary(d) => d,
                Object::Stream(s) => &s.dict,
                _ => return false,
            };
            d.has(b"PatternType") || name_is(d, b"Subtype", b"Type3")
        })
        .map(|(id, _)| *id)
        .collect();
    for id in roots {
        scan.taint(&Object::Reference(id), &mut seen);
    }
    (scan.placed, scan.unknown)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Comps {
    Gray,
    Rgb,
}

impl Comps {
    fn n(self) -> usize {
        match self {
            Comps::Gray => 1,
            Comps::Rgb => 3,
        }
    }
}

fn int(d: &Dictionary, key: &[u8]) -> Option<u32> {
    d.get(key).ok()?.as_i64().ok().and_then(|v| u32::try_from(v).ok())
}

fn colour(doc: &Document, d: &Dictionary) -> Option<Comps> {
    match deref(doc, d.get(b"ColorSpace").ok()?) {
        Object::Name(n) if n == b"DeviceGray" => Some(Comps::Gray),
        Object::Name(n) if n == b"DeviceRGB" => Some(Comps::Rgb),
        Object::Array(a) if a.len() == 2 && a[0].as_name().ok() == Some(b"ICCBased") => match deref(doc, &a[1]) {
            Object::Stream(s) => match int(&s.dict, b"N") {
                Some(1) => Some(Comps::Gray),
                Some(3) => Some(Comps::Rgb),
                _ => None,
            },
            _ => None,
        },
        _ => None,
    }
}

fn single_filter(d: &Dictionary) -> Option<Option<&[u8]>> {
    match d.get(b"Filter").ok() {
        None => Some(None),
        Some(Object::Name(n)) => Some(Some(n.as_slice())),
        Some(Object::Array(a)) if a.is_empty() => Some(None),
        Some(Object::Array(a)) if a.len() == 1 => a[0].as_name().ok().map(Some),
        _ => None,
    }
}

/// The image's pixels (8 bits per component, rows top to bottom), if its layout is supported.
fn pixels(s: &Stream, comps: Comps) -> Option<Vec<u8>> {
    let d = &s.dict;
    let (w, h) = (int(d, b"Width")? as usize, int(d, b"Height")? as usize);
    let want = w.checked_mul(h)?.checked_mul(comps.n())?;
    if want > MAX_IMAGE_BYTES || int(d, b"BitsPerComponent")? != 8 || d.has(b"Decode") || d.has(b"Mask") {
        return None;
    }
    if d.get(b"ImageMask").ok().and_then(|o| o.as_bool().ok()) == Some(true) {
        return None;
    }
    let data = match single_filter(d)? {
        Some(b"DCTDecode") => {
            let img = image::load_from_memory_with_format(&s.content, image::ImageFormat::Jpeg).ok()?;
            if img.width() as usize != w || img.height() as usize != h {
                return None;
            }
            match (comps, img) {
                (Comps::Gray, image::DynamicImage::ImageLuma8(g)) => g.into_raw(),
                (Comps::Rgb, image::DynamicImage::ImageRgb8(c)) => c.into_raw(),
                _ => return None,
            }
        }
        Some(b"FlateDecode") | None => s.decompressed_content_with_limit(want).ok()?,
        _ => return None,
    };
    (data.len() == want).then_some(data)
}

fn resample(data: Vec<u8>, w: u32, h: u32, comps: Comps, nw: u32, nh: u32) -> Option<Vec<u8>> {
    Some(match comps {
        Comps::Gray => {
            image::imageops::resize(&GrayImage::from_raw(w, h, data)?, nw, nh, FilterType::CatmullRom).into_raw()
        }
        Comps::Rgb => {
            image::imageops::resize(&RgbImage::from_raw(w, h, data)?, nw, nh, FilterType::CatmullRom).into_raw()
        }
    })
}

fn jpeg(data: &[u8], w: u32, h: u32, comps: Comps, quality: u8) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let t = match comps {
        Comps::Gray => image::ExtendedColorType::L8,
        Comps::Rgb => image::ExtendedColorType::Rgb8,
    };
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, quality).encode(data, w, h, t).ok()?;
    Some(out)
}

/// The encoded JPEG decodes to the expected size and stays close to the resampled pixels
/// (guards against layout mistakes; JPEG error at the lowest allowed quality is far below this).
fn jpeg_matches(encoded: &[u8], expected: &[u8], w: u32, h: u32, comps: Comps) -> bool {
    let Ok(img) = image::load_from_memory_with_format(encoded, image::ImageFormat::Jpeg) else { return false };
    if (img.width(), img.height()) != (w, h) {
        return false;
    }
    let got = match comps {
        Comps::Gray => img.into_luma8().into_raw(),
        Comps::Rgb => img.into_rgb8().into_raw(),
    };
    if got.len() != expected.len() || got.is_empty() {
        return false;
    }
    let sum: u64 = got.iter().zip(expected).map(|(a, b)| u64::from(a.abs_diff(*b))).sum();
    let mean = sum as f64 / got.len() as f64;
    mean <= MAX_JPEG_MEAN_ERROR
}

fn deflate(data: &[u8]) -> Option<Vec<u8>> {
    use std::io::Write;
    let mut e = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::best());
    e.write_all(data).ok()?;
    e.finish().ok()
}

/// What downsampling did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct Downsampled {
    /// Images resampled and re-encoded.
    pub reduced: u32,
    /// Images above the target resolution that were left as they are (unsupported or unknown use).
    pub kept: u32,
    /// Pages (1-based) that show a changed image.
    pub pages: BTreeSet<u32>,
}

/// Resample every image shown above `max_dpi` and store it as JPEG at `quality`.
pub(crate) fn downsample(
    doc: &mut Document,
    max_dpi: f32,
    quality: u8,
    cancelled: &dyn Fn() -> bool,
) -> PwResult<Downsampled> {
    let (placed, unknown) = placements(doc);
    // Soft masks used by more than one image cannot follow one image's new size.
    let mut smask_users: HashMap<ObjectId, u32> = HashMap::new();
    for o in doc.objects.values() {
        if let Object::Stream(s) = o {
            if let Ok(id) = s.dict.get(b"SMask").and_then(|o| o.as_reference()) {
                *smask_users.entry(id).or_default() += 1;
            }
        }
    }
    let mut ids: Vec<ObjectId> = placed.keys().copied().collect();
    ids.sort_unstable();
    let mut done = Downsampled::default();
    for id in ids {
        if cancelled() {
            return Err(PwError::new(ErrorCategory::Cancelled, "job.cancelled", "Reducing the size was cancelled."));
        }
        let place = &placed[&id];
        let Ok(Object::Stream(s)) = doc.get_object(id) else { continue };
        let (Some(w), Some(h)) = (int(&s.dict, b"Width"), int(&s.dict, b"Height")) else { continue };
        if w == 0 || h == 0 || place.width_pt < 1.0 || place.height_pt < 1.0 {
            continue;
        }
        let need_w = place.width_pt / 72.0 * max_dpi;
        let need_h = place.height_pt / 72.0 * max_dpi;
        let scale = (need_w / w as f32).max(need_h / h as f32);
        if scale >= MIN_SCALE_GAIN {
            continue; // already at or near the target resolution
        }
        let (nw, nh) = (((w as f32 * scale).round() as u32).max(1), ((h as f32 * scale).round() as u32).max(1));
        let plan = (|| {
            if unknown.contains(&id) {
                return None;
            }
            let comps = colour(doc, &s.dict)?;
            let smask = match s.dict.get(b"SMask").ok() {
                None => None,
                Some(o) => {
                    let mid = o.as_reference().ok()?;
                    if smask_users.get(&mid).copied().unwrap_or(0) != 1 || unknown.contains(&mid) {
                        return None;
                    }
                    let Ok(Object::Stream(m)) = doc.get_object(mid) else { return None };
                    if m.dict.has(b"Matte") || colour(doc, &m.dict) != Some(Comps::Gray) {
                        return None;
                    }
                    let (mw, mh) = (int(&m.dict, b"Width")?, int(&m.dict, b"Height")?);
                    let mp = pixels(m, Comps::Gray)?;
                    let small = resample(mp, mw, mh, Comps::Gray, nw, nh)?;
                    Some((mid, deflate(&small)?, m.content.len()))
                }
            };
            let px = pixels(s, comps)?;
            let small = resample(px, w, h, comps, nw, nh)?;
            let encoded = jpeg(&small, nw, nh, comps, quality)?;
            if !jpeg_matches(&encoded, &small, nw, nh, comps) {
                return None;
            }
            let before = s.content.len() + smask.as_ref().map_or(0, |m| m.2);
            let after = encoded.len() + smask.as_ref().map_or(0, |m| m.1.len());
            (after < before).then_some((encoded, smask))
        })();
        let Some((encoded, smask)) = plan else {
            done.kept += 1;
            continue;
        };
        if let Ok(Object::Stream(s)) = doc.get_object_mut(id) {
            s.dict.set("Width", Object::Integer(i64::from(nw)));
            s.dict.set("Height", Object::Integer(i64::from(nh)));
            s.dict.set("Filter", Object::Name(b"DCTDecode".to_vec()));
            s.dict.remove(b"DecodeParms");
            s.set_content(encoded);
        }
        if let Some((mid, data, _)) = smask {
            if let Ok(Object::Stream(m)) = doc.get_object_mut(mid) {
                m.dict.set("Width", Object::Integer(i64::from(nw)));
                m.dict.set("Height", Object::Integer(i64::from(nh)));
                m.dict.set("Filter", Object::Name(b"FlateDecode".to_vec()));
                m.dict.remove(b"DecodeParms");
                m.set_content(data);
            }
        }
        done.reduced += 1;
        done.pages.extend(place.pages.iter().copied());
    }
    Ok(done)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matrices_compose_like_cm() {
        // Scale by 2, then translate by (10, 20): a point at (1, 1) lands on (12, 22).
        let m = mul(&[2.0, 0.0, 0.0, 2.0, 0.0, 0.0], &[1.0, 0.0, 0.0, 1.0, 10.0, 20.0]);
        assert_eq!((1.0 * m[0] + 1.0 * m[2] + m[4], 1.0 * m[1] + 1.0 * m[3] + m[5]), (12.0, 22.0));
        // A 90° turn keeps the side lengths.
        let r = mul(&[0.0, 100.0, -50.0, 0.0, 0.0, 0.0], &IDENTITY);
        assert_eq!((r[0].hypot(r[1]), r[2].hypot(r[3])), (100.0, 50.0));
    }
}

#[cfg(test)]
mod layout_tests {
    use super::*;
    use lopdf::dictionary;

    fn photo(w: u32, h: u32) -> Vec<u8> {
        let mut v = Vec::with_capacity((w * h * 3) as usize);
        for y in 0..h {
            for x in 0..w {
                v.extend_from_slice(&[(x * 255 / w) as u8, (y * 255 / h) as u8, ((x + y) % 200) as u8]);
            }
        }
        v
    }

    #[test]
    fn jpeg_check_accepts_real_encodings_and_rejects_layout_mistakes() {
        let (w, h) = (120, 80);
        let px = photo(w, h);
        let enc = jpeg(&px, w, h, Comps::Rgb, 30).expect("encode");
        assert!(jpeg_matches(&enc, &px, w, h, Comps::Rgb));
        // Channels swapped (BGR read as RGB).
        let swapped: Vec<u8> = px.chunks(3).flat_map(|c| [c[2], c[1], c[0]]).collect();
        assert!(!jpeg_matches(&enc, &swapped, w, h, Comps::Rgb));
        // Wrong size, and rows read with the wrong stride.
        assert!(!jpeg_matches(&enc, &px[..px.len() - 3 * h as usize], w - 1, h, Comps::Rgb));
        let mut shifted = px.clone();
        shifted.rotate_left(3 * 37);
        assert!(!jpeg_matches(&enc, &shifted, w, h, Comps::Rgb));
        assert!(!jpeg_matches(b"not a jpeg", &px, w, h, Comps::Rgb));
    }

    #[test]
    fn unsupported_image_layouts_are_left_alone() {
        let flate = |d: Dictionary, data: &[u8]| Stream::new(d, data.to_vec());
        let base = || {
            dictionary! { "Subtype" => "Image", "Width" => 2, "Height" => 2, "BitsPerComponent" => 8, "ColorSpace" => "DeviceGray" }
        };
        assert!(pixels(&flate(base(), &[1, 2, 3, 4]), Comps::Gray).is_some());
        assert!(pixels(&flate(base(), &[1, 2, 3]), Comps::Gray).is_none(), "short data");
        let mut d = base();
        d.set("BitsPerComponent", 1);
        assert!(pixels(&flate(d, &[0]), Comps::Gray).is_none(), "1-bit");
        let mut d = base();
        d.set("Decode", vec![1.into(), 0.into()]);
        assert!(pixels(&flate(d, &[1, 2, 3, 4]), Comps::Gray).is_none(), "decode array");
        let mut d = base();
        d.set("Mask", vec![0.into(), 10.into()]);
        assert!(pixels(&flate(d, &[1, 2, 3, 4]), Comps::Gray).is_none(), "colour-key mask");
        let mut d = base();
        d.set("Filter", "JBIG2Decode");
        assert!(pixels(&flate(d, &[1, 2, 3, 4]), Comps::Gray).is_none(), "other codec");
        let doc = Document::with_version("1.7");
        let mut d = base();
        d.set("ColorSpace", "DeviceCMYK");
        assert_eq!(colour(&doc, &d), None);
        d.set("ColorSpace", vec![Object::Name(b"Indexed".to_vec())]);
        assert_eq!(colour(&doc, &d), None);
    }

    /// One page placing image `im` through content `content`, with `xobjects` as resources.
    fn doc_with(content: &[u8], xobjects: Dictionary, extra: Option<(&str, Object)>) -> (Document, ObjectId) {
        let mut doc = Document::with_version("1.7");
        let pages = doc.new_object_id();
        let c = doc.add_object(Stream::new(dictionary! {}, content.to_vec()));
        let mut page = dictionary! {
            "Type" => "Page", "Parent" => pages, "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
            "Contents" => c, "Resources" => dictionary! { "XObject" => xobjects },
        };
        if let Some((k, v)) = extra {
            page.set(k, v);
        }
        let p = doc.add_object(page);
        doc.objects.insert(
            pages,
            Object::Dictionary(dictionary! { "Type" => "Pages", "Kids" => vec![p.into()], "Count" => 1 }),
        );
        let cat = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages });
        doc.trailer.set("Root", cat);
        (doc, p)
    }

    fn image(doc: &mut Document) -> ObjectId {
        doc.add_object(Stream::new(
            dictionary! { "Subtype" => "Image", "Width" => 4, "Height" => 4, "BitsPerComponent" => 8, "ColorSpace" => "DeviceGray" },
            vec![0; 16],
        ))
    }

    #[test]
    fn placements_follow_cm_forms_and_largest_use() {
        let (mut doc, page) = doc_with(b"q 100 0 0 50 10 10 cm /I Do Q q 1 0 0 1 0 0 cm /F Do Q", dictionary! {}, None);
        let im = image(&mut doc);
        let form = doc.add_object(Stream::new(
            dictionary! { "Subtype" => "Form", "Matrix" => vec![2.into(), 0.into(), 0.into(), 2.into(), 0.into(), 0.into()],
                          "Resources" => dictionary! { "XObject" => dictionary! { "I" => im } } },
            b"q 50 0 0 30 0 0 cm /I Do Q".to_vec(),
        ));
        if let Ok(Object::Dictionary(p)) = doc.get_object_mut(page) {
            p.set("Resources", dictionary! { "XObject" => dictionary! { "I" => im, "F" => form } });
        }
        let (placed, unknown) = placements(&doc);
        let p = &placed[&im];
        // Direct use 100x50; through the form 50*2 x 30*2 = 100x60: the largest of each side.
        assert_eq!((p.width_pt, p.height_pt), (100.0, 60.0));
        assert!(unknown.is_empty());
    }

    #[test]
    fn annotation_pattern_and_unreadable_uses_make_an_image_unknown() {
        let (mut doc, page) = doc_with(b"q 10 0 0 10 0 0 cm /I Do Q", dictionary! {}, None);
        let im = image(&mut doc);
        let ap = doc.add_object(Stream::new(
            dictionary! { "Subtype" => "Form", "Resources" => dictionary! { "XObject" => dictionary! { "I" => im } } },
            b"/I Do".to_vec(),
        ));
        let annot =
            doc.add_object(dictionary! { "Subtype" => "Stamp", "P" => page, "AP" => dictionary! { "N" => ap } });
        if let Ok(Object::Dictionary(p)) = doc.get_object_mut(page) {
            p.set("Resources", dictionary! { "XObject" => dictionary! { "I" => im } });
            p.set("Annots", vec![annot.into()]);
        }
        let (placed, unknown) = placements(&doc);
        assert!(placed.contains_key(&im) && unknown.contains(&im), "placed by the page but also used by an annotation");

        let (mut doc, page) = doc_with(b"", dictionary! {}, None);
        let im = image(&mut doc);
        doc.add_object(Stream::new(
            dictionary! { "PatternType" => 1, "Resources" => dictionary! { "XObject" => dictionary! { "I" => im } } },
            b"/I Do".to_vec(),
        ));
        let _ = page;
        assert!(placements(&doc).1.contains(&im), "pattern use");

        // Content that cannot be read: every image the page names is unknown.
        let (mut doc, page) = doc_with(b"q 10 0 0 10 0 0 cm /I Do Q (unterminated", dictionary! {}, None);
        let im = image(&mut doc);
        if let Ok(Object::Dictionary(p)) = doc.get_object_mut(page) {
            p.set("Resources", dictionary! { "XObject" => dictionary! { "I" => im } });
        }
        assert!(placements(&doc).1.contains(&im), "unreadable page content");
    }
}
