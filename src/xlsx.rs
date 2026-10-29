//! Export of STDF records to an Excel workbook (.xlsx). The writer needs no spreadsheet crate: one
//! sheet per record kind, one column per field, written as the SpreadsheetML parts of a zip file.
//! Each sheet streams into a temporary file as its records are read, so memory stays flat for any
//! file size; the finished sheets are then copied into the workbook.

use crate::stdf::{record_name, StdfReader};
use anyhow::{Context, Result};
use indexmap::IndexMap;
use serde_json::{Map, Value};
use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};

/// Data rows per sheet: Excel holds 1,048,576 rows, one of them the header.
pub const MAX_ROWS: u64 = 1_048_575;

/// Longest text Excel keeps in one cell.
const MAX_TEXT: usize = 32_767;

/// What an export wrote.
pub struct Summary {
    /// records read from the file
    pub records: u64,
    /// (sheet name, data rows), in workbook order
    pub sheets: Vec<(String, u64)>,
}

/// One worksheet, streamed to a temporary file.
struct Sheet {
    name: String,
    rows: u64,
    path: PathBuf,
    out: BufWriter<File>,
}

/// The sheets of one record kind, and its columns: the fields of the kind's first record, with
/// their column letters worked out once.
struct Kind {
    cols: Vec<String>,
    letters: Vec<String>,
    sheets: Vec<Sheet>,
}

/// Writes every record of `input` to the workbook `out`: one sheet per record kind, named after it.
pub fn export(input: &Path, out: &Path) -> Result<Summary> {
    export_with(input, out, MAX_ROWS)
}

/// As `export`, with at most `max_rows` data rows per sheet. A kind with more records continues on
/// the next sheet, named "PTR (2)" and so on.
pub fn export_with(input: &Path, out: &Path, max_rows: u64) -> Result<Summary> {
    let dir = scratch_dir()?;
    let result = write_book(input, out, &dir, max_rows.max(1));
    // the scratch directory is ours alone; the sheets in it are not needed once the book is written
    let _ = std::fs::remove_dir_all(&dir);
    result
}

fn scratch_dir() -> Result<PathBuf> {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let dir = std::env::temp_dir().join(format!("stdf-xlsx-{}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    Ok(dir)
}

fn write_book(input: &Path, out: &Path, dir: &Path, max_rows: u64) -> Result<Summary> {
    let mut rdr = StdfReader::open(input)?;
    let mut kinds: IndexMap<&'static str, Kind> = IndexMap::new();
    let mut records = 0u64;
    let mut seq = 0usize;
    while let Some((h, rec)) = rdr.next_record()? {
        records += 1;
        let name = record_name(h.typ, h.sub);
        let Value::Object(fields) = serde_json::to_value(&rec)? else {
            continue;
        };
        let kind = kinds.entry(name).or_insert_with(|| {
            let cols: Vec<String> = fields.keys().filter(|k| *k != "rec").cloned().collect();
            let letters = (0..cols.len()).map(column_letters).collect();
            Kind {
                cols,
                letters,
                sheets: vec![],
            }
        });
        if kind.sheets.last().is_none_or(|s| s.rows >= max_rows) {
            let n = kind.sheets.len();
            let sheet_name = if n == 0 {
                name.to_string()
            } else {
                format!("{name} ({})", n + 1)
            };
            kind.sheets
                .push(new_sheet(dir, seq, sheet_name, &kind.cols, &kind.letters)?);
            seq += 1;
        }
        let sheet = kind.sheets.last_mut().expect("a sheet was just added");
        sheet.rows += 1;
        write_row(
            &mut sheet.out,
            sheet.rows + 1,
            &kind.cols,
            &kind.letters,
            &fields,
        )?;
    }
    if kinds.is_empty() {
        // a workbook needs a sheet, even for a file without records
        let s = new_sheet(dir, seq, "Empty".into(), &[], &[])?;
        kinds.insert(
            "Empty",
            Kind {
                cols: vec![],
                letters: vec![],
                sheets: vec![s],
            },
        );
    }
    let mut sheets: Vec<Sheet> = kinds.into_values().flat_map(|k| k.sheets).collect();
    for s in sheets.iter_mut() {
        s.out.write_all(b"</sheetData></worksheet>")?;
        s.out.flush()?;
    }
    write_zip(out, &sheets)?;
    Ok(Summary {
        records,
        sheets: sheets.into_iter().map(|s| (s.name, s.rows)).collect(),
    })
}

/// A new sheet with its header row, frozen above the data.
fn new_sheet(
    dir: &Path,
    id: usize,
    name: String,
    cols: &[String],
    letters: &[String],
) -> Result<Sheet> {
    let path = dir.join(format!("sheet{id}.xml"));
    let file = File::create(&path).with_context(|| format!("creating {}", path.display()))?;
    let mut out = BufWriter::new(file);
    out.write_all(
        concat!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#,
            r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">"#,
            r#"<sheetViews><sheetView workbookViewId="0"><pane ySplit="1" topLeftCell="A2" activePane="bottomLeft" state="frozen"/></sheetView></sheetViews>"#,
            "<sheetData>"
        )
        .as_bytes(),
    )?;
    if !cols.is_empty() {
        out.write_all(b"<row r=\"1\">")?;
        for (col, letter) in cols.iter().zip(letters) {
            write_text(&mut out, letter, 1, col, true)?;
        }
        out.write_all(b"</row>")?;
    }
    Ok(Sheet {
        name,
        rows: 0,
        path,
        out,
    })
}

/// One record as a row. A field the record leaves out, or sets to null, is an empty cell.
fn write_row(
    out: &mut impl Write,
    row: u64,
    cols: &[String],
    letters: &[String],
    fields: &Map<String, Value>,
) -> io::Result<()> {
    write!(out, "<row r=\"{row}\">")?;
    for (col, letter) in cols.iter().zip(letters) {
        match fields.get(col) {
            None | Some(Value::Null) => {}
            Some(Value::Bool(b)) => write!(
                out,
                "<c r=\"{letter}{row}\" t=\"b\"><v>{}</v></c>",
                u8::from(*b)
            )?,
            Some(Value::Number(n)) => write!(out, "<c r=\"{letter}{row}\"><v>{n}</v></c>")?,
            Some(Value::String(s)) => write_text(out, letter, row, s, false)?,
            Some(other) => write_text(out, letter, row, &other.to_string(), false)?,
        }
    }
    out.write_all(b"</row>")
}

/// A text cell: characters XML cannot carry are dropped, and text is cut to what Excel keeps.
fn write_text(
    out: &mut impl Write,
    letter: &str,
    row: u64,
    text: &str,
    bold: bool,
) -> io::Result<()> {
    let style = if bold { " s=\"1\"" } else { "" };
    write!(
        out,
        "<c r=\"{letter}{row}\" t=\"inlineStr\"{style}><is><t xml:space=\"preserve\">"
    )?;
    let mut buf = [0u8; 4];
    for c in text.chars().filter(|c| xml_char(*c)).take(MAX_TEXT) {
        match c {
            '&' => out.write_all(b"&amp;")?,
            '<' => out.write_all(b"&lt;")?,
            '>' => out.write_all(b"&gt;")?,
            c => out.write_all(c.encode_utf8(&mut buf).as_bytes())?,
        }
    }
    out.write_all(b"</t></is></c>")
}

/// The characters XML 1.0 allows.
fn xml_char(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\r' | '\u{20}'..='\u{D7FF}' | '\u{E000}'..='\u{FFFD}' | '\u{10000}'..='\u{10FFFF}')
}

/// Column letters of a 0-based column: 0 is A, 25 is Z, 26 is AA.
fn column_letters(mut i: usize) -> String {
    let mut s = Vec::new();
    loop {
        s.push(b'A' + (i % 26) as u8);
        if i < 26 {
            break;
        }
        i = i / 26 - 1;
    }
    s.reverse();
    String::from_utf8(s).expect("ASCII letters")
}

/// The workbook package: the parts every .xlsx has, then one part per sheet. Sheets are deflated at
/// level 1: the sheets are large and repetitive, and a faster level keeps the export quick.
fn write_zip(out: &Path, sheets: &[Sheet]) -> Result<()> {
    let file = File::create(out).with_context(|| format!("creating {}", out.display()))?;
    let mut zip = zip::ZipWriter::new(BufWriter::new(file));
    let opts = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .compression_level(Some(1))
        .large_file(true);
    let mut overrides = String::new();
    let mut rels = String::new();
    let mut book = String::new();
    for (i, s) in sheets.iter().enumerate() {
        let n = i + 1;
        overrides.push_str(&format!(
            r#"<Override PartName="/xl/worksheets/sheet{n}.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>"#
        ));
        rels.push_str(&format!(
            r#"<Relationship Id="rId{n}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet{n}.xml"/>"#
        ));
        book.push_str(&format!(
            r#"<sheet name="{}" sheetId="{n}" r:id="rId{n}"/>"#,
            xml_attr(&s.name)
        ));
    }
    let styles_rel = sheets.len() + 1;
    let parts = [
        (
            "[Content_Types].xml",
            format!(
                r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml"/>{overrides}</Types>"#
            ),
        ),
        (
            "_rels/.rels",
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#
                .to_string(),
        ),
        (
            "xl/workbook.xml",
            format!(
                r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets>{book}</sheets></workbook>"#
            ),
        ),
        (
            "xl/_rels/workbook.xml.rels",
            format!(
                r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{rels}<Relationship Id="rId{styles_rel}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/></Relationships>"#
            ),
        ),
        (
            "xl/styles.xml",
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><fonts count="2"><font><sz val="11"/><name val="Calibri"/></font><font><b/><sz val="11"/><name val="Calibri"/></font></fonts><fills count="2"><fill><patternFill patternType="none"/></fill><fill><patternFill patternType="gray125"/></fill></fills><borders count="1"><border><left/><right/><top/><bottom/><diagonal/></border></borders><cellStyleXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0"/></cellStyleXfs><cellXfs count="2"><xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/><xf numFmtId="0" fontId="1" fillId="0" borderId="0" xfId="0" applyFont="1"/></cellXfs><cellStyles count="1"><cellStyle name="Normal" xfId="0" builtinId="0"/></cellStyles></styleSheet>"#
                .to_string(),
        ),
    ];
    for (name, body) in parts {
        zip.start_file(name, opts)?;
        zip.write_all(body.as_bytes())?;
    }
    for (i, s) in sheets.iter().enumerate() {
        zip.start_file(format!("xl/worksheets/sheet{}.xml", i + 1), opts)?;
        let mut f = File::open(&s.path).with_context(|| format!("reading {}", s.path.display()))?;
        io::copy(&mut f, &mut zip)?;
    }
    zip.finish()?;
    Ok(())
}

/// A sheet name or attribute value, with the characters XML reserves escaped.
fn xml_attr(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('"', "&quot;")
}
