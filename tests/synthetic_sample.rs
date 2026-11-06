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
            Record::ATR(_) => "ATR",
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
    // V4: FAR, then any ATRs, then MIR; MRR last
    let mir_at = kinds.iter().position(|k| *k != "FAR" && *k != "ATR");
    if kinds.first() != Some(&"FAR") || mir_at.map(|i| kinds[i]) != Some("MIR") || kinds.last() != Some(&"MRR") {
        out.push("FAR, ATRs, MIR first and MRR last".into());
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

/// The record types of a file, by name.
fn record_kinds(path: &Path) -> BTreeSet<&'static str> {
    let mut rdr = StdfReader::open(path).unwrap();
    let mut out = BTreeSet::new();
    while let Some((h, _)) = rdr.next_record().unwrap() {
        out.insert(stdf_cypher::stdf::record_name(h.typ, h.sub));
    }
    out
}

#[test]
fn tc_files_hold_every_stdf_v4_record_type() {
    let d = tempfile::tempdir().unwrap();
    let (ws, rt) = (d.path().join("ws.stdf"), d.path().join("rt.stdf"));
    let tc = SampleSpec {
        profile: Profile::Tc,
        ..Default::default()
    };
    generate(&ws, &tc).unwrap();
    let n = generate(
        &rt,
        &SampleSpec {
            retest_bins: vec![2, 3],
            meas_seed: 1,
            ..tc.clone()
        },
    )
    .unwrap();
    let all = [
        "FAR", "ATR", "MIR", "MRR", "PCR", "HBR", "SBR", "PMR", "PGR", "PLR", "RDR", "SDR", "WIR",
        "WRR", "WCR", "PIR", "PRR", "TSR", "PTR", "MPR", "FTR", "BPS", "EPS", "GDR", "DTR",
    ];
    let mut got = record_kinds(&ws);
    got.extend(record_kinds(&rt));
    let missing: Vec<&str> = all.iter().copied().filter(|k| !got.contains(k)).collect();
    assert!(missing.is_empty(), "missing {missing:?}");
    assert!(record_kinds(&rt).contains("RDR"));
    // the retest holds only parts the first run put in bins 2 and 3, and still follows the profile
    use stdf_cypher::data::{load, LoadOptions};
    let first = load(&ws, &LoadOptions::default()).unwrap();
    let want = first.parts.iter().filter(|p| [2, 3].contains(&p.hbin)).count();
    assert_eq!(n, want);
    assert_eq!(tc_problems(&rt), Vec::<String>::new());
    // and keeps the first run's part IDs
    let retest = load(&rt, &LoadOptions::default()).unwrap();
    let ids: BTreeSet<&str> = first
        .parts
        .iter()
        .filter(|p| [2, 3].contains(&p.hbin))
        .map(|p| p.part_id.as_str())
        .collect();
    assert!(retest.parts.iter().all(|p| ids.contains(p.part_id.as_str())));
}

#[test]
fn instrument_alarms_in_gdr_and_dtr_are_decoded_and_tied_to_their_part() {
    use stdf_cypher::data::{load, LoadOptions};
    let d = tempfile::tempdir().unwrap();
    let ws = d.path().join("ws.stdf");
    generate(
        &ws,
        &SampleSpec {
            profile: Profile::Tc,
            ..Default::default()
        },
    )
    .unwrap();
    let ds = load(&ws, &LoadOptions::default()).unwrap();
    let alarms: Vec<_> = ds.datalog.iter().filter_map(|e| e.alarm.as_ref().map(|a| (e, a))).collect();
    assert!(alarms.len() > 10, "{} alarms", alarms.len());
    // all three styles: DTR pairs, GDR fields, DTR free text
    assert!(alarms.iter().any(|(e, _)| e.kind == "GDR"));
    assert!(alarms.iter().any(|(e, _)| e.kind == "DTR" && e.text.starts_with("INST_ALARM")));
    assert!(alarms.iter().any(|(e, _)| e.kind == "DTR" && e.text.contains("instrument alarm on site")));
    // the free-text style reads the same fields
    let free = alarms.iter().find(|(e, _)| e.text.contains("resource")).unwrap().1;
    assert!(["PPMU", "DPS", "DCVI", "DIGITAL"].iter().any(|i| free.instrument.starts_with(i)), "{free:?}");
    // the other datalog records are not alarms
    assert!(ds.datalog.iter().any(|e| e.text.starts_with("LOT START") && e.alarm.is_none()));
    assert!(ds.datalog.iter().any(|e| e.text.starts_with("MIR_ADD; GUI_NAME=") && e.alarm.is_none()));
    for (e, a) in &alarms {
        let row = a.row.unwrap_or_else(|| panic!("no part for {}", e.text));
        let p = &ds.parts[row];
        assert_eq!(Some(p.site), a.site, "{}", e.text);
        assert!(!a.instrument.is_empty() && !a.code.is_empty() && !a.severity.is_empty(), "{a:?}");
        // the PTR of that part and test carries the alarm flag
        let t = ds.tests.iter().position(|t| Some(t.num) == a.test).unwrap();
        assert!(p.alarms.contains(&(t as u32)), "{}", e.text);
        assert!(p.inst_alarms > 0);
    }
    let flagged = ds.parts.iter().filter(|p| p.inst_alarms > 0).count();
    let rows: BTreeSet<usize> = alarms.iter().filter_map(|(_, a)| a.row).collect();
    assert_eq!(flagged, rows.len());
    // and the filter field finds those parts
    let f = stdf_cypher::data::Filter::parse("inst_alarm", &ds).unwrap();
    assert_eq!(f.select(&ds, &ds.all_rows()).len(), flagged);
}