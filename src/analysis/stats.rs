//! Statistical primitives.

use serde::Serialize;
use statrs::distribution::{ChiSquared, ContinuousCDF, FisherSnedecor, Normal, StudentsT};

#[derive(Debug, Clone, Serialize, Default)]
pub struct Describe {
    pub n: usize,
    pub mean: f64,
    pub std: f64,
    pub min: f64,
    pub q1: f64,
    pub median: f64,
    pub q3: f64,
    pub max: f64,
    /// robust sigma = 1.4826 * MAD
    pub robust_sigma: f64,
    pub skew: f64,
    pub kurtosis: f64,
    pub cp: Option<f64>,
    pub cpk: Option<f64>,
    /// count outside [lo, hi]
    pub fails: usize,
}

pub fn mean(v: &[f64]) -> f64 {
    if v.is_empty() {
        return f64::NAN;
    }
    v.iter().sum::<f64>() / v.len() as f64
}

pub fn variance(v: &[f64]) -> f64 {
    if v.len() < 2 {
        return f64::NAN;
    }
    let m = mean(v);
    v.iter().map(|x| (x - m).powi(2)).sum::<f64>() / (v.len() - 1) as f64
}

pub fn std(v: &[f64]) -> f64 {
    variance(v).sqrt()
}

/// Linear-interpolated quantile of *sorted* data (type 7, like numpy default).
pub fn quantile_sorted(s: &[f64], q: f64) -> f64 {
    if s.is_empty() {
        return f64::NAN;
    }
    let h = (s.len() - 1) as f64 * q.clamp(0.0, 1.0);
    let lo = h.floor() as usize;
    let hi = h.ceil() as usize;
    s[lo] + (h - lo as f64) * (s[hi] - s[lo])
}

pub fn sorted(v: &[f64]) -> Vec<f64> {
    let mut s: Vec<f64> = v.iter().copied().filter(|x| x.is_finite()).collect();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap());
    s
}

pub fn median(v: &[f64]) -> f64 {
    quantile_sorted(&sorted(v), 0.5)
}

/// Median absolute deviation scaled to sigma (×1.4826).
pub fn robust_sigma(v: &[f64]) -> f64 {
    let m = median(v);
    let dev: Vec<f64> = v.iter().map(|x| (x - m).abs()).collect();
    1.4826 * median(&dev)
}

pub fn describe(v: &[f64], lo: Option<f64>, hi: Option<f64>) -> Describe {
    let s = sorted(v);
    let n = s.len();
    if n == 0 {
        return Describe {
            mean: f64::NAN,
            std: f64::NAN,
            min: f64::NAN,
            max: f64::NAN,
            median: f64::NAN,
            q1: f64::NAN,
            q3: f64::NAN,
            robust_sigma: f64::NAN,
            skew: f64::NAN,
            kurtosis: f64::NAN,
            ..Default::default()
        };
    }
    let m = mean(&s);
    let sd = std(&s);
    let (mut m3, mut m4) = (0.0, 0.0);
    for x in &s {
        let d = x - m;
        m3 += d.powi(3);
        m4 += d.powi(4);
    }
    let nn = n as f64;
    let pop_sd = (s.iter().map(|x| (x - m).powi(2)).sum::<f64>() / nn).sqrt();
    let skew = if pop_sd > 0.0 {
        m3 / nn / pop_sd.powi(3)
    } else {
        0.0
    };
    let kurt = if pop_sd > 0.0 {
        m4 / nn / pop_sd.powi(4) - 3.0
    } else {
        0.0
    };
    let med = quantile_sorted(&s, 0.5);
    let dev = sorted(&s.iter().map(|x| (x - med).abs()).collect::<Vec<_>>());
    let rs = 1.4826 * quantile_sorted(&dev, 0.5);
    let (cp, cpk) = cpk(m, sd, lo, hi);
    let fails = s
        .iter()
        .filter(|&&x| lo.map(|l| x < l).unwrap_or(false) || hi.map(|h| x > h).unwrap_or(false))
        .count();
    Describe {
        n,
        mean: m,
        std: if n > 1 { sd } else { 0.0 },
        min: s[0],
        q1: quantile_sorted(&s, 0.25),
        median: med,
        q3: quantile_sorted(&s, 0.75),
        max: s[n - 1],
        robust_sigma: rs,
        skew,
        kurtosis: kurt,
        cp,
        cpk,
        fails,
    }
}

pub fn cpk(mean: f64, sd: f64, lo: Option<f64>, hi: Option<f64>) -> (Option<f64>, Option<f64>) {
    if !(sd > 0.0) {
        return (None, None);
    }
    let cpl = lo.map(|l| (mean - l) / (3.0 * sd));
    let cpu = hi.map(|h| (h - mean) / (3.0 * sd));
    let cp = match (lo, hi) {
        (Some(l), Some(h)) => Some((h - l) / (6.0 * sd)),
        _ => None,
    };
    let cpk = match (cpl, cpu) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        _ => None,
    };
    (cp, cpk)
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct TestResult {
    pub stat: f64,
    pub p: f64,
}

/// Welch's two-sample t-test (two-sided).
pub fn welch_t(a: &[f64], b: &[f64]) -> TestResult {
    let (na, nb) = (a.len() as f64, b.len() as f64);
    if na < 2.0 || nb < 2.0 {
        return TestResult {
            stat: f64::NAN,
            p: f64::NAN,
        };
    }
    let (va, vb) = (variance(a), variance(b));
    let se2 = va / na + vb / nb;
    if !(se2 > 0.0) {
        let same = (mean(a) - mean(b)).abs() < 1e-300;
        return TestResult {
            stat: if same { 0.0 } else { f64::INFINITY },
            p: if same { 1.0 } else { 0.0 },
        };
    }
    let t = (mean(a) - mean(b)) / se2.sqrt();
    let df = se2.powi(2) / ((va / na).powi(2) / (na - 1.0) + (vb / nb).powi(2) / (nb - 1.0));
    let p = StudentsT::new(0.0, 1.0, df.max(1.0))
        .map(|d| 2.0 * d.sf(t.abs()))
        .unwrap_or(f64::NAN);
    TestResult { stat: t, p }
}

/// Two-sample Kolmogorov–Smirnov test (asymptotic p-value).
pub fn ks_2samp(a: &[f64], b: &[f64]) -> TestResult {
    let (sa, sb) = (sorted(a), sorted(b));
    let (na, nb) = (sa.len(), sb.len());
    if na == 0 || nb == 0 {
        return TestResult {
            stat: f64::NAN,
            p: f64::NAN,
        };
    }
    let (mut i, mut j, mut d) = (0usize, 0usize, 0f64);
    while i < na && j < nb {
        let x = sa[i].min(sb[j]);
        while i < na && sa[i] <= x {
            i += 1;
        }
        while j < nb && sb[j] <= x {
            j += 1;
        }
        d = d.max((i as f64 / na as f64 - j as f64 / nb as f64).abs());
    }
    let ne = (na * nb) as f64 / (na + nb) as f64;
    let lambda = (ne.sqrt() + 0.12 + 0.11 / ne.sqrt()) * d;
    TestResult {
        stat: d,
        p: kolmogorov_q(lambda),
    }
}

fn kolmogorov_q(l: f64) -> f64 {
    if l < 1e-3 {
        return 1.0;
    }
    let mut s = 0.0;
    for j in 1..=100 {
        let jf = j as f64;
        let term = 2.0 * if j % 2 == 1 { 1.0 } else { -1.0 } * (-2.0 * jf * jf * l * l).exp();
        s += term;
        if term.abs() < 1e-12 {
            break;
        }
    }
    s.clamp(0.0, 1.0)
}

/// Mann–Whitney U test (normal approximation with tie correction).
pub fn mann_whitney(a: &[f64], b: &[f64]) -> TestResult {
    let (na, nb) = (a.len(), b.len());
    if na == 0 || nb == 0 {
        return TestResult {
            stat: f64::NAN,
            p: f64::NAN,
        };
    }
    let mut all: Vec<(f64, bool)> = a
        .iter()
        .map(|&x| (x, true))
        .chain(b.iter().map(|&x| (x, false)))
        .collect();
    all.sort_by(|x, y| x.0.partial_cmp(&y.0).unwrap());
    let ranks = rank_sorted(&all.iter().map(|x| x.0).collect::<Vec<_>>());
    let ra: f64 = all
        .iter()
        .zip(&ranks)
        .filter(|(x, _)| x.1)
        .map(|(_, r)| r)
        .sum();
    let u = ra - (na * (na + 1)) as f64 / 2.0;
    let n = (na + nb) as f64;
    let ties = tie_term(&all.iter().map(|x| x.0).collect::<Vec<_>>());
    let mu = (na * nb) as f64 / 2.0;
    let sigma = ((na * nb) as f64 / 12.0 * ((n + 1.0) - ties / (n * (n - 1.0)))).sqrt();
    if !(sigma > 0.0) {
        return TestResult { stat: u, p: 1.0 };
    }
    let z = (u - mu) / sigma;
    TestResult {
        stat: u,
        p: 2.0 * norm_sf(z.abs()),
    }
}

fn tie_term(sorted: &[f64]) -> f64 {
    let mut t = 0.0;
    let mut i = 0;
    while i < sorted.len() {
        let mut j = i;
        while j + 1 < sorted.len() && sorted[j + 1] == sorted[i] {
            j += 1;
        }
        let c = (j - i + 1) as f64;
        t += c * c * c - c;
        i = j + 1;
    }
    t
}

/// Average ranks (1-based) of already-sorted data.
fn rank_sorted(s: &[f64]) -> Vec<f64> {
    let mut r = vec![0.0; s.len()];
    let mut i = 0;
    while i < s.len() {
        let mut j = i;
        while j + 1 < s.len() && s[j + 1] == s[i] {
            j += 1;
        }
        let avg = (i + j) as f64 / 2.0 + 1.0;
        for k in i..=j {
            r[k] = avg;
        }
        i = j + 1;
    }
    r
}

/// Average ranks of arbitrary data.
pub fn ranks(v: &[f64]) -> Vec<f64> {
    let mut idx: Vec<usize> = (0..v.len()).collect();
    idx.sort_by(|&a, &b| v[a].partial_cmp(&v[b]).unwrap_or(std::cmp::Ordering::Equal));
    let s: Vec<f64> = idx.iter().map(|&i| v[i]).collect();
    let rs = rank_sorted(&s);
    let mut out = vec![0.0; v.len()];
    for (k, &i) in idx.iter().enumerate() {
        out[i] = rs[k];
    }
    out
}

pub fn pearson(x: &[f64], y: &[f64]) -> f64 {
    let n = x.len().min(y.len());
    if n < 3 {
        return f64::NAN;
    }
    let (mx, my) = (mean(&x[..n]), mean(&y[..n]));
    let (mut sxy, mut sxx, mut syy) = (0.0, 0.0, 0.0);
    for i in 0..n {
        let (dx, dy) = (x[i] - mx, y[i] - my);
        sxy += dx * dy;
        sxx += dx * dx;
        syy += dy * dy;
    }
    if sxx <= 0.0 || syy <= 0.0 {
        return f64::NAN;
    }
    sxy / (sxx * syy).sqrt()
}

pub fn spearman(x: &[f64], y: &[f64]) -> f64 {
    pearson(&ranks(x), &ranks(y))
}

/// p-value of a correlation coefficient (t-test with n-2 df).
pub fn corr_p(r: f64, n: usize) -> f64 {
    if n < 3 || !r.is_finite() {
        return f64::NAN;
    }
    if r.abs() >= 1.0 {
        return 0.0;
    }
    let t = r * ((n as f64 - 2.0) / (1.0 - r * r)).sqrt();
    StudentsT::new(0.0, 1.0, n as f64 - 2.0)
        .map(|d| 2.0 * d.sf(t.abs()))
        .unwrap_or(f64::NAN)
}

/// Ordinary least squares y = a + b x. Returns (intercept, slope, r²).
pub fn linreg(x: &[f64], y: &[f64]) -> (f64, f64, f64) {
    let (mx, my) = (mean(x), mean(y));
    let (mut sxy, mut sxx) = (0.0, 0.0);
    for i in 0..x.len().min(y.len()) {
        sxy += (x[i] - mx) * (y[i] - my);
        sxx += (x[i] - mx).powi(2);
    }
    let b = if sxx > 0.0 { sxy / sxx } else { f64::NAN };
    let a = my - b * mx;
    let r = pearson(x, y);
    (a, b, r * r)
}

pub fn cohen_d(a: &[f64], b: &[f64]) -> f64 {
    let (na, nb) = (a.len() as f64, b.len() as f64);
    if na < 2.0 || nb < 2.0 {
        return f64::NAN;
    }
    let sp = (((na - 1.0) * variance(a) + (nb - 1.0) * variance(b)) / (na + nb - 2.0)).sqrt();
    if sp > 0.0 {
        (mean(b) - mean(a)) / sp
    } else {
        0.0
    }
}

pub fn f_sf(f: f64, d1: f64, d2: f64) -> f64 {
    if !f.is_finite() {
        return if f.is_nan() { f64::NAN } else { 0.0 };
    }
    FisherSnedecor::new(d1, d2)
        .map(|d| d.sf(f))
        .unwrap_or(f64::NAN)
}

pub fn chi2_sf(x: f64, k: f64) -> f64 {
    ChiSquared::new(k).map(|d| d.sf(x)).unwrap_or(f64::NAN)
}

pub fn norm_sf(z: f64) -> f64 {
    Normal::new(0.0, 1.0).unwrap().sf(z)
}

/// Benjamini–Hochberg adjusted p-values (q-values).
pub fn bh_adjust(p: &[f64]) -> Vec<f64> {
    let m = p.iter().filter(|x| x.is_finite()).count() as f64;
    let mut idx: Vec<usize> = (0..p.len()).filter(|&i| p[i].is_finite()).collect();
    idx.sort_by(|&a, &b| p[b].partial_cmp(&p[a]).unwrap());
    let mut q = vec![f64::NAN; p.len()];
    let mut prev = 1.0f64;
    for (k, &i) in idx.iter().enumerate() {
        let rank = m - k as f64;
        let v = (p[i] * m / rank).min(prev).min(1.0);
        q[i] = v;
        prev = v;
    }
    q
}

/// Freedman–Diaconis histogram bin edges.
pub fn histogram(
    v: &[f64],
    bins: Option<usize>,
    range: Option<(f64, f64)>,
) -> (Vec<f64>, Vec<usize>) {
    let s = sorted(v);
    if s.is_empty() {
        return (vec![], vec![]);
    }
    let (lo, hi) = range.unwrap_or((s[0], s[s.len() - 1]));
    let nb = bins.unwrap_or_else(|| {
        let iqr = quantile_sorted(&s, 0.75) - quantile_sorted(&s, 0.25);
        let w = 2.0 * iqr / (s.len() as f64).cbrt();
        if w > 0.0 {
            (((hi - lo) / w).ceil() as usize).clamp(10, 200)
        } else {
            20
        }
    });
    let width = if hi > lo { (hi - lo) / nb as f64 } else { 1.0 };
    let mut counts = vec![0usize; nb];
    for &x in &s {
        if x < lo || x > hi {
            continue;
        }
        let k = (((x - lo) / width) as usize).min(nb - 1);
        counts[k] += 1;
    }
    let edges = (0..=nb).map(|i| lo + i as f64 * width).collect();
    (edges, counts)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn basic() {
        let v = [2.0, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0];
        let d = describe(&v, Some(0.0), Some(10.0));
        assert!((d.mean - 5.0).abs() < 1e-12);
        assert!((d.std - 2.138089935).abs() < 1e-6);
        assert!((d.median - 4.5).abs() < 1e-12);
    }
    #[test]
    fn welch_known() {
        // reference: scipy.stats.ttest_ind(a, b, equal_var=False)
        let a = [19.8, 20.4, 19.6, 17.8, 18.5, 18.9, 18.3, 18.9, 19.5, 22.0];
        let b = [28.2, 26.6, 20.1, 23.3, 25.2, 22.1, 17.7, 27.6, 20.6, 13.7];
        let r = welch_t(&a, &b);
        assert!((r.stat - (-2.0740146)).abs() < 1e-6, "{}", r.stat);
        assert!((r.p - 0.0642800).abs() < 1e-5, "{}", r.p);
        // scipy.stats.mannwhitneyu(a, b, method='asymptotic', use_continuity=False)
        let m = mann_whitney(&a, &b);
        assert!(
            (m.stat - 23.0).abs() < 1e-9 && (m.p - 0.0411738).abs() < 1e-5,
            "{:?}",
            m
        );
        let k = ks_2samp(&a, &b);
        assert!((k.stat - 0.6).abs() < 1e-9);
        assert!((pearson(&a, &b) - (-0.3575014)).abs() < 1e-6);
        assert!((spearman(&a, &b) - (-0.0182372)).abs() < 1e-6);
    }
    #[test]
    fn corr() {
        let x = [1.0, 2.0, 3.0, 4.0, 5.0];
        let y = [2.0, 4.1, 5.9, 8.2, 9.9];
        assert!(pearson(&x, &y) > 0.99);
        assert!((spearman(&x, &y) - 1.0).abs() < 1e-12);
    }
    #[test]
    fn ks() {
        let a: Vec<f64> = (0..100).map(|i| i as f64).collect();
        let b: Vec<f64> = (0..100).map(|i| i as f64 + 50.0).collect();
        let r = ks_2samp(&a, &b);
        assert!((r.stat - 0.5).abs() < 1e-9);
        assert!(r.p < 1e-8);
    }
}
