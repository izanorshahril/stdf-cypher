//! Match parts between two datasets by a shared identity, and trace single
//! parts by that identity.
//!
//! An identity ("key") can come from the PRR (PART_ID, PART_TXT, die X/Y,
//! wafer + X/Y) or from datalogged PTR results: one test that holds a unique
//! per-part code (e.g. an encoded wafer + X/Y), or several tests whose values
//! together form the key (e.g. wafer, X and Y logged in tests 501, 502, 503).
//! Each side of a match has its own key, so test numbers may differ between
//! the two files.

use super::Dataset;
use anyhow::{anyhow, bail, Result};
use serde::Serialize;
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub enum KeySpec {
    PartId,
    PartTxt,
    Xy,
    WaferXy,
    Tests(Vec<usize>),
}

/// Canonical text for a number, so 12, 12.0 and "012" compare equal.
pub fn num_key(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e15 {
        format!("{}", v as i64)
    } else {
        format!("{v}")
    }
}

/// Canonical text for a key component typed by a user or read from a record.
pub fn norm(s: &str) -> String {
    let t = s.trim();
    match t.parse::<f64>() {
        Ok(v) if v.is_finite() => num_key(v),
        _ => t.to_string(),
    }
}

/// Canonical text for a whole key ("1 | 10 | 12" -> "1|10|12").
pub fn norm_key(s: &str) -> String {
    s.split(['|', ',', ';'])
        .map(norm)
        .collect::<Vec<_>>()
        .join("|")
}

impl KeySpec {
    /// `part_id`, `part_txt`, `xy`, `wafer_xy`, or a test list such as
    /// `tests:500` / `tests:501,502,503` (the `tests:` prefix is optional).
    pub fn parse(s: &str, ds: &Dataset) -> Result<KeySpec> {
        let s = s.trim();
        Ok(match s.to_lowercase().as_str() {
            "part_id" | "partid" | "id" => KeySpec::PartId,
            "part_txt" | "txt" => KeySpec::PartTxt,
            "xy" => KeySpec::Xy,
            "wafer_xy" | "wafer+xy" => KeySpec::WaferXy,
            "" => bail!("choose how parts are identified"),
            _ => {
                let list = s
                    .strip_prefix("tests:")
                    .or_else(|| s.strip_prefix("test:"))
                    .unwrap_or(s);
                let tests = list
                    .split([',', ';', '+'])
                    .map(|q| q.trim())
                    .filter(|q| !q.is_empty())
                    .map(|q| {
                        ds.find_test(q)
                            .ok_or_else(|| anyhow!("test '{q}' not found in {}", ds.name))
                    })
                    .collect::<Result<Vec<_>>>()?;
                if tests.is_empty() {
                    bail!("list one or more test numbers for the key");
                }
                KeySpec::Tests(tests)
            }
        })
    }

    pub fn describe(&self, ds: &Dataset) -> String {
        match self {
            KeySpec::PartId => "PRR part ID".into(),
            KeySpec::PartTxt => "PRR part text".into(),
            KeySpec::Xy => "PRR X/Y".into(),
            KeySpec::WaferXy => "wafer + PRR X/Y".into(),
            KeySpec::Tests(t) => format!(
                "PTR {}",
                t.iter()
                    .map(|&i| ds.tests[i].num.to_string())
                    .collect::<Vec<_>>()
                    .join(" + ")
            ),
        }
    }

    /// The part's key, or None when the part has no usable identity.
    pub fn key(&self, ds: &Dataset, row: usize) -> Option<String> {
        let p = &ds.parts[row];
        let s = match self {
            KeySpec::PartId => norm(&p.part_id),
            KeySpec::PartTxt => norm(&p.part_txt),
            KeySpec::Xy | KeySpec::WaferXy => {
                if p.x == -32768 || p.y == -32768 {
                    return None;
                }
                let xy = format!("{}|{}", p.x, p.y);
                if matches!(self, KeySpec::WaferXy) {
                    format!("{}|{xy}", norm(ds.wafer_name(p)))
                } else {
                    xy
                }
            }
            KeySpec::Tests(ts) => {
                let mut parts = Vec::with_capacity(ts.len());
                for &t in ts {
                    let v = ds.value(t, row);
                    if !v.is_finite() {
                        return None;
                    }
                    parts.push(num_key(v));
                }
                parts.join("|")
            }
        };
        (!s.is_empty()).then_some(s)
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct MatchResult {
    pub key_a: String,
    pub key_b: String,
    /// one pair per shared key: the last test of that unit on each side
    pub pairs: Vec<(usize, usize, String)>,
    /// every row (retests included) whose key is present on the other side
    #[serde(skip)]
    pub in_a: Vec<bool>,
    #[serde(skip)]
    pub in_b: Vec<bool>,
    pub matched_a: usize,
    pub matched_b: usize,
    pub only_a: usize,
    pub only_b: usize,
    pub no_key_a: usize,
    pub no_key_b: usize,
    /// rows that share their key with an earlier row of the same side (retests)
    pub dup_a: usize,
    pub dup_b: usize,
}

fn index(ds: &Dataset, rows: &[usize], k: &KeySpec) -> (HashMap<String, Vec<usize>>, usize) {
    let mut m: HashMap<String, Vec<usize>> = HashMap::new();
    let mut none = 0;
    for &r in rows {
        match k.key(ds, r) {
            Some(key) => m.entry(key).or_default().push(r),
            None => none += 1,
        }
    }
    (m, none)
}

pub fn match_parts(
    da: &Dataset,
    ra: &[usize],
    ka: &KeySpec,
    db: &Dataset,
    rb: &[usize],
    kb: &KeySpec,
) -> MatchResult {
    let (ia, no_key_a) = index(da, ra, ka);
    let (ib, no_key_b) = index(db, rb, kb);
    let mut in_a = vec![false; da.n_parts()];
    let mut in_b = vec![false; db.n_parts()];
    let mut pairs = vec![];
    for (key, rows_a) in &ia {
        if let Some(rows_b) = ib.get(key) {
            rows_a.iter().for_each(|&r| in_a[r] = true);
            rows_b.iter().for_each(|&r| in_b[r] = true);
            pairs.push((
                *rows_a.last().unwrap(),
                *rows_b.last().unwrap(),
                key.clone(),
            ));
        }
    }
    pairs.sort_unstable_by_key(|p| p.0);
    let matched_a = ra.iter().filter(|&&r| in_a[r]).count();
    let matched_b = rb.iter().filter(|&&r| in_b[r]).count();
    let dups = |m: &HashMap<String, Vec<usize>>| m.values().map(|v| v.len() - 1).sum();
    MatchResult {
        key_a: ka.describe(da),
        key_b: kb.describe(db),
        matched_a,
        matched_b,
        only_a: ra.len() - no_key_a - matched_a,
        only_b: rb.len() - no_key_b - matched_b,
        no_key_a,
        no_key_b,
        dup_a: dups(&ia),
        dup_b: dups(&ib),
        pairs,
        in_a,
        in_b,
    }
}

/// Rows of `ds` whose key equals `value` (normalized), in test order.
pub fn find(ds: &Dataset, k: &KeySpec, value: &str) -> Vec<usize> {
    let want = norm_key(value);
    (0..ds.n_parts())
        .filter(|&r| k.key(ds, r).as_deref() == Some(want.as_str()))
        .collect()
}
