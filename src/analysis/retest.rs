//! Retest trace: follow parts through a chain of runs (run 1, then a retest
//! of its rejects, then a retest of those rejects…), matched by a part key.
//!
//! The parts selected in run 1 are looked up in every later run by key (the
//! last row with that key, as in [`crate::data::matching`]). In each run a part
//! passes, fails, was not retested (no match while still bad) or passed
//! earlier (no match after a pass: good parts are usually not retested).

use crate::data::matching::KeySpec;
use crate::data::Dataset;
use serde::Serialize;
use std::collections::HashMap;

/// One run of the chain.
pub struct Run<'a> {
    pub ds: &'a Dataset,
    pub key: KeySpec,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Pass,
    Fail,
    NotRetested,
    PassedEarlier,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    /// passed in run 1 and never failed after (only when the start filter
    /// keeps good parts; a later run may retest it good again)
    PassedRun1,
    /// passed in its last run
    Recovered,
    /// failed in the last run of the chain
    StillBad,
    /// failed, then had no match in a later run
    NotRetested,
}

impl Outcome {
    /// Part-set name for parts with this outcome.
    pub fn set_name(&self) -> &'static str {
        match self {
            Outcome::PassedRun1 => "retest_passed_run1",
            Outcome::Recovered => "retest_recovered",
            Outcome::StillBad => "retest_still_bad",
            Outcome::NotRetested => "retest_not_retested",
        }
    }
}

#[derive(Debug, Clone)]
pub struct PartTrace {
    pub key: String,
    /// the part's row in each run
    pub rows: Vec<Option<usize>>,
    pub status: Vec<Status>,
    pub outcome: Outcome,
    /// the run (0-based) it last passed in, for recovered parts
    pub recovered_in: Option<usize>,
}

/// Follow `start` (rows of run 1) through `runs`.
pub fn trace(runs: &[Run], start: &[usize]) -> Vec<PartTrace> {
    let Some(first) = runs.first() else {
        return vec![];
    };
    // one part per key in run 1 (its last row), in run-1 order
    let mut order: Vec<String> = vec![];
    let mut row1: HashMap<String, usize> = HashMap::new();
    for &r in start {
        if let Some(k) = first.key.key(first.ds, r) {
            if row1.insert(k.clone(), r).is_none() {
                order.push(k);
            }
        }
    }
    // the last row per key in every later run
    let index: Vec<HashMap<String, usize>> = runs[1..]
        .iter()
        .map(|run| {
            let mut m = HashMap::new();
            for r in 0..run.ds.n_parts() {
                if let Some(k) = run.key.key(run.ds, r) {
                    m.insert(k, r);
                }
            }
            m
        })
        .collect();
    let pass = |ds: &Dataset, r: usize| {
        if ds.parts[r].is_pass() {
            Status::Pass
        } else {
            Status::Fail
        }
    };
    order
        .into_iter()
        .map(|key| {
            let r1 = row1[&key];
            let mut rows = vec![Some(r1)];
            let mut status = vec![pass(first.ds, r1)];
            for (k, idx) in index.iter().enumerate() {
                let r = idx.get(&key).copied();
                // a later run may retest good parts too, so go by the last result
                let passed = last_result(&rows, &status) == Some(Status::Pass);
                rows.push(r);
                status.push(match r {
                    Some(r) => pass(runs[k + 1].ds, r),
                    None if passed => Status::PassedEarlier,
                    None => Status::NotRetested,
                });
            }
            let (outcome, last) = outcome(&status, &rows);
            PartTrace {
                recovered_in: (outcome == Outcome::Recovered).then_some(last),
                key,
                rows,
                status,
                outcome,
            }
        })
        .collect()
}

/// The part's result in the last run it was found in.
fn last_result(rows: &[Option<usize>], status: &[Status]) -> Option<Status> {
    rows.iter().rposition(|r| r.is_some()).map(|j| status[j])
}

impl PartTrace {
    /// Still bad coming into run `k`: its last result before `k` was a fail.
    pub fn due(&self, k: usize) -> bool {
        k == 0 || last_result(&self.rows[..k], &self.status[..k]) == Some(Status::Fail)
    }
    /// Passed in an earlier run and found again in run `k` (a retest of good parts).
    pub fn retested_good(&self, k: usize) -> bool {
        self.rows[k].is_some() && !self.due(k)
    }
}

/// A part's outcome from its last matched run, and that run.
fn outcome(status: &[Status], rows: &[Option<usize>]) -> (Outcome, usize) {
    let last = rows.iter().rposition(|r| r.is_some()).unwrap_or(0);
    let o = match status[last] {
        Status::Pass if !status[..last].contains(&Status::Fail) => Outcome::PassedRun1,
        Status::Pass => Outcome::Recovered,
        _ if last + 1 == rows.len() => Outcome::StillBad,
        _ => Outcome::NotRetested,
    };
    (o, last)
}

/// What happened in one run.
#[derive(Debug, Clone, Serialize, Default)]
pub struct RunSummary {
    pub run: usize,
    /// parts still bad coming into this run (run 1: every traced part)
    pub due: usize,
    /// of those, matched in this run
    pub retested: usize,
    pub pass: usize,
    pub fail: usize,
    /// failing the (first) traced test in this run
    pub fail_test: usize,
    /// failing, but not on the traced test
    pub fail_other: usize,
    pub not_retested: usize,
    /// parts that had passed before, found again here (a retest of good parts)
    pub retest_good: usize,
    /// of those, failing here
    pub retest_good_fail: usize,
    /// parts recovered in runs 2..this one, over run 1's fails
    pub cum_recovered: usize,
    pub cum_recovery_rate: f64,
}

/// Per-run counts; `fails_test(run, row)` says whether the part failed the
/// traced test there.
pub fn summarize(
    parts: &[PartTrace],
    n_runs: usize,
    fails_test: &dyn Fn(usize, usize) -> bool,
) -> Vec<RunSummary> {
    let fail1 = parts.iter().filter(|p| p.status[0] == Status::Fail).count();
    let mut cum = 0;
    (0..n_runs)
        .map(|k| {
            let mut s = RunSummary {
                run: k,
                ..Default::default()
            };
            for p in parts {
                // due: run 1 = everyone; later = still bad after its last result
                if p.retested_good(k) {
                    s.retest_good += 1;
                    s.retest_good_fail += (p.status[k] == Status::Fail) as usize;
                }
                if !p.due(k) {
                    continue;
                }
                s.due += 1;
                match (p.status[k], p.rows[k]) {
                    (Status::Pass, Some(_)) => {
                        s.retested += 1;
                        s.pass += 1;
                        // count each part's first recovery only
                        if k > 0 && !p.status[..k].contains(&Status::Pass) {
                            cum += 1;
                        }
                    }
                    (Status::Fail, Some(r)) => {
                        s.retested += 1;
                        s.fail += 1;
                        if fails_test(k, r) {
                            s.fail_test += 1;
                        } else {
                            s.fail_other += 1;
                        }
                    }
                    _ => s.not_retested += 1,
                }
            }
            s.cum_recovered = cum;
            s.cum_recovery_rate = if fail1 == 0 {
                f64::NAN
            } else {
                cum as f64 / fail1 as f64
            };
            s
        })
        .collect()
}

/// Pass/fail of the traced parts tested on one site, in one run (`run`) or
/// over all runs (`run` = None: a part counts once per run it was tested in).
#[derive(Debug, Clone, Serialize)]
pub struct SiteCount {
    pub run: Option<usize>,
    pub site: u8,
    pub tested: usize,
    pub pass: usize,
    pub fail: usize,
    pub rate: f64,
    pub ci_lo: f64,
    pub ci_hi: f64,
}

/// Per-site pass rates of the traced parts, and per run (then over all
/// runs) the χ² p-value of pass/fail against site: a small p says the site
/// mattered.
#[derive(Debug, Clone, Serialize)]
pub struct SiteSummary {
    pub counts: Vec<SiteCount>,
    /// one per run, then the all-runs value; NaN with fewer than two sites
    pub p: Vec<f64>,
}

/// Counts every traced part found in a run (rejects and retested good parts
/// alike) under the site `site_of(run, row)` it was tested on.
pub fn by_site(
    parts: &[PartTrace],
    n_runs: usize,
    site_of: &dyn Fn(usize, usize) -> u8,
) -> SiteSummary {
    // (pass, fail) per run, per site; the last slot is all runs
    let mut c: Vec<std::collections::BTreeMap<u8, (usize, usize)>> =
        vec![Default::default(); n_runs + 1];
    for p in parts {
        for (k, r) in p.rows.iter().enumerate() {
            let Some(r) = *r else { continue };
            let s = site_of(k, r);
            for slot in [k, n_runs] {
                let e = c[slot].entry(s).or_default();
                if p.status[k] == Status::Pass {
                    e.0 += 1;
                } else {
                    e.1 += 1;
                }
            }
        }
    }
    let mut counts = vec![];
    let mut pv = vec![];
    for (k, m) in c.iter().enumerate() {
        for (&site, &(pass, fail)) in m {
            let tested = pass + fail;
            let (ci_lo, ci_hi) = wilson(pass, tested);
            counts.push(SiteCount {
                run: (k < n_runs).then_some(k),
                site,
                tested,
                pass,
                fail,
                rate: pass as f64 / tested as f64,
                ci_lo,
                ci_hi,
            });
        }
        pv.push(chi2_pass_fail(m.values().copied()));
    }
    SiteSummary { counts, p: pv }
}

/// χ² test of independence on a groups × {pass, fail} table.
fn chi2_pass_fail(rows: impl Iterator<Item = (usize, usize)>) -> f64 {
    let rows: Vec<(f64, f64)> = rows
        .filter(|r| r.0 + r.1 > 0)
        .map(|(a, b)| (a as f64, b as f64))
        .collect();
    let (tp, tf) = rows.iter().fold((0.0, 0.0), |s, r| (s.0 + r.0, s.1 + r.1));
    let n = tp + tf;
    if rows.len() < 2 || tp == 0.0 || tf == 0.0 {
        return f64::NAN;
    }
    let x: f64 = rows
        .iter()
        .map(|&(a, b)| {
            let m = a + b;
            let (ea, eb) = (m * tp / n, m * tf / n);
            (a - ea).powi(2) / ea + (b - eb).powi(2) / eb
        })
        .sum();
    crate::analysis::stats::chi2_sf(x, (rows.len() - 1) as f64)
}

/// Wilson score interval (95 %) of `k` successes in `n`.
pub fn wilson(k: usize, n: usize) -> (f64, f64) {
    if n == 0 {
        return (0.0, 1.0);
    }
    let (z, n_, p) = (1.96f64, n as f64, k as f64 / n as f64);
    let d = 1.0 + z * z / n_;
    let c = (p + z * z / (2.0 * n_)) / d;
    let h = z * ((p * (1.0 - p) + z * z / (4.0 * n_)) / n_).sqrt() / d;
    ((c - h).max(0.0), (c + h).min(1.0))
}

/// Recovery rate of the parts in one value range.
#[derive(Debug, Clone, Serialize)]
pub struct RateBand {
    /// marginal | hard_bad | extreme for zones; empty for bins
    pub kind: String,
    /// observed value range of its parts
    pub lo: f64,
    pub hi: f64,
    /// for the hard-bad zone: halfway to the nearest part outside it
    pub safe_lo: Option<f64>,
    pub safe_hi: Option<f64>,
    pub n: usize,
    pub recovered: usize,
    pub rate: f64,
    pub ci_lo: f64,
    pub ci_hi: f64,
}

fn band(kind: &str, pts: &[(f64, bool)]) -> RateBand {
    let k = pts.iter().filter(|p| p.1).count();
    let (ci_lo, ci_hi) = wilson(k, pts.len());
    RateBand {
        kind: kind.into(),
        lo: pts.first().map(|p| p.0).unwrap_or(f64::NAN),
        hi: pts.last().map(|p| p.0).unwrap_or(f64::NAN),
        safe_lo: None,
        safe_hi: None,
        n: pts.len(),
        recovered: k,
        rate: if pts.is_empty() {
            f64::NAN
        } else {
            k as f64 / pts.len() as f64
        },
        ci_lo,
        ci_hi,
    }
}

/// Recovery rate in about `n_bins` value bins of equal part count.
pub fn recovery_bins(points: &[(f64, bool)], n_bins: usize) -> Vec<RateBand> {
    let mut p: Vec<(f64, bool)> = points.iter().copied().filter(|p| p.0.is_finite()).collect();
    p.sort_by(|a, b| a.0.total_cmp(&b.0));
    if p.is_empty() {
        return vec![];
    }
    let size = p.len().div_ceil(n_bins.max(1)).max(5);
    let mut out = vec![];
    let mut i = 0;
    while i < p.len() {
        // never split equal values between bins
        let mut j = (i + size).min(p.len());
        while j < p.len() && p[j].0 == p[j - 1].0 {
            j += 1;
        }
        out.push(band("", &p[i..j]));
        i = j;
    }
    out
}

/// Splits failing values into recovery zones. The sorted values are cut into
/// three bands (lower, middle, upper; the outer ones may be empty) at the two
/// points that best fit a separate recovery rate per band (binomial maximum
/// likelihood), so the cuts fall where the recovery rate changes. The middle
/// band is the hard-bad zone when it holds at least `min_n` parts, recovers
/// less often than either side, and its recovery rate has a Wilson 95 % upper
/// bound ≤ `max_rate`. The other bands are marginal (nearer the test limits
/// than the hard-bad parts) or extreme (farther, e.g. clamped readings).
/// Without a hard-bad zone, everything is one marginal zone.
pub fn recovery_zones(
    points: &[(f64, bool)],
    max_rate: f64,
    min_n: usize,
    lo: Option<f64>,
    hi: Option<f64>,
) -> Vec<RateBand> {
    let mut p: Vec<(f64, bool)> = points.iter().copied().filter(|p| p.0.is_finite()).collect();
    p.sort_by(|a, b| a.0.total_cmp(&b.0));
    let n = p.len();
    let mut cum = vec![0usize; n + 1];
    for (i, x) in p.iter().enumerate() {
        cum[i + 1] = cum[i] + x.1 as usize;
    }
    // binomial log-likelihood of rows a..b at their own rate
    let ll = |a: usize, b: usize| -> f64 {
        let (m, k) = ((b - a) as f64, (cum[b] - cum[a]) as f64);
        let t = |x: f64| if x > 0.0 { x * (x / m).ln() } else { 0.0 };
        if m == 0.0 {
            0.0
        } else {
            t(k) + t(m - k)
        }
    };
    let rate = |a: usize, b: usize| (cum[b] - cum[a]) as f64 / (b - a) as f64;
    // cut points: never between equal values
    let cuts: Vec<usize> = (0..=n)
        .filter(|&c| c == 0 || c == n || p[c - 1].0 < p[c].0)
        .collect();
    let mut best: Option<(usize, usize, f64)> = None;
    for (ci, &i) in cuts.iter().enumerate() {
        for &j in &cuts[ci..] {
            if j - i < min_n.max(1) {
                continue;
            }
            let m = rate(i, j);
            if (i > 0 && rate(0, i) <= m) || (j < n && rate(j, n) <= m) {
                continue;
            }
            let v = ll(0, i) + ll(i, j) + ll(j, n);
            if best.is_none_or(|b| v > b.2 + 1e-9) {
                best = Some((i, j, v));
            }
        }
    }
    let best = best
        .filter(|&(i, j, _)| wilson(cum[j] - cum[i], j - i).1 <= max_rate)
        .map(|(i, j, _)| (i, j - 1));
    let Some((i, j)) = best else {
        return if p.is_empty() {
            vec![]
        } else {
            vec![band("marginal", &p)]
        };
    };
    let mut hard = band("hard_bad", &p[i..=j]);
    hard.safe_lo = (i > 0).then(|| (p[i - 1].0 + p[i].0) / 2.0);
    hard.safe_hi = (j + 1 < n).then(|| (p[j].0 + p[j + 1].0) / 2.0);
    // the side whose values sit nearer a limit is the marginal one
    let dist = |s: &[(f64, bool)]| -> f64 {
        let d: Vec<f64> = s
            .iter()
            .map(|x| {
                [lo, hi]
                    .iter()
                    .flatten()
                    .map(|l| (x.0 - l).abs())
                    .fold(f64::INFINITY, f64::min)
            })
            .collect();
        d.iter().sum::<f64>() / d.len().max(1) as f64
    };
    let d_hard = dist(&p[i..=j]);
    let side = |s: &[(f64, bool)]| {
        if dist(s) < d_hard {
            "marginal"
        } else {
            "extreme"
        }
    };
    let (below, above) = (&p[..i], &p[j + 1..]);
    let mut out = vec![];
    if !below.is_empty() {
        out.push(band(side(below), below));
    }
    out.push(hard);
    if !above.is_empty() {
        out.push(band(side(above), above));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outcomes_from_statuses() {
        use Status::*;
        let o = |s: Vec<Status>, r: Vec<Option<usize>>| outcome(&s, &r);
        assert_eq!(
            o(
                vec![Fail, Pass, PassedEarlier],
                vec![Some(0), Some(1), None]
            ),
            (Outcome::Recovered, 1)
        );
        assert_eq!(
            o(vec![Fail, Fail, Fail], vec![Some(0), Some(1), Some(2)]).0,
            Outcome::StillBad
        );
        assert_eq!(
            o(
                vec![Fail, NotRetested, NotRetested],
                vec![Some(0), None, None]
            )
            .0,
            Outcome::NotRetested
        );
        assert_eq!(
            o(vec![Pass, PassedEarlier], vec![Some(0), None]).0,
            Outcome::PassedRun1
        );
        // good in run 1 and again on a retest of good parts
        assert_eq!(
            o(
                vec![Pass, PassedEarlier, Pass],
                vec![Some(0), None, Some(2)]
            )
            .0,
            Outcome::PassedRun1
        );
    }

    #[test]
    fn retest_of_good_parts_and_sites() {
        use Status::*;
        // run 1 fail, run 2 pass, run 3 (reject retest) absent, run 4 (good
        // retest) fail, run 5 fail again
        let p = PartTrace {
            key: "a".into(),
            rows: vec![Some(0), Some(0), None, Some(0), Some(0)],
            status: vec![Fail, Pass, PassedEarlier, Fail, Fail],
            outcome: Outcome::StillBad,
            recovered_in: None,
        };
        assert!(p.due(1) && !p.due(2) && !p.due(3) && p.due(4));
        assert!(p.retested_good(3) && !p.retested_good(4));
        let q = PartTrace {
            key: "b".into(),
            rows: vec![Some(1), Some(1), Some(1), None, None],
            status: vec![Fail, Fail, Pass, PassedEarlier, PassedEarlier],
            outcome: Outcome::Recovered,
            recovered_in: Some(2),
        };
        let s = summarize(&[p.clone(), q.clone()], 5, &|_, _| true);
        assert_eq!((s[3].retest_good, s[3].retest_good_fail), (1, 1));
        assert_eq!((s[4].due, s[4].fail), (1, 1));
        // first recoveries only: a in run 2, b in run 3
        assert_eq!(s[4].cum_recovered, 2);
        // a on site 1, b on site 2
        let bs = by_site(&[p, q], 5, &|_, r| r as u8 + 1);
        let all: Vec<_> = bs.counts.iter().filter(|c| c.run.is_none()).collect();
        assert_eq!((all[0].site, all[0].tested, all[0].pass), (1, 4, 1));
        assert_eq!((all[1].site, all[1].tested, all[1].pass), (2, 3, 1));
        assert_eq!(bs.p.len(), 6);
    }

    #[test]
    fn zones_find_the_band_that_never_recovers() {
        // limit 10: marginal 10..11 mostly recover, 13..17 never, clamp 30 mostly
        let mut pts = vec![];
        for i in 0..40 {
            pts.push((10.0 + i as f64 * 0.025, i % 5 != 0));
            pts.push((13.0 + i as f64 * 0.1, false));
        }
        for i in 0..20 {
            pts.push((30.0, i % 4 != 0));
        }
        let z = recovery_zones(&pts, 0.10, 10, Some(0.0), Some(10.0));
        let kinds: Vec<&str> = z.iter().map(|b| b.kind.as_str()).collect();
        assert_eq!(kinds, ["marginal", "hard_bad", "extreme"]);
        let h = &z[1];
        assert_eq!((h.n, h.recovered), (40, 0));
        assert!((h.lo - 13.0).abs() < 1e-9 && (h.hi - 16.9).abs() < 1e-9);
        assert!(h.ci_hi <= 0.10);
        assert!((h.safe_lo.unwrap() - (10.975 + 13.0) / 2.0).abs() < 1e-9);
        assert!((h.safe_hi.unwrap() - (16.9 + 30.0) / 2.0).abs() < 1e-9);
        assert_eq!((z[0].n, z[0].recovered), (40, 32));
        assert_eq!((z[2].n, z[2].recovered), (20, 15));
        // too few parts for the bound: no hard-bad zone
        assert_eq!(
            recovery_zones(&pts[..8], 0.10, 10, None, Some(10.0)).len(),
            1
        );
        // bins never split equal values and carry Wilson bands
        let b = recovery_bins(&pts, 10);
        assert_eq!(b.iter().map(|x| x.n).sum::<usize>(), 100);
        assert!(b.iter().all(|x| x.ci_lo <= x.rate && x.rate <= x.ci_hi));
        assert_eq!(b.iter().filter(|x| x.lo == 30.0).count(), 1);
        let (l, u) = wilson(0, 35);
        assert!(l == 0.0 && u < 0.1 && u > 0.09);
    }
}
