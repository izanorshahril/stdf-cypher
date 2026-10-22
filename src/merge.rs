//! Merge several STDF files of one lot and test step into one file, e.g. a
//! lot whose wafers were tested on two testers or in two sessions.
//!
//! The first file's header (FAR, MIR and everything before its first wafer
//! or part) opens the output; the wafers and parts of every file follow in
//! order, copied byte-for-byte; HBR/SBR/PCR summaries are regenerated from
//! the PRRs of all files; TSR records are dropped (they cannot be combined
//! exactly) and the last file's MRR closes it. Files must share a byte order.
//! With `renumber`, each later file's PART_IDs continue from the highest one
//! written so far (its own IDs added on top, or counted up when not numeric).

use crate::stdf::*;
use anyhow::{bail, Context, Result};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;

#[derive(Debug, Clone, serde::Serialize)]
pub struct MergeOutput {
    pub path: String,
    pub files: usize,
    pub parts: u64,
    pub good: u64,
}

pub fn merge_files(inputs: &[impl AsRef<Path>], out: &Path, renumber: bool) -> Result<MergeOutput> {
    if inputs.len() < 2 {
        bail!("pick at least two files to merge");
    }
    let mut w = std::io::BufWriter::new(
        std::fs::File::create(out).with_context(|| format!("create {}", out.display()))?,
    );
    let mut endian = None;
    let mut mrr: Option<Vec<u8>> = None;
    let (mut parts, mut good) = (0u64, 0u64);
    let mut hbin: BTreeMap<(u8, u8, u16), u32> = BTreeMap::new();
    let mut sbin: BTreeMap<(u8, u8, u16), u32> = BTreeMap::new();
    let mut pcr: BTreeMap<(u8, u8), (u32, u32, u32)> = BTreeMap::new();
    // bin names and pass/fail from the files' own summaries (first seen wins)
    let mut hdef: BTreeMap<u16, (char, String)> = BTreeMap::new();
    let mut sdef: BTreeMap<u16, (char, String)> = BTreeMap::new();
    let mut raw = Vec::with_capacity(1024);
    // part IDs: highest written so far, and this file's offset
    let mut top = 0u64;
    for (fi, input) in inputs.iter().enumerate() {
        let offset = if fi == 0 { 0 } else { top };
        let mut seq = 0u64;
        let input = input.as_ref();
        let mut rdr = StdfReader::open(input)?;
        let e = *endian.get_or_insert(rdr.endian);
        if rdr.endian != e {
            bail!(
                "{} uses another byte order than the first file: merge files from the same tester type",
                input.display()
            );
        }
        let mut in_body = false;
        while let Some(h) = rdr.next_raw_full(&mut raw)? {
            let code = (h.typ, h.sub);
            let payload = &raw[4..];
            match code {
                records::code::HBR | records::code::SBR => {
                    if let Record::HBR(b) | Record::SBR(b) = Record::parse(h.typ, h.sub, payload, e)
                    {
                        let defs = if code == records::code::HBR {
                            &mut hdef
                        } else {
                            &mut sdef
                        };
                        defs.entry(b.bin_num).or_insert((b.bin_pf, b.bin_nam));
                    }
                }
                records::code::PCR | records::code::TSR => {}
                records::code::MRR => mrr = Some(raw.clone()),
                records::code::WIR | records::code::PIR => {
                    in_body = true;
                    w.write_all(&raw)?;
                }
                records::code::PTR | records::code::MPR | records::code::FTR => {
                    in_body = true;
                    w.write_all(&raw)?;
                }
                records::code::PRR => {
                    in_body = true;
                    let rec = Record::parse(h.typ, h.sub, payload, e);
                    seq += 1;
                    match &rec {
                        Record::PRR(p) if renumber => {
                            let id = p.part_id.trim().parse::<u64>().unwrap_or(seq) + offset;
                            top = top.max(id);
                            let mut p = p.clone();
                            p.part_id = id.to_string();
                            w.write_all(&Record::PRR(p).encode(e))?;
                        }
                        Record::PRR(p) => {
                            top = top.max(p.part_id.trim().parse().unwrap_or(0));
                            w.write_all(&raw)?;
                        }
                        _ => w.write_all(&raw)?,
                    }
                    if let Record::PRR(p) = rec {
                        let ok = p.passed().unwrap_or(false);
                        parts += 1;
                        good += ok as u64;
                        *hbin
                            .entry((p.head_num, p.site_num, p.hard_bin))
                            .or_default() += 1;
                        *sbin
                            .entry((p.head_num, p.site_num, p.soft_bin))
                            .or_default() += 1;
                        let c = pcr.entry((p.head_num, p.site_num)).or_default();
                        c.0 += 1;
                        c.1 += ok as u32;
                        c.2 += p.is_retest() as u32;
                    }
                }
                // the first file's header opens the output; later headers
                // repeat it (FAR, MIR, SDR, PMR…) and are left out
                _ if !in_body => {
                    if fi == 0 {
                        w.write_all(&raw)?;
                    }
                }
                _ => w.write_all(&raw)?,
            }
        }
    }
    let e = endian.unwrap_or(Endian::Little);
    let def = |m: &BTreeMap<u16, (char, String)>, b: u16| {
        m.get(&b).cloned().unwrap_or((' ', String::new()))
    };
    let mut recs = vec![];
    for (bins, defs, sub) in [(&hbin, &hdef, true), (&sbin, &sdef, false)] {
        let mut tot: BTreeMap<u16, u32> = BTreeMap::new();
        let mk = |head_num, site_num, bin_num, bin_cnt| {
            let (bin_pf, bin_nam) = def(defs, bin_num);
            let b = Bin {
                head_num,
                site_num,
                bin_num,
                bin_cnt,
                bin_pf,
                bin_nam,
            };
            if sub {
                Record::HBR(b)
            } else {
                Record::SBR(b)
            }
        };
        for (&(hd, st, b), &c) in bins {
            *tot.entry(b).or_default() += c;
            recs.push(mk(hd, st, b, c));
        }
        for (b, c) in tot {
            recs.push(mk(255, 255, b, c));
        }
    }
    let (mut tc, mut tg, mut tr) = (0, 0, 0);
    for (&(head_num, site_num), &(c, g, r)) in &pcr {
        (tc, tg, tr) = (tc + c, tg + g, tr + r);
        recs.push(Record::PCR(Pcr {
            head_num,
            site_num,
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
        w.write_all(&r.encode(e))?;
    }
    if let Some(m) = mrr {
        w.write_all(&m)?;
    }
    w.flush()?;
    Ok(MergeOutput {
        path: out.to_string_lossy().to_string(),
        files: inputs.len(),
        parts,
        good,
    })
}
