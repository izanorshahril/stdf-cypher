//! The synthetic files written with the TC profile follow its record rules.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use stdf_cypher::stdf::codec::GenData;
use stdf_cypher::stdf::{Record, StdfReader};
use stdf_cypher::synthetic_sample::{
    generate, touchdown_file, Profile, SampleSpec, TouchSpec,
};

/// The base units a TC-profile file may log: no SI prefix.
const BASE_UNITS: &[&str] = &["", "V", "A", "Hz", "s", "ohm", "%", "dB", "code"];

/// Checks one file against the TC profile's record rules and returns the problems found.
fn tc_problems(path: &Path) -> Vec<String> {
    let mut rdr = StdfReader::open(path).unwrap();
    let mut kinds: Vec<&'static str> = vec![];
    let mut sites: BTreeSet<u8> = BTreeSet::new();
    let mut pcr: BTreeSet<u8> = BTreeSet::new();
    let mut hbr: BTreeSet<u8> = BTreeSet::new();
    let mut sbr: BTreeSet<u8> = BTreeSet::new();
    let mut tsr: BTreeSet<(u32, u8)> = BTreeSet::new();
    let mut tests: BTreeMap<u32, BTreeSet<String>> = BTreeMap::new();
    let mut gdrs: Vec<String> = vec![];
    let (mut pir, mut prr, mut wir, mut wrr) = (0, 0, 0, 0);
    let mut out = vec![];
    let (mut hsum, mut pcr_all) = (0u32, None);
    while let Some((_, rec)) = rdr.next_record().unwrap() {
        let kind = match &rec {
            Record::FAR(_) => "FAR",
            Record::MIR(_) => "MIR",
            Record::MRR(_) => "MRR",
            Record::SDR(_) => "SDR",
            Record::PMR(_) => "PMR",
            Record::PGR(_) => "PGR",
            Record::PLR(_) => "PLR",
            Record::MPR(_) => "MPR",
            Record::WCR(_) => "WCR",
            _ => "",
        };
        kinds.push(kind);
        match rec {
            Record::PIR(_) => pir += 1,
            Record::WIR(_) => wir += 1,
            Record::WRR(_) => wrr += 1,
            Record::PRR(p) => {
                prr += 1;
                sites.insert(p.site_num);
                let ok = !p.part_txt.is_empty()
                    && p.part_txt.split(';').all(|e| e.split_once('_').is_some_and(|(n, v)| !n.is_empty() && !v.is_empty()));
                if !ok {
                    out.push(format!("part text is not name_value;...: {}", p.part_txt));
                }
            }
            Record::PCR(p) => {
                pcr.insert(p.site_num);
                if p.site_num == 255 {
                    pcr_all = Some(p.part_cnt);
                }
            }
            Record::HBR(b) => {
                hbr.insert(b.site_num);
                if b.site_num == 255 {
                    hsum += b.bin_cnt;
                }
            }
            Record::SBR(b) => {
                sbr.insert(b.site_num);
            }
            Record::TSR(t) => {
                tsr.insert((t.test_num, t.site_num));
            }
            Record::PTR(p) => {
                if !p.test_txt.is_empty() {
                    tests.entry(p.test_num).or_default().insert(p.test_txt.clone());
                }
                tests.entry(p.test_num).or_default();
                if let Some(u) = &p.units {
                    if !BASE_UNITS.contains(&u.as_str()) {
                        out.push(format!("test {} logs unit {u}, not a base unit", p.test_num));
                    }
                }
            }
            Record::MPR(p) => {
                tests.entry(p.test_num).or_default().insert(p.test_txt.clone());
            }
            Record::FTR(f) => {
                tests.entry(f.test_num).or_default().insert(f.test_txt.clone());
            }
            Record::GDR(g) => {
                if let Some(GenData::Cn(name)) = g.gen_data.first() {
                    gdrs.push(name.clone());
                }
            }
            _ => {}
        }
    }
    let has = |k: &str| kinds.contains(&k);
    if kinds.first() != Some(&"FAR") || kinds.get(1) != Some(&"MIR") || kinds.last() != Some(&"MRR") {
        out.push("FAR, MIR first and MRR last".into());
    }
    if !has("SDR") {
        out.push("no SDR".into());
    }
    if has("MPR") && !(has("PMR") && has("PGR") && has("PLR")) {
        out.push("MPR without PMR, PGR and PLR".into());
    }
    if pir != prr {
        out.push(format!("{pir} PIR but {prr} PRR"));
    }
    if wir != wrr || (wir > 0 && !has("WCR")) {
        out.push("WIR, WRR and WCR do not make one set per wafer".into());
    }
    let count = |n: &str| gdrs.iter().filter(|g| *g == n).count();
    for name in ["MIR_ADD", "AUTOMATION"] {
        if count(name) != 1 {
            out.push(format!("GDR {name}: {} records", count(name)));
        }
    }
    if count("REF_DIE") != wir {
        out.push(format!("GDR REF_DIE: {} records for {wir} wafers", count("REF_DIE")));
    }
    let all: BTreeSet<u8> = sites.iter().copied().chain([255]).collect();
    for (name, got) in [("PCR", &pcr), ("HBR", &hbr), ("SBR", &sbr)] {
        if &all != got {
            out.push(format!("{name} sites {got:?}, want {all:?}"));
        }
    }
    for (&num, names) in &tests {
        if num == 0 {
            out.push("test number 0".into());
        }
        if names.len() != 1 {
            out.push(format!("test {num} has names {names:?}"));
        }
        for &s in &all {
            if !tsr.contains(&(num, s)) {
                out.push(format!("no TSR for test {num} on site {s}"));
            }
        }
    }
    if pcr_all != Some(prr) || hsum != prr {
        out.push(format!("{prr} parts, PCR total {pcr_all:?}, HBR total {hsum}"));
    }
    out
}

#[test]
fn tc_profile_files_follow_the_profile() {
    let d = tempfile::tempdir().unwrap();
    let ws = d.path().join("ws.stdf");
    let ft = d.path().join("ft.stdf");
    let lp = d.path().join("loop.stdf");
    let tc = SampleSpec {
        profile: Profile::Tc,
        ..Default::default()
    };
    generate(&ws, &tc).unwrap();
    generate(
        &ft,
        &SampleSpec {
            final_test: true,
            ..tc.clone()
        },
    )
    .unwrap();
    touchdown_file(
        &lp,
        &TouchSpec {
            sites: 4,
            touchdowns: 30,
            dut_pool: 4,
            profile: Profile::Tc,
            ..Default::default()
        },
    )
    .unwrap();
    for p in [&ws, &ft, &lp] {
        assert_eq!(tc_problems(p), Vec::<String>::new(), "{}", p.display());
    }
    // a plain file is not held to the profile
    let plain = d.path().join("plain.stdf");
    generate(&plain, &SampleSpec::default()).unwrap();
    assert!(!tc_problems(&plain).is_empty());
}

#[test]
fn tc_profile_logs_base_units_with_the_same_values() {
    use stdf_cypher::data::{load, LoadOptions};
    let d = tempfile::tempdir().unwrap();
    let (plain, tc) = (d.path().join("plain.stdf"), d.path().join("tc.stdf"));
    let spec = TouchSpec {
        sites: 2,
        touchdowns: 10,
        dut_pool: 2,
        ..Default::default()
    };
    touchdown_file(&plain, &spec).unwrap();
    touchdown_file(
        &tc,
        &TouchSpec {
            profile: Profile::Tc,
            ..spec
        },
    )
    .unwrap();
    let a = load(&plain, &LoadOptions::default()).unwrap();
    let b = load(&tc, &LoadOptions::default()).unwrap();
    // the same draws: IDD logged in uA in one file, in A with RES_SCAL 6 in the other
    let ia = a.find_test("2000").unwrap();
    let ib = b.find_test("2000").unwrap();
    assert_eq!(a.n_parts(), b.n_parts());
    let (va, vb) = (a.value(ia, 0), b.value(ib, 0));
    assert!(((vb * 1e6) - va).abs() < 1e-3 * va.abs().max(1.0), "{va} vs {vb}");
}
