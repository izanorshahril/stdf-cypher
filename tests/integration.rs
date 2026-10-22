//! End-to-end tests on synthetic STDF files with known, injected effects.

use serde_json::json;
use std::path::{Path, PathBuf};
use stdf_cypher::analysis::{anomaly, anova, compare, correlation};
use stdf_cypher::data::{self, Dataset, Filter, LoadOptions};
use stdf_cypher::sample::{generate, SampleSpec};
use stdf_cypher::stdf::{Endian, Record, StdfReader};
use stdf_cypher::{api, ml, report, split};

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

#[test]
fn parse_both_endians_and_gzip_identically() {
    let d = tempfile::tempdir().unwrap();
    let le = gen(
        d.path(),
        "le.stdf",
        SampleSpec {
            wafers: 1,
            ..Default::default()
        },
    );
    let be = gen(
        d.path(),
        "be.stdf.gz",
        SampleSpec {
            wafers: 1,
            endian: Endian::Big,
            ..Default::default()
        },
    );
    let (a, b) = (load(&le), load(&be));
    assert_eq!(a.n_parts(), b.n_parts());
    assert_eq!(a.n_tests(), b.n_tests());
    for t in 0..a.n_tests() {
        assert_eq!(a.tests[t].key, b.tests[t].key);
        assert_eq!(a.tests[t].lo, b.tests[t].lo);
        for r in 0..a.n_parts() {
            let (x, y) = (a.values[t][r], b.values[t][r]);
            assert!(x == y || (x.is_nan() && y.is_nan()));
        }
    }
    assert_eq!(StdfReader::open(&be).unwrap().endian, Endian::Big);
}

#[test]
fn records_reencode_byte_exact() {
    let d = tempfile::tempdir().unwrap();
    let p = gen(
        d.path(),
        "a.stdf",
        SampleSpec {
            wafers: 1,
            radius: 5,
            ..Default::default()
        },
    );
    let mut r = StdfReader::open(&p).unwrap();
    let e = r.endian;
    let mut raw = vec![];
    let mut n = 0;
    while let Some(h) = r.next_raw_full(&mut raw).unwrap() {
        let rec = Record::parse(h.typ, h.sub, &raw[4..], e);
        assert_eq!(
            rec.encode(e),
            raw,
            "record {} re-encodes differently",
            rec.name()
        );
        n += 1;
    }
    assert!(n > 100);
}

#[test]
fn dataset_shape_limits_and_multipin() {
    let d = tempfile::tempdir().unwrap();
    let ds = load(&gen(d.path(), "a.stdf", SampleSpec::default()));
    assert_eq!(ds.wafers.len(), 3);
    // 40 PTR tests + 6 traceability PTRs + 4 MPR pins + 1 FTR
    assert_eq!(ds.n_tests(), 51);
    let t = ds.find_test("1000").unwrap();
    assert!(ds.tests[t].lo.is_some() && ds.tests[t].hi.is_some());
    assert_eq!(ds.tests[t].units, "uA");
    assert!(ds.find_test("100:CONTINUITY[IO3]").is_some());
    // tester PRR pass flag agrees with the per-test limits for parametric fails
    for (r, p) in ds.parts.iter().enumerate() {
        if p.is_pass() {
            assert!(
                p.first_fail.is_none(),
                "passing part {r} has a failing test"
            );
        }
    }
}

#[test]
fn filter_language() {
    let d = tempfile::tempdir().unwrap();
    let ds = load(&gen(d.path(), "a.stdf", SampleSpec::default()));
    let n = |e: &str| Filter::parse(e, &ds).unwrap().select_all(&ds).len();
    let all = ds.n_parts();
    assert_eq!(n(""), all);
    assert_eq!(n("site in (1,2)") + n("site not in (1,2)"), all);
    assert_eq!(n("pass") + n("not pass"), all);
    assert_eq!(n("hbin = 1"), n("pass"));
    assert_eq!(n("site between 2 and 3"), n("site in (2..3)"));
    assert_eq!(n("wafer ~ \"W0[12]$\""), n("wafer != \"LOTA-W03\""));
    assert_eq!(n("part_id = 17"), 1);
    assert!(n("t[1000] > 0") > 0);
    assert_eq!(
        n("(site = 1 or site = 2) and not (site = 2)"),
        n("site = 1")
    );
    let ids = d.path().join("ids.txt");
    std::fs::write(&ids, "1\n2\n# comment\n3\n").unwrap();
    assert_eq!(n(&format!("part_id in @{}", ids.display())), 3);
    assert!(Filter::parse("bogus_field = 1", &ds).is_err());
    assert!(Filter::parse("t[999999] > 1", &ds).is_err());
}

#[test]
fn split_preserves_every_part_and_limits() {
    let d = tempfile::tempdir().unwrap();
    let p = gen(d.path(), "a.stdf", SampleSpec::default());
    let ds = load(&p);
    let out = split::split_file(
        &p,
        &ds,
        &split::SplitBy::Field("hbin".into()),
        &d.path().join("out"),
        None,
    )
    .unwrap();
    let total: u64 = out.iter().map(|o| o.parts).sum();
    assert_eq!(total as usize, ds.n_parts());
    for o in &out {
        let s = load(Path::new(&o.path));
        assert_eq!(s.n_parts() as u64, o.parts);
        // limits survive even when the first (limit-carrying) PTR went to another file
        let t = s.find_test("1000").unwrap();
        assert_eq!(
            s.tests[t].lo,
            ds.tests[ds.find_test("1000").unwrap()].lo,
            "{}",
            o.name
        );
        assert!(s
            .parts
            .iter()
            .all(|p| p.hbin.to_string() == o.name.trim_start_matches("hbin")));
    }
    // named groups + rest
    let g = split::SplitBy::Groups {
        groups: vec![("s1".into(), "site = 1".into())],
        rest: Some("other".into()),
    };
    let out = split::split_file(&p, &ds, &g, &d.path().join("g"), Some("x")).unwrap();
    assert_eq!(out.len(), 2);
    assert_eq!(
        out.iter().map(|o| o.parts).sum::<u64>() as usize,
        ds.n_parts()
    );
}

#[test]
fn compare_detects_injected_shift() {
    let d = tempfile::tempdir().unwrap();
    let a = load(&gen(d.path(), "a.stdf", SampleSpec::default()));
    let same = load(&gen(
        d.path(),
        "a2.stdf",
        SampleSpec {
            seed: 9,
            ..Default::default()
        },
    ));
    let b = load(&gen(
        d.path(),
        "b.stdf",
        SampleSpec {
            seed: 2,
            shift: 0.8,
            ..Default::default()
        },
    ));
    let rep = compare::compare(
        &compare::Side::all(&a, "a"),
        &compare::Side::all(&b, "b"),
        &Default::default(),
    );
    // even-index tests get the full shift
    let shifted: Vec<_> = rep
        .tests
        .iter()
        .filter(|t| t.flag.contains("SHIFT"))
        .collect();
    assert!(shifted.len() >= 15, "only {} shifted", shifted.len());
    let null = compare::compare(
        &compare::Side::all(&a, "a"),
        &compare::Side::all(&same, "a2"),
        &Default::default(),
    );
    assert!(null.n_flagged <= 2, "false positives: {}", null.n_flagged);
}

#[test]
fn unit_correlation_classifies_tester_offset() {
    let d = tempfile::tempdir().unwrap();
    let a = load(&gen(d.path(), "a.stdf", SampleSpec::default()));
    let b = load(&gen(
        d.path(),
        "b.stdf",
        SampleSpec {
            meas_noise: 0.2,
            tester_offset: 1.5,
            ..Default::default()
        },
    ));
    let rep =
        correlation::unit_correlation(&a, &a.all_rows(), &b, &b.all_rows(), &Default::default());
    assert_eq!(rep.matched, a.n_parts());
    // tests with index % 3 == 2 carry the offset: 13 of 40
    assert_eq!(rep.summary["OFFSET"], 13);
    assert_eq!(rep.summary["UNCORRELATED"], 0);
}

#[test]
fn anova_finds_site_effect_tests() {
    let d = tempfile::tempdir().unwrap();
    let ds = load(&gen(d.path(), "a.stdf", SampleSpec::default()));
    let rep = anova::anova(&ds, &ds.all_rows(), "site", &ds.numeric_tests(), 5, 0.01);
    let sig: Vec<u32> = rep
        .rows
        .iter()
        .filter(|r| r.q < 0.01)
        .map(|r| ds.tests[ds.find_test(&r.key).unwrap()].num)
        .collect();
    // tests with index % 5 == 0 have a site offset: 1000, 1050, ..., 1350
    let expected: Vec<u32> = (0..40)
        .filter(|i| i % 5 == 0)
        .map(|i| 1000 + i * 10)
        .collect();
    for e in &expected {
        assert!(sig.contains(e), "test {e} not significant");
    }
    assert!(sig.len() <= expected.len() + 1);
}

#[test]
fn anomaly_finds_injected_outliers() {
    let d = tempfile::tempdir().unwrap();
    let ds = load(&gen(
        d.path(),
        "a.stdf",
        SampleSpec {
            outlier_rate: 0.01,
            ..Default::default()
        },
    ));
    let rep = anomaly::detect(&ds, &ds.all_rows(), &Default::default());
    let strong = rep.parts.iter().filter(|p| p.max_robust_z > 7.0).count();
    assert!(strong >= 8, "found {strong}");
    assert!(rep.n_flagged < ds.n_parts() / 20);
}

#[test]
fn ml_runs_and_is_informative() {
    let d = tempfile::tempdir().unwrap();
    let ds = load(&gen(d.path(), "a.stdf", SampleSpec::default()));
    let rows = ds.all_rows();
    let pr = ml::pca_report(&ds, &rows, &Default::default(), 5, Some(3)).unwrap();
    assert_eq!(pr.kmeans.unwrap().k, 3);
    assert!(pr.pca.explained_ratio[0] > pr.pca.explained_ratio[1]);
    let fr = ml::explain(
        &ds,
        &rows,
        &ml::TargetSpec::PassFail,
        &Default::default(),
        &Default::default(),
    )
    .unwrap();
    assert!(fr.auc.unwrap() > 0.7, "auc {:?}", fr.auc);
    let reg = ml::explain(
        &ds,
        &rows,
        &ml::TargetSpec::Test(ds.find_test("1000").unwrap()),
        &Default::default(),
        &Default::default(),
    )
    .unwrap();
    assert!(reg.r2.unwrap() > 0.5);
}

#[test]
fn every_api_op_and_report() {
    isolate();
    let d = tempfile::tempdir().unwrap();
    let a = gen(d.path(), "a.stdf", SampleSpec::default());
    let b = gen(
        d.path(),
        "b.stdf",
        SampleSpec {
            meas_noise: 0.2,
            ..Default::default()
        },
    );
    let reg = api::Registry::default();
    let r = api::run(
        &reg,
        "load",
        &json!({"paths": [a.to_string_lossy(), b.to_string_lossy()]}),
    )
    .unwrap();
    assert_eq!(r["ids"].as_array().unwrap().len(), 2);
    let calls = vec![
        ("datasets", json!({})),
        ("overview", json!({"ds": 0, "filter": "site != 2"})),
        ("tests", json!({"ds": 0})),
        ("parts", json!({"ds": 0, "limit": 10})),
        ("part_detail", json!({"ds": 0, "index": 5})),
        (
            "chart",
            json!({"ds": 0, "kind": "hist", "test": "1000", "group_by": "site"}),
        ),
        (
            "chart",
            json!({"ds": 0, "kind": "scatter", "test": "1000", "test2": "1030"}),
        ),
        (
            "chart",
            json!({"ds": 0, "kind": "wafer", "mode": "test", "test": "1000"}),
        ),
        (
            "chart",
            json!({"ds": 0, "kind": "wafer", "mode": "hbin", "wafer": "LOTA-W01"}),
        ),
        (
            "chart",
            json!({"ds": 0, "kind": "anova_means", "test": "1000", "factor": "site"}),
        ),
        (
            "chart",
            json!({"ds": 0, "b": 1, "kind": "compare_test", "test": "1000"}),
        ),
        (
            "chart",
            json!({"ds": 0, "b": 1, "kind": "unitcorr", "test": "1000"}),
        ),
        ("compare", json!({"a": 0, "b": 1})),
        (
            "compare",
            json!({"mode": "group", "a": 0, "select": "site = 1"}),
        ),
        ("unitcorr", json!({"a": 0, "b": 1})),
        ("correlation", json!({"ds": 0, "max_tests": 10})),
        ("target_corr", json!({"ds": 0, "target": "pass"})),
        ("anova", json!({"ds": 0, "factor": "wafer"})),
        ("anomaly", json!({"ds": 0})),
        ("pca", json!({"ds": 0, "color_by": "cluster"})),
        ("explain", json!({"ds": 0, "target": "hbin", "trees": 20})),
    ];
    for (op, p) in calls {
        let v = api::run(&reg, op, &p).unwrap_or_else(|e| panic!("{op} {p}: {e:#}"));
        assert!(!v.is_null(), "{op}");
    }
    let out = d.path().join("sel.stdf");
    let v = api::run(
        &reg,
        "export_stdf",
        &json!({"ds": 0, "filter": "site = 3", "path": out.to_string_lossy()}),
    )
    .unwrap();
    assert_eq!(load(&out).n_parts() as u64, v["parts"].as_u64().unwrap());
    let csv = api::export_csv_op(
        &reg,
        &json!({"ds": "0", "filter": "site = 1", "tests": "1000,1010"}),
    )
    .unwrap();
    assert_eq!(
        csv.lines().count(),
        1 + load(&a).parts.iter().filter(|p| p.site == 1).count()
    );

    let sets = vec![load(&a), load(&b)];
    let merged = Dataset::merge(sets.clone());
    assert_eq!(merged.n_parts(), sets[0].n_parts() * 2);
    let html = report::build(&sets, &merged, &Default::default()).unwrap();
    assert!(html.contains("report-data") && html.contains("unitcorr"));
}

#[test]
fn desktop_ops() {
    isolate();
    let d = tempfile::tempdir().unwrap();
    let reg = api::Registry::app();
    // sample data, load, recent list
    let paths = api::run(&reg, "sample", &json!({})).unwrap()["paths"].clone();
    assert_eq!(paths.as_array().unwrap().len(), 3);
    let bad = d.path().join("not_stdf.stdf");
    std::fs::write(&bad, b"hello").unwrap();
    let mut all = paths.as_array().unwrap().clone();
    all.push(json!(bad.to_string_lossy()));
    let r = api::run(&reg, "load", &json!({ "paths": all })).unwrap();
    assert_eq!(r["ids"].as_array().unwrap().len(), 3, "good files load");
    assert_eq!(
        r["errors"].as_array().unwrap().len(),
        1,
        "bad file reported, not fatal"
    );
    // the recent list is shared on disk and capped at 15: tests running in
    // parallel push to it too, so only check that it is kept
    let recent = api::run(&reg, "recent", &json!({})).unwrap();
    let recent = recent.as_array().unwrap();
    assert!(!recent.is_empty() && recent.len() <= 15);
    assert!(recent.iter().all(|r| r["path"].is_string()));
    // close a dataset: ids of the others stay valid
    api::run(&reg, "remove", &json!({"ds": 1})).unwrap();
    let ds = api::run(&reg, "datasets", &json!({})).unwrap();
    let ids: Vec<u64> = ds
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["id"].as_u64().unwrap())
        .collect();
    assert_eq!(ids, vec![0, 2]);
    assert!(api::run(&reg, "overview", &json!({"ds": 1})).is_err());
    assert!(api::run(&reg, "overview", &json!({"ds": 2})).is_ok());
    // hand-off from a second launch arrives through ping
    api::run(&reg, "open", &json!({"paths": ["x.stdf"]})).unwrap();
    let p = api::run(&reg, "ping", &json!({})).unwrap();
    assert_eq!(p["pending"][0], "x.stdf");
    assert_eq!(
        api::run(&reg, "ping", &json!({})).unwrap()["pending"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
    assert!(!reg.session.should_exit());
    // split + report from the UI
    let out = d.path().join("split");
    let r = api::run(&reg, "split", &json!({"ds": 0, "by": "groups", "groups": [{"name": "s1", "expr": "site = 1"}], "rest": "other", "out_dir": out.to_string_lossy()})).unwrap();
    assert_eq!(r["outputs"].as_array().unwrap().len(), 2);
    let rep = d.path().join("r.html");
    api::run(
        &reg,
        "save_report",
        &json!({"ds": 0, "filter": "site != 4", "path": rep.to_string_lossy()}),
    )
    .unwrap();
    assert!(
        std::fs::metadata(&rep).unwrap().len() > 1_000_000,
        "offline report embeds plotly"
    );
}

#[test]
fn match_trace_and_split_by_identity() {
    isolate();
    let d = tempfile::tempdir().unwrap();
    let a = gen(d.path(), "a.stdf", SampleSpec::default());
    // same lot, second tester: identical dies
    let b = gen(
        d.path(),
        "b.stdf",
        SampleSpec {
            seed: 9,
            meas_noise: 0.3,
            ..SampleSpec::default()
        },
    );
    let reg = api::Registry::app();
    let r = api::run(
        &reg,
        "load",
        &json!({"paths": [a.to_string_lossy(), b.to_string_lossy()]}),
    )
    .unwrap();
    assert_eq!(r["ids"], json!([0, 1]));
    api::run(&reg, "rename", &json!({"ds": 1, "name": "tester 2"})).unwrap();
    assert_eq!(reg.name(1), "tester 2");
    let n = reg.get(0).unwrap().0.n_parts();

    // the three key kinds identify the same dies
    for (ka, kb) in [
        ("part_txt", "part_txt"),
        ("tests:500", "500"),
        ("80100,80101,80102", "wafer_xy"),
    ] {
        let m = api::run(
            &reg,
            "match",
            &json!({"a": 0, "b": 1, "key_a": ka, "key_b": kb}),
        )
        .unwrap();
        if kb == "wafer_xy" {
            // PRR wafer ids are text ("LOTA-W01"), the logged wafer is a number: no overlap
            assert_eq!(m["pairs"], 0, "{ka} vs {kb}");
            continue;
        }
        assert_eq!(m["pairs"].as_u64().unwrap() as usize, n, "{ka} vs {kb}");
    }
    // keep only the first wafer of B, then match by the unique die code
    let m = api::run(
        &reg,
        "match",
        &json!({"a": 0, "b": 1, "key_a": "500", "filter_b": "t[80100] = 1", "name": "w1"}),
    )
    .unwrap();
    let matched = m["matched_a"].as_u64().unwrap() as usize;
    assert!(matched > 0 && matched < n);
    assert_eq!(m["only_a"].as_u64().unwrap() as usize, n - matched);
    // the part set works in filters and in group compare
    let ds0 = reg.get(0).unwrap().0;
    assert_eq!(reg.rows(0, &ds0, "$w1").unwrap().len(), matched);
    assert_eq!(reg.rows(0, &ds0, "not $w1").unwrap().len(), n - matched);
    assert!(reg.rows(0, &ds0, "$nope").is_err());
    let c = api::run(
        &reg,
        "compare",
        &json!({"mode": "group", "a": 0, "select": "$w1", "vs": "not $w1"}),
    )
    .unwrap();
    assert_eq!(c["report"]["n_b"].as_u64().unwrap() as usize, matched);
    // split A into matched / unmatched parts
    let out = d.path().join("split");
    let r = api::run(&reg, "split", &json!({"ds": 0, "by": "groups", "groups": [{"name": "matched", "expr": "$w1"}], "rest": "unmatched", "out_dir": out.to_string_lossy(), "prefix": "a"})).unwrap();
    let outs = r["outputs"].as_array().unwrap();
    assert_eq!(outs.len(), 2);
    let m_file = outs.iter().find(|o| o["name"] == "matched").unwrap();
    assert_eq!(m_file["parts"].as_u64().unwrap() as usize, matched);
    assert_eq!(
        load(Path::new(m_file["path"].as_str().unwrap())).n_parts(),
        matched
    );
    // dual chart: A test 1000 vs B test 1010 over the same dies
    let link = m["link"].clone();
    let dual = api::run(&reg, "match_dual", &json!({"link": link, "test_a": "1000", "test_b": "1010", "color_by": "pass", "range": "limits"})).unwrap();
    assert_eq!(dual["n"].as_u64().unwrap() as usize, matched);
    for f in ["hist_a", "hist_b", "scatter", "delta"] {
        assert!(
            !dual["figures"][f]["data"].as_array().unwrap().is_empty(),
            "{f}"
        );
    }
    // trace one die by its logged code
    let key = m["rows"][0]["key"].as_str().unwrap().to_string();
    let t = api::run(&reg, "trace", &json!({"link": link, "value": key})).unwrap();
    assert_eq!(t["a"]["index"], m["rows"][0]["a_row"]);
    assert_eq!(t["b"]["index"], m["rows"][0]["b_row"]);
    assert!(t["tests"]
        .as_array()
        .unwrap()
        .iter()
        .any(|x| x["num"] == 1000 && x["diff"].is_number()));

    // split by a pasted part list and by a PTR value
    let r = api::run(&reg, "split", &json!({"ds": 0, "by": "list", "list_key": "part_id", "list": "1\n2, 3", "rest": "", "out_dir": out.to_string_lossy(), "prefix": "l"})).unwrap();
    assert_eq!(r["outputs"][0]["parts"], 3);
    let r = api::run(&reg, "split", &json!({"ds": 0, "by": "test", "test": "80100", "out_dir": out.to_string_lossy(), "prefix": "w"})).unwrap();
    assert_eq!(r["outputs"].as_array().unwrap().len(), 3);

    // wafer maps rebuilt from logged wafer/X/Y tests
    let w = api::run(&reg, "chart", &json!({"ds": 0, "kind": "wafer", "coords": "tests", "wafer_test": "80100", "x_test": "80101", "y_test": "80102", "ox_test": "80103", "oy_test": "80104"})).unwrap();
    assert_eq!(w["wafers"], json!(["1", "2", "3"]));
    assert_eq!(w["maps"].as_array().unwrap().len(), 3);

    // raw records: paging, test filter, a part's own block
    let rec = api::run(
        &reg,
        "records",
        &json!({"ds": 0, "rec": "PTR", "test": 500, "limit": 5}),
    )
    .unwrap();
    assert_eq!(rec["total"].as_u64().unwrap() as usize, n);
    assert_eq!(rec["rows"].as_array().unwrap().len(), 5);
    assert_eq!(rec["rows"][0]["test_num"], 500);
    let prr = api::run(
        &reg,
        "records",
        &json!({"ds": 0, "rec": "PRR", "offset": 7, "limit": 1}),
    )
    .unwrap();
    assert_eq!(prr["rows"][0]["_part"], 7);
    let pr = api::run(&reg, "part_records", &json!({"ds": 0, "index": 7})).unwrap();
    let recs = pr["records"].as_array().unwrap();
    assert_eq!(recs.first().unwrap()["_rec"], "PIR");
    assert_eq!(recs.last().unwrap()["_rec"], "PRR");
    assert_eq!(recs.last().unwrap()["part_id"], ds0.parts[7].part_id);
    assert!(recs.iter().filter(|r| r["_rec"] == "PTR").count() >= 44);

    // alarm coloring and range modes
    let h = api::run(
        &reg,
        "chart",
        &json!({"ds": 0, "kind": "hist", "test": "1000", "group_by": "alarm", "range": "data"}),
    )
    .unwrap();
    assert!(h["hist"]["layout"]["xaxis"]["range"].is_array());
    let alarms: usize = ds0.parts.iter().filter(|p| !p.alarms.is_empty()).count();
    assert!(alarms > 0, "sample sets some alarm flags");
}

#[test]
fn marker_tests_fail_filters_and_offset_wafer_maps() {
    isolate();
    let d = tempfile::tempdir().unwrap();
    let a = gen(d.path(), "s1.stdf", SampleSpec::default());
    let b = gen(
        d.path(),
        "s2.stdf",
        SampleSpec {
            meas_noise: 0.3,
            ..SampleSpec::default()
        },
    );
    let reg = api::Registry::app();
    api::run(
        &reg,
        "load",
        &json!({"paths": [a.to_string_lossy(), b.to_string_lossy()]}),
    )
    .unwrap();
    let ds = reg.get(0).unwrap().0;

    // fail(...) / first_fail / n_fails
    let t = ds.find_test("1160").unwrap();
    let expect = (0..ds.n_parts())
        .filter(|&r| {
            let v = ds.value(t, r);
            ds.parts[r].has_fail(t) || (v.is_finite() && !ds.tests[t].within_limits(v))
        })
        .count();
    assert!(expect > 0);
    assert_eq!(reg.rows(0, &ds, "fail(1160)").unwrap().len(), expect);
    let first = reg.rows(0, &ds, "first_fail = 1160").unwrap();
    assert!(!first.is_empty() && first.len() <= expect);
    assert!(
        reg.rows(0, &ds, "fail(1160) and n_fails = 1")
            .unwrap()
            .len()
            <= expect
    );
    assert!(reg.rows(0, &ds, "fail(999999)").is_err());

    // workflow: bin-1 parts of file 1 matched to parts of file 2 that fail a test
    let m = api::run(&reg, "match", &json!({"a": 0, "b": 1, "key_a": "part_txt", "filter_a": "hbin = 1", "filter_b": "not pass", "name": "became_bad"})).unwrap();
    let affected = m["matched_a"].as_u64().unwrap() as usize;
    assert!(affected > 0, "some bin-1 parts fail on the second file");
    // two new datasets from the match, then compare them
    let s1 = api::run(
        &reg,
        "subset_ds",
        &json!({"ds": 0, "filter": "hbin = 1 and $became_bad", "name": "affected"}),
    )
    .unwrap();
    let s2 = api::run(
        &reg,
        "subset_ds",
        &json!({"ds": 0, "filter": "hbin = 1 and not $became_bad", "name": "unaffected"}),
    )
    .unwrap();
    assert_eq!(s1["parts"].as_u64().unwrap() as usize, affected);
    let mk = api::run(
        &reg,
        "markers",
        &json!({"mode": "datasets", "a": s2["id"], "b": s1["id"], "trees": 30}),
    )
    .unwrap();
    assert_eq!(mk["n_aff"].as_u64().unwrap() as usize, affected);
    assert!(mk["rows"].as_array().unwrap().len() > 30);
    assert!(
        api::run(&reg, "records", &json!({"ds": s1["id"]})).is_err(),
        "derived datasets have no file"
    );

    // a planted marker must come out on top: affected = top 4% of test 1050 (one wafer-sort test)
    let t1050 = ds.find_test("1050").unwrap();
    let mut v = ds.values_of(t1050, None);
    v.sort_by(|x, y| x.partial_cmp(y).unwrap());
    let cut = v[(v.len() as f64 * 0.96) as usize];
    let sel = format!("t[1050] > {cut}");
    let mk = api::run(&reg, "markers", &json!({"mode": "group", "a": 0, "select": sel, "vs": format!("not ({sel})"), "later": 1, "key_a": "part_txt", "trees": 40})).unwrap();
    let top = &mk["rows"][0];
    assert_eq!(top["num"], 1050, "planted marker ranks first: {top}");
    assert!(top["separation"].as_f64().unwrap() > 0.9);
    assert!(
        top["drift_sep"].is_number() && top["r_ref"].is_number(),
        "drift view uses the later file"
    );
    assert!(mk["forest_auc"].as_f64().unwrap() > 0.9);
    assert_eq!(mk["figures"]["top"].as_array().unwrap().len(), 6);

    // wafer maps from logged wafer / raw X,Y + offsets reproduce the PRR die grid
    let w = api::run(&reg, "chart", &json!({"ds": 0, "kind": "wafer", "coords": "tests", "wafer": "1", "wafer_test": "80100", "x_test": "80101", "y_test": "80102", "ox_test": "80103", "oy_test": "80104"})).unwrap();
    let xs = w["maps"][0]["figure"]["data"][0]["x"]
        .as_array()
        .unwrap()
        .clone();
    let minx = ds.parts.iter().map(|p| p.x).min().unwrap();
    let maxx = ds.parts.iter().map(|p| p.x).max().unwrap();
    assert_eq!(xs.first().unwrap().as_i64().unwrap(), minx as i64);
    assert_eq!(xs.last().unwrap().as_i64().unwrap(), maxx as i64);
    let raw = api::run(&reg, "chart", &json!({"ds": 0, "kind": "wafer", "coords": "tests", "wafer": "1", "wafer_test": "80100", "x_test": "80101", "y_test": "80102"})).unwrap();
    assert_eq!(
        raw["maps"][0]["figure"]["data"][0]["x"]
            .as_array()
            .unwrap()
            .len(),
        4,
        "without offsets only the 4 reticle columns remain"
    );
}

#[test]
fn bad_candidates_profile_without_splitting() {
    isolate();
    let d = tempfile::tempdir().unwrap();
    let a = gen(d.path(), "s1.stdf", SampleSpec::default());
    let b = gen(
        d.path(),
        "s2.stdf",
        SampleSpec {
            meas_noise: 0.3,
            ..SampleSpec::default()
        },
    );
    let reg = api::Registry::app();
    api::run(
        &reg,
        "load",
        &json!({"paths": [a.to_string_lossy(), b.to_string_lossy()]}),
    )
    .unwrap();
    // good in file 1, bad in file 2: marked in file 1 as $bad_candidate
    let m = api::run(&reg, "match", &json!({"a": 0, "b": 1, "key_a": "part_txt", "filter_a": "hbin = 1", "filter_b": "not pass", "name": "bad_candidate"})).unwrap();
    let n = m["pairs"].as_u64().unwrap();
    assert!(n > 0);

    let excl = "500, 80100-80104";
    let c = api::run(&reg, "candidates", &json!({"a": 0, "filter": "hbin = 1", "name": "bad_candidate", "exclude": excl, "coords": "tests",
        "wafer_test": "80100", "x_test": "80101", "y_test": "80102", "ox_test": "80103", "oy_test": "80104"})).unwrap();
    assert_eq!(c["n_aff"].as_u64().unwrap(), n);
    assert_eq!(c["parts"].as_array().unwrap().len() as u64, n);
    let inds = c["indicators"].as_array().unwrap();
    assert!(inds.len() >= 4);
    assert!(inds
        .iter()
        .all(|i| (0.0..=1.0).contains(&i["auc"].as_f64().unwrap())));
    let keys: Vec<&str> = c["factors"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["key"].as_str().unwrap())
        .collect();
    assert!(
        keys.contains(&"wafer") && keys.contains(&"field"),
        "{keys:?}"
    );
    assert!(c["b_fails"][0]["share"].as_f64().unwrap() > 0.0);
    assert!(!c["stack"].as_array().unwrap().is_empty());
    assert!(
        api::run(
            &reg,
            "candidates",
            &json!({"a": 0, "filter": "hbin = 1", "name": "nope"})
        )
        .is_err(),
        "an unknown part set is an error"
    );

    // the marker ranking leaves the identity tests out and reports PAT and ANOVA
    let mk = api::run(&reg, "markers", &json!({"mode": "group", "a": 0, "select": "$bad_candidate", "vs": "", "filter": "hbin = 1", "exclude": excl, "trees": 30})).unwrap();
    let rows = mk["rows"].as_array().unwrap();
    assert!(rows.iter().all(|r| {
        let num = r["num"].as_u64().unwrap();
        num != 500 && !(80100..=80104).contains(&num)
    }));
    assert!(rows
        .iter()
        .all(|r| r["eta2"].is_number() && r["pat_aff"].is_number()));

    // charts can be colored by the part set
    let h = api::run(&reg, "chart", &json!({"ds": 0, "kind": "hist", "filter": "hbin = 1", "test": rows[0]["key"], "group_by": "$bad_candidate"})).unwrap();
    let names: Vec<&str> = h["hist"]["data"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|t| t["name"].as_str())
        .collect();
    assert_eq!(names.len(), 2);
    assert!(names[1].starts_with("$bad_candidate"), "{names:?}");
}

#[test]
fn signature_learned_in_one_lot_checked_against_a_clean_lot() {
    isolate();
    let d = tempfile::tempdir().unwrap();
    let a1 = gen(d.path(), "a1.stdf", SampleSpec::default());
    let a2 = gen(
        d.path(),
        "a2.stdf",
        SampleSpec {
            meas_noise: 0.3,
            ..SampleSpec::default()
        },
    );
    let b = gen(
        d.path(),
        "b.stdf",
        SampleSpec {
            lot_id: "LOTB".into(),
            seed: 7,
            ..SampleSpec::default()
        },
    );
    let reg = api::Registry::app();
    let paths: Vec<String> = [&a1, &a2, &b]
        .iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    api::run(&reg, "load", &json!({ "paths": paths })).unwrap();
    api::run(&reg, "match", &json!({"a": 0, "b": 1, "key_a": "part_txt", "filter_a": "hbin = 1", "filter_b": "not pass", "name": "bad_candidate"})).unwrap();

    let s = api::run(&reg, "signature", &json!({"a": 0, "name": "bad_candidate", "filter": "hbin = 1", "exclude": "500, 80100-80104",
        "clean": [{"ds": 2, "filter": "hbin = 1"}], "screen": [{"ds": 2, "filter": "hbin = 1"}, {"ds": 0, "filter": "hbin = 1"}]})).unwrap();
    let rows = s["rows"].as_array().unwrap();
    assert!(rows.len() > 30);
    let verdicts = ["keep", "caution", "drop", "untested", "weak", "no data"];
    for r in rows {
        assert!(verdicts.contains(&r["verdict"].as_str().unwrap()), "{r}");
        let num = r["num"].as_u64().unwrap();
        assert!(num != 500 && !(80100..=80104).contains(&num));
        // the rule is valid filter syntax that selects what it claims to
        let f = format!(
            "hbin = 1 and $bad_candidate and {}",
            r["filter"].as_str().unwrap()
        );
        let hits = reg.rows(0, &reg.get(0).unwrap().0, &f).unwrap().len() as f64;
        let n = s["n_cand"].as_f64().unwrap();
        assert!(
            (hits / n - r["capture"].as_f64().unwrap()).abs() < 1e-9,
            "{f}"
        );
    }
    // the same process in another lot: most per-test rules are not specific
    let dropped = rows.iter().filter(|r| r["verdict"] == "drop").count();
    assert!(dropped * 2 > rows.len(), "{dropped} of {}", rows.len());
    // screening the clean lot itself is never worse than the clean lot
    let scr = s["screen"].as_array().unwrap();
    assert_eq!(scr.len(), 2);
    if !s["signature"]["rules"].as_array().unwrap().is_empty() {
        assert_eq!(scr[0]["verdict"], "like the reference");
    }
    assert!(api::run(
        &reg,
        "signature",
        &json!({"a": 0, "name": "nope", "clean": [{"ds": 2}]})
    )
    .is_err());
}

#[test]
fn candidate_range_marginal_confirmed_good_and_score_methods() {
    isolate();
    let d = tempfile::tempdir().unwrap();
    let a1 = gen(d.path(), "a1.stdf", SampleSpec::default());
    let a2 = gen(
        d.path(),
        "a2.stdf",
        SampleSpec {
            meas_noise: 0.3,
            ..SampleSpec::default()
        },
    );
    let b = gen(
        d.path(),
        "b.stdf",
        SampleSpec {
            lot_id: "LOTB".into(),
            seed: 7,
            ..SampleSpec::default()
        },
    );
    let reg = api::Registry::app();
    let paths: Vec<String> = [&a1, &a2, &b]
        .iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    api::run(&reg, "load", &json!({ "paths": paths })).unwrap();
    let (da, db) = (reg.get(0).unwrap().0, reg.get(1).unwrap().0);

    // the test most of file 2's fails fail, and a cut through its failing values
    let bad_b = reg.rows(1, &db, "not pass").unwrap();
    let mut counts = std::collections::HashMap::new();
    for &r in &bad_b {
        for &t in &db.parts[r].fails {
            *counts.entry(t as usize).or_insert(0usize) += 1;
        }
    }
    let t = *counts.iter().max_by_key(|c| *c.1).unwrap().0;
    let num = db.tests[t].num;
    let mut v: Vec<f64> = bad_b
        .iter()
        .map(|&r| db.value(t, r))
        .filter(|x| x.is_finite())
        .collect();
    v.sort_by(|x, y| x.partial_cmp(y).unwrap());
    // halfway between two distinct values, so rounding cannot move a part across
    let i = v.len() / 2;
    let j = (i..v.len()).find(|&j| v[j] > v[i]).unwrap();
    let cut = (v[i] + v[j]) / 2.0;
    let sets = |include: bool| {
        api::run(&reg, "candidate_sets", &json!({"a": 0, "b": 1, "key_a": "part_txt", "filter_a": "hbin = 1",
            "filter_b": format!("not pass and fail({num})"), "test": num.to_string(), "range": format!("{cut}.."),
            "include_marginal": include})).unwrap()
    };
    let cs = sets(false);
    let (nc, nm, nconf) = (
        cs["n_cand"].as_u64().unwrap() as usize,
        cs["n_marginal"].as_u64().unwrap() as usize,
        cs["n_confirmed"].as_u64().unwrap() as usize,
    );
    assert!(nc >= 5 && nm > 0, "{nc} candidates, {nm} marginal");
    assert_eq!(nc + nm, cs["matched"].as_u64().unwrap() as usize);
    // the range is on the file-2 value
    assert!(cs["rows"]
        .as_array()
        .unwrap()
        .iter()
        .all(|r| r["b_value"].as_f64().unwrap() >= cut));
    assert!(cs["marginal_rows"]
        .as_array()
        .unwrap()
        .iter()
        .all(|r| r["b_value"].as_f64().is_none_or(|v| v < cut)));
    let count = |f: &str| reg.rows(0, &da, f).unwrap().len();
    assert_eq!(count("$bad_candidate"), nc);
    assert_eq!(count("$marginal"), nm);
    assert_eq!(count("$bad_candidate and $marginal"), 0);
    // confirmed good: good in file 1, passing in file 2, never a candidate
    assert!(nconf > 30);
    assert_eq!(count("$confirmed_good"), nconf);
    assert_eq!(count("$confirmed_good and not (hbin = 1)"), 0);
    assert_eq!(
        count("$confirmed_good and ($bad_candidate or $marginal)"),
        0
    );
    assert_eq!(
        reg.rows(1, &db, "$confirmed_good and not pass")
            .unwrap()
            .len(),
        0
    );
    // marginal fails as candidates
    let inc = sets(true);
    assert_eq!(inc["n_cand"].as_u64().unwrap() as usize, nc + nm);
    assert_eq!(count("$bad_candidate and $marginal"), nm);
    sets(false);
    // SI suffixes on the failing test's units: the same cut, written in milli-units
    let (scale, _) = stdf_cypher::analysis::candidates::unit_scale(&db.tests[t].units);
    let si = api::run(&reg, "candidate_sets", &json!({"a": 0, "b": 1, "filter_a": "hbin = 1", "filter_b": format!("not pass and fail({num})"),
        "test": num.to_string(), "range": format!("{}m..", cut * scale * 1000.0), "name": "si_check", "marginal": "si_marg", "confirmed": "si_conf"})).unwrap();
    assert_eq!(si["n_cand"].as_u64().unwrap() as usize, nc);
    assert!(
        api::run(
            &reg,
            "candidate_sets",
            &json!({"a": 0, "b": 1, "filter_a": "hbin = 1", "range": "1..2", "name": "x"})
        )
        .is_err(),
        "a range needs the test"
    );

    // signature without a clean lot: learned against $confirmed_good, marginal parts out
    let sig = |extra: serde_json::Value| {
        let mut p = json!({"a": 0, "name": "bad_candidate", "filter": "hbin = 1", "exclude": "500, 80100-80104", "exclude_set": "marginal"});
        p.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        api::run(&reg, "signature", &p)
    };
    let s = sig(json!({})).unwrap();
    assert_eq!(s["normal"], "confirmed good");
    assert_eq!(s["n_normal"].as_u64().unwrap() as usize, nconf);
    assert_eq!(s["n_cand"].as_u64().unwrap() as usize, nc);
    assert_eq!(s["sources"].as_array().unwrap().len(), 1);
    assert_eq!(s["sources"][0]["kind"], "confirmed");
    let notes = s["notes"].to_string();
    assert!(
        notes.contains("$marginal") && notes.contains("in-sample"),
        "{notes}"
    );
    // fewer confirmed good than asked for: back to all other good parts, with a note
    let s = sig(json!({"min_confirmed": nconf + 1})).unwrap();
    assert_eq!(s["normal"], "lot A normal");
    assert!(s["notes"].to_string().contains("fewer than"));

    // both sources: per-source rates, verdicts on the pooled reference
    let s = sig(json!({"clean": [{"ds": 2, "filter": "hbin = 1"}], "screen": [{"ds": 2, "filter": "hbin = 1"}]})).unwrap();
    assert_eq!(s["normal"], "lot A normal");
    let kinds: Vec<&str> = s["sources"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x["kind"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, ["clean", "confirmed"]);
    let rows = s["rows"].as_array().unwrap();
    assert!(rows.len() > 30);
    for r in rows.iter().filter(|r| r["pool"].is_object()) {
        let src = r["sources"].as_array().unwrap();
        let sum = |k: &str| src.iter().map(|x| x[k].as_u64().unwrap()).sum::<u64>();
        assert_eq!(r["pool"]["n"].as_u64().unwrap(), sum("n"));
        assert_eq!(r["pool"]["hits"].as_u64().unwrap(), sum("hits"));
        assert_eq!(r["fa_ref"], r["pool"]["rate"]);
    }
    let src = s["signature"]["sources"].as_array().unwrap();
    assert_eq!(src.len(), 2);
    // any one source can drop a rule, and says so
    for r in rows {
        let drop = r["verdict"] == "drop";
        assert_eq!(drop, r["drop_by"].is_string(), "{}", r["key"]);
        if !drop {
            let cap = r["capture"].as_f64().unwrap();
            for x in r["sources"].as_array().unwrap() {
                assert!(
                    x["rate"].as_f64().unwrap() < 0.5 * cap,
                    "{} {}",
                    r["key"],
                    x["lot"]
                );
            }
        }
    }
    // the greedy signature is never worse than its best single rule
    let usable: Vec<&serde_json::Value> = rows
        .iter()
        .filter(|r| r["verdict"] == "keep" || r["verdict"] == "caution")
        .collect();
    let sg = &s["signature"];
    assert_eq!(sg["considered"].as_u64().unwrap() as usize, usable.len());
    assert_eq!(
        sg["steps"].as_array().unwrap().len(),
        sg["rules"].as_array().unwrap().len()
    );
    if let Some(best) = usable
        .iter()
        .map(|r| r["score"].as_f64().unwrap())
        .reduce(f64::max)
    {
        assert!(sg["score"].as_f64().unwrap() >= best - 1e-9);
        assert_eq!(
            sg["steps"][0]["key"],
            usable
                .iter()
                .max_by(|a, b| a["score"]
                    .as_f64()
                    .partial_cmp(&b["score"].as_f64())
                    .unwrap())
                .unwrap()["key"]
        );
    }
    let s =
        sig(json!({"clean": [{"ds": 2, "filter": "hbin = 1"}], "use_confirmed": false})).unwrap();
    assert_eq!(s["sources"].as_array().unwrap().len(), 1);
    assert!(
        sig(json!({"clean": [{"ds": 2}], "use_clean": false, "use_confirmed": false})).is_err()
    );
    // stayed good on retest: good in run 1, passing again on the retest
    let s = sig(json!({"use_confirmed": false, "stayed": [{"a": 0, "b": 1}]})).unwrap();
    assert_eq!(s["sources"][0]["kind"], "stayed_good");
    assert_eq!(s["sources"][0]["n"].as_u64().unwrap() as usize, nconf);
    assert_eq!(
        s["normal"], "lot A normal",
        "an independent source: no confirmed-good learning"
    );
    assert!(sig(json!({"use_confirmed": false, "stayed": [{"a": 0, "b": 0}]})).is_err());

    // score methods: the score is the largest ticked sub-score
    let mk = |methods: serde_json::Value| {
        api::run(&reg, "markers", &json!({"mode": "group", "a": 0, "select": "$bad_candidate", "vs": "", "filter": "hbin = 1 and not $marginal",
            "later": 1, "key_a": "part_txt", "exclude": "500, 80100-80104", "trees": 30, "methods": methods})).unwrap()
    };
    let all = mk(serde_json::Value::Null);
    let rows = all["rows"].as_array().unwrap();
    assert_eq!(all["n_aff"].as_u64().unwrap() as usize, nc);
    assert!(
        rows.iter()
            .all(|r| r["subs"]["ANOVA"].is_number() && r["why"] != "ANOVA"),
        "ANOVA is off by default"
    );
    let tails = mk(json!(["tails"]));
    for r in tails["rows"].as_array().unwrap() {
        assert_eq!(r["why"], "tails");
        assert_eq!(r["score"], r["subs"]["tails"]);
    }
    let two = mk(json!("ANOVA, separation"));
    for r in two["rows"].as_array().unwrap() {
        let (x, y) = (
            r["subs"]["ANOVA"].as_f64().unwrap(),
            r["subs"]["separation"].as_f64().unwrap(),
        );
        assert_eq!(r["score"].as_f64().unwrap(), x.max(y));
    }
    assert!(api::run(
        &reg,
        "markers",
        &json!({"mode": "group", "a": 0, "select": "$bad_candidate", "methods": ["magic"]})
    )
    .is_err());

    // file 2 logged in base units with RES_SCAL (12 uA stored as 1.2e-5 A):
    // plain numbers follow the display unit, so the same range gives the same parts
    let a2b = gen(
        d.path(),
        "a2b.stdf",
        SampleSpec {
            meas_noise: 0.3,
            base_units: true,
            ..SampleSpec::default()
        },
    );
    api::run(&reg, "load", &json!({ "paths": [a2b.to_string_lossy()] })).unwrap();
    let d3 = reg.get(3).unwrap().0;
    let prefixed = |u: &str| stdf_cypher::analysis::candidates::unit_scale(u).0 != 1.0;
    let tp = (0..db.n_tests())
        .filter(|&t| prefixed(&db.tests[t].units))
        .max_by_key(|&t| {
            bad_b
                .iter()
                .filter(|&&r| db.parts[r].fails.contains(&(t as u32)))
                .count()
        })
        .unwrap();
    let (num, units) = (db.tests[tp].num, db.tests[tp].units.clone());
    let t3 = d3.find_test(&num.to_string()).unwrap();
    assert_ne!(d3.tests[t3].units, units, "stored in the base unit");
    assert_ne!(d3.tests[t3].res_scal, 0);
    let tl = api::run(&reg, "tests", &json!({"ds": 3})).unwrap();
    let info = tl
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["num"] == num)
        .unwrap()
        .clone();
    assert_eq!(info["disp_units"], units.as_str(), "{info}");
    let mut v: Vec<f64> = bad_b
        .iter()
        .filter(|&&r| db.parts[r].fails.contains(&(tp as u32)))
        .map(|&r| db.value(tp, r))
        .collect();
    v.sort_by(|x, y| x.partial_cmp(y).unwrap());
    assert!(v.len() >= 4, "{} fails on test {num}", v.len());
    {
        let i = v.len() / 2;
        let j = (i..v.len()).find(|&j| v[j] > v[i]).unwrap_or(i);
        let cut = (v[i] + v[j.max(i)]) / 2.0;
        let run = |b: usize, name: &str| {
            api::run(&reg, "candidate_sets", &json!({"a": 0, "b": b, "filter_a": "hbin = 1", "filter_b": format!("not pass and fail({num})"),
                "test": num.to_string(), "range": format!("{cut}.."), "name": name, "marginal": format!("{name}_m"), "confirmed": format!("{name}_c")})).unwrap()
        };
        let (shown, based) = (run(1, "rs_shown"), run(3, "rs_based"));
        assert_eq!(based["range"]["units"], units.as_str());
        assert_eq!(shown["n_cand"], based["n_cand"], "{shown:?} vs {based:?}");
        assert_eq!(shown["n_marginal"], based["n_marginal"]);
    }
}

#[test]
fn retest_trace_follows_rejects_through_runs() {
    isolate();
    let d = tempfile::tempdir().unwrap();
    // run 1, and two further measurements of the same parts with fresh noise
    let r1 = gen(d.path(), "r1.stdf", SampleSpec::default());
    let full = |name: &str, seed: u64| {
        gen(
            d.path(),
            name,
            SampleSpec {
                meas_noise: 0.3,
                meas_seed: seed,
                ..SampleSpec::default()
            },
        )
    };
    let (f2, f3) = (full("f2.stdf", 1), full("f3.stdf", 2));
    let reg = api::Registry::app();
    let load = |p: &Path| {
        api::run(&reg, "load", &json!({ "paths": [p.to_string_lossy()] })).unwrap();
    };
    for p in [&r1, &f2, &f3] {
        load(p);
    }
    // run 2 = the rejects of run 1, retested; run 3 = the rejects of run 2
    let retest = |from: usize, full: usize, name: &str| -> PathBuf {
        api::run(
            &reg,
            "match",
            &json!({"a": from, "b": full, "filter_a": "not pass", "name": name}),
        )
        .unwrap();
        let out = d.path().join(format!("{name}.stdf"));
        api::run(
            &reg,
            "export_stdf",
            &json!({"ds": full, "filter": format!("${name}"), "path": out.to_string_lossy()}),
        )
        .unwrap();
        out
    };
    let r2 = retest(0, 1, "rej1");
    load(&r2);
    let r3 = retest(3, 2, "rej2");
    load(&r3);
    let (d1, d2) = (reg.get(0).unwrap().0, reg.get(3).unwrap().0);
    let n_rej = reg.rows(0, &d1, "not pass").unwrap().len();
    assert_eq!(d2.n_parts(), n_rej, "run 2 holds exactly run 1's rejects");

    let t = api::run(
        &reg,
        "retest_trace",
        &json!({"runs": [0, 3, 4], "exclude": "500, 80100-80104"}),
    )
    .unwrap();
    let s = t["summary"].as_array().unwrap();
    let u = |v: &serde_json::Value, k: &str| v[k].as_u64().unwrap() as usize;
    assert_eq!(u(&t, "n_parts"), n_rej);
    assert_eq!((u(&s[0], "due"), u(&s[0], "fail")), (n_rej, n_rej));
    // every reject is retested in run 2; run 2's fails in run 3
    assert_eq!(u(&s[1], "due"), n_rej);
    assert_eq!(u(&s[1], "retested"), n_rej);
    assert_eq!(u(&s[1], "pass") + u(&s[1], "fail"), n_rej);
    assert_eq!(
        u(&s[1], "fail_test") + u(&s[1], "fail_other"),
        u(&s[1], "fail")
    );
    assert_eq!(u(&s[2], "due"), u(&s[1], "fail"));
    assert_eq!(u(&s[2], "retested"), u(&s[2], "due"));
    let o = &t["outcomes"];
    assert!(u(o, "recovered") > 0 && u(o, "still_bad") > 0, "{o}");
    assert_eq!(u(o, "recovered"), u(&s[1], "pass") + u(&s[2], "pass"));
    assert_eq!(u(o, "still_bad"), u(&s[2], "fail"));
    assert_eq!(u(o, "not_retested"), 0);
    assert_eq!(u(&s[2], "cum_recovered"), u(o, "recovered"));
    // flows out of run 1's fails add up
    let from1: usize = t["flows"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|f| f["run"] == 0)
        .map(|f| u(f, "n"))
        .sum();
    assert_eq!(from1, n_rej);
    // the traced test is the most common first fail, and the values are the runs' own
    let key = t["tests"][0]["key"].as_str().unwrap();
    let tt = d1.find_test(key).unwrap();
    let t2 = d2.find_test(key).unwrap();
    for p in t["parts"].as_array().unwrap().iter().take(50) {
        let r = &p["runs"];
        let v1 = r[0]["values"][0].as_f64();
        assert_eq!(
            v1,
            Some(d1.value(tt, u(&r[0], "row")) as f64).filter(|v| v.is_finite())
        );
        if let Some(row2) = r[1]["row"].as_u64() {
            assert_eq!(
                r[1]["values"][0].as_f64(),
                Some(d2.value(t2, row2 as usize)).filter(|v| v.is_finite())
            );
            assert_eq!(
                d2.parts[row2 as usize].part_txt,
                d1.parts[u(&r[0], "row")].part_txt
            );
        }
        if p["outcome"] == "recovered" {
            let k = p["recovered_in"].as_u64().unwrap() as usize;
            assert_eq!(r[k]["status"], "pass");
        }
    }
    // part sets in run 1
    assert_eq!(
        reg.rows(0, &d1, "$retest_recovered").unwrap().len(),
        u(o, "recovered")
    );
    assert_eq!(
        reg.rows(0, &d1, "$retest_still_bad").unwrap().len(),
        u(o, "still_bad")
    );
    // run 3 left out: run 2's fails were never retested
    let t = api::run(&reg, "retest_trace", &json!({"runs": [0, 3]})).unwrap();
    assert_eq!(u(&t["outcomes"], "still_bad"), u(&s[1], "fail"));
    // straight from run 1 to run 3: only run 2's rejects were retested there
    let t = api::run(&reg, "retest_trace", &json!({"runs": [0, 4], "tests": key})).unwrap();
    let n3 = reg.get(4).unwrap().0.n_parts();
    assert_eq!(n3, u(&s[1], "fail"));
    assert_eq!(u(&t["outcomes"], "not_retested"), n_rej - n3);
    assert!(api::run(&reg, "retest_trace", &json!({"runs": [0]})).is_err());
    assert!(api::run(&reg, "retest_trace", &json!({"runs": [0, 0]})).is_err());
}

#[test]
fn recovery_zones_and_loadboard_on_a_planted_retest_chain() {
    isolate();
    let d = tempfile::tempdir().unwrap();
    let band = |seed: u64, noise: f64, board: f64, missing: f64| SampleSpec {
        meas_noise: noise,
        meas_seed: seed,
        band_tests: vec![1050],
        loadboard: Some(board),
        loadboard_missing: missing,
        ..SampleSpec::default()
    };
    let r1 = gen(d.path(), "r1.stdf", band(0, 0.0, 1.0, 0.3));
    let f2 = gen(d.path(), "f2.stdf", band(1, 0.3, 2.0, 0.0));
    let f3 = gen(d.path(), "f3.stdf", band(2, 0.3, 1.0, 0.0));
    let reg = api::Registry::app();
    for p in [&r1, &f2, &f3] {
        api::run(&reg, "load", &json!({ "paths": [p.to_string_lossy()] })).unwrap();
    }
    let retest = |from: usize, full: usize, name: &str| {
        api::run(
            &reg,
            "match",
            &json!({"a": from, "b": full, "filter_a": "not pass", "name": name}),
        )
        .unwrap();
        let out = d.path().join(format!("{name}.stdf"));
        api::run(
            &reg,
            "export_stdf",
            &json!({"ds": full, "filter": format!("${name}"), "path": out.to_string_lossy()}),
        )
        .unwrap();
        api::run(&reg, "load", &json!({ "paths": [out.to_string_lossy()] })).unwrap();
    };
    retest(0, 1, "rej1"); // dataset 3 = run 2
    retest(3, 2, "rej2"); // dataset 4 = run 3

    // loadboard: a file-level value, filled in for parts missing it
    let d1 = reg.get(0).unwrap().0;
    assert!(d1.parts.len() > 1000);
    let missing = (0..d1.n_parts())
        .filter(|&r| d1.field(r, "test:5100") == data::FieldValue::Missing)
        .count();
    assert!(
        missing > 300 && missing < 600,
        "{missing} parts without test 5100"
    );
    assert_eq!(
        reg.rows(0, &d1, "loadboard = 1").unwrap().len(),
        d1.n_parts()
    );
    assert_eq!(d1.file_values(5100)[0].missing, missing);
    assert_eq!(d1.file_values(5100)[0].distinct, 1);
    let t = api::run(
        &reg,
        "retest_trace",
        &json!({"runs": [0, 3, 4], "filter": "fail(1050)", "tests": "1050"}),
    )
    .unwrap();
    assert_eq!(t["runs"][0]["loadboard"], json!([1.0]));
    assert_eq!(t["runs"][1]["loadboard"], json!([2.0]));
    assert_eq!(
        t["runs"][0]["loadboard_missing"].as_u64().unwrap() as usize,
        missing
    );
    let boards: Vec<(u64, &str)> = t["by_board"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| (b["run"].as_u64().unwrap(), b["board"].as_str().unwrap()))
        .collect();
    assert_eq!(boards, [(1, "2"), (2, "1")]);

    // the planted bands: marginal and extreme mostly recover, the middle never
    let ti = d1.find_test("1050").unwrap();
    let (lo, hi) = (d1.tests[ti].lo.unwrap(), d1.tests[ti].hi.unwrap());
    let w = hi - lo;
    let rc = &t["recovery"][0];
    let zones = rc["zones"].as_array().unwrap();
    let kinds: Vec<&str> = zones.iter().map(|z| z["kind"].as_str().unwrap()).collect();
    assert_eq!(kinds, ["marginal", "hard_bad", "extreme"], "{zones:?}");
    let z = |k: usize, f: &str| zones[k][f].as_f64().unwrap();
    // the hard-bad zone covers every planted hard-bad part and little else
    // (a marginal part that happened never to recover may sit at its edge)
    let mid: Vec<f64> = rc["points"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["value"].as_f64().unwrap())
        .filter(|v| *v > hi + 0.25 * w && *v < hi + 0.75 * w)
        .collect();
    let (mlo, mhi) = (
        mid.iter().copied().fold(f64::INFINITY, f64::min),
        mid.iter().copied().fold(f64::NEG_INFINITY, f64::max),
    );
    assert!(
        z(1, "lo") <= mlo && z(1, "hi") >= mhi && z(1, "hi") < hi + 0.75 * w,
        "{:?}",
        zones[1]
    );
    assert!(
        mid.len() as f64 >= 0.9 * z(1, "n"),
        "{} of {}",
        mid.len(),
        z(1, "n")
    );
    assert_eq!(z(1, "recovered"), 0.0);
    assert!(z(1, "n") >= 30.0 && z(1, "ci_hi") <= 0.10);
    assert!(z(0, "hi") < hi + 0.15 * w && z(0, "recovered") / z(0, "n") > 0.6);
    assert!(z(2, "lo") > hi + 2.0 * w && z(2, "recovered") / z(2, "n") > 0.6);
    let n_pts = rc["points"].as_array().unwrap().len() as f64;
    assert_eq!(z(0, "n") + z(1, "n") + z(2, "n"), n_pts);
    // per-retest values: more points (a still-bad part counts at every retest)
    let each = api::run(
        &reg,
        "retest_trace",
        &json!({"runs": [0, 3, 4], "filter": "fail(1050)", "tests": "1050", "value_from": "each"}),
    )
    .unwrap();
    assert!(each["recovery"][0]["points"].as_array().unwrap().len() as f64 > n_pts);

    // a configurable loadboard test, and any test as a factor (merged lots)
    api::run(
        &reg,
        "load",
        &json!({"paths": [r1.to_string_lossy(), f2.to_string_lossy()], "merge": true}),
    )
    .unwrap();
    let m = reg.get(5).unwrap().0;
    let g = m.group_by(&m.all_rows(), "loadboard");
    assert_eq!(
        g.iter().map(|x| x.0.as_str()).collect::<Vec<_>>(),
        ["1", "2"]
    );
    assert_eq!(g[0].1.len(), d1.n_parts());
    let sl = m.group_by(&m.all_rows(), "site_loadboard");
    assert_eq!(sl.len(), 8, "4 sites x 2 boards");
    let an = api::run(&reg, "anova", &json!({"ds": 5, "factor": "loadboard"})).unwrap();
    assert_eq!(an["groups"].as_array().unwrap().len(), 2);
    let an = api::run(&reg, "anova", &json!({"ds": 5, "factor": "test:80100"})).unwrap();
    assert_eq!(an["groups"].as_array().unwrap().len(), 3, "three wafers");
    assert_eq!(
        api::run(&reg, "loadboard", &json!({"test": 80100})).unwrap()["test"],
        80100
    );
    assert_eq!(
        reg.rows(5, &m, "loadboard = 1").unwrap().len(),
        0,
        "wafer 1..3, most common per file"
    );
    api::run(&reg, "loadboard", &json!({"test": 5100})).unwrap();
}

#[test]
fn investigation_case_commonality_across_lots() {
    isolate();
    let d = tempfile::tempdir().unwrap();
    let spec = |lot: &str, seed: u64, ms: u64, band: bool, lb: f64| SampleSpec {
        lot_id: lot.into(),
        seed,
        meas_noise: if ms > 0 { 0.3 } else { 0.0 },
        meas_seed: ms,
        band_tests: if band { vec![1050] } else { vec![] },
        loadboard: Some(lb),
        ..SampleSpec::default()
    };
    // lots A and C: run 1, a 100% retest where some good parts fail 1050,
    // and retests of those rejects; lot B: clean, on another loadboard
    let files = [
        ("a1", spec("LOTA", 1, 0, false, 2.0)),
        ("a2", spec("LOTA", 1, 1, true, 2.0)),
        ("b1", spec("LOTB", 7, 0, false, 1.0)),
        ("c1", spec("LOTC", 5, 0, false, 2.0)),
        ("c2", spec("LOTC", 5, 1, true, 2.0)),
        ("a3f", spec("LOTA", 1, 2, true, 2.0)),
        ("c3f", spec("LOTC", 5, 2, true, 2.0)),
    ];
    let reg = api::Registry::app();
    for (name, s) in &files {
        let p = gen(d.path(), &format!("{name}.stdf"), s.clone());
        api::run(&reg, "load", &json!({ "paths": [p.to_string_lossy()] })).unwrap();
    }
    // the reject retests: 7 = lot A's, 8 = lot C's
    for (from, full, name) in [(1, 5, "ra"), (4, 6, "rc")] {
        api::run(
            &reg,
            "match",
            &json!({"a": from, "b": full, "filter_a": "not pass", "name": name}),
        )
        .unwrap();
        let out = d.path().join(format!("{name}.stdf"));
        api::run(
            &reg,
            "export_stdf",
            &json!({"ds": full, "filter": format!("${name}"), "path": out.to_string_lossy()}),
        )
        .unwrap();
        api::run(&reg, "load", &json!({ "paths": [out.to_string_lossy()] })).unwrap();
    }
    let mut lots = vec![];
    for (run1, retest, chain, name) in [(0, 1, 7, "lot A"), (3, 4, 8, "lot C")] {
        // recovery window, outcome sets marked in run 1 too
        let t = api::run(&reg, "retest_trace", &json!({"runs": [retest, chain], "filter": "fail(1050)", "tests": "1050", "also": [run1]})).unwrap();
        let hb = t["recovery"][0]["zones"]
            .as_array()
            .unwrap()
            .iter()
            .find(|z| z["kind"] == "hard_bad")
            .cloned()
            .unwrap_or_else(|| panic!("{name}: no hard-bad zone {}", t["recovery"][0]["zones"]));
        let r1 = reg.get(run1).unwrap().0;
        assert_eq!(
            reg.rows(run1, &r1, "$retest_still_bad").unwrap().len(),
            t["outcomes"]["still_bad"].as_u64().unwrap() as usize,
            "outcomes marked in run 1 by part text"
        );
        let range = format!("{}..{}", hb["lo"], hb["hi"]);
        let cs = api::run(&reg, "candidate_sets", &json!({"a": run1, "b": retest, "filter_a": "pass", "filter_b": "not pass and fail(1050)",
            "test": "1050", "range": range, "name": "case_bad", "marginal": "case_marginal", "confirmed": "case_good"})).unwrap();
        assert!(cs["n_cand"].as_u64().unwrap() >= 20, "{name}: {cs}");
        lots.push(json!({"name": name, "ds": run1, "kind": "issue", "bad": "$case_bad", "good": "$case_good",
            "groups": [{"label": "marginal", "filter": "$case_marginal"}, {"label": "unrecoverable", "filter": "$retest_still_bad"},
                       {"label": "recovered", "filter": "$retest_recovered"}, {"label": "nope", "filter": "$no_such_set"}]}));
    }
    lots.push(
        json!({"name": "lot B", "ds": 2, "kind": "clean", "bad": "fail(1050)", "good": "pass"}),
    );
    let c = api::run(&reg, "commonality", &json!({"lots": lots, "exclude": "500, 80100-80104",
        "x_test": "80101", "y_test": "80102", "wafer_test": "80100", "ox_test": "80103", "oy_test": "80104"})).unwrap();
    let f = |k: &str| {
        c["factors"]
            .as_array()
            .unwrap()
            .iter()
            .find(|f| f["key"] == k)
            .cloned()
            .unwrap()
    };
    // the issue lots ran on loadboard 2, the clean lot on 1
    let lb = f("loadboard");
    assert_eq!(lb["flag_across"]["level"], "LB 2", "{lb}");
    assert_eq!(lb["flag_across"]["issue_share"], 1.0);
    assert_eq!(lb["flag_across"]["clean_share"], 0.0);
    assert!(
        lb["lots"][0]["test"].is_null(),
        "one board per lot: no within-lot test"
    );
    // within-lot factors are tested per issue lot, with profiles for every group
    let site = f("site");
    assert!(site["lots"][0]["test"]["q"].is_number());
    let groups: Vec<&str> = site["lots"][0]["groups"]
        .as_array()
        .unwrap()
        .iter()
        .map(|g| g["label"].as_str().unwrap())
        .collect();
    assert_eq!(
        groups,
        ["bad", "good", "marginal", "unrecoverable", "recovered"],
        "missing sets are skipped"
    );
    for k in ["zone", "reticle", "test_time", "site_loadboard", "wafer"] {
        assert!(f(k)["levels"].as_array().unwrap().len() >= 2, "{k}");
    }
    let mk = c["markers"].as_array().unwrap();
    assert!(!mk.is_empty() && mk[0]["lots"].as_array().unwrap().len() == 3);
    assert_eq!(c["marker_lots"], json!([true, true, false]));

    // the signature screens lot C's unrecoverable parts under their own label
    let s = api::run(&reg, "signature", &json!({"a": 0, "name": "case_bad", "filter": "pass", "exclude": "500, 80100-80104", "confirmed": "case_good",
        "clean": [{"ds": 2, "filter": "pass"}], "stayed": [{"a": 3, "b": 4}],
        "screen": [{"ds": 3, "filter": "$retest_still_bad", "label": "lot C unrecoverable"}]})).unwrap();
    assert_eq!(s["screen"][0]["name"], "lot C unrecoverable");
    let kinds: Vec<&str> = s["sources"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x["kind"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, ["clean", "stayed_good", "confirmed"]);

    // cases are saved as JSON in the settings folder and load back
    let case = json!({"name": "probe case", "test": "1050", "lots": [{"name": "lot A", "run1": "a1.stdf"}]});
    let sv = api::run(&reg, "case_save", &json!({"case": case})).unwrap();
    assert!(sv["path"].as_str().unwrap().ends_with("probecase.json"));
    let ls = api::run(&reg, "case_list", &json!({})).unwrap();
    assert!(ls
        .as_array()
        .unwrap()
        .iter()
        .any(|x| x["path"] == sv["path"]));
    let ld = api::run(&reg, "case_load", &json!({"path": sv["path"]})).unwrap();
    assert_eq!(ld["case"], case);
}

#[test]
fn merge_pieces_of_one_lot() {
    isolate();
    let d = tempfile::tempdir().unwrap();
    let full = gen(d.path(), "lot.stdf", SampleSpec::default());
    let ds = load(&full);
    // the lot's wafers tested in two sessions: split by wafer, then merge back
    let w0 = ds.wafers[0].clone();
    let g = split::SplitBy::Groups {
        groups: vec![("first".into(), format!("wafer = \"{w0}\""))],
        rest: Some("rest".into()),
    };
    let parts = split::split_file(&full, &ds, &g, d.path(), Some("piece")).unwrap();
    assert_eq!(parts.len(), 2);
    let reg = api::Registry::app();
    // a retest of the same lot shares every part: not a piece
    let retest = gen(
        d.path(),
        "retest.stdf",
        SampleSpec {
            meas_noise: 0.3,
            meas_seed: 1,
            ..SampleSpec::default()
        },
    );
    let paths: Vec<String> = parts
        .iter()
        .map(|p| p.path.clone())
        .chain([retest.to_string_lossy().to_string()])
        .collect();
    api::run(&reg, "load", &json!({ "paths": paths, "sort": "name" })).unwrap();
    // one group for the lot, in test-time order; the retest's shared parts
    // are reported but do not block the merge
    let c = api::run(&reg, "merge_candidates", &json!({})).unwrap();
    let c = c.as_array().unwrap();
    assert_eq!(c.len(), 1);
    let g = &c[0];
    assert!(g["mergeable"].as_bool().unwrap());
    assert_eq!(g["by"], "wafer + X/Y");
    assert_eq!(g["files"].as_array().unwrap().len(), 3);
    assert_eq!(
        g["pairs"].as_array().unwrap().len(),
        2,
        "the retest shares parts with both pieces"
    );
    assert_eq!(g["shared"].as_u64().unwrap() as usize, ds.n_parts());
    let out = d.path().join("merged.stdf");
    let r = api::run(
        &reg,
        "merge_files",
        &json!({"ds": [0, 1], "path": out.to_string_lossy()}),
    )
    .unwrap();
    assert_eq!(r["parts"].as_u64().unwrap() as usize, ds.n_parts());
    let m = load(&out);
    assert_eq!(m.n_parts(), ds.n_parts());
    assert_eq!(m.wafers.len(), ds.wafers.len());
    assert_eq!(m.yield_pct(&m.all_rows()), ds.yield_pct(&ds.all_rows()));
    // one header, recounted summaries
    let mut rdr = StdfReader::open(&out).unwrap();
    let (mut mir, mut pcr_all) = (0, None);
    while let Some((_, rec)) = rdr.next_record().unwrap() {
        match rec {
            Record::MIR(_) => mir += 1,
            Record::PCR(p) if p.head_num == 255 => pcr_all = Some(p.part_cnt),
            _ => {}
        }
    }
    assert_eq!(mir, 1);
    assert_eq!(pcr_all, Some(ds.n_parts() as u32));
    // renumbered, in reverse order: the second file continues after the first's last ID
    let out2 = d.path().join("merged_renumbered.stdf");
    api::run(
        &reg,
        "merge_files",
        &json!({"ds": [1, 0], "path": out2.to_string_lossy(), "renumber": true}),
    )
    .unwrap();
    let m2 = load(&out2);
    let ids: Vec<u64> = m2
        .parts
        .iter()
        .map(|p| p.part_id.parse().unwrap())
        .collect();
    let n1 = parts[1].parts as usize;
    let last1 = *ids[..n1].iter().max().unwrap();
    assert!(
        ids[n1..].iter().all(|&i| i > last1),
        "file 2 continues after file 1"
    );
    assert_eq!(
        ids.iter().collect::<std::collections::HashSet<_>>().len(),
        ids.len()
    );
}

#[test]
fn part_datalog_text_and_csv() {
    isolate();
    let d = tempfile::tempdir().unwrap();
    let p = gen(
        d.path(),
        "lot.stdf",
        SampleSpec {
            extra_tests: vec![1013, 2000],
            ..SampleSpec::default()
        },
    );
    let reg = api::Registry::app();
    api::run(&reg, "load", &json!({ "paths": [p.to_string_lossy()] })).unwrap();
    let ds = load(&p);
    let p0 = &ds.parts[0];
    // by part ID, by X/Y, and a pasted list with one unknown line
    let r = api::run(
        &reg,
        "find_parts",
        &json!({"ds": 0, "q": p0.part_id, "by": "part_id"}),
    )
    .unwrap();
    assert_eq!(r["rows"][0]["index"], 0);
    let r = api::run(
        &reg,
        "find_parts",
        &json!({"ds": 0, "q": format!("{},{}", p0.x, p0.y), "by": "xy"}),
    )
    .unwrap();
    assert!(r["rows"]
        .as_array()
        .unwrap()
        .iter()
        .any(|x| x["index"] == 0));
    let q = format!("{}\n{}\nnope", ds.parts[1].part_txt, ds.parts[2].part_txt);
    let r = api::run(
        &reg,
        "find_parts",
        &json!({"ds": 0, "q": q, "by": "part_txt"}),
    )
    .unwrap();
    assert_eq!(r["missing"], json!(["nope"]));
    let rows: Vec<u64> = r["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x["index"].as_u64().unwrap())
        .collect();
    assert!(rows.contains(&1) && rows.contains(&2));
    let txt = api::run(
        &reg,
        "part_datalog",
        &json!({"ds": 0, "rows": [0, 1], "tests": "1013, 2000", "format": "txt"}),
    )
    .unwrap();
    let t = txt["text"].as_str().unwrap();
    assert_eq!(txt["n_tests"], 2);
    assert_eq!(t.matches("DUMMY_1013").count(), 2, "one line per part");
    assert!(t.contains(&format!("Part {}", p0.part_id)));
    let csv = api::run(
        &reg,
        "part_datalog",
        &json!({"ds": 0, "rows": [0, 1, 2], "tests": "1013,2000", "format": "csv"}),
    )
    .unwrap();
    // header, LO and HI limit lines, one row per part
    assert_eq!(csv["text"].as_str().unwrap().lines().count(), 1 + 2 + 3);
    let long = api::run(
        &reg,
        "part_datalog",
        &json!({"ds": 0, "rows": [0, 1, 2], "tests": "1013,2000", "format": "csv_long"}),
    )
    .unwrap();
    assert_eq!(long["text"].as_str().unwrap().lines().count(), 1 + 3 * 2);
    // a test the file lacks prints as N/A; marks label the parts
    let r = api::run(
        &reg,
        "part_datalog",
        &json!({"ds": 0, "rows": [0, 1], "marks": ["A1", ""], "tests": "1013, 424242", "format": "txt"}),
    )
    .unwrap();
    assert_eq!(r["missing"], json!(["424242"]));
    let t = r["text"].as_str().unwrap();
    assert!(t.contains("Mark A1  |  Part") && t.contains("N/A"));
    assert_eq!(t.matches("Mark ").count(), 1, "blank marks are not printed");
    let c = api::run(
        &reg,
        "part_datalog",
        &json!({"ds": 0, "rows": [0], "marks": ["7"], "tests": "1013", "format": "csv"}),
    )
    .unwrap();
    // header, LO and HI limit lines, then the part
    assert!(c["text"]
        .as_str()
        .unwrap()
        .lines()
        .nth(3)
        .unwrap()
        .starts_with("7,"));
}
