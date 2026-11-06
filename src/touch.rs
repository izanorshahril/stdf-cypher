//! Touchdown and loop estimate (experiment).
//!
//! STDF V4 does not name the touchdown: a multi-site tester touches several parts
//! at once and writes their records in its own order. Two signals are used:
//!
//! * PIR runs. A tester that opens every site of a touchdown together writes the
//!   PIRs of that touchdown one after another, before any results. A run of PIRs
//!   is one touchdown. Used when the file has such runs (at least half of its
//!   parts in runs of two or more) and every run lists each site once.
//! * Site order and test time. The sites of a touchdown rise, so a site that does
//!   not rise starts a new touchdown. Where a rise could be a boundary, the test
//!   times decide, because the parts of one touchdown finish together. A dynamic
//!   programme picks the segments with the least test-time spread and few
//!   touchdowns.
//!
//! The loop is how often the same part (by part ID or part text) has come up before
//! in the file. A sample tested again on every loop shows up as a repeated ID or
//! text. It is not a physical measurement.
//!
//! Nothing here is validated on real tester files. The dashboard shows it only when
//! the experiment is switched on in Settings.

use crate::data::Dataset;
use crate::stdf::{records, StdfReader};
use anyhow::Result;
use serde::Serialize;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

/// Cost of one more touchdown, in units of the test-time variance: a boundary must
/// remove more spread than this to be placed.
const TOUCH_PENALTY: f64 = 4.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Method {
    /// the file's PIR runs
    Pir,
    /// site order and test time
    Sequence,
}

impl Method {
    pub fn name(self) -> &'static str {
        match self {
            Method::Pir => "PIR runs",
            Method::Sequence => "site order and test time",
        }
    }
}

/// The touchdown of every part, 0-based and in row order.
#[derive(Debug, Clone)]
pub struct Touches {
    pub method: Method,
    pub touch: Vec<u32>,
    /// number of touchdowns
    pub n: u32,
    /// most sites in one touchdown
    pub max_sites: u8,
}

/// How the loop is counted: by part ID, by part text, or not at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum LoopKey {
    Id,
    Text,
    Off,
}

/// One touchdown: its sites, parts and results, and the loops it covers.
#[derive(Debug, Clone, Serialize)]
pub struct TouchSummary {
    pub touch: u32,
    pub sites: Vec<u8>,
    pub parts: u64,
    pub good: u64,
    pub bad: u64,
    pub cut_off: u64,
    /// median test time of the parts, ms (None when no part has one)
    pub test_ms_median: Option<f64>,
    pub loop_min: u32,
    pub loop_max: u32,
}

/// Touchdown of every part of `ds`. `path` is the file the dataset was read from,
/// when it is one file; without it only the site order and test time are used.
pub fn touches(path: Option<&Path>, ds: &Dataset) -> Result<Touches> {
    let max_sites = ds.parts.iter().map(|p| p.site).max().unwrap_or(1).max(1);
    if let Some(p) = path {
        if ds.sources.len() == 1 {
            if let Ok(runs) = pir_runs(p, ds) {
                if let Some(touch) = touch_from_runs(&runs, ds, max_sites) {
                    let n = touch.iter().max().map_or(0, |m| m + 1);
                    return Ok(Touches {
                        method: Method::Pir,
                        touch,
                        n,
                        max_sites,
                    });
                }
            }
        }
    }
    let touch = sequence(ds);
    let n = touch.iter().max().map_or(0, |m| m + 1);
    Ok(Touches {
        method: Method::Sequence,
        touch,
        n,
        max_sites,
    })
}

fn key_of(payload: &[u8]) -> (u8, u8) {
    (
        payload.first().copied().unwrap_or(1),
        payload.get(1).copied().unwrap_or(1),
    )
}

/// The PIR run of every part of the file: one number per PRR in file order, then one
/// per part cut off before its PRR (in the (head, site) order the loader keeps them).
fn pir_runs(path: &Path, ds: &Dataset) -> Result<Vec<u32>> {
    let mut rdr = StdfReader::open(path)?;
    let mut raw = Vec::with_capacity(64);
    let mut run = 0u32;
    let mut prev_pir = false;
    // key -> run of its PIR, until a PRR closes it
    let mut open: HashMap<(u8, u8), u32> = HashMap::new();
    let mut runs = Vec::with_capacity(ds.parts.len());
    while let Some(h) = rdr.next_raw_full(&mut raw)? {
        let code = (h.typ, h.sub);
        let is_pir = code == records::code::PIR;
        if is_pir {
            if !prev_pir {
                run += 1;
            }
            open.insert(key_of(&raw[4..]), run);
        } else if code == records::code::PRR {
            let g = match open.remove(&key_of(&raw[4..])) {
                Some(g) => g,
                None => {
                    run += 1;
                    run
                }
            };
            runs.push(g);
        }
        prev_pir = is_pir;
    }
    let mut cut: Vec<((u8, u8), u32)> = open.into_iter().collect();
    cut.sort_unstable();
    runs.extend(cut.into_iter().map(|(_, g)| g));
    Ok(runs)
}

/// Touchdown numbers from PIR runs, or None when the runs do not look like touchdowns.
fn touch_from_runs(runs: &[u32], ds: &Dataset, max_sites: u8) -> Option<Vec<u32>> {
    if runs.len() != ds.parts.len() || runs.is_empty() {
        return None;
    }
    let mut members: BTreeMap<u32, Vec<usize>> = BTreeMap::new();
    for (row, &g) in runs.iter().enumerate() {
        members.entry(g).or_default().push(row);
    }
    let mut in_runs = 0usize;
    for rows in members.values() {
        if rows.len() >= 2 {
            in_runs += rows.len();
        }
        let mut sites: HashSet<u8> = HashSet::new();
        for &r in rows {
            if !sites.insert(ds.parts[r].site) {
                return None;
            }
        }
        if rows.len() > max_sites as usize {
            return None;
        }
    }
    if in_runs * 2 < runs.len() {
        return None;
    }
    let mut number: HashMap<u32, u32> = HashMap::new();
    let mut touch = Vec::with_capacity(runs.len());
    for &g in runs {
        let next = number.len() as u32;
        touch.push(*number.entry(g).or_insert(next));
    }
    Some(touch)
}

/// Touchdown numbers from site order and test time: a dynamic programme over the parts
/// in file order. A segment must rise in site number (so each site once); its cost is
/// the test-time spread around its mean, in units of the within-touchdown spread, plus
/// `TOUCH_PENALTY` per segment.
fn sequence(ds: &Dataset) -> Vec<u32> {
    let n = ds.parts.len();
    if n == 0 {
        return vec![];
    }
    let site: Vec<u8> = ds.parts.iter().map(|p| p.site).collect();
    let t: Vec<f64> = ds.parts.iter().map(|p| p.test_t as f64).collect();
    let max_len = site.iter().collect::<HashSet<_>>().len().max(1);
    // run[i]: parts of the rising site run that ends at part i
    let mut run = vec![1usize; n];
    for i in 1..n {
        if site[i] > site[i - 1] {
            run[i] = run[i - 1] + 1;
        }
    }
    // within-touchdown spread: the median test-time step between rising neighbours,
    // scaled to a standard deviation. TEST_T is whole milliseconds, so the spread is
    // at least 1 ms: a 1 ms step inside a touchdown is rounding, not a boundary.
    let mut steps: Vec<f64> = (1..n)
        .filter(|&i| site[i] > site[i - 1])
        .map(|i| (t[i] - t[i - 1]).abs())
        .collect();
    let sigma = if steps.is_empty() {
        1.0
    } else {
        steps.sort_by(f64::total_cmp);
        (steps[steps.len() / 2] / 0.954).max(1.0)
    };
    let var = sigma * sigma;
    let mut s1 = vec![0.0f64; n + 1];
    let mut s2 = vec![0.0f64; n + 1];
    for i in 0..n {
        s1[i + 1] = s1[i] + t[i];
        s2[i + 1] = s2[i] + t[i] * t[i];
    }
    let mut best = vec![f64::INFINITY; n + 1];
    let mut from = vec![0usize; n + 1];
    best[0] = 0.0;
    for k in 1..=n {
        for len in 1..=max_len.min(run[k - 1]) {
            let a = k - len;
            let sum = s1[k] - s1[a];
            let sq = s2[k] - s2[a];
            let sse = (sq - sum * sum / len as f64).max(0.0);
            let cost = best[a] + sse / var + TOUCH_PENALTY;
            if cost < best[k] {
                best[k] = cost;
                from[k] = a;
            }
        }
    }
    let mut starts = vec![];
    let mut k = n;
    while k > 0 {
        starts.push(from[k]);
        k = from[k];
    }
    starts.reverse();
    let mut touch = vec![0u32; n];
    for (number, &a) in starts.iter().enumerate() {
        let end = starts.get(number + 1).copied().unwrap_or(n);
        for slot in touch.iter_mut().take(end).skip(a) {
            *slot = number as u32;
        }
    }
    touch
}

/// The loop of every part (0-based): how many times its key (part ID or part text) has
/// come up before. `key` is "auto" (whichever repeats more), "id", "text" or "off".
pub fn loops(ds: &Dataset, key: &str) -> Result<(LoopKey, Vec<u32>)> {
    let ids: Vec<&str> = ds.parts.iter().map(|p| p.part_id.trim()).collect();
    let texts: Vec<&str> = ds.parts.iter().map(|p| p.part_txt.trim()).collect();
    let chosen = match key {
        "id" => LoopKey::Id,
        "text" => LoopKey::Text,
        "off" => LoopKey::Off,
        "" | "auto" => {
            let (a, b) = (repeats(&ids), repeats(&texts));
            if a == 0 && b == 0 {
                LoopKey::Off
            } else if a >= b {
                LoopKey::Id
            } else {
                LoopKey::Text
            }
        }
        other => anyhow::bail!("loop key '{other}': use auto, id, text or off"),
    };
    let vals = match chosen {
        LoopKey::Id => ids,
        LoopKey::Text => texts,
        LoopKey::Off => return Ok((chosen, vec![0; ds.parts.len()])),
    };
    let mut seen: HashMap<&str, u32> = HashMap::new();
    let loops = vals
        .iter()
        .map(|&v| {
            if v.is_empty() {
                return 0;
            }
            let c = seen.entry(v).or_insert(0);
            let out = *c;
            *c += 1;
            out
        })
        .collect();
    Ok((chosen, loops))
}

/// How many non-empty values repeat an earlier one.
fn repeats(vals: &[&str]) -> usize {
    let mut seen: HashSet<&str> = HashSet::new();
    let mut n = 0;
    for &v in vals {
        if !v.is_empty() && !seen.insert(v) {
            n += 1;
        }
    }
    n
}

/// Per-touchdown summary of the estimate and the loops.
pub fn summary(ds: &Dataset, touch: &[u32], loops: &[u32]) -> Vec<TouchSummary> {
    let mut by: BTreeMap<u32, Vec<usize>> = BTreeMap::new();
    for (row, &k) in touch.iter().enumerate() {
        by.entry(k).or_default().push(row);
    }
    by.into_iter()
        .map(|(k, rows)| {
            let mut sites: Vec<u8> = rows.iter().map(|&r| ds.parts[r].site).collect();
            sites.sort_unstable();
            sites.dedup();
            let mut times: Vec<f64> = rows.iter().map(|&r| ds.parts[r].test_t as f64).collect();
            times.sort_by(f64::total_cmp);
            let test_ms_median = (!times.is_empty()).then(|| times[times.len() / 2]);
            let (mut good, mut bad, mut cut_off) = (0u64, 0u64, 0u64);
            for &r in &rows {
                let p = &ds.parts[r];
                if p.partial {
                    cut_off += 1;
                } else if let Some(ok) = p.passed {
                    if ok {
                        good += 1;
                    } else {
                        bad += 1;
                    }
                }
            }
            let lo = rows.iter().map(|&r| loops[r]).min().unwrap_or(0);
            let hi = rows.iter().map(|&r| loops[r]).max().unwrap_or(0);
            TouchSummary {
                touch: k,
                sites,
                parts: rows.len() as u64,
                good,
                bad,
                cut_off,
                test_ms_median,
                loop_min: lo,
                loop_max: hi,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::Part;

    fn part(site: u8, test_t: u32) -> Part {
        Part {
            file: 0,
            part_id: String::new(),
            head: 1,
            site,
            x: 0,
            y: 0,
            hbin: 1,
            sbin: 1,
            passed: Some(true),
            part_flg: 0,
            num_test: 0,
            test_t,
            wafer: u32::MAX,
            part_txt: String::new(),
            first_fail: None,
            alarms: vec![],
            fails: vec![],
            partial: false,
            inst_alarms: 0,
        }
    }

    fn sequence_of(sites: &[u8], times: &[u32]) -> Vec<u32> {
        let ds = Dataset {
            parts: sites.iter().zip(times).map(|(&s, &t)| part(s, t)).collect(),
            ..Default::default()
        };
        sequence(&ds)
    }

    #[test]
    fn a_site_that_does_not_rise_starts_a_touchdown() {
        assert_eq!(
            sequence_of(&[1, 2, 3, 1, 2, 3], &[100; 6]),
            vec![0, 0, 0, 1, 1, 1]
        );
    }

    #[test]
    fn a_rising_boundary_follows_the_test_time_step() {
        assert_eq!(
            sequence_of(&[1, 2, 3, 4, 5, 6], &[100, 100, 100, 140, 140, 140]),
            vec![0, 0, 0, 1, 1, 1]
        );
        assert_eq!(sequence_of(&[1, 2, 3, 4, 5, 6], &[100; 6]), vec![0; 6]);
    }

    #[test]
    fn loops_count_repeated_ids_and_texts() {
        let mut ds = Dataset::default();
        for id in ["1", "2", "1", "2", "1"] {
            let mut p = part(1, 100);
            p.part_id = id.into();
            ds.parts.push(p);
        }
        let (key, l) = loops(&ds, "auto").unwrap();
        assert_eq!(key, LoopKey::Id);
        assert_eq!(l, vec![0, 0, 1, 1, 2]);
        assert_eq!(loops(&ds, "off").unwrap().1, vec![0; 5]);
        assert!(loops(&ds, "nonsense").is_err());
    }
}
