//! Trim: picking parts by ID or row, keeping or removing them, backfilled IDs,
//! cut-off parts, datalog that follows its part, and the API ops behind the view.

use serde_json::json;
use std::path::{Path, PathBuf};
use stdf_cypher::api;
use stdf_cypher::data::{self, Dataset, LoadOptions};
use stdf_cypher::sample::{generate, SampleSpec};
use stdf_cypher::trim::{self, Action, By};

/// Keep tests away from the user's real settings/cache folders.
fn isolate() {
    static HOME: std::sync::OnceLock<tempfile::TempDir> = std::sync::OnceLock::new();
    let h = HOME.get_or_init(|| tempfile::tempdir().unwrap());
    std::env::set_var("STDF_CYPHER_HOME", h.path());
}

fn gen(dir: &Path, name: &str, spec: SampleSpec) -> PathBuf {
    let p = dir.join(name);
    generate(&p, &spec).unwrap();
    p
}

fn load(p: &Path) -> Dataset {
    data::load(p, &LoadOptions::default()).unwrap()
}

fn part_ids(ds: &Dataset) -> Vec<String> {
    ds.parts.iter().map(|p| p.part_id.clone()).collect()
}

/// Same results, bit for bit (NaN equal to NaN): row `a` of `x` against row `b` of `y`.
fn same_row(x: &Dataset, a: usize, y: &Dataset, b: usize) {
    assert_eq!(x.n_tests(), y.n_tests());
    for t in 0..x.n_tests() {
        assert_eq!(
            x.values[t][a].to_bits(),
            y.values[t][b].to_bits(),
            "test {t}, row {a} against {b}"
        );
    }
    assert_eq!(x.parts[a].hbin, y.parts[b].hbin);
    assert_eq!(x.parts[a].part_txt, y.parts[b].part_txt);
}

/// Byte offset just after the first PRR of a little-endian file.
fn first_prr_end(data: &[u8]) -> usize {
    let mut i = 0;
    loop {
        let len = u16::from_le_bytes([data[i], data[i + 1]]) as usize;
        if (data[i + 2], data[i + 3]) == (5, 20) {
            return i + 4 + len;
        }
        i += 4 + len;
    }
}

/// Copy of `src` without its last PRR: the last part is cut off.
fn without_last_prr(src: &Path, dst: &Path) {
    let data = std::fs::read(src).unwrap();
    let mut spans = vec![];
    let mut i = 0;
    while i + 4 <= data.len() {
        let len = u16::from_le_bytes([data[i], data[i + 1]]) as usize;
        spans.push((data[i + 2], data[i + 3], i, i + 4 + len));
        i += 4 + len;
    }
    let last = spans.iter().rposition(|s| (s.0, s.1) == (5, 20)).unwrap();
    let mut out = Vec::with_capacity(data.len());
    for (k, s) in spans.iter().enumerate() {
        if k != last {
            out.extend_from_slice(&data[s.2..s.3]);
        }
    }
    std::fs::write(dst, out).unwrap();
}

#[test]
fn keep_and_remove_pick_by_id_and_backfill_the_ids() {
    isolate();
    let d = tempfile::tempdir().unwrap();
    let src = gen(
        d.path(),
        "src.stdf",
        SampleSpec {
            wafers: 1,
            ..Default::default()
        },
    );
    let ds = load(&src);
    let n = ds.n_parts();
    assert!(n > 6, "the sample has enough parts");
    assert_eq!(
        part_ids(&ds),
        (1..=n).map(|i| i.to_string()).collect::<Vec<_>>()
    );

    let picked = trim::resolve(&ds, By::Id, "1-3,5").unwrap();
    assert_eq!(picked.rows, vec![0, 1, 2, 4]);
    assert!(picked.missing.is_empty() && picked.shared.is_empty());

    // keep them: their IDs stay as they are
    let out = d.path().join("kept.stdf");
    let keep = trim::rows_to_keep(n, &picked.rows, Action::Keep);
    let res = trim::write(&src, &ds, &keep, false, &out).unwrap();
    assert_eq!((res.kept, res.removed), (4, (n - 4) as u64));
    assert_eq!((res.first_id.as_str(), res.last_id.as_str()), ("1", "5"));
    let k = load(&out);
    assert_eq!(part_ids(&k), vec!["1", "2", "3", "5"]);
    for (i, &row) in keep.iter().enumerate() {
        same_row(&k, i, &ds, row);
    }

    // remove the same parts and backfill: the rest runs 1…N
    let rest = trim::rows_to_keep(n, &picked.rows, Action::Remove);
    let out2 = d.path().join("rest.stdf");
    let res2 = trim::write(&src, &ds, &rest, true, &out2).unwrap();
    assert_eq!(res2.kept as usize, rest.len());
    let r = load(&out2);
    assert_eq!(
        part_ids(&r),
        (1..=rest.len()).map(|i| i.to_string()).collect::<Vec<_>>()
    );
    for (i, &row) in rest.iter().enumerate() {
        same_row(&r, i, &ds, row);
    }
}

#[test]
fn selections_report_missing_numbers_and_refuse_bad_input() {
    isolate();
    let d = tempfile::tempdir().unwrap();
    let src = gen(
        d.path(),
        "src.stdf",
        SampleSpec {
            wafers: 1,
            ..Default::default()
        },
    );
    let ds = load(&src);
    let n = ds.n_parts();

    let by_id = trim::resolve(&ds, By::Id, &format!("2,{}-{}", n + 1, n + 5)).unwrap();
    assert_eq!(by_id.rows, vec![1]);
    assert_eq!(by_id.missing, vec![format!("{}-{}", n + 1, n + 5)]);

    let by_row = trim::resolve(&ds, By::Row, &format!("1,{n}-{}", n + 2)).unwrap();
    assert_eq!(by_row.rows, vec![0, n - 1]);
    assert_eq!(
        by_row.missing,
        vec![format!("{n}-{} (2 outside 1-{n})", n + 2)]
    );

    assert!(
        trim::resolve(&ds, By::Id, "5-3").is_err(),
        "a range that ends first"
    );
    assert!(trim::resolve(&ds, By::Row, "A7").is_err(), "text is no row");
    assert!(
        trim::write(&src, &ds, &[0], false, &src).is_err(),
        "never over the source"
    );
    assert!(trim::write(&src, &ds, &[], false, &d.path().join("x.stdf")).is_err());
    assert!(!d.path().join("x.stdf").exists());
}

#[test]
fn a_part_cut_off_before_its_prr_is_kept_and_gzip_output_reads_back() {
    isolate();
    let d = tempfile::tempdir().unwrap();
    let full = gen(d.path(), "full.stdf", SampleSpec::default());
    let cut = d.path().join("cut.stdf");
    without_last_prr(&full, &cut);
    let ds = load(&cut);
    let last = ds.n_parts() - 1;
    assert!(ds.parts[last].partial);

    let out = d.path().join("cut_only.stdf.gz");
    let res = trim::write(&cut, &ds, &[last], false, &out).unwrap();
    assert_eq!(res.kept, 1);
    let back = load(&out);
    assert_eq!(back.n_parts(), 1);
    assert!(
        back.parts[0].partial,
        "still cut off, without bin or verdict"
    );
    same_row(&back, 0, &ds, last);
    assert!(
        std::fs::read_dir(d.path()).unwrap().all(|e| !e
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with('.')),
        "no temporary file is left behind"
    );
}

#[test]
fn datalog_follows_its_part() {
    isolate();
    let d = tempfile::tempdir().unwrap();
    let src = gen(
        d.path(),
        "dl.stdf",
        SampleSpec {
            wafers: 1,
            ..Default::default()
        },
    );
    // a DTR right after the first PRR: it belongs to part 1
    let text = b"part 1 note";
    let mut raw = std::fs::read(&src).unwrap();
    let at = first_prr_end(&raw);
    let mut dtr = ((1 + text.len()) as u16).to_le_bytes().to_vec();
    dtr.extend_from_slice(&[50, 30, text.len() as u8]);
    dtr.extend_from_slice(text);
    raw.splice(at..at, dtr);
    std::fs::write(&src, raw).unwrap();
    let ds = load(&src);
    assert_eq!(ds.datalog.len(), 1);
    assert_eq!(ds.datalog[0].after_part, 1);

    // part 1 removed: its note goes too
    let rest = trim::rows_to_keep(ds.n_parts(), &[0], Action::Remove);
    let out = d.path().join("rest.stdf");
    trim::write(&src, &ds, &rest, false, &out).unwrap();
    assert!(load(&out).datalog.is_empty());

    // part 1 kept: the note comes along
    let out1 = d.path().join("one.stdf");
    trim::write(&src, &ds, &[0], false, &out1).unwrap();
    assert_eq!(load(&out1).datalog.len(), 1);
}

#[test]
fn the_api_lists_selects_and_saves() {
    isolate();
    let d = tempfile::tempdir().unwrap();
    let p = gen(
        d.path(),
        "api.stdf",
        SampleSpec {
            wafers: 1,
            ..Default::default()
        },
    );
    let reg = api::Registry::app();
    api::run(&reg, "load", &json!({ "paths": [p.to_string_lossy()] })).unwrap();

    let list = api::run(&reg, "trim_list", &json!({ "ds": 0 })).unwrap();
    let n = list["n"].as_u64().unwrap() as usize;
    assert_eq!(list["rows"].as_array().unwrap().len(), n);
    assert_eq!(list["editable"], true);
    assert_eq!(list["rows"][0][1], "1", "index, part ID, ...");

    let sel = api::run(&reg, "trim_resolve", &json!({ "ds": 0, "text": "2-4" })).unwrap();
    assert_eq!(sel["count"], 3);
    let both = api::run(
        &reg,
        "trim_resolve",
        &json!({ "ds": 0, "text": "2", "filter": "part_id in (2, 3)" }),
    )
    .unwrap();
    assert_eq!(both["count"], 2, "the filter adds to the numbers");
    assert!(api::run(&reg, "trim_resolve", &json!({ "ds": 0 })).is_err());

    let out = d.path().join("picked.stdf");
    let saved = api::run(
        &reg,
        "trim_save",
        &json!({ "ds": 0, "rows": [1, 2], "action": "keep", "renumber": true, "path": out.to_string_lossy() }),
    )
    .unwrap();
    assert_eq!(saved["kept"], 2);
    assert_eq!(saved["first_id"], "1");
    assert_eq!(part_ids(&load(&out)), vec!["1", "2"]);

    assert!(api::run(
        &reg,
        "trim_save",
        &json!({ "ds": 0, "rows": [1], "path": p.to_string_lossy() }),
    )
    .is_err());
}
