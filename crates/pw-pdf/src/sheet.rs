//! Tables to spreadsheets (PRD 06 "Spreadsheet/table extraction"): the ruled tables of a document
//! as an Excel workbook (.xlsx, one sheet per table, merged cells kept) or as CSV files (one per
//! table). Values are never re-typed in a way that can change them: a cell becomes a number only
//! when its text is a plain decimal number that a spreadsheet stores exactly (no leading zeros,
//! no grouping separators, at most 15 significant digits); identifiers, codes, dates, amounts with
//! separators and everything else stay text. Every file is read back and compared cell by cell
//! before it is written.

use crate::document::{load, OpenOptions};
use crate::docx::layout::TableItem;
use crate::{engine, map_pdfium_error};
use pdfium_render::prelude::*;
use pw_core::output::{write_atomic_bytes, CollisionPolicy, CommittedOutput};
use pw_core::range::parse_page_ranges;
use pw_core::{ErrorCategory, PwError, PwResult};
use quick_xml::events::Event;
use quick_xml::{Reader, XmlVersion};
use serde::Serialize;
use std::fmt::Write as _;
use std::io::{Read, Write as _};
use std::path::{Path, PathBuf};

/// Excel's limits.
const MAX_SHEET_ROWS: usize = 1_048_576;
const MAX_CELL_CHARS: usize = 32_767;

/// One extracted table as a grid of cell texts (merged cells: text in the first cell).
#[derive(Debug, Clone, PartialEq)]
pub struct Grid {
    /// 1-based page.
    pub page: u32,
    /// 1-based table number on its page.
    pub index: u32,
    pub cells: Vec<Vec<String>>,
    /// (row, col, rows, cols) of merged areas.
    pub merges: Vec<(usize, usize, usize, usize)>,
    /// Column widths in points.
    pub widths: Vec<f32>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct TablesReport {
    pub pages: u32,
    pub tables: usize,
    /// (page, table number, rows, columns) of every table found.
    pub found: Vec<(u32, u32, usize, usize)>,
    /// Cells written as numbers (the rest are text).
    pub number_cells: usize,
    pub text_cells: usize,
}

/// A plain decimal number that a spreadsheet stores exactly, or `None` to keep the text.
pub fn exact_number(s: &str) -> Option<&str> {
    let t = s.trim();
    if t != s || t.is_empty() || t.len() > 17 {
        return None;
    }
    let body = t.strip_prefix('-').unwrap_or(t);
    let (int, frac) = match body.split_once('.') {
        Some((i, f)) => (i, Some(f)),
        None => (body, None),
    };
    if int.is_empty() || !int.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    if int.len() > 1 && int.starts_with('0') {
        return None; // leading zero: an identifier or code
    }
    if let Some(f) = frac {
        // "12.40" would show as 12.4: trailing zeros are formatting worth keeping.
        if f.is_empty() || !f.bytes().all(|b| b.is_ascii_digit()) || f.ends_with('0') {
            return None;
        }
    }
    let digits = int.trim_start_matches('0').len() + frac.map_or(0, str::len);
    if digits > 15 {
        return None; // beyond a spreadsheet's 15 exact digits
    }
    if body == "0" && t.starts_with('-') {
        return None;
    }
    Some(t)
}

fn cell_text(t: &TableItem, r: usize, c: usize) -> Option<String> {
    let cell = t.cells.iter().find(|x| x.row == r && x.col == c)?;
    Some(
        cell.paragraphs
            .iter()
            .map(|p| p.runs.iter().map(|r| r.text.replace('\t', " ")).collect::<String>().trim().to_string())
            .collect::<Vec<_>>()
            .join("\n"),
    )
}

pub fn grid_of(t: &TableItem, page: u32, index: u32) -> Grid {
    let (rows, cols) = (t.heights.len(), t.widths.len());
    let mut cells = vec![vec![String::new(); cols]; rows];
    let mut merges = Vec::new();
    for c in &t.cells {
        if c.row < rows && c.col < cols {
            cells[c.row][c.col] = cell_text(t, c.row, c.col).unwrap_or_default();
            if c.row_span > 1 || c.col_span > 1 {
                merges.push((c.row, c.col, c.row_span, c.col_span));
            }
        }
    }
    Grid { page, index, cells, merges, widths: t.widths.clone() }
}

fn esc(s: &str) -> String {
    crate::docx::writer::esc(s)
}

/// Spreadsheet column name for a 0-based index (A, B, …, Z, AA, …).
pub fn col_name(mut c: usize) -> String {
    let mut s = Vec::new();
    loop {
        s.push(b'A' + (c % 26) as u8);
        if c < 26 {
            break;
        }
        c = c / 26 - 1;
    }
    s.reverse();
    String::from_utf8(s).unwrap_or_default()
}

fn sheet_name(g: &Grid) -> String {
    format!("Page {} table {}", g.page, g.index)
}

fn sheet_xml(g: &Grid, numbers: &mut usize, texts: &mut usize) -> String {
    let mut s = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<worksheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\"><cols>",
    );
    for (i, w) in g.widths.iter().enumerate() {
        // Excel's width unit is about one digit of the default font (~5.25 pt).
        let _ = write!(
            s,
            "<col min=\"{0}\" max=\"{0}\" width=\"{1:.2}\" customWidth=\"1\"/>",
            i + 1,
            (w / 5.25).clamp(2.0, 255.0)
        );
    }
    s.push_str("</cols><sheetData>");
    for (r, row) in g.cells.iter().enumerate() {
        let _ = write!(s, "<row r=\"{}\">", r + 1);
        for (c, v) in row.iter().enumerate() {
            if v.is_empty() {
                continue;
            }
            let addr = format!("{}{}", col_name(c), r + 1);
            match exact_number(v) {
                Some(n) => {
                    *numbers += 1;
                    let _ = write!(s, "<c r=\"{addr}\"><v>{n}</v></c>");
                }
                None => {
                    *texts += 1;
                    let _ = write!(
                        s,
                        "<c r=\"{addr}\" t=\"inlineStr\"><is><t xml:space=\"preserve\">{}</t></is></c>",
                        esc(v)
                    );
                }
            }
        }
        s.push_str("</row>");
    }
    s.push_str("</sheetData>");
    if !g.merges.is_empty() {
        let _ = write!(s, "<mergeCells count=\"{}\">", g.merges.len());
        for &(r, c, rs, cs) in &g.merges {
            let _ = write!(s, "<mergeCell ref=\"{}{}:{}{}\"/>", col_name(c), r + 1, col_name(c + cs - 1), r + rs);
        }
        s.push_str("</mergeCells>");
    }
    s.push_str("</worksheet>");
    s
}

fn zip_err<E>(_: E) -> PwError {
    PwError::internal("xlsx.write", "The workbook could not be assembled.")
}

/// An Excel workbook with one sheet per table.
pub fn write_xlsx(grids: &[Grid]) -> PwResult<(Vec<u8>, usize, usize)> {
    let (mut numbers, mut texts) = (0, 0);
    let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let o = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    let put = |z: &mut zip::ZipWriter<std::io::Cursor<Vec<u8>>>, name: &str, body: &str| -> PwResult<()> {
        z.start_file(name, o).map_err(zip_err)?;
        z.write_all(body.as_bytes()).map_err(zip_err)
    };
    let mut types = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"xml\" ContentType=\"application/xml\"/><Override PartName=\"/xl/workbook.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml\"/><Override PartName=\"/xl/styles.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml\"/>");
    for i in 0..grids.len() {
        let _ = write!(types, "<Override PartName=\"/xl/worksheets/sheet{}.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml\"/>", i + 1);
    }
    types.push_str("</Types>");
    put(&mut z, "[Content_Types].xml", &types)?;
    put(&mut z, "_rels/.rels", "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"xl/workbook.xml\"/></Relationships>")?;
    let mut wb = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<workbook xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\"><sheets>");
    let mut rels = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rIdStyles\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles\" Target=\"styles.xml\"/>");
    for (i, g) in grids.iter().enumerate() {
        let _ =
            write!(wb, "<sheet name=\"{}\" sheetId=\"{}\" r:id=\"rIdSheet{}\"/>", esc(&sheet_name(g)), i + 1, i + 1);
        let _ = write!(rels, "<Relationship Id=\"rIdSheet{0}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet\" Target=\"worksheets/sheet{0}.xml\"/>", i + 1);
    }
    wb.push_str("</sheets></workbook>");
    rels.push_str("</Relationships>");
    put(&mut z, "xl/workbook.xml", &wb)?;
    put(&mut z, "xl/_rels/workbook.xml.rels", &rels)?;
    put(&mut z, "xl/styles.xml", "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<styleSheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\"><fonts count=\"1\"><font><sz val=\"11\"/><name val=\"Calibri\"/></font></fonts><fills count=\"2\"><fill><patternFill patternType=\"none\"/></fill><fill><patternFill patternType=\"gray125\"/></fill></fills><borders count=\"1\"><border><left/><right/><top/><bottom/><diagonal/></border></borders><cellStyleXfs count=\"1\"><xf numFmtId=\"0\" fontId=\"0\" fillId=\"0\" borderId=\"0\"/></cellStyleXfs><cellXfs count=\"1\"><xf numFmtId=\"0\" fontId=\"0\" fillId=\"0\" borderId=\"0\" xfId=\"0\"/></cellXfs></styleSheet>")?;
    for (i, g) in grids.iter().enumerate() {
        let x = sheet_xml(g, &mut numbers, &mut texts);
        put(&mut z, &format!("xl/worksheets/sheet{}.xml", i + 1), &x)?;
    }
    let bytes = z.finish().map_err(zip_err)?.into_inner();
    Ok((bytes, numbers, texts))
}

fn bad(what: &str) -> PwError {
    PwError::new(
        ErrorCategory::ValidationFailed,
        "output.verify",
        format!("The table file failed verification ({what}) and was not saved."),
    )
}

/// Read a workbook back: per sheet (in order), the cell texts by position.
pub fn read_xlsx(bytes: &[u8]) -> PwResult<Vec<Vec<Vec<String>>>> {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|_| bad("the workbook does not open"))?;
    let mut sheets = Vec::new();
    for i in 1.. {
        let name = format!("xl/worksheets/sheet{i}.xml");
        let mut xml = Vec::new();
        match z.by_name(&name) {
            Ok(mut f) => {
                if f.size() > 512 * 1024 * 1024 {
                    return Err(bad("a sheet is too large"));
                }
                f.read_to_end(&mut xml).map_err(|_| bad("a sheet is unreadable"))?;
            }
            Err(_) => break,
        }
        let mut r = Reader::from_reader(xml.as_slice());
        let mut buf = Vec::new();
        let mut grid: Vec<Vec<String>> = Vec::new();
        let mut cur: Option<(usize, usize)> = None;
        let mut in_value = false;
        loop {
            match r.read_event_into(&mut buf) {
                Ok(Event::Start(e)) => match e.local_name().as_ref() {
                    "c" => {
                        let addr = e
                            .attributes()
                            .flatten()
                            .find(|a| a.key.as_ref() == "r")
                            .and_then(|a| a.normalized_value(XmlVersion::Implicit1_0).ok())
                            .map(|v| v.into_owned())
                            .ok_or_else(|| bad("a cell has no address"))?;
                        let letters: String = addr.chars().take_while(char::is_ascii_uppercase).collect();
                        let row: usize = addr[letters.len()..].parse().map_err(|_| bad("a cell address is invalid"))?;
                        let col = letters.bytes().fold(0usize, |a, b| a * 26 + (b - b'A' + 1) as usize) - 1;
                        cur = Some((row - 1, col));
                    }
                    "t" | "v" => in_value = true,
                    _ => {}
                },
                Ok(Event::Text(t)) if in_value => {
                    if let Some((row, col)) = cur {
                        while grid.len() <= row {
                            grid.push(Vec::new());
                        }
                        while grid[row].len() <= col {
                            grid[row].push(String::new());
                        }
                        grid[row][col].push_str(&t.xml10_content());
                    }
                }
                Ok(Event::GeneralRef(e)) if in_value => {
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
                    if let Some((row, col)) = cur {
                        while grid.len() <= row {
                            grid.push(Vec::new());
                        }
                        while grid[row].len() <= col {
                            grid[row].push(String::new());
                        }
                        grid[row][col].push(c);
                    }
                }
                Ok(Event::End(e)) => match e.local_name().as_ref() {
                    "t" | "v" => in_value = false,
                    "c" => cur = None,
                    _ => {}
                },
                Ok(Event::DocType(_)) => return Err(bad("unexpected document type")),
                Ok(Event::Eof) => break,
                Err(_) => return Err(bad("a sheet does not parse")),
                _ => {}
            }
            buf.clear();
        }
        sheets.push(grid);
    }
    Ok(sheets)
}

/// A cell that a spreadsheet would read as a formula gets a leading apostrophe in CSV.
fn csv_safe(v: &str) -> String {
    let risky = v.starts_with(['=', '+', '@', '\t', '\r']) || (v.starts_with('-') && exact_number(v).is_none());
    if risky {
        format!("'{v}")
    } else {
        v.to_string()
    }
}

/// RFC 4180 CSV, UTF-8 with a byte-order mark (so spreadsheet programs read it as UTF-8), CRLF.
pub fn write_csv(g: &Grid) -> Vec<u8> {
    let mut s = String::from("\u{feff}");
    for row in &g.cells {
        let fields: Vec<String> = row
            .iter()
            .map(|v| {
                let v = csv_safe(v);
                if v.contains([',', '"', '\n', '\r']) {
                    format!("\"{}\"", v.replace('"', "\"\""))
                } else {
                    v
                }
            })
            .collect();
        s.push_str(&fields.join(","));
        s.push_str("\r\n");
    }
    s.into_bytes()
}

/// Parse CSV written by [`write_csv`] (for verification).
pub fn read_csv(bytes: &[u8]) -> PwResult<Vec<Vec<String>>> {
    let s = std::str::from_utf8(bytes).map_err(|_| bad("it is not UTF-8"))?;
    let s = s.strip_prefix('\u{feff}').unwrap_or(s);
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if quoted {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    field.push('"');
                    chars.next();
                } else {
                    quoted = false;
                }
            } else {
                field.push(c);
            }
            continue;
        }
        match c {
            '"' => quoted = true,
            ',' => row.push(std::mem::take(&mut field)),
            '\r' if chars.peek() == Some(&'\n') => {
                chars.next();
                row.push(std::mem::take(&mut field));
                rows.push(std::mem::take(&mut row));
            }
            _ => field.push(c),
        }
    }
    if quoted || !field.is_empty() || !row.is_empty() {
        return Err(bad("the last row is incomplete"));
    }
    Ok(rows)
}

/// What a value reads back as from CSV.
fn csv_back(v: &str) -> String {
    csv_safe(v)
}

/// The ruled tables of the chosen pages.
pub fn find_grids(
    input: &Path,
    opts: &OpenOptions,
    pages: Option<&str>,
    cancelled: &dyn Fn() -> bool,
    progress: &dyn Fn(u64, u64),
) -> PwResult<(Vec<Grid>, u32)> {
    let pdfium = engine()?;
    let doc = load(&pdfium, input, opts)?;
    let n = doc.pages().len().max(0) as u32;
    let idx: Vec<u32> = match pages {
        Some(spec) => parse_page_ranges(spec, n)?,
        None => (0..n).collect(),
    };
    let mut grids = Vec::new();
    for (k, &p) in idx.iter().enumerate() {
        if cancelled() {
            return Err(PwError::cancelled());
        }
        let page = doc.pages().get(p as PdfPageIndex).map_err(|e| map_pdfium_error(&e, "page"))?;
        for (i, t) in crate::docx::page_tables(&page, &doc)?.iter().enumerate() {
            grids.push(grid_of(t, p + 1, i as u32 + 1));
        }
        progress(k as u64 + 1, idx.len() as u64);
    }
    Ok((grids, idx.len() as u32))
}

fn check_limits(grids: &[Grid]) -> PwResult<()> {
    for g in grids {
        if g.cells.len() > MAX_SHEET_ROWS || g.cells.iter().flatten().any(|v| v.chars().count() > MAX_CELL_CHARS) {
            return Err(PwError::invalid("tables.too_large", "A table is larger than a spreadsheet can hold."));
        }
    }
    Ok(())
}

fn none_found(pages: u32) -> PwError {
    PwError::new(
        ErrorCategory::UnsupportedOperation,
        "tables.none",
        format!(
            "No ruled tables were found on {} page{}. Tables without drawn lines are not recognized yet.",
            pages,
            if pages == 1 { "" } else { "s" }
        ),
    )
    .with_action("Try “Convert to Word” or “Export as text” for this document.")
}

fn report_of(grids: &[Grid], pages: u32, numbers: usize, texts: usize) -> TablesReport {
    TablesReport {
        pages,
        tables: grids.len(),
        found: grids.iter().map(|g| (g.page, g.index, g.cells.len(), g.widths.len())).collect(),
        number_cells: numbers,
        text_cells: texts,
    }
}

/// Save the tables as an Excel workbook.
pub fn export_xlsx(
    input: &Path,
    opts: &OpenOptions,
    pages: Option<&str>,
    output: &Path,
    policy: CollisionPolicy,
    cancelled: &dyn Fn() -> bool,
    progress: &dyn Fn(u64, u64),
) -> PwResult<(CommittedOutput, TablesReport)> {
    let (grids, n) = find_grids(input, opts, pages, cancelled, progress)?;
    if grids.is_empty() {
        return Err(none_found(n));
    }
    check_limits(&grids)?;
    let (bytes, numbers, texts) = write_xlsx(&grids)?;
    let want: Vec<Vec<Vec<String>>> = grids.iter().map(|g| g.cells.clone()).collect();
    let verify = |b: &[u8]| -> PwResult<()> {
        let back = read_xlsx(b)?;
        if back.len() != want.len() {
            return Err(bad("a sheet is missing"));
        }
        for (got, exp) in back.iter().zip(&want) {
            for (r, row) in exp.iter().enumerate() {
                for (c, v) in row.iter().enumerate() {
                    let g = got.get(r).and_then(|x| x.get(c)).map(String::as_str).unwrap_or("");
                    if g != v {
                        return Err(bad("a cell differs"));
                    }
                }
            }
        }
        Ok(())
    };
    verify(&bytes)?;
    let c = write_atomic_bytes(output, policy, &[input.to_path_buf()], &bytes, |staged| {
        let back = std::fs::read(staged).map_err(|e| PwError::from_io(&e, "output file"))?;
        if back != bytes {
            return Err(bad("the written file differs"));
        }
        verify(&back)
    })?;
    Ok((c, report_of(&grids, n, numbers, texts)))
}

/// Save each table as a CSV file in `output_dir` (`<name>_p<page>_t<n>.csv`).
#[allow(clippy::too_many_arguments)]
pub fn export_csv(
    input: &Path,
    opts: &OpenOptions,
    pages: Option<&str>,
    output_dir: &Path,
    policy: CollisionPolicy,
    cancelled: &dyn Fn() -> bool,
    progress: &dyn Fn(u64, u64),
) -> PwResult<(Vec<CommittedOutput>, TablesReport)> {
    if !output_dir.is_dir() {
        return Err(PwError::invalid("tables.folder", "Choose an existing folder for the CSV files."));
    }
    let (grids, n) = find_grids(input, opts, pages, cancelled, progress)?;
    if grids.is_empty() {
        return Err(none_found(n));
    }
    check_limits(&grids)?;
    let stem = input.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "tables".into());
    let mut outs = Vec::new();
    for g in &grids {
        if cancelled() {
            return Err(PwError::cancelled());
        }
        let bytes = write_csv(g);
        let want: Vec<Vec<String>> = g.cells.iter().map(|r| r.iter().map(|v| csv_back(v)).collect()).collect();
        let path: PathBuf = output_dir.join(format!("{stem}_p{}_t{}.csv", g.page, g.index));
        let c = write_atomic_bytes(&path, policy, &[input.to_path_buf()], &bytes, |staged| {
            let back = std::fs::read(staged).map_err(|e| PwError::from_io(&e, "output file"))?;
            if read_csv(&back)? != want {
                return Err(bad("a cell differs"));
            }
            Ok(())
        })?;
        outs.push(c);
    }
    let text_cells = grids.iter().flat_map(|g| g.cells.iter().flatten()).filter(|v| !v.is_empty()).count();
    Ok((outs, report_of(&grids, n, 0, text_cells)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_exact_plain_numbers_become_numbers() {
        for ok in ["0", "12", "12.4", "-3.25", "0.82", "123456789012345"] {
            assert_eq!(exact_number(ok), Some(ok), "{ok}");
        }
        for text in [
            "12.40",
            "1.0",
            "007",
            "00123",
            "1,234",
            "1 234",
            "12.",
            ".5",
            "1e5",
            "12/03/2026",
            "2026-03-12",
            "€5",
            "5%",
            " 5",
            "-0",
            "1234567890123456",
            "12.3456789012345678",
            "+5",
            "0x10",
            "NaN",
            "",
        ] {
            assert_eq!(exact_number(text), None, "{text}");
        }
    }

    #[test]
    fn column_names() {
        assert_eq!([0, 25, 26, 27, 701, 702].map(col_name), ["A", "Z", "AA", "AB", "ZZ", "AAA"]);
    }

    fn grid() -> Grid {
        Grid {
            page: 2,
            index: 1,
            cells: vec![
                vec!["Station".into(), "Code".into(), "Note".into()],
                vec!["North".into(), "007".into(), "a, \"quoted\"\nline".into()],
                vec!["=SUM(A1)".into(), "12.4".into(), "-x".into()],
            ],
            merges: vec![(0, 0, 1, 2)],
            widths: vec![100.0, 60.0, 80.0],
        }
    }

    #[test]
    fn workbooks_read_back_exactly_with_numbers_typed_safely() {
        let (bytes, numbers, texts) = write_xlsx(&[grid()]).expect("write");
        assert_eq!(numbers, 1, "only 12.4");
        assert_eq!(texts, 8);
        let back = read_xlsx(&bytes).expect("read");
        assert_eq!(back[0], grid().cells);
        let z = zip::ZipArchive::new(std::io::Cursor::new(&bytes)).expect("zip");
        assert!(z.file_names().flatten().any(|n| n == "xl/styles.xml"));
        let mut sheet = String::new();
        zip::ZipArchive::new(std::io::Cursor::new(&bytes))
            .expect("zip")
            .by_name("xl/worksheets/sheet1.xml")
            .expect("sheet")
            .read_to_string(&mut sheet)
            .expect("utf8");
        assert!(sheet.contains("<mergeCell ref=\"A1:B1\"/>"));
        assert!(
            sheet.contains("<c r=\"B2\" t=\"inlineStr\"><is><t xml:space=\"preserve\">007</t>"),
            "leading zero kept as text"
        );
        assert!(!sheet.contains("<f>"), "never a formula");
    }

    #[test]
    fn csv_quotes_and_defuses_formulas() {
        let bytes = write_csv(&grid());
        let s = String::from_utf8(bytes.clone()).expect("utf8");
        assert!(s.starts_with('\u{feff}'));
        assert!(s.contains("\"a, \"\"quoted\"\"\nline\""));
        assert!(s.contains("'=SUM(A1)"), "formula defused");
        assert!(s.contains("'-x"), "minus-led text defused");
        assert!(s.contains(",12.4,"), "numbers unchanged");
        let back = read_csv(&bytes).expect("read");
        assert_eq!(back[1][2], "a, \"quoted\"\nline");
        assert_eq!(back[2][0], "'=SUM(A1)");
        assert!(read_csv(b"a,\"b").is_err());
    }
}
