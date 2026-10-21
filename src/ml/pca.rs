//! Principal component analysis on a standardized feature matrix.

use super::features::FeatureMatrix;
use nalgebra::{DMatrix, SymmetricEigen};
use rayon::prelude::*;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct Pca {
    pub n_components: usize,
    pub eigenvalues: Vec<f64>,
    pub explained_ratio: Vec<f64>,
    pub cumulative: Vec<f64>,
    /// loadings[k][j]: weight of feature j in component k
    pub loadings: Vec<Vec<f64>>,
    pub feature_names: Vec<String>,
    /// scores[i][k]
    #[serde(skip)]
    pub scores: Vec<Vec<f64>>,
    #[serde(skip)]
    pub means: Vec<f64>,
}

/// Covariance of the columns of `fm` (centered on column means).
pub fn covariance(fm: &FeatureMatrix) -> (Vec<f64>, DMatrix<f64>) {
    let (n, p) = (fm.n, fm.p);
    let mut means = vec![0.0; p];
    for i in 0..n {
        for (j, m) in means.iter_mut().enumerate() {
            *m += fm.get(i, j);
        }
    }
    means.iter_mut().for_each(|m| *m /= n.max(1) as f64);
    let chunk = 2048;
    let acc = (0..n)
        .into_par_iter()
        .step_by(chunk)
        .map(|start| {
            let mut c = vec![0.0; p * p];
            let mut d = vec![0.0; p];
            for i in start..(start + chunk).min(n) {
                let row = fm.row(i);
                for j in 0..p {
                    d[j] = row[j] - means[j];
                }
                for a in 0..p {
                    let da = d[a];
                    if da == 0.0 {
                        continue;
                    }
                    let base = a * p;
                    for b in a..p {
                        c[base + b] += da * d[b];
                    }
                }
            }
            c
        })
        .reduce(
            || vec![0.0; p * p],
            |mut a, b| {
                a.iter_mut().zip(b).for_each(|(x, y)| *x += y);
                a
            },
        );
    let denom = (n.max(2) - 1) as f64;
    let mut m = DMatrix::<f64>::zeros(p, p);
    for a in 0..p {
        for b in a..p {
            let v = acc[a * p + b] / denom;
            m[(a, b)] = v;
            m[(b, a)] = v;
        }
    }
    (means, m)
}

pub fn fit(fm: &FeatureMatrix, n_components: usize) -> Pca {
    let p = fm.p;
    if p == 0 || fm.n < 2 {
        return Pca {
            n_components: 0,
            eigenvalues: vec![],
            explained_ratio: vec![],
            cumulative: vec![],
            loadings: vec![],
            feature_names: vec![],
            scores: vec![vec![]; fm.n],
            means: vec![],
        };
    }
    let (means, cov) = covariance(fm);
    let eig = SymmetricEigen::new(cov);
    let mut order: Vec<usize> = (0..p).collect();
    order.sort_by(|&a, &b| eig.eigenvalues[b].partial_cmp(&eig.eigenvalues[a]).unwrap());
    let total: f64 = eig.eigenvalues.iter().map(|v| v.max(0.0)).sum();
    let k = n_components.min(p);
    let eigenvalues: Vec<f64> = order
        .iter()
        .take(k)
        .map(|&i| eig.eigenvalues[i].max(0.0))
        .collect();
    let explained_ratio: Vec<f64> = eigenvalues
        .iter()
        .map(|v| if total > 0.0 { v / total } else { 0.0 })
        .collect();
    let mut cumulative = vec![];
    let mut c = 0.0;
    for r in &explained_ratio {
        c += r;
        cumulative.push(c);
    }
    let loadings: Vec<Vec<f64>> = order
        .iter()
        .take(k)
        .map(|&i| {
            let mut v: Vec<f64> = eig.eigenvectors.column(i).iter().copied().collect();
            // deterministic sign: largest |loading| positive
            let (mi, _) = v.iter().enumerate().fold((0, 0.0f64), |acc, (j, &x)| {
                if x.abs() > acc.1 {
                    (j, x.abs())
                } else {
                    acc
                }
            });
            if v[mi] < 0.0 {
                v.iter_mut().for_each(|x| *x = -*x);
            }
            v
        })
        .collect();
    let scores: Vec<Vec<f64>> = (0..fm.n)
        .into_par_iter()
        .map(|i| {
            let row = fm.row(i);
            loadings
                .iter()
                .map(|l| {
                    l.iter()
                        .zip(row)
                        .zip(&means)
                        .map(|((w, x), m)| w * (x - m))
                        .sum()
                })
                .collect()
        })
        .collect();
    Pca {
        n_components: k,
        eigenvalues,
        explained_ratio,
        cumulative,
        loadings,
        feature_names: fm.names.clone(),
        scores,
        means,
    }
}

impl Pca {
    /// Number of components needed to explain `frac` of variance.
    pub fn components_for(&self, frac: f64) -> usize {
        self.cumulative
            .iter()
            .position(|&c| c >= frac)
            .map(|i| i + 1)
            .unwrap_or(self.n_components)
    }
    /// Top contributing features of component k.
    pub fn top_loadings(&self, k: usize, n: usize) -> Vec<(String, f64)> {
        let mut v: Vec<(String, f64)> = self.loadings[k]
            .iter()
            .enumerate()
            .map(|(j, &w)| (self.feature_names[j].clone(), w))
            .collect();
        v.sort_by(|a, b| b.1.abs().partial_cmp(&a.1.abs()).unwrap());
        v.truncate(n);
        v
    }
}
