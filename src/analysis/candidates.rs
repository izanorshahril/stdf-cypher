//! What bad candidates have in common.
//!
//! Bad candidates are parts that passed in one file and failed in a later one
//! (found by matching the two files); the reference is every other part of
//! the same population in the first file. Next to the per-test marker ranking
//! ([`super::markers`]) this answers two questions:
//!
//! * **indicators**: would a per-part outlier score (robust z, PAT, NNR,
//!   Mahalanobis, Isolation Forest, sigma-deviation count) have caught the
//!   candidates? Each score gets its rank AUC, the share of candidates above
//!   the reference's 95th percentile (capture) and that share over 5 % (lift);
//! * **factors**: are candidates concentrated on a site, wafer, wafer zone,
//!   reticle position…? A χ² test on the candidate rate per level, with
//!   Cramér's V and the level with the highest rate.

use super::markers::auc;
use super::stats;
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize)]
pub struct Indicator {
    pub key: String,
    pub name: String,
    /// P(candidate score > reference score); 0.5 = no better than chance
    pub auc: f64,
    /// share of candidates above the reference's 95th percentile
    pub capture: f64,
    /// capture ÷ 5 %
    pub lift: f64,
    /// share flagged at the method's usual threshold
    pub flagged_ref: Option<f64>,
    pub flagged_aff: Option<f64>,
    pub n_ref: usize,
    pub n_aff: usize,
}

/// One per-part score for the reference and the candidates (NaN = no value);
/// `flag` is the method's usual threshold, if it has one.
pub fn indicator(
    key: &str,
    name: &str,
    refv: &[f64],
    affv: &[f64],
    flag: Option<f64>,
) -> Option<Indicator> {
    let r: Vec<f64> = refv.iter().copied().filter(|v| v.is_finite()).collect();
    let a: Vec<f64> = affv.iter().copied().filter(|v| v.is_finite()).collect();
    if r.len() < 10 || a.is_empty() {
        return None;
    }
    let p95 = stats::quantile_sorted(&stats::sorted(&r), 0.95);
    let share = |v: &[f64], f: &dyn Fn(f64) -> bool| {
        v.iter().filter(|&&x| f(x)).count() as f64 / v.len() as f64
    };
    let capture = share(&a, &|x| x > p95);
    Some(Indicator {
        key: key.into(),
        name: name.into(),
        auc: auc(&a, &r),
        capture,
        lift: capture / 0.05,
        flagged_ref: flag.map(|t| share(&r, &|x| x >= t)),
        flagged_aff: flag.map(|t| share(&a, &|x| x >= t)),
        n_ref: r.len(),
        n_aff: a.len(),
    })
}

#[derive(Debug, Clone, Serialize)]
pub struct Level {
    pub level: String,
    pub n: usize,
    pub n_aff: usize,
    /// candidates per part in this level
    pub rate: f64,
    /// rate ÷ overall candidate rate
    pub lift: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Factor {
    pub key: String,
    pub name: String,
    pub chi2: f64,
    pub p: f64,
    /// effect size of the association, 0–1
    pub cramers_v: f64,
    pub levels: Vec<Level>,
}

/// Candidate rate per level of one factor. `labels` gives the level of each
/// part (None = unknown, left out), `aff` whether it is a candidate.
pub fn factor(key: &str, name: &str, labels: &[Option<String>], aff: &[bool]) -> Option<Factor> {
    let mut m: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    for (l, &a) in labels.iter().zip(aff) {
        if let Some(l) = l {
            let e = m.entry(l.clone()).or_default();
            e.0 += 1;
            e.1 += a as usize;
        }
    }
    let n: usize = m.values().map(|v| v.0).sum();
    let k: usize = m.values().map(|v| v.1).sum();
    if m.len() < 2 || n == 0 || k == 0 || k == n {
        return None;
    }
    let rate = k as f64 / n as f64;
    // 2 × L contingency table: candidate / not candidate per level
    let mut chi2 = 0.0;
    for &(nl, kl) in m.values() {
        for (obs, exp) in [
            (kl as f64, nl as f64 * rate),
            ((nl - kl) as f64, nl as f64 * (1.0 - rate)),
        ] {
            if exp > 0.0 {
                chi2 += (obs - exp).powi(2) / exp;
            }
        }
    }
    let df = (m.len() - 1) as f64;
    let mut levels: Vec<Level> = m
        .into_iter()
        .map(|(level, (nl, kl))| Level {
            level,
            n: nl,
            n_aff: kl,
            rate: kl as f64 / nl as f64,
            lift: kl as f64 / nl as f64 / rate,
        })
        .collect();
    levels.sort_by(|a, b| {
        b.rate
            .partial_cmp(&a.rate)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(b.n.cmp(&a.n))
    });
    Some(Factor {
        key: key.into(),
        name: name.into(),
        chi2,
        p: stats::chi2_sf(chi2, df),
        cramers_v: (chi2 / n as f64).sqrt().min(1.0),
        levels,
    })
}

/// Radial wafer zone of each die: centre (< 50 % of the wafer radius),
/// middle (< 80 %) or edge, from the die positions of its own wafer.
pub fn zones(pos: &[Option<(String, f64, f64)>]) -> Vec<Option<String>> {
    let mut ext: BTreeMap<&str, (f64, f64, f64, f64)> = BTreeMap::new();
    for (w, x, y) in pos.iter().flatten() {
        let e = ext
            .entry(w.as_str())
            .or_insert((f64::MAX, f64::MIN, f64::MAX, f64::MIN));
        *e = (e.0.min(*x), e.1.max(*x), e.2.min(*y), e.3.max(*y));
    }
    let radius: BTreeMap<&str, (f64, f64, f64)> = ext
        .iter()
        .map(|(w, &(x0, x1, y0, y1))| {
            let (cx, cy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
            let r = pos
                .iter()
                .flatten()
                .filter(|p| p.0 == *w)
                .map(|p| ((p.1 - cx).powi(2) + (p.2 - cy).powi(2)).sqrt())
                .fold(0.0, f64::max);
            (*w, (cx, cy, r))
        })
        .collect();
    pos.iter()
        .map(|p| {
            let (w, x, y) = p.as_ref()?;
            let &(cx, cy, r) = radius.get(w.as_str())?;
            if r <= 0.0 {
                return None;
            }
            let d = ((x - cx).powi(2) + (y - cy).powi(2)).sqrt() / r;
            Some(
                if d < 0.5 {
                    "1 centre"
                } else if d < 0.8 {
                    "2 middle"
                } else {
                    "3 edge"
                }
                .to_string(),
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn factor_finds_the_concentrated_level() {
        // 20 parts per site, candidates only on site 3
        let labels: Vec<Option<String>> =
            (0..80).map(|i| Some(format!("site {}", i / 20))).collect();
        let aff: Vec<bool> = (0..80).map(|i| i / 20 == 3 && i % 2 == 0).collect();
        let f = factor("site", "site", &labels, &aff).unwrap();
        assert_eq!(f.levels[0].level, "site 3");
        assert!(f.p < 1e-6 && f.cramers_v > 0.5);
    }

    #[test]
    fn indicator_capture_and_lift() {
        let r: Vec<f64> = (0..1000).map(|i| i as f64).collect();
        let a = vec![990.0, 995.0, 10.0, 999.0];
        let ind = indicator("x", "x", &r, &a, None).unwrap();
        assert!((ind.capture - 0.75).abs() < 1e-9);
        assert!((ind.lift - 15.0).abs() < 1e-9);
    }

    #[test]
    fn zones_put_the_rim_at_the_edge() {
        let pos: Vec<Option<(String, f64, f64)>> = [
            (0.0, 0.0),
            (10.0, 0.0),
            (-10.0, 0.0),
            (0.0, 10.0),
            (0.0, -10.0),
            (6.0, 0.0),
        ]
        .iter()
        .map(|&(x, y)| Some(("w".to_string(), x, y)))
        .collect();
        let z = zones(&pos);
        assert_eq!(z[0].as_deref(), Some("1 centre"));
        assert_eq!(z[1].as_deref(), Some("3 edge"));
        assert_eq!(z[5].as_deref(), Some("2 middle"));
    }
}
