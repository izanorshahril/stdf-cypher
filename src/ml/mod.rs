//! Machine learning: PCA, k-means, random forest, isolation forest.

pub mod features;
pub mod forest;
pub mod iforest;
pub mod kmeans;
pub mod pca;

use crate::data::Dataset;
use anyhow::{bail, Result};
use features::{FeatureMatrix, FeatureOptions};
use serde::Serialize;

/// What a supervised model should predict.
#[derive(Debug, Clone)]
pub enum TargetSpec {
    PassFail,
    HardBin,
    SoftBin,
    Site,
    /// regression on a test value (index)
    Test(usize),
}

impl TargetSpec {
    pub fn parse(s: &str, ds: &Dataset) -> Result<TargetSpec> {
        Ok(match s.to_lowercase().as_str() {
            "pass" | "passfail" | "pass_fail" | "pf" => TargetSpec::PassFail,
            "hbin" | "hard_bin" => TargetSpec::HardBin,
            "sbin" | "soft_bin" => TargetSpec::SoftBin,
            "site" => TargetSpec::Site,
            other => {
                let q = other.strip_prefix("test:").unwrap_or(s);
                match ds.find_test(q) {
                    Some(t) => TargetSpec::Test(t),
                    None => bail!("unknown target '{s}' (use pass, hbin, sbin, site or a test)"),
                }
            }
        })
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct PcaReport {
    pub features: FeatureMatrix,
    pub pca: pca::Pca,
    pub kmeans: Option<kmeans::KMeans>,
    /// projected scores of the first components for each row (≤ 3 comps)
    pub scores: Vec<Vec<f64>>,
}

pub fn pca_report(
    ds: &Dataset,
    rows: &[usize],
    fopt: &FeatureOptions,
    n_comp: usize,
    clusters: Option<usize>,
) -> Result<PcaReport> {
    let fm = features::build(ds, rows, fopt);
    if fm.p < 2 {
        bail!(
            "need at least 2 usable numeric tests for PCA (found {})",
            fm.p
        );
    }
    let p = pca::fit(&fm, n_comp.max(2));
    let use_k = p.components_for(0.8).clamp(2, 10).min(p.n_components);
    let km = match clusters {
        Some(0) => None,
        Some(k) => Some(kmeans::fit(
            &p.scores
                .iter()
                .map(|s| s[..use_k].to_vec())
                .collect::<Vec<_>>(),
            k,
            11,
            200,
        )),
        None => Some(kmeans::auto(
            &p.scores
                .iter()
                .map(|s| s[..use_k].to_vec())
                .collect::<Vec<_>>(),
            6,
            11,
        )),
    };
    let scores = p
        .scores
        .iter()
        .map(|s| s.iter().take(3).copied().collect())
        .collect();
    Ok(PcaReport {
        features: fm,
        pca: p,
        kmeans: km,
        scores,
    })
}

/// Train a random forest to predict `target` from the parametric tests.
pub fn explain(
    ds: &Dataset,
    rows: &[usize],
    target: &TargetSpec,
    fopt: &FeatureOptions,
    opt: &forest::ForestOptions,
) -> Result<forest::ForestReport> {
    let mut fo = fopt.clone();
    // never use the target test as a feature
    let mut fm_rows = rows.to_vec();
    let (t, name) = match target {
        TargetSpec::PassFail => {
            fm_rows.retain(|&r| ds.parts[r].passed.is_some());
            let y: Vec<usize> = fm_rows
                .iter()
                .map(|&r| if ds.parts[r].is_pass() { 0 } else { 1 })
                .collect();
            (
                forest::Target::Class {
                    y,
                    names: vec!["pass".into(), "fail".into()],
                },
                "pass/fail".to_string(),
            )
        }
        TargetSpec::HardBin | TargetSpec::SoftBin | TargetSpec::Site => {
            let f = match target {
                TargetSpec::HardBin => "hbin",
                TargetSpec::SoftBin => "sbin",
                _ => "site",
            };
            let groups = ds.group_by(&fm_rows, f);
            let mut y = vec![0usize; ds.n_parts()];
            let names: Vec<String> = groups.iter().map(|g| format!("{f} {}", g.0)).collect();
            for (k, (_, rs)) in groups.iter().enumerate() {
                rs.iter().for_each(|&r| y[r] = k);
            }
            (
                forest::Target::Class {
                    y: fm_rows.iter().map(|&r| y[r]).collect(),
                    names,
                },
                f.to_string(),
            )
        }
        TargetSpec::Test(t) => {
            fm_rows.retain(|&r| ds.values[*t][r].is_finite());
            let y: Vec<f64> = fm_rows.iter().map(|&r| ds.values[*t][r] as f64).collect();
            let tests: Vec<usize> = fo
                .tests
                .take()
                .unwrap_or_else(|| ds.numeric_tests())
                .into_iter()
                .filter(|x| x != t)
                .collect();
            fo.tests = Some(tests);
            fo.min_coverage = fopt.min_coverage;
            (forest::Target::Reg(y), ds.tests[*t].key.clone())
        }
    };
    if fm_rows.len() < 20 {
        bail!("not enough parts ({}) to train a model", fm_rows.len());
    }
    if let Some(list) = &fo.tests {
        // apply coverage filter even with explicit list
        let n = fm_rows.len() as f64;
        let filtered: Vec<usize> = list
            .iter()
            .copied()
            .filter(|&t| {
                fm_rows
                    .iter()
                    .filter(|&&r| ds.values[t][r].is_finite())
                    .count() as f64
                    / n
                    >= fopt.min_coverage
            })
            .collect();
        fo.tests = Some(filtered);
    }
    let fm = features::build(ds, &fm_rows, &fo);
    if fm.p == 0 {
        bail!("no usable features");
    }
    Ok(forest::train_and_evaluate(&fm, &t, &name, opt))
}
