//! Table export (Excel and CSV) against the real engine: ruled tables found, cell values kept
//! exactly as text unless they are plain exact numbers, merged cells kept, files read back
//! independently here.

use pw_core::ops::{Operation, OperationRequest, OperationResult, TableFormat};
use pw_core::output::CollisionPolicy;
use pw_core::{ErrorCategory, PwResult};
use pw_pdf::execute;
use std::io::Read;
use std::path::{Path, PathBuf};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/pdf").join(name)
}

fn export(name: &str, format: TableFormat, pages: Option<&str>, output: &Path) -> PwResult<OperationResult> {
    let mut req = OperationRequest::new(Operation::ExportTables {
        input: fixture(name),
        pages: pages.map(str::to_string),
        format,
        output: output.into(),
    });
    req.collision = CollisionPolicy::Fail;
    execute(&req, None)
}

fn part(path: &Path, name: &str) -> String {
    let mut z = zip::ZipArchive::new(std::fs::File::open(path).expect("open")).expect("zip");
    let mut s = String::new();
    z.by_name(name).unwrap_or_else(|_| panic!("{name}")).read_to_string(&mut s).expect("utf-8");
    s
}

#[test]
fn workbook_has_one_sheet_per_table_with_values_kept() {
    let dir = tempfile::tempdir().expect("tmp");
    let out = dir.path().join("t.xlsx");
    let r = export("tables.pdf", TableFormat::Xlsx, None, &out).expect("export");
    assert_eq!(r.data["tables"], 2, "{}", r.data);
    assert_eq!(r.data["found"], serde_json::json!([[1, 1, 4, 4], [2, 1, 3, 2]]));
    let wb = part(&out, "xl/workbook.xml");
    assert!(wb.contains("name=\"Page 1 table 1\"") && wb.contains("name=\"Page 2 table 1\""));
    let s1 = part(&out, "xl/worksheets/sheet1.xml");
    let text =
        |addr: &str, v: &str| format!("<c r=\"{addr}\" t=\"inlineStr\"><is><t xml:space=\"preserve\">{v}</t></is></c>");
    // Header merged across A1:B1; values a spreadsheet could corrupt stay text.
    assert!(s1.contains("<mergeCell ref=\"A1:B1\"/>"), "{s1}");
    for (a, v) in [
        ("A1", "Account"),
        ("A2", "007"),
        ("A3", "123456789012345678"),
        ("A4", "00042"),
        ("C2", "2026-03-12"),
        ("C3", "12/03/2026"),
        ("D2", "1,234.50"),
        ("D4", "12.40"),
    ] {
        assert!(s1.contains(&text(a, v)), "{a} = {v} as text");
    }
    // A plain exact number is a number.
    assert!(s1.contains("<c r=\"D3\"><v>-4.5</v></c>"), "{s1}");
    assert_eq!(r.data["number_cells"], 1);
    // Page 2: North merged down two rows.
    let s2 = part(&out, "xl/worksheets/sheet2.xml");
    assert!(s2.contains("<mergeCell ref=\"A2:A3\"/>"), "{s2}");
    assert!(s2.contains(&text("B3", "N2")));
}

#[test]
fn csv_files_per_table_and_the_loss_is_said() {
    let dir = tempfile::tempdir().expect("tmp");
    let r = export("tables.pdf", TableFormat::Csv, None, dir.path()).expect("export");
    assert_eq!(r.outputs.len(), 2);
    assert!(r.warnings.iter().any(|w| w.contains("CSV keeps only the text")), "{:?}", r.warnings);
    let a = std::fs::read_to_string(dir.path().join("tables_p1_t1.csv")).expect("csv");
    assert!(a.starts_with('\u{feff}'));
    assert_eq!(a.lines().nth(1).expect("row 2"), "007,Savings,2026-03-12,\"1,234.50\"");
    assert_eq!(a.lines().nth(2).expect("row 3"), "123456789012345678,Current,12/03/2026,-4.5");
    assert!(a.contains("\r\n"));
    let b = std::fs::read_to_string(dir.path().join("tables_p2_t1.csv")).expect("csv");
    assert_eq!(b.trim_start_matches('\u{feff}'), "Region,Station\r\nNorth,N1\r\n,N2\r\n");
}

#[test]
fn ruled_tables_of_other_documents_and_clear_failures() {
    let dir = tempfile::tempdir().expect("tmp");
    // The article's results table (page 2 only).
    let out = dir.path().join("a.xlsx");
    let r = export("article.pdf", TableFormat::Xlsx, Some("2"), &out).expect("article");
    assert_eq!(r.data["found"], serde_json::json!([[2, 1, 4, 3]]));
    assert!(part(&out, "xl/worksheets/sheet1.xml").contains("<v>12.4</v>"));
    // No tables → a clear refusal, nothing written.
    let none = dir.path().join("none.xlsx");
    let e = export("five-pages.pdf", TableFormat::Xlsx, None, &none).expect_err("no tables");
    assert_eq!(e.code, "tables.none");
    assert!(e.message.contains("No tables were found on 5 pages"), "{}", e.message);
    assert!(!none.exists());
    // A drawn grid with no text in it is not a table.
    let e = export("blank-pages.pdf", TableFormat::Xlsx, Some("5"), &none).expect_err("empty grid");
    assert_eq!(e.code, "tables.none");
    // Wrong extension, missing folder, existing output, password.
    let e = export("tables.pdf", TableFormat::Xlsx, None, &dir.path().join("x.csv")).expect_err("ext");
    assert_eq!(e.code, "tables.extension");
    let e = export("tables.pdf", TableFormat::Csv, None, &dir.path().join("missing")).expect_err("folder");
    assert_eq!(e.category, ErrorCategory::InvalidInput);
    let e = export("article.pdf", TableFormat::Xlsx, Some("2"), &out).expect_err("exists");
    assert_eq!(e.category, ErrorCategory::OutputConflict);
    let e =
        export("encrypted-rc4-user-password.pdf", TableFormat::Xlsx, None, &dir.path().join("p.xlsx")).expect_err("pw");
    assert_eq!(e.category, ErrorCategory::PasswordRequired);
}

#[test]
fn borderless_tables_are_found_and_prose_columns_are_not() {
    let dir = tempfile::tempdir().expect("tmp");
    // Page 1: a table of aligned columns with no lines, and one with horizontal rules only.
    let r = export("borderless-tables.pdf", TableFormat::Csv, None, dir.path()).expect("export");
    assert_eq!(r.data["found"], serde_json::json!([[1, 1, 5, 4], [1, 2, 4, 3]]), "{}", r.data);
    let a = std::fs::read_to_string(dir.path().join("borderless-tables_p1_t1.csv")).expect("csv");
    assert_eq!(
        a.trim_start_matches('\u{feff}'),
        "Crop,Rows,Yield,Grade\r\nApples,120,0.50,A\r\nPears,80,0.75,B\r\nPlums and damsons,007,\"1,250.00\",A\r\nQuince,12,3.10,C\r\n"
    );
    let b = std::fs::read_to_string(dir.path().join("borderless-tables_p1_t2.csv")).expect("csv");
    assert_eq!(
        b.trim_start_matches('\u{feff}'),
        "Station,Rain mm,Days\r\nUpper farm,412,61\r\nLower farm,388,58\r\nRiver field,455,64\r\n"
    );
    // Values keep their text in the workbook too (007 and 1,250.00 stay text; 0.75 is a number).
    let out = dir.path().join("b.xlsx");
    export("borderless-tables.pdf", TableFormat::Xlsx, Some("1"), &out).expect("xlsx");
    let s1 = part(&out, "xl/worksheets/sheet1.xml");
    assert!(s1.contains(">007</t>") && s1.contains(">1,250.00</t>") && s1.contains("<v>0.75</v>"), "{s1}");
    assert!(s1.contains(">0.50</t>"), "trailing zero kept as text: {s1}");
    // Page 2 is two columns of prose: no table.
    let e =
        export("borderless-tables.pdf", TableFormat::Xlsx, Some("2"), &dir.path().join("p.xlsx")).expect_err("prose");
    assert_eq!(e.code, "tables.none");
    // The article's ruled table is found once, not again as a borderless one.
    let r = export("article.pdf", TableFormat::Xlsx, None, &dir.path().join("a.xlsx")).expect("article");
    assert_eq!(r.data["found"], serde_json::json!([[2, 1, 4, 3]]), "{}", r.data);
}

#[test]
fn libreoffice_opens_the_workbook_with_the_same_values() {
    let available =
        std::process::Command::new("soffice").arg("--version").output().map(|o| o.status.success()).unwrap_or(false);
    if !available {
        assert!(
            std::env::var("PW_SKIP_SOFFICE").as_deref() == Ok("1"),
            "LibreOffice (soffice) is required; set PW_SKIP_SOFFICE=1 only where it cannot be installed"
        );
        eprintln!("SKIPPED: LibreOffice workbook check (PW_SKIP_SOFFICE=1)");
        return;
    }
    let dir = tempfile::tempdir().expect("tmp");
    let out = dir.path().join("t.xlsx");
    export("tables.pdf", TableFormat::Xlsx, Some("1"), &out).expect("export");
    let profile = dir.path().join("lo-profile");
    let st = std::process::Command::new("soffice")
        .arg(format!("-env:UserInstallation=file://{}", profile.display()))
        .args(["--headless", "--norestore", "--convert-to", "csv", "--outdir"])
        .arg(dir.path().join("lo"))
        .arg(&out)
        .output()
        .expect("soffice");
    assert!(st.status.success(), "{}", String::from_utf8_lossy(&st.stderr));
    let csv = std::fs::read_to_string(dir.path().join("lo/t.csv")).unwrap_or_else(|e| {
        panic!(
            "LibreOffice wrote no CSV ({e}); is LibreOffice Calc installed? stdout: {} stderr: {}",
            String::from_utf8_lossy(&st.stdout),
            String::from_utf8_lossy(&st.stderr)
        )
    });
    let rows: Vec<&str> = csv.lines().collect();
    assert_eq!(rows[0], "Account,,Opened,Balance");
    assert_eq!(rows[1], "007,Savings,2026-03-12,\"1,234.50\"", "leading zeros, dates and grouped amounts unchanged");
    assert_eq!(rows[2], "123456789012345678,Current,12/03/2026,-4.5", "long identifiers unchanged");
    assert_eq!(rows[3], "00042,Bonus,2025-11-30,12.40", "trailing zeros unchanged");
}

#[test]
fn shading_inside_cells_does_not_split_a_table() {
    // Word shades a header cell and again the paragraph within its padding (shaded-table.pdf).
    let dir = tempfile::tempdir().expect("tmp");
    let out = dir.path().join("t.xlsx");
    let r = export("shaded-table.pdf", TableFormat::Xlsx, None, &out).expect("export");
    assert_eq!(r.data["found"], serde_json::json!([[1, 1, 3, 3]]), "{}", r.data);
    let s1 = part(&out, "xl/worksheets/sheet1.xml");
    for v in ["Name", "Count", "Share", "Apples", "40%"] {
        assert!(s1.contains(&format!(">{v}</t>")), "{v} in {s1}");
    }
}
