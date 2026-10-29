//! Synthetic STDF generator for demos and tests. Produces realistic wafer-sort
//! data: multi-site, correlated parametrics, a site offset, radial wafer
//! gradient, a few outlier dies, MPR and FTR records, and HBR/SBR/PCR summaries.

use crate::stdf::*;
use anyhow::Result;
use rand::{Rng, SeedableRng};
use rand_distr::{Distribution, Normal};
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Debug, Clone)]
pub struct SampleSpec {
    pub lot_id: String,
    pub wafers: usize,
    /// die grid radius (dies across ≈ 2*radius)
    pub radius: i16,
    pub sites: u8,
    pub tests: usize,
    pub seed: u64,
    /// mean shift applied to tests (in sigma units) – use to create a "B" file
    pub shift: f64,
    /// extra noise multiplier for a "B" file
    pub noise: f64,
    /// fraction of dies that are outliers
    pub outlier_rate: f64,
    pub endian: Endian,
    /// final-test style (no wafer, no x/y)
    pub final_test: bool,
    /// independent re-measurement noise (sigma units) – same seed + this
    /// option simulates re-testing the same parts on another tester
    pub meas_noise: f64,
    /// per-test offsets in sigma units applied to every 3rd test (tester offset)
    pub tester_offset: f64,
    /// log prefixed units (uA, mV, MHz…) as the base unit (A, V, Hz) with the
    /// PTR RES_SCAL a tester uses to show them in the prefixed unit again
    pub base_units: bool,
    /// another draw of the re-measurement noise: same seed, meas_noise and a
    /// different meas_seed = the same parts on a further retest
    pub meas_seed: u64,
    /// limits move from wafer to wafer (a PAT-style program) and every PTR logs its own
    pub dynamic_limits: bool,
    /// loadboard (DIB) ID logged as test 5100 for every part
    pub loadboard: Option<f64>,
    /// share of parts whose loadboard result is missing
    pub loadboard_missing: f64,
    /// plant a retest pattern on these test numbers: some dies fail it just
    /// above the high limit (marginal), well above it (hard bad) or at a clamp
    /// value (extreme). On a retest (`meas_seed` ≠ 0) marginal and extreme
    /// dies mostly pass again; hard-bad dies never do. Each test picks its
    /// own dies.
    pub band_tests: Vec<u32>,
    /// extra parametric tests with these numbers (e.g. 1013, 2000), placed in
    /// test-number order among the generated ones
    pub extra_tests: Vec<u32>,
    /// the dies a band test would fail still pass these tests here, just
    /// inside the high limit: a run 1 where they are good, before a later
    /// file (with `band_tests`) fails them
    pub latent_tests: Vec<u32>,
}

/// Uniform 0..1 from a die position and a salt (splitmix64), independent of
/// the random streams so a die keeps its role in every file.
fn die_u(wafer: usize, x: i16, y: i16, salt: u64) -> f64 {
    let mut z = (wafer as u64) << 40
        ^ ((x as u16 as u64) << 20)
        ^ (y as u16 as u64)
        ^ salt.wrapping_mul(0xD1B5_4A32_D192_ED03);
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    (z >> 11) as f64 / (1u64 << 53) as f64
}

impl Default for SampleSpec {
    fn default() -> Self {
        Self {
            lot_id: "LOTA".into(),
            wafers: 3,
            radius: 12,
            sites: 4,
            tests: 40,
            seed: 1,
            shift: 0.0,
            noise: 1.0,
            outlier_rate: 0.004,
            endian: Endian::Little,
            final_test: false,
            meas_noise: 0.0,
            tester_offset: 0.0,
            base_units: false,
            meas_seed: 0,
            dynamic_limits: false,
            loadboard: None,
            loadboard_missing: 0.0,
            band_tests: vec![],
            extra_tests: vec![],
            latent_tests: vec![],
        }
    }
}

struct TDef {
    num: u32,
    name: String,
    units: &'static str,
    mean: f64,
    sigma: f64,
    lo: f64,
    hi: f64,
    /// weight on the shared latent factor (creates correlation)
    load: f64,
    site_off: f64,
    radial: f64,
}

/// Limit factor of wafer `wi`: 1.0, or 1.00 to 1.09 when the limits are dynamic.
fn limit_factor(spec: &SampleSpec, wi: usize) -> f64 {
    if spec.dynamic_limits {
        1.0 + 0.03 * (wi % 4) as f64
    } else {
        1.0
    }
}

pub fn generate(path: impl AsRef<Path>, spec: &SampleSpec) -> Result<usize> {
    let mut rng = rand::rngs::StdRng::seed_from_u64(spec.seed);
    let defs_rng = &mut rand::rngs::StdRng::seed_from_u64(42); // same test program for every file
    let names = [
        "VDD_LEAK", "IDDQ", "VOH", "VOL", "FREQ_RO", "VTH_N", "VTH_P", "RES_POLY", "CONT_RES",
        "ISB", "VREF", "BG_TRIM", "LDO_OUT", "OSC_FREQ", "IIH", "IIL", "TPD", "DUTY", "GAIN",
        "OFFSET",
    ];
    let units = [
        "uA", "nA", "V", "V", "MHz", "V", "V", "ohm", "ohm", "uA", "V", "code", "V", "MHz", "nA",
        "nA", "ns", "%", "dB", "mV",
    ];
    let mut defs = vec![];
    for i in 0..spec.tests {
        let mean: f64 = defs_rng.gen_range(0.5..50.0);
        let sigma = mean * defs_rng.gen_range(0.01..0.05);
        let k = if i % 13 == 3 {
            defs_rng.gen_range(2.6..3.0)
        } else {
            defs_rng.gen_range(4.0..7.0)
        };
        defs.push(TDef {
            num: 1000 + i as u32 * 10,
            name: format!("{}_{}", names[i % names.len()], i / names.len()),
            units: units[i % units.len()],
            mean,
            sigma,
            lo: mean - k * sigma,
            hi: mean + k * sigma,
            load: if i % 3 == 0 {
                0.9
            } else if i % 3 == 1 {
                -0.6
            } else {
                0.0
            },
            site_off: if i % 5 == 0 { 0.8 } else { 0.0 },
            radial: if i % 4 == 0 { 1.2 } else { 0.0 },
        });
    }
    for &num in &spec.extra_tests {
        if defs.iter().any(|d| d.num == num) {
            continue;
        }
        let r = &mut rand::rngs::StdRng::seed_from_u64(num as u64);
        let mean: f64 = r.gen_range(0.5..50.0);
        let sigma = mean * r.gen_range(0.01..0.05);
        let k = r.gen_range(4.0..7.0);
        defs.push(TDef {
            num,
            name: format!("DUMMY_{num}"),
            units: "V",
            mean,
            sigma,
            lo: mean - k * sigma,
            hi: mean + k * sigma,
            load: 0.5,
            site_off: 0.0,
            radial: 0.0,
        });
    }
    defs.sort_by_key(|d| d.num);
    let n01 = Normal::new(0.0, 1.0).unwrap();
    let mut mrng = rand::rngs::StdRng::seed_from_u64(
        spec.seed ^ 0xABCD_1234 ^ spec.meas_seed.wrapping_mul(0x9E37_79B9_7F4A_7C15),
    );
    // separate stream so alarms do not change the generated values
    let mut arng = rand::rngs::StdRng::seed_from_u64(spec.seed ^ 0x5A5A_0001);
    let e = spec.endian;
    let mut w = StdfWriter::create(path, e)?;
    w.write(&Record::FAR(Far {
        cpu_type: e.cpu_type(),
        stdf_ver: 4,
    }))?;
    // a retest (meas_seed) starts later, so files sort by test time
    let t0 = 1_760_000_000u32 + spec.meas_seed as u32 * 7200;
    w.write(&Record::MIR(Box::new(Mir {
        setup_t: t0,
        start_t: t0,
        stat_num: 1,
        mode_cod: 'P',
        rtst_cod: ' ',
        prot_cod: ' ',
        cmod_cod: ' ',
        lot_id: spec.lot_id.clone(),
        part_typ: "CYPHER01".into(),
        node_nam: "TESTER01".into(),
        tstr_typ: "SYNTH".into(),
        job_nam: "CYPHER01_WS".into(),
        job_rev: "1.0".into(),
        sblot_id: format!("{}.1", spec.lot_id),
        test_cod: if spec.final_test {
            "FT".into()
        } else {
            "WS1".into()
        },
        tst_temp: "25".into(),
        exec_typ: "stdf-cypher-sample".into(),
        ..Default::default()
    })))?;
    w.write(&Record::SDR(Box::new(Sdr {
        head_num: 1,
        site_grp: 1,
        site_num: (1..=spec.sites).collect(),
        hand_typ: "PROBER".into(),
        card_id: "PC-01".into(),
        ..Default::default()
    })))?;
    let n_pins = 4u16;
    for p in 1..=n_pins {
        w.write(&Record::PMR(Pmr {
            pmr_indx: p,
            chan_typ: 0,
            chan_nam: format!("CH{p}"),
            phy_nam: format!("P{p}"),
            log_nam: format!("IO{p}"),
            head_num: 1,
            site_num: 1,
        }))?;
    }
    if !spec.final_test {
        w.write(&Record::WCR(Wcr {
            wafr_siz: 200.0,
            die_ht: 5.0,
            die_wid: 5.0,
            wf_units: 3,
            wf_flat: 'D',
            center_x: 0,
            center_y: 0,
            pos_x: 'R',
            pos_y: 'D',
        }))?;
    }

    let mut hcount: BTreeMap<(u8, u16), u32> = BTreeMap::new();
    let mut scount: BTreeMap<(u8, u16), u32> = BTreeMap::new();
    let mut site_cnt: BTreeMap<u8, (u32, u32)> = BTreeMap::new();
    let mut first_ptr = vec![true; defs.len()];
    let mut first_mpr = true;
    let mut first_log = true;
    let mut part_no = 0usize;
    let r = spec.radius;
    let n_wafers = if spec.final_test { 1 } else { spec.wafers };
    for wi in 0..n_wafers {
        let wafer_id = format!("{}-W{:02}", spec.lot_id, wi + 1);
        if !spec.final_test {
            w.write(&Record::WIR(Wir {
                head_num: 1,
                site_grp: 255,
                start_t: t0 + wi as u32 * 3600,
                wafer_id: wafer_id.clone(),
            }))?;
        }
        let mut dies = vec![];
        for y in -r..=r {
            for x in -r..=r {
                if (x as f64).powi(2) + (y as f64).powi(2) <= (r as f64 + 0.3).powi(2) {
                    dies.push((x, y));
                }
            }
        }
        if spec.final_test {
            let extra = dies.len() * (spec.wafers.saturating_sub(1));
            for i in 0..extra {
                dies.push((i as i16, 0));
            }
        }
        let wafer_off: f64 = n01.sample(&mut rng) * 0.3;
        let (mut wgood, mut wcnt) = (0u32, 0u32);
        for chunk in dies.chunks(spec.sites as usize) {
            // open all sites (multi-site touchdown)
            for (si, _) in chunk.iter().enumerate() {
                w.write(&Record::PIR(Pir {
                    head_num: 1,
                    site_num: si as u8 + 1,
                }))?;
            }
            let mut results = vec![];
            for (si, &(x, y)) in chunk.iter().enumerate() {
                let site = si as u8 + 1;
                let latent: f64 = n01.sample(&mut rng);
                let rad = if spec.final_test {
                    0.7
                } else {
                    ((x as f64).powi(2) + (y as f64).powi(2)).sqrt() / r as f64
                };
                let outlier = rng.gen_bool(spec.outlier_rate.clamp(0.0, 1.0));
                let out_test = rng.gen_range(0..defs.len());
                let mut fail_test: Option<usize> = None;
                let mut vals = vec![];
                for (ti, d) in defs.iter().enumerate() {
                    let mut z = d.load * latent
                        + (1.0 - d.load * d.load).max(0.0).sqrt()
                            * n01.sample(&mut rng)
                            * spec.noise;
                    z += d.site_off * ((site as f64 - 1.0) - (spec.sites as f64 - 1.0) / 2.0) * 0.5;
                    z += d.radial * (rad * rad - 0.5);
                    z += wafer_off + spec.shift * if ti % 2 == 0 { 1.0 } else { 0.2 };
                    if outlier && (ti == out_test || ti % 7 == out_test % 7) {
                        z += if rng.gen_bool(0.5) { 9.0 } else { -9.0 };
                    }
                    if spec.meas_noise > 0.0 {
                        z += spec.meas_noise * n01.sample(&mut mrng);
                    }
                    if ti % 3 == 2 {
                        z += spec.tester_offset;
                    }
                    let mut v = d.mean + z * d.sigma;
                    // which dies, per lot (seed) and test; the first test keeps the
                    // original salt
                    let lot = (spec.seed << 16)
                        + if spec.band_tests.first() == Some(&d.num) {
                            0
                        } else {
                            d.num as u64 * 100
                        };
                    if spec.latent_tests.contains(&d.num) && die_u(wi, x, y, lot + 1) < 0.11 {
                        // good for now, but just inside the high limit
                        v = d.hi - (d.hi - d.lo) * (0.02 + 0.08 * die_u(wi, x, y, lot + 5));
                    }
                    if spec.band_tests.contains(&d.num) {
                        // 4% marginal, 4% hard bad, 3% extreme (clamp)
                        // and whether a retest recovers them
                        let (u, w) = (die_u(wi, x, y, lot + 1), d.hi - d.lo);
                        let again = die_u(wi, x, y, lot + 2 + spec.meas_seed) < 0.85;
                        let retest = spec.meas_seed != 0;
                        if u < 0.04 && !(retest && again) {
                            v = d.hi + w * (0.02 + 0.08 * die_u(wi, x, y, lot + 3));
                        } else if (0.04..0.08).contains(&u) {
                            v = d.hi
                                + w * (0.3 + 0.4 * die_u(wi, x, y, lot + 4))
                                + z * d.sigma * 0.1;
                        } else if (0.08..0.11).contains(&u) && !(retest && again) {
                            v = d.hi + w * 3.0;
                        }
                    }
                    let f = limit_factor(spec, wi);
                    let fail = v < d.lo * f || v > d.hi * f;
                    if fail && fail_test.is_none() {
                        fail_test = Some(ti);
                    }
                    vals.push((v, fail));
                }
                results.push((site, x, y, vals, fail_test, latent));
            }
            // interleave test records site by site (as testers typically do)
            for (ti, d) in defs.iter().enumerate() {
                // stored = shown / 10^RES_SCAL (e.g. 12 uA logged as 1.2e-5 A, RES_SCAL 6)
                let (scale, base) = crate::analysis::candidates::unit_scale(d.units);
                let (scale, units, res_scal) = if spec.base_units && scale != 1.0 {
                    (scale, base, -(scale.log10().round() as i8))
                } else {
                    (1.0, d.units, 0)
                };
                for (site, _, _, vals, _, _) in &results {
                    let (v, fail) = vals[ti];
                    let v = v * scale;
                    let full = first_ptr[ti];
                    let with_lim = full || spec.dynamic_limits;
                    let f = limit_factor(spec, wi);
                    w.write(&Record::PTR(Ptr {
                        test_num: d.num,
                        head_num: 1,
                        site_num: *site,
                        test_flg: if fail { 0x80 } else { 0 }
                            | if arng.gen_bool(0.002) { 0x01 } else { 0 },
                        parm_flg: 0,
                        result: v as f32,
                        test_txt: d.name.clone(),
                        alarm_id: String::new(),
                        opt_flag: if with_lim { Some(0x0C) } else { None },
                        res_scal,
                        lo_limit: with_lim.then_some((d.lo * f * scale) as f32),
                        hi_limit: with_lim.then_some((d.hi * f * scale) as f32),
                        units: full.then(|| units.to_string()),
                        lo_spec: full.then_some(0.0),
                        hi_spec: full.then_some(0.0),
                        ..Default::default()
                    }))?;
                }
                first_ptr[ti] = false;
            }
            // MPR: 4-pin continuity
            for ri in 0..results.len() {
                let (site, latent) = (results[ri].0, results[ri].5);
                let site = &site;
                let rs: Vec<f32> = (0..n_pins)
                    .map(|p| {
                        (0.45 + 0.01 * p as f64 + 0.02 * latent + 0.005 * n01.sample(&mut rng))
                            as f32
                    })
                    .collect();
                let fail = rs.iter().any(|&v| !(0.2..=0.7).contains(&v));
                w.write(&Record::MPR(Mpr {
                    test_num: 100,
                    head_num: 1,
                    site_num: *site,
                    test_flg: if fail { 0x80 } else { 0 },
                    rtn_stat: vec![0; n_pins as usize],
                    rtn_rslt: rs,
                    test_txt: "CONTINUITY".into(),
                    opt_flag: first_mpr.then_some(0x0C),
                    lo_limit: first_mpr.then_some(0.2),
                    hi_limit: first_mpr.then_some(0.7),
                    rtn_indx: if first_mpr {
                        (1..=n_pins).collect()
                    } else {
                        vec![]
                    },
                    units: first_mpr.then(|| "V".to_string()),
                    lo_spec: first_mpr.then_some(0.0),
                    hi_spec: first_mpr.then_some(0.0),
                    ..Default::default()
                }))?;
                // functional test
                let ffail = rng.gen_bool(0.01);
                w.write(&Record::FTR(Box::new(Ftr {
                    test_num: 9000,
                    head_num: 1,
                    site_num: *site,
                    test_flg: if ffail { 0x80 } else { 0 },
                    test_txt: "FUNC_MAIN".into(),
                    vect_nam: "pat_main".into(),
                    ..Default::default()
                })))?;
                if ffail && results[ri].4.is_none() {
                    results[ri].4 = Some(usize::MAX);
                }
            }
            first_mpr = false;
            // traceability datalog: a unique wafer+X/Y code, then wafer and the die
            // position inside its 4x4 reticle plus the reticle offset (X = raw + offset)
            for (site, x, y, _, _, _) in &results {
                let code =
                    (wi as f64 + 1.0) * 10000.0 + (*x as f64 + 50.0) * 100.0 + (*y as f64 + 50.0);
                let (ox, oy) = (x.div_euclid(4) * 4, y.div_euclid(4) * 4);
                for (num, name, v) in [
                    (500, "DIE_ID", code),
                    (80100, "WAFER_NUM", wi as f64 + 1.0),
                    (80101, "DIE_X_RAW", (*x - ox) as f64),
                    (80102, "DIE_Y_RAW", (*y - oy) as f64),
                    (80103, "RETICLE_OX", ox as f64),
                    (80104, "RETICLE_OY", oy as f64),
                    (
                        5100,
                        "DIB_ID",
                        match spec.loadboard {
                            Some(b) if die_u(wi, *x, *y, 9) >= spec.loadboard_missing => b,
                            _ => f64::NAN,
                        },
                    ),
                ] {
                    if v.is_nan() {
                        continue;
                    }
                    w.write(&Record::PTR(Ptr {
                        test_num: num,
                        head_num: 1,
                        site_num: *site,
                        result: v as f32,
                        test_txt: name.into(),
                        opt_flag: first_log.then_some(0xCE),
                        units: first_log.then(String::new),
                        ..Default::default()
                    }))?;
                }
                first_log = false;
            }
            for (site, x, y, _, fail_test, _) in &results {
                part_no += 1;
                let (hb, sb) = match fail_test {
                    None => (1u16, 1u16),
                    Some(usize::MAX) => (6, 60),
                    Some(t) => {
                        let hb = 2 + (*t % 4) as u16;
                        (hb, hb * 10 + (*t % 3) as u16)
                    }
                };
                let failed = fail_test.is_some();
                *hcount.entry((*site, hb)).or_default() += 1;
                *scount.entry((*site, sb)).or_default() += 1;
                let e = site_cnt.entry(*site).or_default();
                e.0 += 1;
                wcnt += 1;
                if !failed {
                    e.1 += 1;
                    wgood += 1;
                }
                w.write(&Record::PRR(Prr {
                    head_num: 1,
                    site_num: *site,
                    part_flg: if failed { 0x08 } else { 0 },
                    num_test: defs.len() as u16 + 2,
                    hard_bin: hb,
                    soft_bin: sb,
                    x_coord: if spec.final_test { -32768 } else { *x },
                    y_coord: if spec.final_test { -32768 } else { *y },
                    test_t: 120 + rng.gen_range(0..30),
                    part_id: part_no.to_string(),
                    part_txt: format!("{}-W{:02}-X{}Y{}", spec.lot_id, wi + 1, x, y),
                    ..Default::default()
                }))?;
            }
        }
        if !spec.final_test {
            w.write(&Record::WRR(Wrr {
                head_num: 1,
                site_grp: 255,
                finish_t: t0 + wi as u32 * 3600 + 3000,
                part_cnt: wcnt,
                good_cnt: wgood,
                rtst_cnt: 0,
                abrt_cnt: 0,
                func_cnt: 0,
                wafer_id,
                ..Default::default()
            }))?;
        }
    }
    // summaries
    for (ti, d) in defs.iter().enumerate() {
        let _ = ti;
        w.write(&Record::TSR(Tsr {
            head_num: 255,
            site_num: 255,
            test_typ: 'P',
            test_num: d.num,
            test_nam: d.name.clone(),
            opt_flag: 0xFF,
            ..Default::default()
        }))?;
    }
    let bin_name = |hb: u16| match hb {
        1 => "PASS".to_string(),
        6 => "FUNC_FAIL".to_string(),
        b => format!("PARAM_FAIL_{b}"),
    };
    let mut htot: BTreeMap<u16, u32> = BTreeMap::new();
    for (&(site, hb), &c) in &hcount {
        *htot.entry(hb).or_default() += c;
        w.write(&Record::HBR(Bin {
            head_num: 1,
            site_num: site,
            bin_num: hb,
            bin_cnt: c,
            bin_pf: if hb == 1 { 'P' } else { 'F' },
            bin_nam: bin_name(hb),
        }))?;
    }
    for (hb, c) in htot {
        w.write(&Record::HBR(Bin {
            head_num: 255,
            site_num: 255,
            bin_num: hb,
            bin_cnt: c,
            bin_pf: if hb == 1 { 'P' } else { 'F' },
            bin_nam: bin_name(hb),
        }))?;
    }
    let mut stot: BTreeMap<u16, u32> = BTreeMap::new();
    for (&(_, sb), &c) in &scount {
        *stot.entry(sb).or_default() += c;
    }
    for (sb, c) in stot {
        w.write(&Record::SBR(Bin {
            head_num: 255,
            site_num: 255,
            bin_num: sb,
            bin_cnt: c,
            bin_pf: if sb == 1 { 'P' } else { 'F' },
            bin_nam: format!("SB{sb}"),
        }))?;
    }
    for (&site, &(cnt, good)) in &site_cnt {
        w.write(&Record::PCR(Pcr {
            head_num: 1,
            site_num: site,
            part_cnt: cnt,
            good_cnt: good,
            rtst_cnt: 0,
            abrt_cnt: 0,
            func_cnt: 0,
        }))?;
    }
    w.write(&Record::MRR(Mrr {
        finish_t: t0 + 86400,
        disp_cod: ' ',
        ..Default::default()
    }))?;
    w.finish()?;
    Ok(part_no)
}
