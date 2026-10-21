//! One-way ANOVA (plus Kruskal–Wallis and Brown–Forsythe/Levene) of every
//! test against a grouping factor such as site, head, wafer, lot/file or bin.

use super::stats;
use crate::data::Dataset;
use rayon::prelude::*;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct GroupStat {
    pub group: String,
    pub n: usize,
    pub mean: f64,
    pub std: f64,
    pub median: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct AnovaRow {
    pub key: String,
    pub units: String,
    pub k: usize,
    pub n: usize,
    pub df_between: f64,
    pub df_within: f64,
    pub f: f64,
    pub p: f64,
    /// effect size: SS_between / SS_total
    pub eta2: f64,
    pub kw_h: f64,
    pub kw_p: f64,
    /// Brown–Forsythe test for equal variances
    pub levene_f: f64,
    pub levene_p: f64,
    /// BH-adjusted p
    pub q: f64,
    /// largest group-mean range as % of limit range
    pub range_pct_limit: Option<f64>,
    pub groups: Vec<GroupStat>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AnovaReport {
    pub factor: String,
    pub groups: Vec<(String, usize)>,
    pub rows: Vec<AnovaRow>,
    pub n_significant: usize,
    pub alpha: f64,
}

pub fn one_way(groups: &[Vec<f64>]) -> (f64, f64, f64, f64, f64) {
    let g: Vec<&Vec<f64>> = groups.iter().filter(|v| !v.is_empty()).collect();
    let k = g.len();
    let n: usize = g.iter().map(|v| v.len()).sum();
    if k < 2 || n <= k {
        return (f64::NAN, f64::NAN, f64::NAN, f64::NAN, f64::NAN);
    }
    let grand = g.iter().flat_map(|v| v.iter()).sum::<f64>() / n as f64;
    let mut ssb = 0.0;
    let mut ssw = 0.0;
    for v in &g {
        let m = stats::mean(v);
        ssb += v.len() as f64 * (m - grand).powi(2);
        ssw += v.iter().map(|x| (x - m).powi(2)).sum::<f64>();
    }
    let dfb = (k - 1) as f64;
    let dfw = (n - k) as f64;
    let f = if ssw > 0.0 {
        (ssb / dfb) / (ssw / dfw)
    } else if ssb > 0.0 {
        f64::INFINITY
    } else {
        0.0
    };
    let p = if f.is_infinite() {
        0.0
    } else {
        stats::f_sf(f, dfb, dfw)
    };
    let eta2 = if ssb + ssw > 0.0 {
        ssb / (ssb + ssw)
    } else {
        0.0
    };
    (f, p, eta2, dfb, dfw)
}

pub fn kruskal_wallis(groups: &[Vec<f64>]) -> (f64, f64) {
    let g: Vec<&Vec<f64>> = groups.iter().filter(|v| !v.is_empty()).collect();
    let k = g.len();
    if k < 2 {
        return (f64::NAN, f64::NAN);
    }
    let all: Vec<f64> = g.iter().flat_map(|v| v.iter().copied()).collect();
    let n = all.len() as f64;
    let ranks = stats::ranks(&all);
    let mut h = 0.0;
    let mut off = 0;
    for v in &g {
        let rs: f64 = ranks[off..off + v.len()].iter().sum();
        h += rs * rs / v.len() as f64;
        off += v.len();
    }
    h = 12.0 / (n * (n + 1.0)) * h - 3.0 * (n + 1.0);
    // tie correction
    let mut s = all.clone();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let mut t = 0.0;
    let mut i = 0;
    while i < s.len() {
        let mut j = i;
        while j + 1 < s.len() && s[j + 1] == s[i] {
            j += 1;
        }
        let c = (j - i + 1) as f64;
        t += c * c * c - c;
        i = j + 1;
    }
    let corr = 1.0 - t / (n * n * n - n);
    if corr > 0.0 {
        h /= corr;
    }
    (h, stats::chi2_sf(h, (k - 1) as f64))
}

/// Brown–Forsythe (median-centred Levene) test.
pub fn levene(groups: &[Vec<f64>]) -> (f64, f64) {
    let dev: Vec<Vec<f64>> = groups
        .iter()
        .filter(|v| !v.is_empty())
        .map(|v| {
            let m = stats::median(v);
            v.iter().map(|x| (x - m).abs()).collect()
        })
        .collect();
    let (f, p, _, _, _) = one_way(&dev);
    (f, p)
}

pub fn anova(
    ds: &Dataset,
    rows: &[usize],
    factor: &str,
    tests: &[usize],
    min_group: usize,
    alpha: f64,
) -> AnovaReport {
    let groups: Vec<(String, Vec<usize>)> = ds
        .group_by(rows, factor)
        .into_iter()
        .filter(|(_, r)| r.len() >= min_group.max(1))
        .collect();
    let mut out: Vec<AnovaRow> = tests
        .par_iter()
        .filter_map(|&t| {
            let info = &ds.tests[t];
            let gv: Vec<Vec<f64>> = groups
                .iter()
                .map(|(_, r)| ds.values_of(t, Some(r)))
                .collect();
            let gs: Vec<GroupStat> = groups
                .iter()
                .zip(&gv)
                .map(|((g, _), v)| GroupStat {
                    group: g.clone(),
                    n: v.len(),
                    mean: stats::mean(v),
                    std: stats::std(v),
                    median: stats::median(v),
                })
                .collect();
            let usable: Vec<Vec<f64>> = gv.into_iter().filter(|v| v.len() >= 2).collect();
            if usable.len() < 2 {
                return None;
            }
            let (f, p, eta2, dfb, dfw) = one_way(&usable);
            let (h, kp) = kruskal_wallis(&usable);
            let (lf, lp) = levene(&usable);
            let means: Vec<f64> = gs.iter().filter(|g| g.n >= 2).map(|g| g.mean).collect();
            let rng = means.iter().cloned().fold(f64::NEG_INFINITY, f64::max)
                - means.iter().cloned().fold(f64::INFINITY, f64::min);
            Some(AnovaRow {
                key: info.key.clone(),
                units: info.units.clone(),
                k: usable.len(),
                n: usable.iter().map(|v| v.len()).sum(),
                df_between: dfb,
                df_within: dfw,
                f,
                p,
                eta2,
                kw_h: h,
                kw_p: kp,
                levene_f: lf,
                levene_p: lp,
                q: f64::NAN,
                range_pct_limit: info.limit_range().map(|lr| rng / lr * 100.0),
                groups: gs,
            })
        })
        .collect();
    let q = stats::bh_adjust(&out.iter().map(|r| r.p).collect::<Vec<_>>());
    out.iter_mut().zip(q).for_each(|(r, q)| r.q = q);
    out.sort_by(|a, b| {
        b.eta2
            .partial_cmp(&a.eta2)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.p.partial_cmp(&b.p).unwrap_or(std::cmp::Ordering::Equal))
    });
    // significant first, ordered by effect size
    out.sort_by_key(|r| !(r.q < alpha));
    let n_sig = out.iter().filter(|r| r.q < alpha).count();
    AnovaReport {
        factor: factor.into(),
        groups: groups.iter().map(|(g, r)| (g.clone(), r.len())).collect(),
        rows: out,
        n_significant: n_sig,
        alpha,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn anova_known() {
        // reference: scipy.stats f_oneway F=9.264706 p=0.0023988; kruskal H=9.420685; levene(median) F=0.508475
        let a = vec![6.0, 8.0, 4.0, 5.0, 3.0, 4.0];
        let b = vec![8.0, 12.0, 9.0, 11.0, 6.0, 8.0];
        let c = vec![13.0, 9.0, 11.0, 8.0, 7.0, 12.0];
        let (f, p, _, _, _) = one_way(&[a.clone(), b.clone(), c.clone()]);
        assert!((f - 9.264706).abs() < 1e-5, "{f}");
        assert!((p - 0.002399).abs() < 1e-4, "{p}");
        let (h, _) = kruskal_wallis(&[a.clone(), b.clone(), c.clone()]);
        assert!((h - 9.420685).abs() < 1e-4, "{h}");
        let (lf, _) = levene(&[a, b, c]);
        assert!((lf - 0.508475).abs() < 1e-5, "{lf}");
    }
}
