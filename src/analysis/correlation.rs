//! Correlation analyses.
//!
//! * **Unit correlation** (tester-to-tester / run-to-run): the same physical
//!   parts measured in two files are matched by part ID or wafer X/Y; every
//!   test is classified as CORRELATED, OFFSET or UNCORRELATED.
//! * **Test-to-test correlation** within one population (Pearson/Spearman
//!   matrix, strongest and redundant pairs).
//! * **Target correlation**: rank tests by their correlation with a target
//!   test or with pass/fail.

use super::stats;
use crate::data::{Dataset, TestKind};
use indexmap::IndexMap;
use rayon::prelude::*;
use serde::Serialize;
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct UnitCorrOptions {
    /// "part_id", "xy" (wafer+x+y) or "auto"
    pub match_by: String,
    /// minimum r for CORRELATED
    pub r_min: f64,
    /// max |mean diff| as % of limit range (or in sigma of A when no limits)
    pub offset_pct: f64,
    pub min_n: usize,
}

impl Default for UnitCorrOptions {
    fn default() -> Self {
        Self {
            match_by: "auto".into(),
            r_min: 0.9,
            offset_pct: 5.0,
            min_n: 5,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct UnitCorrTest {
    pub key: String,
    pub units: String,
    pub n: usize,
    pub r: f64,
    pub r2: f64,
    pub slope: f64,
    pub intercept: f64,
    pub mean_a: f64,
    pub mean_b: f64,
    /// mean(B - A)
    pub mean_diff: f64,
    pub sd_diff: f64,
    pub max_abs_diff: f64,
    pub diff_pct_limit: Option<f64>,
    /// within-run repeatability proxy: sd(A)
    pub sd_a: f64,
    /// CORRELATED | OFFSET | UNCORRELATED | LOW_VARIANCE | INSUFFICIENT
    pub status: String,
    #[serde(skip)]
    pub pairs: Vec<(f64, f64)>,
}

#[derive(Debug, Clone, Serialize)]
pub struct UnitCorrReport {
    pub match_by: String,
    pub matched: usize,
    pub only_a: usize,
    pub only_b: usize,
    pub bin_agreement_pct: f64,
    /// (hbin A, hbin B, count)
    pub bin_matrix: Vec<(u16, u16, usize)>,
    pub tests: Vec<UnitCorrTest>,
    pub summary: IndexMap<String, usize>,
    #[serde(skip)]
    pub pairs: Vec<(usize, usize)>,
}

fn unit_key(ds: &Dataset, r: usize, by: &str) -> String {
    let p = &ds.parts[r];
    match by {
        "xy" => format!("{}|{},{}", ds.wafer_name(p), p.x, p.y),
        _ => p.part_id.clone(),
    }
}

pub fn unit_correlation(
    a: &Dataset,
    rows_a: &[usize],
    b: &Dataset,
    rows_b: &[usize],
    opt: &UnitCorrOptions,
) -> UnitCorrReport {
    let by = if opt.match_by == "auto" {
        let has_xy = rows_a.iter().take(50).any(|&r| a.parts[r].x != -32768);
        if has_xy {
            "xy"
        } else {
            "part_id"
        }
    } else {
        opt.match_by.as_str()
    };
    // last occurrence wins (final retest)
    let mut ma: HashMap<String, usize> = HashMap::new();
    for &r in rows_a {
        ma.insert(unit_key(a, r, by), r);
    }
    let mut mb: HashMap<String, usize> = HashMap::new();
    for &r in rows_b {
        mb.insert(unit_key(b, r, by), r);
    }
    let mut pairs: Vec<(usize, usize)> = ma
        .iter()
        .filter_map(|(k, &ra)| mb.get(k).map(|&rb| (ra, rb)))
        .collect();
    pairs.sort_unstable();
    let only_a = ma.len() - pairs.len();
    let only_b = mb.len() - pairs.len();
    let mut bm: IndexMap<(u16, u16), usize> = IndexMap::new();
    let mut agree = 0;
    for &(ra, rb) in &pairs {
        let (ha, hb) = (a.parts[ra].hbin, b.parts[rb].hbin);
        *bm.entry((ha, hb)).or_default() += 1;
        if ha == hb {
            agree += 1;
        }
    }
    let tests: Vec<UnitCorrTest> = (0..a.n_tests())
        .into_par_iter()
        .filter_map(|ta| {
            let info = &a.tests[ta];
            if info.kind == TestKind::Functional {
                return None;
            }
            let tb = *b.test_index.get(&info.key)?;
            let pr: Vec<(f64, f64)> = pairs
                .iter()
                .map(|&(ra, rb)| (a.value(ta, ra), b.value(tb, rb)))
                .filter(|(x, y)| x.is_finite() && y.is_finite())
                .collect();
            let xs: Vec<f64> = pr.iter().map(|p| p.0).collect();
            let ys: Vec<f64> = pr.iter().map(|p| p.1).collect();
            let diffs: Vec<f64> = pr.iter().map(|p| p.1 - p.0).collect();
            let n = pr.len();
            let r = stats::pearson(&xs, &ys);
            let (ic, sl, r2) = if n >= 3 {
                stats::linreg(&xs, &ys)
            } else {
                (f64::NAN, f64::NAN, f64::NAN)
            };
            let md = stats::mean(&diffs);
            let sd_a = stats::std(&xs);
            let diff_pct = info.limit_range().map(|lr| md / lr * 100.0);
            let offset_bad = match diff_pct {
                Some(p) => p.abs() > opt.offset_pct,
                None => sd_a > 0.0 && (md / sd_a).abs() > 1.0,
            };
            // when the population spread is tiny relative to limits, r is meaningless
            let low_var = info
                .limit_range()
                .map(|lr| sd_a * 6.0 < lr * 0.02)
                .unwrap_or(false);
            let status = if n < opt.min_n {
                "INSUFFICIENT"
            } else if r.is_finite() && r >= opt.r_min {
                if offset_bad {
                    "OFFSET"
                } else {
                    "CORRELATED"
                }
            } else if low_var
                && !offset_bad
                && diff_pct
                    .map(|_| {
                        stats::std(&diffs) / (info.limit_range().unwrap()) * 100.0 < opt.offset_pct
                    })
                    .unwrap_or(false)
            {
                "LOW_VARIANCE"
            } else {
                "UNCORRELATED"
            };
            Some(UnitCorrTest {
                key: info.key.clone(),
                units: info.units.clone(),
                n,
                r,
                r2,
                slope: sl,
                intercept: ic,
                mean_a: stats::mean(&xs),
                mean_b: stats::mean(&ys),
                mean_diff: md,
                sd_diff: stats::std(&diffs),
                max_abs_diff: diffs.iter().fold(0.0f64, |m, d| m.max(d.abs())),
                diff_pct_limit: diff_pct,
                sd_a,
                status: status.into(),
                pairs: pr,
            })
        })
        .collect();
    let mut tests = tests;
    let order = |s: &str| match s {
        "UNCORRELATED" => 0,
        "OFFSET" => 1,
        "LOW_VARIANCE" => 2,
        "CORRELATED" => 3,
        _ => 4,
    };
    tests.sort_by(|x, y| {
        order(&x.status)
            .cmp(&order(&y.status))
            .then(x.r.partial_cmp(&y.r).unwrap_or(std::cmp::Ordering::Equal))
    });
    let mut summary: IndexMap<String, usize> = IndexMap::new();
    for s in [
        "CORRELATED",
        "OFFSET",
        "UNCORRELATED",
        "LOW_VARIANCE",
        "INSUFFICIENT",
    ] {
        summary.insert(s.into(), 0);
    }
    for t in &tests {
        *summary.entry(t.status.clone()).or_default() += 1;
    }
    let mut bin_matrix: Vec<(u16, u16, usize)> =
        bm.into_iter().map(|((x, y), c)| (x, y, c)).collect();
    bin_matrix.sort();
    UnitCorrReport {
        match_by: by.into(),
        matched: pairs.len(),
        only_a,
        only_b,
        bin_agreement_pct: if pairs.is_empty() {
            0.0
        } else {
            agree as f64 * 100.0 / pairs.len() as f64
        },
        bin_matrix,
        tests,
        summary,
        pairs,
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct CorrMatrix {
    pub method: String,
    pub tests: Vec<usize>,
    pub names: Vec<String>,
    /// row-major k×k
    pub r: Vec<Vec<f64>>,
    /// strongest pairs |r| >= threshold, sorted
    pub pairs: Vec<(String, String, f64, usize)>,
}

/// Pairwise-complete correlation matrix among `tests` over `rows`.
pub fn matrix(
    ds: &Dataset,
    rows: &[usize],
    tests: &[usize],
    method: &str,
    threshold: f64,
) -> CorrMatrix {
    let k = tests.len();
    let spearman = method.eq_ignore_ascii_case("spearman");
    // gather columns
    let cols: Vec<Vec<f64>> = tests
        .par_iter()
        .map(|&t| rows.iter().map(|&r| ds.values[t][r] as f64).collect())
        .collect();
    // rank-transform once for spearman (NaN stays NaN)
    let cols: Vec<Vec<f64>> = if spearman {
        cols.into_par_iter()
            .map(|c| {
                let idx: Vec<usize> = (0..c.len()).filter(|&i| c[i].is_finite()).collect();
                let vals: Vec<f64> = idx.iter().map(|&i| c[i]).collect();
                let rk = stats::ranks(&vals);
                let mut out = vec![f64::NAN; c.len()];
                for (j, &i) in idx.iter().enumerate() {
                    out[i] = rk[j];
                }
                out
            })
            .collect()
    } else {
        cols
    };
    let pair_list: Vec<(usize, usize)> = (0..k).flat_map(|i| (i..k).map(move |j| (i, j))).collect();
    let vals: Vec<(usize, usize, f64, usize)> = pair_list
        .par_iter()
        .map(|&(i, j)| {
            if i == j {
                return (i, j, 1.0, 0);
            }
            let (a, b) = (&cols[i], &cols[j]);
            let mut xs = Vec::with_capacity(a.len());
            let mut ys = Vec::with_capacity(a.len());
            for q in 0..a.len() {
                if a[q].is_finite() && b[q].is_finite() {
                    xs.push(a[q]);
                    ys.push(b[q]);
                }
            }
            (i, j, stats::pearson(&xs, &ys), xs.len())
        })
        .collect();
    let mut r = vec![vec![f64::NAN; k]; k];
    let mut pairs = vec![];
    for (i, j, v, n) in vals {
        r[i][j] = v;
        r[j][i] = v;
        if i != j && v.is_finite() && v.abs() >= threshold {
            pairs.push((
                ds.tests[tests[i]].key.clone(),
                ds.tests[tests[j]].key.clone(),
                v,
                n,
            ));
        }
    }
    pairs.sort_by(|a, b| b.2.abs().partial_cmp(&a.2.abs()).unwrap());
    CorrMatrix {
        method: if spearman {
            "spearman".into()
        } else {
            "pearson".into()
        },
        tests: tests.to_vec(),
        names: tests.iter().map(|&t| ds.tests[t].key.clone()).collect(),
        r,
        pairs,
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct TargetCorr {
    pub key: String,
    pub r: f64,
    pub p: f64,
    pub n: usize,
}

/// Correlation of every numeric test with a target vector (NaN = missing).
pub fn with_target(
    ds: &Dataset,
    rows: &[usize],
    target: &[f64],
    method: &str,
    exclude: Option<usize>,
) -> Vec<TargetCorr> {
    let spearman = method.eq_ignore_ascii_case("spearman");
    let mut out: Vec<TargetCorr> = ds
        .numeric_tests()
        .into_par_iter()
        .filter(|&t| Some(t) != exclude)
        .filter_map(|t| {
            let mut xs = vec![];
            let mut ys = vec![];
            for (k, &r) in rows.iter().enumerate() {
                let v = ds.values[t][r] as f64;
                if v.is_finite() && target[k].is_finite() {
                    xs.push(v);
                    ys.push(target[k]);
                }
            }
            if xs.len() < 5 {
                return None;
            }
            let r = if spearman {
                stats::spearman(&xs, &ys)
            } else {
                stats::pearson(&xs, &ys)
            };
            if !r.is_finite() {
                return None;
            }
            Some(TargetCorr {
                key: ds.tests[t].key.clone(),
                r,
                p: stats::corr_p(r, xs.len()),
                n: xs.len(),
            })
        })
        .collect();
    out.sort_by(|a, b| b.r.abs().partial_cmp(&a.r.abs()).unwrap());
    out
}

/// Choose up to `max` tests for a correlation matrix: the most variable ones.
pub fn pick_tests(ds: &Dataset, rows: &[usize], max: usize) -> Vec<usize> {
    let mut v: Vec<(usize, f64)> = ds
        .numeric_tests()
        .into_par_iter()
        .filter_map(|t| {
            let vals = ds.values_of(t, Some(rows));
            if vals.len() < rows.len() / 2 || vals.len() < 5 {
                return None;
            }
            let s = stats::std(&vals);
            let m = stats::mean(&vals).abs();
            (s > 0.0).then_some((t, s / (m + s)))
        })
        .collect();
    v.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
    v.truncate(max);
    let mut t: Vec<usize> = v.into_iter().map(|x| x.0).collect();
    t.sort_unstable();
    t
}
