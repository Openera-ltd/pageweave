//! WordprocessingML package writer (ECMA-376 transitional, the subset Word, LibreOffice and other
//! editors read): one section per band of each page with the page's own size and margins,
//! columns, paragraphs with exact spacing and indents, formatted runs, links, inline pictures and
//! pictures behind text. No macros, no external content other than web/mail links.

use super::layout::{Align, Item, PageLayout, Paragraph, Picture, RowPicture, Run, TableItem, VertAlign, CELL_PAD};
use pw_core::{PwError, PwResult};
use std::fmt::Write as _;
use std::io::Write as _;
use zip::write::SimpleFileOptions;
use zip::CompressionMethod;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaKind {
    Png,
    Jpeg,
}

impl MediaKind {
    pub fn ext(self) -> &'static str {
        match self {
            MediaKind::Png => "png",
            MediaKind::Jpeg => "jpeg",
        }
    }
}

pub struct Media {
    pub bytes: Vec<u8>,
    pub kind: MediaKind,
}

pub struct DocxInput<'a> {
    pub pages: &'a [PageLayout],
    /// Picture data by picture id (ids are global across pages).
    pub media: &'a [Media],
    /// Link targets by link id (already checked: http, https or mailto only).
    pub links: &'a [String],
    pub title: Option<&'a str>,
}

const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
const R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const WP: &str = "http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing";
const A: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
const PIC: &str = "http://schemas.openxmlformats.org/drawingml/2006/picture";

fn twips(pt: f32) -> i64 {
    (pt * 20.0).round() as i64
}

fn emu(pt: f32) -> i64 {
    (pt * 12_700.0).round().max(1.0) as i64
}

/// XML-escape text, dropping characters XML 1.0 cannot carry.
pub fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            '\t' | '\n' | '\r' => o.push(c),
            c if (c as u32) < 0x20 || c == '\u{fffe}' || c == '\u{ffff}' => {}
            c => o.push(c),
        }
    }
    o
}

/// Characters `esc` drops (reported by the caller and excluded from verification).
pub fn is_xml_invalid(c: char) -> bool {
    ((c as u32) < 0x20 && !matches!(c, '\t' | '\n' | '\r')) || c == '\u{fffe}' || c == '\u{ffff}'
}

/// Font family for Word from a PDF font name: subset tag and style suffixes removed, the
/// standard PDF fonts mapped to the fonts Windows ships.
pub fn family(pdf_name: &str) -> String {
    let mut n = pdf_name.trim();
    if let Some((tag, rest)) = n.split_once('+') {
        if tag.len() == 6 && tag.chars().all(|c| c.is_ascii_uppercase()) {
            n = rest;
        }
    }
    let base = n.split([',', '-']).next().unwrap_or(n).trim();
    let base = base.trim_end_matches("MT").trim_end_matches("PS").trim();
    let lower = base.to_ascii_lowercase();
    let mapped = match lower.as_str() {
        "helvetica" | "arial" | "arialmt" => "Arial",
        "times" | "timesroman" | "times new roman" | "timesnewroman" | "timesnewromanps" => "Times New Roman",
        "courier" | "couriernew" | "courier new" => "Courier New",
        "symbol" => "Symbol",
        "zapfdingbats" => "Wingdings",
        "" => "Arial",
        _ => "",
    };
    if !mapped.is_empty() {
        return mapped.to_string();
    }
    // Split CamelCase family names ("MinionPro" → "Minion Pro") the way installed fonts are named.
    let mut out = String::new();
    let chars: Vec<char> = base.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        // A two-letter word stays attached ("DejaVuSans" → "DejaVu Sans").
        let word_len = chars[i..].iter().skip(1).take_while(|x| x.is_ascii_lowercase()).count() + 1;
        let short = word_len == 2 && chars.get(i + word_len).is_some_and(|x| x.is_ascii_uppercase());
        if i > 0 && c.is_ascii_uppercase() && chars[i - 1].is_ascii_lowercase() && !short {
            out.push(' ');
        }
        out.push(c);
    }
    out
}

fn hex([r, g, b]: [u8; 3]) -> String {
    format!("{r:02X}{g:02X}{b:02X}")
}

struct Ctx<'a> {
    input: &'a DocxInput<'a>,
    body: String,
    next_doc_pr: u32,
}

impl Ctx<'_> {
    fn run(&mut self, r: &Run) {
        let font = esc(&family(&r.font));
        let mut x = String::new();
        let _ = write!(
            x,
            "<w:r><w:rPr><w:rFonts w:ascii=\"{font}\" w:hAnsi=\"{font}\" w:cs=\"{font}\" w:eastAsia=\"{font}\"/>"
        );
        if r.bold {
            x.push_str("<w:b/><w:bCs/>");
        }
        if r.italic {
            x.push_str("<w:i/><w:iCs/>");
        }
        if r.color != [0, 0, 0] {
            let _ = write!(x, "<w:color w:val=\"{}\"/>", hex(r.color));
        }
        // Word draws super/subscript at about two thirds of the size: compensate so the result
        // matches the size drawn in the PDF.
        let size = match r.vert {
            VertAlign::Baseline => r.size,
            _ => r.size / 0.65,
        };
        let hp = ((size * 2.0).round() as i64).clamp(2, 3276);
        let _ = write!(x, "<w:sz w:val=\"{hp}\"/><w:szCs w:val=\"{hp}\"/>");
        match r.vert {
            VertAlign::Superscript => x.push_str("<w:vertAlign w:val=\"superscript\"/>"),
            VertAlign::Subscript => x.push_str("<w:vertAlign w:val=\"subscript\"/>"),
            VertAlign::Baseline => {}
        }
        // Right-to-left letters: Word orders and joins them right to left (D-039). Schema order:
        // after vertAlign.
        if r.text.chars().any(super::layout::is_rtl) {
            x.push_str("<w:rtl/>");
        }
        x.push_str("</w:rPr>");
        for (i, part) in r.text.split('\t').enumerate() {
            if i > 0 {
                x.push_str("<w:tab/>");
            }
            if !part.is_empty() {
                let _ = write!(x, "<w:t xml:space=\"preserve\">{}</w:t>", esc(part));
            }
        }
        x.push_str("</w:r>");
        self.body.push_str(&x);
    }

    fn picture_xml(&mut self, id: usize, width: f32, height: f32, anchor: Option<(f32, f32, bool)>) -> String {
        let n = self.next_doc_pr;
        self.next_doc_pr += 1;
        let (cx, cy) = (emu(width), emu(height));
        let graphic = format!(
            "<wp:docPr id=\"{n}\" name=\"Picture {n}\"/><wp:cNvGraphicFramePr><a:graphicFrameLocks noChangeAspect=\"1\"/></wp:cNvGraphicFramePr>\
<a:graphic><a:graphicData uri=\"{PIC}\"><pic:pic><pic:nvPicPr><pic:cNvPr id=\"{n}\" name=\"Picture {n}\"/><pic:cNvPicPr/></pic:nvPicPr>\
<pic:blipFill><a:blip r:embed=\"rIdImg{id}\"/><a:stretch><a:fillRect/></a:stretch></pic:blipFill>\
<pic:spPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"{cx}\" cy=\"{cy}\"/></a:xfrm><a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom></pic:spPr></pic:pic></a:graphicData></a:graphic>"
        );
        match anchor {
            None => format!(
                "<w:r><w:drawing><wp:inline distT=\"0\" distB=\"0\" distL=\"0\" distR=\"0\"><wp:extent cx=\"{cx}\" cy=\"{cy}\"/><wp:effectExtent l=\"0\" t=\"0\" r=\"0\" b=\"0\"/>{graphic}</wp:inline></w:drawing></w:r>"
            ),
            Some((x, y, behind)) => format!(
                "<w:r><w:drawing><wp:anchor distT=\"0\" distB=\"0\" distL=\"{gap}\" distR=\"{gap}\" simplePos=\"0\" relativeHeight=\"{n}\" behindDoc=\"{}\" locked=\"0\" layoutInCell=\"1\" allowOverlap=\"1\">\
<wp:simplePos x=\"0\" y=\"0\"/><wp:positionH relativeFrom=\"page\"><wp:posOffset>{}</wp:posOffset></wp:positionH>\
<wp:positionV relativeFrom=\"page\"><wp:posOffset>{}</wp:posOffset></wp:positionV><wp:extent cx=\"{cx}\" cy=\"{cy}\"/><wp:effectExtent l=\"0\" t=\"0\" r=\"0\" b=\"0\"/>{}{graphic}</wp:anchor></w:drawing></w:r>",
                behind as u8,
                (x * 12_700.0).round() as i64,
                (y * 12_700.0).round() as i64,
                if behind { "<wp:wrapNone/>" } else { "<wp:wrapSquare wrapText=\"bothSides\"/>" },
                gap = if behind { 0 } else { 76_200 },
            ),
        }
    }

    fn para(&mut self, p: &Paragraph, prefix: &str, sect: Option<&str>) {
        self.body.push_str("<w:p><w:pPr>");
        if let Some(h) = p.heading {
            let _ = write!(self.body, "<w:pStyle w:val=\"Heading{h}\"/>");
        }
        self.body.push_str("<w:widowControl w:val=\"0\"/>");
        // Borders (a box's coloured bar, a line below) and shading, in schema order.
        let hex = |c: [u8; 3]| format!("{:02X}{:02X}{:02X}", c[0], c[1], c[2]);
        let eighths = |w: f32| ((w * 8.0).round() as i64).clamp(2, 96);
        if p.deco.left_bar.is_some() || p.deco.rule_below.is_some() {
            self.body.push_str("<w:pBdr>");
            if let Some((c, w)) = p.deco.left_bar {
                let _ = write!(
                    self.body,
                    "<w:left w:val=\"single\" w:sz=\"{}\" w:space=\"6\" w:color=\"{}\"/>",
                    eighths(w),
                    hex(c)
                );
            }
            if let Some((c, w)) = p.deco.rule_below {
                let _ = write!(
                    self.body,
                    "<w:bottom w:val=\"single\" w:sz=\"{}\" w:space=\"4\" w:color=\"{}\"/>",
                    eighths(w),
                    hex(c)
                );
            }
            self.body.push_str("</w:pBdr>");
        }
        if let Some(c) = p.deco.shade {
            let _ = write!(self.body, "<w:shd w:val=\"clear\" w:color=\"auto\" w:fill=\"{}\"/>", hex(c));
        }
        if !p.tabs.is_empty() {
            self.body.push_str("<w:tabs>");
            for t in &p.tabs {
                let _ = write!(self.body, "<w:tab w:val=\"left\" w:pos=\"{}\"/>", twips(*t).clamp(0, 31_680));
            }
            self.body.push_str("</w:tabs>");
        }
        // A right-to-left paragraph (schema order: bidi before spacing).
        let text: String = p.runs.iter().map(|r| r.text.as_str()).collect();
        let rtl = super::layout::mostly_rtl(&text);
        if rtl {
            self.body.push_str("<w:bidi/>");
        }
        let _ = write!(
            self.body,
            "<w:spacing w:before=\"{}\" w:after=\"0\" w:line=\"{}\" w:lineRule=\"exact\"/>",
            twips(p.space_before).clamp(0, 31_680),
            twips(p.pitch).clamp(20, 31_680)
        );
        let left = twips(p.indent).clamp(0, 31_680);
        let first = twips(p.first_line).clamp(-left, 31_680);
        if rtl {
            // In a right-to-left paragraph "left" is the start (the right side): the visual left
            // indent is its end ("right"). Its lines start on the right, so no first-line indent.
            if left != 0 {
                let _ = write!(self.body, "<w:ind w:right=\"{left}\"/>");
            }
        } else if left != 0 || first != 0 {
            let _ = write!(self.body, "<w:ind w:left=\"{left}\"");
            if first >= 0 {
                let _ = write!(self.body, " w:firstLine=\"{first}\"/>");
            } else {
                let _ = write!(self.body, " w:hanging=\"{}\"/>", -first);
            }
        }
        // A right-to-left paragraph reads from the right; in it Word takes "left" and "right" as the
        // paragraph's start and end, so the visual alignment is written the other way round.
        let jc = match (p.align, rtl) {
            (Align::Left, false) | (Align::Right, true) => "left",
            (Align::Right, false) | (Align::Left, true) => "right",
            (Align::Center, _) => "center",
            (Align::Justify, _) => "both",
        };
        let _ = write!(self.body, "<w:jc w:val=\"{jc}\"/>");
        if let Some(s) = sect {
            self.body.push_str(s);
        }
        self.body.push_str("</w:pPr>");
        self.body.push_str(prefix);
        let mut i = 0;
        while i < p.runs.len() {
            match p.runs[i].link {
                Some(l) if l < self.input.links.len() => {
                    let _ = write!(self.body, "<w:hyperlink r:id=\"rIdLink{l}\">");
                    while i < p.runs.len() && p.runs[i].link == Some(l) {
                        let r = p.runs[i].clone();
                        self.run(&r);
                        i += 1;
                    }
                    self.body.push_str("</w:hyperlink>");
                }
                _ => {
                    let r = p.runs[i].clone();
                    self.run(&r);
                    i += 1;
                }
            }
        }
        self.body.push_str("</w:p>");
    }

    /// A spacer paragraph (exact height) that can carry a prefix and section properties.
    fn spacer(&mut self, height: f32, prefix: &str, sect: Option<&str>) {
        let _ = write!(
            self.body,
            "<w:p><w:pPr><w:widowControl w:val=\"0\"/><w:spacing w:before=\"0\" w:after=\"0\" w:line=\"{}\" w:lineRule=\"exact\"/>",
            twips(height.max(1.0)).clamp(20, 31_680)
        );
        if let Some(s) = sect {
            self.body.push_str(s);
        }
        self.body.push_str("</w:pPr>");
        self.body.push_str(prefix);
        self.body.push_str("</w:p>");
    }

    fn table(&mut self, t: &TableItem, indent: f32, space_before: f32, prefix: &str, sect: Option<&str>) {
        // The gap above (and any column break) goes in a spacer paragraph: tables have no spacing.
        self.spacer(space_before, prefix, None);
        let total: f32 = t.widths.iter().sum();
        let color =
            t.border.map(|c| format!("{:02X}{:02X}{:02X}", c[0], c[1], c[2])).unwrap_or_else(|| "000000".into());
        let border = format!("w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"{color}\"");
        let _ = write!(
            self.body,
            "<w:tbl><w:tblPr><w:tblW w:w=\"{}\" w:type=\"dxa\"/><w:tblInd w:w=\"{}\" w:type=\"dxa\"/>\
<w:tblBorders><w:top {border}/><w:left {border}/><w:bottom {border}/><w:right {border}/><w:insideH {border}/><w:insideV {border}/></w:tblBorders>\
<w:tblLayout w:type=\"fixed\"/><w:tblCellMar><w:top w:w=\"0\" w:type=\"dxa\"/><w:left w:w=\"{pad}\" w:type=\"dxa\"/><w:bottom w:w=\"0\" w:type=\"dxa\"/><w:right w:w=\"{pad}\" w:type=\"dxa\"/></w:tblCellMar>\
<w:tblLook w:val=\"0000\"/></w:tblPr><w:tblGrid>",
            twips(total),
            twips(indent).clamp(0, 31_680),
            pad = twips(CELL_PAD)
        );
        for w in &t.widths {
            let _ = write!(self.body, "<w:gridCol w:w=\"{}\"/>", twips(*w).max(20));
        }
        self.body.push_str("</w:tblGrid>");
        let cols = t.widths.len();
        for (r, h) in t.heights.iter().enumerate() {
            let _ = write!(
                self.body,
                "<w:tr><w:trPr><w:trHeight w:val=\"{}\" w:hRule=\"atLeast\"/></w:trPr>",
                twips(*h).max(20)
            );
            let mut c = 0;
            while c < cols {
                let Some(cell) = t
                    .cells
                    .iter()
                    .find(|x| r >= x.row && r < x.row + x.row_span && c >= x.col && c < x.col + x.col_span)
                else {
                    // A slot no cell covers cannot happen for a detected grid; keep the row valid anyway.
                    let _ = write!(
                        self.body,
                        "<w:tc><w:tcPr><w:tcW w:w=\"{}\" w:type=\"dxa\"/></w:tcPr><w:p/></w:tc>",
                        twips(t.widths[c])
                    );
                    c += 1;
                    continue;
                };
                let width: f32 = t.widths[cell.col..cell.col + cell.col_span].iter().sum();
                let _ = write!(self.body, "<w:tc><w:tcPr><w:tcW w:w=\"{}\" w:type=\"dxa\"/>", twips(width));
                if cell.col_span > 1 {
                    let _ = write!(self.body, "<w:gridSpan w:val=\"{}\"/>", cell.col_span);
                }
                let first_row = cell.row == r;
                if cell.row_span > 1 {
                    self.body.push_str(if first_row { "<w:vMerge w:val=\"restart\"/>" } else { "<w:vMerge/>" });
                }
                if cell.open.contains(&true) {
                    self.body.push_str("<w:tcBorders>");
                    if cell.open[0] {
                        self.body.push_str("<w:top w:val=\"nil\"/>");
                    }
                    if cell.open[1] {
                        self.body.push_str("<w:bottom w:val=\"nil\"/>");
                    }
                    self.body.push_str("</w:tcBorders>");
                }
                if let Some(c) = cell.fill {
                    let _ = write!(
                        self.body,
                        "<w:shd w:val=\"clear\" w:color=\"auto\" w:fill=\"{:02X}{:02X}{:02X}\"/>",
                        c[0], c[1], c[2]
                    );
                }
                if cell.middle {
                    self.body.push_str("<w:vAlign w:val=\"center\"/>");
                }
                self.body.push_str("</w:tcPr>");
                if first_row && !cell.paragraphs.is_empty() {
                    for p in &cell.paragraphs {
                        self.para(p, "", None);
                    }
                } else {
                    self.body.push_str("<w:p/>");
                }
                self.body.push_str("</w:tc>");
                c = cell.col + cell.col_span;
            }
            self.body.push_str("</w:tr>");
        }
        self.body.push_str("</w:tbl>");
        // A table cannot carry section properties, and two tables in a row would merge.
        self.spacer(1.0, "", sect);
    }

    fn pictures_para(&mut self, row: &[RowPicture], space_before: f32, prefix: &str, sect: Option<&str>) {
        let _ = write!(self.body, "<w:p><w:pPr><w:widowControl w:val=\"0\"/>");
        // Pictures after the first sit at tab stops at their own positions.
        if row.len() > 1 {
            self.body.push_str("<w:tabs>");
            for r in &row[1..] {
                let _ = write!(self.body, "<w:tab w:val=\"left\" w:pos=\"{}\"/>", twips(r.offset).clamp(0, 31_680));
            }
            self.body.push_str("</w:tabs>");
        }
        let _ = write!(
            self.body,
            "<w:spacing w:before=\"{}\" w:after=\"0\" w:line=\"240\" w:lineRule=\"auto\"/>",
            twips(space_before).clamp(0, 31_680)
        );
        if let Some(first) = row.first() {
            if first.offset > 0.0 {
                let _ = write!(self.body, "<w:ind w:left=\"{}\"/>", twips(first.offset).clamp(0, 31_680));
            }
        }
        if let Some(s) = sect {
            self.body.push_str(s);
        }
        self.body.push_str("</w:pPr>");
        self.body.push_str(prefix);
        for (i, r) in row.iter().enumerate() {
            if i > 0 {
                self.body.push_str("<w:r><w:tab/></w:r>");
            }
            let pic = self.picture_xml(r.id, r.width, r.height, None);
            self.body.push_str(&pic);
        }
        self.body.push_str("</w:p>");
    }
}

fn sect_pr(page: &PageLayout, band: &super::layout::Band, continuous: bool) -> String {
    let mut s = String::from("<w:sectPr>");
    let _ = write!(s, "<w:type w:val=\"{}\"/>", if continuous { "continuous" } else { "nextPage" });
    let (w, h) = (twips(page.width).clamp(2880, 31_680), twips(page.height).clamp(2880, 31_680));
    let orient = if w > h { " w:orient=\"landscape\"" } else { "" };
    let _ = write!(s, "<w:pgSz w:w=\"{w}\" w:h=\"{h}\"{orient}/>");
    let _ = write!(
        s,
        "<w:pgMar w:top=\"{}\" w:right=\"{}\" w:bottom=\"{}\" w:left=\"{}\" w:header=\"0\" w:footer=\"0\" w:gutter=\"0\"/>",
        twips(page.margin_top),
        twips(page.margin_right),
        twips(page.margin_bottom),
        twips(page.margin_left)
    );
    if band.columns.len() > 1 {
        let _ = write!(s, "<w:cols w:num=\"{}\" w:equalWidth=\"0\">", band.columns.len());
        for (i, c) in band.columns.iter().enumerate() {
            let space = band.columns.get(i + 1).map(|n| n.left - c.right).unwrap_or(0.0);
            let _ =
                write!(s, "<w:col w:w=\"{}\" w:space=\"{}\"/>", twips(c.right - c.left).max(20), twips(space).max(0));
        }
        s.push_str("</w:cols>");
    } else {
        s.push_str("<w:cols w:space=\"720\"/>");
    }
    s.push_str("</w:sectPr>");
    s
}

fn document_xml(input: &DocxInput) -> String {
    let mut ctx = Ctx { input, body: String::new(), next_doc_pr: 1 };
    // Flatten to sections: (page, band).
    let mut sections: Vec<(usize, usize)> = Vec::new();
    for (pi, p) in input.pages.iter().enumerate() {
        for bi in 0..p.bands.len().max(1) {
            sections.push((pi, bi));
        }
    }
    let empty_band = super::layout::Band { columns: vec![] };
    for (si, &(pi, bi)) in sections.iter().enumerate() {
        let page = &input.pages[pi];
        let band = page.bands.get(bi).unwrap_or(&empty_band);
        let last_section = si + 1 == sections.len();
        // A section's properties go in its last paragraph; the document's last section is the
        // body's own sectPr.
        let continuous = bi > 0;
        let sect = sect_pr(page, band, continuous);
        // Backgrounds are anchored in the first paragraph of the page.
        let mut prefix = String::new();
        if bi == 0 {
            for (pics, behind) in [(&page.backgrounds, true), (&page.floating, false)] {
                for Picture { id, rect } in pics {
                    let x = ctx.picture_xml(
                        *id,
                        rect.width(),
                        rect.height(),
                        Some((rect.left, page.height - rect.top, behind)),
                    );
                    prefix.push_str(&x);
                }
            }
        }
        // Paragraph slots in this section, columns separated by column breaks.
        let mut slots: Vec<(Option<&Item>, String)> = Vec::new();
        for (ci, col) in band.columns.iter().enumerate() {
            let brk = if ci > 0 { "<w:r><w:br w:type=\"column\"/></w:r>" } else { "" };
            if col.items.is_empty() {
                slots.push((None, brk.to_string()));
            } else {
                for (ii, it) in col.items.iter().enumerate() {
                    slots.push((Some(it), if ii == 0 { brk.to_string() } else { String::new() }));
                }
            }
        }
        if slots.is_empty() {
            slots.push((None, String::new()));
        }
        if let Some(first) = slots.first_mut() {
            first.1 = format!("{prefix}{}", first.1);
        }
        let n = slots.len();
        for (k, (item, pre)) in slots.into_iter().enumerate() {
            let s = (k + 1 == n && !last_section).then_some(sect.as_str());
            match item {
                Some(Item::Para(p)) => ctx.para(p, &pre, s),
                Some(Item::Pictures { row, space_before }) => ctx.pictures_para(row, *space_before, &pre, s),
                Some(Item::Table { table, indent, space_before }) => ctx.table(table, *indent, *space_before, &pre, s),
                None => {
                    ctx.body.push_str("<w:p><w:pPr>");
                    if let Some(s) = s {
                        ctx.body.push_str(s);
                    }
                    ctx.body.push_str("</w:pPr>");
                    ctx.body.push_str(&pre);
                    ctx.body.push_str("</w:p>");
                }
            }
        }
        if last_section {
            ctx.body.push_str(&sect);
        }
    }
    if sections.is_empty() {
        ctx.body.push_str("<w:p/><w:sectPr/>");
    }
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<w:document xmlns:w=\"{W}\" xmlns:r=\"{R}\" xmlns:wp=\"{WP}\" xmlns:a=\"{A}\" xmlns:pic=\"{PIC}\"><w:body>{}</w:body></w:document>",
        ctx.body
    )
}

fn styles_xml() -> String {
    let mut s = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<w:styles xmlns:w=\"{W}\">\
<w:docDefaults><w:rPrDefault><w:rPr><w:rFonts w:ascii=\"Arial\" w:hAnsi=\"Arial\" w:cs=\"Arial\" w:eastAsia=\"Arial\"/><w:sz w:val=\"22\"/><w:szCs w:val=\"22\"/><w:lang w:val=\"en-US\"/></w:rPr></w:rPrDefault>\
<w:pPrDefault><w:pPr><w:spacing w:after=\"0\" w:line=\"240\" w:lineRule=\"auto\"/></w:pPr></w:pPrDefault></w:docDefaults>\
<w:style w:type=\"paragraph\" w:default=\"1\" w:styleId=\"Normal\"><w:name w:val=\"Normal\"/><w:qFormat/></w:style>"
    );
    for (i, name) in ["heading 1", "heading 2", "heading 3"].iter().enumerate() {
        let _ = write!(
            s,
            "<w:style w:type=\"paragraph\" w:styleId=\"Heading{}\"><w:name w:val=\"{name}\"/><w:basedOn w:val=\"Normal\"/><w:next w:val=\"Normal\"/><w:qFormat/><w:pPr><w:keepNext/><w:outlineLvl w:val=\"{i}\"/></w:pPr></w:style>",
            i + 1
        );
    }
    s.push_str("</w:styles>");
    s
}

fn content_types() -> String {
    "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
<Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\
<Default Extension=\"xml\" ContentType=\"application/xml\"/>\
<Default Extension=\"png\" ContentType=\"image/png\"/>\
<Default Extension=\"jpeg\" ContentType=\"image/jpeg\"/>\
<Override PartName=\"/word/document.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml\"/>\
<Override PartName=\"/word/styles.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml\"/>\
<Override PartName=\"/word/settings.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.settings+xml\"/>\
<Override PartName=\"/docProps/core.xml\" ContentType=\"application/vnd.openxmlformats-package.core-properties+xml\"/>\
<Override PartName=\"/docProps/app.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.extended-properties+xml\"/>\
</Types>"
        .to_string()
}

fn package_rels() -> String {
    "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
<Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"word/document.xml\"/>\
<Relationship Id=\"rId2\" Type=\"http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties\" Target=\"docProps/core.xml\"/>\
<Relationship Id=\"rId3\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/extended-properties\" Target=\"docProps/app.xml\"/>\
</Relationships>"
        .to_string()
}

fn document_rels(input: &DocxInput) -> String {
    let mut s = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
<Relationship Id=\"rIdStyles\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles\" Target=\"styles.xml\"/>\
<Relationship Id=\"rIdSettings\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/settings\" Target=\"settings.xml\"/>",
    );
    for (i, m) in input.media.iter().enumerate() {
        let _ = write!(
            s,
            "<Relationship Id=\"rIdImg{i}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/image\" Target=\"media/image{i}.{}\"/>",
            m.kind.ext()
        );
    }
    for (i, l) in input.links.iter().enumerate() {
        let _ = write!(
            s,
            "<Relationship Id=\"rIdLink{i}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink\" Target=\"{}\" TargetMode=\"External\"/>",
            esc(l)
        );
    }
    s.push_str("</Relationships>");
    s
}

fn settings_xml() -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<w:settings xmlns:w=\"{W}\"><w:defaultTabStop w:val=\"720\"/>\
<w:compat><w:compatSetting w:name=\"compatibilityMode\" w:uri=\"http://schemas.microsoft.com/office/word\" w:val=\"15\"/></w:compat></w:settings>"
    )
}

fn core_xml(title: Option<&str>) -> String {
    let t = title.map(|t| format!("<dc:title>{}</dc:title>", esc(t))).unwrap_or_default();
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<cp:coreProperties xmlns:cp=\"http://schemas.openxmlformats.org/package/2006/metadata/core-properties\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\">{t}</cp:coreProperties>"
    )
}

fn app_xml() -> String {
    "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Properties xmlns=\"http://schemas.openxmlformats.org/officeDocument/2006/extended-properties\"><Application>PageWeave</Application></Properties>"
        .to_string()
}

fn zip_err<E>(_: E) -> PwError {
    PwError::internal("docx.write", "The Word file could not be assembled.")
}

/// Assemble the package.
pub fn write_docx(input: &DocxInput) -> PwResult<Vec<u8>> {
    let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let xml = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    let parts: [(&str, String); 7] = [
        ("[Content_Types].xml", content_types()),
        ("_rels/.rels", package_rels()),
        ("docProps/core.xml", core_xml(input.title)),
        ("docProps/app.xml", app_xml()),
        ("word/document.xml", document_xml(input)),
        ("word/styles.xml", styles_xml()),
        ("word/settings.xml", settings_xml()),
    ];
    for (name, body) in parts {
        z.start_file(name, xml).map_err(zip_err)?;
        z.write_all(body.as_bytes()).map_err(zip_err)?;
    }
    z.start_file("word/_rels/document.xml.rels", xml).map_err(zip_err)?;
    z.write_all(document_rels(input).as_bytes()).map_err(zip_err)?;
    for (i, m) in input.media.iter().enumerate() {
        z.start_file(format!("word/media/image{i}.{}", m.kind.ext()), stored).map_err(zip_err)?;
        z.write_all(&m.bytes).map_err(zip_err)?;
    }
    Ok(z.finish().map_err(zip_err)?.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn font_families_for_word() {
        assert_eq!(family("ABCDEF+Helvetica-Bold"), "Arial");
        assert_eq!(family("Times-Roman"), "Times New Roman");
        assert_eq!(family("Courier"), "Courier New");
        assert_eq!(family("QWERTY+MinionPro-Regular"), "Minion Pro");
        assert_eq!(family("ArialMT"), "Arial");
        assert_eq!(family("DejaVuSans-Bold"), "DejaVu Sans");
        assert_eq!(family("SegoeUI"), "Segoe UI");
        assert_eq!(family("Calibri,Bold"), "Calibri");
        assert_eq!(family(""), "Arial");
        assert_eq!(family("abc+Foo"), "abc+Foo", "only six capitals are a subset tag");
    }

    #[test]
    fn escaping_drops_what_xml_cannot_carry() {
        assert_eq!(esc("a<b>&\"c\u{1}\u{fffe}"), "a&lt;b&gt;&amp;&quot;c");
        assert!(is_xml_invalid('\u{1}') && !is_xml_invalid('\t') && !is_xml_invalid('é'));
    }
}
