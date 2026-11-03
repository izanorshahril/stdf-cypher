//! Touchdown and loop estimate (experiment): checked against the touchdowns the
//! dummy generator planted, plus the site yield summary and the API ops behind them.

use serde_json::json;
use std::path::{Path, PathBuf};
use stdf_cypher::api;
use stdf_cypher::data::{self, Dataset, LoadOptions};
use stdf_cypher::sample::{touchdown_file, TouchSpec};
use stdf_cypher::touch::{self, Method};

fn isolate() {
    static HOME: std::sync::OnceLock<tempfile::TempDir> = std::sync::OnceLock::new();
    let h = HOME.get_or_init(|| tempfile::tempdir().unwrap());
    std::env::set_var("STDF_CYPHER_HOME", h.path());
}

/// A dummy file for `spec` and the planted touchdown of each part.
fn make(dir: &Path, name: &str, spec: &TouchSpec) -> (PathBuf, Vec<u32>) {
    let p = dir.join(name);
    let truth = touchdown_file(&p, spec).unwrap();
    (p, truth)
}

fn load(p: &Path) -> Dataset {
    data::load(p, &LoadOptions::default()).unwrap()
}

#[test]
fn the_generator_plants_touchdowns_of_unique_sites_in_site_order() {
    isolate();
    let d = tempfile::tempdir().unwrap();
    let spec = TouchSpec::default();
    let (p, truth) = make(d.path(), "td.stdf", &spec);
    let ds = load(&p);
    assert_eq!(ds.n_parts(), truth.len());
    assert!(truth.len() > 150, "enough parts for the checks");
    let n_touch = truth.iter().max().unwrap() + 1;
    for t in 0..n_touch {
        let sites: Vec<u8> = (0..ds.n_parts())
            .filter(|&r| truth[r] == t)
            .map(|r| ds.parts[r].site)
            .collect();
        assert!(
            sites.windows(2).all(|w| w[0] < w[1]),
            "touchdown {t}: {sites:?}"
        );
        assert!(sites.iter().all(|&s| s >= 1 && s <= spec.sites));
    }
}

#[test]
fn pir_runs_give_the_planted_touchdowns() {
    isolate();
    let d = tempfile::tempdir().unwrap();
    let (p, truth) = make(d.path(), "pir.stdf", &TouchSpec::default());
    let ds = load(&p);
    let t = touch::touches(Some(&p), &ds).unwrap();
    assert_eq!(t.method, Method::Pir);
    assert_eq!(t.touch, truth);
    assert_eq!(t.max_sites, 8);
}

#[test]
fn without_pir_runs_site_order_and_test_time_find_most_boundaries() {
    isolate();
    for seed in [7u64, 1, 2, 3, 4] {
        let d = tempfile::tempdir().unwrap();
        let spec = TouchSpec {
            pir_runs: false,
            seed,
            ..Default::default()
        };
        let (p, truth) = make(d.path(), "seq.stdf", &spec);
        let ds = load(&p);
        // no PIR runs in the file: the estimate must not use them
        let t = touch::touches(Some(&p), &ds).unwrap();
        assert_eq!(t.method, Method::Sequence);
        // compare every pair of neighbouring parts: same touchdown or not
        let agree = (1..truth.len())
            .filter(|&i| (truth[i] == truth[i - 1]) == (t.touch[i] == t.touch[i - 1]))
            .count();
        let share = agree as f64 / (truth.len() - 1) as f64;
        eprintln!(
            "seed {seed}: neighbour pairs agree on {:.2}%, touchdowns {} planted, {} estimated",
            share * 100.0,
            truth.iter().max().unwrap() + 1,
            t.n
        );
        assert!(
            share >= 0.95,
            "seed {seed}: neighbour pairs agree on {:.1}%",
            share * 100.0
        );
    }
}

#[test]
fn a_file_without_a_path_uses_site_order_and_test_time() {
    isolate();
    let d = tempfile::tempdir().unwrap();
    let spec = TouchSpec {
        pir_runs: false,
        ..Default::default()
    };
    let (p, _) = make(d.path(), "nopath.stdf", &spec);
    let ds = load(&p);
    assert_eq!(touch::touches(None, &ds).unwrap().method, Method::Sequence);
}

#[test]
fn the_same_duts_on_every_loop_number_their_repeats() {
    isolate();
    let d = tempfile::tempdir().unwrap();
    let spec = TouchSpec {
        touchdowns: 25,
        dut_pool: 8,
        empty: 0.0,
        ..Default::default()
    };
    let (p, truth) = make(d.path(), "loops.stdf", &spec);
    let ds = load(&p);
    let (key, loops) = touch::loops(&ds, "auto").unwrap();
    assert_eq!(
        key,
        touch::LoopKey::Id,
        "part IDs repeat, texts repeat the same way"
    );
    // every DUT comes back on each of the 25 touchdowns: loops 0..25 per part ID
    for id in 1..=8 {
        let mut seen: Vec<u32> = (0..ds.n_parts())
            .filter(|&r| ds.parts[r].part_id == id.to_string())
            .map(|r| loops[r])
            .collect();
        seen.sort_unstable();
        assert_eq!(seen, (0..25).collect::<Vec<u32>>(), "DUT {id}");
    }
    assert_eq!(truth.iter().max(), Some(&24));
    // with new DUTs in every touchdown there are no repeats to count
    let spec = TouchSpec {
        dut_pool: 0,
        ..spec
    };
    let (q, _) = make(d.path(), "unique.stdf", &spec);
    let (key, loops) = touch::loops(&load(&q), "auto").unwrap();
    assert_eq!(key, touch::LoopKey::Off);
    assert!(loops.iter().all(|&l| l == 0));
}

#[test]
fn the_summary_lists_each_touchdown_with_its_sites_and_loops() {
    isolate();
    let d = tempfile::tempdir().unwrap();
    let (p, truth) = make(d.path(), "sum.stdf", &TouchSpec::default());
    let ds = load(&p);
    let t = touch::touches(Some(&p), &ds).unwrap();
    let (_, loops) = touch::loops(&ds, "auto").unwrap();
    let s = touch::summary(&ds, &t.touch, &loops);
    assert_eq!(s.len(), (truth.iter().max().unwrap() + 1) as usize);
    assert_eq!(s.iter().map(|x| x.parts).sum::<u64>(), ds.n_parts() as u64);
    for x in &s {
        assert!(x.sites.windows(2).all(|w| w[0] < w[1]));
        assert!(x.good + x.bad + x.cut_off <= x.parts);
        assert!(x.test_ms_median.is_some());
    }
}

#[test]
fn site_yield_shows_the_weak_site_and_the_numbering_base_is_applied() {
    isolate();
    let d = tempfile::tempdir().unwrap();
    let spec = TouchSpec {
        touchdowns: 60,
        ..Default::default()
    };
    let (p, _) = make(d.path(), "yield.stdf", &spec);
    let reg = api::Registry::app();
    api::run(&reg, "load", &json!({ "paths": [p.to_string_lossy()] })).unwrap();

    let y = api::run(&reg, "site_yield", &json!({ "ds": 0 })).unwrap();
    let sites = y["sites"].as_array().unwrap();
    let total = y["total"]["parts"].as_u64().unwrap();
    assert_eq!(
        sites
            .iter()
            .map(|s| s["parts"].as_u64().unwrap())
            .sum::<u64>(),
        total
    );
    let yield_of = |site: u64| {
        sites
            .iter()
            .find(|s| s["site"] == site)
            .and_then(|s| s["yield_pct"].as_f64())
            .unwrap()
    };
    assert!(yield_of(3) < 80.0, "weak site 3 at {:.1}%", yield_of(3));
    for site in [1u64, 2, 4, 5, 6, 7, 8] {
        assert!(
            yield_of(site) > 85.0,
            "site {site} at {:.1}%",
            yield_of(site)
        );
    }

    let zero = api::run(&reg, "touch_estimate", &json!({ "ds": 0, "base": 0 })).unwrap();
    let one = api::run(&reg, "touch_estimate", &json!({ "ds": 0, "base": 1 })).unwrap();
    assert_eq!(zero["method"], "PIR runs");
    let min_of = |v: &serde_json::Value| {
        v["touch"]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| x.as_u64().unwrap())
            .min()
            .unwrap()
    };
    assert_eq!(min_of(&zero), 0);
    assert_eq!(min_of(&one), 1);
    assert_eq!(one["loop_key"], "Id");
    assert_eq!(
        one["touchdowns"].as_array().unwrap().len(),
        zero["n_touchdowns"].as_u64().unwrap() as usize
    );
    assert!(api::run(&reg, "touch_estimate", &json!({ "ds": 0, "base": 2 })).is_err());
}
