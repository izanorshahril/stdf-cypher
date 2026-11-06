//! Qualification mean-drift reports, written as Excel workbooks. Both report one row per test and
//! site: a looped unit sits on each site, so each site is its own trial.
//!
//! * Endurance check (one file): the same unit runs on each site for 1,000 loops or 1.5 hours. The
//!   mean of the first 25 loops is compared with the mean of the last 25:
//!   `drift = ABS(mean1 - mean2) / (USL - LSL)`. The trial is valid when the yield is at least 95%
//!   and the compared loops all pass. Loops past the trial window (a run left going too long, where
//!   the unit degrades) can be cut by loop count and by time, and the noisy loops at the start can be
//!   skipped: the first and last 25 good loops of the window are compared, and a site whose yield over
//!   the compared span stays below the threshold is dropped.
//! * 25x loop compare (two files): the same units loop 25 times on the old and on the new test program.
//!   Each site's 25 good loops are taken from each file, and only sites with 25 good loops in both are
//!   compared. Tests are matched by key, else by test number (and pin), so renamed tests still pair.

use crate::analysis::stats;
use crate::data::{Dataset, TestKind};
use crate::xlsx::{self, column_letters, style, write_styled, Sheet};
use anyhow::{bail, Result};
use serde::Serialize;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{self, Write};
use std::path::Path;

/// How the endurance check picks the compared loops of a site.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Pick {
    /// the first and last good loops of the window; failing loops between are skipped
    Clean,
    /// the first and last loops of the window as they are: all of them must pass
    Raw,
}

impl Pick {
    pub fn parse(s: &str) -> Result<Pick> {
        match s.trim() {
            "" | "clean" => Ok(Pick::Clean),
            "raw" => Ok(Pick::Raw),
            o => bail!("pick '{o}': use clean or raw"),
        }
    }
}

/// Which loops the endurance check may leave out, so the sites reach the yield threshold. A loop is
/// the k-th loop of every site (one touchdown): a loop left out goes for every site.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Trim {
    /// no loop is left out
    #[default]
    Off,
    /// leading and trailing loops in which any site fails
    Ends,
    /// as Ends, then failing loops between, until every site reaches the yield
    Any,
}

impl Trim {
    pub fn parse(s: &str) -> Result<Trim> {
        match s.trim() {
            "" | "off" => Ok(Trim::Off),
            "ends" => Ok(Trim::Ends),
            "any" => Ok(Trim::Any),
            o => bail!("trim '{o}': use off, ends or any"),
        }
    }
}

/// Which good loops of each site the loop compare takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Take {
    First,
    Last,
}

impl Take {
    pub fn parse(s: &str) -> Result<Take> {
        match s.trim() {
            "" | "first" => Ok(Take::First),
            "last" => Ok(Take::Last),
            o => bail!("take '{o}': use first or last"),
        }
    }
    fn name(self) -> &'static str {
        match self {
            Take::First => "first",
            Take::Last => "last",
        }
    }
}

/// Settings of the endurance check.
#[derive(Debug, Clone, Serialize)]
pub struct EnduranceOpts {
    /// loops compared at each end
    pub runs: usize,
    /// drift above this, in % of the limit range, is commented
    pub drift_pct: f64,
    /// the lowest yield, in %, of a valid trial
    pub yield_pct: f64,
    /// Cpk below this is commented
    pub cpk_min: f64,
    pub pick: Pick,
    /// loops skipped at the start of each site (warm-up)
    pub skip: usize,
    /// the trial window ends after this many loops of a site (0 = no cut)
    pub max_loops: usize,
    /// the trial window ends after this many minutes from the start of the file (0 = no cut); the
    /// time of a part is estimated from its place between the file's start and finish times
    pub max_minutes: f64,
    /// loops the check may leave out to reach the yield
    pub trim: Trim,
    /// the most loops the trim may leave out, in % of the window (0 = no limit); a site that needs
    /// more is dropped instead
    pub max_trim_pct: f64,
}

impl Default for EnduranceOpts {
    fn default() -> Self {
        EnduranceOpts {
            runs: 25,
            drift_pct: 5.0,
            yield_pct: 95.0,
            cpk_min: 1.33,
            pick: Pick::Clean,
            skip: 0,
            max_loops: 0,
            max_minutes: 0.0,
            trim: Trim::Off,
            max_trim_pct: 20.0,
        }
    }
}

/// Settings of the loop compare.
#[derive(Debug, Clone, Serialize)]
pub struct LoopOpts {
    /// good loops taken from each site of each file
    pub runs: usize,
    pub drift_pct: f64,
    pub cpk_min: f64,
    pub take: Take,
    /// loops skipped at the start of each site (warm-up)
    pub skip: usize,
}

impl Default for LoopOpts {
    fn default() -> Self {
        LoopOpts {
            runs: 25,
            drift_pct: 5.0,
            cpk_min: 1.33,
            take: Take::First,
            skip: 0,
        }
    }
}

/// Statistics of one test over the compared loops of one site.
#[derive(Debug, Clone, Serialize)]
pub struct Stat {
    pub n: usize,
    pub mean: f64,
    pub sd: f64,
    pub min: f64,
    pub max: f64,
    pub cp: Option<f64>,
    pub cpk: Option<f64>,
    /// values outside the limits
    pub fails: usize,
    /// the limits of the compared loops: the test's own, or for a test whose limits change part
    /// by part, those of the first compared loop
    pub lo: Option<f64>,
    pub hi: Option<f64>,
    /// the limits change between the compared loops: Cp, Cpk, fails and drift use the first loop's
    pub limits_vary: bool,
}

impl Stat {
    fn range(&self) -> Option<f64> {
        match (self.lo, self.hi) {
            (Some(l), Some(h)) if h > l => Some(h - l),
            _ => None,
        }
    }
}

/// The limits of test `t` on `rows`, and whether they change between the rows.
fn limits_of(ds: &Dataset, t: usize, rows: &[usize]) -> (Option<f64>, Option<f64>, bool) {
    let info = &ds.tests[t];
    let Some(pl) = ds.part_limits.get(t).and_then(Option::as_ref) else {
        return (info.lo, info.hi, false);
    };
    let at = |v: &[f32], r: usize| v.get(r).copied().filter(|x| x.is_finite());
    let key = |r: usize| (at(&pl.lo, r).map(f32::to_bits), at(&pl.hi, r).map(f32::to_bits));
    let first = rows.first().copied();
    let varies = rows.iter().any(|&r| Some(key(r)) != first.map(key));
    let lo = first.and_then(|r| at(&pl.lo, r)).map(f64::from);
    let hi = first.and_then(|r| at(&pl.hi, r)).map(f64::from);
    (lo, hi, varies)
}

/// Statistics of test `t` over `rows`; with no values, n is 0 and the statistics are NaN.
fn stat(ds: &Dataset, t: usize, rows: &[usize]) -> Stat {
    let v = ds.values_of(t, Some(rows));
    let (lo, hi, limits_vary) = limits_of(ds, t, rows);
    if v.is_empty() {
        return Stat {
            n: 0,
            mean: f64::NAN,
            sd: f64::NAN,
            min: f64::NAN,
            max: f64::NAN,
            cp: None,
            cpk: None,
            fails: 0,
            lo,
            hi,
            limits_vary,
        };
    }
    let mean = stats::mean(&v);
    let sd = if v.len() > 1 { stats::std(&v) } else { 0.0 };
    let (cp, cpk) = stats::cpk(mean, sd, lo, hi);
    let inside = |x: f64| lo.is_none_or(|l| x >= l) && hi.is_none_or(|h| x <= h);
    Stat {
        n: v.len(),
        mean,
        sd,
        min: v.iter().copied().fold(f64::INFINITY, f64::min),
        max: v.iter().copied().fold(f64::NEG_INFINITY, f64::max),
        cp,
        cpk,
        fails: v.iter().filter(|&&x| !inside(x)).count(),
        lo,
        hi,
        limits_vary,
    }
}

impl Stat {
    /// The same statistics in another unit: `f` turns a value into it.
    fn scaled(mut self, f: f64) -> Stat {
        self.mean *= f;
        self.sd *= f;
        self.min *= f;
        self.max *= f;
        self.lo = self.lo.map(|v| v * f);
        self.hi = self.hi.map(|v| v * f);
        self
    }
}

/// The factor that turns a value in unit `from` into unit `to`, when they differ only by an SI
/// prefix ("mV" to "V" is 0.001); None when they are not the same base unit.
pub fn unit_factor(from: &str, to: &str) -> Option<f64> {
    fn split(u: &str) -> (f64, &str) {
        let u = u.trim();
        let mut it = u.chars();
        let Some(c) = it.next() else { return (1.0, u) };
        let rest = it.as_str();
        let p = match c {
            'f' => 1e-15,
            'p' => 1e-12,
            'n' => 1e-9,
            'u' | 'µ' | 'μ' => 1e-6,
            'm' => 1e-3,
            'k' | 'K' => 1e3,
            'M' => 1e6,
            'G' => 1e9,
            _ => return (1.0, u),
        };
        // a lone prefix letter is a unit of its own ("m" metre), and "min" is minutes
        if rest.is_empty() || u == "min" {
            (1.0, u)
        } else {
            (p, rest)
        }
    }
    if from.trim() == to.trim() {
        return Some(1.0);
    }
    let (a, ba) = split(from);
    let (b, bb) = split(to);
    (ba == bb).then(|| a / b)
}

/// Notes on how many values a comparison has, when it is fewer than the loops compared.
fn count_note(runs: usize, sides: [(&str, usize); 2]) -> Option<String> {
    sides.iter().any(|(_, n)| *n < runs).then(|| {
        format!(
            "Fewer than {runs} values: {} {}, {} {}",
            sides[0].0, sides[0].1, sides[1].0, sides[1].1
        )
    })
}

/// The largest runs count accepted: far above any loop trial, and safe to double.
const MAX_RUNS: usize = 1_000_000;

fn check_runs(n: usize) -> Result<()> {
    if n == 0 {
        bail!("compare at least one loop");
    }
    if n > MAX_RUNS {
        bail!("compare at most {MAX_RUNS} loops");
    }
    Ok(())
}

/// Tests worth a drift: parametric and multi-pin tests with at least one value.
fn numeric(ds: &Dataset) -> Vec<usize> {
    ds.numeric_tests()
        .into_iter()
        .filter(|&t| ds.tests[t].kind != TestKind::Functional)
        .collect()
}

/// A site's parts in file order, among `rows`. The key is (head, site).
fn by_site(ds: &Dataset, rows: &[usize]) -> BTreeMap<(u8, u8), Vec<usize>> {
    let mut m: BTreeMap<(u8, u8), Vec<usize>> = BTreeMap::new();
    for &r in rows {
        let p = &ds.parts[r];
        m.entry((p.head, p.site)).or_default().push(r);
    }
    m
}

fn is_good(ds: &Dataset, r: usize) -> bool {
    let p = &ds.parts[r];
    !p.partial && p.passed == Some(true)
}

/// "3-27, 29": 1-based loop numbers as ranges.
fn ranges(loops: &[usize]) -> String {
    let mut out: Vec<String> = vec![];
    let mut i = 0;
    while i < loops.len() {
        let mut j = i;
        while j + 1 < loops.len() && loops[j + 1] == loops[j] + 1 {
            j += 1;
        }
        out.push(if i == j {
            format!("{}", loops[i])
        } else {
            format!("{}-{}", loops[i], loops[j])
        });
        i = j + 1;
    }
    out.join(", ")
}

/// The label of a site: its number, or head/site when the file has several heads.
fn site_label(head: u8, site: u8, heads: bool) -> String {
    if heads {
        format!("{head}/{site}")
    } else {
        site.to_string()
    }
}

fn pct(n: usize, of: usize) -> Option<f64> {
    (of > 0).then(|| n as f64 * 100.0 / of as f64)
}

// ---------------------------------------------------------------- endurance

/// One site of the endurance check.
#[derive(Debug, Clone, Serialize)]
pub struct EnduranceSite {
    pub head: u8,
    pub site: u8,
    pub label: String,
    /// loops (parts) of the site
    pub loops: usize,
    /// loops inside the trial window
    pub window: usize,
    /// what ended the window early: "", "loops" or "time"
    pub cut_by: String,
    /// good loops in the window
    pub good: usize,
    pub window_yield: Option<f64>,
    /// loops of the window the auto trim left out, and the loops (and good ones) it kept
    pub trimmed: usize,
    pub kept: usize,
    pub kept_good: usize,
    pub kept_yield: Option<f64>,
    /// loops from the first compared loop to the last, and the good ones among them
    pub span: usize,
    pub span_good: usize,
    pub span_yield: Option<f64>,
    /// failing loops skipped before the first compared loop, and after the last
    pub skipped_lead: usize,
    pub skipped_tail: usize,
    /// 1-based loop numbers of the compared loops, as ranges
    pub first_loops: String,
    pub last_loops: String,
    pub first_all_pass: bool,
    pub last_all_pass: bool,
    /// estimated minutes from the file start to the site's last loop, when the file has times
    pub minutes: Option<f64>,
    /// the trial reached the loops or minutes it should run (None when neither is set)
    pub complete: Option<bool>,
    /// compared in the report
    pub ok: bool,
    pub status: String,
    #[serde(skip)]
    pub first: Vec<usize>,
    #[serde(skip)]
    pub last: Vec<usize>,
    /// rows from the first compared loop to the last of a compared site, without trimmed loops
    #[serde(skip)]
    pub kept_rows: Vec<usize>,
}

/// One test of one site.
#[derive(Debug, Clone, Serialize)]
pub struct DriftRow {
    pub test: usize,
    pub site: usize,
    pub first: Stat,
    pub last: Stat,
    pub drift: Option<f64>,
    pub over: bool,
    pub low_cpk: bool,
    pub comment: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Endurance {
    pub opts: EnduranceOpts,
    pub sites: Vec<EnduranceSite>,
    pub rows: Vec<DriftRow>,
    /// rows with drift above the threshold
    pub over: usize,
    /// rows with a Cpk below the minimum
    pub low_cpk: usize,
    /// tests with values on the compared sites
    pub tests: usize,
    /// rows with fewer values than the loops compared
    pub short_rows: usize,
    pub heads: bool,
    pub trim: TrimInfo,
}

impl Endurance {
    /// The rows of the trimmed trial: every compared site from its first compared loop to its
    /// last, without the loops the trim left out, in file order.
    pub fn kept_rows(&self) -> Vec<usize> {
        let mut v: Vec<usize> = self.sites.iter().flat_map(|s| s.kept_rows.iter().copied()).collect();
        v.sort_unstable();
        v
    }
}

/// Estimated minutes from the start of its file to the end of every part of `ds`, when the file
/// has a start and finish time.
fn minutes(ds: &Dataset) -> Vec<Option<f64>> {
    let mut n_in = vec![0usize; ds.sources.len().max(1)];
    for p in &ds.parts {
        if let Some(c) = n_in.get_mut(p.file as usize) {
            *c += 1;
        }
    }
    let mut seen = vec![0usize; n_in.len()];
    ds.parts
        .iter()
        .map(|p| {
            let f = p.file as usize;
            let s = ds.sources.get(f)?;
            let k = seen.get_mut(f)?;
            *k += 1;
            (s.finish_t > s.start_t && n_in[f] > 0)
                .then(|| f64::from(s.finish_t - s.start_t) / 60.0 * *k as f64 / n_in[f] as f64)
        })
        .collect()
}

/// One site's loops and its trial window, before the trim.
struct Win<'a> {
    rs: &'a [usize],
    /// the window is loops start..end (0-based, per site)
    start: usize,
    end: usize,
    complete: Option<bool>,
}

/// What the auto trim left out. Loop k is the k-th loop of every site (one touchdown).
#[derive(Debug, Clone, Default, Serialize)]
pub struct TrimInfo {
    pub mode: Trim,
    /// the most loops it may leave out, % of the window (0 = no limit)
    pub max_pct: f64,
    /// the sites the trim worked for; the others did not decide which loops go
    pub sites: Vec<String>,
    /// loops left out at the start, at the end and between
    pub start: usize,
    pub end: usize,
    pub between: usize,
    /// 1-based numbers of the loops left out, as ranges
    pub loops: String,
    #[serde(skip)]
    pub removed: Vec<bool>,
    /// per site (in window order): the trim kept it; a site it left out is dropped. Empty when the
    /// trim is off.
    #[serde(skip)]
    pub eligible: Vec<bool>,
}

/// Loops of the windows left out by the auto trim. Leading and trailing loops in which any of the
/// trimmed sites fails go first; with `Trim::Any`, loops between follow, the loop failing on the
/// most sites still below the yield first, until every trimmed site reaches it. A site that still
/// misses the yield or the good loops it needs is left out of the trim (it will be dropped), and the
/// trim starts again without it, so one broken site never removes every loop.
fn auto_trim(ds: &Dataset, wins: &[Win], opts: &EnduranceOpts) -> TrimInfo {
    let len = wins.iter().map(|w| w.end).max().unwrap_or(0);
    let mut info = TrimInfo {
        mode: opts.trim,
        max_pct: opts.max_trim_pct,
        removed: vec![false; len],
        ..Default::default()
    };
    if opts.trim == Trim::Off {
        return info;
    }
    info.eligible = vec![false; wins.len()];
    if len == 0 {
        return info;
    }
    let need = 2 * opts.runs;
    let fails = |w: &Win, i: usize| i >= w.start && i < w.end && !is_good(ds, w.rs[i]);
    // (good, loops) of a site over the loops not removed
    let count = |w: &Win, removed: &[bool]| {
        (w.start..w.end).filter(|&i| !removed[i]).fold((0usize, 0usize), |(g, t), i| {
            (g + usize::from(is_good(ds, w.rs[i])), t + 1)
        })
    };
    let below = |w: &Win, removed: &[bool]| {
        let (g, t) = count(w, removed);
        t == 0 || (g as f64 * 100.0 / t as f64) < opts.yield_pct
    };
    let mut sites: Vec<usize> = (0..wins.len())
        .filter(|&k| {
            let w = &wins[k];
            w.complete != Some(false) && count(w, &info.removed).0 >= need
        })
        .collect();
    while !sites.is_empty() {
        let lo = sites.iter().map(|&k| wins[k].start).min().unwrap_or(0);
        let hi = sites.iter().map(|&k| wins[k].end).max().unwrap_or(0);
        let clean = |i: usize| sites.iter().all(|&k| !fails(&wins[k], i));
        let a = (lo..hi).find(|&i| clean(i)).unwrap_or(hi);
        let b = (lo..hi).rev().find(|&i| clean(i)).map_or(a, |b| b + 1);
        let mut removed = vec![false; len];
        for (i, r) in removed.iter_mut().enumerate() {
            *r = i < a || i >= b;
        }
        let mut between = 0;
        let window = hi - lo;
        let cap = (opts.max_trim_pct > 0.0).then(|| (window as f64 * opts.max_trim_pct / 100.0).floor() as usize);
        let gone = |removed: &[bool]| (lo..hi).filter(|&i| removed[i]).count();
        if opts.trim == Trim::Any {
            loop {
                if cap.is_some_and(|c| gone(&removed) >= c) {
                    break;
                }
                let low: Vec<usize> = sites
                    .iter()
                    .copied()
                    .filter(|&k| below(&wins[k], &removed))
                    .collect();
                if low.is_empty() {
                    break;
                }
                let best = (a..b)
                    .filter(|&i| !removed[i])
                    .map(|i| {
                        let on_low = low.iter().filter(|&&k| fails(&wins[k], i)).count();
                        let on_all = sites.iter().filter(|&&k| fails(&wins[k], i)).count();
                        (on_low, on_all, std::cmp::Reverse(i))
                    })
                    .filter(|x| x.0 > 0)
                    .max();
                let Some((_, _, std::cmp::Reverse(i))) = best else {
                    break;
                };
                removed[i] = true;
                between += 1;
            }
        }
        // the site furthest from making it is left out of the trim, and the trim runs again
        let short = |k: usize| {
            let w = &wins[k];
            let (g, t) = count(w, &removed);
            let enough = match opts.pick {
                Pick::Clean => g >= need,
                // raw compares the first and last loops kept as they are: they must all pass
                Pick::Raw => {
                    let kept: Vec<usize> = (w.start..w.end).filter(|&i| !removed[i]).collect();
                    kept.len() >= need
                        && kept[..opts.runs]
                            .iter()
                            .chain(&kept[kept.len() - opts.runs..])
                            .all(|&i| is_good(ds, w.rs[i]))
                }
            };
            (below(w, &removed) || !enough).then(|| if t == 0 { 0.0 } else { g as f64 / t as f64 })
        };
        // over the limit, the site with the lowest yield before the trim goes first
        let window_yield = |k: usize| {
            let (g, t) = count(&wins[k], &vec![false; len]);
            if t == 0 { 0.0 } else { g as f64 / t as f64 }
        };
        // the limit holds for the last site too: it is dropped rather than trimmed past it
        let worst = if cap.is_some_and(|c| gone(&removed) > c) {
            sites
                .iter()
                .map(|&k| (window_yield(k), k))
                .min_by(|x, y| x.0.total_cmp(&y.0))
        } else {
            sites
                .iter()
                .filter_map(|&k| short(k).map(|y| (y, k)))
                .min_by(|x, y| x.0.total_cmp(&y.0))
        };
        match worst {
            Some((_, k)) => sites.retain(|&s| s != k),
            None => {
                info.start = (lo..a.min(hi)).count();
                info.end = (b.max(lo)..hi).count();
                info.between = between;
                let gone: Vec<usize> = (lo..hi).filter(|&i| removed[i]).map(|i| i + 1).collect();
                info.loops = ranges(&gone);
                info.removed = removed;
                info.sites = sites.iter().map(|k| k.to_string()).collect();
                for &k in &sites {
                    info.eligible[k] = true;
                }
                return info;
            }
        }
    }
    info
}

/// The endurance check of `rows` of `ds` (one file).
pub fn endurance(ds: &Dataset, rows: &[usize], opts: &EnduranceOpts) -> Result<Endurance> {
    let n = opts.runs;
    check_runs(n)?;
    let mins = minutes(ds);
    let groups = by_site(ds, rows);
    let heads = groups.keys().map(|k| k.0).collect::<HashSet<_>>().len() > 1;
    // the trial window of every site
    let mut wins = vec![];
    let mut meta = vec![];
    for (&(head, site), rs) in &groups {
        let loops = rs.len();
        let mut end = loops;
        let mut cut_by = String::new();
        if opts.max_loops > 0 && loops > opts.max_loops {
            end = opts.max_loops;
            cut_by = "loops".to_string();
        }
        if opts.max_minutes > 0.0 {
            if let Some(i) = rs[..end]
                .iter()
                .position(|&r| mins[r].is_some_and(|m| m > opts.max_minutes))
            {
                end = i;
                cut_by = "time".to_string();
            }
        }
        let site_minutes = rs.last().and_then(|&r| mins[r]);
        let reached = (opts.max_loops > 0 && loops >= opts.max_loops)
            || (opts.max_minutes > 0.0 && site_minutes.is_some_and(|m| m >= opts.max_minutes));
        // short only when it can be told: a time target needs the file's start and finish
        let complete = if opts.max_loops == 0 && opts.max_minutes <= 0.0 {
            None
        } else if reached {
            Some(true)
        } else if opts.max_minutes <= 0.0 || site_minutes.is_some() {
            Some(false)
        } else {
            None
        };
        wins.push(Win {
            rs,
            start: opts.skip.min(end),
            end,
            complete,
        });
        meta.push((head, site, cut_by, site_minutes));
    }
    let mut trim = auto_trim(ds, &wins, opts);
    trim.sites = trim
        .sites
        .iter()
        .filter_map(|k| k.parse::<usize>().ok())
        .map(|k| site_label(meta[k].0, meta[k].1, heads))
        .collect();
    let mut sites = vec![];
    for (k, (w, (head, site, cut_by, site_minutes))) in wins.iter().zip(meta).enumerate() {
        let loops = w.rs.len();
        let complete = w.complete;
        let window = w.end - w.start;
        let window_good = (w.start..w.end).filter(|&i| is_good(ds, w.rs[i])).count();
        // the loops of the window the trim kept: (loop index, row)
        let win: Vec<(usize, usize)> = (w.start..w.end)
            .filter(|&i| !trim.removed.get(i).copied().unwrap_or(false))
            .map(|i| (i, w.rs[i]))
            .collect();
        let good_pos: Vec<usize> = (0..win.len()).filter(|&i| is_good(ds, win[i].1)).collect();
        let good = good_pos.len();
        let mut s = EnduranceSite {
            head,
            site,
            label: site_label(head, site, heads),
            loops,
            window,
            cut_by,
            good: window_good,
            window_yield: pct(window_good, window),
            trimmed: window - win.len(),
            kept: win.len(),
            kept_good: good,
            kept_yield: pct(good, win.len()),
            span: 0,
            span_good: 0,
            span_yield: None,
            skipped_lead: 0,
            skipped_tail: 0,
            first_loops: String::new(),
            last_loops: String::new(),
            first_all_pass: false,
            last_all_pass: false,
            minutes: site_minutes,
            complete,
            ok: false,
            status: String::new(),
            first: vec![],
            last: vec![],
            kept_rows: vec![],
        };
        let what = if opts.trim == Trim::Off { "in the window" } else { "after the trim" };
        // positions in `win` of the compared loops
        let picked: Option<(Vec<usize>, Vec<usize>)> = match opts.pick {
            Pick::Clean if good >= 2 * n => Some((
                good_pos[..n].to_vec(),
                good_pos[good - n..].to_vec(),
            )),
            Pick::Clean => {
                s.status = format!("Dropped: {good} good loops {what}, {} needed", 2 * n);
                None
            }
            Pick::Raw if win.len() >= 2 * n => Some((
                (0..n).collect(),
                (win.len() - n..win.len()).collect(),
            )),
            Pick::Raw => {
                s.status = format!("Dropped: {} loops {what}, {} needed", win.len(), 2 * n);
                None
            }
        };
        if let Some((fp, lp)) = picked {
            let (a, b) = (fp[0], lp[n - 1]);
            s.span = b - a + 1;
            s.span_good = good_pos.iter().filter(|&&i| i >= a && i <= b).count();
            s.span_yield = pct(s.span_good, s.span);
            s.skipped_lead = a;
            s.skipped_tail = win.len() - 1 - b;
            s.first = fp.iter().map(|&i| win[i].1).collect();
            s.last = lp.iter().map(|&i| win[i].1).collect();
            s.first_loops = ranges(&fp.iter().map(|&i| win[i].0 + 1).collect::<Vec<_>>());
            s.last_loops = ranges(&lp.iter().map(|&i| win[i].0 + 1).collect::<Vec<_>>());
            s.first_all_pass = s.first.iter().all(|&r| is_good(ds, r));
            s.last_all_pass = s.last.iter().all(|&r| is_good(ds, r));
            let y = s.span_yield.unwrap_or(0.0);
            let bad_first = s.first.iter().filter(|&&r| !is_good(ds, r)).count();
            let bad_last = s.last.iter().filter(|&&r| !is_good(ds, r)).count();
            s.status = if complete == Some(false) {
                format!("Dropped: short trial: {}", short_trial(loops, site_minutes, opts))
            } else if trim.eligible.get(k) == Some(&false) {
                trim_dropped(opts)
            } else if bad_first > 0 {
                format!("Dropped: {bad_first} failing loops in the first {n}")
            } else if bad_last > 0 {
                format!("Dropped: {bad_last} failing loops in the last {n}")
            } else if y < opts.yield_pct {
                format!(
                    "Dropped: yield {y:.2}% over the compared loops, below {}%",
                    opts.yield_pct
                )
            } else {
                s.ok = true;
                s.kept_rows = win[a..=b].iter().map(|x| x.1).collect();
                if s.trimmed > 0 {
                    format!("Compared after trimming {} loops", s.trimmed)
                } else {
                    "Compared".to_string()
                }
            };
        }
        sites.push(s);
    }
    let tests = numeric(ds);
    let mut out = vec![];
    let mut seen_tests = 0;
    for &t in &tests {
        let compared: Vec<(usize, Stat, Stat)> = sites
            .iter()
            .enumerate()
            .filter(|(_, s)| s.ok)
            .map(|(k, s)| (k, stat(ds, t, &s.first), stat(ds, t, &s.last)))
            .collect();
        // a test without a value on any compared site is not on these sites at all
        let any = compared.iter().any(|(_, a, b)| a.n + b.n > 0);
        if !any {
            continue;
        }
        for (k, a, b) in compared {
            let d = if a.n > 0 && b.n > 0 {
                a.range().map(|r| (a.mean - b.mean).abs() / r * 100.0)
            } else {
                None
            };
            let over = d.is_some_and(|d| d > opts.drift_pct);
            let low_cpk = [a.cpk, b.cpk]
                .iter()
                .any(|c| c.is_some_and(|c| c < opts.cpk_min));
            let mut notes = vec![];
            if over {
                notes.push(format!("Drift above {}%", opts.drift_pct));
            }
            if a.n == 0 || b.n == 0 {
                notes.push("No values on one side: drift not computed".to_string());
            } else if d.is_none() {
                notes.push("Not both limits: drift not computed".to_string());
            }
            if low_cpk {
                notes.push(format!("Cpk below {}", opts.cpk_min));
            }
            if a.limits_vary || b.limits_vary || !same(a.lo, b.lo) || !same(a.hi, b.hi) {
                notes.push("Limits change between loops: the first loop's are used".into());
            }
            if let Some(c) = count_note(n, [("first", a.n), ("last", b.n)]) {
                notes.push(c);
            }
            out.push(DriftRow {
                test: t,
                site: k,
                comment: if notes.is_empty() {
                    "No drift".into()
                } else {
                    notes.join("; ")
                },
                first: a,
                last: b,
                drift: d,
                over,
                low_cpk,
            });
        }
        seen_tests += usize::from(any);
    }
    Ok(Endurance {
        opts: opts.clone(),
        over: out.iter().filter(|r| r.over).count(),
        low_cpk: out.iter().filter(|r| r.low_cpk).count(),
        sites,
        short_rows: out.iter().filter(|r| r.first.n < n || r.last.n < n).count(),
        rows: out,
        tests: seen_tests,
        heads,
        trim,
    })
}

/// Why the auto trim left a site out.
fn trim_dropped(o: &EnduranceOpts) -> String {
    let limit = if o.max_trim_pct > 0.0 {
        format!(" within the trim limit of {}% of the loops", o.max_trim_pct)
    } else {
        String::new()
    };
    match o.pick {
        Pick::Clean => format!(
            "Dropped: cannot reach {}% yield with {} good loops{limit}",
            o.yield_pct,
            2 * o.runs
        ),
        Pick::Raw => format!(
            "Dropped: cannot reach {}% yield with passing first and last {} loops{limit}",
            o.yield_pct, o.runs
        ),
    }
}

/// The auto trim in words, for the title lines and the Summary sheet.
fn trim_text(t: &TrimInfo) -> String {
    match t.mode {
        Trim::Off => "off".to_string(),
        _ if t.start + t.end + t.between == 0 => "on: no loop left out".to_string(),
        m => format!(
            "{} (at most {}): {} loops left out at the start, {} at the end, {} between (loops {})",
            if m == Trim::Ends { "ends" } else { "ends and between" },
            if t.max_pct > 0.0 { format!("{}% of the loops", t.max_pct) } else { "no limit".to_string() },
            t.start,
            t.end,
            t.between,
            t.loops
        ),
    }
}

/// "300 loops, 20 min, below 1000 loops or 90 min"
fn short_trial(loops: usize, minutes: Option<f64>, o: &EnduranceOpts) -> String {
    let mut have = vec![format!("{loops} loops")];
    if let Some(m) = minutes {
        have.push(format!("{m:.0} min"));
    }
    let mut need = vec![];
    if o.max_loops > 0 {
        need.push(format!("{} loops", o.max_loops));
    }
    if o.max_minutes > 0.0 {
        need.push(format!("{} min", o.max_minutes));
    }
    format!("{}, below {}", have.join(", "), need.join(" or "))
}

// ---------------------------------------------------------------- loop compare

/// One side of the loop compare.
pub struct Side<'a> {
    pub name: String,
    pub ds: &'a Dataset,
    pub rows: Vec<usize>,
}

/// One site of the loop compare.
#[derive(Debug, Clone, Serialize)]
pub struct LoopSite {
    pub head: u8,
    pub site: u8,
    pub label: String,
    /// loops and good loops of the site in the old and new file (after the skipped loops)
    pub loops: [usize; 2],
    pub good: [usize; 2],
    /// 1-based loop numbers taken, as ranges
    pub used: [String; 2],
    pub ok: bool,
    pub status: String,
    #[serde(skip)]
    pub rows: [Vec<usize>; 2],
}

/// An old test and the new test it is compared with.
#[derive(Debug, Clone, Serialize)]
pub struct Pair {
    pub old: usize,
    pub new: usize,
    pub name_match: bool,
    pub unit_match: bool,
    pub limits_match: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct LoopRow {
    pub pair: usize,
    pub site: usize,
    pub old: Stat,
    pub new: Stat,
    /// |new mean - old mean| in % of the old limit range (the new one when the old has none)
    pub drift: Option<f64>,
    /// new sigma - old sigma, in % of the same range
    pub sd_change: Option<f64>,
    /// the limits of the compared loops agree (in the old unit)
    pub limits_match: bool,
    pub over: bool,
    pub low_cpk: bool,
    pub comment: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct LoopCompare {
    pub opts: LoopOpts,
    pub names: [String; 2],
    pub sites: Vec<LoopSite>,
    pub pairs: Vec<Pair>,
    /// numeric tests only in the old file, and only in the new one
    pub only_old: Vec<usize>,
    pub only_new: Vec<usize>,
    pub rows: Vec<LoopRow>,
    pub over: usize,
    pub low_cpk: usize,
    /// rows with fewer values than the loops compared
    pub short_rows: usize,
    pub heads: bool,
}

/// The pin of a multi-pin test's name ("VOH[DQ3]" -> "DQ3"), or "".
fn pin_of(name: &str) -> &str {
    name.strip_suffix(']')
        .and_then(|s| s.rfind('[').map(|i| &s[i + 1..]))
        .unwrap_or("")
}

fn same(a: Option<f64>, b: Option<f64>) -> bool {
    match (a, b) {
        // limits are 32-bit floats in STDF: 0.9 V and 900 mV x 0.001 differ in the 8th digit
        (Some(x), Some(y)) => (x - y).abs() <= 1e-6 * x.abs().max(y.abs()).max(1e-30),
        (None, None) => true,
        _ => false,
    }
}

/// Old and new tests paired: by key first, then by test number and pin when that is unique.
pub fn match_tests(a: &Dataset, b: &Dataset) -> (Vec<Pair>, Vec<usize>, Vec<usize>) {
    let ta = numeric(a);
    let tb = numeric(b);
    let in_b: HashSet<usize> = tb.iter().copied().collect();
    let mut used: HashSet<usize> = HashSet::new();
    let mut found: HashMap<usize, usize> = HashMap::new();
    for &t in &ta {
        if let Some(&u) = b.test_index.get(&a.tests[t].key) {
            if in_b.contains(&u) && used.insert(u) {
                found.insert(t, u);
            }
        }
    }
    let mut by_num: HashMap<(u32, &str), Vec<usize>> = HashMap::new();
    for &u in tb.iter().filter(|u| !used.contains(u)) {
        by_num
            .entry((b.tests[u].num, pin_of(&b.tests[u].name)))
            .or_default()
            .push(u);
    }
    let mut left: HashMap<(u32, &str), Vec<usize>> = HashMap::new();
    for &t in ta.iter().filter(|t| !found.contains_key(t)) {
        left.entry((a.tests[t].num, pin_of(&a.tests[t].name)))
            .or_default()
            .push(t);
    }
    for (k, ts) in &left {
        if let (1, Some(us)) = (ts.len(), by_num.get(k)) {
            if us.len() == 1 {
                found.insert(ts[0], us[0]);
                used.insert(us[0]);
            }
        }
    }
    let mut pairs = vec![];
    let mut only_old = vec![];
    for &t in &ta {
        match found.get(&t) {
            Some(&u) => {
                let (x, y) = (&a.tests[t], &b.tests[u]);
                // limits compared in the old unit, so 1 V and 1000 mV match
                let f = unit_factor(&y.units, &x.units);
                let scaled = |v: Option<f64>| v.map(|v| v * f.unwrap_or(1.0));
                pairs.push(Pair {
                    old: t,
                    new: u,
                    name_match: x.name == y.name,
                    unit_match: x.units == y.units,
                    limits_match: same(x.lo, scaled(y.lo)) && same(x.hi, scaled(y.hi)),
                });
            }
            None => only_old.push(t),
        }
    }
    let only_new = tb.into_iter().filter(|u| !used.contains(u)).collect();
    (pairs, only_old, only_new)
}

/// The 25x loop compare of the old file `a` (reference) and the new file `b`.
pub fn loop_compare(a: &Side, b: &Side, opts: &LoopOpts) -> Result<LoopCompare> {
    let n = opts.runs;
    check_runs(n)?;
    let ga = by_site(a.ds, &a.rows);
    let gb = by_site(b.ds, &b.rows);
    let keys: Vec<(u8, u8)> = ga
        .keys()
        .chain(gb.keys())
        .copied()
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    let heads = keys.iter().map(|k| k.0).collect::<HashSet<_>>().len() > 1;
    let empty = vec![];
    let mut sites = vec![];
    for key in keys {
        let mut s = LoopSite {
            head: key.0,
            site: key.1,
            label: site_label(key.0, key.1, heads),
            loops: [0; 2],
            good: [0; 2],
            used: Default::default(),
            ok: false,
            status: String::new(),
            rows: Default::default(),
        };
        let mut short = vec![];
        for (k, (side, g)) in [(a, &ga), (b, &gb)].into_iter().enumerate() {
            let rs = g.get(&key).unwrap_or(&empty);
            let start = opts.skip.min(rs.len());
            let win = &rs[start..];
            let good: Vec<usize> = (0..win.len()).filter(|&i| is_good(side.ds, win[i])).collect();
            s.loops[k] = win.len();
            s.good[k] = good.len();
            if good.len() < n {
                short.push(if rs.is_empty() {
                    format!("not in {}", side.name)
                } else {
                    format!("{} good loops in {}", good.len(), side.name)
                });
                continue;
            }
            let pos = match opts.take {
                Take::First => &good[..n],
                Take::Last => &good[good.len() - n..],
            };
            s.used[k] = ranges(&pos.iter().map(|&i| start + i + 1).collect::<Vec<_>>());
            s.rows[k] = pos.iter().map(|&i| win[i]).collect();
        }
        if short.is_empty() {
            s.ok = true;
            s.status = "Compared".into();
        } else {
            s.status = format!("Dropped: {} ({n} needed)", short.join(", "));
            s.used = Default::default();
            s.rows = Default::default();
        }
        sites.push(s);
    }
    let (pairs, only_old, only_new) = match_tests(a.ds, b.ds);
    let mut rows = vec![];
    for (p, pair) in pairs.iter().enumerate() {
        let (ta, tb) = (&a.ds.tests[pair.old], &b.ds.tests[pair.new]);
        // the new values in the old unit (mV to V); None when the units do not convert
        let factor = unit_factor(&tb.units, &ta.units);
        let compared: Vec<(usize, Stat, Stat)> = sites
            .iter()
            .enumerate()
            .filter(|(_, s)| s.ok)
            .map(|(k, s)| {
                let y = stat(b.ds, pair.new, &s.rows[1]);
                // shown and compared in the old unit when the units convert
                let y = match factor {
                    Some(f) if f != 1.0 => y.scaled(f),
                    _ => y,
                };
                (k, stat(a.ds, pair.old, &s.rows[0]), y)
            })
            .collect();
        if !compared.iter().any(|(_, x, y)| x.n + y.n > 0) {
            continue;
        }
        for (k, x, y) in compared {
            let both = x.n > 0 && y.n > 0;
            let range = factor.and(x.range().or(y.range())).filter(|_| both);
            let d = range.map(|r| (y.mean - x.mean).abs() / r * 100.0);
            let sd_change = range.map(|r| (y.sd - x.sd) / r * 100.0);
            let limits_match = same(x.lo, y.lo) && same(x.hi, y.hi);
            let over = d.is_some_and(|d| d > opts.drift_pct);
            let low_cpk = [x.cpk, y.cpk]
                .iter()
                .any(|c| c.is_some_and(|c| c < opts.cpk_min));
            let mut notes = vec![];
            if over {
                notes.push(format!("Mean drift above {}%", opts.drift_pct));
            }
            if factor.is_none() {
                notes.push(format!(
                    "Units {} and {} do not convert: drift not computed",
                    ta.units, tb.units
                ));
            } else if !both {
                notes.push("No values on one side: drift not computed".to_string());
            } else if d.is_none() {
                notes.push("Not both limits: drift not computed".to_string());
            }
            if low_cpk {
                notes.push(format!("Cpk below {}", opts.cpk_min));
            }
            if x.limits_vary || y.limits_vary {
                notes.push("Limits change between loops: the first loop's are used".into());
            }
            if let Some(c) = count_note(n, [("old", x.n), ("new", y.n)]) {
                notes.push(c);
            }
            if !limits_match {
                notes.push("Limits changed".into());
            }
            if !pair.name_match {
                notes.push("Name changed".into());
            }
            if !pair.unit_match && factor.is_some() {
                notes.push(format!("Unit changed: {} converted to {}", tb.units, ta.units));
            }
            rows.push(LoopRow {
                pair: p,
                site: k,
                comment: if notes.is_empty() {
                    "No drift".into()
                } else {
                    notes.join("; ")
                },
                old: x,
                new: y,
                drift: d,
                sd_change,
                limits_match,
                over,
                low_cpk,
            });
        }
    }
    Ok(LoopCompare {
        opts: opts.clone(),
        names: [a.name.clone(), b.name.clone()],
        over: rows.iter().filter(|r| r.over).count(),
        low_cpk: rows.iter().filter(|r| r.low_cpk).count(),
        short_rows: rows.iter().filter(|r| r.old.n < n || r.new.n < n).count(),
        sites,
        pairs,
        only_old,
        only_new,
        rows,
        heads,
    })
}

// ---------------------------------------------------------------- workbook

/// A cell of a written row.
enum C {
    T(String, u32),
    N(f64, u32),
    /// a number that may be missing: blank when None or not finite
    O(Option<f64>, u32),
    E,
}

fn t(s: impl Into<String>) -> C {
    C::T(s.into(), 0)
}

fn row_out(out: &mut impl Write, row: u64, cells: &[C]) -> io::Result<()> {
    write!(out, "<row r=\"{row}\">")?;
    for (i, c) in cells.iter().enumerate() {
        let l = column_letters(i);
        match c {
            C::T(s, st) => {
                if !s.is_empty() || *st != 0 {
                    write_styled(out, &l, row, s, *st)?
                }
            }
            C::N(v, st) | C::O(Some(v), st) if v.is_finite() => {
                if *st == 0 {
                    write!(out, "<c r=\"{l}{row}\"><v>{v}</v></c>")?
                } else {
                    write!(out, "<c r=\"{l}{row}\" s=\"{st}\"><v>{v}</v></c>")?
                }
            }
            _ => {}
        }
    }
    out.write_all(b"</row>")
}

fn cols(widths: &[u32]) -> String {
    let mut s = String::from("<cols>");
    for (i, w) in widths.iter().enumerate() {
        let k = i + 1;
        s.push_str(&format!(
            r#"<col min="{k}" max="{k}" width="{w}" customWidth="1"/>"#
        ));
    }
    s.push_str("</cols>");
    s
}

/// A sheet frozen below row `freeze` (0 = none).
fn sheet(dir: &Path, id: usize, name: &str, widths: &[u32], freeze: u64) -> Result<Sheet> {
    let view = if freeze > 0 {
        format!(
            r#"<sheetViews><sheetView workbookViewId="0"><pane ySplit="{freeze}" topLeftCell="A{}" activePane="bottomLeft" state="frozen"/></sheetView></sheetViews>"#,
            freeze + 1
        )
    } else {
        String::new()
    };
    let head = format!(
        r#"{view}<sheetFormatPr defaultRowHeight="15"/>{}"#,
        cols(widths)
    );
    xlsx::open_sheet(dir, id, name.to_string(), &head)
}

/// Closes a sheet: an autofilter over `filter` (e.g. "A9:Q40") and merged ranges.
fn finish(s: &mut Sheet, filter: Option<&str>, merges: &[String]) -> Result<()> {
    s.out.write_all(b"</sheetData>")?;
    if let Some(f) = filter {
        write!(s.out, r#"<autoFilter ref="{f}"/>"#)?;
    }
    if !merges.is_empty() {
        write!(s.out, r#"<mergeCells count="{}">"#, merges.len())?;
        for m in merges {
            write!(s.out, r#"<mergeCell ref="{m}"/>"#)?;
        }
        s.out.write_all(b"</mergeCells>")?;
    }
    s.out.write_all(b"</worksheet>")?;
    s.out.flush()?;
    Ok(())
}

/// Excel's names for the filter ranges: (sheet position, sheet name, range).
fn filter_names(f: &[(usize, &str, String)]) -> String {
    if f.is_empty() {
        return String::new();
    }
    let abs = |c: &str| {
        let at = c.find(|ch: char| ch.is_ascii_digit()).unwrap_or(c.len());
        format!("${}${}", &c[..at], &c[at..])
    };
    let mut s = String::from("<definedNames>");
    for (i, name, range) in f {
        let (a, b) = range.split_once(':').expect("a range has two ends");
        s.push_str(&format!(
            r#"<definedName name="_xlnm._FilterDatabase" localSheetId="{i}" hidden="1">'{}'!{}:{}</definedName>"#,
            name.replace('\'', "''"),
            abs(a),
            abs(b)
        ));
    }
    s.push_str("</definedNames>");
    s
}

fn check_out(out: &Path) -> Result<()> {
    if !out
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("xlsx"))
    {
        bail!("the report is an Excel workbook: save it as a .xlsx file");
    }
    Ok(())
}

fn drift_style(d: Option<f64>, over: bool) -> u32 {
    match (d, over) {
        (None, _) => 0,
        (_, true) => style::BAD,
        _ => style::GOOD,
    }
}

fn cpk_style(c: Option<f64>, min: f64) -> u32 {
    if c.is_some_and(|c| c < min) {
        style::WARN
    } else {
        0
    }
}

/// A limit as the file wrote it: STDF stores limits as 32-bit floats, so 0.9 reads as 0.899999976.
fn f32_clean(v: f64) -> f64 {
    (v as f32).to_string().parse().unwrap_or(v)
}

fn lim(v: Option<f64>) -> C {
    C::O(v.map(f32_clean), 0)
}

/// Files of a report and their MIR fields, for the Summary sheet.
fn file_rows(out: &mut impl Write, row: &mut u64, sides: &[(&str, &Dataset)]) -> io::Result<()> {
    let titles = [
        "Name", "File", "Lot", "Sublot", "Tester", "Tester type", "Program", "Revision",
        "Test code", "Temperature", "Start (UTC)", "Finish (UTC)",
    ];
    row_out(
        out,
        *row,
        &titles.iter().map(|x| C::T(x.to_string(), style::TITLE)).collect::<Vec<_>>(),
    )?;
    let when = |t: u32| {
        chrono::DateTime::from_timestamp(i64::from(t), 0)
            .filter(|_| t != 0)
            .map(|t| t.format("%Y-%m-%d %H:%M:%S").to_string())
            .unwrap_or_default()
    };
    for (name, ds) in sides {
        for s in &ds.sources {
            *row += 1;
            row_out(
                out,
                *row,
                &[
                    t(*name),
                    t(crate::data::short_name(&s.path)),
                    t(&s.lot_id),
                    t(&s.sublot_id),
                    t(&s.node_nam),
                    t(&s.tstr_typ),
                    t(&s.job_nam),
                    t(&s.job_rev),
                    t(&s.test_cod),
                    t(&s.tst_temp),
                    t(when(s.start_t)),
                    t(when(s.finish_t)),
                ],
            )?;
        }
    }
    Ok(())
}

/// The title and the lines under it; returns the next free row.
fn heading(out: &mut impl Write, title: &str, lines: &[(String, String)]) -> io::Result<u64> {
    row_out(out, 1, &[C::T(title.into(), style::HEADLINE)])?;
    let mut row = 2;
    for (k, v) in lines {
        row_out(out, row, &[C::T(k.clone(), style::SUBHEAD), C::E, C::E, C::T(v.clone(), style::SUBHEAD)])?;
        row += 1;
    }
    Ok(row + 1)
}

fn sites_text<'a>(labels: impl Iterator<Item = &'a str>) -> String {
    let v: Vec<&str> = labels.collect();
    if v.is_empty() {
        "none".into()
    } else {
        v.join(", ")
    }
}

/// What the endurance workbook holds.
#[derive(Debug, Clone, Serialize)]
pub struct Written {
    pub path: String,
    pub rows: usize,
    pub over: usize,
    pub low_cpk: usize,
    pub sites: usize,
    pub dropped: usize,
}

/// Writes the endurance check to the workbook `out`: Drift (one row per test and site), Sites and
/// Summary.
pub fn write_endurance(e: &Endurance, ds: &Dataset, name: &str, filter: &str, out: &Path) -> Result<Written> {
    check_out(out)?;
    let dir = xlsx::scratch_dir()?;
    let r = write_endurance_in(e, ds, name, filter, out, &dir);
    let _ = std::fs::remove_dir_all(&dir);
    r
}

fn write_endurance_in(
    e: &Endurance,
    ds: &Dataset,
    name: &str,
    filter: &str,
    out: &Path,
    dir: &Path,
) -> Result<Written> {
    let o = &e.opts;
    let n = o.runs;
    let ok: Vec<&EnduranceSite> = e.sites.iter().filter(|s| s.ok).collect();
    let dropped: Vec<&EnduranceSite> = e.sites.iter().filter(|s| !s.ok).collect();
    let all_pass = e.sites.iter().all(|s| s.ok);
    if e.rows.len() as u64 + 20 > xlsx::MAX_ROWS {
        bail!(
            "{} test-site rows do not fit on one Excel sheet: narrow the tests or sites with the part filter",
            e.rows.len()
        );
    }
    let short: Vec<&str> = ok
        .iter()
        .filter(|s| s.complete == Some(false))
        .map(|s| s.label.as_str())
        .collect();
    let trial = if o.max_loops == 0 && o.max_minutes <= 0.0 {
        "Not checked: no loop or time target set".to_string()
    } else if short.is_empty() {
        "Complete on every compared site".to_string()
    } else {
        format!("SHORT on site {}: see Sites", short.join(", "))
    };
    let lines = vec![
        ("Number of runs compared".to_string(), n.to_string()),
        ("Critical threshold drift %".to_string(), o.drift_pct.to_string()),
        ("Yield threshold %".to_string(), o.yield_pct.to_string()),
        (
            "Last run".to_string(),
            if all_pass {
                "All sites pass".to_string()
            } else {
                format!("{} of {} sites pass", ok.len(), e.sites.len())
            },
        ),
        ("Trial length".to_string(), trial),
        ("Auto trim".to_string(), trim_text(&e.trim)),
        (
            "Sites compared".to_string(),
            sites_text(ok.iter().map(|s| s.label.as_str())),
        ),
        (
            "Sites dropped".to_string(),
            sites_text(dropped.iter().map(|s| s.label.as_str())),
        ),
        (
            "Tests above the drift threshold".to_string(),
            format!("{} of {} test-site rows", e.over, e.rows.len()),
        ),
        (
            format!("Rows with fewer than {n} values"),
            e.short_rows.to_string(),
        ),
    ];
    let widths = [10, 40, 8, 12, 12, 12, 6, 13, 13, 9, 9, 13, 13, 9, 9, 10, 36, 8, 8];
    let mut s = sheet(dir, 0, "Drift", &widths, 0)?;
    let mut row = heading(&mut s.out, "TP Qualification Drift Analysis", &lines)?;
    // freeze is set on the title rows' sheet view: rebuild with the right pane after the header
    let first_t = format!("First {n} runs");
    let last_t = format!("Last {n} runs");
    let mut group = vec![];
    group.resize_with(7, || C::E);
    group.push(C::T(first_t.clone(), style::GROUP));
    for _ in 0..3 {
        group.push(C::T(String::new(), style::GROUP));
    }
    group.push(C::T(last_t.clone(), style::GROUP + 2));
    for _ in 0..3 {
        group.push(C::T(String::new(), style::GROUP + 2));
    }
    row_out(&mut s.out, row, &group)?;
    let merges = vec![
        format!("H{row}:K{row}"),
        format!("L{row}:O{row}"),
        "A1:S1".to_string(),
    ];
    row += 1;
    let titles = [
        "TestNumber", "Parameter", "Unit", "LSL", "USL", "Limits", "Site", "Average", "Deviation",
        "Cp", "Cpk", "Average", "Deviation", "Cp", "Cpk", "%Drift", "Comment/Remark", "N first",
        "N last",
    ];
    let header = row;
    row_out(
        &mut s.out,
        row,
        &titles.iter().map(|x| C::T(x.to_string(), style::TITLE)).collect::<Vec<_>>(),
    )?;
    for r in &e.rows {
        row += 1;
        let ti = &ds.tests[r.test];
        let site = &e.sites[r.site];
        row_out(
            &mut s.out,
            row,
            &[
                C::N(f64::from(ti.num), 0),
                t(&ti.name),
                t(&ti.units),
                lim(r.first.lo),
                lim(r.first.hi),
                C::O(r.first.lo.zip(r.first.hi).map(|(l, h)| f32_clean(f32_clean(h) - f32_clean(l))), 0),
                C::T(site.label.clone(), 0),
                C::N(r.first.mean, 0),
                C::N(r.first.sd, 0),
                C::O(r.first.cp, 0),
                C::O(r.first.cpk, cpk_style(r.first.cpk, o.cpk_min)),
                C::N(r.last.mean, 0),
                C::N(r.last.sd, 0),
                C::O(r.last.cp, 0),
                C::O(r.last.cpk, cpk_style(r.last.cpk, o.cpk_min)),
                C::O(r.drift, drift_style(r.drift, r.over)),
                t(&r.comment),
                C::N(r.first.n as f64, if r.first.n < n { style::FAIL } else { 0 }),
                C::N(r.last.n as f64, if r.last.n < n { style::FAIL } else { 0 }),
            ],
        )?;
    }
    let filter_ref = format!("A{header}:S{}", row.max(header));
    s.rows = row;
    finish(&mut s, Some(&filter_ref), &merges)?;
    let mut sheets = vec![freeze_at(dir, s, header)?];

    // Sites
    let widths = [8, 10, 10, 10, 10, 11, 9, 9, 11, 10, 10, 11, 12, 12, 16, 16, 9, 9, 12, 10, 60];
    let mut st = sheet(dir, 1, "Sites", &widths, 1)?;
    let titles = [
        "Site", "Loops", "In window", "Window cut by", "Good in window", "Window yield %",
        "Trimmed", "Kept", "Kept yield %", "Compared span", "Good in span", "Span yield %", "Skipped at start", "Skipped at end",
        &format!("First {n} loops"), &format!("Last {n} loops"), "First all pass", "Last all pass",
        "Minutes (est.)", "Full trial", "Status",
    ];
    row_out(
        &mut st.out,
        1,
        &titles.iter().map(|x| C::T(x.to_string(), style::TITLE)).collect::<Vec<_>>(),
    )?;
    let mut row = 1;
    for x in &e.sites {
        row += 1;
        let yes = |b: bool, has: bool| t(if !has { "" } else if b { "yes" } else { "no" });
        let has = !x.first.is_empty();
        let ys = |y: Option<f64>| match y {
            Some(v) if v >= o.yield_pct => C::N(v, style::GOOD),
            Some(v) => C::N(v, style::BAD),
            None => C::E,
        };
        row_out(
            &mut st.out,
            row,
            &[
                t(&x.label),
                C::N(x.loops as f64, 0),
                C::N(x.window as f64, 0),
                t(&x.cut_by),
                C::N(x.good as f64, 0),
                ys(x.window_yield),
                C::N(x.trimmed as f64, 0),
                C::N(x.kept as f64, 0),
                ys(x.kept_yield),
                if has { C::N(x.span as f64, 0) } else { C::E },
                if has { C::N(x.span_good as f64, 0) } else { C::E },
                ys(x.span_yield),
                if has { C::N(x.skipped_lead as f64, 0) } else { C::E },
                if has { C::N(x.skipped_tail as f64, 0) } else { C::E },
                t(&x.first_loops),
                t(&x.last_loops),
                yes(x.first_all_pass, has),
                yes(x.last_all_pass, has),
                C::O(x.minutes.map(|m| (m * 10.0).round() / 10.0), 0),
                match x.complete {
                    Some(true) => t("yes"),
                    Some(false) => C::T("no".into(), style::FAIL),
                    None => C::E,
                },
                C::T(x.status.clone(), if x.ok { 0 } else { style::FAIL }),
            ],
        )?;
    }
    st.rows = row;
    finish(&mut st, None, &[])?;
    sheets.push(st);

    // Summary
    let mut sm = sheet(dir, 2, "Summary", &[34, 30, 16, 16, 16, 16, 30, 12, 12, 12, 20, 20], 0)?;
    let filter = if filter.trim().is_empty() { "none" } else { filter.trim() };
    let pick = match o.pick {
        Pick::Clean => format!("Clean: first and last {n} good loops of each site; failing loops between are skipped"),
        Pick::Raw => format!("Raw: first and last {n} loops of each site as they are; all must pass"),
    };
    let window = {
        let mut w = vec![];
        if o.skip > 0 {
            w.push(format!("first {} loops skipped", o.skip));
        }
        if o.max_loops > 0 {
            w.push(format!("ends after {} loops", o.max_loops));
        }
        if o.max_minutes > 0.0 {
            w.push(format!("ends after {} minutes (estimated from the file's start and finish)", o.max_minutes));
        }
        if w.is_empty() {
            "every loop".to_string()
        } else {
            w.join(", ")
        }
    };
    let lines = [
        ("Report", "Endurance check: mean drift of each test on each site".to_string()),
        ("Drift", "ABS(Mean first - Mean last) / (USL - LSL) x 100%".to_string()),
        ("Compared loops", pick),
        ("Trial window", window),
        ("Auto trim", trim_text(&e.trim)),
        (
            "Trimmed for sites",
            if e.trim.sites.is_empty() { "none".to_string() } else { e.trim.sites.join(", ") },
        ),
        ("Valid site", format!("yield at least {}% over the compared loops, and the compared loops all pass", o.yield_pct)),
        ("Comment on", format!("drift above {}%, Cpk below {}", o.drift_pct, o.cpk_min)),
        ("Part filter", filter.to_string()),
        ("Test-site rows", e.rows.len().to_string()),
        ("Tests", e.tests.to_string()),
        ("Rows with drift above the threshold", e.over.to_string()),
        ("Rows with Cpk below the minimum", e.low_cpk.to_string()),
    ];
    let mut row = 0;
    for (k, v) in lines {
        row += 1;
        row_out(&mut sm.out, row, &[C::T(k.into(), style::BOLD), t(v)])?;
    }
    row += 2;
    file_rows(&mut sm.out, &mut row, &[(name, ds)])?;
    sm.rows = row;
    finish(&mut sm, None, &[])?;
    sheets.push(sm);

    xlsx::write_zip(out, &sheets, &filter_names(&[(0, "Drift", filter_ref)]))?;
    Ok(Written {
        path: out.to_string_lossy().to_string(),
        rows: e.rows.len(),
        over: e.over,
        low_cpk: e.low_cpk,
        sites: ok.len(),
        dropped: dropped.len(),
    })
}

/// The sheet with its view frozen under the table's title row: the sheet view goes before the data,
/// so the finished sheet is rewritten with it.
fn freeze_at(dir: &Path, s: Sheet, header: u64) -> Result<Sheet> {
    let Sheet { name, rows, path, out } = s;
    drop(out);
    let body = std::fs::read_to_string(&path)?;
    let pane = format!(
        r#"<sheetViews><sheetView workbookViewId="0"><pane xSplit="2" ySplit="{header}" topLeftCell="C{}" activePane="bottomRight" state="frozen"/></sheetView></sheetViews><sheetFormatPr"#,
        header + 1
    );
    let body = body.replacen("<sheetFormatPr", &pane, 1);
    let new_path = dir.join(format!("{}.frozen.xml", path.file_stem().map_or("s".into(), |s| s.to_string_lossy().to_string())));
    std::fs::write(&new_path, body)?;
    let _ = std::fs::remove_file(&path);
    let file = std::fs::OpenOptions::new().append(true).open(&new_path)?;
    Ok(Sheet {
        name,
        rows,
        path: new_path,
        out: io::BufWriter::new(file),
    })
}

/// Writes the loop compare to the workbook `out`: Mean drift (one row per test and site), Sites,
/// Unmatched tests and Summary.
pub fn write_loop_compare(c: &LoopCompare, a: &Side, b: &Side, filter: &str, out: &Path) -> Result<Written> {
    check_out(out)?;
    let dir = xlsx::scratch_dir()?;
    let r = write_loop_in(c, a, b, filter, out, &dir);
    let _ = std::fs::remove_dir_all(&dir);
    r
}

fn write_loop_in(c: &LoopCompare, a: &Side, b: &Side, filter: &str, out: &Path, dir: &Path) -> Result<Written> {
    let o = &c.opts;
    let n = o.runs;
    let ok: Vec<&LoopSite> = c.sites.iter().filter(|s| s.ok).collect();
    let dropped: Vec<&LoopSite> = c.sites.iter().filter(|s| !s.ok).collect();
    let prog = |ds: &Dataset| {
        ds.sources
            .first()
            .map(|s| [s.job_nam.as_str(), s.job_rev.as_str()].iter().filter(|x| !x.is_empty()).copied().collect::<Vec<_>>().join(" rev "))
            .unwrap_or_default()
    };
    let lines = vec![
        ("Reference (old)".to_string(), format!("{}  {}", a.name, prog(a.ds))),
        ("New".to_string(), format!("{}  {}", b.name, prog(b.ds))),
        ("Loops compared per site".to_string(), format!("{n} ({} {n} good loops)", o.take.name())),
        ("Critical threshold drift %".to_string(), o.drift_pct.to_string()),
        ("Sites compared".to_string(), sites_text(ok.iter().map(|s| s.label.as_str()))),
        ("Sites dropped".to_string(), sites_text(dropped.iter().map(|s| s.label.as_str()))),
        ("Tests above the drift threshold".to_string(), format!("{} of {} test-site rows", c.over, c.rows.len())),
        (format!("Rows with fewer than {n} values"), c.short_rows.to_string()),
    ];
    if c.rows.len() as u64 + 20 > xlsx::MAX_ROWS {
        bail!(
            "{} test-site rows do not fit on one Excel sheet: narrow the tests or sites with the part filter",
            c.rows.len()
        );
    }
    let widths = [
        10, 36, 8, 7, 11, 11, // A-F: test, name, unit, site, LSL, USL (old)
        12, 11, 11, 11, 9, 9, 6, // G-M old: avg, sd, min, max, cp, cpk, fails
        36, 11, 11, 12, 11, 11, 11, 9, 9, 6, // N-W new: name, LSL, USL, avg, sd, min, max, cp, cpk, fails
        12, 14, 12, 12, 13, 40, // X-AC: drift, sd change, name, unit, limits match, comment
        8, 8, // AD-AE: values old, new
    ];
    let mut s = sheet(dir, 0, "Mean drift", &widths, 0)?;
    let mut row = heading(&mut s.out, "25x Loop Mean Drift by Site", &lines)?;
    let mut group: Vec<C> = (0..6).map(|_| C::E).collect();
    group.push(C::T(a.name.clone(), style::GROUP));
    group.extend((0..6).map(|_| C::T(String::new(), style::GROUP)));
    group.push(C::T(b.name.clone(), style::GROUP + 2));
    group.extend((0..9).map(|_| C::T(String::new(), style::GROUP + 2)));
    group.push(C::T("Comparison".into(), style::GROUP + 4));
    group.extend((0..5).map(|_| C::T(String::new(), style::GROUP + 4)));
    row_out(&mut s.out, row, &group)?;
    let merges = vec![
        format!("G{row}:M{row}"),
        format!("N{row}:W{row}"),
        format!("X{row}:AC{row}"),
        "A1:AE1".to_string(),
    ];
    row += 1;
    let titles = [
        "TestNumber", "TestName", "Unit", "Site", "LSL", "USL", "Average", "Deviation", "Min", "Max",
        "Cp", "Cpk", "Fails", "TestName", "LSL", "USL", "Average", "Deviation", "Min", "Max", "Cp",
        "Cpk", "Fails", "MeanDrift %", "DeltaDeviation/Limits %", "Name match", "Unit match",
        "Limits match", "Comment/Remark", "N old", "N new",
    ];
    let header = row;
    row_out(&mut s.out, row, &titles.iter().map(|x| C::T(x.to_string(), style::TITLE)).collect::<Vec<_>>())?;
    let tf = |b: bool| C::T((if b { "TRUE" } else { "FALSE" }).into(), if b { 0 } else { style::FAIL });
    for r in &c.rows {
        row += 1;
        let p = &c.pairs[r.pair];
        let (x, y) = (&a.ds.tests[p.old], &b.ds.tests[p.new]);
        row_out(
            &mut s.out,
            row,
            &[
                C::N(f64::from(x.num), 0),
                t(&x.name),
                t(&x.units),
                t(&c.sites[r.site].label),
                lim(r.old.lo),
                lim(r.old.hi),
                C::N(r.old.mean, 0),
                C::N(r.old.sd, 0),
                C::N(r.old.min, 0),
                C::N(r.old.max, 0),
                C::O(r.old.cp, 0),
                C::O(r.old.cpk, cpk_style(r.old.cpk, o.cpk_min)),
                C::N(r.old.fails as f64, 0),
                t(&y.name),
                lim(r.new.lo),
                lim(r.new.hi),
                C::N(r.new.mean, 0),
                C::N(r.new.sd, 0),
                C::N(r.new.min, 0),
                C::N(r.new.max, 0),
                C::O(r.new.cp, 0),
                C::O(r.new.cpk, cpk_style(r.new.cpk, o.cpk_min)),
                C::N(r.new.fails as f64, 0),
                C::O(r.drift, drift_style(r.drift, r.over)),
                C::O(r.sd_change, 0),
                tf(p.name_match),
                tf(p.unit_match),
                tf(r.limits_match),
                t(&r.comment),
                C::N(r.old.n as f64, if r.old.n < n { style::FAIL } else { 0 }),
                C::N(r.new.n as f64, if r.new.n < n { style::FAIL } else { 0 }),
            ],
        )?;
    }
    let filter_ref = format!("A{header}:AE{}", row.max(header));
    s.rows = row;
    finish(&mut s, Some(&filter_ref), &merges)?;
    let mut sheets = vec![freeze_at(dir, s, header)?];

    // Sites
    let mut st = sheet(dir, 1, "Sites", &[8, 12, 12, 16, 12, 12, 16, 60], 1)?;
    let titles = [
        "Site".to_string(),
        format!("{} loops", a.name),
        format!("{} good", a.name),
        format!("{} loops used", a.name),
        format!("{} loops", b.name),
        format!("{} good", b.name),
        format!("{} loops used", b.name),
        "Status".to_string(),
    ];
    row_out(&mut st.out, 1, &titles.iter().map(|x| C::T(x.clone(), style::TITLE)).collect::<Vec<_>>())?;
    let mut row = 1;
    for x in &c.sites {
        row += 1;
        let g = |k: usize| C::N(x.good[k] as f64, if x.good[k] >= n { 0 } else { style::FAIL });
        row_out(
            &mut st.out,
            row,
            &[
                t(&x.label),
                C::N(x.loops[0] as f64, 0),
                g(0),
                t(&x.used[0]),
                C::N(x.loops[1] as f64, 0),
                g(1),
                t(&x.used[1]),
                C::T(x.status.clone(), if x.ok { 0 } else { style::FAIL }),
            ],
        )?;
    }
    st.rows = row;
    finish(&mut st, None, &[])?;
    sheets.push(st);

    // Unmatched tests
    if (c.only_old.len() + c.only_new.len()) as u64 + 1 > xlsx::MAX_ROWS {
        bail!("too many unmatched tests for one Excel sheet");
    }
    let mut um = sheet(dir, 2, "Unmatched tests", &[24, 12, 44, 10, 12, 12], 1)?;
    row_out(
        &mut um.out,
        1,
        &["Only in", "TestNumber", "TestName", "Unit", "LSL", "USL"]
            .iter()
            .map(|x| C::T(x.to_string(), style::TITLE))
            .collect::<Vec<_>>(),
    )?;
    let mut row = 1;
    for (side, list) in [(a, &c.only_old), (b, &c.only_new)] {
        for &ti in list.iter() {
            row += 1;
            let x = &side.ds.tests[ti];
            row_out(&mut um.out, row, &[t(&side.name), C::N(f64::from(x.num), 0), t(&x.name), t(&x.units), lim(x.lo), lim(x.hi)])?;
        }
    }
    um.rows = row;
    finish(&mut um, None, &[])?;
    sheets.push(um);

    // Summary
    let mut sm = sheet(dir, 3, "Summary", &[34, 30, 16, 16, 16, 16, 30, 12, 12, 12, 20, 20], 0)?;
    let filter = if filter.trim().is_empty() { "none" } else { filter.trim() };
    let lines = [
        ("Report", "25x loop compare: mean drift of each test on each site, old vs new".to_string()),
        ("Mean drift", "ABS(Mean new - Mean old) / (USL - LSL) x 100%, limits of the old file (the new file's when the old has none)".to_string()),
        ("Cp and Cpk", "each file with its own limits".to_string()),
        ("Loops compared", format!("the {} {n} good loops of each site in each file{}", o.take.name(), if o.skip > 0 { format!(", after skipping the first {} loops", o.skip) } else { String::new() })),
        ("Valid site", format!("{n} good loops in both files")),
        ("Tests paired", "by test number and name, else by test number (and pin) when that is unique".to_string()),
        ("Comment on", format!("mean drift above {}%, Cpk below {}, changed limits, names or units", o.drift_pct, o.cpk_min)),
        ("Part filter", filter.to_string()),
        ("Tests paired", c.pairs.len().to_string()),
        ("Tests only in the old file", c.only_old.len().to_string()),
        ("Tests only in the new file", c.only_new.len().to_string()),
        ("Test-site rows", c.rows.len().to_string()),
        ("Rows with drift above the threshold", c.over.to_string()),
        ("Rows with Cpk below the minimum", c.low_cpk.to_string()),
    ];
    let mut row = 0;
    for (k, v) in lines {
        row += 1;
        row_out(&mut sm.out, row, &[C::T(k.into(), style::BOLD), t(v)])?;
    }
    row += 2;
    file_rows(&mut sm.out, &mut row, &[(a.name.as_str(), a.ds), (b.name.as_str(), b.ds)])?;
    sm.rows = row;
    finish(&mut sm, None, &[])?;
    sheets.push(sm);

    xlsx::write_zip(out, &sheets, &filter_names(&[(0, "Mean drift", filter_ref)]))?;
    Ok(Written {
        path: out.to_string_lossy().to_string(),
        rows: c.rows.len(),
        over: c.over,
        low_cpk: c.low_cpk,
        sites: ok.len(),
        dropped: dropped.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::{Part, PartLimits, TestInfo};

    fn part(site: u8, pass: bool) -> Part {
        Part {
            file: 0,
            part_id: String::new(),
            head: 1,
            site,
            x: -32768,
            y: -32768,
            hbin: if pass { 1 } else { 5 },
            sbin: if pass { 1 } else { 5 },
            passed: Some(pass),
            part_flg: 0,
            num_test: 1,
            test_t: 100,
            wafer: u32::MAX,
            part_txt: String::new(),
            first_fail: None,
            alarms: vec![],
            fails: vec![],
            partial: false,
            inst_alarms: 0,
        }
    }

    fn test(num: u32, name: &str, lo: f64, hi: f64) -> TestInfo {
        TestInfo {
            key: format!("{num}:{name}"),
            num,
            name: name.into(),
            kind: TestKind::Parametric,
            units: "V".into(),
            lo: Some(lo),
            hi: Some(hi),
            lo_spec: None,
            hi_spec: None,
            res_scal: 0,
            exec_count: 0,
            fail_count: 0,
        }
    }

    /// Parts on sites 1 and 2 in turn, with one test: (site, pass, value).
    fn ds(parts: &[(u8, bool, f32)], t: TestInfo) -> Dataset {
        let mut d = Dataset {
            parts: parts.iter().map(|&(s, p, _)| part(s, p)).collect(),
            values: vec![parts.iter().map(|x| x.2).collect()],
            ..Default::default()
        };
        d.test_index.insert(t.key.clone(), 0);
        d.tests.push(t);
        d
    }

    fn loops(site: u8, vals: &[(bool, f32)]) -> Vec<(u8, bool, f32)> {
        vals.iter().map(|&(p, v)| (site, p, v)).collect()
    }

    #[test]
    fn ranges_join_runs() {
        assert_eq!(ranges(&[1, 2, 3, 5, 7, 8]), "1-3, 5, 7-8");
        assert_eq!(ranges(&[]), "");
    }

    #[test]
    fn endurance_drift_is_per_site_and_skips_noisy_loops() {
        // site 1: two failing warm-up loops, 4 good at 1.0, 4 good at 1.6 (drift 6% of range 10)
        let mut p = loops(1, &[(false, 9.0), (false, 9.0)]);
        p.extend(loops(1, &[(true, 1.0); 4]));
        p.extend(loops(1, &[(true, 1.6); 4]));
        // site 2: steady at 2.0
        p.extend(loops(2, &[(true, 2.0); 8]));
        let d = ds(&p, test(100, "VOUT", 0.0, 10.0));
        let opts = EnduranceOpts { runs: 4, yield_pct: 50.0, ..Default::default() };
        let e = endurance(&d, &d.all_rows(), &opts).unwrap();
        assert_eq!(e.sites.len(), 2);
        assert!(e.sites.iter().all(|s| s.ok));
        assert_eq!(e.sites[0].skipped_lead, 2);
        assert_eq!(e.sites[0].first_loops, "3-6");
        assert_eq!(e.rows.len(), 2);
        let r1 = &e.rows[0];
        assert!((r1.drift.unwrap() - 6.0).abs() < 1e-4);
        assert!(r1.over);
        assert_eq!(e.rows[1].drift, Some(0.0));
        assert_eq!(e.rows[1].comment, "No drift");
    }

    #[test]
    fn endurance_drops_a_low_yield_site_and_raw_needs_passing_ends() {
        let mut p = loops(1, &[(true, 1.0), (false, 1.0), (true, 1.0), (false, 1.0), (true, 1.0), (true, 1.0)]);
        p.extend(loops(2, &[(true, 1.0); 6]));
        let d = ds(&p, test(100, "VOUT", 0.0, 10.0));
        let opts = EnduranceOpts { runs: 2, ..Default::default() };
        let e = endurance(&d, &d.all_rows(), &opts).unwrap();
        assert!(!e.sites[0].ok, "{}", e.sites[0].status);
        assert!(e.sites[0].status.contains("yield"));
        assert!(e.sites[1].ok);
        let raw = EnduranceOpts { runs: 2, pick: Pick::Raw, yield_pct: 0.0, ..Default::default() };
        let e = endurance(&d, &d.all_rows(), &raw).unwrap();
        assert!(!e.sites[0].ok);
        assert!(e.sites[0].status.contains("failing loops in the first"));
    }

    #[test]
    fn endurance_window_cut_by_loops() {
        let mut v = vec![(true, 1.0f32); 4];
        v.extend(vec![(true, 5.0f32); 4]);
        let d = ds(&loops(1, &v), test(100, "VOUT", 0.0, 10.0));
        let opts = EnduranceOpts { runs: 2, max_loops: 4, ..Default::default() };
        let e = endurance(&d, &d.all_rows(), &opts).unwrap();
        assert_eq!(e.sites[0].window, 4);
        assert_eq!(e.sites[0].cut_by, "loops");
        assert_eq!(e.rows[0].drift, Some(0.0));
    }

    #[test]
    fn loop_compare_drops_sites_short_of_good_loops_and_pairs_renamed_tests() {
        let mut pa = loops(1, &[(true, 1.0); 3]);
        pa.extend(loops(2, &[(true, 1.0), (false, 1.0), (true, 1.0)]));
        let mut pb = loops(1, &[(true, 1.5); 4]);
        pb.extend(loops(2, &[(true, 1.0); 3]));
        let a = ds(&pa, test(100, "VOUT", 0.0, 10.0));
        let b = ds(&pb, test(100, "VOUT_NEW", 0.0, 5.0));
        let (sa, sb) = (
            Side { name: "old".into(), ds: &a, rows: a.all_rows() },
            Side { name: "new".into(), ds: &b, rows: b.all_rows() },
        );
        let c = loop_compare(&sa, &sb, &LoopOpts { runs: 3, ..Default::default() }).unwrap();
        assert_eq!(c.pairs.len(), 1);
        assert!(!c.pairs[0].name_match && !c.pairs[0].limits_match);
        assert!(c.sites[0].ok);
        assert!(!c.sites[1].ok);
        assert!(c.sites[1].status.contains("2 good loops in old"));
        assert_eq!(c.rows.len(), 1);
        assert!((c.rows[0].drift.unwrap() - 5.0).abs() < 1e-4);
        assert!(!c.rows[0].over);
        assert!(c.rows[0].comment.contains("Limits changed"));
    }

    #[test]
    fn units_convert_by_si_prefix_only() {
        assert_eq!(unit_factor("mV", "V"), Some(1e-3));
        assert!((unit_factor("uA", "nA").unwrap() - 1e3).abs() < 1e-9);
        assert_eq!(unit_factor("V", "V"), Some(1.0));
        assert_eq!(unit_factor("V", "A"), None);
        assert_eq!(unit_factor("m", "V"), None);
    }

    #[test]
    fn loop_compare_converts_units_and_refuses_unconvertible_ones() {
        let pa = loops(1, &[(true, 1.0); 3]);
        let pb = loops(1, &[(true, 1000.0); 3]);
        let a = ds(&pa, test(100, "VOUT", 0.0, 2.0));
        let mut tb = test(100, "VOUT", 0.0, 2000.0);
        tb.units = "mV".into();
        let b = ds(&pb, tb);
        let (sa, sb) = (
            Side { name: "old".into(), ds: &a, rows: a.all_rows() },
            Side { name: "new".into(), ds: &b, rows: b.all_rows() },
        );
        let c = loop_compare(&sa, &sb, &LoopOpts { runs: 3, ..Default::default() }).unwrap();
        assert!(c.pairs[0].limits_match);
        assert_eq!(c.rows[0].drift, Some(0.0));
        assert!(c.rows[0].comment.contains("mV converted to V"));
        // the new statistics are shown in the old unit
        assert!((c.rows[0].new.mean - 1.0).abs() < 1e-9);
        assert_eq!(c.rows[0].new.hi, Some(2.0));
        let mut tc = test(100, "VOUT", 0.0, 2.0);
        tc.units = "A".into();
        let cb = ds(&pb, tc);
        let sc = Side { name: "new".into(), ds: &cb, rows: cb.all_rows() };
        let c = loop_compare(&sa, &sc, &LoopOpts { runs: 3, ..Default::default() }).unwrap();
        assert_eq!(c.rows[0].drift, None);
        assert!(c.rows[0].comment.contains("do not convert"));
    }

    #[test]
    fn missing_values_and_short_trials_are_flagged() {
        let mut p = loops(1, &[(true, 1.0); 4]);
        p.extend(loops(1, &[(true, f32::NAN), (true, f32::NAN), (true, 1.0), (true, 1.0)]));
        let d = ds(&p, test(100, "VOUT", 0.0, 10.0));
        let opts = EnduranceOpts { runs: 4, max_loops: 1000, ..Default::default() };
        let e = endurance(&d, &d.all_rows(), &opts).unwrap();
        assert_eq!(e.sites[0].complete, Some(false));
        assert!(!e.sites[0].ok);
        assert!(e.sites[0].status.contains("short trial: 8 loops"), "{}", e.sites[0].status);
        // a time target without file times cannot be told short
        let timed = EnduranceOpts { runs: 4, max_loops: 1000, max_minutes: 90.0, ..Default::default() };
        let e = endurance(&d, &d.all_rows(), &timed).unwrap();
        assert_eq!(e.sites[0].complete, None);
        assert!(e.sites[0].ok);
        let e = endurance(&d, &d.all_rows(), &EnduranceOpts { runs: 4, ..Default::default() }).unwrap();
        assert_eq!(e.short_rows, 1);
        assert!(e.rows[0].comment.contains("Fewer than 4 values: first 4, last 2"));
        assert!(endurance(&d, &d.all_rows(), &EnduranceOpts { runs: usize::MAX, ..Default::default() }).is_err());
    }

    /// Three sites, `n` loops each, in touchdown order: site 1 fails loops 0-1 (unstable start),
    /// site 2 fails every 8th loop from loop 4 (about 88% yield), site 3 passes 3 loops only.
    fn unstable(n: usize) -> Dataset {
        let mut p = vec![];
        for i in 0..n {
            p.push((1, i >= 2, 1.0));
            p.push((2, i < 4 || (i - 4) % 8 != 0, 1.0));
            p.push((3, i < 3, 1.0));
        }
        ds(&p, test(100, "VOUT", 0.0, 10.0))
    }

    #[test]
    fn auto_trim_cuts_failing_ends_then_bad_loops_between() {
        let d = unstable(80);
        let base = EnduranceOpts { runs: 10, ..Default::default() };
        let off = endurance(&d, &d.all_rows(), &base).unwrap();
        assert!(off.sites[0].ok, "{}", off.sites[0].status);
        assert!(!off.sites[1].ok, "{}", off.sites[1].status);
        assert!(!off.sites[2].ok);
        // ends: the unstable start of site 1 goes for every site; site 2 stays below 95%
        let ends = endurance(&d, &d.all_rows(), &EnduranceOpts { trim: Trim::Ends, ..base.clone() }).unwrap();
        assert_eq!(ends.trim.start, 2);
        assert_eq!(ends.trim.between, 0);
        assert!(!ends.sites[1].ok);
        // any: site 2's failing loops between go too, and the broken site 3 removes nothing
        let any = endurance(&d, &d.all_rows(), &EnduranceOpts { trim: Trim::Any, ..base.clone() }).unwrap();
        assert!(any.sites[0].ok && any.sites[1].ok, "{}", any.sites[1].status);
        assert!(!any.sites[2].ok);
        assert_eq!(any.trim.sites, vec!["1", "2"]);
        assert!(any.trim.between > 0);
        assert!(any.sites[1].kept_yield.unwrap() >= 95.0);
        assert!(any.sites[1].status.starts_with("Compared after trimming"));
        // the trimmed trial holds the compared sites' kept loops only, in file order
        let keep = any.kept_rows();
        assert!(keep.windows(2).all(|w| w[0] < w[1]));
        assert!(keep.iter().all(|&r| d.parts[r].site != 3));
        // a few failing loops may stay, as long as each site keeps the yield
        for site in [1u8, 2] {
            let mine: Vec<&usize> = keep.iter().filter(|&&r| d.parts[r].site == site).collect();
            let good = mine.iter().filter(|&&&r| d.parts[r].is_pass()).count();
            assert!(good as f64 * 100.0 / mine.len() as f64 >= 95.0);
        }
        assert_eq!(keep.len(), any.sites[0].kept_rows.len() + any.sites[1].kept_rows.len());
    }

    #[test]
    fn auto_trim_drops_a_site_that_needs_more_than_the_limit() {
        // site 2 fails every 3rd loop (67% yield): rescuing it would cost a third of the loops
        let mut p = vec![];
        for i in 0..90 {
            p.push((1, true, 1.0));
            p.push((2, i % 3 != 1, 1.0));
        }
        let d = ds(&p, test(100, "VOUT", 0.0, 10.0));
        let opts = EnduranceOpts { runs: 10, trim: Trim::Any, ..Default::default() };
        let e = endurance(&d, &d.all_rows(), &opts).unwrap();
        assert!(e.sites[0].ok);
        assert_eq!(e.sites[0].trimmed, 0, "the good site keeps every loop");
        assert!(!e.sites[1].ok);
        assert_eq!(e.trim.sites, vec!["1"]);
        // with no limit the weak site is rescued at the cost of its failing loops
        let all = endurance(&d, &d.all_rows(), &EnduranceOpts { max_trim_pct: 0.0, ..opts }).unwrap();
        assert!(all.sites[1].ok);
        // only as many failing loops as the yield needs: 60 good of 63 kept is 95.2%
        assert_eq!(all.trim.between, 27);
        assert!(all.sites[1].kept_yield.unwrap() >= 95.0);
    }

    #[test]
    fn a_site_the_trim_leaves_out_stays_dropped_and_the_limit_holds_for_one_site() {
        // site 1 fails its first 30 of 100 loops; site 2 passes all
        let mut p = vec![];
        for i in 0..100 {
            p.push((1, i >= 30, 1.0));
            p.push((2, true, 1.0));
        }
        let d = ds(&p, test(100, "VOUT", 0.0, 10.0));
        let opts = EnduranceOpts { runs: 2, trim: Trim::Any, ..Default::default() };
        let e = endurance(&d, &d.all_rows(), &opts).unwrap();
        assert!(!e.sites[0].ok, "{}", e.sites[0].status);
        assert!(e.sites[0].status.contains("trim limit"));
        assert!(e.sites[1].ok);
        assert_eq!(e.sites[1].trimmed, 0);
        assert!(e.kept_rows().iter().all(|&r| d.parts[r].site == 2));
        // alone, site 1 would need 30% of the loops: over the 20% limit it is dropped too
        let one = d.subset(&(0..d.n_parts()).filter(|&r| d.parts[r].site == 1).collect::<Vec<_>>());
        let e = endurance(&one, &one.all_rows(), &opts).unwrap();
        assert!(!e.sites[0].ok, "{}", e.sites[0].status);
        assert!(e.kept_rows().is_empty());
    }

    #[test]
    fn raw_trim_reruns_without_a_site_whose_compared_loops_fail() {
        // runs 2: site 1 fails loops 1 and 3, so its first two kept loops can never all pass
        // without leaving out both; site 2 passes all and should lose nothing
        let mut p = vec![];
        for i in 0..40 {
            p.push((1, i != 1 && i != 3, 1.0));
            p.push((2, true, 1.0));
        }
        let d = ds(&p, test(100, "VOUT", 0.0, 10.0));
        let opts = EnduranceOpts { runs: 2, pick: Pick::Raw, trim: Trim::Ends, yield_pct: 90.0, ..Default::default() };
        let e = endurance(&d, &d.all_rows(), &opts).unwrap();
        assert!(!e.sites[0].ok, "{}", e.sites[0].status);
        assert!(e.sites[1].ok);
        assert_eq!(e.sites[1].trimmed, 0);
    }

    #[test]
    fn a_site_without_values_on_one_side_keeps_its_row() {
        let mut p = loops(1, &[(true, 1.0); 4]);
        p.extend(loops(2, &[(true, 1.0), (true, 1.0), (true, f32::NAN), (true, f32::NAN)]));
        let d = ds(&p, test(100, "VOUT", 0.0, 10.0));
        let e = endurance(&d, &d.all_rows(), &EnduranceOpts { runs: 2, ..Default::default() }).unwrap();
        assert_eq!(e.rows.len(), 2);
        let r = &e.rows[1];
        assert_eq!((r.first.n, r.last.n, r.drift), (2, 0, None));
        assert!(r.comment.contains("No values on one side"));
        let dir = xlsx::scratch_dir().unwrap();
        write_endurance(&e, &d, "x", "", &dir.join("e.xlsx")).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn loop_compare_checks_the_limits_of_the_compared_loops() {
        let p = loops(1, &[(true, 0.5); 2]);
        let mut a = ds(&p, test(100, "VOUT", 0.0, 10.0));
        let mut b = ds(&p, test(100, "VOUT", 0.0, 10.0));
        a.part_limits = vec![Some(PartLimits { lo: vec![0.0; 2], hi: vec![1.0; 2] })];
        b.part_limits = vec![Some(PartLimits { lo: vec![0.0; 2], hi: vec![2.0; 2] })];
        let (sa, sb) = (
            Side { name: "old".into(), ds: &a, rows: a.all_rows() },
            Side { name: "new".into(), ds: &b, rows: b.all_rows() },
        );
        let c = loop_compare(&sa, &sb, &LoopOpts { runs: 2, ..Default::default() }).unwrap();
        assert!(c.pairs[0].limits_match);
        assert!(!c.rows[0].limits_match);
        assert!(c.rows[0].comment.contains("Limits changed"));
    }

    #[test]
    fn per_part_limits_are_the_compared_loops_limits() {
        let mut p = loops(1, &[(true, 0.4); 2]);
        p.extend(loops(1, &[(true, 0.5); 2]));
        let mut d = ds(&p, test(100, "VOUT", 0.0, 10.0));
        d.part_limits = vec![Some(PartLimits { lo: vec![0.0; 4], hi: vec![1.0; 4] })];
        let e = endurance(&d, &d.all_rows(), &EnduranceOpts { runs: 2, ..Default::default() }).unwrap();
        let r = &e.rows[0];
        assert_eq!(r.first.hi, Some(1.0));
        assert!((r.drift.unwrap() - 10.0).abs() < 1e-4, "{:?}", r.drift);
    }

    #[test]
    fn workbooks_are_written() {
        let mut p = loops(1, &[(true, 1.0); 4]);
        p.extend(loops(1, &[(true, 1.9); 4]));
        let d = ds(&p, test(100, "VOUT", 0.0, 10.0));
        let dir = xlsx::scratch_dir().unwrap();
        let e = endurance(&d, &d.all_rows(), &EnduranceOpts { runs: 4, ..Default::default() }).unwrap();
        let out = dir.join("e.xlsx");
        let w = write_endurance(&e, &d, "loop", "", &out).unwrap();
        assert_eq!((w.rows, w.over), (1, 1));
        let s = Side { name: "a".into(), ds: &d, rows: d.all_rows() };
        let c = loop_compare(&s, &s, &LoopOpts { runs: 4, ..Default::default() }).unwrap();
        let out2 = dir.join("c.xlsx");
        write_loop_compare(&c, &s, &s, "", &out2).unwrap();
        let book = std::fs::File::open(&out).unwrap();
        let mut z = zip::ZipArchive::new(book).unwrap();
        let mut x = String::new();
        io::Read::read_to_string(&mut z.by_name("xl/worksheets/sheet1.xml").unwrap(), &mut x).unwrap();
        assert!(x.contains("TP Qualification Drift Analysis"));
        assert!(x.contains(&format!(" s=\"{}\"><v>", style::BAD)));
        assert!(x.contains("<pane xSplit=\"2\""));
        assert!(write_endurance(&e, &d, "loop", "", &dir.join("e.csv")).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
