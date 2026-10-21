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
    // 40 PTR tests + 4 traceability PTRs + 4 MPR pins + 1 FTR
    assert_eq!(ds.n_tests(), 49);
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
    let recent = api::run(&reg, "recent", &json!({})).unwrap();
    assert_eq!(recent.as_array().unwrap().len(), 3);
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
        ("501,502,503", "wafer_xy"),
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
        &json!({"a": 0, "b": 1, "key_a": "500", "filter_b": "t[501] = 1", "name": "w1"}),
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
    let r = api::run(&reg, "split", &json!({"ds": 0, "by": "test", "test": "501", "out_dir": out.to_string_lossy(), "prefix": "w"})).unwrap();
    assert_eq!(r["outputs"].as_array().unwrap().len(), 3);

    // wafer maps rebuilt from logged wafer/X/Y tests
    let w = api::run(&reg, "chart", &json!({"ds": 0, "kind": "wafer", "coords": "tests", "wafer_test": "501", "x_test": "502", "y_test": "503"})).unwrap();
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
