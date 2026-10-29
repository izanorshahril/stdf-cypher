//! Dataset summaries: per-test statistics, bins, sites, wafers, fail pareto.

use super::stats::{self, Describe};
use crate::data::{short_name, Dataset};
use indexmap::IndexMap;
use rayon::prelude::*;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct TestSummary {
    pub idx: usize,
    pub key: String,
    pub num: u32,
    pub name: String,
    pub kind: String,
    pub units: String,
    pub lo: Option<f64>,
    pub hi: Option<f64>,
    pub stats: Describe,
    pub fail_pct: f64,
}

pub fn tests(ds: &Dataset, rows: &[usize]) -> Vec<TestSummary> {
    (0..ds.n_tests())
        .into_par_iter()
        .map(|t| {
            let info = &ds.tests[t];
            let v = ds.values_of(t, Some(rows));
            let d = stats::describe(&v, info.lo, info.hi);
            let fp = if d.n > 0 {
                d.fails as f64 * 100.0 / d.n as f64
            } else {
                0.0
            };
            TestSummary {
                idx: t,
                key: info.key.clone(),
                num: info.num,
                name: info.name.clone(),
                kind: format!("{:?}", info.kind),
                units: info.units.clone(),
                lo: info.lo,
                hi: info.hi,
                stats: d,
                fail_pct: fp,
            }
        })
        .collect()
}

#[derive(Debug, Clone, Serialize)]
pub struct BinCount {
    pub bin: u16,
    pub name: String,
    pub pf: String,
    pub count: usize,
    pub pct: f64,
}

pub fn bins(ds: &Dataset, rows: &[usize], hard: bool) -> Vec<BinCount> {
    let mut m: IndexMap<u16, usize> = IndexMap::new();
    for &r in rows {
        let p = &ds.parts[r];
        *m.entry(if hard { p.hbin } else { p.sbin }).or_default() += 1;
    }
    let n = rows.len().max(1) as f64;
    let mut v: Vec<BinCount> = m
        .into_iter()
        .map(|(b, c)| {
            let def = if hard {
                ds.hbins.get(&b)
            } else {
                ds.sbins.get(&b)
            };
            BinCount {
                bin: b,
                name: def.map(|d| d.name.clone()).unwrap_or_default(),
                pf: def.map(|d| d.pf.to_string()).unwrap_or_default(),
                count: c,
                pct: c as f64 * 100.0 / n,
            }
        })
        .collect();
    v.sort_by(|a, b| b.count.cmp(&a.count));
    v
}

#[derive(Debug, Clone, Serialize)]
pub struct GroupYield {
    pub group: String,
    pub n: usize,
    pub pass: usize,
    pub yield_pct: f64,
}

pub fn yield_by(ds: &Dataset, rows: &[usize], field: &str) -> Vec<GroupYield> {
    ds.group_by(rows, field)
        .into_iter()
        .map(|(g, r)| {
            let pass = r.iter().filter(|&&i| ds.parts[i].is_pass()).count();
            GroupYield {
                group: g,
                n: r.len(),
                pass,
                yield_pct: if r.is_empty() {
                    0.0
                } else {
                    pass as f64 * 100.0 / r.len() as f64
                },
            }
        })
        .collect()
}

#[derive(Debug, Clone, Serialize)]
pub struct FailPareto {
    pub key: String,
    /// parts whose first failing test is this one
    pub first_fail: usize,
    /// parts failing this test at all (value outside limits)
    pub any_fail: usize,
    pub pct_of_fails: f64,
}

pub fn fail_pareto(ds: &Dataset, rows: &[usize]) -> Vec<FailPareto> {
    let mut first: IndexMap<usize, usize> = IndexMap::new();
    let n_fail = rows
        .iter()
        .filter(|&&r| !ds.parts[r].is_pass())
        .count()
        .max(1);
    for &r in rows {
        if let Some(f) = ds.parts[r].first_fail {
            *first.entry(f as usize).or_default() += 1;
        }
    }
    let mut v: Vec<FailPareto> = (0..ds.n_tests())
        .into_par_iter()
        .filter_map(|t| {
            let info = &ds.tests[t];
            let col = ds.column(t);
            let any = rows
                .iter()
                .filter(|&&r| {
                    let v = col[r] as f64;
                    v.is_finite() && !info.within_limits(v)
                })
                .count();
            let ff = first.get(&t).copied().unwrap_or(0);
            (any > 0 || ff > 0).then(|| FailPareto {
                key: info.key.clone(),
                first_fail: ff,
                any_fail: any,
                pct_of_fails: ff as f64 * 100.0 / n_fail as f64,
            })
        })
        .collect();
    v.sort_by(|a, b| {
        b.first_fail
            .cmp(&a.first_fail)
            .then(b.any_fail.cmp(&a.any_fail))
    });
    v
}

#[derive(Debug, Clone, Serialize)]
pub struct Overview {
    pub name: String,
    pub files: Vec<String>,
    pub lots: Vec<String>,
    pub part_typ: String,
    pub job: String,
    pub tester: String,
    pub n_parts: usize,
    pub n_pass: usize,
    /// parts with no PRR record (the file was cut off or is incomplete)
    pub partial_parts: u32,
    /// yield without those parts, in percent (None when there are none)
    pub yield_without_partial_pct: Option<f64>,
    pub yield_pct: f64,
    pub n_tests: usize,
    pub n_wafers: usize,
    pub sites: usize,
    pub retests: usize,
    pub record_counts: IndexMap<String, u64>,
}

/// Yield in percent over the parts that are not cut off (None when no part is).
pub fn yield_without_partial(ds: &Dataset) -> Option<f64> {
    let cut = ds.parts.iter().filter(|p| p.partial).count();
    if cut == 0 {
        return None;
    }
    let kept = ds.parts.len() - cut;
    let pass = ds.parts.iter().filter(|p| p.is_pass()).count();
    Some(if kept == 0 {
        0.0
    } else {
        pass as f64 * 100.0 / kept as f64
    })
}

/// The warning for parts cut off before their PRR, with the yield both ways (counting them,
/// and leaving them out). None when no part is cut off.
pub fn cut_off_warning(ds: &Dataset) -> Option<String> {
    let cut = ds.parts.iter().filter(|p| p.partial).count();
    if cut == 0 {
        return None;
    }
    let all = ds.parts.len();
    let pass = ds.parts.iter().filter(|p| p.is_pass()).count();
    let with = pass as f64 * 100.0 / all as f64;
    let without = yield_without_partial(ds).unwrap_or(0.0);
    Some(format!(
        "warning: {cut} part(s) cut off before their PRR (file ends inside a part or is incomplete): \
         kept with their results, no bin, no pass/fail. Yield {with:.2}% counting them ({pass}/{all}), \
         {without:.2}% without them ({pass}/{})",
        all - cut
    ))
}

pub fn overview(ds: &Dataset) -> Overview {
    let rows = ds.all_rows();
    let n_pass = ds.parts.iter().filter(|p| p.is_pass()).count();
    let mut rc: IndexMap<String, u64> = IndexMap::new();
    for s in &ds.sources {
        for (k, v) in &s.record_counts {
            *rc.entry(k.clone()).or_default() += v;
        }
    }
    let mut lots: Vec<String> = ds.sources.iter().map(|s| s.lot_id.clone()).collect();
    lots.dedup();
    let src0 = ds.sources.first().cloned().unwrap_or_default();
    Overview {
        partial_parts: ds.parts.iter().filter(|p| p.partial).count() as u32,
        yield_without_partial_pct: yield_without_partial(ds),
        name: ds.name.clone(),
        files: ds.sources.iter().map(|s| short_name(&s.path)).collect(),
        lots,
        part_typ: src0.part_typ,
        job: format!("{} {}", src0.job_nam, src0.job_rev)
            .trim()
            .to_string(),
        tester: format!("{} {}", src0.tstr_typ, src0.node_nam)
            .trim()
            .to_string(),
        n_parts: ds.n_parts(),
        n_pass,
        yield_pct: ds.yield_pct(&rows),
        n_tests: ds.n_tests(),
        n_wafers: ds.wafers.len(),
        sites: ds.group_by(&rows, "site").len(),
        retests: ds.parts.iter().filter(|p| p.part_flg & 0x03 != 0).count(),
        record_counts: rc,
    }
}
