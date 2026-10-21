//! Split (or extract from) an STDF file by part ID, PRR fields or arbitrary
//! filter expressions.
//!
//! Two passes: the file is first loaded as a [`Dataset`] so every PRR can be
//! assigned to an output with the full filter language (including test
//! values); then the raw records are streamed again and each part block
//! (PIR ... PRR) is copied byte-for-byte to its output. Header records are
//! copied to every output; HBR/SBR/PCR/WRR summaries are regenerated so the
//! counts match the parts actually written; TSR records are dropped because
//! they cannot be recomputed exactly.

use crate::data::{Dataset, Filter};
use crate::stdf::*;
use anyhow::{bail, Result};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub enum SplitBy {
    /// one output per distinct value of a part field (part_id, hbin, sbin, site, head, pass, wafer, xy, ...)
    Field(String),
    /// named filter groups (first match wins); `rest` collects unmatched parts
    Groups {
        groups: Vec<(String, String)>,
        rest: Option<String>,
    },
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct SplitOutput {
    pub name: String,
    pub path: String,
    pub parts: u64,
    pub good: u64,
}

#[derive(Default)]
struct Out {
    path: PathBuf,
    buf: Vec<u8>,
    started: bool,
    created: bool,
    wafer_written: Option<usize>,
    parts: u64,
    good: u64,
    hbin: BTreeMap<(u8, u8, u16), u32>,
    sbin: BTreeMap<(u8, u8, u16), u32>,
    pcr: BTreeMap<(u8, u8), (u32, u32, u32)>,
    wafer_cnt: (u32, u32),
    seen_tests: HashSet<(u8, u32)>,
}

impl Out {
    fn flush(&mut self, force: bool) -> Result<()> {
        if self.buf.is_empty() || (!force && self.buf.len() < (4 << 20)) {
            return Ok(());
        }
        let mut f = OpenOptions::new()
            .create(true)
            .write(true)
            .append(self.created)
            .truncate(!self.created)
            .open(&self.path)?;
        f.write_all(&self.buf)?;
        self.created = true;
        self.buf.clear();
        Ok(())
    }
}

/// Does a PTR/MPR payload carry the optional (limits) section?
fn has_opt_section(typ_sub: (u8, u8), p: &[u8], endian: Endian) -> bool {
    let u2 = |o: usize| -> usize {
        if o + 2 > p.len() {
            return 0;
        }
        match endian {
            Endian::Little => u16::from_le_bytes([p[o], p[o + 1]]) as usize,
            Endian::Big => u16::from_be_bytes([p[o], p[o + 1]]) as usize,
        }
    };
    let mut o = if typ_sub == records::code::PTR {
        12
    } else {
        let (j, k) = (u2(8), u2(10));
        12 + j.div_ceil(2) + 4 * k
    };
    for _ in 0..2 {
        o += 1 + p.get(o).copied().unwrap_or(0) as usize;
    }
    o < p.len()
}

fn test_num(p: &[u8], endian: Endian) -> u32 {
    if p.len() < 4 {
        return 0;
    }
    let b = [p[0], p[1], p[2], p[3]];
    match endian {
        Endian::Little => u32::from_le_bytes(b),
        Endian::Big => u32::from_be_bytes(b),
    }
}

fn sanitize(s: &str) -> String {
    let t: String = s
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if t.is_empty() {
        "NA".into()
    } else {
        t
    }
}

/// Assign every part (row) of `ds` to an output name (None = dropped).
pub fn assign(ds: &Dataset, by: &SplitBy) -> Result<Vec<Option<String>>> {
    Ok(match by {
        SplitBy::Field(f) => {
            if !crate::data::PART_FIELDS.contains(&f.as_str())
                && !["partid", "hard_bin", "soft_bin", "wafer_id"].contains(&f.as_str())
            {
                bail!(
                    "unknown split field '{f}'. Use one of: {}",
                    crate::data::PART_FIELDS.join(", ")
                );
            }
            (0..ds.n_parts())
                .map(|r| {
                    Some(format!(
                        "{}{}",
                        field_prefix(f),
                        ds.field(r, f).as_group_key()
                    ))
                })
                .collect()
        }
        SplitBy::Groups { groups, rest } => {
            let filters: Vec<(String, Filter)> = groups
                .iter()
                .map(|(n, e)| Ok((n.clone(), Filter::parse(e, ds)?)))
                .collect::<Result<_>>()?;
            (0..ds.n_parts())
                .map(|r| {
                    filters
                        .iter()
                        .find(|(_, f)| f.matches(ds, r))
                        .map(|(n, _)| n.clone())
                        .or_else(|| rest.clone())
                })
                .collect()
        }
    })
}

fn field_prefix(f: &str) -> &'static str {
    match f {
        "hbin" | "hard_bin" => "hbin",
        "sbin" | "soft_bin" => "sbin",
        "site" => "site",
        "head" => "head",
        "part_id" | "partid" | "id" => "part",
        "wafer" | "wafer_id" => "",
        "pass" => "",
        "xy" => "xy",
        _ => "",
    }
}

/// Split `input` into files in `out_dir`. `ds` must have been loaded from the
/// same file without de-duplication (row k == k-th PRR).
pub fn split_file(
    input: &Path,
    ds: &Dataset,
    by: &SplitBy,
    out_dir: &Path,
    prefix: Option<&str>,
) -> Result<Vec<SplitOutput>> {
    let assignment = assign(ds, by)?;
    split_with_assignment(input, ds, &assignment, out_dir, prefix)
}

pub fn split_with_assignment(
    input: &Path,
    ds: &Dataset,
    assignment: &[Option<String>],
    out_dir: &Path,
    prefix: Option<&str>,
) -> Result<Vec<SplitOutput>> {
    std::fs::create_dir_all(out_dir)?;
    let stem = prefix.map(|s| s.to_string()).unwrap_or_else(|| {
        let n = input
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        n.trim_end_matches(".gz")
            .trim_end_matches(".stdf")
            .trim_end_matches(".std")
            .to_string()
    });
    let mut rdr = StdfReader::open(input)?;
    let endian = rdr.endian;
    let mut outs: HashMap<String, Out> = HashMap::new();
    let mut order: Vec<String> = vec![];
    let mut header: Vec<Vec<u8>> = vec![];
    let mut seen_part = false;
    let mut blocks: HashMap<(u8, u8), Vec<Vec<u8>>> = HashMap::new();
    let mut cur_wir: Option<(usize, Vec<u8>)> = None;
    let mut wafer_idx = 0usize;
    let mut mrr: Option<Vec<u8>> = None;
    let mut prr_k = 0usize;
    let mut raw = Vec::with_capacity(1024);
    let mut defaults: HashMap<(u8, u32), Record> = HashMap::new();

    while let Some(h) = rdr.next_raw_full(&mut raw)? {
        let code = (h.typ, h.sub);
        let payload = &raw[4..];
        match code {
            records::code::HBR | records::code::SBR | records::code::PCR | records::code::TSR => {}
            records::code::MRR => mrr = Some(raw.clone()),
            records::code::WIR => {
                wafer_idx += 1;
                cur_wir = Some((wafer_idx, raw.clone()));
                for o in outs.values_mut() {
                    o.wafer_cnt = (0, 0);
                }
            }
            records::code::WRR => {
                if let Record::WRR(mut wrr) = Record::parse(h.typ, h.sub, payload, endian) {
                    for o in outs.values_mut() {
                        if o.wafer_written == Some(wafer_idx) {
                            wrr.part_cnt = o.wafer_cnt.0;
                            wrr.good_cnt = o.wafer_cnt.1;
                            wrr.rtst_cnt = 0;
                            wrr.abrt_cnt = 0;
                            o.buf
                                .extend_from_slice(&Record::WRR(wrr.clone()).encode(endian));
                        }
                    }
                }
                cur_wir = None;
            }
            records::code::PIR => {
                seen_part = true;
                let hs = (
                    payload.first().copied().unwrap_or(1),
                    payload.get(1).copied().unwrap_or(1),
                );
                blocks.insert(hs, vec![raw.clone()]);
            }
            records::code::PTR | records::code::MPR | records::code::FTR => {
                seen_part = true;
                if code != records::code::FTR {
                    let k = (h.sub, test_num(payload, endian));
                    if !defaults.contains_key(&k) && has_opt_section(code, payload, endian) {
                        defaults.insert(k, Record::parse(h.typ, h.sub, payload, endian));
                    }
                }
                let hs = (
                    payload.get(4).copied().unwrap_or(1),
                    payload.get(5).copied().unwrap_or(1),
                );
                let key = if blocks.contains_key(&hs) || blocks.len() != 1 {
                    hs
                } else {
                    *blocks.keys().next().unwrap()
                };
                blocks.entry(key).or_default().push(raw.clone());
            }
            records::code::PRR => {
                seen_part = true;
                let hs = (
                    payload.first().copied().unwrap_or(1),
                    payload.get(1).copied().unwrap_or(1),
                );
                let mut block = blocks
                    .remove(&hs)
                    .or_else(|| {
                        if blocks.len() == 1 {
                            let k = *blocks.keys().next().unwrap();
                            blocks.remove(&k)
                        } else {
                            None
                        }
                    })
                    .unwrap_or_default();
                block.push(raw.clone());
                let target = assignment.get(prr_k).cloned().flatten();
                prr_k += 1;
                let Some(name) = target else { continue };
                let prr = match Record::parse(h.typ, h.sub, payload, endian) {
                    Record::PRR(p) => p,
                    _ => unreachable!(),
                };
                let o = outs.entry(name.clone()).or_insert_with(|| {
                    order.push(name.clone());
                    Out {
                        path: out_dir.join(format!("{}_{}.stdf", stem, sanitize(&name))),
                        ..Default::default()
                    }
                });
                if !o.started {
                    for r in &header {
                        o.buf.extend_from_slice(r);
                    }
                    o.started = true;
                }
                if let Some((wi, wir)) = &cur_wir {
                    if o.wafer_written != Some(*wi) {
                        o.buf.extend_from_slice(wir);
                        o.wafer_written = Some(*wi);
                        o.wafer_cnt = (0, 0);
                    }
                }
                for r in &block {
                    let (t, sb) = (r[2], r[3]);
                    if t == 15 && (sb == 10 || sb == 15) {
                        let tn = test_num(&r[4..], endian);
                        if o.seen_tests.insert((sb, tn))
                            && !has_opt_section((t, sb), &r[4..], endian)
                        {
                            if let Some(def) = defaults.get(&(sb, tn)) {
                                let fixed = match (Record::parse(t, sb, &r[4..], endian), def) {
                                    (Record::PTR(mut p), Record::PTR(d)) => {
                                        p.opt_flag = d.opt_flag;
                                        p.res_scal = d.res_scal;
                                        p.llm_scal = d.llm_scal;
                                        p.hlm_scal = d.hlm_scal;
                                        p.lo_limit = d.lo_limit;
                                        p.hi_limit = d.hi_limit;
                                        p.units = d.units.clone();
                                        p.c_resfmt = d.c_resfmt.clone();
                                        p.c_llmfmt = d.c_llmfmt.clone();
                                        p.c_hlmfmt = d.c_hlmfmt.clone();
                                        p.lo_spec = d.lo_spec;
                                        p.hi_spec = d.hi_spec;
                                        Some(Record::PTR(p))
                                    }
                                    (Record::MPR(mut p), Record::MPR(d)) => {
                                        p.opt_flag = d.opt_flag;
                                        p.res_scal = d.res_scal;
                                        p.llm_scal = d.llm_scal;
                                        p.hlm_scal = d.hlm_scal;
                                        p.lo_limit = d.lo_limit;
                                        p.hi_limit = d.hi_limit;
                                        p.start_in = d.start_in;
                                        p.incr_in = d.incr_in;
                                        p.rtn_indx = d.rtn_indx.clone();
                                        p.rtn_indx.resize(p.rtn_stat.len(), 0);
                                        p.units = d.units.clone();
                                        p.units_in = d.units_in.clone();
                                        p.c_resfmt = d.c_resfmt.clone();
                                        p.c_llmfmt = d.c_llmfmt.clone();
                                        p.c_hlmfmt = d.c_hlmfmt.clone();
                                        p.lo_spec = d.lo_spec;
                                        p.hi_spec = d.hi_spec;
                                        Some(Record::MPR(p))
                                    }
                                    _ => None,
                                };
                                if let Some(f) = fixed {
                                    o.buf.extend_from_slice(&f.encode(endian));
                                    continue;
                                }
                            }
                        }
                    }
                    o.buf.extend_from_slice(r);
                }
                let good = prr.passed().unwrap_or(false);
                o.parts += 1;
                o.wafer_cnt.0 += 1;
                if good {
                    o.good += 1;
                    o.wafer_cnt.1 += 1;
                }
                *o.hbin
                    .entry((prr.head_num, prr.site_num, prr.hard_bin))
                    .or_default() += 1;
                *o.sbin
                    .entry((prr.head_num, prr.site_num, prr.soft_bin))
                    .or_default() += 1;
                let e = o.pcr.entry((prr.head_num, prr.site_num)).or_default();
                e.0 += 1;
                if good {
                    e.1 += 1;
                }
                if prr.is_retest() {
                    e.2 += 1;
                }
                o.flush(false)?;
            }
            _ => {
                if !seen_part {
                    header.push(raw.clone());
                } else if !blocks.is_empty() {
                    for b in blocks.values_mut() {
                        b.push(raw.clone());
                    }
                } else {
                    for o in outs.values_mut() {
                        o.buf.extend_from_slice(&raw);
                    }
                }
            }
        }
    }

    let mut result = vec![];
    for name in order {
        let o = outs.get_mut(&name).unwrap();
        let mut recs = vec![];
        let mut htot: BTreeMap<u16, u32> = BTreeMap::new();
        for (&(hd, st, b), &c) in &o.hbin {
            *htot.entry(b).or_default() += c;
            let d = ds.hbins.get(&b).cloned().unwrap_or_default();
            recs.push(Record::HBR(Bin {
                head_num: hd,
                site_num: st,
                bin_num: b,
                bin_cnt: c,
                bin_pf: d.pf,
                bin_nam: d.name,
            }));
        }
        for (b, c) in htot {
            let d = ds.hbins.get(&b).cloned().unwrap_or_default();
            recs.push(Record::HBR(Bin {
                head_num: 255,
                site_num: 255,
                bin_num: b,
                bin_cnt: c,
                bin_pf: d.pf,
                bin_nam: d.name,
            }));
        }
        let mut stot: BTreeMap<u16, u32> = BTreeMap::new();
        for (&(hd, st, b), &c) in &o.sbin {
            *stot.entry(b).or_default() += c;
            let d = ds.sbins.get(&b).cloned().unwrap_or_default();
            recs.push(Record::SBR(Bin {
                head_num: hd,
                site_num: st,
                bin_num: b,
                bin_cnt: c,
                bin_pf: d.pf,
                bin_nam: d.name,
            }));
        }
        for (b, c) in stot {
            let d = ds.sbins.get(&b).cloned().unwrap_or_default();
            recs.push(Record::SBR(Bin {
                head_num: 255,
                site_num: 255,
                bin_num: b,
                bin_cnt: c,
                bin_pf: d.pf,
                bin_nam: d.name,
            }));
        }
        let (mut tc, mut tg, mut tr) = (0, 0, 0);
        for (&(hd, st), &(c, g, r)) in &o.pcr {
            tc += c;
            tg += g;
            tr += r;
            recs.push(Record::PCR(Pcr {
                head_num: hd,
                site_num: st,
                part_cnt: c,
                rtst_cnt: r,
                abrt_cnt: 0,
                good_cnt: g,
                func_cnt: 0,
            }));
        }
        recs.push(Record::PCR(Pcr {
            head_num: 255,
            site_num: 255,
            part_cnt: tc,
            rtst_cnt: tr,
            abrt_cnt: 0,
            good_cnt: tg,
            func_cnt: 0,
        }));
        for r in recs {
            o.buf.extend_from_slice(&r.encode(endian));
        }
        if let Some(m) = &mrr {
            o.buf.extend_from_slice(m);
        }
        o.flush(true)?;
        result.push(SplitOutput {
            name: name.clone(),
            path: o.path.to_string_lossy().to_string(),
            parts: o.parts,
            good: o.good,
        });
    }
    if prr_k != assignment.len() {
        eprintln!(
            "warning: dataset has {} parts but file has {} PRRs – was the dataset de-duplicated?",
            assignment.len(),
            prr_k
        );
    }
    Ok(result)
}
