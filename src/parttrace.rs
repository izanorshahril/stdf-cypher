//! Part trace: the history of each physical part across several STDF files, as an Excel workbook.
//! One row per part, one group of columns per file, in the order and under the names the user
//! chose. The files need not be a retest chain: a part shows in every file that holds it and the
//! other groups stay blank.
//!
//! A part can be found by several IDs at once (PRR part text, a WXY test, wafer, X and Y tests):
//! two tests are the same part when they share any of them, so one ID fills the gaps of another
//! (an engineering file with no part text, a part that failed before its WXY test). Two tests whose
//! IDs disagree (the same part text, another WXY) are never merged: both rows are kept and marked.
//!
//! Also finds which of several files hold the same parts, so they can be offered for tracing.

use crate::analysis::retest::placeholder;
use crate::data::matching::{norm, num_key, wxy_text, KeySpec};
use crate::data::{Dataset, Part};
use crate::xlsx::{self, column_letters, style, write_styled, Sheet, MAX_ROWS};
use anyhow::{bail, Result};
use indexmap::IndexMap;
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::io::{self, Write};
use std::path::Path;

/// Columns Excel holds on a sheet.
const MAX_COLS: usize = 16_384;

/// Largest whole number a PTR result holds exactly: STDF stores results as 32-bit floats, so a code
/// above this is rounded and different parts can end up with the same one.
const MAX_EXACT: f64 = 16_777_216.0;

/// A file's key from a key spec, or None when the file lacks the tests: its parts then have no
/// such ID, rather than stopping the trace of the other files.
pub fn key_for(spec: &str, ds: &Dataset) -> Option<KeySpec> {
    KeySpec::parse(spec, ds).ok()
}

/// An ID as a column title.
pub fn id_title(k: &KeySpec, ds: &Dataset) -> String {
    let num = |t: usize| ds.tests[t].num.to_string();
    match k {
        KeySpec::PartTxt => "Part_Text".into(),
        KeySpec::PartId => "Part_ID".into(),
        KeySpec::Xy => "X/Y".into(),
        KeySpec::WaferXy => "Wafer X/Y".into(),
        KeySpec::Tests(t) if t.len() == 1 => format!("PTR {}", num(t[0])),
        KeySpec::Tests(t) => format!(
            "PTR {}",
            t.iter().map(|&t| num(t)).collect::<Vec<_>>().join("+")
        ),
        KeySpec::WxyTests { .. } => "W,X,Y".into(),
    }
}

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
    /// one key per ID of the trace, in the same order in every file; None where the file lacks it
    pub keys: Vec<Option<KeySpec>>,
    /// the rows to trace, in test order
    pub rows: Vec<usize>,
    /// keep only the last test of a part this file holds more than once
    pub last_only: bool,
}

/// Leaves out the IDs no file has (a test no file logs), so the report has no empty ID column.
/// Returns the names of those left out, as `specs` names them.
pub fn drop_unused_ids(runs: &mut [Run], specs: &[String]) -> Vec<String> {
    let n = runs.iter().map(|r| r.keys.len()).max().unwrap_or(0);
    let used: Vec<bool> = (0..n)
        .map(|i| {
            runs.iter()
                .any(|r| r.keys.get(i).is_some_and(Option::is_some))
        })
        .collect();
    for r in runs.iter_mut() {
        let mut i = 0;
        r.keys.retain(|_| {
            i += 1;
            used[i - 1]
        });
    }
    (0..n)
        .filter(|&i| !used[i])
        .map(|i| specs.get(i).cloned().unwrap_or_default())
        .collect()
}

/// The ID titles of a trace, from the first file that has each ID.
pub fn id_titles(runs: &[Run]) -> Vec<String> {
    let n = runs.iter().map(|r| r.keys.len()).max().unwrap_or(0);
    (0..n)
        .map(|i| {
            runs.iter()
                .find_map(|r| {
                    r.keys
                        .get(i)
                        .and_then(Option::as_ref)
                        .map(|k| id_title(k, r.ds))
                })
                .unwrap_or_else(|| format!("ID {}", i + 1))
        })
        .collect()
}

/// What a file adds to the trace.
#[derive(Debug, Clone, Default, Serialize)]
pub struct RunStats {
    pub name: String,
    pub parts: usize,
    pub good: usize,
    /// parts without any ID, which cannot be traced and are left out
    pub no_id: usize,
    /// parts whose only IDs are placeholders (all zeros, one character repeated…): each keeps a
    /// row of its own and is never matched
    pub placeholder: usize,
    /// different parts in the file
    pub distinct: usize,
    /// parts already seen in an earlier file
    pub in_earlier: usize,
    /// parts also in another file of the trace
    pub shared: usize,
    /// tests of a part beyond its first in this file
    pub repeats: usize,
    /// of those, the earlier tests left out because the file keeps only each part's last test;
    /// `parts` and `good` count what is kept
    pub left_out: usize,
    /// parts with an ID from a result too large to be exact, so it may not be unique
    pub rounded: usize,
    /// per ID: whether the file has it (logs its tests), and on how many parts
    pub has: Vec<bool>,
    pub with: Vec<usize>,
}

/// Two tests that share one ID but disagree on another, so they were not merged.
#[derive(Debug, Clone, Serialize)]
pub struct Conflict {
    /// the ID they share, and its value
    pub by: String,
    pub value: String,
    /// the ID that differs, and each test's value
    pub field: String,
    pub mine: String,
    pub theirs: String,
    /// (file, row) of the test that was kept apart, and of a test of the part it matched
    pub at: (usize, usize),
    pub other: (usize, usize),
}

impl Conflict {
    pub fn text(&self) -> String {
        format!(
            "{} {}: {} {} and {}",
            self.by, self.value, self.field, self.theirs, self.mine
        )
    }
}

/// What an export wrote.
#[derive(Debug, Clone, Serialize)]
pub struct Summary {
    /// different parts traced
    pub parts: usize,
    /// parts found in more than one file
    pub in_several: usize,
    /// tests kept apart because their IDs disagree
    pub conflicts: usize,
    /// problems the ID audit found, listed on their own sheet
    pub id_problems: usize,
    /// rows of the trace: one per part, and one more per extra test of it in one file
    pub rows: u64,
    pub sheets: Vec<String>,
    pub runs: Vec<RunStats>,
}

/// One part: its IDs as written, and its (file, row) results in file order, then test order.
#[derive(Debug, Clone, Default)]
struct Entry {
    vals: Vec<Option<String>>,
    texts: Vec<Option<String>>,
    hits: Vec<(u32, u32)>,
    /// the ID audit found a problem on one of its tests
    flagged: bool,
    /// it shares an ID with another part whose other IDs differ
    conflict: bool,
    /// the part it conflicts with, so the two rows are written together
    follows: Option<usize>,
    /// merged into another part
    into: Option<usize>,
}

/// The parts of a trace, and what each file adds.
struct Parts {
    entries: Vec<Entry>,
    stats: Vec<RunStats>,
    conflicts: Vec<Conflict>,
}

fn root(e: &[Entry], mut i: usize) -> usize {
    while let Some(j) = e[i].into {
        i = j;
    }
    i
}

/// Two sets of IDs agree when no ID has a different value in each.
fn agree(a: &[Option<String>], b: &[Option<String>]) -> bool {
    a.iter().zip(b).all(|(x, y)| match (x, y) {
        (Some(x), Some(y)) => x == y,
        _ => true,
    })
}

/// The IDs of one test of a part, placeholders left out, and its IDs as logged.
fn ids_of(run: &Run, n: usize, r: usize) -> (Vec<Option<String>>, Vec<Option<String>>) {
    let raw: Vec<Option<String>> = (0..n)
        .map(|i| {
            run.keys
                .get(i)
                .and_then(Option::as_ref)
                .and_then(|key| key.key(run.ds, r))
        })
        .collect();
    let ids = raw
        .iter()
        .map(|v| v.clone().filter(|v| placeholder(v).is_none()))
        .collect();
    (ids, raw)
}

fn collect(runs: &[Run], audit: &Audit) -> Parts {
    let titles = id_titles(runs);
    let n = titles.len();
    let mut entries: Vec<Entry> = vec![];
    let mut index: Vec<HashMap<String, Vec<usize>>> = vec![HashMap::new(); n];
    let mut stats = Vec::with_capacity(runs.len());
    let mut conflicts = vec![];
    for (k, run) in runs.iter().enumerate() {
        let mut st = RunStats {
            name: run.name.clone(),
            parts: run.rows.len(),
            has: (0..n)
                .map(|i| run.keys.get(i).is_some_and(Option::is_some))
                .collect(),
            with: vec![0; n],
            ..Default::default()
        };
        let big: Vec<usize> = run.keys.iter().flatten().flat_map(KeySpec::tests).collect();
        for &r in &run.rows {
            let p = &run.ds.parts[r];
            st.good += usize::from(p.is_pass());
            let (ids, raw) = ids_of(run, n, r);
            if ids.iter().all(Option::is_none) {
                if raw.iter().all(Option::is_none) {
                    st.no_id += 1;
                    continue;
                }
                // a placeholder (all zeros…) is on many different parts: the part keeps a row of
                // its own and is never matched
                st.placeholder += 1;
                st.distinct += 1;
                entries.push(Entry {
                    texts: raw,
                    vals: vec![None; n],
                    hits: vec![(k as u32, r as u32)],
                    flagged: audit.flagged.contains(&(k as u32, r as u32)),
                    ..Default::default()
                });
                continue;
            }
            for (i, v) in ids.iter().enumerate() {
                st.with[i] += usize::from(v.is_some());
            }
            st.rounded += usize::from(big.iter().any(|&t| run.ds.value(t, r).abs() > MAX_EXACT));
            // every part an ID of this test points at, in the order they were found
            let mut cands: Vec<usize> = vec![];
            for (i, v) in ids.iter().enumerate() {
                for &c in v
                    .as_ref()
                    .and_then(|v| index[i].get(v))
                    .into_iter()
                    .flatten()
                {
                    let c = root(&entries, c);
                    if !cands.contains(&c) {
                        cands.push(c);
                    }
                }
            }
            cands.sort_unstable();
            let (fit, clash): (Vec<usize>, Vec<usize>) =
                cands.iter().partition(|&&c| agree(&entries[c].vals, &ids));
            let target = match fit.first() {
                Some(&t) => {
                    // parts found by different IDs that turn out to be one: merge them
                    for &c in &fit[1..] {
                        if !agree(&entries[t].vals, &entries[c].vals) {
                            continue;
                        }
                        let other = std::mem::take(&mut entries[c].hits);
                        let (vals, texts) = (entries[c].vals.clone(), entries[c].texts.clone());
                        let (flagged, conflict) = (entries[c].flagged, entries[c].conflict);
                        entries[c].into = Some(t);
                        let e = &mut entries[t];
                        e.hits.extend(other);
                        e.hits.sort_unstable();
                        for i in 0..n {
                            if e.vals[i].is_none() {
                                e.vals[i] = vals[i].clone();
                                e.texts[i] = texts[i].clone();
                            }
                        }
                        e.flagged |= flagged;
                        e.conflict |= conflict;
                    }
                    t
                }
                None => {
                    entries.push(Entry {
                        vals: vec![None; n],
                        texts: vec![None; n],
                        follows: clash.first().copied(),
                        ..Default::default()
                    });
                    entries.len() - 1
                }
            };
            // kept apart: it shares an ID with a part but disagrees on another, and no part fits.
            // (One that joins its part while an ID also points at another part is an ID on two
            // parts: the ID audit lists those.)
            for &c in clash.iter().take(usize::from(fit.is_empty())) {
                entries[c].conflict = true;
                entries[target].conflict = true;
                let by = (0..n).find(|&i| ids[i].is_some() && ids[i] == entries[c].vals[i]);
                let field = (0..n).find(
                    |&i| matches!((&ids[i], &entries[c].vals[i]), (Some(a), Some(b)) if a != b),
                );
                if let (Some(by), Some(field)) = (by, field) {
                    let at = entries[c].hits.last().copied().unwrap_or_default();
                    conflicts.push(Conflict {
                        by: titles[by].clone(),
                        value: entries[c].texts[by].clone().unwrap_or_default(),
                        field: titles[field].clone(),
                        mine: ids[field].clone().unwrap_or_default(),
                        theirs: entries[c].vals[field].clone().unwrap_or_default(),
                        at: (k, r),
                        other: (at.0 as usize, at.1 as usize),
                    });
                }
            }
            let e = &mut entries[target];
            for i in 0..n {
                if e.vals[i].is_some() {
                    continue;
                }
                if let Some(v) = &ids[i] {
                    e.vals[i] = Some(v.clone());
                    e.texts[i] = Some(match run.keys[i] {
                        Some(KeySpec::PartTxt) => p.part_txt.trim().to_string(),
                        _ => v.clone(),
                    });
                    index[i].entry(v.clone()).or_default().push(target);
                }
            }
            e.flagged |= audit.flagged.contains(&(k as u32, r as u32));
            match e.hits.last() {
                Some(&(last, earlier)) if last == k as u32 => {
                    st.repeats += 1;
                    if run.last_only {
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
    for e in entries.iter().filter(|e| e.into.is_none()) {
        let files: HashSet<u32> = e.hits.iter().map(|h| h.0).collect();
        if files.len() > 1 {
            for f in files {
                stats[f as usize].shared += 1;
            }
        }
    }
    Parts {
        entries,
        stats,
        conflicts,
    }
}

/// The parts in the order they are written: as first found, each followed by the parts it
/// conflicts with.
fn order(entries: &[Entry]) -> Vec<usize> {
    let mut after: Vec<Vec<usize>> = vec![vec![]; entries.len()];
    let mut top = vec![];
    for (i, e) in entries.iter().enumerate().filter(|(_, e)| e.into.is_none()) {
        match e.follows.map(|f| root(entries, f)).filter(|&f| f != i) {
            Some(f) => after[f].push(i),
            None => top.push(i),
        }
    }
    let mut out = Vec::with_capacity(entries.len());
    let mut stack: Vec<usize> = top.into_iter().rev().collect();
    let mut seen = vec![false; entries.len()];
    while let Some(i) = stack.pop() {
        if std::mem::replace(&mut seen[i], true) {
            continue;
        }
        out.push(i);
        stack.extend(after[i].iter().rev());
    }
    out
}

/// Which files hold the same parts by any of their IDs, and the tests kept apart because their IDs
/// disagree. File numbers refer to the runs given.
#[derive(Debug, Clone, Serialize)]
pub struct Detection {
    pub titles: Vec<String>,
    pub files: Vec<RunStats>,
    pub pairs: Vec<Pair>,
    /// files linked by shared parts, directly or through another file; two or more in each
    pub groups: Vec<Vec<usize>>,
    /// parts found in more than one file
    pub in_several: usize,
    /// per ID: of those, the parts that have this ID in two or more files
    pub by_id: Vec<usize>,
    pub conflicts: Vec<Conflict>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Pair {
    pub a: usize,
    pub b: usize,
    pub shared: usize,
}

pub fn detect(runs: &[Run]) -> Detection {
    let parts = collect(runs, &Audit::default());
    let titles = id_titles(runs);
    let n = titles.len();
    let mut pairs: IndexMap<(usize, usize), usize> = IndexMap::new();
    let mut in_several = 0;
    let mut by_id = vec![0; n];
    for e in parts.entries.iter().filter(|e| e.into.is_none()) {
        let mut files: Vec<usize> = e.hits.iter().map(|h| h.0 as usize).collect();
        files.dedup();
        if files.len() < 2 {
            continue;
        }
        in_several += 1;
        for (a, &fa) in files.iter().enumerate() {
            for &fb in &files[a + 1..] {
                *pairs.entry((fa, fb)).or_default() += 1;
            }
        }
        for (i, count) in by_id.iter_mut().enumerate() {
            let with: HashSet<u32> = e
                .hits
                .iter()
                .filter(|&&(k, r)| ids_of(&runs[k as usize], n, r as usize).0[i].is_some())
                .map(|h| h.0)
                .collect();
            *count += usize::from(with.len() > 1);
        }
    }
    let mut root: Vec<usize> = (0..runs.len()).collect();
    fn find(root: &mut [usize], i: usize) -> usize {
        if root[i] != i {
            root[i] = find(root, root[i]);
        }
        root[i]
    }
    for &(a, b) in pairs.keys() {
        let (ra, rb) = (find(&mut root, a), find(&mut root, b));
        root[rb] = ra;
    }
    let mut by_root: IndexMap<usize, Vec<usize>> = IndexMap::new();
    for i in 0..runs.len() {
        let r = find(&mut root, i);
        by_root.entry(r).or_default().push(i);
    }
    Detection {
        titles,
        files: parts.stats,
        pairs: pairs
            .into_iter()
            .map(|((a, b), shared)| Pair { a, b, shared })
            .collect(),
        groups: by_root.into_values().filter(|g| g.len() > 1).collect(),
        in_several,
        by_id,
        conflicts: parts.conflicts,
    }
}

/// Writes the trace of `runs` to the workbook `out`.
pub fn export(runs: &[Run], out: &Path, filter: &str) -> Result<Summary> {
    export_with(runs, out, filter, MAX_ROWS - 1)
}

/// As `export`, marking the parts the ID audit flagged and listing its problems on their own sheet.
/// `audit` holds file numbers in the order of `runs`.
pub fn export_audited(runs: &[Run], out: &Path, filter: &str, audit: &Audit) -> Result<Summary> {
    export_full(runs, out, filter, MAX_ROWS - 1, audit)
}

/// As `export`, with at most `max_rows` part rows per sheet (below the two title rows). A longer
/// trace continues on the next sheet, named "Part trace (2)" and so on.
pub fn export_with(runs: &[Run], out: &Path, filter: &str, max_rows: u64) -> Result<Summary> {
    export_full(runs, out, filter, max_rows, &Audit::default())
}

fn export_full(
    runs: &[Run],
    out: &Path,
    filter: &str,
    max_rows: u64,
    audit: &Audit,
) -> Result<Summary> {
    if runs.is_empty() {
        bail!("choose at least one file to trace");
    }
    if !out
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("xlsx"))
    {
        bail!("the report is an Excel workbook: save it as a .xlsx file");
    }
    if runs.iter().all(|r| r.keys.iter().all(Option::is_none)) {
        bail!("none of these files logs the tests chosen to identify parts");
    }
    let parts = collect(runs, audit);
    let layout = Layout::new(runs, !parts.conflicts.is_empty())?;
    let dir = xlsx::scratch_dir()?;
    let result = write_book(
        runs,
        out,
        filter,
        max_rows.max(1),
        &layout,
        &dir,
        audit,
        parts,
    );
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
    audit: &Audit,
    parts: Parts,
) -> Result<Summary> {
    let Parts {
        entries,
        stats,
        conflicts,
    } = parts;
    let mut sheets: Vec<Sheet> = vec![];
    let mut rows = 0u64;
    let mut by_run: Vec<Vec<u32>> = vec![vec![]; runs.len()];
    let mut in_several = 0usize;
    // which part each test ended up in, for the note on parts whose IDs disagree
    let mut part_of: HashMap<(u32, u32), usize> = HashMap::new();
    for (i, e) in entries.iter().enumerate().filter(|(_, e)| e.into.is_none()) {
        for &h in &e.hits {
            part_of.insert(h, i);
        }
    }
    let mut notes: HashMap<usize, String> = HashMap::new();
    for c in &conflicts {
        let note = format!("same {} as another row, other {}", c.by, c.field);
        for (k, r) in [c.at, c.other] {
            if let Some(&i) = part_of.get(&(k as u32, r as u32)) {
                notes.entry(i).or_insert_with(|| note.clone());
            }
        }
    }
    let shown = order(&entries);
    for &n in &shown {
        let e = &entries[n];
        by_run.iter_mut().for_each(Vec::clear);
        for &(k, r) in &e.hits {
            by_run[k as usize].push(r);
        }
        in_several += usize::from(by_run.iter().filter(|h| !h.is_empty()).count() > 1);
        let depth = by_run.iter().map(Vec::len).max().unwrap_or(0);
        let note = e.conflict.then(|| {
            notes
                .get(&n)
                .map_or("IDs disagree with another row", String::as_str)
        });
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
                e,
                note,
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
        parts: shown.len(),
        in_several,
        conflicts: conflicts.len(),
        id_problems: audit.findings.len(),
        rows,
        sheets: sheets
            .iter()
            .map(|s| s.name.clone())
            .chain(["Summary".to_string()])
            .chain((!audit.findings.is_empty()).then(|| "ID audit".to_string()))
            .collect(),
        runs: stats,
    };
    sheets.push(summary_sheet(dir, sheets.len(), runs, filter, &summary)?);
    if !audit.findings.is_empty() {
        sheets.push(audit_sheet(dir, sheets.len(), runs, audit)?);
    }
    xlsx::write_zip(out, &sheets, &names)?;
    Ok(summary)
}

/// Where the columns are: the IDs first, then a note on IDs that disagree, then each file's group.
struct Layout {
    titles: Vec<String>,
    /// the letter of the note column, when some part's IDs disagree
    note: Option<String>,
    /// per file: the letters of Part ID, Hard Bin, Site, First and Last Fail Test
    groups: Vec<Vec<String>>,
    last: String,
}

impl Layout {
    fn new(runs: &[Run], note: bool) -> Result<Layout> {
        let titles = id_titles(runs);
        let mut next = titles.len() + usize::from(note);
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
            note: note.then(|| column_letters(titles.len())),
            titles,
            groups,
            last: column_letters(next - 1),
        })
    }

    /// Columns before the first file's group: the IDs and the note.
    fn fixed(&self) -> usize {
        self.titles.len() + usize::from(self.note.is_some())
    }
}

fn new_trace_sheet(dir: &Path, id: usize, runs: &[Run], layout: &Layout) -> Result<Sheet> {
    let n = layout.titles.len();
    let mut cols = format!(r#"<col min="1" max="{n}" width="22" customWidth="1"/>"#);
    let mut at = n + 1;
    if layout.note.is_some() {
        cols.push_str(&format!(
            r#"<col min="{at}" max="{at}" width="34" customWidth="1"/>"#
        ));
        at += 1;
    }
    for _ in runs {
        for w in [8, 8, 5, 34, 34] {
            cols.push_str(&format!(
                r#"<col min="{at}" max="{at}" width="{w}" customWidth="1"/>"#
            ));
            at += 1;
        }
    }
    let fixed = layout.fixed();
    let top_left = format!("{}3", column_letters(fixed));
    let head = format!(
        r#"<sheetViews><sheetView workbookViewId="0"><pane xSplit="{fixed}" ySplit="2" topLeftCell="{top_left}" activePane="bottomRight" state="frozen"/></sheetView></sheetViews><sheetFormatPr defaultRowHeight="15"/><cols>{cols}</cols>"#
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
    for i in 0..fixed {
        write_styled(out, &column_letters(i), 1, "", style::LABEL)?;
    }
    for (k, (run, letters)) in runs.iter().zip(&layout.groups).enumerate() {
        let colour = style::GROUP + k as u32 % style::GROUPS;
        write_styled(out, &letters[0], 1, &run.name, colour)?;
        for letter in &letters[1..] {
            write!(out, "<c r=\"{letter}1\" s=\"{colour}\"/>")?;
        }
    }
    out.write_all(b"</row><row r=\"2\">")?;
    for (i, title) in layout.titles.iter().enumerate() {
        write_styled(out, &column_letters(i), 2, title, style::TITLE)?;
    }
    if let Some(letter) = &layout.note {
        write_styled(out, letter, 2, "ID check", style::TITLE)?;
    }
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

/// One row: the part's `i`th test in each file that has one. The part's IDs are text, so a code
/// such as 12E4 is never read as a number.
fn write_part_row(
    out: &mut impl Write,
    row: u64,
    runs: &[Run],
    layout: &Layout,
    e: &Entry,
    note: Option<&str>,
    by_run: &[Vec<u32>],
    i: usize,
) -> io::Result<()> {
    write!(out, "<row r=\"{row}\">")?;
    let id_style = if e.flagged || e.conflict {
        style::FAIL
    } else {
        style::LABEL
    };
    for (c, text) in e.texts.iter().enumerate() {
        write_styled(
            out,
            &column_letters(c),
            row,
            text.as_deref().unwrap_or(""),
            id_style,
        )?;
    }
    if let (Some(letter), Some(note)) = (&layout.note, note) {
        write_styled(out, letter, row, note, style::FAIL)?;
    }
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
        ("Parts matched by", id_titles(runs).join(" or ")),
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
        ("ID problems (see ID audit)", s.id_problems as u64),
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

/// The problems of the ID audit, one row per test of a part.
fn audit_sheet(dir: &Path, id: usize, runs: &[Run], audit: &Audit) -> Result<Sheet> {
    let head = concat!(
        r#"<sheetViews><sheetView workbookViewId="0"><pane ySplit="1" topLeftCell="A2" activePane="bottomLeft" state="frozen"/></sheetView></sheetViews>"#,
        r#"<sheetFormatPr defaultRowHeight="15"/><cols><col min="1" max="1" width="8" customWidth="1"/><col min="2" max="2" width="60" customWidth="1"/><col min="3" max="3" width="30" customWidth="1"/>"#,
        r#"<col min="4" max="4" width="9" customWidth="1"/><col min="5" max="5" width="26" customWidth="1"/><col min="6" max="7" width="16" customWidth="1"/><col min="8" max="8" width="9" customWidth="1"/></cols>"#
    );
    let mut sheet = xlsx::open_sheet(dir, id, "ID audit".to_string(), head)?;
    let out = &mut sheet.out;
    let titles = [
        "Problem",
        "What",
        "File",
        "Part ID",
        "Part text",
        "WXY",
        "W,X,Y",
        "Hard Bin",
    ];
    out.write_all(b"<row r=\"1\">")?;
    for (i, title) in titles.iter().enumerate() {
        write_styled(out, &column_letters(i), 1, title, style::TITLE)?;
    }
    out.write_all(b"</row>")?;
    let mut row = 1u64;
    'all: for (n, f) in audit.findings.iter().enumerate() {
        let what = f.text();
        for r in &f.rows {
            if row >= MAX_ROWS {
                break 'all;
            }
            row += 1;
            write!(out, "<row r=\"{row}\">")?;
            write_num(out, "A", row, n as u64 + 1, 0)?;
            write_styled(out, "B", row, &what, 0)?;
            let name = runs.get(r.run).map_or("", |run| run.name.as_str());
            write_styled(out, "C", row, name, 0)?;
            write_id(out, "D", row, &r.part_id)?;
            write_styled(out, "E", row, &r.part_txt, 0)?;
            write_styled(out, "F", row, &r.wxy, 0)?;
            write_styled(out, "G", row, &r.w_x_y, 0)?;
            write_num(out, "H", row, u64::from(r.hbin), 0)?;
            out.write_all(b"</row>")?;
        }
    }
    sheet.rows = row - 1;
    out.write_all(b"</sheetData></worksheet>")?;
    out.flush()?;
    Ok(sheet)
}

/// The tests that identify a part besides its part text, as numbers or names.
#[derive(Debug, Clone, Default)]
pub struct IdTests {
    pub wxy: String,
    pub w: String,
    pub x: String,
    pub y: String,
    /// X and Y are relative to these: die X and Y are the tests with these applied
    pub offsets: Option<(String, String)>,
    /// offsets are taken off rather than added
    pub subtract: bool,
}

/// One file to audit.
pub struct AuditFile<'a> {
    pub ds: &'a Dataset,
    pub rows: Vec<usize>,
}

/// A test of a part in a problem. `run` is the file's number in the audit.
#[derive(Debug, Clone, Serialize)]
pub struct AuditRow {
    pub run: usize,
    pub row: usize,
    pub part_id: String,
    pub part_txt: String,
    pub wxy: String,
    pub w_x_y: String,
    pub hbin: u16,
}

/// An ID problem: one value of an ID goes with several values of another, or a WXY result is not
/// the wafer, X and Y of the same part.
#[derive(Debug, Clone, Serialize)]
pub struct Finding {
    /// `id_differs` or `wxy_differs`
    pub kind: &'static str,
    pub id: &'static str,
    pub value: String,
    pub other: &'static str,
    pub others: Vec<String>,
    pub rows: Vec<AuditRow>,
}

impl Finding {
    pub fn text(&self) -> String {
        if self.kind == "wxy_differs" {
            return format!("WXY {} is not its W,X,Y {}", self.value, self.others[0]);
        }
        const SHOWN: usize = 5;
        let mut list = self.others[..self.others.len().min(SHOWN)].join(", ");
        if self.others.len() > SHOWN {
            list.push_str(", …");
        }
        format!(
            "{} {} has {} values of {}: {list}",
            self.id,
            self.value,
            self.others.len(),
            self.other
        )
    }
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Audit {
    pub findings: Vec<Finding>,
    /// tests of parts that have at least two IDs to compare
    pub compared: usize,
    /// WXY results too large to be exact, which are not used to tell parts apart
    pub rounded: usize,
    /// (file, row) of every test in a problem
    #[serde(skip)]
    pub flagged: HashSet<(u32, u32)>,
}

const ID_NAMES: [&str; 3] = ["Part text", "WXY", "W,X,Y"];

/// The test numbers an ID reads in one file; a test the file lacks leaves that ID out.
struct IdReader {
    wxy: Option<usize>,
    wt: Option<[usize; 3]>,
    offsets: [Option<usize>; 2],
    sign: f64,
}

impl IdReader {
    fn new(ds: &Dataset, t: &IdTests) -> IdReader {
        let find = |q: &str| {
            let q = q.trim();
            (!q.is_empty()).then(|| ds.find_test(q)).flatten()
        };
        let wt = match (find(&t.w), find(&t.x), find(&t.y)) {
            (Some(w), Some(x), Some(y)) => Some([w, x, y]),
            _ => None,
        };
        let (ox, oy) = t
            .offsets
            .as_ref()
            .map_or((None, None), |o| (find(&o.0), find(&o.1)));
        IdReader {
            wxy: find(&t.wxy),
            wt,
            offsets: [ox, oy],
            sign: if t.subtract { -1.0 } else { 1.0 },
        }
    }

    /// Wafer, X and Y of a part, the offsets applied where the part has them.
    fn w_x_y(&self, ds: &Dataset, r: usize) -> Option<[f64; 3]> {
        let [w, x, y] = self.wt?;
        let (w, x, y) = (ds.value(w, r), ds.value(x, r), ds.value(y, r));
        if !(w.is_finite() && x.is_finite() && y.is_finite()) {
            return None;
        }
        let off = |o: Option<usize>| {
            o.map(|t| ds.value(t, r))
                .filter(|v| v.is_finite())
                .map_or(0.0, |v| v * self.sign)
        };
        Some([w, x + off(self.offsets[0]), y + off(self.offsets[1])])
    }
}

/// The WXY code of a wafer, X and Y: the wafer, then X and Y zero-padded to three digits. None when
/// they do not fit that form, so the code cannot be checked.
fn wxy_code(v: [f64; 3]) -> Option<f64> {
    let [w, x, y] = v.map(f64::round);
    let fits = (0.0..=999.0).contains(&x) && (0.0..=999.0).contains(&y) && w >= 0.0;
    fits.then_some(w * 1e6 + x * 1e3 + y)
}

/// Checks that a part's IDs agree, in each file and across files: part text, the WXY test and the
/// wafer, X and Y tests. A missing value says nothing (a part can fail before those tests), and
/// placeholder values are not IDs. Results are compared as logged, so a WXY rounded by the 32-bit
/// result still matches its own W,X,Y, and a rounded WXY never accuses two parts of sharing it.
pub fn audit(files: &[AuditFile], tests: &IdTests) -> Audit {
    struct Seen {
        ids: [Option<String>; 3],
        rounded: bool,
        row: AuditRow,
    }
    let mut seen: Vec<Seen> = vec![];
    let mut out = Audit::default();
    for (k, f) in files.iter().enumerate() {
        let reader = IdReader::new(f.ds, tests);
        for &r in &f.rows {
            let p = &f.ds.parts[r];
            let txt = norm(&p.part_txt);
            let wxy = reader
                .wxy
                .map(|t| f.ds.value(t, r))
                .filter(|v| v.is_finite());
            let wt = reader.w_x_y(f.ds, r);
            let wt_text = wt.map(|v| {
                let [w, x, y] = v.map(num_key);
                wxy_text(&w, &x, &y)
            });
            let clean = |s: Option<String>| s.filter(|v| !v.is_empty() && placeholder(v).is_none());
            let ids = [clean(Some(txt)), clean(wxy.map(num_key)), clean(wt_text)];
            let rounded = wxy.is_some_and(|v| v.abs() > MAX_EXACT);
            out.rounded += usize::from(rounded);
            let row = AuditRow {
                run: k,
                row: r,
                part_id: p.part_id.clone(),
                part_txt: p.part_txt.trim().to_string(),
                wxy: ids[1].clone().unwrap_or_default(),
                w_x_y: ids[2].clone().unwrap_or_default(),
                hbin: p.hbin,
            };
            if ids.iter().flatten().count() >= 2 {
                out.compared += 1;
            }
            // the code as a 32-bit result would hold it, as the logged WXY is
            if let (Some(a), Some(b), Some(code)) = (&ids[1], &ids[2], wt.and_then(wxy_code)) {
                if wxy.is_some_and(|v| code as f32 != v as f32) {
                    out.flagged.insert((k as u32, r as u32));
                    out.findings.push(Finding {
                        kind: "wxy_differs",
                        id: "WXY",
                        value: a.clone(),
                        other: "W,X,Y",
                        others: vec![b.clone()],
                        rows: vec![row.clone()],
                    });
                }
            }
            seen.push(Seen { ids, rounded, row });
        }
    }
    for a in 0..ID_NAMES.len() {
        for b in (0..ID_NAMES.len()).filter(|&b| b != a) {
            let mut groups: IndexMap<&str, IndexMap<&str, Vec<usize>>> = IndexMap::new();
            for (i, s) in seen.iter().enumerate() {
                if a == 1 && s.rounded {
                    continue;
                }
                if let (Some(x), Some(y)) = (&s.ids[a], &s.ids[b]) {
                    groups.entry(x).or_default().entry(y).or_default().push(i);
                }
            }
            for (value, others) in groups.iter().filter(|(_, o)| o.len() > 1) {
                let rows: Vec<AuditRow> = others
                    .values()
                    .flatten()
                    .map(|&i| seen[i].row.clone())
                    .collect();
                for r in &rows {
                    out.flagged.insert((r.run as u32, r.row as u32));
                }
                out.findings.push(Finding {
                    kind: "id_differs",
                    id: ID_NAMES[a],
                    value: value.to_string(),
                    other: ID_NAMES[b],
                    others: others.keys().map(|s| s.to_string()).collect(),
                    rows,
                });
            }
        }
    }
    out
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
                    inst_alarms: 0,
                })
                .collect(),
            ..Default::default()
        }
    }

    fn run<'a>(name: &str, d: &'a Dataset) -> Run<'a> {
        Run {
            name: name.into(),
            ds: d,
            keys: vec![Some(KeySpec::PartTxt)],
            rows: (0..d.n_parts()).collect(),
            last_only: false,
        }
    }

    /// A file traced by the IDs `specs`.
    fn by<'a>(name: &str, d: &'a Dataset, specs: &[&str]) -> Run<'a> {
        Run {
            name: name.into(),
            ds: d,
            keys: specs.iter().map(|k| key_for(k, d)).collect(),
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
        // each is a part of its own, not a repeat of one
        assert_eq!(
            (s.rows, s.parts, s.runs[0].left_out, s.runs[0].repeats),
            (2, 2, 0, 0)
        );
        assert_eq!(s.runs[0].placeholder, 2);
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

    /// `ds`, with one test holding these results, one per part.
    fn with_values(rows: &[Row], values: &[f32]) -> Dataset {
        let mut d = ds(rows);
        d.values = vec![
            values.to_vec(),
            vec![f32::NAN; values.len()],
            vec![f32::NAN; values.len()],
        ];
        d
    }

    #[test]
    fn a_test_combination_identifies_parts_and_a_file_without_the_tests_has_no_ids() {
        let d = tempfile::tempdir().unwrap();
        let book = d.path().join("trace.xlsx");
        let mut one = with_values(
            &[("1", "", 1, true, &[]), ("2", "", 1, true, &[])],
            &[7.0, 8.0],
        );
        one.values[1] = vec![10.0, 11.0];
        one.test_index = one
            .tests
            .iter()
            .enumerate()
            .map(|(i, t)| (t.key.clone(), i))
            .collect();
        let mut two = one.clone();
        two.parts.truncate(1);
        two.values = vec![vec![7.0], vec![10.0], vec![f32::NAN]];
        let other = Dataset {
            tests: vec![],
            ..ds(&[("1", "", 1, true, &[])])
        };
        let spec = "tests:100,200";
        let traced = |name: &str, d, spec| by(name, d, &[spec]);
        assert!(key_for(spec, &other).is_none());
        let runs = [
            traced("a", &one, spec),
            traced("b", &two, spec),
            traced("c", &other, spec),
        ];
        let s = export(&runs, &book, "").unwrap();
        assert_eq!((s.parts, s.in_several), (2, 1));
        assert_eq!(s.runs[2].no_id, 1);
        let x = part(&book, "xl/worksheets/sheet1.xml");
        assert_eq!(cell(&x, "A2").unwrap(), "PTR 100+200");
        assert_eq!(cell(&x, "A3").unwrap(), "7_10");
        assert_eq!(cell(&x, "G3").unwrap(), "1");
        // nothing to trace at all is an error, not an empty report
        assert!(export(&[traced("c", &other, spec)], &book, "").is_err());
    }

    #[test]
    fn a_result_too_large_to_be_exact_is_counted() {
        // two neighbouring dies of a wafer 25: a 32-bit float cannot tell them apart
        assert_eq!(25113139f32, 25113140f32);
        let d = with_values(
            &[("1", "", 1, true, &[]), ("2", "", 1, true, &[])],
            &[25113139.0, 25113140.0],
        );
        let f = detect(&[Run {
            name: "a".into(),
            ds: &d,
            keys: vec![Some(KeySpec::Tests(vec![0]))],
            rows: vec![0, 1],
            last_only: false,
        }]);
        assert_eq!((f.files[0].rounded, f.files[0].repeats), (2, 1));
    }

    /// Parts as (part text, WXY, wafer, X, Y); NaN = not logged.
    fn with_ids(rows: &[(&str, f32, f32, f32, f32)]) -> Dataset {
        let mut d = ds(&[]);
        d.parts = rows
            .iter()
            .enumerate()
            .map(|(i, r)| {
                let mut p = ds(&[("", "", 1, true, &[])]).parts.remove(0);
                p.part_id = (i + 1).to_string();
                p.part_txt = r.0.into();
                p
            })
            .collect();
        let names = [(1, "WXY"), (2, "W"), (3, "X"), (4, "Y")];
        d.tests = names
            .map(|(num, name)| TestInfo {
                key: format!("{num}:{name}"),
                num,
                ..d.tests[0].clone()
            })
            .to_vec();
        d.test_index = d
            .tests
            .iter()
            .enumerate()
            .map(|(i, t)| (t.key.clone(), i))
            .collect();
        d.values = vec![
            rows.iter().map(|r| r.1).collect(),
            rows.iter().map(|r| r.2).collect(),
            rows.iter().map(|r| r.3).collect(),
            rows.iter().map(|r| r.4).collect(),
        ];
        d
    }

    fn id_tests() -> IdTests {
        IdTests {
            wxy: "1".into(),
            w: "2".into(),
            x: "3".into(),
            y: "4".into(),
            ..Default::default()
        }
    }

    fn audited(sets: &[&Dataset]) -> Audit {
        let files: Vec<AuditFile> = sets
            .iter()
            .map(|d| AuditFile {
                ds: d,
                rows: (0..d.n_parts()).collect(),
            })
            .collect();
        audit(&files, &id_tests())
    }

    const NA: f32 = f32::NAN;

    #[test]
    fn audit_passes_consistent_parts_and_a_part_that_failed_before_the_tests() {
        let one = with_ids(&[
            ("A", 3113138.0, 3.0, 113.0, 138.0),
            ("B", 3114140.0, 3.0, 114.0, 140.0),
        ]);
        // A failed before its WXY and X/Y tests this time: nothing to compare, and no problem
        let two = with_ids(&[("A", NA, NA, NA, NA), ("B", 3114140.0, 3.0, 114.0, 140.0)]);
        let a = audited(&[&one, &two]);
        assert!(a.findings.is_empty(), "{:?}", a.findings);
        assert_eq!(a.compared, 3);
    }

    #[test]
    fn audit_finds_ids_that_disagree_across_files() {
        let one = with_ids(&[
            ("A", 3113138.0, 3.0, 113.0, 138.0),
            ("B", 3114140.0, 3.0, 114.0, 140.0),
        ]);
        // part text A now reads a different die; die 114,140 now carries part text C
        let two = with_ids(&[
            ("A", 3200200.0, 3.0, 200.0, 200.0),
            ("C", 3114140.0, 3.0, 114.0, 140.0),
        ]);
        let a = audited(&[&one, &two]);
        let texts: Vec<String> = a.findings.iter().map(Finding::text).collect();
        assert!(
            texts
                .iter()
                .any(|t| t == "Part text A has 2 values of WXY: 3113138, 3200200"),
            "{texts:?}"
        );
        assert!(
            texts
                .iter()
                .any(|t| t == "WXY 3114140 has 2 values of Part text: B, C"),
            "{texts:?}"
        );
        assert!(a.flagged.contains(&(1, 0)) && a.flagged.contains(&(1, 1)));
    }

    #[test]
    fn audit_checks_wxy_against_its_own_wafer_x_and_y() {
        let d = with_ids(&[
            ("A", 17125132.0, 17.0, 125.0, 130.0),
            ("B", 17113138.0, 17.0, 113.0, 138.0),
        ]);
        let a = audited(&[&d]);
        assert_eq!(a.findings.len(), 1);
        assert_eq!(a.findings[0].kind, "wxy_differs");
        assert_eq!(a.findings[0].rows[0].part_txt, "A");
    }

    #[test]
    fn audit_does_not_accuse_parts_of_a_wxy_rounded_by_the_32_bit_result() {
        // neighbouring dies of wafer 25: their WXY results are the same number
        let d = with_ids(&[
            ("A", 25113139.0, 25.0, 113.0, 139.0),
            ("B", 25113140.0, 25.0, 113.0, 140.0),
        ]);
        let a = audited(&[&d]);
        assert!(a.findings.is_empty(), "{:?}", a.findings);
        assert_eq!(a.rounded, 2);
    }

    #[test]
    fn audit_problems_are_marked_on_the_trace_and_listed_on_their_own_sheet() {
        let dir = tempfile::tempdir().unwrap();
        let book = dir.path().join("trace.xlsx");
        let one = with_ids(&[
            ("A", 3113138.0, 3.0, 113.0, 138.0),
            ("B", 3114140.0, 3.0, 114.0, 140.0),
        ]);
        let two = with_ids(&[("A", 3200200.0, 3.0, 200.0, 200.0)]);
        let a = audited(&[&one, &two]);
        let s = export_audited(&[run("one", &one), run("two", &two)], &book, "", &a).unwrap();
        assert_eq!(s.sheets, ["Part trace", "Summary", "ID audit"]);
        assert_eq!(s.id_problems, a.findings.len());
        let x = part(&book, "xl/worksheets/sheet1.xml");
        assert!(x.contains(&format!(
            "<c r=\"A3\" t=\"inlineStr\" s=\"{}\"",
            style::FAIL
        )));
        assert!(x.contains(&format!(
            "<c r=\"A4\" t=\"inlineStr\" s=\"{}\"",
            style::LABEL
        )));
        let x3 = part(&book, "xl/worksheets/sheet3.xml");
        assert!(x3.contains("Part text A has 2 values of WXY"));
        // a clean report has no such sheet
        let s = export(&[run("one", &one)], &book, "").unwrap();
        assert_eq!(s.sheets, ["Part trace", "Summary"]);
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
        let cand = |d| run("f", d);
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

    const IDS: [&str; 3] = ["part_txt", "tests:1", "w_x_y:2,3,4"];

    #[test]
    fn any_id_links_a_part_so_one_fills_the_gaps_of_another() {
        let d = tempfile::tempdir().unwrap();
        let book = d.path().join("trace.xlsx");
        let nan = f32::NAN;
        // production: part text always, WXY only where the part reached that test
        let prod = with_ids(&[("P1", 3001002.0, 3.0, 1.0, 2.0), ("P2", nan, nan, nan, nan)]);
        // engineering: no part text, only the PTR IDs
        let eng = with_ids(&[("", 3001002.0, 3.0, 1.0, 2.0)]);
        // a retest where P2 now gets its WXY
        let retest = with_ids(&[("P2", 3001003.0, 3.0, 1.0, 3.0)]);
        let runs = [
            by("prod", &prod, &IDS),
            by("eng", &eng, &IDS),
            by("retest", &retest, &IDS),
        ];
        let f = detect(&runs);
        assert_eq!(f.titles, ["Part_Text", "PTR 1", "W,X,Y"]);
        assert_eq!((f.in_several, f.groups.clone()), (2, vec![vec![0, 1, 2]]));
        // P1 is in two files by WXY and W,X,Y; P2 by part text only
        assert_eq!(f.by_id, [1, 1, 1]);
        assert!(f.conflicts.is_empty());
        let s = export(&runs, &book, "").unwrap();
        assert_eq!((s.parts, s.in_several, s.conflicts), (2, 2, 0));
        let x = part(&book, "xl/worksheets/sheet1.xml");
        assert_eq!(cell(&x, "A2").unwrap(), "Part_Text");
        assert_eq!(cell(&x, "C2").unwrap(), "W,X,Y");
        assert_eq!(cell(&x, "A3").unwrap(), "P1");
        assert_eq!(cell(&x, "C3").unwrap(), "W3X1Y2");
        // the engineering file's group starts after prod's five columns
        assert_eq!(cell(&x, "I3").unwrap(), "1");
        // P2's WXY comes from the retest
        assert_eq!(cell(&x, "B4").unwrap(), "3001003");
        assert_eq!(cell(&x, "N4").unwrap(), "1");
    }

    #[test]
    fn ids_that_disagree_keep_two_rows_side_by_side() {
        let d = tempfile::tempdir().unwrap();
        let book = d.path().join("trace.xlsx");
        let one = with_ids(&[
            ("P1", 3001002.0, 3.0, 1.0, 2.0),
            ("P9", 3009009.0, 3.0, 9.0, 9.0),
        ]);
        // the same part text, another die: a 2D code read onto the wrong part
        let two = with_ids(&[("P1", 3005005.0, 3.0, 5.0, 5.0)]);
        let runs = [by("one", &one, &IDS), by("two", &two, &IDS)];
        let f = detect(&runs);
        assert_eq!(f.in_several, 0);
        assert_eq!(f.conflicts.len(), 1);
        assert_eq!(
            f.conflicts[0].text(),
            "Part_Text P1: PTR 1 3001002 and 3005005"
        );
        let s = export(&runs, &book, "").unwrap();
        assert_eq!((s.parts, s.conflicts), (3, 1));
        let x = part(&book, "xl/worksheets/sheet1.xml");
        // the note column comes after the IDs, and the kept-apart row right after its partner
        assert_eq!(cell(&x, "D2").unwrap(), "ID check");
        assert_eq!(cell(&x, "A3").unwrap(), "P1");
        assert_eq!(cell(&x, "A4").unwrap(), "P1");
        assert_eq!(cell(&x, "C4").unwrap(), "W3X5Y5");
        assert_eq!(cell(&x, "A5").unwrap(), "P9");
        assert!(cell(&x, "D4").unwrap().contains("same Part_Text"));
        assert!(!x.contains(r#"<c r="D5""#));
    }

    #[test]
    fn an_unticked_id_is_neither_a_column_nor_a_link() {
        let d = tempfile::tempdir().unwrap();
        let book = d.path().join("trace.xlsx");
        let one = with_ids(&[("P1", 3001002.0, 3.0, 1.0, 2.0)]);
        let two = with_ids(&[("", 3001002.0, 3.0, 1.0, 2.0)]);
        let ids = ["part_txt", "w_x_y:2,3,4"];
        let s = export(&[by("one", &one, &ids), by("two", &two, &ids)], &book, "").unwrap();
        assert_eq!(s.in_several, 1);
        let x = part(&book, "xl/worksheets/sheet1.xml");
        assert_eq!(cell(&x, "B2").unwrap(), "W,X,Y");
        assert_eq!(cell(&x, "C2").unwrap(), "Part ID");
        let s = export(
            &[by("one", &one, &ids[..1]), by("two", &two, &ids[..1])],
            &book,
            "",
        )
        .unwrap();
        assert_eq!((s.parts, s.in_several, s.runs[1].no_id), (1, 0, 1));
    }

    #[test]
    fn wafer_x_and_y_read_as_w_x_y_with_die_offsets() {
        let mut d = with_ids(&[("", 0.0, 3.0, -2.0, 25.0)]);
        // two more tests: X and Y offsets
        let t = d.tests[0].clone();
        for num in [5, 6] {
            d.tests.push(TestInfo {
                key: format!("{num}:off"),
                num,
                ..t.clone()
            });
        }
        d.test_index = d
            .tests
            .iter()
            .enumerate()
            .map(|(i, t)| (t.key.clone(), i))
            .collect();
        d.values.push(vec![10.0]);
        d.values.push(vec![-5.0]);
        let raw = KeySpec::parse("w_x_y:2,3,4", &d).unwrap();
        assert_eq!(raw.key(&d, 0).unwrap(), "W3X-2Y25");
        let die = KeySpec::parse("w_x_y:2,3,4,5,6", &d).unwrap();
        assert_eq!(die.key(&d, 0).unwrap(), "W3X8Y20");
        let sub = KeySpec::parse("w_x_y_sub:2,3,4,5,6", &d).unwrap();
        assert_eq!(sub.key(&d, 0).unwrap(), "W3X-12Y30");
        // a value typed with separators, or in any case, finds the part
        for typed in ["3|-2|25", "3, -2, 25", "w3x-2y25", "W03X-2Y25"] {
            assert_eq!(raw.norm_input(typed), "W3X-2Y25", "{typed}");
        }
        assert!(KeySpec::parse("w_x_y:2,3", &d).is_err());
        assert_eq!(placeholder("W0X0Y0").as_deref(), Some("all zeros"));
    }

    #[test]
    fn an_id_no_file_has_is_left_out_of_the_report() {
        let d = tempfile::tempdir().unwrap();
        let book = d.path().join("trace.xlsx");
        let one = with_ids(&[("P1", f32::NAN, 3.0, 1.0, 2.0)]);
        let specs: Vec<String> = ["part_txt", "tests:999", "w_x_y:2,3,4"]
            .map(String::from)
            .to_vec();
        let mut runs = [by("one", &one, &["part_txt", "tests:999", "w_x_y:2,3,4"])];
        assert_eq!(drop_unused_ids(&mut runs, &specs), ["tests:999"]);
        export(&runs, &book, "").unwrap();
        let x = part(&book, "xl/worksheets/sheet1.xml");
        assert_eq!(cell(&x, "B2").unwrap(), "W,X,Y");
    }
}
