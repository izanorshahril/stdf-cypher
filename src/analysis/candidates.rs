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

/// SI prefix scale of a leading character.
fn si_scale(c: char) -> Option<f64> {
    Some(match c {
        'f' => 1e-15,
        'p' => 1e-12,
        'n' => 1e-9,
        'u' | 'µ' | 'μ' => 1e-6,
        'm' => 1e-3,
        'k' | 'K' => 1e3,
        'M' => 1e6,
        'G' => 1e9,
        'T' => 1e12,
        _ => return None,
    })
}

/// Splits a unit such as `mA` into its prefix scale and base unit (`A`);
/// units without a recognised prefix (`A`, `ohm`, `%`, `code`) scale by 1.
pub fn unit_scale(units: &str) -> (f64, &str) {
    let u = units.trim();
    let mut ch = u.chars();
    if let Some(c) = ch.next() {
        let rest = ch.as_str();
        let base = matches!(
            rest,
            "A" | "V" | "Hz" | "s" | "W" | "F" | "H" | "Ω" | "ohm" | "Ohm"
        );
        if base {
            if let Some(s) = si_scale(c) {
                return (s, rest);
            }
        }
    }
    (1.0, u)
}

/// How a tester shows a test's results: the logged `units` scaled by the PTR
/// `RES_SCAL` (shown value = stored × 10^RES_SCAL, so A with RES_SCAL 3 is
/// shown in mA). Returns the display unit and that factor; falls back to the
/// logged unit (factor 1) without a scale or without a matching SI prefix.
pub fn display_unit(units: &str, res_scal: i8) -> (String, f64) {
    let u = units.trim();
    if res_scal == 0 || u.is_empty() {
        return (u.to_string(), 1.0);
    }
    let (s0, base) = unit_scale(u);
    if !matches!(
        base,
        "A" | "V" | "Hz" | "s" | "W" | "F" | "H" | "Ω" | "ohm" | "Ohm"
    ) {
        return (u.to_string(), 1.0);
    }
    let prefix = match s0.log10().round() as i32 - res_scal as i32 {
        -15 => "f",
        -12 => "p",
        -9 => "n",
        -6 => "u",
        -3 => "m",
        0 => "",
        3 => "k",
        6 => "M",
        9 => "G",
        12 => "T",
        _ => return (u.to_string(), 1.0),
    };
    (format!("{prefix}{base}"), 10f64.powi(res_scal as i32))
}

/// One bound of a value range in a test's stored `units`: a plain number is
/// in the display unit (`disp`: stored × disp = shown, see [`display_unit`]);
/// `10m`, `10mA`, `2.5uA` override it and are converted from the SI prefix
/// (and checked against the test's base unit, if given).
pub fn parse_si(s: &str, units: &str, disp: f64) -> anyhow::Result<f64> {
    let s = s.trim();
    let end = s
        .char_indices()
        .find(|&(i, c)| {
            !(c.is_ascii_digit()
                || c == '.'
                || ((c == '-' || c == '+')
                    && (i == 0 || matches!(s[..i].chars().last(), Some('e' | 'E'))))
                || ((c == 'e' || c == 'E')
                    && s[i + 1..]
                        .starts_with(|d: char| d.is_ascii_digit() || d == '-' || d == '+')))
        })
        .map(|p| p.0)
        .unwrap_or(s.len());
    let v: f64 = s[..end]
        .parse()
        .map_err(|_| anyhow::anyhow!("'{s}' is not a number"))?;
    let suffix = s[end..].trim();
    if suffix.is_empty() {
        return Ok(v / disp);
    }
    let (t_scale, t_base) = unit_scale(units);
    // the suffix: a prefix alone (`m`), prefix + unit (`mA`) or a unit (`A`)
    let mut ch = suffix.chars();
    let prefix = ch.next().and_then(si_scale);
    let (scale, base) = match prefix {
        Some(sc) if !suffix.eq_ignore_ascii_case(t_base) => (sc, ch.as_str()),
        _ => (1.0, suffix),
    };
    if !base.is_empty() && !t_base.is_empty() && !base.eq_ignore_ascii_case(t_base) {
        anyhow::bail!("'{s}' is in {base}, but the test is in {units}");
    }
    Ok(v * scale / t_scale)
}

/// A value range `lo..hi` (either end may be left open) in the test's stored units.
#[derive(Debug, Clone, Copy, Serialize, PartialEq)]
pub struct Range {
    pub lo: Option<f64>,
    pub hi: Option<f64>,
}

impl Range {
    /// `10..20` (in the display unit of `units` and `res_scal`), or with SI
    /// suffixes `10m..20m`, `10mA..20mA`, `10mA..`, `..20mA`; `to` and `–`
    /// also separate the ends.
    pub fn parse(s: &str, units: &str, res_scal: i8) -> anyhow::Result<Range> {
        let disp = display_unit(units, res_scal).1;
        let s = s.trim();
        let (a, b) = s
            .split_once("..")
            .or_else(|| s.split_once(" to "))
            .or_else(|| s.split_once('–'))
            .ok_or_else(|| anyhow::anyhow!("write the range as low..high, e.g. 10mA..20mA"))?;
        let side = |x: &str| -> anyhow::Result<Option<f64>> {
            let x = x.trim();
            if x.is_empty() {
                Ok(None)
            } else {
                parse_si(x, units, disp).map(Some)
            }
        };
        let (lo, hi) = (side(a)?, side(b)?);
        if lo.is_none() && hi.is_none() {
            anyhow::bail!("the range needs at least one end");
        }
        if let (Some(l), Some(h)) = (lo, hi) {
            if l > h {
                anyhow::bail!("the range's low end is above its high end");
            }
        }
        Ok(Range { lo, hi })
    }
    /// Inside the range, ends included; NaN is never inside.
    pub fn contains(&self, v: f64) -> bool {
        v.is_finite() && self.lo.is_none_or(|l| v >= l) && self.hi.is_none_or(|h| v <= h)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn si_ranges() {
        let near = |a: f64, b: f64| (a - b).abs() <= 1e-12 * b.abs().max(1.0);
        let r = Range::parse("10m..20m", "A", 0).unwrap();
        assert!(near(r.lo.unwrap(), 0.01) && near(r.hi.unwrap(), 0.02));
        let r = Range::parse("10mA..20mA", "A", 0).unwrap();
        assert!(near(r.lo.unwrap(), 0.01) && near(r.hi.unwrap(), 0.02));
        assert_eq!(
            Range::parse("0.01..0.02", "A", 0).unwrap(),
            Range {
                lo: Some(0.01),
                hi: Some(0.02)
            }
        );
        // a test logged in uA: mA values are converted, plain numbers are uA
        let r = Range::parse("10mA..20mA", "uA", 0).unwrap();
        assert!(near(r.lo.unwrap(), 10_000.0) && near(r.hi.unwrap(), 20_000.0));
        assert_eq!(Range::parse("5..", "uA", 0).unwrap().hi, None);
        let si = |x: &str, u: &str| parse_si(x, u, 1.0).unwrap();
        assert!(near(si("2.5k", "ohm"), 2500.0));
        assert!(near(si("1e-3", "A"), 1e-3));
        assert!(near(si("-5mV", "V"), -5e-3));
        assert!(near(si("3A", "A"), 3.0));
        assert!(Range::parse("10mV..20mV", "A", 0).is_err(), "wrong unit");
        assert!(Range::parse("20m..10m", "A", 0).is_err());
        assert!(Range::parse("10m", "A", 0).is_err());
        let r = Range::parse("10m..20m", "A", 0).unwrap();
        assert!(
            r.contains(0.01) && r.contains(0.015) && !r.contains(0.021) && !r.contains(f64::NAN)
        );
    }

    #[test]
    fn plain_numbers_follow_the_res_scal_display_unit() {
        let near = |a: f64, b: f64| (a - b).abs() <= 1e-12 * b.abs().max(1.0);
        assert_eq!(display_unit("A", 3), ("mA".to_string(), 1e3));
        assert_eq!(display_unit("A", 6).0, "uA");
        assert_eq!(display_unit("Hz", -6).0, "MHz");
        assert_eq!(display_unit("mA", 3).0, "uA");
        assert_eq!(display_unit("uA", 0), ("uA".to_string(), 1.0));
        // no matching prefix, or not an SI unit: the logged unit, unscaled
        assert_eq!(display_unit("A", 1), ("A".to_string(), 1.0));
        assert_eq!(display_unit("code", 3), ("code".to_string(), 1.0));
        // stored in A, shown in mA: 10..20 means 10 mA..20 mA
        let r = Range::parse("10..20", "A", 3).unwrap();
        assert!(near(r.lo.unwrap(), 0.01) && near(r.hi.unwrap(), 0.02));
        // an explicit suffix still overrides the hint
        let r = Range::parse("10uA..20uA", "A", 3).unwrap();
        assert!(near(r.lo.unwrap(), 1e-5) && near(r.hi.unwrap(), 2e-5));
    }

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
