//! Part trace: the history of each physical part across several STDF files, as an Excel workbook.
//! One row per part (found by part text or another ID), one group of columns per file, in the order
//! and under the names the user chose. The files need not be a retest chain: a part shows in every
//! file that holds it and the other groups stay blank.
//!
//! Also finds which of several files hold the same parts, so they can be offered for tracing.

use crate::analysis::retest::placeholder;
use crate::data::matching::KeySpec;
use crate::data::{Dataset, Part};
use crate::xlsx::{self, column_letters, style, write_styled, Sheet, MAX_ROWS};
use anyhow::{bail, Result};
use indexmap::IndexMap;
use serde::Serialize;
use std::collections::HashSet;
use std::io::{self, Write};
use std::path::Path;

/// Columns Excel holds on a sheet.
const MAX_COLS: usize = 16_384;

/// The columns of each file.
const GROUP_TITLES: [&str; 5] = [
    "Part ID",
    "Hard Bin",
    "Site",
    "First Fail Test",
    "Last Fail Test",
];

/// One file of a trace.
pub struct Run<'a> {
    /// the name over its columns
    pub name: String,
    pub ds: &'a Dataset,
    pub key: KeySpec,
    /// the rows to trace, in test order
    pub rows: Vec<usize>,
    /// keep only the last test of a part this file holds more than once
    pub last_only: bool,
}

/// What a file adds to the trace.
#[derive(Debug, Clone, Serialize)]
pub struct RunStats {
    pub name: String,
    pub parts: usize,
    pub good: usize,
    /// parts without an ID, which cannot be traced and are left out
    pub no_id: usize,
    /// different IDs in the file
    pub distinct: usize,
    /// IDs already seen in an earlier file
    pub in_earlier: usize,
    /// tests of a part beyond its first in this file
    pub repeats: usize,
    /// of those, the earlier tests left out because the file keeps only each part's last test;
    /// `parts` and `good` count what is kept
    pub left_out: usize,
}

/// What an export wrote.
#[derive(Debug, Clone, Serialize)]
pub struct Summary {
    /// different parts traced
    pub parts: usize,
    /// parts found in more than one file
    pub in_several: usize,
    /// rows of the trace: one per part, and one more per extra test of it in one file
    pub rows: u64,
    pub sheets: Vec<String>,
    pub runs: Vec<RunStats>,
}

/// One part: its ID as written, and its (file, row) results in file order, then test order.
struct Entry {
    text: String,
    hits: Vec<(u32, u32)>,
}

fn collect(runs: &[Run]) -> (Vec<Entry>, Vec<RunStats>) {
    let mut map: IndexMap<String, Entry> = IndexMap::new();
    let mut stats = Vec::with_capacity(runs.len());
    for (k, run) in runs.iter().enumerate() {
        let mut st = RunStats {
            name: run.name.clone(),
            parts: run.rows.len(),
            good: 0,
            no_id: 0,
            distinct: 0,
            in_earlier: 0,
            repeats: 0,
            left_out: 0,
        };
        for &r in &run.rows {
            st.good += usize::from(run.ds.parts[r].is_pass());
            let Some(key) = run.key.key(run.ds, r) else {
                st.no_id += 1;
                continue;
            };
            // a placeholder is on many different parts, so those tests are not repeats of one
            let keep_last = run.last_only && placeholder(&key).is_none();
            let e = map.entry(key).or_insert_with_key(|key| Entry {
                text: match run.key {
                    KeySpec::PartTxt => run.ds.parts[r].part_txt.trim().to_string(),
                    _ => key.clone(),
                },
                hits: vec![],
            });
            match e.hits.last() {
                Some(&(last, earlier)) if last == k as u32 => {
                    st.repeats += 1;
                    if keep_last {
                        st.left_out += 1;
                        st.parts -= 1;
                        st.good -= usize::from(run.ds.parts[earlier as usize].is_pass());
                        e.hits.pop();
                    }
                }
                Some(_) => {
                    st.distinct += 1;
                    st.in_earlier += 1;
                }
                None => st.distinct += 1,
            }
            e.hits.push((k as u32, r as u32));
        }
        stats.push(st);
    }
    (map.into_values().collect(), stats)
}

/// Writes the trace of `runs` to the workbook `out`.
pub fn export(runs: &[Run], out: &Path, filter: &str) -> Result<Summary> {
    export_with(runs, out, filter, MAX_ROWS - 1)
}

/// As `export`, with at most `max_rows` part rows per sheet (below the two title rows). A longer
/// trace continues on the next sheet, named "Part trace (2)" and so on.
pub fn export_with(runs: &[Run], out: &Path, filter: &str, max_rows: u64) -> Result<Summary> {
    if runs.is_empty() {
        bail!("choose at least one file to trace");
    }
    if !out
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("xlsx"))
    {
        bail!("the report is an Excel workbook: save it as a .xlsx file");
    }
    let layout = Layout::new(runs)?;
    let dir = xlsx::scratch_dir()?;
    let result = write_book(runs, out, filter, max_rows.max(1), &layout, &dir);
    // the scratch directory is ours alone; the sheets in it are not needed once the book is written
    let _ = std::fs::remove_dir_all(&dir);
    result
}

fn write_book(
    runs: &[Run],
    out: &Path,
    filter: &str,
    max_rows: u64,
    layout: &Layout,
    dir: &Path,
) -> Result<Summary> {
    let (entries, stats) = collect(runs);
    let mut sheets: Vec<Sheet> = vec![];
    let mut rows = 0u64;
    let mut by_run: Vec<Vec<u32>> = vec![vec![]; runs.len()];
    let mut in_several = 0usize;
    for e in &entries {
        by_run.iter_mut().for_each(Vec::clear);
        for &(k, r) in &e.hits {
            by_run[k as usize].push(r);
        }
        in_several += usize::from(by_run.iter().filter(|h| !h.is_empty()).count() > 1);
        let depth = by_run.iter().map(Vec::len).max().unwrap_or(0);
        for i in 0..depth {
            if sheets.last().is_none_or(|s| s.rows >= max_rows) {
                finish_trace_sheet(sheets.last_mut(), layout)?;
                sheets.push(new_trace_sheet(dir, sheets.len(), runs, layout)?);
            }
            let sheet = sheets.last_mut().expect("a sheet was just added");
            sheet.rows += 1;
            write_part_row(
                &mut sheet.out,
                sheet.rows + 2,
                runs,
                layout,
                &e.text,
                &by_run,
                i,
            )?;
            rows += 1;
        }
    }
    if sheets.is_empty() {
        sheets.push(new_trace_sheet(dir, 0, runs, layout)?);
    }
    finish_trace_sheet(sheets.last_mut(), layout)?;
    let names = filter_names(&sheets, layout);
    let summary = Summary {
        parts: entries.len(),
        in_several,
        rows,
        sheets: sheets
            .iter()
            .map(|s| s.name.clone())
            .chain(["Summary".to_string()])
            .collect(),
        runs: stats,
    };
    sheets.push(summary_sheet(dir, sheets.len(), runs, filter, &summary)?);
    xlsx::write_zip(out, &sheets, &names)?;
    Ok(summary)
}

/// Where each file's columns are. Column A is the part's ID.
struct Layout {
    /// per file: the letters of Part ID, Hard Bin, Site, First and Last Fail Test
    groups: Vec<Vec<String>>,
    last: String,
}

impl Layout {
    fn new(runs: &[Run]) -> Result<Layout> {
        let mut next = 1;
        let mut groups = Vec::with_capacity(runs.len());
        for _ in runs {
            if next + GROUP_TITLES.len() > MAX_COLS {
                bail!("too many files for one sheet: Excel holds {MAX_COLS} columns");
            }
            groups.push(
                (next..next + GROUP_TITLES.len())
                    .map(column_letters)
                    .collect(),
            );
            next += GROUP_TITLES.len();
        }
        Ok(Layout {
            groups,
            last: column_letters(next - 1),
        })
    }
}

fn new_trace_sheet(dir: &Path, id: usize, runs: &[Run], layout: &Layout) -> Result<Sheet> {
    let mut cols = String::from(r#"<col min="1" max="1" width="24" customWidth="1"/>"#);
    let mut at = 2;
    for _ in runs {
        for w in [8, 8, 5, 34, 34] {
            cols.push_str(&format!(
                r#"<col min="{at}" max="{at}" width="{w}" customWidth="1"/>"#
            ));
            at += 1;
        }
    }
    let head = format!(
        r#"<sheetViews><sheetView workbookViewId="0"><pane xSplit="1" ySplit="2" topLeftCell="B3" activePane="bottomRight" state="frozen"/></sheetView></sheetViews><sheetFormatPr defaultRowHeight="15"/><cols>{cols}</cols>"#
    );
    let name = if id == 0 {
        "Part trace".to_string()
    } else {
        format!("Part trace ({})", id + 1)
    };
    let mut sheet = xlsx::open_sheet(dir, id, name, &head)?;
    let out = &mut sheet.out;
    // the file names, each over its columns
    out.write_all(b"<row r=\"1\">")?;
    write_styled(out, "A", 1, "", style::LABEL)?;
    for (k, (run, letters)) in runs.iter().zip(&layout.groups).enumerate() {
        let colour = style::GROUP + k as u32 % style::GROUPS;
        write_styled(out, &letters[0], 1, &run.name, colour)?;
        for letter in &letters[1..] {
            write!(out, "<c r=\"{letter}1\" s=\"{colour}\"/>")?;
        }
    }
    out.write_all(b"</row><row r=\"2\">")?;
    let key_title = match runs[0].key {
        KeySpec::PartTxt => "Part_Text".to_string(),
        _ => runs[0].key.describe(runs[0].ds),
    };
    write_styled(out, "A", 2, &key_title, style::TITLE)?;
    for letters in &layout.groups {
        for (title, letter) in GROUP_TITLES.iter().zip(letters) {
            write_styled(out, letter, 2, title, style::TITLE)?;
        }
    }
    out.write_all(b"</row>")?;
    Ok(sheet)
}

/// A number stays a number in Excel only when writing it back gives the same text ("007" stays text).
fn write_id(out: &mut impl Write, letter: &str, row: u64, id: &str) -> io::Result<()> {
    match id.parse::<u32>() {
        Ok(n) if n.to_string() == id => write_num(out, letter, row, u64::from(n), 0),
        _ => write_styled(out, letter, row, id, 0),
    }
}

fn write_num(out: &mut impl Write, letter: &str, row: u64, v: u64, style: u32) -> io::Result<()> {
    if style == 0 {
        write!(out, "<c r=\"{letter}{row}\"><v>{v}</v></c>")
    } else {
        write!(out, "<c r=\"{letter}{row}\" s=\"{style}\"><v>{v}</v></c>")
    }
}

/// A test as the report names it.
fn test_label(ds: &Dataset, t: u32) -> String {
    let t = &ds.tests[t as usize];
    format!("{} {}", t.num, t.name)
}

/// One row: the part's `i`th test in each file that has one. The part's ID is text, so a code such
/// as 12E4 is never read as a number.
fn write_part_row(
    out: &mut impl Write,
    row: u64,
    runs: &[Run],
    layout: &Layout,
    id: &str,
    by_run: &[Vec<u32>],
    i: usize,
) -> io::Result<()> {
    write!(out, "<row r=\"{row}\">")?;
    write_styled(out, "A", row, id, style::LABEL)?;
    for (k, run) in runs.iter().enumerate() {
        let Some(&r) = by_run[k].get(i) else { continue };
        let letters = &layout.groups[k];
        let p: &Part = &run.ds.parts[r as usize];
        write_id(out, &letters[0], row, &p.part_id)?;
        if p.partial {
            write_styled(out, &letters[1], row, "cut off", 0)?;
        } else {
            let bin_style = if p.passed == Some(false) {
                style::FAIL
            } else {
                0
            };
            write_num(out, &letters[1], row, u64::from(p.hbin), bin_style)?;
        }
        write_num(out, &letters[2], row, u64::from(p.site), 0)?;
        if let (Some(&first), Some(&last)) = (p.fails.first(), p.fails.last()) {
            write_styled(out, &letters[3], row, &test_label(run.ds, first), 0)?;
            write_styled(out, &letters[4], row, &test_label(run.ds, last), 0)?;
        }
    }
    out.write_all(b"</row>")
}

fn filter_ref(sheet: &Sheet, layout: &Layout) -> String {
    format!("A2:{}{}", layout.last, sheet.rows + 2)
}

/// The filter buttons on the title row, and the file names merged over their columns.
fn finish_trace_sheet(sheet: Option<&mut Sheet>, layout: &Layout) -> Result<()> {
    let Some(sheet) = sheet else { return Ok(()) };
    let filter = filter_ref(sheet, layout);
    write!(
        sheet.out,
        r#"</sheetData><autoFilter ref="{filter}"/><mergeCells count="{}">"#,
        layout.groups.len()
    )?;
    for letters in &layout.groups {
        let (a, b) = (&letters[0], &letters[letters.len() - 1]);
        write!(sheet.out, r#"<mergeCell ref="{a}1:{b}1"/>"#)?;
    }
    sheet.out.write_all(b"</mergeCells></worksheet>")?;
    sheet.out.flush()?;
    Ok(())
}

/// Excel's own name for each sheet's filter range.
fn filter_names(sheets: &[Sheet], layout: &Layout) -> String {
    let mut s = String::from("<definedNames>");
    for (i, sheet) in sheets.iter().enumerate() {
        let (from, to) = filter_ref(sheet, layout)
            .split_once(':')
            .map(|(a, b)| (absolute(a), absolute(b)))
            .expect("a range has two ends");
        s.push_str(&format!(
            r#"<definedName name="_xlnm._FilterDatabase" localSheetId="{i}" hidden="1">'{}'!{from}:{to}</definedName>"#,
            sheet.name
        ));
    }
    s.push_str("</definedNames>");
    s
}

/// A2 as $A$2.
fn absolute(cell: &str) -> String {
    let at = cell
        .find(|c: char| c.is_ascii_digit())
        .unwrap_or(cell.len());
    format!("${}${}", &cell[..at], &cell[at..])
}

/// The files of the trace and what each adds, so the workbook says where its columns came from.
fn summary_sheet(dir: &Path, id: usize, runs: &[Run], filter: &str, s: &Summary) -> Result<Sheet> {
    let head = concat!(
        r#"<sheetFormatPr defaultRowHeight="15"/><cols><col min="1" max="1" width="30" customWidth="1"/><col min="2" max="2" width="30" customWidth="1"/>"#,
        r#"<col min="3" max="7" width="16" customWidth="1"/><col min="8" max="8" width="20" customWidth="1"/><col min="9" max="15" width="11" customWidth="1"/></cols>"#
    );
    let mut sheet = xlsx::open_sheet(dir, id, "Summary".to_string(), head)?;
    let out = &mut sheet.out;
    let filter = if filter.trim().is_empty() {
        "none"
    } else {
        filter.trim()
    };
    let lines = [
        ("Parts matched by", runs[0].key.describe(runs[0].ds)),
        ("Part filter", filter.to_string()),
    ];
    let mut row = 1u64;
    for (label, value) in &lines {
        write!(out, "<row r=\"{row}\">")?;
        write_styled(out, "A", row, label, style::BOLD)?;
        write_styled(out, "B", row, value, 0)?;
        out.write_all(b"</row>")?;
        row += 1;
    }
    for (label, value) in [
        ("Parts traced", s.parts as u64),
        ("Parts in more than one file", s.in_several as u64),
        ("Rows in the trace", s.rows),
    ] {
        write!(out, "<row r=\"{row}\">")?;
        write_styled(out, "A", row, label, style::BOLD)?;
        write_num(out, "B", row, value, 0)?;
        out.write_all(b"</row>")?;
        row += 1;
    }
    row += 1;
    let titles = [
        "Name",
        "File",
        "Lot",
        "Sublot",
        "Tester",
        "Tester type",
        "Program",
        "Start (UTC)",
        "Parts",
        "Good",
        "No ID (left out)",
        "Different IDs",
        "In an earlier file",
        "Tested again in file",
        "Earlier tests left out",
    ];
    write!(out, "<row r=\"{row}\">")?;
    for (i, title) in titles.iter().enumerate() {
        write_styled(out, &column_letters(i), row, title, style::TITLE)?;
    }
    out.write_all(b"</row>")?;
    for (run, st) in runs.iter().zip(&s.runs) {
        row += 1;
        write!(out, "<row r=\"{row}\">")?;
        let source = run.ds.sources.first().cloned().unwrap_or_default();
        let file = run
            .ds
            .sources
            .iter()
            .map(|f| {
                Path::new(&f.path)
                    .file_name()
                    .map_or(f.path.clone(), |n| n.to_string_lossy().to_string())
            })
            .collect::<Vec<_>>()
            .join(", ");
        let start = chrono::DateTime::from_timestamp(i64::from(source.start_t), 0)
            .filter(|_| source.start_t != 0)
            .map(|t| t.format("%Y-%m-%d %H:%M:%S").to_string())
            .unwrap_or_default();
        let text = [
            &st.name,
            &file,
            &source.lot_id,
            &source.sublot_id,
            &source.node_nam,
            &source.tstr_typ,
            &source.job_nam,
            &start,
        ];
        for (i, value) in text.iter().enumerate() {
            write_styled(out, &column_letters(i), row, value, 0)?;
        }
        let nums = [
            st.parts,
            st.good,
            st.no_id,
            st.distinct,
            st.in_earlier,
            st.repeats,
            st.left_out,
        ];
        for (i, value) in nums.iter().enumerate() {
            write_num(out, &column_letters(text.len() + i), row, *value as u64, 0)?;
        }
        out.write_all(b"</row>")?;
    }
    sheet.rows = row;
    out.write_all(b"</sheetData></worksheet>")?;
    out.flush()?;
    Ok(sheet)
}

/// One file to look through for shared parts.
pub struct Candidate<'a> {
    pub ds: &'a Dataset,
    pub key: KeySpec,
    pub rows: Vec<usize>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FileIds {
    pub parts: usize,
    /// parts without an ID
    pub no_id: usize,
    /// parts whose ID is a placeholder (all zeros, one character repeated…), which match nothing
    pub placeholder: usize,
    pub distinct: usize,
    /// its IDs that another file also holds
    pub shared: usize,
    /// tests of a part beyond its first in this file
    pub repeats: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct Pair {
    pub a: usize,
    pub b: usize,
    pub shared: usize,
}

/// Which files hold the same parts. Indexes refer to the candidates given.
#[derive(Debug, Clone, Serialize)]
pub struct Detection {
    pub files: Vec<FileIds>,
    pub pairs: Vec<Pair>,
    /// files linked by shared parts, directly or through another file; two or more in each
    pub groups: Vec<Vec<usize>>,
    /// different IDs found in more than one file
    pub in_several: usize,
}

/// Which files keep only each part's last test: `all`, or 1-based file numbers such as `1,3`.
pub fn parse_last_only(spec: &str, files: usize) -> Result<Vec<bool>> {
    let spec = spec.trim();
    if spec.is_empty() {
        return Ok(vec![false; files]);
    }
    if spec.eq_ignore_ascii_case("all") {
        return Ok(vec![true; files]);
    }
    let mut on = vec![false; files];
    for n in spec.split(',').map(str::trim) {
        match n.parse::<usize>() {
            Ok(i) if (1..=files).contains(&i) => on[i - 1] = true,
            _ => bail!("--last-only takes all or file numbers from 1 to {files}, not '{n}'"),
        }
    }
    Ok(on)
}

pub fn detect(files: &[Candidate]) -> Detection {
    let mut stats = Vec::with_capacity(files.len());
    let mut sets: Vec<HashSet<String>> = Vec::with_capacity(files.len());
    for f in files {
        let mut st = FileIds {
            parts: f.rows.len(),
            no_id: 0,
            placeholder: 0,
            distinct: 0,
            shared: 0,
            repeats: 0,
        };
        let mut set = HashSet::new();
        for &r in &f.rows {
            match f.key.key(f.ds, r) {
                None => st.no_id += 1,
                Some(k) if placeholder(&k).is_some() => st.placeholder += 1,
                Some(k) => {
                    set.insert(k);
                }
            }
        }
        st.distinct = set.len();
        st.repeats = st.parts - st.no_id - st.placeholder - st.distinct;
        stats.push(st);
        sets.push(set);
    }
    let mut pairs = vec![];
    let mut root: Vec<usize> = (0..files.len()).collect();
    fn find(root: &mut [usize], i: usize) -> usize {
        if root[i] != i {
            root[i] = find(root, root[i]);
        }
        root[i]
    }
    for a in 0..files.len() {
        for b in a + 1..files.len() {
            let shared = sets[a].intersection(&sets[b]).count();
            if shared > 0 {
                pairs.push(Pair { a, b, shared });
                let (ra, rb) = (find(&mut root, a), find(&mut root, b));
                root[rb] = ra;
            }
        }
    }
    for (a, st) in stats.iter_mut().enumerate() {
        st.shared = sets[a]
            .iter()
            .filter(|k| (0..sets.len()).any(|b| b != a && sets[b].contains(*k)))
            .count();
    }
    let mut several: HashSet<&String> = HashSet::new();
    for a in 0..sets.len() {
        for b in a + 1..sets.len() {
            several.extend(sets[a].intersection(&sets[b]));
        }
    }
    let mut groups: IndexMap<usize, Vec<usize>> = IndexMap::new();
    for i in 0..files.len() {
        let r = find(&mut root, i);
        groups.entry(r).or_default().push(i);
    }
    Detection {
        in_several: several.len(),
        files: stats,
        pairs,
        groups: groups.into_values().filter(|g| g.len() > 1).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::{SourceInfo, TestInfo, TestKind};
    use std::io::Read;

    /// (part ID, part text, hard bin, passed, failed tests)
    type Row<'a> = (&'a str, &'a str, u16, bool, &'a [u32]);

    fn ds(rows: &[Row]) -> Dataset {
        Dataset {
            sources: vec![SourceInfo {
                path: "C:/data/lot.stdf".into(),
                lot_id: "LOT1".into(),
                ..Default::default()
            }],
            tests: [(100, "Main.cont"), (200, "Main.gnd"), (300, "Main.leak")]
                .map(|(num, name)| TestInfo {
                    key: format!("{num}:{name}"),
                    num,
                    name: name.into(),
                    kind: TestKind::Parametric,
                    units: String::new(),
                    lo: None,
                    hi: None,
                    lo_spec: None,
                    hi_spec: None,
                    res_scal: 0,
                    exec_count: 0,
                    fail_count: 0,
                })
                .to_vec(),
            parts: rows
                .iter()
                .map(|&(id, txt, bin, ok, fails)| Part {
                    file: 0,
                    part_id: id.into(),
                    head: 1,
                    site: 3,
                    x: -32768,
                    y: -32768,
                    hbin: bin,
                    sbin: bin,
                    passed: Some(ok),
                    part_flg: 0,
                    num_test: 0,
                    test_t: 0,
                    wafer: u32::MAX,
                    part_txt: txt.into(),
                    first_fail: fails.first().copied(),
                    alarms: vec![],
                    fails: fails.to_vec(),
                    partial: false,
                })
                .collect(),
            ..Default::default()
        }
    }

    fn run<'a>(name: &str, d: &'a Dataset) -> Run<'a> {
        Run {
            name: name.into(),
            ds: d,
            key: KeySpec::PartTxt,
            rows: (0..d.n_parts()).collect(),
            last_only: false,
        }
    }

    fn part(book: &Path, name: &str) -> String {
        let mut zip = zip::ZipArchive::new(std::fs::File::open(book).unwrap()).unwrap();
        let mut s = String::new();
        zip.by_name(name).unwrap().read_to_string(&mut s).unwrap();
        s
    }

    /// The text or number of one cell of a sheet's XML.
    fn cell(sheet: &str, at: &str) -> Option<String> {
        let open = sheet.find(&format!("<c r=\"{at}\""))?;
        let rest = &sheet[open..];
        let end = rest.find("</c>")?;
        let c = &rest[..end];
        let value = match c.find("<v>") {
            Some(i) => &c[i + 3..],
            None => c.split("preserve\">").nth(1)?,
        };
        Some(value.split('<').next().unwrap_or("").to_string())
    }

    #[test]
    fn a_row_per_part_with_the_files_it_was_in_and_blanks_for_the_rest() {
        let d = tempfile::tempdir().unwrap();
        let book = d.path().join("trace.xlsx");
        let one = ds(&[
            ("1", "AAA", 14, false, &[0, 2]),
            ("2", "12E4", 1, true, &[]),
            ("3", "", 1, true, &[]),
        ]);
        let two = ds(&[
            ("304", "AAA", 1, true, &[]),
            ("305", "CCC", 10, false, &[1]),
        ]);
        let s = export(
            &[run("V93K first", &one), run("V93K retest", &two)],
            &book,
            "",
        )
        .unwrap();
        assert_eq!((s.parts, s.in_several, s.rows), (3, 1, 3));
        assert_eq!(s.runs[0].no_id, 1);
        assert_eq!(s.runs[1].in_earlier, 1);
        let x = part(&book, "xl/worksheets/sheet1.xml");
        // the names over their columns, merged, and the titles under them
        assert!(x.contains("V93K first"));
        assert!(x.contains(r#"<mergeCell ref="B1:F1"/>"#));
        assert!(x.contains(r#"<mergeCell ref="G1:K1"/>"#));
        assert_eq!(cell(&x, "A2").unwrap(), "Part_Text");
        // first file's parts first, then what only a later file has
        assert_eq!(cell(&x, "A3").unwrap(), "AAA");
        assert_eq!(cell(&x, "A5").unwrap(), "CCC");
        // first and last fail, and the later file's result on the same row
        assert_eq!(cell(&x, "E3").unwrap(), "100 Main.cont");
        assert_eq!(cell(&x, "F3").unwrap(), "300 Main.leak");
        assert_eq!(cell(&x, "G3").unwrap(), "304");
        // blank where the part was not in that file; a failing bin is flagged, a passing one is not
        assert!(cell(&x, "G4").is_none());
        assert!(cell(&x, "B5").is_none());
        assert!(x.contains(&format!("<c r=\"C3\" s=\"{}\"><v>14</v>", style::FAIL)));
        assert!(x.contains("<c r=\"H3\"><v>1</v>"));
        // an ID that looks like a number stays text
        assert!(x.contains("<c r=\"A4\" t=\"inlineStr\""));
        assert!(part(&book, "xl/workbook.xml").contains("name=\"Summary\""));
        assert!(part(&book, "xl/worksheets/sheet2.xml").contains("LOT1"));
    }

    #[test]
    fn a_part_tested_twice_in_a_file_takes_a_second_row() {
        let d = tempfile::tempdir().unwrap();
        let book = d.path().join("trace.xlsx");
        let one = ds(&[("1", "AAA", 10, false, &[1]), ("9", "AAA", 1, true, &[])]);
        let two = ds(&[("5", "AAA", 1, true, &[])]);
        let s = export(&[run("a", &one), run("b", &two)], &book, "").unwrap();
        assert_eq!((s.parts, s.rows), (1, 2));
        assert_eq!(s.runs[0].repeats, 1);
        let x = part(&book, "xl/worksheets/sheet1.xml");
        assert_eq!(cell(&x, "B3").unwrap(), "1");
        assert_eq!(cell(&x, "B4").unwrap(), "9");
        assert_eq!(cell(&x, "A4").unwrap(), "AAA");
        assert_eq!(cell(&x, "G3").unwrap(), "5");
        assert!(cell(&x, "G4").is_none());
    }

    #[test]
    fn last_only_keeps_a_parts_last_test_in_that_file_alone() {
        let d = tempfile::tempdir().unwrap();
        let book = d.path().join("trace.xlsx");
        let eng = ds(&[
            ("1", "AAA", 10, false, &[1]),
            ("2", "BBB", 1, true, &[]),
            ("3", "AAA", 11, false, &[2]),
            ("4", "AAA", 1, true, &[]),
        ]);
        let again = ds(&[("5", "AAA", 1, true, &[]), ("6", "AAA", 1, true, &[])]);
        let mut first = run("eng", &eng);
        first.last_only = true;
        let s = export(&[first, run("other", &again)], &book, "").unwrap();
        // the first file keeps part 4 alone, on the part's first row; the other file keeps both
        assert_eq!((s.parts, s.rows), (2, 3));
        let st = &s.runs[0];
        assert_eq!((st.parts, st.good, st.repeats, st.left_out), (2, 2, 2, 2));
        assert_eq!(s.runs[1].left_out, 0);
        let x = part(&book, "xl/worksheets/sheet1.xml");
        assert_eq!(cell(&x, "B3").unwrap(), "4");
        assert_eq!(cell(&x, "C3").unwrap(), "1");
        assert_eq!(cell(&x, "G3").unwrap(), "5");
        assert_eq!(cell(&x, "A4").unwrap(), "AAA");
        assert!(cell(&x, "B4").is_none());
        assert_eq!(cell(&x, "G4").unwrap(), "6");
        assert_eq!(cell(&x, "A5").unwrap(), "BBB");
        assert!(part(&book, "xl/worksheets/sheet2.xml").contains("Earlier tests left out"));
    }

    #[test]
    fn last_only_never_merges_different_parts_with_a_placeholder_id() {
        let d = tempfile::tempdir().unwrap();
        let book = d.path().join("trace.xlsx");
        let one = ds(&[
            ("1", "00000000", 1, true, &[]),
            ("2", "00000000", 10, false, &[1]),
        ]);
        let mut r = run("a", &one);
        r.last_only = true;
        let s = export(&[r], &book, "").unwrap();
        assert_eq!((s.rows, s.runs[0].left_out, s.runs[0].repeats), (2, 0, 1));
    }

    #[test]
    fn last_only_files_are_all_or_numbers() {
        assert_eq!(parse_last_only("", 2).unwrap(), [false, false]);
        assert_eq!(parse_last_only("all", 2).unwrap(), [true, true]);
        assert_eq!(parse_last_only("2", 2).unwrap(), [false, true]);
        assert!(parse_last_only("3", 2).is_err());
        assert!(parse_last_only("x", 2).is_err());
    }

    #[test]
    fn a_long_trace_continues_on_the_next_sheet() {
        let d = tempfile::tempdir().unwrap();
        let book = d.path().join("trace.xlsx");
        let one = ds(&[
            ("1", "A", 1, true, &[]),
            ("2", "B", 1, true, &[]),
            ("3", "C", 1, true, &[]),
        ]);
        let s = export_with(&[run("a", &one)], &book, "", 2).unwrap();
        assert_eq!(s.sheets, ["Part trace", "Part trace (2)", "Summary"]);
        let x = part(&book, "xl/worksheets/sheet1.xml");
        assert!(x.contains(r#"<autoFilter ref="A2:F4"/>"#));
        let x2 = part(&book, "xl/worksheets/sheet2.xml");
        assert_eq!(cell(&x2, "A2").unwrap(), "Part_Text");
        assert_eq!(cell(&x2, "A3").unwrap(), "C");

        let wb = part(&book, "xl/workbook.xml");
        assert!(wb.contains(r#"localSheetId="1" hidden="1">'Part trace (2)'!$A$2:$F$3<"#));
    }

    #[test]
    fn the_report_must_be_an_xlsx_file() {
        let d = tempfile::tempdir().unwrap();
        let one = ds(&[("1", "A", 1, true, &[])]);
        let e = export(&[run("a", &one)], &d.path().join("trace.csv"), "");
        assert!(e.unwrap_err().to_string().contains(".xlsx"));
    }

    #[test]
    fn detection_groups_files_by_shared_parts_and_ignores_placeholders() {
        let cand = |d| Candidate {
            ds: d,
            key: KeySpec::PartTxt,
            rows: (0..d.n_parts()).collect(),
        };
        let dupes = ds(&[("1", "D", 1, true, &[]), ("2", "D", 1, true, &[])]);
        assert_eq!(detect(&[cand(&dupes)]).files[0].repeats, 1);
        let a = ds(&[
            ("1", "A", 1, true, &[]),
            ("2", "B", 1, true, &[]),
            ("3", "00000000", 1, true, &[]),
        ]);
        let b = ds(&[("1", "B", 1, true, &[]), ("2", "C", 1, true, &[])]);
        let c = ds(&[("1", "C", 1, true, &[]), ("2", "", 1, true, &[])]);
        // another lot: only a placeholder in common with the first file, which is no link
        let other = ds(&[("1", "X", 1, true, &[]), ("2", "00000000", 1, true, &[])]);
        let d = detect(&[cand(&a), cand(&b), cand(&c), cand(&other)]);
        assert_eq!(d.groups, vec![vec![0, 1, 2]]);
        assert_eq!(d.in_several, 2);
        assert_eq!(d.files[0].placeholder, 1);
        assert_eq!(d.files[1].shared, 2);
        assert_eq!(d.files[2].no_id, 1);
        assert_eq!(d.files[3].shared, 0);
        assert_eq!(d.pairs.len(), 2);
    }
}
