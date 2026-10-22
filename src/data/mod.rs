//! In-memory analytic model of one or more STDF files:
//! a part table (one row per PRR) and a column-major test matrix.

pub mod filter;
pub mod loader;
pub mod matching;

use indexmap::IndexMap;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

/// Test number holding the loadboard (DIB) ID, read by the `loadboard` field.
static LOADBOARD_TEST: AtomicU32 = AtomicU32::new(5100);

pub fn loadboard_test() -> u32 {
    LOADBOARD_TEST.load(Ordering::Relaxed)
}

pub fn set_loadboard_test(num: u32) {
    LOADBOARD_TEST.store(num, Ordering::Relaxed);
}

/// A test read as a file-level attribute (e.g. the loadboard ID): its most
/// common value in one source file.
#[derive(Debug, Clone, Serialize, Default)]
pub struct FileValue {
    pub value: Option<f64>,
    /// distinct values seen in the file (more than 1 = it changed mid-file)
    pub distinct: usize,
    /// parts of the file without a value
    pub missing: usize,
}

/// Per-test cache of [`FileValue`]s, filled on first use.
#[derive(Debug, Clone, Default)]
pub struct FileValueCache(Arc<Mutex<HashMap<u32, Arc<Vec<FileValue>>>>>);

pub use filter::Filter;
pub use loader::{load, load_many, LoadOptions};

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, Hash)]
pub enum TestKind {
    Parametric,
    MultiPin,
    Functional,
}

#[derive(Debug, Clone, Serialize)]
pub struct TestInfo {
    /// unique key: "<num>:<name>" (MPR pins: "<num>:<name>[<pin>]")
    pub key: String,
    pub num: u32,
    pub name: String,
    pub kind: TestKind,
    pub units: String,
    pub lo: Option<f64>,
    pub hi: Option<f64>,
    pub lo_spec: Option<f64>,
    pub hi_spec: Option<f64>,
    pub res_scal: i8,
    /// executions counted while loading
    pub exec_count: u64,
    /// failures flagged by the tester (TEST_FLG bit 7)
    pub fail_count: u64,
}

impl TestInfo {
    pub fn label(&self) -> String {
        if self.units.is_empty() {
            format!("{} {}", self.num, self.name)
        } else {
            format!("{} {} [{}]", self.num, self.name, self.units)
        }
    }
    /// Limit range used to normalize shifts (hi - lo), when both limits exist.
    pub fn limit_range(&self) -> Option<f64> {
        match (self.lo, self.hi) {
            (Some(l), Some(h)) if h > l => Some(h - l),
            _ => None,
        }
    }
    pub fn within_limits(&self, v: f64) -> bool {
        self.lo.map(|l| v >= l).unwrap_or(true) && self.hi.map(|h| v <= h).unwrap_or(true)
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Part {
    /// source file index within the dataset
    pub file: u16,
    pub part_id: String,
    pub head: u8,
    pub site: u8,
    pub x: i16,
    pub y: i16,
    pub hbin: u16,
    pub sbin: u16,
    pub passed: Option<bool>,
    pub part_flg: u8,
    pub num_test: u16,
    pub test_t: u32,
    /// index into Dataset::wafers, or u32::MAX
    pub wafer: u32,
    pub part_txt: String,
    /// first failing test (index into Dataset::tests) if any
    pub first_fail: Option<u32>,
    /// tests whose result carried the alarm flag (TEST_FLG bit 0)
    pub alarms: Vec<u32>,
    /// tests the tester flagged as failed for this part
    pub fails: Vec<u32>,
}

impl Part {
    pub fn is_pass(&self) -> bool {
        self.passed.unwrap_or(false)
    }
    pub fn has_alarm(&self, t: usize) -> bool {
        self.alarms.contains(&(t as u32))
    }
    pub fn has_fail(&self, t: usize) -> bool {
        self.fails.contains(&(t as u32))
    }
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct SourceInfo {
    pub path: String,
    pub endian: String,
    pub lot_id: String,
    pub sublot_id: String,
    pub part_typ: String,
    pub job_nam: String,
    pub job_rev: String,
    pub node_nam: String,
    pub tstr_typ: String,
    pub test_cod: String,
    pub tst_temp: String,
    pub flow_id: String,
    pub start_t: u32,
    pub finish_t: u32,
    pub record_counts: IndexMap<String, u64>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct BinDef {
    pub name: String,
    /// 'P', 'F' or ' '
    pub pf: char,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct Dataset {
    pub name: String,
    pub sources: Vec<SourceInfo>,
    pub parts: Vec<Part>,
    pub tests: Vec<TestInfo>,
    #[serde(skip)]
    pub test_index: HashMap<String, usize>,
    /// column-major: values[test][part]; NaN = not tested / invalid
    #[serde(skip)]
    pub values: Vec<Vec<f32>>,
    pub wafers: Vec<String>,
    pub hbins: IndexMap<u16, BinDef>,
    pub sbins: IndexMap<u16, BinDef>,
    #[serde(skip)]
    pub file_vals: FileValueCache,
}

/// A value of a part attribute, used by filters and grouping.
#[derive(Debug, Clone, PartialEq)]
pub enum FieldValue {
    Num(f64),
    Str(String),
    Bool(bool),
    Missing,
}

impl FieldValue {
    pub fn as_group_key(&self) -> String {
        match self {
            FieldValue::Num(v) => {
                if v.fract() == 0.0 {
                    format!("{}", *v as i64)
                } else {
                    format!("{v}")
                }
            }
            FieldValue::Str(s) => s.clone(),
            FieldValue::Bool(b) => (if *b { "pass" } else { "fail" }).into(),
            FieldValue::Missing => "NA".into(),
        }
    }
}

pub const PART_FIELDS: &[&str] = &[
    "part_id",
    "file",
    "head",
    "site",
    "x",
    "y",
    "hbin",
    "sbin",
    "pass",
    "wafer",
    "lot",
    "num_test",
    "test_t",
    "part_txt",
    "index",
    "xy",
    "retest",
    "alarm",
    "first_fail",
    "n_fails",
    "loadboard",
    "site_loadboard",
];

impl Dataset {
    pub fn n_parts(&self) -> usize {
        self.parts.len()
    }
    pub fn n_tests(&self) -> usize {
        self.tests.len()
    }

    pub fn column(&self, t: usize) -> &[f32] {
        &self.values[t]
    }

    /// Finite values of a test over the given rows (all rows when `rows` is None).
    pub fn values_of(&self, t: usize, rows: Option<&[usize]>) -> Vec<f64> {
        let col = &self.values[t];
        match rows {
            Some(r) => r
                .iter()
                .map(|&i| col[i] as f64)
                .filter(|v| v.is_finite())
                .collect(),
            None => col
                .iter()
                .map(|&v| v as f64)
                .filter(|v| v.is_finite())
                .collect(),
        }
    }

    pub fn value(&self, t: usize, part: usize) -> f64 {
        self.values[t][part] as f64
    }

    pub fn all_rows(&self) -> Vec<usize> {
        (0..self.parts.len()).collect()
    }

    /// Resolve a user-provided test reference: exact key, test number, exact
    /// name, or case-insensitive substring of the name (if unique).
    pub fn find_test(&self, q: &str) -> Option<usize> {
        if let Some(&i) = self.test_index.get(q) {
            return Some(i);
        }
        if let Ok(n) = q.trim().parse::<u32>() {
            if let Some(i) = self.tests.iter().position(|t| t.num == n) {
                return Some(i);
            }
        }
        if let Some(i) = self.tests.iter().position(|t| t.name == q) {
            return Some(i);
        }
        let ql = q.to_lowercase();
        let hits: Vec<usize> = self
            .tests
            .iter()
            .enumerate()
            .filter(|(_, t)| t.name.to_lowercase().contains(&ql))
            .map(|(i, _)| i)
            .collect();
        if hits.len() == 1 {
            Some(hits[0])
        } else {
            None
        }
    }

    pub fn find_tests(&self, q: &str) -> Vec<usize> {
        if let Some(i) = self.find_test(q) {
            return vec![i];
        }
        let ql = q.to_lowercase();
        self.tests
            .iter()
            .enumerate()
            .filter(|(_, t)| t.key.to_lowercase().contains(&ql))
            .map(|(i, _)| i)
            .collect()
    }

    /// Indices of tests that have numeric data worth analysing.
    pub fn numeric_tests(&self) -> Vec<usize> {
        (0..self.tests.len())
            .filter(|&t| {
                self.tests[t].kind != TestKind::Functional
                    && self.values[t].iter().any(|v| v.is_finite())
            })
            .collect()
    }

    pub fn wafer_name(&self, p: &Part) -> &str {
        self.wafers
            .get(p.wafer as usize)
            .map(|s| s.as_str())
            .unwrap_or("")
    }

    pub fn field(&self, row: usize, field: &str) -> FieldValue {
        let p = &self.parts[row];
        match field {
            "part_id" | "partid" | "id" => FieldValue::Str(p.part_id.clone()),
            "file" => FieldValue::Num(p.file as f64),
            "head" => FieldValue::Num(p.head as f64),
            "site" => FieldValue::Num(p.site as f64),
            "x" => FieldValue::Num(p.x as f64),
            "y" => FieldValue::Num(p.y as f64),
            "xy" => FieldValue::Str(format!("{},{}", p.x, p.y)),
            "hbin" | "hard_bin" => FieldValue::Num(p.hbin as f64),
            "sbin" | "soft_bin" => FieldValue::Num(p.sbin as f64),
            "pass" | "passed" => p
                .passed
                .map(FieldValue::Bool)
                .unwrap_or(FieldValue::Missing),
            "fail" | "failed" => p
                .passed
                .map(|b| FieldValue::Bool(!b))
                .unwrap_or(FieldValue::Missing),
            "wafer" | "wafer_id" => FieldValue::Str(self.wafer_name(p).to_string()),
            "lot" | "lot_id" => FieldValue::Str(
                self.sources
                    .get(p.file as usize)
                    .map(|s| s.lot_id.clone())
                    .unwrap_or_default(),
            ),
            "sublot" => FieldValue::Str(
                self.sources
                    .get(p.file as usize)
                    .map(|s| s.sublot_id.clone())
                    .unwrap_or_default(),
            ),
            "num_test" => FieldValue::Num(p.num_test as f64),
            "test_t" => FieldValue::Num(p.test_t as f64),
            "part_txt" => FieldValue::Str(p.part_txt.clone()),
            "index" | "row" => FieldValue::Num(row as f64),
            "retest" => FieldValue::Bool(p.part_flg & 0x03 != 0),
            "alarm" => FieldValue::Bool(!p.alarms.is_empty()),
            "first_fail" => p
                .first_fail
                .map(|f| FieldValue::Num(self.tests[f as usize].num as f64))
                .unwrap_or(FieldValue::Missing),
            "n_fails" => FieldValue::Num(p.fails.len() as f64),
            "loadboard" | "lb" | "dib" => self
                .file_value(row, loadboard_test())
                .map(FieldValue::Num)
                .unwrap_or(FieldValue::Missing),
            "site_loadboard" => match self.file_value(row, loadboard_test()) {
                Some(b) => FieldValue::Str(format!(
                    "site {} / LB {}",
                    p.site,
                    FieldValue::Num(b).as_group_key()
                )),
                None => FieldValue::Missing,
            },
            // any test as a factor: `test:5100`
            f => match f
                .strip_prefix("test:")
                .and_then(|n| n.trim().parse::<u32>().ok())
            {
                Some(num) => self
                    .tests
                    .iter()
                    .position(|t| t.num == num)
                    .map(|t| self.value(t, row))
                    .filter(|v| v.is_finite())
                    .map(FieldValue::Num)
                    .unwrap_or(FieldValue::Missing),
                None => FieldValue::Missing,
            },
        }
    }

    /// Test `num` read as a file-level attribute: its most common value in
    /// each source file (parts without a value take it too), cached.
    pub fn file_values(&self, num: u32) -> Arc<Vec<FileValue>> {
        if let Some(v) = self.file_vals.0.lock().unwrap().get(&num) {
            return v.clone();
        }
        let mut counts: Vec<IndexMap<String, (usize, f64)>> =
            vec![IndexMap::new(); self.sources.len().max(1)];
        let mut missing = vec![0usize; counts.len()];
        let t = self.tests.iter().position(|t| t.num == num);
        for (r, p) in self.parts.iter().enumerate() {
            let f = (p.file as usize).min(counts.len() - 1);
            match t.map(|t| self.value(t, r)).filter(|v| v.is_finite()) {
                Some(v) => {
                    let e = counts[f]
                        .entry(FieldValue::Num(v).as_group_key())
                        .or_insert((0, v));
                    e.0 += 1;
                }
                None => missing[f] += 1,
            }
        }
        let out: Arc<Vec<FileValue>> = Arc::new(
            counts
                .iter()
                .zip(missing)
                .map(|(c, missing)| FileValue {
                    value: c.values().max_by_key(|e| e.0).map(|e| e.1),
                    distinct: c.len(),
                    missing,
                })
                .collect(),
        );
        self.file_vals.0.lock().unwrap().insert(num, out.clone());
        out
    }

    /// The file-level value of test `num` for the file of `row`.
    pub fn file_value(&self, row: usize, num: u32) -> Option<f64> {
        let f = self.parts[row].file as usize;
        self.file_values(num).get(f).and_then(|v| v.value)
    }

    /// Group rows by a part field (or the special "all"). Groups are sorted
    /// numerically when keys are numbers.
    pub fn group_by(&self, rows: &[usize], field: &str) -> Vec<(String, Vec<usize>)> {
        let mut map: IndexMap<String, Vec<usize>> = IndexMap::new();
        if field.is_empty() || field == "all" || field == "none" {
            return vec![("all".into(), rows.to_vec())];
        }
        for &r in rows {
            let k = if field == "file" {
                let f = self.parts[r].file as usize;
                self.sources
                    .get(f)
                    .map(|s| short_name(&s.path))
                    .unwrap_or_else(|| f.to_string())
            } else {
                self.field(r, field).as_group_key()
            };
            map.entry(k).or_default().push(r);
        }
        let mut v: Vec<(String, Vec<usize>)> = map.into_iter().collect();
        if v.iter().all(|(k, _)| k.parse::<f64>().is_ok()) {
            v.sort_by(|a, b| {
                a.0.parse::<f64>()
                    .unwrap()
                    .partial_cmp(&b.0.parse::<f64>().unwrap())
                    .unwrap()
            });
        } else if field != "file" {
            v.sort_by(|a, b| a.0.cmp(&b.0));
        }
        v
    }

    pub fn yield_pct(&self, rows: &[usize]) -> f64 {
        if rows.is_empty() {
            return 0.0;
        }
        rows.iter().filter(|&&r| self.parts[r].is_pass()).count() as f64 * 100.0 / rows.len() as f64
    }

    /// New dataset containing only `rows` (in that order).
    pub fn subset(&self, rows: &[usize]) -> Dataset {
        let mut d = Dataset {
            name: self.name.clone(),
            sources: self.sources.clone(),
            parts: rows.iter().map(|&r| self.parts[r].clone()).collect(),
            tests: self.tests.clone(),
            test_index: self.test_index.clone(),
            values: self
                .values
                .iter()
                .map(|c| rows.iter().map(|&r| c[r]).collect())
                .collect(),
            wafers: self.wafers.clone(),
            hbins: self.hbins.clone(),
            sbins: self.sbins.clone(),
            file_vals: Default::default(),
        };
        d.recount_tests();
        d
    }

    fn recount_tests(&mut self) {
        for (t, info) in self.tests.iter_mut().enumerate() {
            let col = &self.values[t];
            info.exec_count = col.iter().filter(|v| v.is_finite()).count() as u64;
            info.fail_count = col
                .iter()
                .filter(|v| v.is_finite() && !info.within_limits(**v as f64))
                .count() as u64;
        }
    }

    /// Unique physical unit identity used for retest de-duplication.
    pub fn unit_key(&self, row: usize) -> String {
        let p = &self.parts[row];
        if p.x != -32768 && p.y != -32768 {
            format!(
                "{}|{}|{},{}",
                self.sources
                    .get(p.file as usize)
                    .map(|s| s.lot_id.as_str())
                    .unwrap_or(""),
                self.wafer_name(p),
                p.x,
                p.y
            )
        } else {
            format!("{}|{}", p.file, p.part_id)
        }
    }

    /// Keep only the first or last test of each physical unit.
    pub fn dedup(&self, keep_last: bool) -> Dataset {
        let mut seen: HashMap<String, usize> = HashMap::new();
        for r in 0..self.parts.len() {
            let k = self.unit_key(r);
            if keep_last {
                seen.insert(k, r);
            } else {
                seen.entry(k).or_insert(r);
            }
        }
        let mut rows: Vec<usize> = seen.into_values().collect();
        rows.sort_unstable();
        self.subset(&rows)
    }

    /// Merge datasets into one; tests are aligned by key and each part keeps
    /// its source file index (useful for lot-to-lot ANOVA and overlays).
    pub fn merge(sets: Vec<Dataset>) -> Dataset {
        let mut out = Dataset {
            name: sets
                .iter()
                .map(|d| d.name.clone())
                .collect::<Vec<_>>()
                .join(" + "),
            ..Default::default()
        };
        for d in sets {
            let file_off = out.sources.len() as u16;
            let wafer_off = out.wafers.len() as u32;
            let n_before = out.parts.len();
            let tmap: Vec<usize> = d
                .tests
                .iter()
                .map(|t| {
                    if let Some(&i) = out.test_index.get(&t.key) {
                        i
                    } else {
                        let i = out.tests.len();
                        out.tests.push(t.clone());
                        out.test_index.insert(t.key.clone(), i);
                        out.values.push(vec![f32::NAN; n_before]);
                        i
                    }
                })
                .collect();
            for mut p in d.parts {
                p.file += file_off;
                if p.wafer != u32::MAX {
                    p.wafer += wafer_off;
                }
                p.first_fail = p.first_fail.map(|f| tmap[f as usize] as u32);
                p.alarms
                    .iter_mut()
                    .for_each(|a| *a = tmap[*a as usize] as u32);
                p.fails
                    .iter_mut()
                    .for_each(|a| *a = tmap[*a as usize] as u32);
                out.parts.push(p);
            }
            let n_after = out.parts.len();
            for col in out.values.iter_mut() {
                col.resize(n_after, f32::NAN);
            }
            for (src_t, col) in d.values.into_iter().enumerate() {
                let dst = &mut out.values[tmap[src_t]];
                dst[n_before..n_after].copy_from_slice(&col);
            }
            out.sources.extend(d.sources);
            out.wafers.extend(d.wafers);
            for (k, v) in d.hbins {
                out.hbins.entry(k).or_insert(v);
            }
            for (k, v) in d.sbins {
                out.sbins.entry(k).or_insert(v);
            }
        }
        out.recount_tests();
        out
    }

    pub fn bin_name(&self, hard: bool, bin: u16) -> String {
        let m = if hard { &self.hbins } else { &self.sbins };
        m.get(&bin)
            .map(|b| b.name.clone())
            .filter(|s| !s.is_empty())
            .unwrap_or_default()
    }
}

pub fn short_name(path: &str) -> String {
    std::path::Path::new(path)
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| path.to_string())
}

/// Paths to open: files as given, folders searched (with subfolders) for
/// files that read as STDF (plain or gzip; other files are skipped and
/// counted). Duplicates are dropped, first one kept.
pub fn expand_paths(paths: &[String]) -> (Vec<String>, usize) {
    fn walk(dir: &std::path::Path, out: &mut Vec<String>, skipped: &mut usize, depth: usize) {
        let Ok(rd) = std::fs::read_dir(dir) else {
            return;
        };
        let mut entries: Vec<std::path::PathBuf> = rd.flatten().map(|e| e.path()).collect();
        entries.sort();
        for p in entries {
            if p.is_dir() {
                if depth < 8 {
                    walk(&p, out, skipped, depth + 1);
                }
            } else if crate::stdf::StdfReader::open(&p).is_ok() {
                out.push(p.to_string_lossy().to_string());
            } else {
                *skipped += 1;
            }
        }
    }
    let (mut out, mut skipped) = (vec![], 0);
    for p in paths {
        let pb = std::path::Path::new(p);
        if pb.is_dir() {
            // canonical, so the same file always gets the same path (recent list)
            let c = std::fs::canonicalize(pb).unwrap_or_else(|_| pb.to_path_buf());
            let c = c.to_string_lossy();
            walk(
                std::path::Path::new(c.strip_prefix(r"\\?\").unwrap_or(&c)),
                &mut out,
                &mut skipped,
                0,
            );
        } else {
            out.push(p.clone());
        }
    }
    let mut seen = std::collections::HashSet::new();
    out.retain(|p| seen.insert(p.clone()));
    (out, skipped)
}

/// Order for opened files: `time` = test start time (MIR START_T, else the
/// file's modified time), `name` = file name (natural: run2 before run10);
/// anything else keeps the given order.
pub fn sort_opened<T>(
    items: &mut [T],
    how: &str,
    path: impl Fn(&T) -> &str,
    start_t: impl Fn(&T) -> Option<u32>,
) {
    let mtime = |p: &str| {
        std::fs::metadata(p)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0)
    };
    match how {
        "time" => items.sort_by_cached_key(|x| {
            (
                start_t(x)
                    .filter(|&t| t > 0)
                    .map(u64::from)
                    .unwrap_or_else(|| mtime(path(x))),
                natural_key(&short_name(path(x))),
            )
        }),
        "name" => items.sort_by_cached_key(|x| natural_key(&short_name(path(x)))),
        _ => {}
    }
}

/// Sort key that compares digit runs by value ("run2" < "run10"), ignoring case.
fn natural_key(s: &str) -> Vec<(u8, u64, String)> {
    let mut out = vec![];
    let mut cur = String::new();
    let mut digits = false;
    let push = |out: &mut Vec<(u8, u64, String)>, cur: &mut String, digits: bool| {
        if !cur.is_empty() {
            out.push(if digits {
                (0, cur.parse().unwrap_or(u64::MAX), String::new())
            } else {
                (1, 0, cur.to_lowercase())
            });
            cur.clear();
        }
    };
    for c in s.chars() {
        if c.is_ascii_digit() != digits {
            push(&mut out, &mut cur, digits);
            digits = c.is_ascii_digit();
        }
        cur.push(c);
    }
    push(&mut out, &mut cur, digits);
    out
}

#[cfg(test)]
mod open_tests {
    #[test]
    fn natural_name_order() {
        let mut v = vec!["run10.stdf", "Run2.stdf", "run1.stdf"];
        super::sort_opened(&mut v, "name", |x| x, |_| None);
        assert_eq!(v, ["run1.stdf", "Run2.stdf", "run10.stdf"]);
        let mut v = vec![("b", 20u32), ("a", 30), ("c", 10)];
        super::sort_opened(&mut v, "time", |x| x.0, |x| Some(x.1));
        assert_eq!(v.iter().map(|x| x.0).collect::<String>(), "cba");
    }
}
