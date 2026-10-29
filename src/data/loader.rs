//! Build a [`Dataset`] from STDF files.

use super::{BinDef, DatalogEntry, Dataset, Part, PartLimits, SourceInfo, TestInfo, TestKind};
use crate::stdf::{Record, StdfReader};
use anyhow::Result;
use indexmap::IndexMap;
use rayon::prelude::*;
use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

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
    fails: Vec<u32>,
    /// a PIR opened this part (results without one are not a part)
    started: bool,
    /// per-part limits of the tests whose limits change from part to part: (test, lo, hi)
    lims: Vec<(u32, f32, f32)>,
}

struct Builder<'o> {
    opt: &'o LoadOptions,
    ds: Dataset,
    src: SourceInfo,
    /// test_num -> [(name, test idx)]
    by_num: FastMap<u32, Vec<(String, usize)>>,
    mpr_by_key: FastMap<(usize, u16), usize>,
    last_name: FastMap<u32, String>,
    mpr_default_indx: FastMap<u32, Vec<u16>>,
    limits_set: Vec<bool>,
    /// per test: the part row its limits were first set on
    first_limit_row: Vec<usize>,
    part_limits: Vec<Option<PartLimits>>,
    datalog: Vec<DatalogEntry>,
    pending: FastMap<(u8, u8), Pending>,
    pin_names: HashMap<u16, String>,
    wafer_ids: IndexMap<String, u32>,
    /// head -> wafer index of its last WIR (see wafer_of)
    cur_wafer: BTreeMap<u8, u32>,
    /// the builder is one chunk of a file (see new_chunk)
    chunk: bool,
    /// the chunk read state from before it, so it is handled on the file's builder instead
    dep: bool,
    saw_mir: bool,
    saw_mrr: bool,
    /// the tests this chunk named, in order: the file's last name for a number is the last one new to the file
    named: Vec<(u32, String)>,
    /// parts of a chunk whose wafer depends on the chunks before it: (part row, head, the lowest
    /// head with a WIR in the chunk at that part, and its wafer index)
    wafer_fix: Vec<(usize, u8, Option<(u8, u32)>)>,
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
            by_num: FastMap::default(),
            mpr_by_key: FastMap::default(),
            last_name: FastMap::default(),
            mpr_default_indx: FastMap::default(),
            limits_set: vec![],
            first_limit_row: vec![],
            part_limits: vec![],
            datalog: vec![],
            pending: FastMap::default(),
            pin_names: HashMap::new(),
            wafer_ids: IndexMap::new(),
            cur_wafer: BTreeMap::new(),
            chunk: false,
            dep: false,
            saw_mir: false,
            saw_mrr: false,
            named: vec![],
            wafer_fix: vec![],
        }
    }

    /// A builder for one chunk of a file. Its records are handled from an empty state; where they
    /// read the state before the chunk, `dep` is set and the chunk is handled on the file's builder.
    fn new_chunk(opt: &'o LoadOptions) -> Self {
        Self {
            chunk: true,
            ..Self::new(opt, "")
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
        self.first_limit_row.push(0);
        self.part_limits.push(None);
        i
    }

    /// Resolve test index for (num, txt) creating it if needed.
    fn test_for(&mut self, num: u32, txt: &str, kind: TestKind) -> usize {
        let name: &str = if txt.is_empty() {
            // the file's last name for this number is not known to a chunk (see merge)
            if self.chunk {
                self.dep = true;
            }
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
            if self.chunk {
                self.named.push((num, txt.to_string()));
            }
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
        self.first_limit_row[t] = self.ds.parts.len();
    }

    /// Records a PTR's own limits when they differ from the test's limits. A test whose limits
    /// change from part to part keeps them per part (Dataset::part_limits).
    fn note_limits(&mut self, t: usize, head: u8, site: u8, lim: Option<(f32, f32)>) {
        let stat = self.static_limits(t);
        // a PTR that logs no limits inherits the test's
        let lim = lim.unwrap_or(stat);
        let same = |a: f32, b: f32| a.to_bits() == b.to_bits() || (a.is_nan() && b.is_nan());
        if self.part_limits[t].is_none() {
            if same(lim.0, stat.0) && same(lim.1, stat.1) {
                return;
            }
            // this part (row n) and the parts from the test's first limits on had the test's limits
            let n = self.ds.parts.len();
            let mut pl = PartLimits {
                lo: vec![f32::NAN; n + 1],
                hi: vec![f32::NAN; n + 1],
            };
            for r in self.first_limit_row[t].min(n)..=n {
                pl.lo[r] = stat.0;
                pl.hi[r] = stat.1;
            }
            // the parts still open had the test's limits too, and their rows are written later
            for p in self.pending.values_mut() {
                if p.results.iter().any(|(r, _)| *r == t as u32) {
                    p.lims.push((t as u32, stat.0, stat.1));
                }
            }
            self.part_limits[t] = Some(pl);
        }
        self.pending_for(head, site)
            .lims
            .push((t as u32, lim.0, lim.1));
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
        if failed && !p.fails.contains(&(t as u32)) {
            p.fails.push(t as u32);
        }
        if failed && p.first_fail.is_none() {
            p.first_fail = Some(t as u32);
        }
    }

    fn handle(&mut self, rec: Record) {
        match rec {
            Record::MIR(m) => {
                self.saw_mir = true;
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
            Record::MRR(m) => {
                self.saw_mrr = true;
                self.src.finish_t = m.finish_t;
            }
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
                self.pending.insert(
                    (p.head_num, p.site_num),
                    Pending {
                        started: true,
                        ..Pending::default()
                    },
                );
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
                let lim = part_limit(p.opt_flag, p.lo_limit, p.hi_limit);
                self.note_limits(t, p.head_num, p.site_num, lim);
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
                if self.chunk {
                    self.dep = true;
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
                    self.first_limit_row[t] = self.ds.parts.len();
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
                // renames tests of the file so far, which a chunk cannot see
                if self.chunk {
                    self.dep = true;
                }
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
                self.write_results(row, pend.results);
                self.write_limits(row, &pend.lims);
                let passed = p.passed();
                // a head with no WIR yet in a chunk takes its wafer from the chunks before it
                let wafer = if self.chunk && !self.cur_wafer.contains_key(&p.head_num) {
                    let lowest = self.cur_wafer.iter().next().map(|(k, v)| (*k, *v));
                    self.wafer_fix
                        .push((self.ds.parts.len(), p.head_num, lowest));
                    u32::MAX
                } else {
                    self.wafer_of(p.head_num)
                };
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
                    wafer,
                    part_txt: p.part_txt,
                    first_fail: pend.first_fail,
                    alarms: pend.alarms,
                    fails: pend.fails,
                    partial: false,
                });
            }
            Record::GDR(g) => self.datalog.push(DatalogEntry {
                kind: "GDR".into(),
                after_part: self.ds.parts.len(),
                text: format!("{:?}", g.gen_data),
            }),
            Record::DTR(x) => self.datalog.push(DatalogEntry {
                kind: "DTR".into(),
                after_part: self.ds.parts.len(),
                text: x.text_dat.trim_end().to_string(),
            }),
            _ => {}
        }
    }

    /// Allocation-free handling of a PTR whose test is already known. Returns false when the full
    /// parser must be used: when the test is unknown, or when the PTR logs limits and the test has
    /// not taken its limits yet.
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
        // an empty name resolves from the file so far, which a chunk does not have (see test_for)
        if n == 0 && self.chunk {
            return false;
        }
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
        let Some(lim) = ptr_limits(d, 13 + n, e) else {
            return false;
        };
        // set_limits does nothing for a PTR without OPT_FLAG, so only a PTR with limits needs it
        if !self.limits_set[t] && lim.is_some() {
            return false;
        }
        self.note_limits(t, head, site, lim);
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

    /// Writes one part's own limits into the per-part limit columns at `row`.
    fn write_limits(&mut self, row: usize, lims: &[(u32, f32, f32)]) {
        for &(t, lo, hi) in lims {
            if let Some(pl) = self.part_limits[t as usize].as_mut() {
                if pl.lo.len() <= row {
                    pl.lo.resize(row + 1, f32::NAN);
                    pl.hi.resize(row + 1, f32::NAN);
                }
                pl.lo[row] = lo;
                pl.hi[row] = hi;
            }
        }
    }

    /// Writes one part's results into the value columns at `row`.
    fn write_results(&mut self, row: usize, results: Vec<(u32, f32)>) {
        for (t, v) in results {
            let col = &mut self.ds.values[t as usize];
            if col.len() <= row {
                col.resize(row + 1, f32::NAN);
            }
            col[row] = v;
        }
    }

    /// The wafer of a part on `head`: the last WIR of that head, else of any head.
    fn wafer_of(&self, head: u8) -> u32 {
        self.cur_wafer
            .get(&head)
            .copied()
            .or_else(|| self.cur_wafer.values().next().copied())
            .unwrap_or(u32::MAX)
    }

    /// Parts that started (PIR) but never got a PRR: a file cut off inside a part, or an
    /// incomplete tmp file. Their results are kept; they have no bin and no pass/fail verdict.
    fn keep_partial_parts(&mut self) {
        let mut keys: Vec<(u8, u8)> = self
            .pending
            .iter()
            .filter(|(_, p)| p.started)
            .map(|(k, _)| *k)
            .collect();
        keys.sort_unstable();
        for (head, site) in keys {
            let pend = self.pending.remove(&(head, site)).unwrap_or_default();
            let row = self.ds.parts.len();
            let num_test = pend.results.len() as u16;
            self.write_results(row, pend.results);
            self.write_limits(row, &pend.lims);
            self.ds.parts.push(Part {
                file: 0,
                part_id: String::new(),
                head,
                site,
                x: i16::MIN,
                y: i16::MIN,
                hbin: 0,
                sbin: 0,
                passed: None,
                part_flg: 0,
                num_test,
                test_t: 0,
                wafer: self.wafer_of(head),
                part_txt: String::new(),
                first_fail: pend.first_fail,
                alarms: pend.alarms,
                fails: pend.fails,
                partial: true,
            });
            self.src.partial_parts += 1;
        }
    }

    /// Handles the records of a batch in file order.
    fn feed(&mut self, batch: &Batch, endian: crate::stdf::Endian) {
        for &(typ, sub, start, end) in &batch.spans {
            let data = &batch.bytes[start..end];
            if (typ, sub) == (15, 10) && self.fast_ptr(data, endian) {
                continue;
            }
            self.handle(Record::parse(typ, sub, data, endian));
        }
    }

    /// The test with this number and name, as test_for finds it.
    fn test_at(&self, num: u32, name: &str) -> Option<usize> {
        self.by_num
            .get(&num)?
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, i)| *i)
    }

    /// The test's static limits as note_limits reads them (NaN where none).
    fn static_limits(&self, t: usize) -> (f32, f32) {
        let ti = &self.ds.tests[t];
        (
            ti.lo.map_or(f32::NAN, |v| v as f32),
            ti.hi.map_or(f32::NAN, |v| v as f32),
        )
    }

    /// Whether the chunk `c`, built from an empty state, can be merged after this builder. It must
    /// not have read the state before it, this builder must have no open part, and it may not have
    /// per-part limits. A test both sides know must have the same static limits on both sides,
    /// since the chunk judged its own PTR limits against its static limits.
    fn can_merge(&self, c: &Builder<'_>) -> bool {
        if c.dep || !self.pending.is_empty() || c.part_limits.iter().any(Option::is_some) {
            return false;
        }
        c.by_num.iter().all(|(num, list)| {
            list.iter().all(|(name, l)| {
                let Some(g) = self.test_at(*num, name) else {
                    return true;
                };
                if self.part_limits[g].is_some() {
                    return false;
                }
                if !self.limits_set[g] || !c.limits_set[*l] {
                    return true;
                }
                let (a, b) = (self.static_limits(g), c.static_limits(*l));
                same_limit(a.0, b.0) && same_limit(a.1, b.1)
            })
        })
    }

    /// Merges a chunk that `can_merge` accepted, as if its records came after this builder's.
    fn merge(&mut self, mut c: Builder<'_>) {
        let base = self.ds.parts.len();
        // the file's last name for a number is the last test under it that the file did not have
        for (num, name) in std::mem::take(&mut c.named) {
            if self.test_at(num, &name).is_none() {
                self.last_name.insert(num, name);
            }
        }
        // wafer indices of the chunk, continued in the order the file first saw them
        let mut wmap = Vec::with_capacity(c.wafer_ids.len());
        for name in std::mem::take(&mut c.wafer_ids).into_keys() {
            let n = self.wafer_ids.len() as u32;
            wmap.push(*self.wafer_ids.entry(name).or_insert(n));
        }
        // the number and name each test was created with, which is how test_for finds it again
        let mut created = vec![(0u32, String::new()); c.ds.tests.len()];
        for (num, list) in std::mem::take(&mut c.by_num) {
            for (name, l) in list {
                created[l] = (num, name);
            }
        }
        let mut tmap: Vec<u32> = Vec::with_capacity(created.len());
        let tests = std::mem::take(&mut c.ds.tests);
        let values = std::mem::take(&mut c.ds.values);
        for (l, (ti, col)) in tests.into_iter().zip(values).enumerate() {
            let (num, name) = std::mem::take(&mut created[l]);
            let g = match self.test_at(num, &name) {
                Some(g) => {
                    let t = &mut self.ds.tests[g];
                    t.exec_count += ti.exec_count;
                    t.fail_count += ti.fail_count;
                    // the first limits set in the file stay (see set_limits)
                    if !self.limits_set[g] && c.limits_set[l] {
                        t.lo = ti.lo;
                        t.hi = ti.hi;
                        t.lo_spec = ti.lo_spec;
                        t.hi_spec = ti.hi_spec;
                        t.units = ti.units;
                        t.res_scal = ti.res_scal;
                        self.limits_set[g] = true;
                        self.first_limit_row[g] = base + c.first_limit_row[l];
                    }
                    g
                }
                None => {
                    let mut key = format!("{num}:{name}");
                    if self.ds.test_index.contains_key(&key) {
                        key = format!("{num}:{name}#{}", self.ds.tests.len());
                    }
                    let g = self.ds.tests.len();
                    self.ds.test_index.insert(key.clone(), g);
                    self.ds.tests.push(TestInfo { key, ..ti });
                    self.ds.values.push(Vec::new());
                    self.limits_set.push(c.limits_set[l]);
                    self.first_limit_row.push(base + c.first_limit_row[l]);
                    self.part_limits.push(None);
                    self.by_num.entry(num).or_default().push((name, g));
                    g
                }
            };
            tmap.push(g as u32);
            let dst = &mut self.ds.values[g];
            dst.resize(base, f32::NAN);
            dst.extend_from_slice(&col);
        }
        let mut parts = std::mem::take(&mut c.ds.parts);
        for p in parts.iter_mut() {
            if p.wafer != u32::MAX {
                p.wafer = wmap[p.wafer as usize];
            }
            p.first_fail = p.first_fail.map(|t| tmap[t as usize]);
            for t in p.fails.iter_mut().chain(p.alarms.iter_mut()) {
                *t = tmap[*t as usize];
            }
        }
        // before the chunk's own WIRs are added to the file's wafer state
        for (row, head, lowest) in std::mem::take(&mut c.wafer_fix) {
            parts[row].wafer = self.deferred_wafer(head, lowest, &wmap);
        }
        for (head, w) in std::mem::take(&mut c.cur_wafer) {
            self.cur_wafer.insert(head, wmap[w as usize]);
        }
        self.ds.parts.extend(parts);
        for mut e in std::mem::take(&mut c.datalog) {
            e.after_part += base;
            self.datalog.push(e);
        }
        // the first bin record that names a bin wins (see handle)
        for (num, bin) in std::mem::take(&mut c.ds.hbins) {
            if self.ds.hbins.get(&num).is_none_or(|e| e.name.is_empty()) {
                self.ds.hbins.insert(num, bin);
            }
        }
        for (num, bin) in std::mem::take(&mut c.ds.sbins) {
            if self.ds.sbins.get(&num).is_none_or(|e| e.name.is_empty()) {
                self.ds.sbins.insert(num, bin);
            }
        }
        self.pin_names.extend(std::mem::take(&mut c.pin_names));
        // the last MIR and MRR of the file win (see handle)
        if c.saw_mir {
            let s = &c.src;
            self.src.lot_id = s.lot_id.clone();
            self.src.sublot_id = s.sublot_id.clone();
            self.src.part_typ = s.part_typ.clone();
            self.src.job_nam = s.job_nam.clone();
            self.src.job_rev = s.job_rev.clone();
            self.src.node_nam = s.node_nam.clone();
            self.src.tstr_typ = s.tstr_typ.clone();
            self.src.test_cod = s.test_cod.clone();
            self.src.tst_temp = s.tst_temp.clone();
            self.src.flow_id = s.flow_id.clone();
            self.src.start_t = s.start_t;
        }
        if c.saw_mrr {
            self.src.finish_t = c.src.finish_t;
        }
        // the chunk ends with the parts it has open (see can_merge: none are open here)
        // the open parts name the chunk's tests: their results follow the file's indices
        let mut pending = std::mem::take(&mut c.pending);
        for p in pending.values_mut() {
            for (t, _) in p.results.iter_mut() {
                *t = tmap[*t as usize];
            }
            for (t, _, _) in p.lims.iter_mut() {
                *t = tmap[*t as usize];
            }
            p.first_fail = p.first_fail.map(|t| tmap[t as usize]);
            for t in p.fails.iter_mut().chain(p.alarms.iter_mut()) {
                *t = tmap[*t as usize];
            }
        }
        self.pending = pending;
    }

    /// The wafer of a part of a chunk whose head had no WIR in the chunk yet: the head's wafer
    /// from the chunks before it, else the fallback of wafer_of over every head seen by then.
    fn deferred_wafer(&self, head: u8, lowest: Option<(u8, u32)>, wmap: &[u32]) -> u32 {
        if let Some(&w) = self.cur_wafer.get(&head) {
            return w;
        }
        match (self.cur_wafer.iter().next(), lowest) {
            (Some((kb, vb)), Some((kl, l))) => {
                if kl <= *kb {
                    wmap[l as usize]
                } else {
                    *vb
                }
            }
            (Some((_, vb)), None) => *vb,
            (None, Some((_, l))) => wmap[l as usize],
            (None, None) => u32::MAX,
        }
    }

    fn finish(mut self) -> Dataset {
        self.keep_partial_parts();
        let n = self.ds.parts.len();
        for c in self.ds.values.iter_mut() {
            c.resize(n, f32::NAN);
        }
        for pl in self.part_limits.iter_mut().flatten() {
            pl.lo.resize(n, f32::NAN);
            pl.hi.resize(n, f32::NAN);
        }
        let mut part_limits = std::mem::take(&mut self.part_limits);
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
            part_limits = part_limits
                .into_iter()
                .zip(&keep)
                .filter(|(_, k)| **k)
                .map(|(pl, _)| pl)
                .collect();
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
                p.fails = p
                    .fails
                    .iter()
                    .map(|&a| remap[a as usize])
                    .filter(|&r| r != u32::MAX)
                    .collect();
                p.alarms = p
                    .alarms
                    .iter()
                    .map(|&a| remap[a as usize])
                    .filter(|&r| r != u32::MAX)
                    .collect();
            }
        }
        self.ds.part_limits = part_limits;
        self.ds.datalog = self.datalog;
        self.ds.wafers = self.wafer_ids.into_keys().collect();
        self.ds.sources.push(self.src);
        self.ds
    }
}

/// Load one STDF file into a dataset.
/// Hash map for the loader's small integer keys (test numbers, head and site).
/// SipHash costs more than the lookup itself at one or more lookups per result.
type FastMap<K, V> = HashMap<K, V, std::hash::BuildHasherDefault<FastHasher>>;

#[derive(Default)]
struct FastHasher(u64);

impl std::hash::Hasher for FastHasher {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.add(b as u64);
        }
    }

    fn write_u8(&mut self, i: u8) {
        self.add(i as u64);
    }

    fn write_u32(&mut self, i: u32) {
        self.add(i as u64);
    }

    fn write_usize(&mut self, i: usize) {
        self.add(i as u64);
    }
}

impl FastHasher {
    fn add(&mut self, word: u64) {
        self.0 = (self.0.rotate_left(5) ^ word).wrapping_mul(0x51_7c_c1_b7_27_22_0a_95);
    }
}

/// The limits a PTR logs, as the test limits are read from them (NaN where none): OPT_FLAG bits
/// 4 and 5 drop the low limit, 6 and 7 the high one, and a missing OPT_FLAG means no limits.
fn part_limit(opt: Option<u8>, lo: Option<f32>, hi: Option<f32>) -> Option<(f32, f32)> {
    let opt = opt?;
    let lo = match lo {
        Some(v) if opt & 0x40 == 0 && opt & 0x10 == 0 && v.is_finite() => v,
        _ => f32::NAN,
    };
    let hi = match hi {
        Some(v) if opt & 0x80 == 0 && opt & 0x20 == 0 && v.is_finite() => v,
        _ => f32::NAN,
    };
    Some((lo, hi))
}

/// The same limits read from the raw PTR body: the fields after ALARM_ID, which starts at `at`.
/// None when the body is cut short, so the full parser decides.
fn ptr_limits(d: &[u8], at: usize, e: crate::stdf::Endian) -> Option<Option<(f32, f32)>> {
    let alarm = *d.get(at)? as usize;
    let pos = at + 1 + alarm;
    let Some(&opt) = d.get(pos) else {
        return Some(None);
    };
    let f4 = |i: usize| -> Option<f32> {
        let b = d.get(i..i + 4)?;
        let b = [b[0], b[1], b[2], b[3]];
        Some(match e {
            crate::stdf::Endian::Little => f32::from_le_bytes(b),
            crate::stdf::Endian::Big => f32::from_be_bytes(b),
        })
    };
    let (lo, hi) = (f4(pos + 4)?, f4(pos + 8)?);
    Some(part_limit(Some(opt), Some(lo), Some(hi)))
}

/// Bytes of record payloads per batch handed from the reader thread to the builder.
const BATCH_BYTES: usize = 4 << 20;

/// Records per batch, so zero-length records cannot grow a batch without limit.
const BATCH_RECORDS: usize = 1 << 16;

/// Record payloads of one batch, in file order: (type, subtype, byte range in `bytes`).
#[derive(Default)]
struct Batch {
    bytes: Vec<u8>,
    spans: Vec<(u8, u8, usize, usize)>,
}

impl Batch {
    fn push(&mut self, typ: u8, sub: u8, data: &[u8]) {
        let start = self.bytes.len();
        self.bytes.extend_from_slice(data);
        self.spans.push((typ, sub, start, self.bytes.len()));
    }
}

pub fn load(path: impl AsRef<Path>, opt: &LoadOptions) -> Result<Dataset> {
    let path = path.as_ref();
    let mut rdr = StdfReader::open(path)?;
    let mut b = Builder::new(opt, &path.to_string_lossy());
    b.src.endian = format!("{:?}", rdr.endian);
    let endian = rdr.endian;
    // The reader thread frames the records and counts them by (type, subtype). The batches are
    // built in file order: on this thread, or in windows of chunks built in parallel and merged in
    // file order. Either way the dataset is the one a single thread would build.
    let in_pool = rayon::current_thread_index().is_some();
    let threads = if in_pool {
        rayon::current_num_threads()
    } else {
        parse_threads()
    };
    let pool = if in_pool || threads <= 1 {
        None
    } else {
        Some(load_pool(threads)?)
    };
    // a short first window shows early whether the file can be built in parallel
    let mut window_len = 2;
    let mut parallel = threads > 1;
    // the reader runs ahead by about a window while the chunks of one are built and merged
    let (tx, rx) = std::sync::mpsc::sync_channel::<Batch>(threads.max(4));
    let counts = std::thread::scope(|s| -> Result<Counts> {
        let reader = s.spawn(move || frame(&mut rdr, &tx));
        if threads <= 1 {
            for batch in rx {
                b.feed(&batch, endian);
            }
        } else {
            let mut window = Vec::with_capacity(window_len);
            for batch in rx {
                window.push(batch);
                if window.len() == window_len {
                    parallel =
                        build_window(&mut b, opt, endian, &window, pool.as_deref(), parallel);
                    window_len = if parallel { 2 * threads } else { 1 };
                    window.clear();
                }
            }
            build_window(&mut b, opt, endian, &window, pool.as_deref(), parallel);
        }
        reader
            .join()
            .map_err(|_| anyhow::anyhow!("reader thread panicked"))?
    })?;
    let mut named: IndexMap<String, u64> = IndexMap::new();
    for ((typ, sub), n) in counts {
        *named
            .entry(crate::stdf::record_name(typ, sub).to_string())
            .or_default() += n;
    }
    b.src.record_counts = named;
    let ds = b.finish();
    Ok(match opt.dedup {
        Some(DedupMode::Last) => ds.dedup(true),
        Some(DedupMode::First) => ds.dedup(false),
        None => ds,
    })
}

/// Record counts by (type, subtype), in the order the kinds first appear. Counting every record
/// with the default hasher cost more than framing the file.
type Counts = IndexMap<(u8, u8), u64, std::hash::BuildHasherDefault<FastHasher>>;

/// The head and site of a PIR or PRR, which open and close a part.
fn part_key(data: &[u8]) -> Option<(u8, u8)> {
    Some((*data.first()?, *data.get(1)?))
}

/// Frames the records of the file into batches in file order and counts every record by (type,
/// subtype). A batch closes before a PIR once it is full and no part is open, so a part is never
/// split between two batches; a batch that grows far past the limits closes anyway.
fn frame(rdr: &mut StdfReader, tx: &std::sync::mpsc::SyncSender<Batch>) -> Result<Counts> {
    let mut counts = Counts::default();
    let mut batch = Batch::default();
    let mut open = std::collections::HashSet::new();
    while let Some((h, data)) = rdr.next_raw()? {
        *counts.entry((h.typ, h.sub)).or_default() += 1;
        // Skip records we don't use.
        if !matches!(
            (h.typ, h.sub),
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
                | (15, 20)
                | (50, 10)
                | (50, 30)
        ) {
            continue;
        }
        let full = batch.bytes.len() >= BATCH_BYTES || batch.spans.len() >= BATCH_RECORDS;
        let huge = batch.bytes.len() >= 4 * BATCH_BYTES || batch.spans.len() >= 4 * BATCH_RECORDS;
        if (full && (h.typ, h.sub) == (5, 10) && open.is_empty()) || huge {
            // stop reading when the builder has stopped on an error
            if tx.send(std::mem::take(&mut batch)).is_err() {
                return Ok(counts);
            }
        }
        match (h.typ, h.sub) {
            (5, 10) => open.extend(part_key(data)),
            (5, 20) => {
                if let Some(k) = part_key(data) {
                    open.remove(&k);
                }
            }
            _ => {}
        }
        batch.push(h.typ, h.sub, data);
    }
    if !batch.spans.is_empty() {
        let _ = tx.send(batch);
    }
    Ok(counts)
}

/// Builds a window of batches as chunks, on `pool` (or on the pool this thread is in), then merges
/// them into `b` in file order. A chunk that cannot be merged is handled on `b` itself. Returns
/// whether the next window is built in parallel too: not once most chunks of a window had to wait
/// for the chunks before them, since the file then depends on its order and is read on one thread.
fn build_window(
    b: &mut Builder<'_>,
    opt: &LoadOptions,
    endian: crate::stdf::Endian,
    window: &[Batch],
    pool: Option<&rayon::ThreadPool>,
    parallel: bool,
) -> bool {
    if window.is_empty() {
        return parallel;
    }
    if !parallel {
        for batch in window {
            b.feed(batch, endian);
        }
        return false;
    }
    let build = || {
        window
            .par_iter()
            .map(|batch| {
                let mut c = Builder::new_chunk(opt);
                c.feed(batch, endian);
                c
            })
            .collect::<Vec<_>>()
    };
    let chunks = match pool {
        Some(pool) => pool.install(build),
        None => build(),
    };
    let mut replayed = 0;
    for (batch, c) in window.iter().zip(chunks) {
        if b.can_merge(&c) {
            b.merge(c);
        } else {
            replayed += 1;
            b.feed(batch, endian);
        }
    }
    replayed * 2 < window.len()
}

/// Equal as limits are compared: bit for bit, or both missing.
fn same_limit(a: f32, b: f32) -> bool {
    a.to_bits() == b.to_bits() || (a.is_nan() && b.is_nan())
}

/// Threads for loading files: 0 = auto (one per logical core). Set from the saved settings,
/// the STDF_CYPHER_THREADS variable or --threads.
static LOAD_THREADS: AtomicUsize = AtomicUsize::new(0);

/// The pool used for loading, rebuilt when its size changes.
static LOAD_POOL: Mutex<Option<(usize, Arc<rayon::ThreadPool>)>> = Mutex::new(None);

/// Sets the number of threads for loading files (0 = auto).
pub fn set_load_threads(n: usize) {
    LOAD_THREADS.store(n.min(256), Ordering::Relaxed);
}

/// The setting as given (0 = auto).
pub fn load_threads_setting() -> usize {
    LOAD_THREADS.load(Ordering::Relaxed)
}

/// Logical cores of this PC.
pub fn cores() -> usize {
    std::thread::available_parallelism().map_or(1, |n| n.get())
}

/// Threads for a load of `files` files: the setting, or one per core, never more than the files.
pub fn effective_threads(files: usize) -> usize {
    parse_threads().min(files).max(1)
}

/// Threads for reading one file: the setting, or one per core.
fn parse_threads() -> usize {
    match LOAD_THREADS.load(Ordering::Relaxed) {
        0 => cores(),
        n => n,
    }
}

fn load_pool(n: usize) -> Result<Arc<rayon::ThreadPool>> {
    let mut slot = LOAD_POOL.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((k, pool)) = slot.as_ref() {
        if *k == n {
            return Ok(pool.clone());
        }
    }
    let pool = Arc::new(
        rayon::ThreadPoolBuilder::new()
            .num_threads(n)
            .thread_name(|i| format!("stdf-load-{i}"))
            .build()?,
    );
    *slot = Some((n, pool.clone()));
    Ok(pool)
}

/// Load several files and merge them into one dataset, on the threads of the setting.
pub fn load_many<P: AsRef<Path> + Sync>(paths: &[P], opt: &LoadOptions) -> Result<Dataset> {
    load_many_threads(paths, opt, effective_threads(paths.len()))
}

/// Load several files on `threads` threads (1 = one after another) and merge them into one
/// dataset. The files keep their order, so the result is the same for any thread count.
pub fn load_many_threads<P: AsRef<Path> + Sync>(
    paths: &[P],
    opt: &LoadOptions,
    threads: usize,
) -> Result<Dataset> {
    let sets: Vec<Dataset> = if threads <= 1 {
        paths
            .iter()
            .map(|p| load(p, opt))
            .collect::<Result<Vec<_>>>()?
    } else {
        load_pool(threads)?.install(|| {
            paths
                .par_iter()
                .map(|p| load(p, opt))
                .collect::<Result<Vec<_>>>()
        })?
    };
    if sets.len() == 1 {
        return Ok(sets.into_iter().next().unwrap());
    }
    Ok(Dataset::merge(sets))
}
