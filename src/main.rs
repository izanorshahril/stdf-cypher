use anyhow::{bail, Context, Result};
use clap::{Args, Parser, Subcommand};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use stdf_cypher::analysis::{anomaly, anova, compare, correlation, summary};
use stdf_cypher::data::{self, loader::DedupMode, Dataset, Filter, LoadOptions};
use stdf_cypher::stdf::{record_code, StdfReader};
use stdf_cypher::{api, ml, report, sample, server, split};

#[derive(Parser)]
#[command(
    name = "stdf-cypher",
    version,
    about = "Open STDF v4 analytics: parse, compare, split, correlate, ANOVA, anomaly detection, ML and interactive visualization",
    after_help = "Run without arguments (or double-click stdf-cypher.exe) to open the desktop app."
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Args, Clone)]
struct LoadArgs {
    /// keep only the first or last test of each physical unit (retest handling)
    #[arg(long, value_parser = ["first", "last"], global = true)]
    dedup: Option<String>,
    /// ignore FTR (functional) records
    #[arg(long, global = true)]
    no_ftr: bool,
    /// ignore MPR (multi-pin) records
    #[arg(long, global = true)]
    no_mpr: bool,
    /// part filter expression, e.g. "site in (1,2) and hbin != 5"
    #[arg(short = 'f', long, global = true, default_value = "")]
    filter: String,
}

impl LoadArgs {
    fn opts(&self) -> LoadOptions {
        LoadOptions {
            functional: !self.no_ftr,
            multi_pin: !self.no_mpr,
            dedup: match self.dedup.as_deref() {
                Some("first") => Some(DedupMode::First),
                Some("last") => Some(DedupMode::Last),
                _ => None,
            },
        }
    }
}

#[derive(Subcommand)]
enum Cmd {
    /// File summary: MIR, record counts, yield, bins, sites
    Info {
        files: Vec<PathBuf>,
        #[arg(long)]
        json: bool,
        #[command(flatten)]
        l: LoadArgs,
    },
    /// Dump records as JSON lines
    Dump {
        file: PathBuf,
        /// record types to include, e.g. PRR,PTR
        #[arg(short, long, value_delimiter = ',')]
        rec: Vec<String>,
        #[arg(short = 'n', long)]
        limit: Option<usize>,
    },
    /// Part results table (PRR) – prints or writes CSV
    Prr {
        files: Vec<PathBuf>,
        #[arg(short, long)]
        out: Option<PathBuf>,
        #[command(flatten)]
        l: LoadArgs,
    },
    /// Per-test statistics (mean, sigma, Cpk, fails)
    Tests {
        files: Vec<PathBuf>,
        #[arg(short, long)]
        out: Option<PathBuf>,
        #[arg(long, default_value_t = 40)]
        top: usize,
        #[command(flatten)]
        l: LoadArgs,
    },
    /// Export parts × tests matrix to CSV
    Export {
        files: Vec<PathBuf>,
        #[arg(short, long)]
        out: PathBuf,
        /// comma-separated tests (default all)
        #[arg(short, long, default_value = "")]
        tests: String,
        #[command(flatten)]
        l: LoadArgs,
    },
    /// Split one STDF into several by part ID / PRR field or named filter groups
    Split {
        file: PathBuf,
        /// part field: part_id, hbin, sbin, site, head, pass, wafer, xy, ...
        #[arg(long, conflicts_with = "group")]
        by: Option<String>,
        /// named group: NAME=EXPR (repeatable, first match wins)
        #[arg(short, long)]
        group: Vec<String>,
        /// name of the output for parts matching no group (omit to drop them)
        #[arg(long)]
        rest: Option<String>,
        #[arg(short, long, default_value = "split_out")]
        out_dir: PathBuf,
        #[arg(long)]
        prefix: Option<String>,
        #[command(flatten)]
        l: LoadArgs,
    },
    /// Write the parts matching --filter to a new STDF file
    Extract {
        file: PathBuf,
        #[arg(short, long)]
        out: PathBuf,
        #[command(flatten)]
        l: LoadArgs,
    },
    /// Compare two STDF files test by test
    Compare {
        a: PathBuf,
        b: PathBuf,
        #[arg(long, default_value = "")]
        filter_a: String,
        #[arg(long, default_value = "")]
        filter_b: String,
        /// match tests by key (num+name), num or name
        #[arg(long, default_value = "key")]
        match_by: String,
        #[arg(long, default_value_t = 25)]
        top: usize,
        #[arg(short, long)]
        out: Option<PathBuf>,
        /// also write an HTML report
        #[arg(long)]
        html: Option<PathBuf>,
        #[command(flatten)]
        l: LoadArgs,
    },
    /// Compare a group of parts against the others (or another group) in one dataset
    Group {
        files: Vec<PathBuf>,
        /// selection expression for the group of interest
        #[arg(short, long)]
        select: String,
        /// reference expression (default: all other parts)
        #[arg(long)]
        vs: Option<String>,
        #[arg(long, default_value_t = 25)]
        top: usize,
        #[arg(short, long)]
        out: Option<PathBuf>,
        #[command(flatten)]
        l: LoadArgs,
    },
    /// Correlation: unit-to-unit between two files (--with), test matrix, or vs a target
    Correlate {
        file: PathBuf,
        /// second file: unit-level (tester-to-tester) correlation
        #[arg(long)]
        with: Option<PathBuf>,
        /// rank tests by correlation with a target: "pass" or a test
        #[arg(long)]
        target: Option<String>,
        #[arg(long, default_value = "pearson")]
        method: String,
        #[arg(long, default_value_t = 0.8)]
        threshold: f64,
        #[arg(long, default_value_t = 100)]
        max_tests: usize,
        /// unit match: auto, part_id, xy
        #[arg(long, default_value = "auto")]
        match_by: String,
        #[arg(long, default_value_t = 0.9)]
        r_min: f64,
        #[arg(long, default_value_t = 5.0)]
        offset_pct: f64,
        #[arg(long, default_value_t = 30)]
        top: usize,
        #[arg(short, long)]
        out: Option<PathBuf>,
        #[command(flatten)]
        l: LoadArgs,
    },
    /// One-way ANOVA / Kruskal–Wallis / Levene of every test by a factor
    Anova {
        files: Vec<PathBuf>,
        /// site, head, hbin, sbin, wafer, file, lot, x, y ...
        #[arg(long, default_value = "site")]
        factor: String,
        #[arg(long, default_value_t = 5)]
        min_group: usize,
        #[arg(long, default_value_t = 0.01)]
        alpha: f64,
        #[arg(long, default_value_t = 25)]
        top: usize,
        #[arg(short, long)]
        out: Option<PathBuf>,
        #[command(flatten)]
        l: LoadArgs,
    },
    /// Abnormal part detection: robust z, PAT, NNR, Mahalanobis, Isolation Forest
    Anomaly {
        files: Vec<PathBuf>,
        #[arg(long, default_value_t = 6.0)]
        robust_k: f64,
        #[arg(long, default_value_t = 6.0)]
        pat_k: f64,
        #[arg(long, default_value_t = 5.0)]
        nnr_k: f64,
        #[arg(long)]
        no_nnr: bool,
        #[arg(long)]
        no_mahalanobis: bool,
        #[arg(long)]
        no_iforest: bool,
        #[arg(long)]
        only_passing: bool,
        #[arg(long, default_value_t = 30)]
        top: usize,
        #[arg(short, long)]
        out: Option<PathBuf>,
        #[command(flatten)]
        l: LoadArgs,
    },
    /// Machine learning: PCA + clustering, random-forest drivers
    Ml {
        #[command(subcommand)]
        cmd: MlCmd,
    },
    /// Write a self-contained interactive HTML report
    Report {
        files: Vec<PathBuf>,
        #[arg(short, long, default_value = "stdf_report.html")]
        out: PathBuf,
        #[arg(long)]
        title: Option<String>,
        /// embed plotly.js so the report works without internet
        #[arg(long)]
        offline: bool,
        #[arg(long, default_value_t = 12)]
        top_tests: usize,
        #[arg(long, default_value = "site")]
        group_by: String,
        #[arg(long)]
        no_ml: bool,
        #[arg(long)]
        no_anomaly: bool,
        #[command(flatten)]
        l: LoadArgs,
    },
    /// Open the desktop app (same as double-clicking the exe)
    App {
        files: Vec<PathBuf>,
        /// don't open a window (for testing / remote use)
        #[arg(long)]
        no_window: bool,
    },
    /// Start the interactive dashboard in your browser
    Serve {
        files: Vec<PathBuf>,
        #[arg(short, long, default_value_t = 8765)]
        port: u16,
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
        /// also load all files merged into one dataset
        #[arg(long)]
        merge: bool,
        #[arg(long)]
        no_open: bool,
        #[command(flatten)]
        l: LoadArgs,
    },
    /// Generate a synthetic STDF file (demo / testing)
    Sample {
        out: PathBuf,
        #[arg(long, default_value = "LOTA")]
        lot: String,
        #[arg(long, default_value_t = 3)]
        wafers: usize,
        #[arg(long, default_value_t = 12)]
        radius: i16,
        #[arg(long, default_value_t = 4)]
        sites: u8,
        #[arg(long, default_value_t = 40)]
        tests: usize,
        #[arg(long, default_value_t = 1)]
        seed: u64,
        /// mean shift in sigma (makes a "different lot")
        #[arg(long, default_value_t = 0.0)]
        shift: f64,
        /// re-measurement noise in sigma (same seed = same parts on another tester)
        #[arg(long, default_value_t = 0.0)]
        meas_noise: f64,
        /// tester offset in sigma on every 3rd test
        #[arg(long, default_value_t = 0.0)]
        tester_offset: f64,
        #[arg(long, default_value_t = 0.004)]
        outlier_rate: f64,
        #[arg(long)]
        big_endian: bool,
        #[arg(long)]
        final_test: bool,
        /// log prefixed units (uA, mV…) as base units (A, V) with RES_SCAL
        #[arg(long)]
        base_units: bool,
        /// another draw of the re-measurement noise (a further retest)
        #[arg(long, default_value_t = 0)]
        meas_seed: u64,
        /// loadboard (DIB) ID written as test 5100
        #[arg(long)]
        loadboard: Option<f64>,
        /// share of parts without the loadboard result
        #[arg(long, default_value_t = 0.0)]
        loadboard_missing: f64,
        /// plant marginal / hard-bad / clamped fails on this test that retests
        /// (--meas-seed) partly recover (repeat for several tests)
        #[arg(long)]
        band_test: Vec<u32>,
        /// add extra parametric tests with these numbers (repeat, e.g. 1013 and 2000)
        #[arg(long)]
        extra_test: Vec<u32>,
        /// the dies --band-test would fail pass this test here, just inside the
        /// high limit (a run 1 before a retest fails them; repeatable)
        #[arg(long)]
        latent_test: Vec<u32>,
    },
}

#[derive(Subcommand)]
enum MlCmd {
    /// PCA with k-means clustering
    Pca {
        files: Vec<PathBuf>,
        #[arg(long, default_value_t = 10)]
        components: usize,
        /// number of clusters (0 = none, omit = auto)
        #[arg(long)]
        clusters: Option<usize>,
        #[arg(long, default_value_t = 150)]
        max_features: usize,
        /// write per-part scores and cluster labels
        #[arg(short, long)]
        out: Option<PathBuf>,
        #[command(flatten)]
        l: LoadArgs,
    },
    /// Random forest: which tests explain pass/fail, a bin, site, or another test
    Explain {
        files: Vec<PathBuf>,
        /// pass | hbin | sbin | site | <test>
        #[arg(long, default_value = "pass")]
        target: String,
        #[arg(long, default_value_t = 100)]
        trees: usize,
        #[arg(long, default_value_t = 12)]
        depth: usize,
        #[arg(long, default_value_t = 25)]
        top: usize,
        #[command(flatten)]
        l: LoadArgs,
    },
}

fn load(files: &[PathBuf], l: &LoadArgs) -> Result<Dataset> {
    if files.is_empty() {
        bail!("no input files");
    }
    let t = std::time::Instant::now();
    let ds = data::load_many(files, &l.opts())?;
    eprintln!(
        "loaded {} part(s), {} test(s) from {} file(s) in {:.2?}",
        ds.n_parts(),
        ds.n_tests(),
        files.len(),
        t.elapsed()
    );
    Ok(ds)
}

fn rows(ds: &Dataset, filter: &str) -> Result<Vec<usize>> {
    let r = Filter::parse(filter, ds)?.select_all(ds);
    if !filter.trim().is_empty() {
        eprintln!("filter selected {} of {} parts", r.len(), ds.n_parts());
    }
    Ok(r)
}

fn f(v: f64) -> String {
    stdf_cypher::viz::fmt(v)
}
fn fo(v: Option<f64>) -> String {
    v.map(f).unwrap_or_else(|| "–".into())
}
fn fp(p: f64) -> String {
    if !p.is_finite() {
        "–".into()
    } else if p < 1e-4 {
        format!("{p:.1e}")
    } else {
        format!("{p:.4}")
    }
}

fn print_table(headers: &[&str], rows: &[Vec<String>]) {
    let mut w: Vec<usize> = headers.iter().map(|h| h.chars().count()).collect();
    for r in rows {
        for (i, c) in r.iter().enumerate() {
            if i < w.len() {
                w[i] = w[i].max(c.chars().count().min(48));
            }
        }
    }
    let line = |cells: Vec<String>| {
        let s: Vec<String> = cells
            .iter()
            .enumerate()
            .map(|(i, c)| {
                let c: String = if c.chars().count() > 48 {
                    c.chars().take(47).collect::<String>() + "…"
                } else {
                    c.clone()
                };
                let pad = w[i].saturating_sub(c.chars().count());
                if i == 0 || c.parse::<f64>().is_err() && !c.ends_with('%') {
                    format!("{c}{}", " ".repeat(pad))
                } else {
                    format!("{}{c}", " ".repeat(pad))
                }
            })
            .collect();
        println!("{}", s.join("  ").trim_end());
    };
    line(headers.iter().map(|s| s.to_string()).collect());
    println!(
        "{}",
        w.iter()
            .map(|&n| "─".repeat(n))
            .collect::<Vec<_>>()
            .join("  ")
    );
    for r in rows {
        line(r.clone());
    }
}

fn write_csv(path: &Path, headers: &[&str], rows: &[Vec<String>]) -> Result<()> {
    let mut w =
        csv::Writer::from_path(path).with_context(|| format!("create {}", path.display()))?;
    w.write_record(headers)?;
    for r in rows {
        w.write_record(r)?;
    }
    w.flush()?;
    eprintln!("wrote {}", path.display());
    Ok(())
}

fn output(out: &Option<PathBuf>, headers: &[&str], rows: &[Vec<String>], top: usize) -> Result<()> {
    match out {
        Some(p) => write_csv(p, headers, rows),
        None => {
            print_table(headers, &rows[..rows.len().min(top)]);
            if rows.len() > top {
                println!("… {} more rows (use -o file.csv for all)", rows.len() - top);
            }
            Ok(())
        }
    }
}

fn main() {
    // Double-click / drag-and-drop / "Open with": no sub-command, only files (or nothing)
    let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    // files or folders (a folder opens its STDF files); a sub-command wins
    // over a folder of the same name (e.g. `tests` in the current folder)
    let is_cmd = args.first().is_some_and(|a| {
        use clap::CommandFactory;
        Cli::command()
            .get_subcommands()
            .any(|c| a.to_str() == Some(c.get_name()))
    });
    let is_app = args.is_empty()
        || !is_cmd
            && args
                .iter()
                .all(|a| Path::new(a).is_file() || Path::new(a).is_dir());
    if is_app {
        let files: Vec<String> = args.iter().map(|a| absolute(Path::new(a))).collect();
        if let Err(e) = run_app(files, true) {
            stdf_cypher::desktop::fatal(&format!("{e:#}"));
            std::process::exit(1);
        }
        return;
    }
    if let Err(e) = run() {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}

fn absolute(p: &Path) -> String {
    std::fs::canonicalize(p)
        .map(|c| {
            let s = c.to_string_lossy().to_string();
            // strip the \\?\ prefix Windows adds
            s.strip_prefix(r"\\?\").map(String::from).unwrap_or(s)
        })
        .unwrap_or_else(|_| p.to_string_lossy().to_string())
}

/// Desktop app: hand files to a running instance, or start a new one with its own window.
fn run_app(files: Vec<String>, window: bool) -> Result<()> {
    use stdf_cypher::desktop;
    if desktop::hand_off_to_running(&files) {
        return Ok(());
    }
    desktop::detach_console_if_owned();
    desktop::clean_uploads();
    let reg = Arc::new(api::Registry::app());
    reg.session.pending_open.lock().unwrap().extend(files); // picked up by the first poll
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    rt.block_on(server::app(reg, window))
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Info { files, json, l } => {
            let ds = load(&files, &l)?;
            let r = rows(&ds, &l.filter)?;
            let ov = summary::overview(&ds);
            if json {
                let v = serde_json::json!({"overview": ov, "sources": ds.sources, "hbins": summary::bins(&ds, &r, true), "sbins": summary::bins(&ds, &r, false), "sites": summary::yield_by(&ds, &r, "site"), "wafers": summary::yield_by(&ds, &r, "wafer")});
                println!("{}", serde_json::to_string_pretty(&v)?);
                return Ok(());
            }
            for s in &ds.sources {
                println!("File      {}  ({} endian)", s.path, s.endian);
                println!(
                    "Lot       {}  sublot {}  product {}",
                    s.lot_id, s.sublot_id, s.part_typ
                );
                println!(
                    "Program   {} {}   tester {} {}   step {}  temp {}",
                    s.job_nam, s.job_rev, s.tstr_typ, s.node_nam, s.test_cod, s.tst_temp
                );
                if s.start_t > 0 {
                    if let Some(t) = chrono::DateTime::from_timestamp(s.start_t as i64, 0) {
                        println!("Start     {}", t.format("%Y-%m-%d %H:%M:%S UTC"));
                    }
                }
                println!(
                    "Records   {}",
                    s.record_counts
                        .iter()
                        .map(|(k, v)| format!("{k}:{v}"))
                        .collect::<Vec<_>>()
                        .join(" ")
                );
                println!();
            }
            println!(
                "Parts {}  pass {}  yield {:.2}%  tests {}  wafers {}  sites {}  retests {}",
                r.len(),
                r.iter().filter(|&&i| ds.parts[i].is_pass()).count(),
                ds.yield_pct(&r),
                ov.n_tests,
                ov.n_wafers,
                ov.sites,
                ov.retests
            );
            println!("\nHard bins");
            let hb: Vec<Vec<String>> = summary::bins(&ds, &r, true)
                .iter()
                .map(|b| {
                    vec![
                        b.bin.to_string(),
                        b.name.clone(),
                        b.pf.clone(),
                        b.count.to_string(),
                        format!("{:.2}%", b.pct),
                    ]
                })
                .collect();
            print_table(&["bin", "name", "P/F", "parts", "%"], &hb);
            println!("\nYield by site");
            let sy: Vec<Vec<String>> = summary::yield_by(&ds, &r, "site")
                .iter()
                .map(|g| {
                    vec![
                        g.group.clone(),
                        g.n.to_string(),
                        g.pass.to_string(),
                        format!("{:.2}%", g.yield_pct),
                    ]
                })
                .collect();
            print_table(&["site", "parts", "pass", "yield"], &sy);
            if ds.wafers.len() > 1 {
                println!("\nYield by wafer");
                let wy: Vec<Vec<String>> = summary::yield_by(&ds, &r, "wafer")
                    .iter()
                    .map(|g| {
                        vec![
                            g.group.clone(),
                            g.n.to_string(),
                            g.pass.to_string(),
                            format!("{:.2}%", g.yield_pct),
                        ]
                    })
                    .collect();
                print_table(&["wafer", "parts", "pass", "yield"], &wy);
            }
        }
        Cmd::Dump { file, rec, limit } => {
            let want: Vec<(u8, u8)> = rec
                .iter()
                .map(|r| record_code(r).with_context(|| format!("unknown record type {r}")))
                .collect::<Result<_>>()?;
            let mut rdr = StdfReader::open(&file)?;
            let mut n = 0usize;
            use std::io::Write;
            let stdout = std::io::stdout();
            let mut out = std::io::BufWriter::new(stdout.lock());
            while let Some((h, r)) = rdr.next_record()? {
                if !want.is_empty() && !want.contains(&(h.typ, h.sub)) {
                    continue;
                }
                if writeln!(out, "{}", serde_json::to_string(&r)?).is_err() {
                    break;
                }
                n += 1;
                if limit.map(|l| n >= l).unwrap_or(false) {
                    break;
                }
            }
        }
        Cmd::Prr { files, out, l } => {
            let ds = load(&files, &l)?;
            let r = rows(&ds, &l.filter)?;
            let table: Vec<Vec<String>> = r
                .iter()
                .map(|&i| {
                    let p = &ds.parts[i];
                    vec![
                        i.to_string(),
                        p.file.to_string(),
                        p.part_id.clone(),
                        ds.wafer_name(p).to_string(),
                        p.head.to_string(),
                        p.site.to_string(),
                        if p.x == -32768 {
                            String::new()
                        } else {
                            p.x.to_string()
                        },
                        if p.y == -32768 {
                            String::new()
                        } else {
                            p.y.to_string()
                        },
                        p.hbin.to_string(),
                        p.sbin.to_string(),
                        p.passed
                            .map(|b| if b { "P" } else { "F" })
                            .unwrap_or("?")
                            .to_string(),
                        format!("0x{:02X}", p.part_flg),
                        p.num_test.to_string(),
                        p.test_t.to_string(),
                        p.first_fail
                            .map(|t| ds.tests[t as usize].key.clone())
                            .unwrap_or_default(),
                    ]
                })
                .collect();
            output(
                &out,
                &[
                    "row",
                    "file",
                    "part_id",
                    "wafer",
                    "head",
                    "site",
                    "x",
                    "y",
                    "hbin",
                    "sbin",
                    "pf",
                    "part_flg",
                    "num_test",
                    "test_t",
                    "first_fail",
                ],
                &table,
                60,
            )?;
        }
        Cmd::Tests { files, out, top, l } => {
            let ds = load(&files, &l)?;
            let r = rows(&ds, &l.filter)?;
            let mut s = summary::tests(&ds, &r);
            s.sort_by(|a, b| {
                b.fail_pct
                    .partial_cmp(&a.fail_pct)
                    .unwrap()
                    .then(a.num.cmp(&b.num))
            });
            let t: Vec<Vec<String>> = s
                .iter()
                .map(|t| {
                    vec![
                        t.key.clone(),
                        t.units.clone(),
                        fo(t.lo),
                        fo(t.hi),
                        t.stats.n.to_string(),
                        f(t.stats.mean),
                        f(t.stats.std),
                        f(t.stats.min),
                        f(t.stats.max),
                        t.stats.cpk.map(|c| format!("{c:.2}")).unwrap_or("–".into()),
                        t.stats.fails.to_string(),
                        format!("{:.2}%", t.fail_pct),
                    ]
                })
                .collect();
            output(
                &out,
                &[
                    "test", "units", "lo", "hi", "n", "mean", "std", "min", "max", "cpk", "fails",
                    "fail%",
                ],
                &t,
                top,
            )?;
        }
        Cmd::Export {
            files,
            out,
            tests,
            l,
        } => {
            let ds = load(&files, &l)?;
            let r = rows(&ds, &l.filter)?;
            let t: Vec<usize> = if tests.is_empty() {
                (0..ds.n_tests()).collect()
            } else {
                tests
                    .split(',')
                    .map(|q| {
                        ds.find_test(q.trim())
                            .with_context(|| format!("test {q} not found"))
                    })
                    .collect::<Result<_>>()?
            };
            std::fs::write(&out, api::export_csv(&ds, &r, &t))?;
            eprintln!(
                "wrote {} rows × {} tests to {}",
                r.len(),
                t.len(),
                out.display()
            );
        }
        Cmd::Split {
            file,
            by,
            group,
            rest,
            out_dir,
            prefix,
            l,
        } => {
            if l.dedup.is_some() {
                bail!("--dedup is not supported for split (every PRR must map to a part)");
            }
            let ds = load(std::slice::from_ref(&file), &l)?;
            let sb = if let Some(by) = by {
                if !l.filter.is_empty() {
                    // filter + field: groups by field among filtered parts
                    let flt = Filter::parse(&l.filter, &ds)?;
                    let mut a = split::assign(&ds, &split::SplitBy::Field(by))?;
                    for (i, x) in a.iter_mut().enumerate() {
                        if !flt.matches(&ds, i) {
                            *x = None;
                        }
                    }
                    let res =
                        split::split_with_assignment(&file, &ds, &a, &out_dir, prefix.as_deref())?;
                    print_split(&res);
                    return Ok(());
                }
                split::SplitBy::Field(by)
            } else if !group.is_empty() {
                let groups = group
                    .iter()
                    .map(|g| {
                        g.split_once('=')
                            .map(|(n, e)| (n.trim().to_string(), e.trim().to_string()))
                            .with_context(|| format!("--group needs NAME=EXPR, got '{g}'"))
                    })
                    .collect::<Result<Vec<_>>>()?;
                split::SplitBy::Groups { groups, rest }
            } else {
                bail!("use --by FIELD or --group NAME=EXPR");
            };
            let res = split::split_file(&file, &ds, &sb, &out_dir, prefix.as_deref())?;
            print_split(&res);
        }
        Cmd::Extract { file, out, l } => {
            if l.filter.trim().is_empty() {
                bail!("--filter is required for extract");
            }
            let ds = load(
                std::slice::from_ref(&file),
                &LoadArgs {
                    dedup: None,
                    ..l.clone()
                },
            )?;
            let flt = Filter::parse(&l.filter, &ds)?;
            let a: Vec<Option<String>> = (0..ds.n_parts())
                .map(|i| flt.matches(&ds, i).then(|| "x".to_string()))
                .collect();
            let dir = std::env::temp_dir().join(format!("stdf-cypher-{}", std::process::id()));
            let res = split::split_with_assignment(&file, &ds, &a, &dir, Some("extract"))?;
            let o = res.first().context("no parts matched the filter")?;
            if let Some(p) = out.parent() {
                if !p.as_os_str().is_empty() {
                    std::fs::create_dir_all(p)?;
                }
            }
            std::fs::copy(&o.path, &out)?;
            let _ = std::fs::remove_dir_all(&dir);
            eprintln!(
                "wrote {} parts ({} good) to {}",
                o.parts,
                o.good,
                out.display()
            );
        }
        Cmd::Compare {
            a,
            b,
            filter_a,
            filter_b,
            match_by,
            top,
            out,
            html,
            l,
        } => {
            let da = load(std::slice::from_ref(&a), &l)?;
            let db = load(std::slice::from_ref(&b), &l)?;
            let ra = rows(&da, &filter_a)?;
            let rb = rows(&db, &filter_b)?;
            let opt = compare::CompareOptions {
                match_by,
                ..Default::default()
            };
            let rep = compare::compare(
                &compare::Side {
                    ds: &da,
                    rows: ra,
                    label: data::short_name(&a.to_string_lossy()),
                },
                &compare::Side {
                    ds: &db,
                    rows: rb,
                    label: data::short_name(&b.to_string_lossy()),
                },
                &opt,
            );
            print_compare(&rep, &out, top)?;
            if let Some(h) = html {
                let merged = Dataset::merge(vec![da.clone(), db.clone()]);
                let page = report::build(
                    &[da, db],
                    &merged,
                    &report::ReportOptions {
                        group_by: "file".into(),
                        ..Default::default()
                    },
                )?;
                std::fs::write(&h, page)?;
                eprintln!("wrote {}", h.display());
            }
        }
        Cmd::Group {
            files,
            select,
            vs,
            top,
            out,
            l,
        } => {
            let ds = load(&files, &l)?;
            let base = rows(&ds, &l.filter)?;
            let g = Filter::parse(&select, &ds)?.select(&ds, &base);
            let set: std::collections::HashSet<usize> = g.iter().copied().collect();
            let other = match &vs {
                Some(v) => Filter::parse(v, &ds)?.select(&ds, &base),
                None => base.iter().copied().filter(|r| !set.contains(r)).collect(),
            };
            eprintln!("group: {} parts, reference: {} parts", g.len(), other.len());
            if g.is_empty() || other.is_empty() {
                bail!("both groups need parts");
            }
            let rep = compare::compare(
                &compare::Side {
                    ds: &ds,
                    rows: other,
                    label: vs.unwrap_or_else(|| "rest".into()),
                },
                &compare::Side {
                    ds: &ds,
                    rows: g,
                    label: select,
                },
                &Default::default(),
            );
            print_compare(&rep, &out, top)?;
        }
        Cmd::Correlate {
            file,
            with,
            target,
            method,
            threshold,
            max_tests,
            match_by,
            r_min,
            offset_pct,
            top,
            out,
            l,
        } => {
            let ds = load(std::slice::from_ref(&file), &l)?;
            let r = rows(&ds, &l.filter)?;
            if let Some(b) = with {
                let db = load(std::slice::from_ref(&b), &l)?;
                let rb = rows(&db, &l.filter)?;
                let rep = correlation::unit_correlation(
                    &ds,
                    &r,
                    &db,
                    &rb,
                    &correlation::UnitCorrOptions {
                        match_by,
                        r_min,
                        offset_pct,
                        min_n: 5,
                    },
                );
                println!(
                    "matched {} parts by {} (only A {}, only B {}), hard-bin agreement {:.1}%",
                    rep.matched, rep.match_by, rep.only_a, rep.only_b, rep.bin_agreement_pct
                );
                println!(
                    "{}\n",
                    rep.summary
                        .iter()
                        .map(|(k, v)| format!("{k} {v}"))
                        .collect::<Vec<_>>()
                        .join("  ·  ")
                );
                let t: Vec<Vec<String>> = rep
                    .tests
                    .iter()
                    .map(|t| {
                        vec![
                            t.status.clone(),
                            t.key.clone(),
                            t.n.to_string(),
                            format!("{:.4}", t.r),
                            format!("{:.3}", t.slope),
                            f(t.mean_diff),
                            t.diff_pct_limit
                                .map(|p| format!("{p:.2}%"))
                                .unwrap_or("–".into()),
                            f(t.sd_diff),
                            f(t.max_abs_diff),
                        ]
                    })
                    .collect();
                output(
                    &out,
                    &[
                        "status",
                        "test",
                        "n",
                        "r",
                        "slope",
                        "mean B-A",
                        "diff %lim",
                        "sd diff",
                        "max |diff|",
                    ],
                    &t,
                    top,
                )?;
            } else if let Some(tg) = target {
                let (tv, excl) = if tg == "pass" || tg == "fail" {
                    (
                        r.iter()
                            .map(|&i| {
                                ds.parts[i]
                                    .passed
                                    .map(|b| if b { 0.0 } else { 1.0 })
                                    .unwrap_or(f64::NAN)
                            })
                            .collect::<Vec<_>>(),
                        None,
                    )
                } else {
                    let t = ds
                        .find_test(&tg)
                        .with_context(|| format!("test {tg} not found"))?;
                    (r.iter().map(|&i| ds.value(t, i)).collect(), Some(t))
                };
                let res = correlation::with_target(&ds, &r, &tv, &method, excl);
                let t: Vec<Vec<String>> = res
                    .iter()
                    .map(|x| {
                        vec![
                            x.key.clone(),
                            format!("{:.4}", x.r),
                            fp(x.p),
                            x.n.to_string(),
                        ]
                    })
                    .collect();
                output(&out, &["test", "r", "p", "n"], &t, top)?;
            } else {
                let tests = correlation::pick_tests(&ds, &r, max_tests);
                let m = correlation::matrix(&ds, &r, &tests, &method, threshold);
                println!(
                    "{} correlation over {} tests; {} pairs with |r| ≥ {threshold}\n",
                    m.method,
                    tests.len(),
                    m.pairs.len()
                );
                let t: Vec<Vec<String>> = m
                    .pairs
                    .iter()
                    .map(|(a, b, r, n)| {
                        vec![a.clone(), b.clone(), format!("{r:.4}"), n.to_string()]
                    })
                    .collect();
                output(&out, &["test A", "test B", "r", "n"], &t, top)?;
            }
        }
        Cmd::Anova {
            files,
            factor,
            min_group,
            alpha,
            top,
            out,
            l,
        } => {
            let ds = load(&files, &l)?;
            let r = rows(&ds, &l.filter)?;
            let rep = anova::anova(&ds, &r, &factor, &ds.numeric_tests(), min_group, alpha);
            println!(
                "factor {factor}: {} groups ({}); {} of {} tests significant at q < {alpha}\n",
                rep.groups.len(),
                rep.groups
                    .iter()
                    .take(8)
                    .map(|(g, n)| format!("{g}:{n}"))
                    .collect::<Vec<_>>()
                    .join(" "),
                rep.n_significant,
                rep.rows.len()
            );
            let t: Vec<Vec<String>> = rep
                .rows
                .iter()
                .map(|x| {
                    vec![
                        x.key.clone(),
                        format!("{:.4}", x.eta2),
                        f(x.f),
                        fp(x.p),
                        fp(x.q),
                        fp(x.kw_p),
                        fp(x.levene_p),
                        x.range_pct_limit
                            .map(|v| format!("{v:.1}%"))
                            .unwrap_or("–".into()),
                    ]
                })
                .collect();
            output(
                &out,
                &[
                    "test",
                    "eta2",
                    "F",
                    "p",
                    "q",
                    "kruskal p",
                    "levene p",
                    "mean range %lim",
                ],
                &t,
                top,
            )?;
        }
        Cmd::Anomaly {
            files,
            robust_k,
            pat_k,
            nnr_k,
            no_nnr,
            no_mahalanobis,
            no_iforest,
            only_passing,
            top,
            out,
            l,
        } => {
            let ds = load(&files, &l)?;
            let r = rows(&ds, &l.filter)?;
            let rep = anomaly::detect(
                &ds,
                &r,
                &anomaly::AnomalyOptions {
                    robust_k,
                    pat_k,
                    nnr_k,
                    nnr: !no_nnr,
                    mahalanobis: !no_mahalanobis,
                    iforest: !no_iforest,
                    only_passing,
                    ..Default::default()
                },
            );
            let mut bm: Vec<_> = rep.by_method.iter().collect();
            bm.sort();
            println!(
                "{} of {} parts flagged ({} tests): {}\n",
                rep.n_flagged,
                rep.n_parts,
                rep.n_tests,
                bm.iter()
                    .map(|(k, v)| format!("{k} {v}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            let t: Vec<Vec<String>> = rep
                .parts
                .iter()
                .map(|p| {
                    vec![
                        p.part_id.clone(),
                        p.wafer.clone(),
                        if p.x == -32768 {
                            "".into()
                        } else {
                            format!("{},{}", p.x, p.y)
                        },
                        p.site.to_string(),
                        p.hbin.to_string(),
                        if p.pass { "P".into() } else { "F".into() },
                        format!("{:.2}", p.score),
                        p.methods.join("+"),
                        format!("{:.1}", p.max_robust_z),
                        f(p.iforest),
                        p.top_tests
                            .first()
                            .map(|t| format!("{} z={:.1}", t.0, t.1))
                            .unwrap_or_default(),
                    ]
                })
                .collect();
            output(
                &out,
                &[
                    "part", "wafer", "x,y", "site", "hbin", "pf", "score", "methods", "max|z|",
                    "iforest", "top test",
                ],
                &t,
                top,
            )?;
        }
        Cmd::Ml { cmd } => match cmd {
            MlCmd::Pca {
                files,
                components,
                clusters,
                max_features,
                out,
                l,
            } => {
                let ds = load(&files, &l)?;
                let r = rows(&ds, &l.filter)?;
                let rep = ml::pca_report(
                    &ds,
                    &r,
                    &ml::features::FeatureOptions {
                        max_features,
                        ..Default::default()
                    },
                    components,
                    clusters,
                )?;
                println!(
                    "PCA on {} parts × {} features",
                    rep.features.n, rep.features.p
                );
                for k in 0..rep.pca.n_components.min(components) {
                    let tl = rep.pca.top_loadings(k, 4);
                    println!(
                        "  PC{:<2} {:5.1}%  (cum {:5.1}%)  {}",
                        k + 1,
                        rep.pca.explained_ratio[k] * 100.0,
                        rep.pca.cumulative[k] * 100.0,
                        tl.iter()
                            .map(|(n, w)| format!("{n} {w:+.2}"))
                            .collect::<Vec<_>>()
                            .join(", ")
                    );
                }
                if let Some(km) = &rep.kmeans {
                    println!(
                        "\nk-means: k = {} (silhouette {:.3}); sizes {:?}",
                        km.k, km.silhouette, km.sizes
                    );
                    if !km.k_scan.is_empty() {
                        println!(
                            "  scan: {}",
                            km.k_scan
                                .iter()
                                .map(|(k, s)| format!("k{k}={s:.3}"))
                                .collect::<Vec<_>>()
                                .join(" ")
                        );
                    }
                    for c in 0..km.k {
                        let rs: Vec<usize> = (0..rep.features.n)
                            .filter(|&i| km.labels[i] == c)
                            .map(|i| rep.features.rows[i])
                            .collect();
                        println!(
                            "  cluster {}: {} parts, yield {:.2}%",
                            c + 1,
                            rs.len(),
                            ds.yield_pct(&rs)
                        );
                    }
                }
                if let Some(o) = out {
                    let mut t = vec![];
                    for (i, &row) in rep.features.rows.iter().enumerate() {
                        let mut v = vec![row.to_string(), ds.parts[row].part_id.clone()];
                        v.extend(rep.pca.scores[i].iter().take(3).map(|x| format!("{x:.5}")));
                        v.push(
                            rep.kmeans
                                .as_ref()
                                .map(|k| (k.labels[i] + 1).to_string())
                                .unwrap_or_default(),
                        );
                        t.push(v);
                    }
                    write_csv(&o, &["row", "part_id", "pc1", "pc2", "pc3", "cluster"], &t)?;
                }
            }
            MlCmd::Explain {
                files,
                target,
                trees,
                depth,
                top,
                l,
            } => {
                let ds = load(&files, &l)?;
                let r = rows(&ds, &l.filter)?;
                let tg = ml::TargetSpec::parse(&target, &ds)?;
                let rep = ml::explain(
                    &ds,
                    &r,
                    &tg,
                    &Default::default(),
                    &ml::forest::ForestOptions {
                        n_trees: trees,
                        max_depth: depth,
                        ..Default::default()
                    },
                )?;
                println!(
                    "random forest ({}) predicting {}: {} train / {} hold-out, {} features",
                    rep.kind, rep.target, rep.n_train, rep.n_test, rep.n_features
                );
                if rep.kind == "classification" {
                    println!(
                        "ROC AUC {:.3}  accuracy {:.2}%  balanced accuracy {:.2}%{}",
                        rep.auc.unwrap_or(f64::NAN),
                        rep.accuracy.unwrap_or(0.0) * 100.0,
                        rep.balanced_accuracy.unwrap_or(0.0) * 100.0,
                        rep.threshold
                            .map(|t| format!("  (threshold P(fail) ≥ {t:.3}, tuned out-of-bag)"))
                            .unwrap_or_default()
                    );
                    println!(
                        "confusion (rows = actual, cols = predicted): {}",
                        rep.class_names.join(" | ")
                    );
                    for (i, row) in rep.confusion.iter().enumerate() {
                        println!(
                            "  {:>12}  {}",
                            rep.class_names[i],
                            row.iter().map(|c| format!("{c:>7}")).collect::<String>()
                        );
                    }
                } else {
                    println!(
                        "R² {:.4}  RMSE {}  MAE {}",
                        rep.r2.unwrap_or(f64::NAN),
                        f(rep.rmse.unwrap_or(f64::NAN)),
                        f(rep.mae.unwrap_or(f64::NAN))
                    );
                }
                println!();
                let t: Vec<Vec<String>> = rep
                    .importance
                    .iter()
                    .map(|(k, v)| vec![k.clone(), format!("{:.2}%", v * 100.0)])
                    .collect();
                print_table(&["test", "importance"], &t[..t.len().min(top)]);
            }
        },
        Cmd::Report {
            files,
            out,
            title,
            offline,
            top_tests,
            group_by,
            no_ml,
            no_anomaly,
            l,
        } => {
            if files.is_empty() {
                bail!("no input files");
            }
            let t = std::time::Instant::now();
            let sets: Vec<Dataset> = files
                .iter()
                .map(|p| data::load(p, &l.opts()))
                .collect::<Result<_>>()?;
            let merged = if sets.len() == 1 {
                sets[0].clone()
            } else {
                Dataset::merge(sets.clone())
            };
            let merged = if l.filter.is_empty() {
                merged
            } else {
                merged.subset(&rows(&merged, &l.filter)?)
            };
            let page = report::build(
                &sets,
                &merged,
                &report::ReportOptions {
                    title,
                    offline,
                    top_tests,
                    group_by,
                    ml: !no_ml,
                    anomaly: !no_anomaly,
                    ..Default::default()
                },
            )?;
            std::fs::write(&out, page)?;
            eprintln!("wrote {} in {:.2?}", out.display(), t.elapsed());
        }
        Cmd::App { files, no_window } => {
            run_app(files.iter().map(|f| absolute(f)).collect(), !no_window)?;
        }
        Cmd::Serve {
            files,
            port,
            host,
            merge,
            no_open,
            l,
        } => {
            let reg = Arc::new(api::Registry::default());
            let opt = l.opts();
            for p in &files {
                let ds = data::load(p, &opt)?;
                eprintln!(
                    "loaded {} ({} parts, {} tests)",
                    p.display(),
                    ds.n_parts(),
                    ds.n_tests()
                );
                reg.add(ds.name.clone(), vec![p.to_string_lossy().to_string()], ds);
            }
            if merge && files.len() > 1 {
                let ds = data::load_many(&files, &opt)?;
                reg.add(
                    format!("merged ({} files)", files.len()),
                    files
                        .iter()
                        .map(|p| p.to_string_lossy().to_string())
                        .collect(),
                    ds,
                );
            }
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()?;
            rt.block_on(server::serve(reg, &host, port, !no_open))?;
        }
        Cmd::Sample {
            out,
            lot,
            wafers,
            radius,
            sites,
            tests,
            seed,
            shift,
            meas_noise,
            tester_offset,
            outlier_rate,
            big_endian,
            final_test,
            base_units,
            meas_seed,
            loadboard,
            loadboard_missing,
            band_test,
            extra_test,
            latent_test,
        } => {
            let spec = sample::SampleSpec {
                lot_id: lot,
                wafers,
                radius,
                sites,
                tests,
                seed,
                shift,
                meas_noise,
                tester_offset,
                outlier_rate,
                endian: if big_endian {
                    stdf_cypher::stdf::Endian::Big
                } else {
                    stdf_cypher::stdf::Endian::Little
                },
                final_test,
                base_units,
                meas_seed,
                loadboard,
                loadboard_missing,
                band_tests: band_test,
                extra_tests: extra_test,
                latent_tests: latent_test,
                ..Default::default()
            };
            let n = sample::generate(&out, &spec)?;
            eprintln!("wrote {} parts to {}", n, out.display());
        }
    }
    Ok(())
}

fn print_split(res: &[split::SplitOutput]) {
    let t: Vec<Vec<String>> = res
        .iter()
        .map(|o| {
            vec![
                o.name.clone(),
                o.parts.to_string(),
                o.good.to_string(),
                o.path.clone(),
            ]
        })
        .collect();
    print_table(&["output", "parts", "good", "file"], &t);
    eprintln!("{} file(s) written", res.len());
}

fn print_compare(rep: &compare::CompareReport, out: &Option<PathBuf>, top: usize) -> Result<()> {
    println!(
        "A = {}: {} parts, yield {:.2}%",
        rep.label_a, rep.n_a, rep.yield_a
    );
    println!(
        "B = {}: {} parts, yield {:.2}%  (Δ {:+.2} pt)",
        rep.label_b,
        rep.n_b,
        rep.yield_b,
        rep.yield_b - rep.yield_a
    );
    println!("{} of {} tests flagged\n", rep.n_flagged, rep.tests.len());
    let t: Vec<Vec<String>> = rep
        .tests
        .iter()
        .map(|t| {
            vec![
                t.flag.clone(),
                t.key.clone(),
                f(t.a.mean),
                f(t.b.mean),
                format!("{:+.2}", t.shift_sigma),
                t.shift_pct_limit
                    .map(|p| format!("{p:+.1}%"))
                    .unwrap_or("–".into()),
                format!("{:.2}", t.sigma_ratio),
                fp(t.q),
                format!("{:.2}%", t.fail_rate_a),
                format!("{:.2}%", t.fail_rate_b),
                t.a.n.to_string(),
                t.b.n.to_string(),
            ]
        })
        .collect();
    output(
        out,
        &[
            "flag",
            "test",
            "mean A",
            "mean B",
            "shift σ",
            "shift %lim",
            "σ ratio",
            "q",
            "fail% A",
            "fail% B",
            "n A",
            "n B",
        ],
        &t,
        top,
    )?;
    if out.is_none() {
        println!("\nHard bins");
        let b: Vec<Vec<String>> = rep
            .hbins
            .iter()
            .map(|b| {
                vec![
                    b.bin.to_string(),
                    b.name.clone(),
                    b.count_a.to_string(),
                    format!("{:.2}%", b.pct_a),
                    b.count_b.to_string(),
                    format!("{:.2}%", b.pct_b),
                    format!("{:+.2}", b.delta_pct),
                ]
            })
            .collect();
        print_table(&["bin", "name", "A", "A %", "B", "B %", "Δ pt"], &b);
    }
    Ok(())
}
