//! Timing of the STDF read path on one file, not run by default:
//! STDF_BENCH_FILE=path cargo test --release --test parse_bench -- --ignored --nocapture

use std::time::Instant;
use stdf_cypher::data::loader::{load, LoadOptions};
use stdf_cypher::stdf::{Record, StdfReader};

/// Best of `runs` timings, and the item count of the last run.
fn best<F: FnMut() -> u64>(runs: usize, mut f: F) -> (f64, u64) {
    let mut best = f64::MAX;
    let mut n = 0;
    for _ in 0..runs {
        let t = Instant::now();
        n = f();
        best = best.min(t.elapsed().as_secs_f64());
    }
    (best, n)
}

#[test]
#[ignore]
fn read_phases() {
    let Ok(path) = std::env::var("STDF_BENCH_FILE") else {
        return;
    };
    if let Ok(n) = std::env::var("STDF_BENCH_THREADS") {
        stdf_cypher::data::loader::set_load_threads(n.parse().unwrap());
    }
    let size_mb = std::fs::metadata(&path).unwrap().len() as f64 / 1e6;
    let runs = 3;
    let (t, n) = best(runs, || {
        let mut r = StdfReader::open(&path).unwrap();
        let mut n = 0u64;
        while r.next_raw().unwrap().is_some() {
            n += 1;
        }
        n
    });
    println!(
        "framing only (next_raw)      {t:6.2} s  {n} records  {:.0} MB/s",
        size_mb / t
    );
    let (t, n) = best(runs, || {
        let mut r = StdfReader::open(&path).unwrap();
        let e = r.endian;
        let mut n = 0u64;
        while let Some((h, d)) = r.next_raw().unwrap() {
            if matches!(
                (h.typ, h.sub),
                (1, 10) | (1, 20) | (2, 10) | (5, 10) | (5, 20) | (15, 10) | (15, 20)
            ) {
                let _ = Record::parse(h.typ, h.sub, d, e);
                n += 1;
            }
        }
        n
    });
    println!(
        "framing + Record::parse      {t:6.2} s  {n} records  {:.0} MB/s",
        size_mb / t
    );
    let (t, n) = best(runs, || {
        load(&path, &LoadOptions::default()).unwrap().parts.len() as u64
    });
    println!(
        "full load (dataset)          {t:6.2} s  {n} parts  {:.0} MB/s",
        size_mb / t
    );
}

/// FNV-1a over every field of a loaded dataset; equal fingerprints mean equal datasets.
/// STDF_FINGERPRINT=file1;file2 cargo test --release --test parse_bench fingerprint -- --ignored --nocapture
#[test]
#[ignore]
fn fingerprint() {
    let Ok(list) = std::env::var("STDF_FINGERPRINT") else {
        return;
    };
    if let Ok(n) = std::env::var("STDF_BENCH_THREADS") {
        stdf_cypher::data::loader::set_load_threads(n.parse().unwrap());
    }
    for path in list.split(';').filter(|p| !p.is_empty()) {
        let ds = load(path, &LoadOptions::default()).unwrap();
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        let mut feed = |bytes: &[u8]| {
            for b in bytes {
                h ^= *b as u64;
                h = h.wrapping_mul(0x0100_0000_01b3);
            }
        };
        feed(&serde_json::to_vec(&ds.parts).unwrap());
        feed(&serde_json::to_vec(&ds.tests).unwrap());
        feed(&serde_json::to_vec(&ds.wafers).unwrap());
        feed(&serde_json::to_vec(&ds.hbins).unwrap());
        feed(&serde_json::to_vec(&ds.sbins).unwrap());
        feed(&serde_json::to_vec(&ds.datalog).unwrap());
        feed(&serde_json::to_vec(&ds.sources).unwrap());
        feed(format!("{:?}", ds.part_limits).as_bytes());
        for col in &ds.values {
            for v in col {
                feed(&v.to_bits().to_le_bytes());
            }
        }
        println!(
            "{h:016x}  parts {:>6}  tests {:>4}  {path}",
            ds.parts.len(),
            ds.tests.len()
        );
    }
}
