//! Signature check: learn a failure signature in one lot, prune it with clean
//! lots, and screen other files with what is left.
//!
//! * **learn**: in the lot with the issue, bad candidates (good parts that
//!   failed later) against the normal parts (the other good parts, or the
//!   ones the later file confirmed good). Every test gets the threshold rule
//!   that best separates them (`x > c`, `x < c`, or outside median ± k·σ),
//!   chosen by Youden's J = capture − false-alarm rate;
//! * **validate**: good parts known to be fine (a clean lot, parts that
//!   passed the later file) form the reference pool, so every part of it
//!   that matches a rule is a false alarm. Each source's rate is reported;
//!   the pooled rate sets the score (capture − the larger false-alarm rate,
//!   normal parts or pool) and the verdict: keep, drop (the pool shows the
//!   same pattern), not tested (the pool never reaches the rule's region),
//!   or weak;
//! * **combine**: the best kept rules form one signature, "at least m of n
//!   rules", with m chosen by the same score;
//! * **screen**: the signature's hit rate in another file against the
//!   reference pool (one-sided Fisher exact test) and against lot A.

use super::stats;
use crate::data::{Dataset, TestInfo};
use serde::Serialize;
use statrs::distribution::{DiscreteCDF, Hypergeometric};

/// A part population in one dataset.
pub struct Pop<'a> {
    pub name: String,
    pub ds: &'a Dataset,
    pub rows: Vec<usize>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Above,
    Below,
    Outside,
}

/// A threshold rule on one test (values in the test's units).
#[derive(Debug, Clone, Serialize)]
pub struct Rule {
    pub kind: Kind,
    /// threshold, or the lower edge of the normal band for `Outside`
    pub a: f64,
    /// upper edge of the normal band for `Outside`
    pub b: f64,
}

impl Rule {
    pub fn hit(&self, v: f64) -> bool {
        v.is_finite()
            && match self.kind {
                Kind::Above => v > self.a,
                Kind::Below => v < self.a,
                Kind::Outside => v < self.a || v > self.b,
            }
    }
    pub fn text(&self) -> String {
        match self.kind {
            Kind::Above => format!("> {}", g(self.a)),
            Kind::Below => format!("< {}", g(self.a)),
            Kind::Outside => format!("outside {} … {}", g(self.a), g(self.b)),
        }
    }
    /// The rule in the filter language, e.g. `t[1290] > 20.61`.
    pub fn filter(&self, test: &str) -> String {
        match self.kind {
            Kind::Above => format!("{test} > {}", g(self.a)),
            Kind::Below => format!("{test} < {}", g(self.a)),
            Kind::Outside => format!("({test} < {} or {test} > {})", g(self.a), g(self.b)),
        }
    }
}

/// About 5 significant digits, without trailing zeros.
fn g(v: f64) -> String {
    let d = if v == 0.0 {
        0
    } else {
        (4 - v.abs().log10().floor() as i32).clamp(0, 9) as usize
    };
    let s = format!("{v:.d$}");
    if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        s
    }
}

/// Hit rate of one rule in one reference source (or the pooled reference).
#[derive(Debug, Clone, Serialize)]
pub struct LotRate {
    pub lot: String,
    pub n: usize,
    pub hits: usize,
    pub rate: f64,
    /// source median − normal median, in normal σ, positive toward the rule
    pub shift_sigma: f64,
    /// limits differ from lot A (beyond 0.1 % of the limit window)
    pub limits_changed: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct RuleRow {
    pub key: String,
    pub num: u32,
    pub units: String,
    pub rule: Rule,
    pub text: String,
    pub filter: String,
    /// share of bad candidates matching the rule
    pub capture: f64,
    /// share of the normal parts (the learning group) matching it
    pub fa_a: f64,
    /// false-alarm rate of the pooled reference (None = no data)
    pub fa_ref: Option<f64>,
    /// the pooled reference
    pub pool: Option<LotRate>,
    /// each reference source on its own
    pub sources: Vec<LotRate>,
    /// Youden J in lot A
    pub j_a: f64,
    /// capture − max(normal, pooled false-alarm rate)
    pub score: f64,
    /// keep | caution | drop | untested | weak | no data
    pub verdict: String,
    pub why: String,
    /// the reference source that forced a drop (or "the reference pool")
    pub drop_by: Option<String>,
}

/// Best single-cut rule for `pos` against `neg`: (rule, J).
fn best_cut(pos: &[f64], neg: &[f64], kind: Kind) -> Option<(f64, f64)> {
    let (p, n) = (stats::sorted(pos), stats::sorted(neg));
    if p.is_empty() || n.is_empty() {
        return None;
    }
    let mut cuts: Vec<f64> = p.iter().chain(&n).copied().collect();
    cuts.sort_by(|a, b| a.partial_cmp(b).unwrap());
    cuts.dedup();
    let share_above =
        |s: &[f64], c: f64| (s.len() - s.partition_point(|&x| x <= c)) as f64 / s.len() as f64;
    let share_below = |s: &[f64], c: f64| s.partition_point(|&x| x < c) as f64 / s.len() as f64;
    let mut best: Option<(f64, f64)> = None;
    for w in cuts.windows(2) {
        let c = (w[0] + w[1]) / 2.0;
        let j = match kind {
            Kind::Below => share_below(&p, c) - share_below(&n, c),
            _ => share_above(&p, c) - share_above(&n, c),
        };
        if best.map(|b| j > b.1).unwrap_or(true) {
            best = Some((c, j));
        }
    }
    best
}

/// The rule that best separates `pos` from `neg` (Youden's J).
pub fn learn(pos: &[f64], neg: &[f64]) -> Option<(Rule, f64)> {
    let mut best: Option<(Rule, f64)> = None;
    let mut consider = |r: Rule, j: f64, bonus: f64| {
        if best.as_ref().map(|b| j > b.1 + bonus).unwrap_or(true) {
            best = Some((r, j));
        }
    };
    for kind in [Kind::Above, Kind::Below] {
        if let Some((c, j)) = best_cut(pos, neg, kind) {
            consider(Rule { kind, a: c, b: c }, j, 0.0);
        }
    }
    // a band around the normal parts, for markers that spread both ways;
    // preferred only when clearly better than one cut
    let (m, s) = (stats::median(neg), stats::robust_sigma(neg));
    if s > 0.0 {
        let d = |v: &[f64]| v.iter().map(|x| (x - m).abs() / s).collect::<Vec<_>>();
        if let Some((k, j)) = best_cut(&d(pos), &d(neg), Kind::Above) {
            consider(
                Rule {
                    kind: Kind::Outside,
                    a: m - k * s,
                    b: m + k * s,
                },
                j,
                0.05,
            );
        }
    }
    // snap the cuts to the digits shown, so the rule and its filter text agree
    let snap = |v: f64| g(v).parse().unwrap_or(v);
    best.filter(|b| b.1 > 0.0).map(|(mut r, j)| {
        (r.a, r.b) = (snap(r.a), snap(r.b));
        (r, j)
    })
}

pub fn rate(rule: &Rule, v: &[f64]) -> f64 {
    if v.is_empty() {
        return f64::NAN;
    }
    v.iter().filter(|&&x| rule.hit(x)).count() as f64 / v.len() as f64
}

fn limits_changed(a: &TestInfo, b: &TestInfo) -> bool {
    let w = match (a.lo, a.hi) {
        (Some(l), Some(h)) if h > l => h - l,
        _ => 1.0,
    };
    let diff = |x: Option<f64>, y: Option<f64>| match (x, y) {
        (Some(x), Some(y)) => (x - y).abs() > 1e-3 * w,
        (None, None) => false,
        _ => true,
    };
    diff(a.lo, b.lo) || diff(a.hi, b.hi)
}

/// Learn and validate one rule per test. `tests` are (test in lot A, the
/// same test in each reference source).
pub fn rules(
    cand: &Pop,
    normal: &Pop,
    sources: &[Pop],
    tests: &[(usize, Vec<Option<usize>>)],
    min_n: usize,
) -> Vec<RuleRow> {
    use rayon::prelude::*;
    let ds = normal.ds;
    let mut rows: Vec<RuleRow> = tests
        .par_iter()
        .filter_map(|(t, ct)| {
            let pos = cand.ds.values_of(*t, Some(&cand.rows));
            let neg = ds.values_of(*t, Some(&normal.rows));
            if pos.len() < min_n || neg.len() < 10 {
                return None;
            }
            let (rule, j_a) = learn(&pos, &neg)?;
            let info = &ds.tests[*t];
            let (m, s) = (stats::median(&neg), stats::robust_sigma(&neg));
            let toward = match rule.kind {
                Kind::Above => 1.0,
                Kind::Below => -1.0,
                Kind::Outside => 0.0,
            };
            let rate_of = |name: &str, v: &[f64], changed: bool| {
                let hits = v.iter().filter(|&&x| rule.hit(x)).count();
                let d = if s > 0.0 {
                    (stats::median(v) - m) / s
                } else {
                    0.0
                };
                LotRate {
                    lot: name.to_string(),
                    n: v.len(),
                    hits,
                    rate: hits as f64 / v.len() as f64,
                    shift_sigma: if toward == 0.0 { d.abs() } else { d * toward },
                    limits_changed: changed,
                }
            };
            let (mut pooled, mut changed_any) = (vec![], false);
            let lots: Vec<LotRate> = sources
                .iter()
                .zip(ct)
                .filter_map(|(c, t2)| {
                    let t2 = (*t2)?;
                    let v = c.ds.values_of(t2, Some(&c.rows));
                    if v.len() < 10 {
                        return None;
                    }
                    let changed = limits_changed(info, &c.ds.tests[t2]);
                    changed_any |= changed;
                    pooled.extend_from_slice(&v);
                    Some(rate_of(&c.name, &v, changed))
                })
                .collect();
            let pool = (!lots.is_empty()).then(|| rate_of("reference pool", &pooled, changed_any));
            let capture = rate(&rule, &pos);
            let fa_a = rate(&rule, &neg);
            let fa_ref = pool.as_ref().map(|p| p.rate);
            let score = capture - fa_a.max(fa_ref.unwrap_or(0.0));
            let (verdict, why, drop_by) = verdict(capture, fa_a, neg.len(), pool.as_ref(), &lots);
            Some(RuleRow {
                key: info.key.clone(),
                num: info.num,
                units: info.units.clone(),
                text: rule.text(),
                filter: rule.filter(&test_ref(ds, *t)),
                rule,
                capture,
                fa_a,
                fa_ref,
                pool,
                sources: lots,
                j_a,
                score,
                verdict: verdict.into(),
                why,
                drop_by,
            })
        })
        .collect();
    rows.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    rows
}

/// How the pooled reference judges a rule learned in lot A.
/// Each source on its own can drop a rule, so a large pool cannot outvote a
/// source that shows the pattern; the pooled rate decides everything else.
/// Returns (verdict, why, the source that forced a drop).
fn verdict(
    capture: f64,
    fa_a: f64,
    n_a: usize,
    pool: Option<&LotRate>,
    sources: &[LotRate],
) -> (&'static str, String, Option<String>) {
    let Some(l) = pool else {
        return (
            "no data",
            "no reference part has results for this test".into(),
            None,
        );
    };
    // a reference matching the rule nearly as often as the candidates do,
    // or clearly more often than the normal parts, contradicts it
    let k_a = (fa_a * n_a as f64).round() as usize;
    let sig = |l: &LotRate| l.hits > 0 && fisher_greater(l.hits, l.n, k_a, n_a) < 0.01;
    let drops =
        |l: &LotRate| l.rate >= 0.5 * capture || (l.hits >= 3 && l.rate >= 2.0 * fa_a && sig(l));
    let src = sources.iter().filter(|x| drops(x)).max_by(|a, b| {
        a.rate
            .partial_cmp(&b.rate)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    if let Some(x) = src.or(drops(l).then_some(l)) {
        let by = if std::ptr::eq(x, l) {
            "the reference pool".to_string()
        } else {
            x.lot.clone()
        };
        return (
            "drop",
            format!(
                "{by} matches it in {:.1}% of good parts without failures (normal parts {:.1}%)",
                x.rate * 100.0,
                fa_a * 100.0
            ),
            Some(by),
        );
    }
    let sig = sig(l);
    // the reference sits on the safe side: few hits there prove little
    if l.shift_sigma < -0.5 {
        return (
            "untested",
            format!(
                "the reference pool sits {:.1}σ away from the rule's side, so it never reaches the region",
                -l.shift_sigma
            ),
            None,
        );
    }
    if capture - fa_a < 0.2 {
        return (
            "weak",
            "the rule separates the candidates from the normal parts only weakly".into(),
            None,
        );
    }
    if sig {
        return ("caution", format!("the reference pool matches it more often than the normal parts ({:.1}% vs {:.1}%), still well below the candidates ({:.0}%)", l.rate * 100.0, fa_a * 100.0, capture * 100.0), None);
    }
    (
        "keep",
        format!(
            "the reference pool matches it no more often than the normal parts ({:.1}% vs {:.1}%)",
            l.rate * 100.0,
            fa_a * 100.0
        ),
        None,
    )
}

/// Filter-language reference to a test: `t[1290]` when its number is unique.
pub fn test_ref(ds: &Dataset, t: usize) -> String {
    let num = ds.tests[t].num;
    if ds.tests.iter().filter(|x| x.num == num).count() == 1 {
        format!("t[{num}]")
    } else {
        format!("t[\"{}\"]", ds.tests[t].key)
    }
}

/// One-sided Fisher exact test: is `k1 / n1` higher than `k2 / n2`?
pub fn fisher_greater(k1: usize, n1: usize, k2: usize, n2: usize) -> f64 {
    if n1 == 0 || k1 == 0 {
        return 1.0;
    }
    let (n, k) = ((n1 + n2) as u64, (k1 + k2) as u64);
    Hypergeometric::new(n, k, n1 as u64)
        .map(|h| h.sf(k1 as u64 - 1))
        .unwrap_or(f64::NAN)
}

/// The combined signature: a part matches when it hits at least `m` of the
/// rules.
#[derive(Debug, Clone, Serialize)]
pub struct Signature {
    pub rules: Vec<RuleRow>,
    pub m: usize,
    pub capture: f64,
    pub fa_a: f64,
    /// pooled reference false-alarm rate
    pub fa_ref: Option<f64>,
    /// false-alarm rate per reference source
    pub sources: Vec<SourceRate>,
    pub score: f64,
    /// the signature in the filter language (only for m = 1)
    pub filter: Option<String>,
    /// the greedy search: each rule as it was added, with m and the score after it
    pub steps: Vec<Step>,
    /// usable (keep / caution) rules the search chose from
    pub considered: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct Step {
    pub key: String,
    pub m: usize,
    pub score: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct SourceRate {
    pub name: String,
    pub n: usize,
    pub rate: f64,
}

/// Rules hit per row of `pop`, for the signature's rules paired into `pop`'s
/// dataset by `pair` (None = test missing).
pub fn hits(pop: &Pop, tests: &[Option<usize>], rules: &[RuleRow]) -> Vec<usize> {
    pop.rows
        .iter()
        .map(|&r| {
            rules
                .iter()
                .zip(tests)
                .filter(|(rr, t)| t.map(|t| rr.rule.hit(pop.ds.value(t, r))).unwrap_or(false))
                .count()
        })
        .collect()
}

/// Which of `rules` each part of `pop` hits (rules paired into `pop`'s dataset
/// by `tests`; None = test missing).
pub fn hit_matrix(pop: &Pop, tests: &[Option<usize>], rules: &[RuleRow]) -> Vec<Vec<bool>> {
    pop.rows
        .iter()
        .map(|&r| {
            rules
                .iter()
                .zip(tests)
                .map(|(rr, t)| t.map(|t| rr.rule.hit(pop.ds.value(t, r))).unwrap_or(false))
                .collect()
        })
        .collect()
}

/// Rules hit per part, counting only the rules in `sel`.
fn counts(mat: &[Vec<bool>], sel: &[usize]) -> Vec<usize> {
    mat.iter()
        .map(|row| sel.iter().filter(|&&i| row[i]).count())
        .collect()
}

/// Greedy signature: starting from nothing, add the rule that most improves
/// the score (capture − the larger of the normal and pooled false-alarm
/// rates, with m re-chosen each time) and stop when no rule improves it or
/// `max_rules` are in. The first rule is the best single rule, so the result
/// is never worse than it.
pub fn select(
    rules: Vec<RuleRow>,
    cand: &[Vec<bool>],
    normal: &[Vec<bool>],
    sources: &[(String, Vec<Vec<bool>>)],
    max_rules: usize,
) -> Signature {
    let pooled: Vec<Vec<bool>> = sources.iter().flat_map(|s| s.1.iter().cloned()).collect();
    // best (m, score) for a set of rules, as combine() scores it
    let eval = |sel: &[usize]| {
        let (hc, hn, hp) = (counts(cand, sel), counts(normal, sel), counts(&pooled, sel));
        let mut best = (1, f64::NEG_INFINITY);
        for m in 1..=sel.len() {
            let fa = if hp.is_empty() {
                0.0
            } else {
                share_at_least(&hp, m)
            };
            let v = share_at_least(&hc, m) - share_at_least(&hn, m).max(fa);
            if v > best.1 {
                best = (m, v);
            }
        }
        best
    };
    let (mut sel, mut cur, mut steps) = (vec![], f64::NEG_INFINITY, vec![]);
    while sel.len() < max_rules.max(1) {
        let mut best: Option<(usize, usize, f64)> = None;
        for i in (0..rules.len()).filter(|i| !sel.contains(i)) {
            let mut s2 = sel.clone();
            s2.push(i);
            let (m, v) = eval(&s2);
            if best.is_none_or(|b| v > b.2) {
                best = Some((i, m, v));
            }
        }
        match best {
            Some((i, m, v)) if v > cur + 1e-12 => {
                sel.push(i);
                cur = v;
                steps.push(Step {
                    key: rules[i].key.clone(),
                    m,
                    score: v,
                });
            }
            _ => break,
        }
    }
    let chosen: Vec<RuleRow> = sel.iter().map(|&i| rules[i].clone()).collect();
    let src: Vec<(String, Vec<usize>)> = sources
        .iter()
        .map(|(n, m)| (n.clone(), counts(m, &sel)))
        .collect();
    let mut sig = combine(chosen, &counts(cand, &sel), &counts(normal, &sel), &src);
    sig.steps = steps;
    sig.considered = rules.len();
    sig
}

pub fn share_at_least(h: &[usize], m: usize) -> f64 {
    if h.is_empty() {
        return f64::NAN;
    }
    h.iter().filter(|&&x| x >= m).count() as f64 / h.len() as f64
}

/// Picks `m` for the given rules from hit counts in the candidates, the
/// normal parts and each reference source (scored on the pooled reference).
pub fn combine(
    rules: Vec<RuleRow>,
    cand: &[usize],
    normal: &[usize],
    sources: &[(String, Vec<usize>)],
) -> Signature {
    let pooled: Vec<usize> = sources.iter().flat_map(|s| s.1.iter().copied()).collect();
    let fa_pool = |m: usize| (!pooled.is_empty()).then(|| share_at_least(&pooled, m));
    let mut best = (1usize, f64::NEG_INFINITY);
    for m in 1..=rules.len().max(1) {
        let s = share_at_least(cand, m) - share_at_least(normal, m).max(fa_pool(m).unwrap_or(0.0));
        if s > best.1 {
            best = (m, s);
        }
    }
    let m = best.0;
    Signature {
        fa_ref: fa_pool(m),
        sources: sources
            .iter()
            .map(|(name, h)| SourceRate {
                name: name.clone(),
                n: h.len(),
                rate: share_at_least(h, m),
            })
            .collect(),
        filter: (m == 1 && !rules.is_empty()).then(|| {
            rules
                .iter()
                .map(|r| r.filter.clone())
                .collect::<Vec<_>>()
                .join(" or ")
        }),
        m,
        capture: share_at_least(cand, m),
        fa_a: share_at_least(normal, m),
        score: best.1,
        rules,
        steps: vec![],
        considered: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn learns_the_cut_and_the_band() {
        let neg: Vec<f64> = (0..200).map(|i| (i % 20) as f64).collect();
        let pos: Vec<f64> = (0..20).map(|i| 25.0 + i as f64 * 0.1).collect();
        let (r, j) = learn(&pos, &neg).unwrap();
        assert_eq!(r.kind, Kind::Above);
        assert!(r.a > 19.0 && r.a < 25.0 && (j - 1.0).abs() < 1e-9);
        // candidates on both sides of the normal parts
        let pos2: Vec<f64> = (0..20)
            .map(|i| if i % 2 == 0 { -10.0 } else { 30.0 })
            .collect();
        let (r2, _) = learn(&pos2, &neg).unwrap();
        assert_eq!(r2.kind, Kind::Outside);
        assert!(r2.hit(-10.0) && r2.hit(30.0) && !r2.hit(10.0));
        assert_eq!(
            Rule {
                kind: Kind::Above,
                a: 20.5,
                b: 20.5
            }
            .filter("t[1290]"),
            "t[1290] > 20.5"
        );
        assert_eq!(g(20.723424), "20.723");
        assert_eq!(g(0.00123456), "0.0012346");
        assert_eq!(g(12345.6), "12346");
    }

    #[test]
    fn reference_with_the_pattern_drops_the_rule() {
        let pool = |rate: f64, shift: f64| LotRate {
            lot: "reference pool".into(),
            n: 1000,
            hits: (rate * 1000.0) as usize,
            rate,
            shift_sigma: shift,
            limits_changed: false,
        };
        let v = |l: LotRate| verdict(0.8, 0.02, 1000, Some(&l), &[]).0;
        assert_eq!(v(pool(0.5, 0.0)), "drop");
        assert_eq!(v(pool(0.1, 0.0)), "drop", "5x the normal false alarms");
        assert_eq!(v(pool(0.02, 0.0)), "keep");
        assert_eq!(v(pool(0.039, 0.0)), "caution", "1.95x, significant");
        assert_eq!(v(pool(0.0, -2.0)), "untested");
        assert_eq!(verdict(0.8, 0.02, 1000, None, &[]).0, "no data");
    }

    #[test]
    fn one_source_with_the_pattern_drops_the_rule() {
        let src = |lot: &str, n: usize, hits: usize| LotRate {
            lot: lot.into(),
            n,
            hits,
            rate: hits as f64 / n as f64,
            shift_sigma: 0.0,
            limits_changed: false,
        };
        // a small clean lot at 10% (5x the normal parts) inside a quiet pool
        let (small, big) = (src("lot C", 200, 20), src("lot B", 20000, 400));
        let pool = src("reference pool", 20200, 420);
        assert_eq!(
            verdict(0.8, 0.02, 1000, Some(&pool), std::slice::from_ref(&big)).0,
            "keep"
        );
        let (v, why, by) = verdict(0.8, 0.02, 1000, Some(&pool), &[big, small]);
        assert_eq!((v, by.as_deref()), ("drop", Some("lot C")));
        assert!(why.starts_with("lot C matches it in 10.0%"), "{why}");
    }

    #[test]
    fn greedy_signature_beats_or_equals_the_best_single_rule() {
        let rule = |key: &str| RuleRow {
            key: key.into(),
            num: 0,
            units: String::new(),
            rule: Rule {
                kind: Kind::Above,
                a: 0.0,
                b: 0.0,
            },
            text: String::new(),
            filter: format!("t[{key}] > 0"),
            capture: 0.0,
            fa_a: 0.0,
            fa_ref: None,
            pool: None,
            sources: vec![],
            j_a: 0.0,
            score: 0.0,
            verdict: "keep".into(),
            why: String::new(),
            drop_by: None,
        };
        // 10 candidates, 100 normal and 100 reference parts; rules a, b, c
        let mat = |n: usize, f: &dyn Fn(usize) -> [bool; 3]| {
            (0..n).map(|i| f(i).to_vec()).collect::<Vec<_>>()
        };
        // a: catches 9 of 10, 5% false alarms; b: catches the 10th, 30% false
        // alarms; c: catches the first 5, no false alarms
        let cand = mat(10, &|i| [i < 9, i == 9, i < 5]);
        let normal = mat(100, &|i| [i < 5, i < 30, false]);
        let refp = mat(100, &|i| [i >= 95, i >= 70, false]);
        let s = select(
            vec![rule("a"), rule("b"), rule("c")],
            &cand,
            &normal,
            &[("ref".into(), refp.clone())],
            5,
        );
        let keys: Vec<&str> = s.rules.iter().map(|r| r.key.as_str()).collect();
        assert_eq!(keys, ["a"], "b only adds false alarms, c adds nothing");
        assert!((s.score - (0.9 - 0.05)).abs() < 1e-12);
        assert_eq!(s.steps.len(), 1);
        assert_eq!(s.considered, 3);
        // two complementary clean rules are combined with m = 1
        let cand = mat(10, &|i| [i < 6, i >= 6, false]);
        let quiet = mat(100, &|_| [false, false, false]);
        let s = select(
            vec![rule("a"), rule("b"), rule("c")],
            &cand,
            &quiet,
            &[("ref".into(), quiet.clone())],
            5,
        );
        let keys: Vec<&str> = s.rules.iter().map(|r| r.key.as_str()).collect();
        assert_eq!((keys, s.m), (vec!["a", "b"], 1));
        assert!((s.score - 1.0).abs() < 1e-12);
        assert_eq!(s.filter.as_deref(), Some("t[a] > 0 or t[b] > 0"));
    }

    #[test]
    fn signature_scored_on_the_pooled_reference() {
        // one rule; the pool is two sources of 100 parts with 2 and 10 hits
        let src = |k: usize| (0..100).map(|i| (i < k) as usize).collect::<Vec<_>>();
        let s = combine(
            vec![],
            &[1, 1, 1, 0],
            &src(1),
            &[("clean".into(), src(2)), ("confirmed".into(), src(10))],
        );
        assert_eq!(s.m, 1);
        assert!((s.fa_ref.unwrap() - 0.06).abs() < 1e-12);
        assert_eq!(s.sources.len(), 2);
        assert!((s.sources[1].rate - 0.1).abs() < 1e-12);
        assert!((s.score - (0.75 - 0.06)).abs() < 1e-12);
    }

    #[test]
    fn fisher_one_sided() {
        assert!(fisher_greater(30, 1000, 5, 1000) < 1e-4);
        assert!(fisher_greater(5, 1000, 5, 1000) > 0.4);
        assert_eq!(fisher_greater(0, 1000, 5, 1000), 1.0);
    }
}
