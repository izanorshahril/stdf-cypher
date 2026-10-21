//! Distribution comparison between two populations of parts. The same engine
//! serves file-vs-file comparison and group-vs-rest within one file.

use super::stats::{self, Describe};
use crate::data::{Dataset, TestKind};
use indexmap::IndexMap;
use rayon::prelude::*;
use serde::Serialize;

/// One side of a comparison: a dataset and the rows that belong to it.
pub struct Side<'a> {
    pub ds: &'a Dataset,
    pub rows: Vec<usize>,
    pub label: String,
}

impl<'a> Side<'a> {
    pub fn all(ds: &'a Dataset, label: impl Into<String>) -> Self {
        Side {
            ds,
            rows: ds.all_rows(),
            label: label.into(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct CompareOptions {
    /// minimum values per side for a test to be compared
    pub min_n: usize,
    /// shift threshold in pooled-sigma units
    pub sigma_threshold: f64,
    /// shift threshold in % of limit range
    pub limit_pct_threshold: f64,
    /// spread ratio threshold (sd_b / sd_a or inverse)
    pub spread_ratio: f64,
    /// q-value (BH-adjusted KS p) significance level
    pub alpha: f64,
    /// match tests by "key" (num+name), "num" or "name"
    pub match_by: String,
}

impl Default for CompareOptions {
    fn default() -> Self {
        Self {
            min_n: 5,
            sigma_threshold: 0.5,
            limit_pct_threshold: 5.0,
            spread_ratio: 1.5,
            alpha: 0.01,
            match_by: "key".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct TestComparison {
    pub key: String,
    pub num: u32,
    pub name: String,
    pub units: String,
    pub lo: Option<f64>,
    pub hi: Option<f64>,
    pub a: Describe,
    pub b: Describe,
    /// mean(B) - mean(A)
    pub mean_shift: f64,
    /// median(B) - median(A)
    pub median_shift: f64,
    /// shift in pooled sigma (Cohen's d)
    pub shift_sigma: f64,
    /// shift as % of limit range
    pub shift_pct_limit: Option<f64>,
    /// sd(B)/sd(A)
    pub sigma_ratio: f64,
    pub t_p: f64,
    pub ks_d: f64,
    pub ks_p: f64,
    pub q: f64,
    pub fail_rate_a: f64,
    pub fail_rate_b: f64,
    /// OK | SHIFT | SPREAD | SHIFT+SPREAD | ONLY_A | ONLY_B
    pub flag: String,
    pub score: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct BinRow {
    pub bin: u16,
    pub name: String,
    pub count_a: usize,
    pub pct_a: f64,
    pub count_b: usize,
    pub pct_b: f64,
    pub delta_pct: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct CompareReport {
    pub label_a: String,
    pub label_b: String,
    pub n_a: usize,
    pub n_b: usize,
    pub yield_a: f64,
    pub yield_b: f64,
    pub tests: Vec<TestComparison>,
    pub hbins: Vec<BinRow>,
    pub sbins: Vec<BinRow>,
    /// (group, yield A, n A, yield B, n B) by site
    pub site_yield: Vec<(String, f64, usize, f64, usize)>,
    pub n_flagged: usize,
}

fn match_key(ds: &Dataset, t: usize, by: &str) -> String {
    let ti = &ds.tests[t];
    match by {
        "num" => ti.num.to_string(),
        "name" => ti.name.clone(),
        _ => ti.key.clone(),
    }
}

fn bins(a: &Side, b: &Side, hard: bool) -> Vec<BinRow> {
    let mut m: IndexMap<u16, (usize, usize)> = IndexMap::new();
    for &r in &a.rows {
        let p = &a.ds.parts[r];
        m.entry(if hard { p.hbin } else { p.sbin }).or_default().0 += 1;
    }
    for &r in &b.rows {
        let p = &b.ds.parts[r];
        m.entry(if hard { p.hbin } else { p.sbin }).or_default().1 += 1;
    }
    let (na, nb) = (a.rows.len().max(1) as f64, b.rows.len().max(1) as f64);
    let mut v: Vec<BinRow> = m
        .into_iter()
        .map(|(bin, (ca, cb))| {
            let name = {
                let n = a.ds.bin_name(hard, bin);
                if n.is_empty() {
                    b.ds.bin_name(hard, bin)
                } else {
                    n
                }
            };
            let (pa, pb) = (ca as f64 * 100.0 / na, cb as f64 * 100.0 / nb);
            BinRow {
                bin,
                name,
                count_a: ca,
                pct_a: pa,
                count_b: cb,
                pct_b: pb,
                delta_pct: pb - pa,
            }
        })
        .collect();
    v.sort_by_key(|r| r.bin);
    v
}

pub fn compare(a: &Side, b: &Side, opt: &CompareOptions) -> CompareReport {
    // align tests
    let mut b_map: IndexMap<String, usize> = IndexMap::new();
    for t in 0..b.ds.n_tests() {
        b_map.entry(match_key(b.ds, t, &opt.match_by)).or_insert(t);
    }
    let mut pairs: Vec<(Option<usize>, Option<usize>)> = vec![];
    let mut used_b = vec![false; b.ds.n_tests()];
    for t in 0..a.ds.n_tests() {
        let k = match_key(a.ds, t, &opt.match_by);
        let bt = b_map.get(&k).copied();
        if let Some(x) = bt {
            used_b[x] = true;
        }
        pairs.push((Some(t), bt));
    }
    for (t, u) in used_b.iter().enumerate() {
        if !u {
            pairs.push((None, Some(t)));
        }
    }
    let mut tests: Vec<TestComparison> = pairs
        .par_iter()
        .filter_map(|&(ta, tb)| {
            let info = ta
                .map(|t| &a.ds.tests[t])
                .or_else(|| tb.map(|t| &b.ds.tests[t]))?;
            let va = ta
                .map(|t| a.ds.values_of(t, Some(&a.rows)))
                .unwrap_or_default();
            let vb = tb
                .map(|t| b.ds.values_of(t, Some(&b.rows)))
                .unwrap_or_default();
            if va.is_empty() && vb.is_empty() {
                return None;
            }
            let (lo, hi) = (info.lo, info.hi);
            let da = stats::describe(&va, lo, hi);
            let db = stats::describe(&vb, lo, hi);
            let enough = va.len() >= opt.min_n && vb.len() >= opt.min_n;
            let mut tc = TestComparison {
                key: info.key.clone(),
                num: info.num,
                name: info.name.clone(),
                units: info.units.clone(),
                lo,
                hi,
                mean_shift: db.mean - da.mean,
                median_shift: db.median - da.median,
                shift_sigma: f64::NAN,
                shift_pct_limit: info.limit_range().map(|r| (db.mean - da.mean) / r * 100.0),
                sigma_ratio: if da.std > 0.0 {
                    db.std / da.std
                } else {
                    f64::NAN
                },
                t_p: f64::NAN,
                ks_d: f64::NAN,
                ks_p: f64::NAN,
                q: f64::NAN,
                fail_rate_a: if da.n > 0 {
                    da.fails as f64 / da.n as f64 * 100.0
                } else {
                    0.0
                },
                fail_rate_b: if db.n > 0 {
                    db.fails as f64 / db.n as f64 * 100.0
                } else {
                    0.0
                },
                flag: String::new(),
                score: 0.0,
                a: da,
                b: db,
            };
            if va.is_empty() {
                tc.flag = "ONLY_B".into();
            } else if vb.is_empty() {
                tc.flag = "ONLY_A".into();
            } else if enough {
                tc.shift_sigma = stats::cohen_d(&va, &vb);
                tc.t_p = stats::welch_t(&va, &vb).p;
                let ks = stats::ks_2samp(&va, &vb);
                tc.ks_d = ks.stat;
                tc.ks_p = ks.p;
            }
            if info.kind == TestKind::Functional {
                tc.shift_pct_limit = None;
            }
            Some(tc)
        })
        .collect();
    // multiple-testing correction
    let ps: Vec<f64> = tests.iter().map(|t| t.ks_p.min(t.t_p)).collect();
    let qs = stats::bh_adjust(&ps);
    for (t, q) in tests.iter_mut().zip(qs) {
        t.q = q;
        if !t.flag.is_empty() {
            t.score = 1e9;
            continue;
        }
        let sig = q.is_finite() && q < opt.alpha;
        let shift_big = t.shift_sigma.abs() >= opt.sigma_threshold
            || t.shift_pct_limit
                .map(|p| p.abs() >= opt.limit_pct_threshold)
                .unwrap_or(false);
        let spread_big = t.sigma_ratio.is_finite()
            && (t.sigma_ratio >= opt.spread_ratio || t.sigma_ratio <= 1.0 / opt.spread_ratio);
        t.flag = match (sig && shift_big, sig && spread_big) {
            (true, true) => "SHIFT+SPREAD",
            (true, false) => "SHIFT",
            (false, true) => "SPREAD",
            _ => "OK",
        }
        .into();
        let s1 = if t.shift_sigma.is_finite() {
            t.shift_sigma.abs()
        } else {
            0.0
        };
        let s2 = t.shift_pct_limit.map(|p| p.abs() / 10.0).unwrap_or(0.0);
        let s3 = if t.sigma_ratio.is_finite() && t.sigma_ratio > 0.0 {
            t.sigma_ratio.ln().abs()
        } else {
            0.0
        };
        let s4 = (t.fail_rate_b - t.fail_rate_a).abs() / 5.0;
        t.score = s1.max(s2) + s3 + s4 + if sig { 0.0 } else { -10.0 };
    }
    let rank = |f: &str| match f {
        "SHIFT+SPREAD" | "SHIFT" | "SPREAD" => 0,
        "ONLY_A" | "ONLY_B" => 1,
        _ => 2,
    };
    tests.sort_by(|x, y| {
        rank(&x.flag).cmp(&rank(&y.flag)).then(
            y.score
                .partial_cmp(&x.score)
                .unwrap_or(std::cmp::Ordering::Equal),
        )
    });
    let n_flagged = tests.iter().filter(|t| t.flag != "OK").count();
    let sa = a.ds.group_by(&a.rows, "site");
    let sb = b.ds.group_by(&b.rows, "site");
    let mut sites: IndexMap<String, (f64, usize, f64, usize)> = IndexMap::new();
    for (k, rows) in sa {
        let e = sites.entry(k).or_insert((f64::NAN, 0, f64::NAN, 0));
        e.0 = a.ds.yield_pct(&rows);
        e.1 = rows.len();
    }
    for (k, rows) in sb {
        let e = sites.entry(k).or_insert((f64::NAN, 0, f64::NAN, 0));
        e.2 = b.ds.yield_pct(&rows);
        e.3 = rows.len();
    }
    CompareReport {
        label_a: a.label.clone(),
        label_b: b.label.clone(),
        n_a: a.rows.len(),
        n_b: b.rows.len(),
        yield_a: a.ds.yield_pct(&a.rows),
        yield_b: b.ds.yield_pct(&b.rows),
        tests,
        hbins: bins(a, b, true),
        sbins: bins(a, b, false),
        site_yield: sites
            .into_iter()
            .map(|(k, v)| (k, v.0, v.1, v.2, v.3))
            .collect(),
        n_flagged,
    }
}
