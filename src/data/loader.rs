//! Build a [`Dataset`] from STDF files.

use super::{BinDef, Dataset, Part, SourceInfo, TestInfo, TestKind};
use crate::stdf::{Record, StdfReader};
use anyhow::Result;
use indexmap::IndexMap;
use rayon::prelude::*;
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Clone)]
pub struct LoadOptions {
    /// Store FTR results as 1 (pass) / 0 (fail) columns.
    pub functional: bool,
    /// Expand MPR results into one column per pin.
    pub multi_pin: bool,
    /// Keep only the first / last test of each physical unit.
    pub dedup: Option<DedupMode>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DedupMode {
    First,
    Last,
}

impl Default for LoadOptions {
    fn default() -> Self {
        Self {
            functional: true,
            multi_pin: true,
            dedup: None,
        }
    }
}

#[derive(Default)]
struct Pending {
    results: Vec<(u32, f32)>,
    first_fail: Option<u32>,
    alarms: Vec<u32>,
}

struct Builder<'o> {
    opt: &'o LoadOptions,
    ds: Dataset,
    src: SourceInfo,
    /// test_num -> [(name, test idx)]
    by_num: HashMap<u32, Vec<(String, usize)>>,
    mpr_by_key: HashMap<(usize, u16), usize>,
    last_name: HashMap<u32, String>,
    mpr_default_indx: HashMap<u32, Vec<u16>>,
    limits_set: Vec<bool>,
    pending: HashMap<(u8, u8), Pending>,
    pin_names: HashMap<u16, String>,
    wafer_ids: IndexMap<String, u32>,
    cur_wafer: HashMap<u8, u32>,
}

impl<'o> Builder<'o> {
    fn new(opt: &'o LoadOptions, path: &str) -> Self {
        Self {
            opt,
            ds: Dataset {
                name: super::short_name(path),
                ..Default::default()
            },
            src: SourceInfo {
                path: path.to_string(),
                ..Default::default()
            },
            by_num: HashMap::new(),
            mpr_by_key: HashMap::new(),
            last_name: HashMap::new(),
            mpr_default_indx: HashMap::new(),
            limits_set: vec![],
            pending: HashMap::new(),
            pin_names: HashMap::new(),
            wafer_ids: IndexMap::new(),
            cur_wafer: HashMap::new(),
        }
    }

    fn new_test(&mut self, key: String, num: u32, name: String, kind: TestKind) -> usize {
        let i = self.ds.tests.len();
        self.ds.tests.push(TestInfo {
            key: key.clone(),
            num,
            name,
            kind,
            units: String::new(),
            lo: None,
            hi: None,
            lo_spec: None,
            hi_spec: None,
            res_scal: 0,
            exec_count: 0,
            fail_count: 0,
        });
        self.ds.test_index.insert(key, i);
        self.ds.values.push(Vec::new());
        self.limits_set.push(false);
        i
    }

    /// Resolve test index for (num, txt) creating it if needed.
    fn test_for(&mut self, num: u32, txt: &str, kind: TestKind) -> usize {
        let name: &str = if txt.is_empty() {
            self.last_name.get(&num).map(|s| s.as_str()).unwrap_or("")
        } else {
            txt
        };
        if let Some(list) = self.by_num.get(&num) {
            if let Some((_, i)) = list.iter().find(|(n, _)| n == name) {
                return *i;
            }
        }
        let name = name.to_string();
        let mut key = format!("{num}:{name}");
        if self.ds.test_index.contains_key(&key) {
            key = format!("{num}:{name}#{}", self.ds.tests.len());
        }
        let i = self.new_test(key, num, name.clone(), kind);
        if !txt.is_empty() {
            self.last_name.insert(num, txt.to_string());
        }
        self.by_num.entry(num).or_default().push((name, i));
        i
    }

    fn pending_for(&mut self, head: u8, site: u8) -> &mut Pending {
        if !self.pending.contains_key(&(head, site)) && self.pending.len() == 1 {
            let k = *self.pending.keys().next().unwrap();
            return self.pending.get_mut(&k).unwrap();
        }
        self.pending.entry((head, site)).or_default()
    }

    fn set_limits(
        &mut self,
        t: usize,
        opt: Option<u8>,
        lo: Option<f32>,
        hi: Option<f32>,
        lo_spec: Option<f32>,
        hi_spec: Option<f32>,
        units: Option<&String>,
        res_scal: i8,
    ) {
        if self.limits_set[t] {
            return;
        }
        let Some(opt) = opt else { return };
        let ti = &mut self.ds.tests[t];
        if opt & 0x40 == 0 && opt & 0x10 == 0 {
            ti.lo = lo.map(|v| v as f64).filter(|v| v.is_finite());
        }
        if opt & 0x80 == 0 && opt & 0x20 == 0 {
            ti.hi = hi.map(|v| v as f64).filter(|v| v.is_finite());
        }
        if opt & 0x04 == 0 {
            ti.lo_spec = lo_spec.map(|v| v as f64).filter(|v| v.is_finite());
        }
        if opt & 0x08 == 0 {
            ti.hi_spec = hi_spec.map(|v| v as f64).filter(|v| v.is_finite());
        }
        if let Some(u) = units {
            ti.units = u.trim().to_string();
        }
        if opt & 0x01 == 0 {
            ti.res_scal = res_scal;
        }
        self.limits_set[t] = true;
    }

    fn record_result(
        &mut self,
        head: u8,
        site: u8,
        t: usize,
        value: f32,
        failed: bool,
        alarm: bool,
    ) {
        self.ds.tests[t].exec_count += 1;
        if failed {
            self.ds.tests[t].fail_count += 1;
        }
        let p = self.pending_for(head, site);
        p.results.push((t as u32, value));
        if alarm {
            p.alarms.push(t as u32);
        }
        if failed && p.first_fail.is_none() {
            p.first_fail = Some(t as u32);
        }
    }

    fn handle(&mut self, rec: Record) {
        match rec {
            Record::MIR(m) => {
                self.src.lot_id = m.lot_id.clone();
                self.src.sublot_id = m.sblot_id.clone();
                self.src.part_typ = m.part_typ.clone();
                self.src.job_nam = m.job_nam.clone();
                self.src.job_rev = m.job_rev.clone();
                self.src.node_nam = m.node_nam.clone();
                self.src.tstr_typ = m.tstr_typ.clone();
                self.src.test_cod = m.test_cod.clone();
                self.src.tst_temp = m.tst_temp.clone();
                self.src.flow_id = m.flow_id.clone();
                self.src.start_t = m.start_t;
            }
            Record::MRR(m) => self.src.finish_t = m.finish_t,
            Record::HBR(b) => {
                let e = self.ds.hbins.entry(b.bin_num).or_default();
                if e.name.is_empty() {
                    *e = BinDef {
                        name: b.bin_nam,
                        pf: b.bin_pf,
                    };
                }
            }
            Record::SBR(b) => {
                let e = self.ds.sbins.entry(b.bin_num).or_default();
                if e.name.is_empty() {
                    *e = BinDef {
                        name: b.bin_nam,
                        pf: b.bin_pf,
                    };
                }
            }
            Record::PMR(p) => {
                let n = if !p.log_nam.is_empty() {
                    p.log_nam
                } else if !p.chan_nam.is_empty() {
                    p.chan_nam
                } else {
                    p.phy_nam
                };
                self.pin_names.insert(p.pmr_indx, n);
            }
            Record::WIR(w) => {
                let n = self.wafer_ids.len() as u32;
                let idx = *self.wafer_ids.entry(w.wafer_id.clone()).or_insert(n);
                self.cur_wafer.insert(w.head_num, idx);
            }
            Record::PIR(p) => {
                self.pending
                    .insert((p.head_num, p.site_num), Pending::default());
            }
            Record::PTR(p) => {
                let t = self.test_for(p.test_num, &p.test_txt, TestKind::Parametric);
                self.set_limits(
                    t,
                    p.opt_flag,
                    p.lo_limit,
                    p.hi_limit,
                    p.lo_spec,
                    p.hi_spec,
                    p.units.as_ref(),
                    p.res_scal,
                );
                let v = if p.result_valid() && p.result.is_finite() {
                    p.result
                } else {
                    f32::NAN
                };
                self.record_result(
                    p.head_num,
                    p.site_num,
                    t,
                    v,
                    p.passed() == Some(false),
                    p.test_flg & 0x01 != 0,
                );
            }
            Record::MPR(m) => {
                if !self.opt.multi_pin {
                    return;
                }
                let base = self.test_for(m.test_num, &m.test_txt, TestKind::MultiPin);
                let indx = if !m.rtn_indx.is_empty() {
                    self.mpr_default_indx
                        .entry(m.test_num)
                        .or_insert_with(|| m.rtn_indx.clone());
                    m.rtn_indx.clone()
                } else {
                    self.mpr_default_indx
                        .get(&m.test_num)
                        .cloned()
                        .unwrap_or_default()
                };
                let failed = m.test_flg & 0x80 != 0 && m.test_flg & 0x40 == 0;
                let valid = m.test_flg & 0b0011_1110 == 0;
                let base_name = self.ds.tests[base].name.clone();
                for (k, &r) in m.rtn_rslt.iter().enumerate() {
                    let pin = indx.get(k).copied().unwrap_or(k as u16);
                    let t = if let Some(&t) = self.mpr_by_key.get(&(base, pin)) {
                        t
                    } else {
                        let pname = self
                            .pin_names
                            .get(&pin)
                            .cloned()
                            .unwrap_or_else(|| pin.to_string());
                        let key = format!("{}:{}[{}]", m.test_num, base_name, pname);
                        let t = if let Some(&t) = self.ds.test_index.get(&key) {
                            t
                        } else {
                            self.new_test(
                                key,
                                m.test_num,
                                format!("{base_name}[{pname}]"),
                                TestKind::MultiPin,
                            )
                        };
                        self.mpr_by_key.insert((base, pin), t);
                        t
                    };
                    self.set_limits(
                        t,
                        m.opt_flag,
                        m.lo_limit,
                        m.hi_limit,
                        m.lo_spec,
                        m.hi_spec,
                        m.units.as_ref(),
                        m.res_scal,
                    );
                    let v = if valid && r.is_finite() { r } else { f32::NAN };
                    let ti = &self.ds.tests[t];
                    let pin_fail = if ti.lo.is_some() || ti.hi.is_some() {
                        v.is_finite() && !ti.within_limits(v as f64)
                    } else {
                        failed
                    };
                    self.record_result(
                        m.head_num,
                        m.site_num,
                        t,
                        v,
                        pin_fail,
                        m.test_flg & 0x01 != 0,
                    );
                }
                // the base entry itself carries no values; it is removed at finish if empty
            }
            Record::FTR(f) => {
                if !self.opt.functional {
                    return;
                }
                let t = self.test_for(f.test_num, &f.test_txt, TestKind::Functional);
                let pf = f.passed();
                let v = match pf {
                    Some(true) => 1.0,
                    Some(false) => 0.0,
                    None => f32::NAN,
                };
                if !self.limits_set[t] {
                    self.ds.tests[t].lo = Some(0.5);
                    self.limits_set[t] = true;
                }
                self.record_result(
                    f.head_num,
                    f.site_num,
                    t,
                    v,
                    pf == Some(false),
                    f.test_flg & 0x01 != 0,
                );
            }
            Record::TSR(t) => {
                if let Some(list) = self.by_num.get(&t.test_num) {
                    for &(_, i) in list {
                        if self.ds.tests[i].name.is_empty() && !t.test_nam.is_empty() {
                            self.ds.tests[i].name = t.test_nam.clone();
                        }
                    }
                }
            }
            Record::PRR(p) => {
                let pend = self.pending.remove(&(p.head_num, p.site_num)).or_else(|| {
                    if self.pending.len() == 1 {
                        let k = *self.pending.keys().next().unwrap();
                        self.pending.remove(&k)
                    } else {
                        None
                    }
                });
                let pend = pend.unwrap_or_default();
                let row = self.ds.parts.len();
                for (t, v) in pend.results {
                    let col = &mut self.ds.values[t as usize];
                    if col.len() <= row {
                        col.resize(row + 1, f32::NAN);
                    }
                    col[row] = v;
                }
                let passed = p.passed();
                self.ds.parts.push(Part {
                    file: 0,
                    part_id: p.part_id,
                    head: p.head_num,
                    site: p.site_num,
                    x: p.x_coord,
                    y: p.y_coord,
                    hbin: p.hard_bin,
                    sbin: p.soft_bin,
                    passed,
                    part_flg: p.part_flg,
                    num_test: p.num_test,
                    test_t: p.test_t,
                    wafer: self
                        .cur_wafer
                        .get(&p.head_num)
                        .copied()
                        .or_else(|| self.cur_wafer.values().next().copied())
                        .unwrap_or(u32::MAX),
                    part_txt: p.part_txt,
                    first_fail: pend.first_fail,
                    alarms: pend.alarms,
                });
            }
            _ => {}
        }
    }

    /// Allocation-free handling of a PTR whose test is already known and whose
    /// limits are set. Returns false when the full parser must be used.
    fn fast_ptr(&mut self, d: &[u8], e: crate::stdf::Endian) -> bool {
        if d.len() < 13 {
            return false;
        }
        let u4 = |b: &[u8]| match e {
            crate::stdf::Endian::Little => u32::from_le_bytes([b[0], b[1], b[2], b[3]]),
            crate::stdf::Endian::Big => u32::from_be_bytes([b[0], b[1], b[2], b[3]]),
        };
        let num = u4(&d[0..4]);
        let (head, site, test_flg) = (d[4], d[5], d[6]);
        let result = f32::from_bits(u4(&d[8..12]));
        let n = d[12] as usize;
        let Some(txt) = d.get(13..13 + n) else {
            return false;
        };
        let Some(list) = self.by_num.get(&num) else {
            return false;
        };
        let t = if n == 0 {
            match self.last_name.get(&num) {
                Some(last) => match list.iter().find(|(nm, _)| nm == last) {
                    Some(&(_, t)) => t,
                    None => return false,
                },
                None => match list.iter().find(|(nm, _)| nm.is_empty()) {
                    Some(&(_, t)) => t,
                    None => return false,
                },
            }
        } else {
            match list.iter().find(|(nm, _)| nm.as_bytes() == txt) {
                Some(&(_, t)) => t,
                None => return false,
            }
        };
        if !self.limits_set[t] {
            return false;
        }
        let valid = test_flg & 0b0011_1110 == 0;
        let v = if valid && result.is_finite() {
            result
        } else {
            f32::NAN
        };
        let failed = test_flg & 0x40 == 0 && test_flg & 0x80 != 0;
        self.record_result(head, site, t, v, failed, test_flg & 0x01 != 0);
        true
    }

    fn finish(mut self) -> Dataset {
        let n = self.ds.parts.len();
        for c in self.ds.values.iter_mut() {
            c.resize(n, f32::NAN);
        }
        // drop MPR base placeholders (no values)
        let keep: Vec<bool> = self
            .ds
            .values
            .iter()
            .map(|c| c.iter().any(|v| !v.is_nan()))
            .collect();
        if keep.iter().any(|k| !k) {
            let mut remap = vec![u32::MAX; keep.len()];
            let mut tests = vec![];
            let mut values = vec![];
            for (i, (t, c)) in self
                .ds
                .tests
                .drain(..)
                .zip(self.ds.values.drain(..))
                .enumerate()
            {
                if keep[i] {
                    remap[i] = tests.len() as u32;
                    tests.push(t);
                    values.push(c);
                }
            }
            self.ds.tests = tests;
            self.ds.values = values;
            self.ds.test_index = self
                .ds
                .tests
                .iter()
                .enumerate()
                .map(|(i, t)| (t.key.clone(), i))
                .collect();
            for p in self.ds.parts.iter_mut() {
                p.first_fail = p.first_fail.and_then(|f| {
                    let r = remap[f as usize];
                    (r != u32::MAX).then_some(r)
                });
                p.alarms = p
                    .alarms
                    .iter()
                    .map(|&a| remap[a as usize])
                    .filter(|&r| r != u32::MAX)
                    .collect();
            }
        }
        self.ds.wafers = self.wafer_ids.into_keys().collect();
        self.ds.sources.push(self.src);
        self.ds
    }
}

/// Load one STDF file into a dataset.
pub fn load(path: impl AsRef<Path>, opt: &LoadOptions) -> Result<Dataset> {
    let path = path.as_ref();
    let mut rdr = StdfReader::open(path)?;
    let mut b = Builder::new(opt, &path.to_string_lossy());
    b.src.endian = format!("{:?}", rdr.endian);
    let endian = rdr.endian;
    let mut counts: IndexMap<String, u64> = IndexMap::new();
    while let Some((h, data)) = rdr.next_raw()? {
        *counts
            .entry(crate::stdf::record_name(h.typ, h.sub).to_string())
            .or_default() += 1;
        // Skip decoding of records we don't use.
        match (h.typ, h.sub) {
            (1, 10)
            | (1, 20)
            | (1, 40)
            | (1, 50)
            | (1, 60)
            | (2, 10)
            | (5, 10)
            | (5, 20)
            | (10, 30)
            | (15, 10)
            | (15, 15)
            | (15, 20) => {
                if (h.typ, h.sub) == (15, 10) && b.fast_ptr(data, endian) {
                    continue;
                }
                let rec = Record::parse(h.typ, h.sub, data, endian);
                b.handle(rec);
            }
            _ => {}
        }
    }
    b.src.record_counts = counts;
    let ds = b.finish();
    Ok(match opt.dedup {
        Some(DedupMode::Last) => ds.dedup(true),
        Some(DedupMode::First) => ds.dedup(false),
        None => ds,
    })
}

/// Load several files in parallel and merge them into one dataset.
pub fn load_many<P: AsRef<Path> + Sync>(paths: &[P], opt: &LoadOptions) -> Result<Dataset> {
    let sets: Vec<Dataset> = paths
        .par_iter()
        .map(|p| load(p, opt))
        .collect::<Result<Vec<_>>>()?;
    if sets.len() == 1 {
        return Ok(sets.into_iter().next().unwrap());
    }
    Ok(Dataset::merge(sets))
}
