//! Build a dense, imputed, standardized feature matrix from a dataset.

use crate::analysis::stats;
use crate::data::{Dataset, TestKind};
use rayon::prelude::*;
use serde::Serialize;

#[derive(Debug, Clone)]
pub struct FeatureOptions {
    /// minimum fraction of rows where the test has a value
    pub min_coverage: f64,
    /// maximum number of features (highest robust CV kept)
    pub max_features: usize,
    /// use median/robust-sigma instead of mean/std scaling
    pub robust: bool,
    /// explicit list of tests (indices) – overrides automatic selection
    pub tests: Option<Vec<usize>>,
}

impl Default for FeatureOptions {
    fn default() -> Self {
        Self {
            min_coverage: 0.8,
            max_features: 200,
            robust: true,
            tests: None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct FeatureMatrix {
    /// dataset rows represented by each matrix row
    pub rows: Vec<usize>,
    /// dataset test index for each column
    pub tests: Vec<usize>,
    pub names: Vec<String>,
    pub n: usize,
    pub p: usize,
    /// row-major standardized values (n × p)
    #[serde(skip)]
    pub x: Vec<f64>,
    pub center: Vec<f64>,
    pub scale: Vec<f64>,
    /// standardized test limits per column (useful split points for trees)
    pub limit_edges: Vec<Vec<f64>>,
}

impl FeatureMatrix {
    #[inline]
    pub fn row(&self, i: usize) -> &[f64] {
        &self.x[i * self.p..(i + 1) * self.p]
    }
    #[inline]
    pub fn get(&self, i: usize, j: usize) -> f64 {
        self.x[i * self.p + j]
    }
}

pub fn build(ds: &Dataset, rows: &[usize], opt: &FeatureOptions) -> FeatureMatrix {
    let candidates: Vec<usize> = match &opt.tests {
        Some(t) => t.clone(),
        None => (0..ds.n_tests())
            .filter(|&t| ds.tests[t].kind != TestKind::Functional)
            .collect(),
    };
    // per-test stats over the rows
    let mut info: Vec<(usize, f64, f64, f64)> = candidates
        .par_iter()
        .filter_map(|&t| {
            let col = ds.column(t);
            let vals: Vec<f64> = rows
                .iter()
                .map(|&r| col[r] as f64)
                .filter(|v| v.is_finite())
                .collect();
            let cov = vals.len() as f64 / rows.len().max(1) as f64;
            if cov < opt.min_coverage && opt.tests.is_none() {
                return None;
            }
            if vals.len() < 3 {
                return None;
            }
            let (c, s) = if opt.robust {
                let m = stats::median(&vals);
                let mut rs = stats::robust_sigma(&vals);
                if !(rs > 0.0) {
                    rs = stats::std(&vals);
                }
                (m, rs)
            } else {
                (stats::mean(&vals), stats::std(&vals))
            };
            if !(s > 0.0) || !s.is_finite() {
                return None;
            }
            let rank_key = s / (c.abs() + s);
            Some((t, c, s, rank_key))
        })
        .collect();
    if opt.tests.is_none() && info.len() > opt.max_features {
        // prefer tests with distinct information: keep highest relative spread
        info.sort_by(|a, b| b.3.partial_cmp(&a.3).unwrap());
        info.truncate(opt.max_features);
        info.sort_by_key(|x| x.0);
    }
    let p = info.len();
    let n = rows.len();
    let mut x = vec![0.0; n * p];
    x.par_chunks_mut(p.max(1)).enumerate().for_each(|(i, row)| {
        if p == 0 {
            return;
        }
        let r = rows[i];
        for (j, &(t, c, s, _)) in info.iter().enumerate() {
            let v = ds.values[t][r] as f64;
            // median imputation => 0 after centering; clip extreme values to ±50σ
            row[j] = if v.is_finite() {
                ((v - c) / s).clamp(-50.0, 50.0)
            } else {
                0.0
            };
        }
    });
    FeatureMatrix {
        rows: rows.to_vec(),
        tests: info.iter().map(|x| x.0).collect(),
        names: info.iter().map(|x| ds.tests[x.0].key.clone()).collect(),
        n,
        p,
        x,
        center: info.iter().map(|x| x.1).collect(),
        scale: info.iter().map(|x| x.2).collect(),
        limit_edges: info
            .iter()
            .map(|&(t, c, s, _)| {
                let ti = &ds.tests[t];
                [ti.lo, ti.hi]
                    .iter()
                    .flatten()
                    .map(|l| ((l - c) / s).clamp(-50.0, 50.0))
                    .collect()
            })
            .collect(),
    }
}
