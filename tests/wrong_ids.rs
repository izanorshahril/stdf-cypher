//! Part trace on real STDF files with wrong IDs planted: two files of one lot, where the second has
//! part texts recorded on the wrong parts (a handler out of step), wafer, X, Y read from a corrupt
//! memory (all zeros, all ones), a single WXY test that reads wrong, and one wafer, X, Y copied onto
//! another part. Run 1 is the lot as tested; run 2 retests every part.

use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::Path;
use stdf_cypher::api;
use stdf_cypher::stdf::{Record, StdfReader, StdfWriter};

const W: u32 = 80100;
const X: u32 = 80101;
const Y: u32 = 80102;
const OX: u32 = 80103;
const OY: u32 = 80104;
const WXY: u32 = 80149;

/// Copies `src` to `dst`, one part at a time (its PIR … PRR records, by site): `edit` gets the part's
/// number in the file and its records, and may change them. Every part gets a WXY test (80149):
/// wafer, X, Y as logged in its tests, as one code.
fn rewrite(src: &Path, dst: &Path, mut edit: impl FnMut(usize, &mut Vec<Record>)) {
    let mut r = StdfReader::open(src).unwrap();
    let e = r.endian;
    let mut w = StdfWriter::create(dst, e).unwrap();
    let mut open: HashMap<(u8, u8), Vec<Record>> = HashMap::new();
    let mut n = 0usize;
    let mut raw = vec![];
    while let Some(h) = r.next_raw_full(&mut raw).unwrap() {
        let rec = Record::parse(h.typ, h.sub, &raw[4..], e);
        let site = match &rec {
            Record::PIR(p) => Some((p.head_num, p.site_num)),
            Record::PTR(p) => Some((p.head_num, p.site_num)),
            Record::PRR(p) => Some((p.head_num, p.site_num)),
            _ => None,
        };
        match (site, &rec) {
            (Some(s), Record::PIR(_)) => {
                open.insert(s, vec![rec]);
            }
            (Some(s), Record::PRR(_)) => {
                let mut part = open.remove(&s).unwrap_or_default();
                let v = |part: &[Record], t: u32| {
                    part.iter()
                        .find_map(|x| match x {
                            Record::PTR(p) if p.test_num == t => Some(p.result),
                            _ => None,
                        })
                        .unwrap_or(f32::NAN)
                };
                let code = v(&part, W) * 1e6 + v(&part, X) * 1e3 + v(&part, Y);
                if let Some(Record::PTR(model)) = part.iter().find(|x| matches!(x, Record::PTR(p) if p.test_num == W)) {
                    let mut p = model.clone();
                    p.test_num = WXY;
                    p.test_txt = "ChipID_WXY".into();
                    p.result = code;
                    p.lo_limit = Some(0.0);
                    p.hi_limit = Some(1e8);
                    part.push(Record::PTR(p));
                }
                part.push(rec);
                edit(n, &mut part);
                n += 1;
                for x in &part {
                    w.write(x).unwrap();
                }
            }
            (Some(s), _) => open.entry(s).or_default().push(rec),
            _ => w.write(&rec).unwrap(),
        }
    }
}

fn set(part: &mut [Record], t: u32, v: f32) {
    for x in part.iter_mut() {
        if let Record::PTR(p) = x {
            if p.test_num == t {
                p.result = v;
            }
        }
    }
}

fn get(part: &[Record], t: u32) -> f32 {
    part.iter()
        .find_map(|x| match x {
            Record::PTR(p) if p.test_num == t => Some(p.result),
            _ => None,
        })
        .unwrap()
}

fn part_txt(part: &[Record]) -> String {
    part.iter()
        .find_map(|x| match x {
            Record::PRR(p) => Some(p.part_txt.clone()),
            _ => None,
        })
        .unwrap()
}

fn set_part_txt(part: &mut [Record], t: &str) {
    for x in part.iter_mut() {
        if let Record::PRR(p) = x {
            p.part_txt = t.to_string();
        }
    }
}

#[test]
fn wrong_ids_are_traced_by_the_trusted_id_and_never_merge_different_parts() {
    std::env::set_var("STDF_CYPHER_HOME", tempfile::tempdir().unwrap().keep());
    let d = tempfile::tempdir().unwrap();
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("synthetic_samples/tc_profile/demo/synthetic_demo_lotA.stdf");
    let run1 = d.path().join("run1.stdf");
    let run2 = d.path().join("run2.stdf");
    // run 1 as tested, keeping each part's part text and wafer, X, Y
    let mut txt: Vec<String> = vec![];
    rewrite(&src, &run1, |_, p| txt.push(part_txt(p)));
    // run 2: the same parts, with wrong IDs planted
    let shifted = 10..20; // part text of the next part (the handler a part ahead)
    let zeros = 30..33; // wafer, X, Y all zeros (corrupt memory)
    let ones = 33..36; // wafer, X, Y all ones (255 in every byte)
    let wrong_wxy = 40..50; // the single WXY test reads one more
    let (copied, from) = (60, 62); // part 60's wafer, X, Y copied from part 62
    let mut wxy_of: HashMap<usize, (f32, f32)> = HashMap::new();
    let mut die62 = [0f32; 5];
    rewrite(&run1.clone(), &d.path().join("tmp.stdf"), |n, p| {
        if n == from {
            die62 = [W, X, Y, OX, OY].map(|t| get(p, t));
        }
    });
    rewrite(&src, &run2, |n, p| {
        if shifted.contains(&n) {
            set_part_txt(p, &txt[n + 1]);
        }
        if zeros.contains(&n) || ones.contains(&n) {
            let v = if zeros.contains(&n) { 0.0 } else { 255.0 };
            for t in [W, X, Y] {
                set(p, t, v);
            }
            set(p, OX, 0.0);
            set(p, OY, 0.0);
            set(p, WXY, if zeros.contains(&n) { 0.0 } else { 255255255.0 });
        }
        if wrong_wxy.contains(&n) {
            let v = get(p, WXY);
            set(p, WXY, v + 1.0);
            wxy_of.insert(n, (v, v + 1.0));
        }
        if n == copied {
            for (t, v) in [W, X, Y, OX, OY].into_iter().zip(die62) {
                set(p, t, v);
            }
        }
    });

    let reg = api::Registry::app();
    let paths = [&run1, &run2].map(|p| p.to_string_lossy().to_string());
    api::run(&reg, "load", &json!({"paths": paths, "sort": "none"})).unwrap();
    let keys = json!(["w_x_y:80100,80101,80102,80103,80104", "part_txt", "tests:80149"]);
    let links = json!([true, true, false]);
    let runs = json!([{"ds": 0, "name": "run1"}, {"ds": 1, "name": "run2"}]);
    let v: Value = api::run(&reg, "part_trace_view", &json!({"runs": runs, "keys": keys, "links": links})).unwrap();
    let parts = v["parts"].as_array().unwrap();
    let n1 = txt.len();
    let in_both = |p: &Value| !p["runs"][0].is_null() && !p["runs"][1].is_null();
    let row_of = |p: &Value, k: usize| p["runs"][k]["row"].as_u64().map(|r| r as usize);
    let by_row1: HashMap<usize, &Value> = parts.iter().filter_map(|p| Some((row_of(p, 0)?, p))).collect();
    let report = |label: &str, rows: std::ops::Range<usize>| {
        for r in rows {
            let p = by_row1[&r];
            eprintln!("{label} part {r}: run2 row {:?}, ids {}, note {}", row_of(p, 1), p["ids"], p["note"]);
        }
    };
    report("shifted", shifted.clone());
    report("zeros", zeros.clone());
    report("ones", ones.clone());
    report("wrong wxy", wrong_wxy.clone());
    report("copied", copied..copied + 1);
    report("source of the copy", from..from + 1);

    // every part of run 1 is a part of the trace, and no two run-1 parts were merged
    assert_eq!(by_row1.len(), n1, "every run-1 part has its own row");
    // a part text on the wrong part: traced by wafer, X, Y to its own die, and marked
    for r in shifted.clone() {
        let p = by_row1[&r];
        assert_eq!(row_of(p, 1), Some(r), "shifted part {r} found by its wafer, X, Y");
        // marked: matched by W,X,Y over its part text, or (the last one, whose borrowed part text
        // is also on the part it came from) that part text is on two parts of run 2
        let note = p["note"].as_str().unwrap_or("");
        assert!(note.contains("matched by W,X,Y") || note.contains("on several parts"), "part {r} marked: {note}");
    }
    // corrupt wafer, X, Y (all zeros, all ones): found by part text instead, nothing merged
    for r in zeros.clone().chain(ones.clone()) {
        let p = by_row1[&r];
        assert_eq!(row_of(p, 1), Some(r), "corrupt part {r} found by its part text");
    }
    // a wrong single WXY changes nothing but its column, which shows both values
    for r in wrong_wxy.clone() {
        let p = by_row1[&r];
        assert_eq!(row_of(p, 1), Some(r));
        assert!(p["note"].is_null(), "a column-only ID never flags a part");
        let (a, b) = wxy_of[&r];
        let shown = p["ids"][2].as_str().unwrap();
        assert_eq!(shown, format!("{a} / {b}"));
    }
    // a wafer, X, Y copied onto another part is on two parts of run 2: it matches neither, and
    // both parts are found by their part text, marked
    for r in [copied, from] {
        let p = by_row1[&r];
        assert_eq!(row_of(p, 1), Some(r), "part {r} keeps its own run-2 test");
        assert!(p["note"].as_str().unwrap_or("").contains("on several parts"), "part {r} marked");
    }
    // untouched parts: one row each, found in both runs
    let clean = (0..n1).filter(|r| ![shifted.clone(), zeros.clone(), ones.clone(), wrong_wxy.clone()].iter().any(|x| x.contains(r)) && *r != copied && *r != from);
    for r in clean {
        let p = by_row1[&r];
        assert!(in_both(p) && row_of(p, 1) == Some(r), "clean part {r}");
    }
    let joined = parts.iter().filter(|p| p["note"].as_str().is_some_and(|n| n.contains("matched by"))).count();
    eprintln!("parts {}, conflicts {}, joined notes {joined}", parts.len(), v["conflicts"]);
}
