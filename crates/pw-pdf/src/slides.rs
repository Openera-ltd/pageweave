//! PDF → PowerPoint (.pptx) (PRD 06 interchange targets). Each page becomes a slide of the same
//! size; its paragraphs become text boxes at their places (editable text with fonts, sizes,
//! bold/italic, colours, links), ruled tables become PowerPoint tables, pictures are placed as
//! pictures (JPEG unchanged). The page analysis is the Word export's (`docx::analyze`). Before the
//! file is written it is read back: every slide parses, every reference resolves, every picture
//! decodes, and the slides hold exactly the PDF's characters.

use crate::document::{load, OpenOptions};
use crate::docx::layout::{Align, Item, PageLayout, Paragraph, Picture, Rect, Run, TableItem, VertAlign};
use crate::docx::writer::{esc, family, Media};
use crate::docx::{analyze, notes, Analysis, DocxReport};
use crate::engine;
use pw_core::output::{write_atomic_bytes, CollisionPolicy, CommittedOutput};
use pw_core::range::parse_page_ranges;
use pw_core::{ErrorCategory, PwError, PwResult};
use quick_xml::events::Event;
use quick_xml::{Reader, XmlVersion};
use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::io::{Read, Write as _};
use std::path::Path;

const A: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
const R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const P: &str = "http://schemas.openxmlformats.org/presentationml/2006/main";

fn emu(pt: f32) -> i64 {
    (pt * 12_700.0).round() as i64
}

/// One slide's drawing: shapes in back-to-front order, with its relationships.
struct SlideXml {
    body: String,
    next_id: u32,
    images: Vec<usize>,
    links: Vec<usize>,
}

impl SlideXml {
    fn id(&mut self) -> u32 {
        self.next_id += 1;
        self.next_id
    }

    fn image_rel(&mut self, media: usize) -> String {
        if !self.images.contains(&media) {
            self.images.push(media);
        }
        format!("rIdImg{media}")
    }

    fn link_rel(&mut self, link: usize) -> String {
        if !self.links.contains(&link) {
            self.links.push(link);
        }
        format!("rIdLink{link}")
    }

    fn picture(&mut self, media: usize, x: f32, y: f32, w: f32, h: f32) {
        let id = self.id();
        let rel = self.image_rel(media);
        let _ = write!(
            self.body,
            "<p:pic><p:nvPicPr><p:cNvPr id=\"{id}\" name=\"Picture {id}\"/><p:cNvPicPr><a:picLocks noChangeAspect=\"1\"/></p:cNvPicPr><p:nvPr/></p:nvPicPr>\
<p:blipFill><a:blip r:embed=\"{rel}\"/><a:stretch><a:fillRect/></a:stretch></p:blipFill>\
<p:spPr><a:xfrm><a:off x=\"{}\" y=\"{}\"/><a:ext cx=\"{}\" cy=\"{}\"/></a:xfrm><a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom></p:spPr></p:pic>",
            emu(x),
            emu(y),
            emu(w).max(1),
            emu(h).max(1)
        );
    }

    fn runs(&mut self, runs: &[Run], k: f32) -> String {
        let mut x = String::new();
        for r in runs {
            let size = match r.vert {
                VertAlign::Baseline => r.size,
                _ => r.size / 0.65,
            } * k;
            let sz = ((size * 100.0).round() as i64).clamp(100, 400_000);
            let _ = write!(x, "<a:r><a:rPr lang=\"en-US\" sz=\"{sz}\"");
            if r.bold {
                x.push_str(" b=\"1\"");
            }
            if r.italic {
                x.push_str(" i=\"1\"");
            }
            match r.vert {
                VertAlign::Superscript => x.push_str(" baseline=\"30000\""),
                VertAlign::Subscript => x.push_str(" baseline=\"-25000\""),
                VertAlign::Baseline => {}
            }
            let font = esc(&family(&r.font));
            let [cr, cg, cb] = r.color;
            let _ = write!(
                x,
                " dirty=\"0\"><a:solidFill><a:srgbClr val=\"{cr:02X}{cg:02X}{cb:02X}\"/></a:solidFill><a:latin typeface=\"{font}\"/><a:cs typeface=\"{font}\"/>"
            );
            if let Some(l) = r.link {
                let rel = self.link_rel(l);
                let _ = write!(x, "<a:hlinkClick r:id=\"{rel}\"/>");
            }
            let _ = write!(x, "</a:rPr><a:t>{}</a:t></a:r>", esc(&r.text));
        }
        x
    }

    fn paragraph_xml(&mut self, p: &Paragraph, tab_origin: f32, k: f32) -> String {
        let algn = match p.align {
            Align::Left => "l",
            Align::Center => "ctr",
            Align::Right => "r",
            Align::Justify => "just",
        };
        let mut x = format!(
            "<a:p><a:pPr algn=\"{algn}\"><a:lnSpc><a:spcPts val=\"{}\"/></a:lnSpc>",
            ((p.pitch * k * 100.0).round() as i64).clamp(100, 158_400)
        );
        if !p.tabs.is_empty() {
            x.push_str("<a:tabLst>");
            for t in &p.tabs {
                let _ = write!(x, "<a:tab pos=\"{}\" algn=\"l\"/>", emu(((t - tab_origin) * k).max(0.0)));
            }
            x.push_str("</a:tabLst>");
        }
        x.push_str("</a:pPr>");
        x.push_str(&self.runs(&p.runs, k));
        x.push_str("</a:p>");
        x
    }

    fn text_box(&mut self, p: &Paragraph, col_left: f32, page_h: f32, k: f32, fallback: Rect) {
        let r = if p.rect.width() > 0.0 { p.rect } else { fallback };
        // One line never wraps (an editor may draw it a little wider); longer paragraphs wrap at
        // the PDF's width plus a little room for rounding.
        let single = r.height() < 1.5 * p.pitch;
        let w = (r.width() * if single { 1.1 } else { 1.015 } + 2.0) * k;
        let wrap = if single { "none" } else { "square" };
        let h = r.height().max(p.pitch) * k;
        let (x, y) = (r.left * k, (page_h - r.top) * k);
        let id = self.id();
        // Tab stops are column-relative; the box starts at the paragraph's own left edge.
        let para = self.paragraph_xml(p, r.left - col_left, k);
        let _ = write!(
            self.body,
            "<p:sp><p:nvSpPr><p:cNvPr id=\"{id}\" name=\"Text {id}\"/><p:cNvSpPr txBox=\"1\"/><p:nvPr/></p:nvSpPr>\
<p:spPr><a:xfrm><a:off x=\"{}\" y=\"{}\"/><a:ext cx=\"{}\" cy=\"{}\"/></a:xfrm><a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom><a:noFill/></p:spPr>\
<p:txBody><a:bodyPr wrap=\"{wrap}\" lIns=\"0\" tIns=\"0\" rIns=\"0\" bIns=\"0\" rtlCol=\"0\"><a:noAutofit/></a:bodyPr><a:lstStyle/>{para}</p:txBody></p:sp>",
            emu(x),
            emu(y),
            emu(w).max(1),
            emu(h).max(1)
        );
    }

    fn table(&mut self, t: &TableItem, page_h: f32, k: f32) {
        let id = self.id();
        let total: f32 = t.widths.iter().sum();
        let sx = if total > 0.0 { t.rect.width() / total } else { 1.0 };
        let widths: Vec<f32> = t.widths.iter().map(|w| w * sx).collect();
        let _ = write!(
            self.body,
            "<p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id=\"{id}\" name=\"Table {id}\"/><p:cNvGraphicFramePr><a:graphicFrameLocks noGrp=\"1\"/></p:cNvGraphicFramePr><p:nvPr/></p:nvGraphicFramePr>\
<p:xfrm><a:off x=\"{}\" y=\"{}\"/><a:ext cx=\"{}\" cy=\"{}\"/></p:xfrm><a:graphic><a:graphicData uri=\"http://schemas.openxmlformats.org/drawingml/2006/table\"><a:tbl><a:tblPr/><a:tblGrid>",
            emu(t.rect.left * k),
            emu((page_h - t.rect.top) * k),
            emu(t.rect.width() * k).max(1),
            emu(t.rect.height() * k).max(1)
        );
        for w in &widths {
            let _ = write!(self.body, "<a:gridCol w=\"{}\"/>", emu(w * k).max(1));
        }
        self.body.push_str("</a:tblGrid>");
        let line = |side: &str| {
            format!("<a:{side} w=\"6350\"><a:solidFill><a:srgbClr val=\"000000\"/></a:solidFill></a:{side}>")
        };
        let borders = format!("{}{}{}{}", line("lnL"), line("lnR"), line("lnT"), line("lnB"));
        let cols = widths.len();
        for (r, h) in t.heights.iter().enumerate() {
            let _ = write!(self.body, "<a:tr h=\"{}\">", emu(h * k).max(1));
            for c in 0..cols {
                let cell = t
                    .cells
                    .iter()
                    .find(|x| r >= x.row && r < x.row + x.row_span && c >= x.col && c < x.col + x.col_span);
                let mut attrs = String::new();
                let mut paras = String::new();
                match cell {
                    Some(cell) if cell.row == r && cell.col == c => {
                        if cell.col_span > 1 {
                            let _ = write!(attrs, " gridSpan=\"{}\"", cell.col_span);
                        }
                        if cell.row_span > 1 {
                            let _ = write!(attrs, " rowSpan=\"{}\"", cell.row_span);
                        }
                        // Cell tab stops are relative to the cell's text area already.
                        for p in &cell.paragraphs {
                            let x = self.paragraph_xml(p, 0.0, k);
                            paras.push_str(&x);
                        }
                    }
                    Some(cell) => {
                        if c > cell.col {
                            attrs.push_str(" hMerge=\"1\"");
                        }
                        if r > cell.row {
                            attrs.push_str(" vMerge=\"1\"");
                        }
                    }
                    None => {}
                }
                if paras.is_empty() {
                    paras.push_str("<a:p><a:endParaRPr lang=\"en-US\" dirty=\"0\"/></a:p>");
                }
                let pad = emu(crate::docx::layout::CELL_PAD * k);
                let _ = write!(
                    self.body,
                    "<a:tc{attrs}><a:txBody><a:bodyPr/><a:lstStyle/>{paras}</a:txBody><a:tcPr marL=\"{pad}\" marR=\"{pad}\" marT=\"0\" marB=\"0\">{borders}<a:noFill/></a:tcPr></a:tc>"
                );
            }
            self.body.push_str("</a:tr>");
        }
        self.body.push_str("</a:tbl></a:graphicData></a:graphic></p:graphicFrame>");
    }
}

fn slide_xml(page: &PageLayout, k: f32) -> SlideXml {
    let mut s = SlideXml { body: String::new(), next_id: 1, images: Vec::new(), links: Vec::new() };
    let h = page.height;
    for Picture { id, rect } in &page.backgrounds {
        s.picture(*id, rect.left * k, (h - rect.top) * k, rect.width() * k, rect.height() * k);
    }
    // Text added after a page's content (turned text) goes along the bottom margin.
    let mut fallback_y = page.margin_bottom.max(18.0);
    for band in &page.bands {
        for col in &band.columns {
            for it in &col.items {
                match it {
                    Item::Para(p) => {
                        let fb =
                            Rect { left: col.left, right: col.right, top: fallback_y + p.pitch, bottom: fallback_y };
                        if p.rect.width() <= 0.0 {
                            fallback_y = (fallback_y - p.pitch).max(0.0);
                        }
                        s.text_box(p, col.left, h, k, fb);
                    }
                    Item::Pictures { row, .. } => {
                        for r in row {
                            s.picture(
                                r.id,
                                r.rect.left * k,
                                (h - r.rect.top) * k,
                                r.rect.width() * k,
                                r.rect.height() * k,
                            );
                        }
                    }
                    Item::Table { table, .. } => s.table(table, h, k),
                }
            }
        }
    }
    for Picture { id, rect } in &page.floating {
        s.picture(*id, rect.left * k, (h - rect.top) * k, rect.width() * k, rect.height() * k);
    }
    s
}

const THEME: &str = r#"<a:theme xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" name="PageWeave"><a:themeElements><a:clrScheme name="PageWeave"><a:dk1><a:srgbClr val="000000"/></a:dk1><a:lt1><a:srgbClr val="FFFFFF"/></a:lt1><a:dk2><a:srgbClr val="1F2937"/></a:dk2><a:lt2><a:srgbClr val="F3F4F6"/></a:lt2><a:accent1><a:srgbClr val="2563EB"/></a:accent1><a:accent2><a:srgbClr val="0EA5E9"/></a:accent2><a:accent3><a:srgbClr val="10B981"/></a:accent3><a:accent4><a:srgbClr val="F59E0B"/></a:accent4><a:accent5><a:srgbClr val="EF4444"/></a:accent5><a:accent6><a:srgbClr val="8B5CF6"/></a:accent6><a:hlink><a:srgbClr val="1D4ED8"/></a:hlink><a:folHlink><a:srgbClr val="7C3AED"/></a:folHlink></a:clrScheme><a:fontScheme name="PageWeave"><a:majorFont><a:latin typeface="Arial"/><a:ea typeface=""/><a:cs typeface=""/></a:majorFont><a:minorFont><a:latin typeface="Arial"/><a:ea typeface=""/><a:cs typeface=""/></a:minorFont></a:fontScheme><a:fmtScheme name="PageWeave"><a:fillStyleLst><a:solidFill><a:schemeClr val="phClr"/></a:solidFill><a:solidFill><a:schemeClr val="phClr"/></a:solidFill><a:solidFill><a:schemeClr val="phClr"/></a:solidFill></a:fillStyleLst><a:lnStyleLst><a:ln w="6350"><a:solidFill><a:schemeClr val="phClr"/></a:solidFill></a:ln><a:ln w="12700"><a:solidFill><a:schemeClr val="phClr"/></a:solidFill></a:ln><a:ln w="19050"><a:solidFill><a:schemeClr val="phClr"/></a:solidFill></a:ln></a:lnStyleLst><a:effectStyleLst><a:effectStyle><a:effectLst/></a:effectStyle><a:effectStyle><a:effectLst/></a:effectStyle><a:effectStyle><a:effectLst/></a:effectStyle></a:effectStyleLst><a:bgFillStyleLst><a:solidFill><a:schemeClr val="phClr"/></a:solidFill><a:solidFill><a:schemeClr val="phClr"/></a:solidFill><a:solidFill><a:schemeClr val="phClr"/></a:solidFill></a:bgFillStyleLst></a:fmtScheme></a:themeElements></a:theme>"#;

const XML_HEAD: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n";

fn zip_err<E>(_: E) -> PwError {
    PwError::internal("pptx.write", "The presentation could not be assembled.")
}

/// Assemble the presentation. Slides use the first page's size; other pages are scaled to fit.
fn write_pptx(
    pages: &[PageLayout],
    media: &[Media],
    links: &[String],
    title: Option<&str>,
) -> PwResult<(Vec<u8>, usize)> {
    let first = pages.first().ok_or_else(|| PwError::invalid("pptx.no_pages", "Choose at least one page."))?;
    let (sw, sh) = (first.width.clamp(72.0, 4032.0), first.height.clamp(72.0, 4032.0));
    let mut scaled = 0;
    let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let xml = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    let stored = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    let put = |z: &mut zip::ZipWriter<std::io::Cursor<Vec<u8>>>, name: &str, body: &str| -> PwResult<()> {
        z.start_file(name, xml).map_err(zip_err)?;
        z.write_all(body.as_bytes()).map_err(zip_err)
    };
    let n = pages.len();
    let mut types = format!(
        "{XML_HEAD}<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"xml\" ContentType=\"application/xml\"/><Default Extension=\"png\" ContentType=\"image/png\"/><Default Extension=\"jpeg\" ContentType=\"image/jpeg\"/>\
<Override PartName=\"/ppt/presentation.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml\"/>\
<Override PartName=\"/ppt/slideMasters/slideMaster1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.slideMaster+xml\"/>\
<Override PartName=\"/ppt/slideLayouts/slideLayout1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.slideLayout+xml\"/>\
<Override PartName=\"/ppt/theme/theme1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.theme+xml\"/>\
<Override PartName=\"/docProps/core.xml\" ContentType=\"application/vnd.openxmlformats-package.core-properties+xml\"/>\
<Override PartName=\"/docProps/app.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.extended-properties+xml\"/>"
    );
    for i in 1..=n {
        let _ = write!(types, "<Override PartName=\"/ppt/slides/slide{i}.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.slide+xml\"/>");
    }
    types.push_str("</Types>");
    put(&mut z, "[Content_Types].xml", &types)?;
    put(&mut z, "_rels/.rels", &format!("{XML_HEAD}<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"ppt/presentation.xml\"/><Relationship Id=\"rId2\" Type=\"http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties\" Target=\"docProps/core.xml\"/><Relationship Id=\"rId3\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/extended-properties\" Target=\"docProps/app.xml\"/></Relationships>"))?;
    let t = title.map(|t| format!("<dc:title>{}</dc:title>", esc(t))).unwrap_or_default();
    put(&mut z, "docProps/core.xml", &format!("{XML_HEAD}<cp:coreProperties xmlns:cp=\"http://schemas.openxmlformats.org/package/2006/metadata/core-properties\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\">{t}</cp:coreProperties>"))?;
    put(&mut z, "docProps/app.xml", &format!("{XML_HEAD}<Properties xmlns=\"http://schemas.openxmlformats.org/officeDocument/2006/extended-properties\"><Application>PageWeave</Application><Slides>{n}</Slides></Properties>"))?;
    let mut pres = format!("{XML_HEAD}<p:presentation xmlns:a=\"{A}\" xmlns:r=\"{R}\" xmlns:p=\"{P}\"><p:sldMasterIdLst><p:sldMasterId id=\"2147483648\" r:id=\"rIdMaster\"/></p:sldMasterIdLst><p:sldIdLst>");
    let mut prels = format!("{XML_HEAD}<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rIdMaster\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideMaster\" Target=\"slideMasters/slideMaster1.xml\"/><Relationship Id=\"rIdTheme\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/theme\" Target=\"theme/theme1.xml\"/>");
    for i in 1..=n {
        let _ = write!(pres, "<p:sldId id=\"{}\" r:id=\"rIdSlide{i}\"/>", 255 + i);
        let _ = write!(prels, "<Relationship Id=\"rIdSlide{i}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide\" Target=\"slides/slide{i}.xml\"/>");
    }
    let _ = write!(
        pres,
        "</p:sldIdLst><p:sldSz cx=\"{}\" cy=\"{}\"/><p:notesSz cx=\"6858000\" cy=\"9144000\"/></p:presentation>",
        emu(sw),
        emu(sh)
    );
    prels.push_str("</Relationships>");
    put(&mut z, "ppt/presentation.xml", &pres)?;
    put(&mut z, "ppt/_rels/presentation.xml.rels", &prels)?;
    let empty_tree = "<p:cSld><p:spTree><p:nvGrpSpPr><p:cNvPr id=\"1\" name=\"\"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr/></p:spTree></p:cSld>";
    put(&mut z, "ppt/slideMasters/slideMaster1.xml", &format!("{XML_HEAD}<p:sldMaster xmlns:a=\"{A}\" xmlns:r=\"{R}\" xmlns:p=\"{P}\">{empty_tree}<p:clrMap bg1=\"lt1\" tx1=\"dk1\" bg2=\"lt2\" tx2=\"dk2\" accent1=\"accent1\" accent2=\"accent2\" accent3=\"accent3\" accent4=\"accent4\" accent5=\"accent5\" accent6=\"accent6\" hlink=\"hlink\" folHlink=\"folHlink\"/><p:sldLayoutIdLst><p:sldLayoutId id=\"2147483649\" r:id=\"rIdLayout\"/></p:sldLayoutIdLst></p:sldMaster>"))?;
    put(&mut z, "ppt/slideMasters/_rels/slideMaster1.xml.rels", &format!("{XML_HEAD}<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rIdLayout\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideLayout\" Target=\"../slideLayouts/slideLayout1.xml\"/><Relationship Id=\"rIdTheme\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/theme\" Target=\"../theme/theme1.xml\"/></Relationships>"))?;
    put(&mut z, "ppt/slideLayouts/slideLayout1.xml", &format!("{XML_HEAD}<p:sldLayout xmlns:a=\"{A}\" xmlns:r=\"{R}\" xmlns:p=\"{P}\" type=\"blank\" preserve=\"1\">{}<p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sldLayout>", empty_tree.replace("<p:cSld>", "<p:cSld name=\"Blank\">")))?;
    put(&mut z, "ppt/slideLayouts/_rels/slideLayout1.xml.rels", &format!("{XML_HEAD}<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rIdMaster\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideMaster\" Target=\"../slideMasters/slideMaster1.xml\"/></Relationships>"))?;
    put(&mut z, "ppt/theme/theme1.xml", &format!("{XML_HEAD}{THEME}"))?;
    for (i, page) in pages.iter().enumerate() {
        let k = (sw / page.width.max(1.0)).min(sh / page.height.max(1.0));
        if (k - 1.0).abs() > 0.001 {
            scaled += 1;
        }
        let s = slide_xml(page, k);
        put(&mut z, &format!("ppt/slides/slide{}.xml", i + 1), &format!(
            "{XML_HEAD}<p:sld xmlns:a=\"{A}\" xmlns:r=\"{R}\" xmlns:p=\"{P}\"><p:cSld><p:spTree><p:nvGrpSpPr><p:cNvPr id=\"1\" name=\"\"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr/>{}</p:spTree></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sld>",
            s.body
        ))?;
        let mut rels = format!("{XML_HEAD}<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rIdLayout\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideLayout\" Target=\"../slideLayouts/slideLayout1.xml\"/>");
        for m in &s.images {
            let _ = write!(rels, "<Relationship Id=\"rIdImg{m}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/image\" Target=\"../media/image{m}.{}\"/>", media[*m].kind.ext());
        }
        for l in &s.links {
            let _ = write!(rels, "<Relationship Id=\"rIdLink{l}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink\" Target=\"{}\" TargetMode=\"External\"/>", esc(&links[*l]));
        }
        rels.push_str("</Relationships>");
        put(&mut z, &format!("ppt/slides/_rels/slide{}.xml.rels", i + 1), &rels)?;
    }
    for (i, m) in media.iter().enumerate() {
        z.start_file(format!("ppt/media/image{i}.{}", m.kind.ext()), stored).map_err(zip_err)?;
        z.write_all(&m.bytes).map_err(zip_err)?;
    }
    Ok((z.finish().map_err(zip_err)?.into_inner(), scaled))
}

/// What a presentation read back holds.
#[derive(Debug, Default)]
pub struct PptxCheck {
    pub slides: usize,
    pub chars: HashMap<char, usize>,
    pub pictures: usize,
    pub tables: usize,
    pub links: usize,
    pub text: String,
}

fn bad(what: &str) -> PwError {
    PwError::new(
        ErrorCategory::ValidationFailed,
        "output.verify",
        format!("The presentation failed verification ({what}) and was not saved."),
    )
}

fn read_part(z: &mut zip::ZipArchive<std::io::Cursor<&[u8]>>, name: &str) -> PwResult<Vec<u8>> {
    let mut f = z.by_name(name).map_err(|_| bad(&format!("{name} is missing")))?;
    if f.size() > 512 * 1024 * 1024 {
        return Err(bad("a part is too large"));
    }
    let mut v = Vec::new();
    f.read_to_end(&mut v).map_err(|_| bad("a part is unreadable"))?;
    Ok(v)
}

fn attr(e: &quick_xml::events::BytesStart, key: &str) -> Option<String> {
    e.attributes()
        .flatten()
        .find(|a| a.key.as_ref() == key)
        .and_then(|a| a.normalized_value(XmlVersion::Implicit1_0).ok())
        .map(|v| v.into_owned())
}

fn rel_targets(xml: &[u8]) -> PwResult<HashMap<String, (String, bool)>> {
    let mut r = Reader::from_reader(xml);
    let mut buf = Vec::new();
    let mut out = HashMap::new();
    loop {
        match r.read_event_into(&mut buf) {
            Ok(Event::Start(e) | Event::Empty(e)) if e.local_name().as_ref() == "Relationship" => {
                let id = attr(&e, "Id").ok_or_else(|| bad("a relationship has no id"))?;
                let t = attr(&e, "Target").ok_or_else(|| bad("a relationship has no target"))?;
                out.insert(id, (t, attr(&e, "TargetMode").as_deref() == Some("External")));
            }
            Ok(Event::DocType(_)) => return Err(bad("unexpected document type")),
            Ok(Event::Eof) => break,
            Err(_) => return Err(bad("relationships do not parse")),
            _ => {}
        }
        buf.clear();
    }
    Ok(out)
}

/// Read a presentation back (independently of how it was written).
pub fn check_pptx(bytes: &[u8]) -> PwResult<PptxCheck> {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|_| bad("the package does not open"))?;
    for name in [
        "[Content_Types].xml",
        "ppt/presentation.xml",
        "ppt/slideMasters/slideMaster1.xml",
        "ppt/slideLayouts/slideLayout1.xml",
        "ppt/theme/theme1.xml",
    ] {
        read_part(&mut z, name)?;
    }
    let mut out = PptxCheck::default();
    let mut decoded: HashSet<String> = HashSet::new();
    for i in 1.. {
        let name = format!("ppt/slides/slide{i}.xml");
        if z.by_name(&name).is_err() {
            break;
        }
        out.slides += 1;
        let xml = read_part(&mut z, &name)?;
        let rels = rel_targets(&read_part(&mut z, &format!("ppt/slides/_rels/slide{i}.xml.rels"))?)?;
        let mut r = Reader::from_reader(xml.as_slice());
        let mut buf = Vec::new();
        let mut in_t = false;
        let mut depth = 0i64;
        let mut embeds = Vec::new();
        loop {
            match r.read_event_into(&mut buf) {
                Ok(Event::Start(e)) => {
                    depth += 1;
                    match e.local_name().as_ref() {
                        "t" => in_t = true,
                        "tbl" => out.tables += 1,
                        _ => {}
                    }
                }
                Ok(Event::Empty(e)) => match e.local_name().as_ref() {
                    "blip" => {
                        out.pictures += 1;
                        embeds.push(attr(&e, "r:embed").ok_or_else(|| bad("a picture has no data"))?);
                    }
                    "hlinkClick" => {
                        let id = attr(&e, "r:id").ok_or_else(|| bad("a link has no target"))?;
                        match rels.get(&id) {
                            Some((t, true))
                                if t.starts_with("http://")
                                    || t.starts_with("https://")
                                    || t.starts_with("mailto:") =>
                            {
                                out.links += 1
                            }
                            _ => return Err(bad("a link points nowhere or at a disallowed place")),
                        }
                    }
                    _ => {}
                },
                Ok(Event::Text(t)) if in_t => {
                    let s = t.xml10_content();
                    for c in s.chars().filter(|c| !c.is_whitespace()) {
                        *out.chars.entry(c).or_default() += 1;
                    }
                    out.text.push_str(&s);
                }
                Ok(Event::GeneralRef(e)) if in_t => {
                    let c = match e.resolve_char_ref() {
                        Ok(Some(c)) => c,
                        _ => match e.xml10_content().as_ref() {
                            "amp" => '&',
                            "lt" => '<',
                            "gt" => '>',
                            "quot" => '"',
                            "apos" => '\'',
                            _ => return Err(bad("unknown entity")),
                        },
                    };
                    *out.chars.entry(c).or_default() += 1;
                    out.text.push(c);
                }
                Ok(Event::End(e)) => {
                    depth -= 1;
                    match e.local_name().as_ref() {
                        "t" => in_t = false,
                        "p" => out.text.push('\n'),
                        _ => {}
                    }
                }
                Ok(Event::DocType(_)) => return Err(bad("unexpected document type")),
                Ok(Event::Eof) => break,
                Err(_) => return Err(bad("a slide does not parse")),
                _ => {}
            }
            buf.clear();
        }
        if depth != 0 {
            return Err(bad("a slide is incomplete"));
        }
        for id in embeds {
            let Some((target, false)) = rels.get(&id) else { return Err(bad("a picture points nowhere")) };
            let part = format!("ppt/{}", target.trim_start_matches("../"));
            if decoded.insert(part.clone()) {
                let data = read_part(&mut z, &part)?;
                image::load_from_memory(&data).map_err(|_| bad("a picture does not decode"))?;
            }
        }
    }
    Ok(out)
}

/// Convert `input` (all pages, or `pages`) to a PowerPoint presentation at `output`.
pub fn convert_to_pptx(
    input: &Path,
    opts: &OpenOptions,
    pages: Option<&str>,
    output: &Path,
    policy: CollisionPolicy,
    cancelled: &dyn Fn() -> bool,
    progress: &dyn Fn(u64, u64),
) -> PwResult<(CommittedOutput, DocxReport, Vec<String>)> {
    let pdfium = engine()?;
    let doc = load(&pdfium, input, opts)?;
    let n = doc.pages().len().max(0) as u32;
    let idx: Vec<u32> = match pages {
        Some(spec) => parse_page_ranges(spec, n)?,
        None => (0..n).collect(),
    };
    if idx.is_empty() {
        return Err(PwError::invalid("pptx.no_pages", "Choose at least one page."));
    }
    let title = doc
        .metadata()
        .get(pdfium_render::prelude::PdfDocumentMetadataTagType::Title)
        .map(|t| t.value().trim().to_string())
        .filter(|t| !t.is_empty());
    let Analysis { layouts, media, links, expected, mut report, placed_pictures, expected_tables } =
        analyze(&doc, &idx, cancelled, &|d, t| progress(d, t + 1))?;
    let (bytes, scaled) = write_pptx(&layouts, &media, &links, title.as_deref())?;
    let verify = |b: &[u8]| -> PwResult<PptxCheck> {
        let c = check_pptx(b)?;
        if c.slides != layouts.len() {
            return Err(bad("a slide is missing"));
        }
        if c.pictures != placed_pictures {
            return Err(bad("a picture is missing"));
        }
        if c.tables != expected_tables {
            return Err(bad("a table is missing"));
        }
        if c.chars != expected {
            return Err(bad("text is missing or changed"));
        }
        Ok(c)
    };
    let check = verify(&bytes)?;
    if cancelled() {
        return Err(PwError::cancelled());
    }
    let c = write_atomic_bytes(output, policy, &[input.to_path_buf()], &bytes, |staged| {
        let back = std::fs::read(staged).map_err(|e| PwError::from_io(&e, "output file"))?;
        if back != bytes {
            return Err(bad("the written file differs"));
        }
        verify(&back).map(|_| ())
    })?;
    progress(idx.len() as u64 + 1, idx.len() as u64 + 1);
    report.pictures = check.pictures;
    report.links = check.links;
    report.tables = check.tables;
    let mut warnings = notes(&report, &layouts, crate::edit::source_encrypted(&doc), "PowerPoint", "the presentation");
    warnings.retain(|w| !w.contains("smaller to fit their column"));
    if scaled > 0 {
        warnings.push(format!("{scaled} page(s) of another size were scaled to the slide size (the first page's)."));
    }
    Ok((c, report, warnings))
}
