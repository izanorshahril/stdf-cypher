//! Abnormal-part detection.
//!
//! Methods (all can run together; each part gets one row with every score):
//! * **robust-z** – per-test |x − median| / (1.4826·MAD) above `robust_k`
//! * **PAT** – static Part Average Testing limits (robust mean ± k·σ, clipped
//!   to spec limits, AEC-Q001 style); flags *passing* parts outside PAT
//! * **NNR** – wafer Nearest-Neighbour Residual: die value vs. median of its
//!   8 neighbours, residual robust-z above `nnr_k`
//! * **Mahalanobis** – multivariate distance on PCA-whitened data, χ² p-value
//! * **Isolation Forest** – unsupervised ML anomaly score

use super::stats;
use crate::data::{Dataset, TestKind};
use crate::ml::{features, iforest::IsolationForest, pca};
use rayon::prelude::*;
use serde::Serialize;
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct AnomalyOptions {
    pub robust_k: f64,
    pub pat_k: f64,
    pub nnr: bool,
    pub nnr_k: f64,
    pub mahalanobis: bool,
    pub mahal_p: f64,
    pub iforest: bool,
    pub iforest_threshold: f64,
    pub max_features: usize,
    /// only report parts that passed (latent-defect screening)
    pub only_passing: bool,
    /// tests to analyse (None = all numeric)
    pub tests: Option<Vec<usize>>,
}

impl Default for AnomalyOptions {
    fn default() -> Self {
        Self {
            robust_k: 6.0,
            pat_k: 6.0,
            nnr: true,
            nnr_k: 5.0,
            mahalanobis: true,
            mahal_p: 1e-6,
            iforest: true,
            iforest_threshold: 0.62,
            max_features: 150,
            only_passing: false,
            tests: None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct TestPat {
    pub key: String,
    pub center: f64,
    pub robust_sigma: f64,
    pub pat_lo: f64,
    pub pat_hi: f64,
    pub lo: Option<f64>,
    pub hi: Option<f64>,
    /// passing parts outside PAT limits
    pub n_outside: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct PartAnomaly {
    pub row: usize,
    pub part_id: String,
    pub wafer: String,
    pub x: i16,
    pub y: i16,
    pub site: u8,
    pub hbin: u16,
    pub pass: bool,
    pub n_robust: usize,
    pub max_robust_z: f64,
    pub n_pat: usize,
    pub n_nnr: usize,
    pub max_nnr_z: f64,
    pub mahal_d2: f64,
    pub mahal_p: f64,
    pub iforest: f64,
    /// methods that flagged this part
    pub methods: Vec<String>,
    /// tests with largest |robust z|
    pub top_tests: Vec<(String, f64)>,
    pub score: f64,
}

/// Every method's raw score for one analysed part (NaN = not computed).
#[derive(Debug, Clone, Copy, Default)]
pub struct RowScores {
    pub max_robust_z: f64,
    pub n_pat: usize,
    pub max_nnr_z: f64,
    pub mahal_d2: f64,
    pub iforest: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct AnomalyReport {
    pub n_parts: usize,
    pub n_tests: usize,
    pub n_flagged: usize,
    pub by_method: HashMap<String, usize>,
    pub pat: Vec<TestPat>,
    /// flagged parts sorted by score (all parts' scores are in `scores`)
    pub parts: Vec<PartAnomaly>,
    /// combined score for every analysed row (same order as `rows`)
    #[serde(skip)]
    pub scores: Vec<f64>,
    #[serde(skip)]
    pub rows: Vec<usize>,
    #[serde(skip)]
    pub iforest_scores: Vec<f64>,
    /// per-method scores for every analysed row (same order as `rows`)
    #[serde(skip)]
    pub per_row: Vec<RowScores>,
}

pub fn detect(ds: &Dataset, rows: &[usize], opt: &AnomalyOptions) -> AnomalyReport {
    let tests: Vec<usize> = opt
        .tests
        .clone()
        .unwrap_or_else(|| ds.numeric_tests())
        .into_iter()
        .filter(|&t| ds.tests[t].kind != TestKind::Functional)
        .collect();
    let n = rows.len();
    // per test robust center/scale
    let centers: Vec<(usize, f64, f64)> = tests
        .par_iter()
        .filter_map(|&t| {
            let v = ds.values_of(t, Some(rows));
            if v.len() < 10 {
                return None;
            }
            let m = stats::median(&v);
            let s = stats::robust_sigma(&v);
            (s > 0.0).then_some((t, m, s))
        })
        .collect();
    // PAT limits
    let pat: Vec<TestPat> = centers
        .iter()
        .map(|&(t, m, s)| {
            let info = &ds.tests[t];
            let mut lo = m - opt.pat_k * s;
            let mut hi = m + opt.pat_k * s;
            if let Some(l) = info.lo {
                lo = lo.max(l);
            }
            if let Some(h) = info.hi {
                hi = hi.min(h);
            }
            let n_out = rows
                .iter()
                .filter(|&&r| ds.parts[r].is_pass())
                .filter(|&&r| {
                    let v = ds.value(t, r);
                    v.is_finite() && (v < lo || v > hi)
                })
                .count();
            TestPat {
                key: info.key.clone(),
                center: m,
                robust_sigma: s,
                pat_lo: lo,
                pat_hi: hi,
                lo: info.lo,
                hi: info.hi,
                n_outside: n_out,
            }
        })
        .collect();
    // univariate per part
    let uni: Vec<(usize, f64, usize, Vec<(String, f64)>)> = rows
        .par_iter()
        .map(|&r| {
            let mut cnt = 0;
            let mut maxz = 0.0f64;
            let mut patc = 0;
            let mut top: Vec<(usize, f64)> = vec![];
            for (ci, &(t, m, s)) in centers.iter().enumerate() {
                let v = ds.value(t, r);
                if !v.is_finite() {
                    continue;
                }
                let z = (v - m) / s;
                if z.abs() >= opt.robust_k {
                    cnt += 1;
                }
                if v < pat[ci].pat_lo || v > pat[ci].pat_hi {
                    patc += 1;
                }
                maxz = maxz.max(z.abs());
                top.push((t, z));
            }
            top.sort_by(|a, b| b.1.abs().partial_cmp(&a.1.abs()).unwrap());
            top.truncate(5);
            (
                cnt,
                maxz,
                patc,
                top.into_iter()
                    .map(|(t, z)| (ds.tests[t].key.clone(), z))
                    .collect(),
            )
        })
        .collect();
    // NNR
    let mut nnr = vec![(0usize, 0.0f64); n];
    let nnr_on = opt.nnr && rows.iter().any(|&r| ds.parts[r].x != -32768);
    if nnr_on {
        let mut by_wafer: HashMap<(u16, u32), Vec<usize>> = HashMap::new();
        for (k, &r) in rows.iter().enumerate() {
            let p = &ds.parts[r];
            if p.x != -32768 {
                by_wafer.entry((p.file, p.wafer)).or_default().push(k);
            }
        }
        let per_wafer: Vec<Vec<(usize, usize, f64)>> = by_wafer
            .par_iter()
            .map(|(_, ks)| {
                let mut grid: HashMap<(i16, i16), usize> = HashMap::new();
                for &k in ks {
                    let p = &ds.parts[rows[k]];
                    grid.insert((p.x, p.y), k); // last test wins
                }
                let mut res: HashMap<usize, (usize, f64)> = HashMap::new();
                for &(t, _, _) in &centers {
                    let mut resid: Vec<(usize, f64)> = vec![];
                    for (&(x, y), &k) in &grid {
                        let v = ds.value(t, rows[k]);
                        if !v.is_finite() {
                            continue;
                        }
                        let mut nb = vec![];
                        for dx in -1..=1i16 {
                            for dy in -1..=1i16 {
                                if dx == 0 && dy == 0 {
                                    continue;
                                }
                                if let Some(&kk) = grid.get(&(x + dx, y + dy)) {
                                    let w = ds.value(t, rows[kk]);
                                    if w.is_finite() {
                                        nb.push(w);
                                    }
                                }
                            }
                        }
                        if nb.len() >= 3 {
                            resid.push((k, v - stats::median(&nb)));
                        }
                    }
                    if resid.len() < 10 {
                        continue;
                    }
                    let rv: Vec<f64> = resid.iter().map(|x| x.1).collect();
                    let (m, s) = (stats::median(&rv), stats::robust_sigma(&rv));
                    if !(s > 0.0) {
                        continue;
                    }
                    for (k, e) in resid {
                        let z = ((e - m) / s).abs();
                        let ent = res.entry(k).or_insert((0, 0.0));
                        if z >= opt.nnr_k {
                            ent.0 += 1;
                        }
                        ent.1 = ent.1.max(z);
                    }
                }
                res.into_iter().map(|(k, (c, z))| (k, c, z)).collect()
            })
            .collect();
        for v in per_wafer {
            for (k, c, z) in v {
                nnr[k] = (c, z);
            }
        }
    }
    // multivariate
    let fm = if opt.mahalanobis || opt.iforest {
        let fo = features::FeatureOptions {
            max_features: opt.max_features,
            tests: Some(centers.iter().map(|c| c.0).collect()),
            ..Default::default()
        };
        let mut fm = features::build(ds, rows, &fo);
        if fm.p > opt.max_features {
            let fo2 = features::FeatureOptions {
                max_features: opt.max_features,
                ..Default::default()
            };
            fm = features::build(ds, rows, &fo2);
        }
        Some(fm)
    } else {
        None
    };
    let mut mahal = vec![(f64::NAN, f64::NAN); n];
    if let (true, Some(fm)) = (opt.mahalanobis, &fm) {
        if fm.p >= 2 && fm.n > fm.p * 2 {
            let p = pca::fit(fm, fm.p.min(50));
            let k = p.components_for(0.95).max(1);
            let ev: Vec<f64> = p.eigenvalues[..k].to_vec();
            mahal = p
                .scores
                .par_iter()
                .map(|s| {
                    let d2: f64 = s[..k]
                        .iter()
                        .zip(&ev)
                        .filter(|(_, &e)| e > 1e-12)
                        .map(|(x, e)| x * x / e)
                        .sum();
                    (d2, stats::chi2_sf(d2, k as f64))
                })
                .collect();
        }
    }
    let mut ifs = vec![f64::NAN; n];
    if let (true, Some(fm)) = (opt.iforest, &fm) {
        if fm.p >= 1 && fm.n >= 50 {
            let f = IsolationForest::fit(fm, 200, 256, 17);
            ifs = f.score_all(fm);
        }
    }
    let mut parts: Vec<PartAnomaly> = vec![];
    let mut scores = vec![0.0; n];
    let per_row: Vec<RowScores> = (0..n)
        .map(|k| RowScores {
            max_robust_z: uni[k].1,
            n_pat: if ds.parts[rows[k]].is_pass() {
                uni[k].2
            } else {
                0
            },
            max_nnr_z: if nnr_on { nnr[k].1 } else { f64::NAN },
            mahal_d2: mahal[k].0,
            iforest: ifs[k],
        })
        .collect();
    let mut by_method: HashMap<String, usize> = HashMap::new();
    for (k, &r) in rows.iter().enumerate() {
        let p = &ds.parts[r];
        let (nr, mz, np, top) = &uni[k];
        let mut methods = vec![];
        if *nr > 0 {
            methods.push("robust-z".to_string());
        }
        if *np > 0 && p.is_pass() {
            methods.push("PAT".to_string());
        }
        if nnr[k].0 > 0 {
            methods.push("NNR".to_string());
        }
        if mahal[k].1.is_finite() && mahal[k].1 < opt.mahal_p {
            methods.push("mahalanobis".to_string());
        }
        if ifs[k].is_finite() && ifs[k] >= opt.iforest_threshold {
            methods.push("iforest".to_string());
        }
        let score = mz / opt.robust_k
            + nnr[k].1 / opt.nnr_k
            + if mahal[k].1.is_finite() {
                (-mahal[k].1.max(1e-300).log10() / 6.0).min(5.0)
            } else {
                0.0
            }
            + if ifs[k].is_finite() {
                ((ifs[k] - 0.5).max(0.0) * 10.0).min(5.0)
            } else {
                0.0
            }
            + methods.len() as f64;
        scores[k] = score;
        if methods.is_empty() || (opt.only_passing && !p.is_pass()) {
            continue;
        }
        for m in &methods {
            *by_method.entry(m.clone()).or_default() += 1;
        }
        parts.push(PartAnomaly {
            row: r,
            part_id: p.part_id.clone(),
            wafer: ds.wafer_name(p).to_string(),
            x: p.x,
            y: p.y,
            site: p.site,
            hbin: p.hbin,
            pass: p.is_pass(),
            n_robust: *nr,
            max_robust_z: *mz,
            n_pat: *np,
            n_nnr: nnr[k].0,
            max_nnr_z: nnr[k].1,
            mahal_d2: mahal[k].0,
            mahal_p: mahal[k].1,
            iforest: ifs[k],
            methods,
            top_tests: top.clone(),
            score,
        });
    }
    parts.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap());
    AnomalyReport {
        n_parts: n,
        n_tests: centers.len(),
        n_flagged: parts.len(),
        by_method,
        pat,
        parts,
        scores,
        rows: rows.to_vec(),
        iforest_scores: ifs,
        per_row,
    }
}
