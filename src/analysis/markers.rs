//! Marker tests: which tests tell an "affected" group of parts apart from a
//! reference group, ranked so the strongest candidates come first.
//!
//! Typical use: parts that passed (bin 1) at one insertion and failed at a
//! later one ("affected", found by matching the two files) against the bin-1
//! parts that kept passing ("reference"). Each test gets several views of the
//! difference, because a marker can show up in different ways:
//!
//! * **separation**: rank-based AUC of affected vs reference (|2·AUC − 1|,
//!   0 = identical, 1 = no overlap), with Mann–Whitney q-values, KS D and a
//!   robust shift in reference σ;
//! * **tails**: share of affected parts beyond ±3 robust σ of the reference
//!   minus the reference's own share (sigma-deviation counting, as used for
//!   latent-defect screening);
//! * **PAT**: the same excess beyond static Part Average Testing limits of the
//!   reference (median ± 6 robust σ, clipped to the test limits, AEC-Q001);
//! * **ANOVA**: one-way F test and η² (share of variance explained by the
//!   group), reported alongside;
//! * **drift** (when the same units are known in a second file): how much the
//!   per-unit change between the two files differs between the groups, how
//!   far affected units sit from the reference's file-1 → file-2 regression
//!   (residual), and how much the file-1 ↔ file-2 correlation breaks down;
//! * **model**: impurity importance from a class-balanced random forest that
//!   predicts affected vs reference from all tests at once, weighted by the
//!   model's hold-out skill (2·AUC − 1) so a model that cannot separate the
//!   groups does not promote its noise features.
//!
//! The candidate score is the strongest of these normalized signals, with the
//! signal that produced it reported, so a test surfaces if it is a strong
//! marker in any one view.

use super::anova;
use super::stats;
use crate::data::Dataset;
use crate::ml::{features, forest};
use rayon::prelude::*;
use serde::Serialize;
use std::collections::HashMap;

/// One group of parts in one dataset.
pub struct Group<'a> {
    pub ds: &'a Dataset,
    pub rows: Vec<usize>,
}

/// The same units measured in another file: dataset plus row → row pairing.
pub struct Later<'a> {
    pub ds: &'a Dataset,
    pub pairs: HashMap<usize, usize>,
    pub name: String,
}

#[derive(Debug, Clone)]
pub struct MarkerOptions {
    pub min_n: usize,
    pub forest: bool,
    pub trees: usize,
    /// test pairing between datasets: "key", "num" or "name"
    pub match_by: String,
    /// test numbers left out (e.g. identity tests: wafer, X/Y, die ID)
    pub exclude: Vec<u32>,
}

impl Default for MarkerOptions {
    fn default() -> Self {
        Self {
            min_n: 5,
            forest: true,
            trees: 150,
            match_by: "key".into(),
            exclude: vec![],
        }
    }
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct MarkerRow {
    pub key: String,
    pub num: u32,
    pub units: String,
    pub lo: Option<f64>,
    pub hi: Option<f64>,
    pub n_ref: usize,
    pub n_aff: usize,
    pub median_ref: f64,
    pub median_aff: f64,
    /// (median affected − median reference) / robust σ of reference
    pub shift_sigma: f64,
    pub auc: f64,
    pub separation: f64,
    pub ks_d: f64,
    pub p: f64,
    pub q: f64,
    /// Brown–Forsythe/Levene p for a spread difference
    pub levene_p: f64,
    pub sigma_ratio: f64,
    /// share beyond ±3 robust σ of the reference
    pub tail_ref: f64,
    pub tail_aff: f64,
    pub tail_excess: f64,
    /// share outside the reference's PAT limits (median ± 6σ within limits)
    pub pat_ref: f64,
    pub pat_aff: f64,
    pub pat_excess: f64,
    /// one-way ANOVA affected vs reference
    pub anova_p: f64,
    pub eta2: f64,
    pub fail_ref: f64,
    pub fail_aff: f64,
    // cross-file (same units)
    pub n_drift_ref: usize,
    pub n_drift_aff: usize,
    /// separation of the per-unit change (file 2 − file 1)
    pub drift_sep: Option<f64>,
    pub drift_shift_sigma: Option<f64>,
    pub r_ref: Option<f64>,
    pub r_aff: Option<f64>,
    pub corr_break: Option<f64>,
    /// median residual of affected units vs the reference regression, in σ
    pub resid_sigma: Option<f64>,
    pub importance: Option<f64>,
    pub score: f64,
    pub why: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PartScore {
    pub aff_auc: f64,
    /// tests beyond ±3σ of reference, per part
    pub counts_ref: Vec<usize>,
    pub counts_aff: Vec<usize>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MarkerReport {
    pub n_ref: usize,
    pub n_aff: usize,
    pub n_tests: usize,
    pub later: Option<String>,
    pub rows: Vec<MarkerRow>,
    pub forest_auc: Option<f64>,
    pub forest_error: Option<String>,
    /// sigma-deviation count per part (how many tests sit outside ±3σ)
    pub sdc: PartScore,
}

/// Static PAT limits: median ± 6 robust σ, never wider than the test limits.
pub fn pat_limits(center: f64, sigma: f64, lo: Option<f64>, hi: Option<f64>) -> (f64, f64) {
    let (mut a, mut b) = (center - 6.0 * sigma, center + 6.0 * sigma);
    if let Some(l) = lo {
        a = a.max(l);
    }
    if let Some(h) = hi {
        b = b.min(h);
    }
    (a, b)
}

/// Rank-based AUC: P(affected > reference) with ties counted half.
pub fn auc(aff: &[f64], refv: &[f64]) -> f64 {
    let (na, nr) = (aff.len() as f64, refv.len() as f64);
    if na == 0.0 || nr == 0.0 {
        return f64::NAN;
    }
    let all: Vec<f64> = aff.iter().chain(refv).copied().collect();
    let r = stats::ranks(&all);
    let ra: f64 = r[..aff.len()].iter().sum();
    (ra - na * (na + 1.0) / 2.0) / (na * nr)
}

fn tail_share(v: &[f64], center: f64, sigma: f64) -> f64 {
    if v.is_empty() || !(sigma > 0.0) {
        return 0.0;
    }
    v.iter()
        .filter(|&&x| (x - center).abs() > 3.0 * sigma)
        .count() as f64
        / v.len() as f64
}

fn vals(ds: &Dataset, t: usize, rows: &[usize]) -> Vec<f64> {
    ds.values_of(t, Some(rows))
}

fn pair_test(a: &Dataset, b: &Dataset, t: usize, by: &str) -> Option<usize> {
    let info = &a.tests[t];
    match by {
        "num" => b.tests.iter().position(|x| x.num == info.num),
        "name" => b.tests.iter().position(|x| x.name == info.name),
        _ => b.test_index.get(&info.key).copied().or_else(|| {
            b.tests
                .iter()
                .position(|x| x.num == info.num && x.name == info.name)
        }),
    }
}

/// Find marker tests: `aff` (affected) against `refg` (reference). Tests are
/// taken from the reference dataset and paired with the affected dataset by
/// key / number / name. With `later`, the same units in another file add the
/// drift and correlation-break views (reference and affected must then be
/// rows of the same dataset as `later.pairs` keys).
pub fn find(refg: &Group, aff: &Group, later: Option<&Later>, opt: &MarkerOptions) -> MarkerReport {
    let (dr, da) = (refg.ds, aff.ds);
    let tests: Vec<(usize, usize)> = dr
        .numeric_tests()
        .into_iter()
        .filter(|&t| !opt.exclude.contains(&dr.tests[t].num))
        .filter_map(|t| {
            if std::ptr::eq(dr, da) {
                Some((t, t))
            } else {
                pair_test(dr, da, t, &opt.match_by).map(|u| (t, u))
            }
        })
        .collect();
    let mut rows: Vec<MarkerRow> = tests
        .par_iter()
        .filter_map(|&(tr, ta)| {
            let vr = vals(dr, tr, &refg.rows);
            let va = vals(da, ta, &aff.rows);
            if vr.len() < opt.min_n || va.len() < opt.min_n {
                return None;
            }
            let info = &dr.tests[tr];
            let (mr, ma) = (stats::median(&vr), stats::median(&va));
            let sr = stats::robust_sigma(&vr);
            let sa = stats::robust_sigma(&va);
            let a = auc(&va, &vr);
            let mw = stats::mann_whitney(&va, &vr);
            let ks = stats::ks_2samp(&va, &vr);
            let (_, lev_p) = anova::levene(&[vr.clone(), va.clone()]);
            let (tail_ref, tail_aff) = (tail_share(&vr, mr, sr), tail_share(&va, mr, sr));
            let fails = |v: &[f64]| {
                v.iter().filter(|&&x| !info.within_limits(x)).count() as f64 / v.len() as f64
            };
            let (plo, phi) = pat_limits(mr, sr, info.lo, info.hi);
            let pat = |v: &[f64]| {
                v.iter().filter(|&&x| x < plo || x > phi).count() as f64 / v.len() as f64
            };
            let (pat_ref, pat_aff) = if sr > 0.0 {
                (pat(&vr), pat(&va))
            } else {
                (0.0, 0.0)
            };
            let (_, anova_p, eta2, _, _) = anova::one_way(&[vr.clone(), va.clone()]);
            let mut row = MarkerRow {
                key: info.key.clone(),
                num: info.num,
                units: info.units.clone(),
                lo: info.lo,
                hi: info.hi,
                n_ref: vr.len(),
                n_aff: va.len(),
                median_ref: mr,
                median_aff: ma,
                shift_sigma: if sr > 0.0 { (ma - mr) / sr } else { f64::NAN },
                auc: a,
                separation: (2.0 * a - 1.0).abs(),
                ks_d: ks.stat,
                p: mw.p,
                levene_p: lev_p,
                sigma_ratio: if sr > 0.0 { sa / sr } else { f64::NAN },
                tail_ref,
                tail_aff,
                tail_excess: tail_aff - tail_ref,
                pat_ref,
                pat_aff,
                pat_excess: pat_aff - pat_ref,
                anova_p,
                eta2,
                fail_ref: fails(&vr),
                fail_aff: fails(&va),
                ..Default::default()
            };
            if let Some(l) = later {
                if let Some(t2) = pair_test(dr, l.ds, tr, &opt.match_by) {
                    let pairs = |rows: &[usize], ds1: &Dataset, t1: usize| -> Vec<(f64, f64)> {
                        rows.iter()
                            .filter_map(|r| {
                                let r2 = *l.pairs.get(r)?;
                                let (x, y) = (ds1.value(t1, *r), l.ds.value(t2, r2));
                                (x.is_finite() && y.is_finite()).then_some((x, y))
                            })
                            .collect()
                    };
                    let pr = pairs(&refg.rows, dr, tr);
                    let pa = pairs(&aff.rows, da, ta);
                    row.n_drift_ref = pr.len();
                    row.n_drift_aff = pa.len();
                    if pr.len() >= opt.min_n && pa.len() >= opt.min_n.min(3) {
                        let dref: Vec<f64> = pr.iter().map(|p| p.1 - p.0).collect();
                        let daff: Vec<f64> = pa.iter().map(|p| p.1 - p.0).collect();
                        let ds_ = stats::robust_sigma(&dref);
                        row.drift_sep = Some((2.0 * auc(&daff, &dref) - 1.0).abs());
                        row.drift_shift_sigma = (ds_ > 0.0)
                            .then(|| (stats::median(&daff) - stats::median(&dref)) / ds_);
                        let (xr, yr): (Vec<f64>, Vec<f64>) = pr.iter().copied().unzip();
                        let (xa, ya): (Vec<f64>, Vec<f64>) = pa.iter().copied().unzip();
                        let r_ref = stats::pearson(&xr, &yr);
                        let r_aff = stats::pearson(&xa, &ya);
                        row.r_ref = r_ref.is_finite().then_some(r_ref);
                        row.r_aff = (r_aff.is_finite() && pa.len() >= 8).then_some(r_aff);
                        if let (Some(a), Some(b)) = (row.r_ref, row.r_aff) {
                            row.corr_break = Some((a - b).max(0.0));
                        }
                        let (icpt, slope, _) = stats::linreg(&xr, &yr);
                        if slope.is_finite() && icpt.is_finite() {
                            let res = |x: f64, y: f64| y - (icpt + slope * x);
                            let rr: Vec<f64> = pr.iter().map(|p| res(p.0, p.1)).collect();
                            let ra: Vec<f64> = pa.iter().map(|p| res(p.0, p.1)).collect();
                            let s = stats::robust_sigma(&rr);
                            row.resid_sigma =
                                (s > 0.0).then(|| (stats::median(&ra) - stats::median(&rr)) / s);
                        }
                    }
                }
            }
            Some(row)
        })
        .collect();

    // multiple-testing correction on the Mann–Whitney p-values
    let q = stats::bh_adjust(&rows.iter().map(|r| r.p).collect::<Vec<_>>());
    rows.iter_mut().zip(q).for_each(|(r, q)| r.q = q);

    // multivariate view: random forest affected (1) vs reference (0)
    let (mut forest_auc, mut forest_error) = (None, None);
    if opt.forest {
        match forest_importance(refg, aff, opt) {
            Ok((imp, a)) => {
                forest_auc = a;
                let max = imp.values().copied().fold(0.0, f64::max);
                for r in rows.iter_mut() {
                    if let Some(v) = imp.get(&r.key) {
                        r.importance = Some(if max > 0.0 { v / max } else { 0.0 });
                    }
                }
            }
            Err(e) => forest_error = Some(e),
        }
    }

    let skill = forest_auc
        .map(|a| (2.0 * a - 1.0).clamp(0.0, 1.0))
        .unwrap_or(0.0);
    for r in rows.iter_mut() {
        let sig = r.q < 0.05;
        let mut cands: Vec<(f64, &str)> = vec![
            (
                if sig {
                    r.separation
                } else {
                    r.separation * 0.5
                },
                "separation",
            ),
            ((r.tail_excess * 2.0).clamp(0.0, 1.0), "tails"),
            ((r.pat_excess * 4.0).clamp(0.0, 1.0), "PAT"),
        ];
        if let Some(d) = r.drift_sep {
            cands.push((d, "drift"));
        }
        if let Some(c) = r.corr_break {
            cands.push((c.min(1.0), "correlation break"));
        }
        if let Some(s) = r.resid_sigma {
            cands.push(((s.abs() / 6.0).min(1.0), "residual"));
        }
        if let Some(i) = r.importance {
            // importance is relative to the top feature, so it only counts as
            // much as the model can actually tell the groups apart
            cands.push((i * skill, "random forest"));
        }
        let (s, why) = cands
            .into_iter()
            .filter(|c| c.0.is_finite())
            .fold((0.0, "separation"), |a, c| if c.0 > a.0 { c } else { a });
        r.score = s;
        r.why = why.to_string();
    }
    rows.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // per-part sigma-deviation count against the reference distributions
    let centers: Vec<(usize, usize, f64, f64)> = tests
        .iter()
        .filter_map(|&(tr, ta)| {
            let vr = vals(dr, tr, &refg.rows);
            if vr.len() < opt.min_n {
                return None;
            }
            let s = stats::robust_sigma(&vr);
            (s > 0.0).then(|| (tr, ta, stats::median(&vr), s))
        })
        .collect();
    let count = |ds: &Dataset, row: usize, use_a: bool| {
        centers
            .iter()
            .filter(|&&(tr, ta, c, s)| {
                let v = ds.value(if use_a { ta } else { tr }, row);
                v.is_finite() && (v - c).abs() > 3.0 * s
            })
            .count()
    };
    let counts_ref: Vec<usize> = refg.rows.par_iter().map(|&r| count(dr, r, false)).collect();
    let counts_aff: Vec<usize> = aff.rows.par_iter().map(|&r| count(da, r, true)).collect();
    let f = |v: &[usize]| v.iter().map(|&x| x as f64).collect::<Vec<_>>();
    let sdc = PartScore {
        aff_auc: auc(&f(&counts_aff), &f(&counts_ref)),
        counts_ref,
        counts_aff,
    };

    MarkerReport {
        n_ref: refg.rows.len(),
        n_aff: aff.rows.len(),
        n_tests: rows.len(),
        later: later.map(|l| l.name.clone()),
        rows,
        forest_auc,
        forest_error,
        sdc,
    }
}

/// Random-forest importance (by test key) and hold-out AUC.
fn forest_importance(
    refg: &Group,
    aff: &Group,
    opt: &MarkerOptions,
) -> Result<(HashMap<String, f64>, Option<f64>), String> {
    if refg.rows.len() < 10 || aff.rows.len() < 5 {
        return Err("too few parts for a model (need ≥ 10 reference and ≥ 5 affected)".into());
    }
    // one dataset holding both groups
    let (ds, ref_rows, aff_rows): (std::borrow::Cow<Dataset>, Vec<usize>, Vec<usize>) =
        if std::ptr::eq(refg.ds, aff.ds) {
            (
                std::borrow::Cow::Borrowed(refg.ds),
                refg.rows.clone(),
                aff.rows.clone(),
            )
        } else {
            let m = Dataset::merge(vec![refg.ds.subset(&refg.rows), aff.ds.subset(&aff.rows)]);
            let n = refg.rows.len();
            (
                std::borrow::Cow::Owned(m),
                (0..n).collect(),
                (n..n + aff.rows.len()).collect(),
            )
        };
    let set: std::collections::HashSet<usize> = aff_rows.iter().copied().collect();
    let rows: Vec<usize> = ref_rows.iter().chain(&aff_rows).copied().collect();
    let fo = features::FeatureOptions {
        min_coverage: 0.5,
        max_features: 5000,
        // an explicit list skips the coverage check, so apply it here
        tests: (!opt.exclude.is_empty()).then(|| {
            ds.numeric_tests()
                .into_iter()
                .filter(|&t| !opt.exclude.contains(&ds.tests[t].num))
                .filter(|&t| {
                    let have = rows.iter().filter(|&&r| ds.value(t, r).is_finite()).count();
                    have as f64 >= 0.5 * rows.len() as f64
                })
                .collect()
        }),
        ..Default::default()
    };
    let fm = features::build(&ds, &rows, &fo);
    if fm.p == 0 {
        return Err("no usable tests for a model".into());
    }
    let y: Vec<usize> = fm.rows.iter().map(|r| set.contains(r) as usize).collect();
    let target = forest::Target::Class {
        y,
        names: vec!["reference".into(), "affected".into()],
    };
    let fopt = forest::ForestOptions {
        n_trees: opt.trees,
        max_depth: 10,
        min_leaf: 2,
        ..Default::default()
    };
    let rep = forest::train_and_evaluate(&fm, &target, "affected", &fopt);
    Ok((rep.importance.into_iter().collect(), rep.auc))
}
