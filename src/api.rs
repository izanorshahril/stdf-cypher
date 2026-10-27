//! JSON operations behind the interactive dashboard. Each op takes a JSON
//! object of parameters and returns JSON (analysis results + Plotly figures).

use crate::analysis::{anomaly, anova, compare, correlation, stats, summary};
use crate::data::{self, Dataset, Filter, LoadOptions};
use crate::ml;
use crate::viz;
use anyhow::{anyhow, bail, Context, Result};
use indexmap::IndexMap;
use serde_json::{json, Value};
use std::sync::{Arc, RwLock};

pub struct Entry {
    pub name: String,
    pub paths: Vec<String>,
    pub ds: Arc<Dataset>,
    /// named part sets (`$name` in filters), one flag per row
    pub sets: IndexMap<String, Arc<Vec<bool>>>,
}

/// The outcome of matching the parts of two datasets, kept for tracing and
/// paired (dual) charts.
pub struct MatchLink {
    pub a: usize,
    pub b: usize,
    pub name: String,
    pub key_a: String,
    pub key_b: String,
    pub pairs: Vec<(usize, usize, String)>,
}

#[derive(Default)]
pub struct Registry {
    /// loaded datasets; removed ones become None so ids stay stable
    pub items: RwLock<Vec<Option<Entry>>>,
    pub session: crate::desktop::Session,
    pub matches: RwLock<Vec<MatchLink>>,
}

impl Registry {
    pub fn app() -> Registry {
        Registry {
            items: RwLock::new(vec![]),
            session: crate::desktop::Session {
                app_mode: true,
                ..Default::default()
            },
            matches: RwLock::new(vec![]),
        }
    }
    pub fn add(&self, name: String, paths: Vec<String>, ds: Dataset) -> usize {
        let mut v = self.items.write().unwrap();
        v.push(Some(Entry {
            name,
            paths,
            ds: Arc::new(ds),
            sets: IndexMap::new(),
        }));
        v.len() - 1
    }
    pub fn get(&self, id: usize) -> Result<(Arc<Dataset>, Vec<String>)> {
        let v = self.items.read().unwrap();
        let e = v
            .get(id)
            .and_then(|e| e.as_ref())
            .ok_or_else(|| anyhow!("dataset {id} is not loaded (it may have been closed)"))?;
        Ok((e.ds.clone(), e.paths.clone()))
    }
    /// Display name of a dataset (as renamed by the user).
    pub fn name(&self, id: usize) -> String {
        self.items
            .read()
            .unwrap()
            .get(id)
            .and_then(|e| e.as_ref())
            .map(|e| e.name.clone())
            .unwrap_or_default()
    }
    fn set(&self, id: usize, name: &str) -> Option<Arc<Vec<bool>>> {
        let v = self.items.read().unwrap();
        v.get(id)?.as_ref()?.sets.get(name).cloned()
    }
    /// Compile a filter for dataset `id`, resolving its `$name` part sets.
    pub fn filter(&self, id: usize, ds: &Dataset, src: &str) -> Result<Filter> {
        Filter::parse_with(src, ds, &|n| self.set(id, n))
    }
    pub fn rows(&self, id: usize, ds: &Dataset, src: &str) -> Result<Vec<usize>> {
        Ok(self.filter(id, ds, src)?.select_all(ds))
    }
    fn put_set(&self, id: usize, name: &str, flags: Vec<bool>) {
        if let Some(Some(e)) = self.items.write().unwrap().get_mut(id) {
            e.sets.insert(name.to_string(), Arc::new(flags));
        }
    }
}

fn str_list(p: &Value, k: &str) -> Vec<String> {
    p.get(k)
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

struct P<'a>(&'a Value);

impl<'a> P<'a> {
    fn str(&self, k: &str) -> String {
        self.0
            .get(k)
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    }
    fn str_or(&self, k: &str, d: &str) -> String {
        let s = self.str(k);
        if s.is_empty() {
            d.to_string()
        } else {
            s
        }
    }
    fn usize(&self, k: &str, d: usize) -> usize {
        self.0
            .get(k)
            .and_then(|v| {
                v.as_u64()
                    .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
            })
            .map(|v| v as usize)
            .unwrap_or(d)
    }
    fn f64(&self, k: &str, d: f64) -> f64 {
        self.0
            .get(k)
            .and_then(|v| {
                v.as_f64()
                    .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
            })
            .unwrap_or(d)
    }
    fn bool(&self, k: &str, d: bool) -> bool {
        self.0.get(k).and_then(|v| v.as_bool()).unwrap_or(d)
    }
    fn opt_usize(&self, k: &str) -> Option<usize> {
        self.0
            .get(k)
            .and_then(|v| {
                v.as_u64()
                    .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
            })
            .map(|v| v as usize)
    }
}

fn test_ref(ds: &Dataset, q: &str) -> Result<usize> {
    if q.is_empty() {
        bail!("no test selected");
    }
    ds.find_test(q)
        .ok_or_else(|| anyhow!("test '{q}' not found"))
}

/// Die coordinates rebuilt from datalogged results: wafer, X and Y tests plus
/// optional per-part X/Y offsets (e.g. reticle origin logged next to X/Y).
struct TestXy {
    tx: usize,
    ty: usize,
    tw: Option<usize>,
    tox: Option<usize>,
    toy: Option<usize>,
    /// +1 adds the offset, −1 subtracts it
    sign: f64,
    flip_x: bool,
    flip_y: bool,
}

impl TestXy {
    fn parse(ds: &Dataset, pp: &P) -> Result<Self> {
        let opt_test = |k: &str, what: &str| -> Result<Option<usize>> {
            let q = pp.str(k);
            if q.trim().is_empty() {
                Ok(None)
            } else {
                Ok(Some(test_ref(ds, &q).context(what.to_string())?))
            }
        };
        Ok(Self {
            tx: test_ref(ds, &pp.str("x_test")).context("X test")?,
            ty: test_ref(ds, &pp.str("y_test")).context("Y test")?,
            tw: opt_test("wafer_test", "wafer test")?,
            tox: opt_test("ox_test", "X offset test")?,
            toy: opt_test("oy_test", "Y offset test")?,
            sign: if pp.str("offset_mode") == "sub" {
                -1.0
            } else {
                1.0
            },
            flip_x: pp.bool("flip_x", false),
            flip_y: pp.bool("flip_y", false),
        })
    }
    fn xy(&self, ds: &Dataset, row: usize) -> Option<(i16, i16)> {
        let off = |t: Option<usize>| t.map(|t| ds.value(t, row)).unwrap_or(0.0);
        let x = ds.value(self.tx, row) + self.sign * off(self.tox);
        let y = ds.value(self.ty, row) + self.sign * off(self.toy);
        let ok = |v: f64| v.is_finite() && v.abs() < 32767.0;
        let (x, y) = (
            if self.flip_x { -x } else { x },
            if self.flip_y { -y } else { y },
        );
        (ok(x) && ok(y)).then(|| (x.round() as i16, y.round() as i16))
    }
    /// Wafer label ("all" without a wafer test); None when the result is missing.
    fn wafer(&self, ds: &Dataset, row: usize) -> Option<String> {
        match self.tw {
            Some(tw) => {
                let v = ds.value(tw, row);
                v.is_finite().then(|| data::matching::num_key(v))
            }
            None => Some("all".to_string()),
        }
    }
    /// Position within the stepping field: X/Y before the offsets are added.
    fn field_xy(&self, ds: &Dataset, row: usize) -> Option<(i64, i64)> {
        self.tox.or(self.toy)?;
        let (x, y) = (ds.value(self.tx, row), ds.value(self.ty, row));
        (x.is_finite() && y.is_finite()).then(|| (x.round() as i64, y.round() as i64))
    }
}

/// Test numbers from a list like "500, 80100-80104".
fn parse_nums(s: &str) -> Result<Vec<u32>> {
    let mut v = vec![];
    for part in s
        .split([',', ' ', ';'])
        .map(str::trim)
        .filter(|p| !p.is_empty())
    {
        let part = part.split(':').next().unwrap_or(part);
        match part.split_once('-') {
            Some((a, b)) => {
                let (a, b): (u32, u32) = (a.trim().parse()?, b.trim().parse()?);
                if b < a || b - a > 100_000 {
                    bail!("bad test range {part}");
                }
                v.extend(a..=b);
            }
            None => v.push(
                part.parse()
                    .with_context(|| format!("'{part}' is not a test number"))?,
            ),
        }
    }
    Ok(v)
}

/// Groups for a `$set` color choice: parts outside the set, then inside it.
fn set_groups(
    reg: &Registry,
    id: usize,
    rows: &[usize],
    by: &str,
) -> Option<Vec<(String, Vec<usize>)>> {
    let name = by.strip_prefix('$')?;
    let flags = reg.set(id, name)?;
    let (inn, out): (Vec<usize>, Vec<usize>) = rows
        .iter()
        .partition(|&&r| flags.get(r).copied().unwrap_or(false));
    Some(
        vec![(format!("not {by}"), out), (by.to_string(), inn)]
            .into_iter()
            .filter(|g| !g.1.is_empty())
            .collect(),
    )
}

fn groups_for(ds: &Dataset, rows: &[usize], by: &str) -> Vec<(String, Vec<usize>)> {
    groups_for_test(ds, rows, by, None)
}

/// Group rows for coloring a chart. Pass/fail groups are named good/bad and
/// alarm groups alarm/no alarm (so they get semantic colors); with a test,
/// "alarm" means an alarm on that test rather than on any test of the part.
fn groups_for_test(
    ds: &Dataset,
    rows: &[usize],
    by: &str,
    t: Option<usize>,
) -> Vec<(String, Vec<usize>)> {
    if by == "alarm" {
        let (a, n): (Vec<usize>, Vec<usize>) = rows.iter().partition(|&&r| match t {
            Some(t) => ds.parts[r].has_alarm(t),
            None => !ds.parts[r].alarms.is_empty(),
        });
        return [("no alarm", n), ("alarm", a)]
            .into_iter()
            .filter(|(_, r)| !r.is_empty())
            .map(|(k, r)| (k.to_string(), r))
            .collect();
    }
    let g = ds.group_by(rows, if by.is_empty() { "all" } else { by });
    let g = g
        .into_iter()
        .map(|(k, r)| {
            (
                match by {
                    "" | "all" => "all".to_string(),
                    "pass" => match k.as_str() {
                        "pass" => "good".to_string(),
                        "fail" => "bad".to_string(),
                        _ => "no result".to_string(),
                    },
                    _ => format!("{by} {k}"),
                },
                r,
            )
        })
        .collect();
    viz::fold_groups(g)
}

/// Group-vs-rest populations within the parts selected by `base`: the group
/// (`sel`) and either `vs` or every other selected part.
fn group_rows(
    reg: &Registry,
    id: usize,
    ds: &Dataset,
    sel: &str,
    vs: &str,
    base: &str,
) -> Result<(Vec<usize>, Vec<usize>)> {
    let base = reg.filter(id, ds, base)?;
    let rb: Vec<usize> = base.select(ds, &reg.rows(id, ds, sel)?);
    let ra = if vs.is_empty() {
        let set: std::collections::HashSet<usize> = rb.iter().copied().collect();
        base.select_all(ds)
            .into_iter()
            .filter(|r| !set.contains(r))
            .collect()
    } else {
        base.select(ds, &reg.rows(id, ds, vs)?)
    };
    Ok((ra, rb))
}

/// Rows and display name of one side of a pairing, keeping only pairs whose
/// rows pass both sides' filters.
fn link_pairs(
    reg: &Registry,
    pp: &P,
) -> Result<(usize, usize, Vec<(usize, usize, String)>, String, String)> {
    let li = pp.usize("link", usize::MAX);
    let m = reg.matches.read().unwrap();
    let l = m
        .get(li)
        .ok_or_else(|| anyhow!("match the two files first"))?;
    let (a, b) = (l.a, l.b);
    let (ka, kb) = (l.key_a.clone(), l.key_b.clone());
    let pairs = l.pairs.clone();
    drop(m);
    let (da, _) = reg.get(a)?;
    let (db, _) = reg.get(b)?;
    let fa = reg.filter(a, &da, &pp.str("filter_a"))?;
    let fb = reg.filter(b, &db, &pp.str("filter_b"))?;
    let pairs = pairs
        .into_iter()
        .filter(|(ra, rb, _)| fa.matches(&da, *ra) && fb.matches(&db, *rb))
        .collect();
    Ok((a, b, pairs, ka, kb))
}

fn part_json(ds: &Dataset, i: usize) -> Value {
    let p = &ds.parts[i];
    json!({"index": i, "part_id": p.part_id, "part_txt": p.part_txt, "file": p.file, "wafer": ds.wafer_name(p), "head": p.head, "site": p.site,
        "x": if p.x == -32768 { Value::Null } else { json!(p.x) }, "y": if p.y == -32768 { Value::Null } else { json!(p.y) },
        "hbin": p.hbin, "sbin": p.sbin, "pass": p.passed, "alarm": !p.alarms.is_empty(), "num_test": p.num_test, "test_t": p.test_t,
        "first_fail": p.first_fail.map(|f| ds.tests[f as usize].key.clone()).unwrap_or_default()})
}

/// One raw STDF record as JSON, without the serde tag.
fn record_json(rec: &crate::stdf::Record) -> Value {
    let mut v = serde_json::to_value(rec).unwrap_or(Value::Null);
    if let Value::Object(m) = &mut v {
        m.remove("rec");
    }
    v
}

fn n(v: f64) -> Value {
    if v.is_finite() {
        json!(v)
    } else {
        Value::Null
    }
}

fn on(v: Option<f64>) -> Value {
    v.map(n).unwrap_or(Value::Null)
}

pub fn run(reg: &Registry, op: &str, p: &Value) -> Result<Value> {
    let pp = P(p);
    match op {
        "datasets" => {
            let v = reg.items.read().unwrap();
            Ok(json!(v.iter().enumerate().filter_map(|(i, e)| e.as_ref().map(|e| json!({
                "id": i, "name": e.name, "paths": e.paths, "n_parts": e.ds.n_parts(), "n_tests": e.ds.n_tests(),
                "yield": n(e.ds.yield_pct(&e.ds.all_rows())), "wafers": e.ds.wafers, "files": e.ds.sources.len(), "derived": e.paths.is_empty(),
                "lots": e.ds.sources.iter().map(|s| s.lot_id.clone()).collect::<Vec<_>>(),
            }))).collect::<Vec<_>>()))
        }
        "remove" => {
            let id = pp.usize("ds", usize::MAX);
            let mut v = reg.items.write().unwrap();
            match v.get_mut(id) {
                Some(slot) if slot.is_some() => {
                    *slot = None;
                    Ok(json!({"removed": id}))
                }
                _ => bail!("dataset {id} not loaded"),
            }
        }
        "info" => Ok(json!({
            "app": "stdf-cypher",
            "version": env!("CARGO_PKG_VERSION"),
            "app_mode": reg.session.app_mode,
            "native_dialogs": crate::desktop::dialogs_available(),
            "config_dir": crate::desktop::config_dir(),
            "os": std::env::consts::OS,
        })),
        "ping" => {
            reg.session.ping();
            let pending: Vec<String> =
                std::mem::take(&mut *reg.session.pending_open.lock().unwrap());
            Ok(json!({"app": "stdf-cypher", "pending": pending}))
        }
        "bye" => {
            reg.session.bye();
            Ok(json!({}))
        }
        "open" => {
            // another launch (file dropped on the exe / "Open with") hands us files
            reg.session.queue_open(str_list(p, "paths"));
            Ok(json!({"app": "stdf-cypher"}))
        }
        "recent" => {
            let s = reg.session.synced_settings();
            Ok(json!(s.recent.iter().map(|r| {
                let m = std::fs::metadata(&r.path).ok();
                json!({"path": r.path, "name": data::short_name(&r.path), "exists": m.is_some(), "size": m.map(|m| m.len()).unwrap_or(0), "opened": r.opened})
            }).collect::<Vec<_>>()))
        }
        "forget_recent" => {
            let path = pp.str("path");
            let mut s = reg.session.synced_settings();
            if path.is_empty() {
                s.recent.clear();
            } else {
                s.recent.retain(|r| r.path != path);
            }
            s.save();
            Ok(json!({}))
        }
        "pick_files" => {
            if !crate::desktop::dialogs_available() {
                bail!("native file dialogs are not available on this platform – drag files into the window instead");
            }
            let dir = reg
                .session
                .settings
                .lock()
                .unwrap()
                .recent
                .first()
                .and_then(|r| {
                    std::path::Path::new(&r.path)
                        .parent()
                        .map(|p| p.to_path_buf())
                });
            let files = crate::desktop::pick_files(dir);
            Ok(
                json!({"paths": files.iter().map(|p| p.to_string_lossy().to_string()).collect::<Vec<_>>()}),
            )
        }
        "pick_folder" => {
            if !crate::desktop::dialogs_available() {
                bail!("native dialogs are not available on this platform – type a path instead");
            }
            Ok(
                json!({"path": crate::desktop::pick_folder(&pp.str_or("title", "Choose a folder")).map(|p| p.to_string_lossy().to_string())}),
            )
        }
        "pick_save" => {
            if !crate::desktop::dialogs_available() {
                bail!("native dialogs are not available on this platform – type a path instead");
            }
            Ok(
                json!({"path": crate::desktop::pick_save(&pp.str_or("title", "Save as"), &pp.str("name"), &pp.str("ext")).map(|p| p.to_string_lossy().to_string())}),
            )
        }
        "reveal" => {
            crate::desktop::reveal(&pp.str("path"))?;
            Ok(json!({}))
        }
        "sample" => Ok(json!({"paths": crate::desktop::make_samples()?})),
        "split" => {
            let id = pp.usize("ds", 0);
            let (ds, paths) = reg.get(id)?;
            if paths.len() != 1 || ds.sources.len() != 1 {
                bail!("splitting works on datasets loaded from a single file");
            }
            let out_dir = pp.str("out_dir");
            if out_dir.is_empty() {
                bail!("choose an output folder");
            }
            let by = pp.str("by");
            let filter = pp.str("filter");
            let rest = pp.str("rest");
            let rest = (!rest.is_empty()).then_some(rest);
            let mut assign = match by.as_str() {
                "groups" => {
                    let groups: Vec<(String, String)> = p
                        .get("groups")
                        .and_then(|v| v.as_array())
                        .map(|a| {
                            a.iter()
                                .filter_map(|g| {
                                    Some((
                                        g.get("name")?.as_str()?.trim().to_string(),
                                        g.get("expr")?.as_str()?.trim().to_string(),
                                    ))
                                })
                                .filter(|(n, e)| !n.is_empty() && !e.is_empty())
                                .collect()
                        })
                        .unwrap_or_default();
                    if groups.is_empty() {
                        bail!("add at least one group (name + filter expression)");
                    }
                    let filters: Vec<(String, Filter)> = groups
                        .iter()
                        .map(|(n, e)| Ok((n.clone(), reg.filter(id, &ds, e)?)))
                        .collect::<Result<_>>()?;
                    (0..ds.n_parts())
                        .map(|r| {
                            filters
                                .iter()
                                .find(|(_, f)| f.matches(&ds, r))
                                .map(|(n, _)| n.clone())
                                .or_else(|| rest.clone())
                        })
                        .collect()
                }
                "test" => {
                    // one output per distinct result of a PTR (e.g. a datalogged wafer number)
                    let t = test_ref(&ds, &pp.str("test"))?;
                    let num = ds.tests[t].num;
                    (0..ds.n_parts())
                        .map(|r| {
                            let v = ds.value(t, r);
                            if v.is_finite() {
                                Some(format!("t{num}_{}", data::matching::num_key(v)))
                            } else {
                                rest.clone()
                            }
                        })
                        .collect()
                }
                "list" => {
                    // parts named in a pasted list, matched on a PRR field or a test value
                    let key =
                        data::matching::KeySpec::parse(&pp.str_or("list_key", "part_id"), &ds)?;
                    let wanted: std::collections::HashSet<String> = pp
                        .str("list")
                        .split(['\n', '\r', ',', '\t'])
                        .map(|s| s.trim())
                        .filter(|s| !s.is_empty())
                        .map(data::matching::norm_key)
                        .collect();
                    if wanted.is_empty() {
                        bail!("paste at least one part ID or value");
                    }
                    let name = pp.str_or("list_name", "listed");
                    (0..ds.n_parts())
                        .map(|r| {
                            if key.key(&ds, r).is_some_and(|k| wanted.contains(&k)) {
                                Some(name.clone())
                            } else {
                                rest.clone()
                            }
                        })
                        .collect()
                }
                _ => crate::split::assign(
                    &ds,
                    &crate::split::SplitBy::Field(if by.is_empty() { "hbin".into() } else { by }),
                )?,
            };
            if !filter.trim().is_empty() {
                let f = reg.filter(id, &ds, &filter)?;
                for (i, a) in assign.iter_mut().enumerate() {
                    if !f.matches(&ds, i) {
                        *a = None;
                    }
                }
            }
            if assign.iter().all(|a| a.is_none()) {
                bail!("no parts were assigned to any output");
            }
            let prefix = pp.str("prefix");
            let res = crate::split::split_with_assignment(
                std::path::Path::new(&paths[0]),
                &ds,
                &assign,
                std::path::Path::new(&out_dir),
                (!prefix.is_empty()).then_some(prefix.as_str()),
            )?;
            Ok(json!({"outputs": res, "out_dir": out_dir}))
        }
        "save_report" => {
            let id = pp.usize("ds", 0);
            let (ds, _) = reg.get(id)?;
            let path = pp.str("path");
            if path.is_empty() {
                bail!("choose where to save the report");
            }
            let r = reg.rows(id, &ds, &pp.str("filter"))?;
            let sub = if r.len() == ds.n_parts() {
                (*ds).clone()
            } else {
                ds.subset(&r)
            };
            // per-file datasets for the file-vs-file sections
            let files: Vec<Dataset> = if ds.sources.len() > 1 {
                (0..ds.sources.len() as u16)
                    .map(|f| {
                        let rs: Vec<usize> = (0..sub.n_parts())
                            .filter(|&i| sub.parts[i].file == f)
                            .collect();
                        let mut d = sub.subset(&rs);
                        d.sources = vec![ds.sources[f as usize].clone()];
                        d.parts.iter_mut().for_each(|p| p.file = 0);
                        d
                    })
                    .collect()
            } else {
                vec![sub.clone()]
            };
            let html = crate::report::build(
                &files,
                &sub,
                &crate::report::ReportOptions {
                    offline: true,
                    title: (!pp.str("title").is_empty()).then(|| pp.str("title")),
                    ..Default::default()
                },
            )?;
            std::fs::write(&path, html).with_context(|| format!("write {path}"))?;
            Ok(json!({"path": path}))
        }
        "export_csv_file" => {
            let path = pp.str("path");
            if path.is_empty() {
                bail!("choose where to save the CSV");
            }
            let csv = export_csv_op(reg, p)?;
            std::fs::write(&path, csv).with_context(|| format!("write {path}"))?;
            Ok(json!({"path": path}))
        }
        "load" => {
            let paths: Vec<String> = p
                .get("paths")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            if paths.is_empty() {
                bail!("no paths given");
            }
            // folders: their STDF files only; then the chosen order
            let (paths, skipped) = data::expand_paths(&paths);
            if paths.is_empty() {
                bail!("no STDF files found ({skipped} other file(s) skipped)");
            }
            let sort = pp.str("sort");
            let opt = LoadOptions {
                dedup: match pp.str("dedup").as_str() {
                    "last" => Some(data::loader::DedupMode::Last),
                    "first" => Some(data::loader::DedupMode::First),
                    _ => None,
                },
                ..Default::default()
            };
            let mut ids = vec![];
            let mut errors = vec![];
            let mut ok_paths = vec![];
            if pp.bool("merge", false) && paths.len() > 1 {
                // start times are only known after loading: merge by file time then
                let mut paths = paths.clone();
                data::sort_opened(&mut paths, &sort, |p| p, |_| None);
                let ds = data::load_many(&paths, &opt)?;
                ids.push(reg.add(format!("merged: {}", ds.name), paths.clone(), ds));
                ok_paths = paths.clone();
            } else {
                use rayon::prelude::*;
                let mut results: Vec<(String, Result<Dataset>)> = paths
                    .par_iter()
                    .map(|p| (p.clone(), data::load(p, &opt)))
                    .collect();
                data::sort_opened(
                    &mut results,
                    &sort,
                    |x| &x.0,
                    |x| {
                        x.1.as_ref()
                            .ok()
                            .and_then(|d| d.sources.first())
                            .map(|s| s.start_t)
                    },
                );
                for (path, r) in results {
                    match r {
                        Ok(ds) => {
                            ids.push(reg.add(ds.name.clone(), vec![path.clone()], ds));
                            ok_paths.push(path);
                        }
                        Err(e) => errors.push(json!({"path": path, "name": data::short_name(&path), "error": format!("{e:#}")})),
                    }
                }
                if ids.is_empty() {
                    bail!(
                        "{}",
                        errors
                            .iter()
                            .map(|e| format!(
                                "{}: {}",
                                e["name"].as_str().unwrap_or(""),
                                e["error"].as_str().unwrap_or("")
                            ))
                            .collect::<Vec<_>>()
                            .join("\n")
                    );
                }
            }
            let paths = ok_paths;
            reg.session.remember(&paths);
            Ok(json!({"ids": ids, "errors": errors, "skipped": skipped}))
        }
        "overview" => {
            let id = pp.usize("ds", 0);
            let (ds, _) = reg.get(id)?;
            let r = reg.rows(id, &ds, &pp.str("filter"))?;
            let ov = summary::overview(&ds);
            let hb = summary::bins(&ds, &r, true);
            let sb = summary::bins(&ds, &r, false);
            let ys = summary::yield_by(&ds, &r, "site");
            let yw = summary::yield_by(&ds, &r, "wafer");
            let yf = summary::yield_by(&ds, &r, "file");
            let fp: Vec<summary::FailPareto> =
                summary::fail_pareto(&ds, &r).into_iter().take(50).collect();
            let mut figs = json!({
                "hbin": viz::bin_pareto(&hb, true),
                "sbin": viz::bin_pareto(&sb, false),
                "site": viz::yield_bar(&ys, "Yield by site", "site"),
            });
            if ds.wafers.len() > 1 {
                figs["wafer"] = viz::yield_bar(&yw, "Yield by wafer", "wafer");
            }
            if ds.sources.len() > 1 {
                figs["file"] = viz::yield_bar(&yf, "Yield by file", "file");
            }
            if !ds.wafers.is_empty() {
                figs["wafermap"] =
                    viz::wafer_map(&ds, &r, "hbin", None, "Hard-bin map (all wafers stacked)");
            }
            Ok(json!({
                "overview": ov, "selected": r.len(), "selected_yield": n(ds.yield_pct(&r)),
                "hbins": hb, "sbins": sb, "fail_pareto": fp, "sources": ds.sources, "figures": figs
            }))
        }
        "tests" => {
            let id = pp.usize("ds", 0);
            let (ds, _) = reg.get(id)?;
            let r = reg.rows(id, &ds, &pp.str("filter"))?;
            let s = summary::tests(&ds, &r);
            Ok(json!(s.iter().map(|t| json!({
                "key": t.key, "num": t.num, "name": t.name, "kind": t.kind, "units": t.units, "lo": on(t.lo), "hi": on(t.hi),
                "res_scal": ds.tests[ds.test_index[&t.key]].res_scal,
                "disp_units": crate::analysis::candidates::display_unit(&t.units, ds.tests[ds.test_index[&t.key]].res_scal).0,
                "n": t.stats.n, "mean": n(t.stats.mean), "std": n(t.stats.std), "min": n(t.stats.min), "max": n(t.stats.max),
                "median": n(t.stats.median), "cpk": on(t.stats.cpk), "fails": t.stats.fails, "fail_pct": n(t.fail_pct)
            })).collect::<Vec<_>>()))
        }
        "parts" => {
            let id = pp.usize("ds", 0);
            let (ds, _) = reg.get(id)?;
            let r = reg.rows(id, &ds, &pp.str("filter"))?;
            let limit = pp.usize("limit", 5000);
            Ok(
                json!({"total": r.len(), "rows": r.iter().take(limit).map(|&i| part_json(&ds, i)).collect::<Vec<_>>()}),
            )
        }
        "rename" => {
            let id = pp.usize("ds", usize::MAX);
            let name = pp.str("name").trim().to_string();
            if name.is_empty() {
                bail!("enter a name");
            }
            let mut v = reg.items.write().unwrap();
            match v.get_mut(id) {
                Some(Some(e)) => {
                    e.name = name.clone();
                    Ok(json!({"id": id, "name": name}))
                }
                _ => bail!("dataset {id} not loaded"),
            }
        }
        "records" => records_op(reg, &pp),
        "part_records" => part_records_op(reg, &pp),
        "match" => match_op(reg, &pp),
        "candidate_sets" => candidate_sets_op(reg, &pp),
        "match_dual" => match_dual_op(reg, &pp),
        "trace" => trace_op(reg, &pp),
        "markers" => markers_op(reg, &pp),
        "candidates" => candidates_op(reg, &pp),
        "signature" => signature_op(reg, &pp),
        "retest_trace" => retest_op(reg, &pp),
        "merge_candidates" => merge_candidates_op(reg),
        // part datalog: search parts, then write the chosen tests' results
        "find_parts" => {
            let id = pp.usize("ds", 0);
            let (ds, _) = reg.get(id)?;
            let limit = pp.usize("limit", 200);
            // ticked modes: part_id, xy, part_txt, test_xy (die X/Y from tests)
            let by = pp.str_or("by", "part_id,xy,part_txt");
            let modes: Vec<&str> = by
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .collect();
            let txy = if modes.contains(&"test_xy") {
                match TestXy::parse(&ds, &pp) {
                    Ok(t) => Some(t),
                    Err(e) if modes == ["test_xy"] => {
                        return Err(
                            e.context("die X/Y from tests (set the tests on the Settings page)")
                        )
                    }
                    Err(_) => None,
                }
            } else {
                None
            };
            let custom = |r: usize| -> Option<(String, i16, i16)> {
                let t = txy.as_ref()?;
                let (x, y) = t.xy(&ds, r)?;
                Some((t.wafer(&ds, r).unwrap_or_default(), x, y))
            };
            let custom_ref: Option<crate::datalog::CustomXy> =
                if txy.is_some() { Some(&custom) } else { None };
            let mut rows = vec![];
            // one query per line (a pasted list); every hit of each
            let mut missing = vec![];
            for q in pp.str("q").lines().map(str::trim).filter(|q| !q.is_empty()) {
                let hits = crate::datalog::find_parts(&ds, q, &modes, custom_ref, limit);
                if hits.is_empty() {
                    missing.push(q.to_string());
                }
                rows.extend(hits);
            }
            let mut seen = std::collections::HashSet::new();
            rows.retain(|r| seen.insert(*r));
            Ok(json!({
                "rows": rows.iter().take(limit).map(|&i| {
                    let mut v = part_json(&ds, i);
                    if let Some((w, x, y)) = custom(i) {
                        v["test_xy"] = json!({"wafer": w, "x": x, "y": y});
                    }
                    v
                }).collect::<Vec<_>>(),
                "total": rows.len(), "missing": missing,
            }))
        }
        "part_datalog" => {
            let id = pp.usize("ds", 0);
            let (ds, _) = reg.get(id)?;
            // rows and their marks, dropping rows outside the dataset
            let marks_in: Vec<String> =
                pp.0.get("marks")
                    .and_then(|v| v.as_array())
                    .map(|a| {
                        a.iter()
                            .map(|x| x.as_str().unwrap_or("").to_string())
                            .collect()
                    })
                    .unwrap_or_default();
            let (rows, marks): (Vec<usize>, Vec<String>) = id_list(pp.0.get("rows"))
                .into_iter()
                .enumerate()
                .filter(|&(_, r)| r < ds.n_parts())
                .map(|(i, r)| (r, marks_in.get(i).cloned().unwrap_or_default()))
                .unzip();
            if rows.is_empty() {
                bail!("add parts to the list first");
            }
            let cols = crate::datalog::resolve_tests(&ds, &pp.str("tests"));
            let text =
                crate::datalog::build(&ds, &rows, &marks, &cols, &pp.str_or("format", "txt"));
            let path = pp.str("path");
            if !path.is_empty() {
                std::fs::write(&path, &text).with_context(|| format!("write {path}"))?;
            }
            let missing: Vec<&str> = cols
                .iter()
                .filter_map(|c| match c {
                    crate::datalog::Col::Missing(q) => Some(q.as_str()),
                    _ => None,
                })
                .collect();
            Ok(
                json!({"text": text, "path": path, "n_parts": rows.len(), "n_tests": cols.len(), "missing": missing}),
            )
        }
        "merge_files" => {
            let ids = id_list(pp.0.get("ds"));
            let out = pp.str("path");
            if out.is_empty() {
                bail!("choose where to save the merged file");
            }
            let mut paths = vec![];
            for id in ids {
                let (_, p) = reg.get(id)?;
                if p.len() != 1 {
                    bail!(
                        "{} is not a single file: merge files as they were opened",
                        reg.name(id)
                    );
                }
                paths.extend(p);
            }
            Ok(json!(crate::merge::merge_files(
                &paths,
                std::path::Path::new(&out),
                pp.bool("renumber", false)
            )?))
        }
        "commonality" => commonality_op(reg, &pp),
        "case_save" | "case_load" | "case_list" => case_op(&pp, op),
        "loadboard" => {
            if let Some(t) = pp.opt_usize("test") {
                data::set_loadboard_test(t as u32);
            }
            Ok(json!({"test": data::loadboard_test()}))
        }
        "subset_ds" => subset_op(reg, &pp),
        "part_detail" => {
            let id = pp.usize("ds", 0);
            let (ds, _) = reg.get(id)?;
            let i = pp.usize("index", 0);
            if i >= ds.n_parts() {
                bail!("no such part");
            }
            let all = ds.all_rows();
            let tests: Vec<Value> = (0..ds.n_tests()).filter_map(|t| {
                let v = ds.value(t, i);
                if !v.is_finite() {
                    return None;
                }
                let info = &ds.tests[t];
                let vals = ds.values_of(t, Some(&all));
                let (m, s) = (stats::median(&vals), stats::robust_sigma(&vals));
                Some(json!({"key": info.key, "units": info.units, "value": v, "lo": on(info.lo), "hi": on(info.hi), "pass": info.within_limits(v), "robust_z": if s > 0.0 { n((v - m) / s) } else { Value::Null }}))
            }).collect();
            Ok(json!({"part": ds.parts[i], "wafer": ds.wafer_name(&ds.parts[i]), "tests": tests}))
        }
        "chart" => chart(reg, &pp),
        "compare" => {
            let a_id = pp.usize("a", 0);
            let b_id = pp.usize("b", a_id);
            let (da, _) = reg.get(a_id)?;
            let (db, _) = reg.get(b_id)?;
            let mode = pp.str_or("mode", "datasets");
            let (ra, rb, la, lb) = if mode == "group" {
                let sel = pp.str("select");
                if sel.is_empty() {
                    bail!("enter a selection expression for the group");
                }
                let vs = pp.str("vs");
                let (ra, rb) = group_rows(reg, a_id, &da, &sel, &vs, &pp.str("filter"))?;
                (
                    ra,
                    rb,
                    if vs.is_empty() {
                        "rest".to_string()
                    } else {
                        vs
                    },
                    sel,
                )
            } else {
                (
                    reg.rows(a_id, &da, &pp.str("filter_a"))?,
                    reg.rows(b_id, &db, &pp.str("filter_b"))?,
                    format!("A: {}", reg.name(a_id)),
                    format!("B: {}", reg.name(b_id)),
                )
            };
            let db_ref: &Dataset = if mode == "group" { &da } else { &db };
            let opt = compare::CompareOptions {
                match_by: pp.str_or("match_by", "key"),
                sigma_threshold: pp.f64("sigma", 0.5),
                limit_pct_threshold: pp.f64("limit_pct", 5.0),
                alpha: pp.f64("alpha", 0.01),
                ..Default::default()
            };
            let rep = compare::compare(
                &compare::Side {
                    ds: &da,
                    rows: ra,
                    label: la,
                },
                &compare::Side {
                    ds: db_ref,
                    rows: rb,
                    label: lb,
                },
                &opt,
            );
            Ok(
                json!({"report": rep, "figures": {"volcano": viz::compare_volcano(&rep), "bins": viz::bin_compare(&rep)}}),
            )
        }
        "unitcorr" => {
            let (a_id, b_id) = (pp.usize("a", 0), pp.usize("b", 1));
            let (da, _) = reg.get(a_id)?;
            let (db, _) = reg.get(b_id)?;
            let ra = reg.rows(a_id, &da, &pp.str("filter_a"))?;
            let rb = reg.rows(b_id, &db, &pp.str("filter_b"))?;
            let opt = correlation::UnitCorrOptions {
                match_by: pp.str_or("match_by", "auto"),
                r_min: pp.f64("r_min", 0.9),
                offset_pct: pp.f64("offset_pct", 5.0),
                min_n: 5,
            };
            let rep = correlation::unit_correlation(&da, &ra, &db, &rb, &opt);
            let first = rep
                .tests
                .first()
                .map(|t| viz::unit_corr_scatter(t, &reg.name(a_id), &reg.name(b_id)));
            Ok(json!({"report": rep, "first": first}))
        }
        "correlation" => {
            let id = pp.usize("ds", 0);
            let (ds, _) = reg.get(id)?;
            let r = reg.rows(id, &ds, &pp.str("filter"))?;
            let tests: Vec<usize> = match p.get("tests").and_then(|v| v.as_array()) {
                Some(a) if !a.is_empty() => a
                    .iter()
                    .filter_map(|x| x.as_str().and_then(|s| ds.find_test(s)))
                    .collect(),
                _ => correlation::pick_tests(&ds, &r, pp.usize("max_tests", 60)),
            };
            let m = correlation::matrix(
                &ds,
                &r,
                &tests,
                &pp.str_or("method", "pearson"),
                pp.f64("threshold", 0.7),
            );
            let pairs: Vec<Value> = m
                .pairs
                .iter()
                .take(500)
                .map(|(a, b, r, n)| json!({"a": a, "b": b, "r": r, "n": n}))
                .collect();
            Ok(json!({"figure": viz::corr_heatmap(&m), "pairs": pairs, "n_tests": tests.len()}))
        }
        "target_corr" => {
            let id = pp.usize("ds", 0);
            let (ds, _) = reg.get(id)?;
            let r = reg.rows(id, &ds, &pp.str("filter"))?;
            let target = pp.str_or("target", "pass");
            let (tv, excl, label): (Vec<f64>, Option<usize>, String) = if target == "pass" {
                (
                    r.iter()
                        .map(|&i| {
                            ds.parts[i]
                                .passed
                                .map(|b| if b { 0.0 } else { 1.0 })
                                .unwrap_or(f64::NAN)
                        })
                        .collect(),
                    None,
                    "fail (1) vs pass (0)".into(),
                )
            } else {
                let t = test_ref(&ds, &target)?;
                (
                    r.iter().map(|&i| ds.value(t, i)).collect(),
                    Some(t),
                    ds.tests[t].key.clone(),
                )
            };
            let res =
                correlation::with_target(&ds, &r, &tv, &pp.str_or("method", "spearman"), excl);
            Ok(
                json!({"figure": viz::target_corr_bar(&res, &format!("Correlation with {label}"), 30), "rows": res}),
            )
        }
        "anova" => {
            let id = pp.usize("ds", 0);
            let (ds, _) = reg.get(id)?;
            let r = reg.rows(id, &ds, &pp.str("filter"))?;
            let factor = pp.str_or("factor", "site");
            let rep = anova::anova(
                &ds,
                &r,
                &factor,
                &ds.numeric_tests(),
                pp.usize("min_group", 5),
                pp.f64("alpha", 0.01),
            );
            let first = rep.rows.first().map(|row| viz::anova_means(row, &factor));
            let table: Vec<Value> = rep.rows.iter().map(|x| json!({"key": x.key, "units": x.units, "k": x.k, "n": x.n, "f": n(x.f), "p": n(x.p), "q": n(x.q), "eta2": n(x.eta2), "kw_p": n(x.kw_p), "levene_p": n(x.levene_p), "range_pct_limit": on(x.range_pct_limit)})).collect();
            Ok(
                json!({"factor": factor, "groups": rep.groups, "n_significant": rep.n_significant, "rows": table, "figure": viz::anova_effects(&rep.rows, &factor, 25), "first": first}),
            )
        }
        "anomaly" => {
            let id = pp.usize("ds", 0);
            let (ds, _) = reg.get(id)?;
            let r = reg.rows(id, &ds, &pp.str("filter"))?;
            let methods = pp.str_or("methods", "robust,pat,nnr,mahalanobis,iforest");
            let opt = anomaly::AnomalyOptions {
                robust_k: pp.f64("robust_k", 6.0),
                pat_k: pp.f64("pat_k", 6.0),
                nnr: methods.contains("nnr"),
                nnr_k: pp.f64("nnr_k", 5.0),
                mahalanobis: methods.contains("mahal"),
                iforest: methods.contains("iforest"),
                iforest_threshold: pp.f64("iforest_threshold", 0.62),
                only_passing: pp.bool("only_passing", false),
                ..Default::default()
            };
            let mut rep = anomaly::detect(&ds, &r, &opt);
            if !methods.contains("robust") {
                rep.parts
                    .retain(|p| p.methods.iter().any(|m| m != "robust-z"));
            }
            if !methods.contains("pat") {
                rep.parts.retain(|p| p.methods.iter().any(|m| m != "PAT"));
            }
            rep.n_flagged = rep.parts.len();
            let mut figs = json!({"scores": viz::anomaly_scatter(&ds, &rep)});
            if !ds.wafers.is_empty() {
                let w = pp.opt_usize("wafer").map(|w| w as u32).unwrap_or_else(|| {
                    rep.parts
                        .iter()
                        .map(|p| ds.parts[p.row].wafer)
                        .next()
                        .unwrap_or(0)
                });
                figs["wafer"] = viz::anomaly_wafer(&ds, &rep, w);
            }
            let pat: Vec<&anomaly::TestPat> = rep.pat.iter().filter(|t| t.n_outside > 0).collect();
            Ok(
                json!({"n_parts": rep.n_parts, "n_tests": rep.n_tests, "n_flagged": rep.n_flagged, "by_method": rep.by_method, "parts": rep.parts.iter().take(2000).collect::<Vec<_>>(), "pat": pat, "figures": figs}),
            )
        }
        "pca" => {
            let id = pp.usize("ds", 0);
            let (ds, _) = reg.get(id)?;
            let r = reg.rows(id, &ds, &pp.str("filter"))?;
            let fo = ml::features::FeatureOptions {
                max_features: pp.usize("max_features", 150),
                ..Default::default()
            };
            let clusters = match pp.str_or("clusters", "auto").as_str() {
                "auto" => None,
                s => Some(s.parse().unwrap_or(0)),
            };
            let rep = ml::pca_report(&ds, &r, &fo, pp.usize("components", 10), clusters)?;
            let color_by = pp.str_or("color_by", "pass");
            let loadings: Vec<Value> = (0..rep.pca.n_components.min(3))
                .map(|k| json!({"pc": k + 1, "top": rep.pca.top_loadings(k, 10)}))
                .collect();
            let clusters_info = rep.kmeans.as_ref().map(|km| json!({"k": km.k, "sizes": km.sizes, "silhouette": km.silhouette, "scan": km.k_scan}));
            Ok(
                json!({"n_features": rep.features.p, "explained": rep.pca.explained_ratio, "loadings": loadings, "clusters": clusters_info,
                "figures": {"scatter": viz::pca_scatter(&ds, &rep, &color_by), "scree": viz::pca_scree(&rep)}}),
            )
        }
        "explain" => {
            let id = pp.usize("ds", 0);
            let (ds, _) = reg.get(id)?;
            let r = reg.rows(id, &ds, &pp.str("filter"))?;
            let target = ml::TargetSpec::parse(&pp.str_or("target", "pass"), &ds)?;
            let fo = ml::features::FeatureOptions {
                max_features: pp.usize("max_features", 300),
                ..Default::default()
            };
            let fopt = ml::forest::ForestOptions {
                n_trees: pp.usize("trees", 100),
                max_depth: pp.usize("depth", 12),
                ..Default::default()
            };
            let rep = ml::explain(&ds, &r, &target, &fo, &fopt)?;
            let mut figs = json!({"importance": viz::importance_bar(&rep, 25)});
            if rep.kind == "classification" {
                figs["eval"] = viz::confusion(&rep);
            } else {
                figs["eval"] = viz::pred_vs_actual(&rep);
            }
            Ok(json!({"report": rep, "figures": figs}))
        }
        "export_stdf" => {
            let id = pp.usize("ds", 0);
            let (ds, paths) = reg.get(id)?;
            if paths.len() != 1 || ds.sources.len() != 1 {
                bail!("STDF export works on datasets loaded from a single file");
            }
            let out = pp.str("path");
            if out.is_empty() {
                bail!("output path required");
            }
            let r = reg.rows(id, &ds, &pp.str("filter"))?;
            let set: std::collections::HashSet<usize> = r.into_iter().collect();
            let assign: Vec<Option<String>> = (0..ds.n_parts())
                .map(|i| set.contains(&i).then(|| "selected".to_string()))
                .collect();
            let out_path = std::path::PathBuf::from(&out);
            let dir = out_path
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| ".".into());
            let stem = out_path
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or("export".into());
            let res = crate::split::split_with_assignment(
                std::path::Path::new(&paths[0]),
                &ds,
                &assign,
                &dir,
                Some(&stem),
            )?;
            let o = res.first().ok_or_else(|| anyhow!("selection is empty"))?;
            std::fs::rename(&o.path, &out_path).with_context(|| format!("rename to {out}"))?;
            Ok(json!({"path": out, "parts": o.parts}))
        }
        _ => bail!("unknown op '{op}'"),
    }
}

fn chart(reg: &Registry, pp: &P) -> Result<Value> {
    let kind = pp.str_or("kind", "hist");
    let id = pp.usize("ds", 0);
    let (ds, _) = reg.get(id)?;
    let r = reg.rows(id, &ds, &pp.str("filter"))?;
    let group_by = pp.str("group_by");
    let range = viz::RangeMode::parse(&pp.str("range"));
    Ok(match kind.as_str() {
        "hist" | "box" | "cdf" | "trend" | "stats" => {
            let t = test_ref(&ds, &pp.str("test"))?;
            let g = set_groups(reg, id, &r, &group_by)
                .unwrap_or_else(|| groups_for_test(&ds, &r, &group_by, Some(t)));
            let info = &ds.tests[t];
            let st: Vec<Value> = g.iter().map(|(name, rows)| {
                let d = stats::describe(&ds.values_of(t, Some(rows)), info.lo, info.hi);
                json!({"group": name, "n": d.n, "mean": n(d.mean), "std": n(d.std), "min": n(d.min), "q1": n(d.q1), "median": n(d.median), "q3": n(d.q3), "max": n(d.max), "cpk": on(d.cpk), "fails": d.fails})
            }).collect();
            let vals = ds.values_of(t, Some(&r));
            let mut bx = viz::boxplot(&ds, t, &g);
            let mut cdf = viz::cdf(&ds, t, &g);
            let mut trend = viz::trend(&ds, t, &g);
            viz::set_range(&mut bx, "yaxis", info, &vals, range);
            viz::set_range(&mut cdf, "xaxis", info, &vals, range);
            viz::set_range(&mut trend, "yaxis", info, &vals, range);
            json!({
                "hist": viz::histogram_fine(info, &viz::values_groups(&ds, t, &g), pp.opt_usize("bins"), pp.bool("density", g.len() > 1), range),
                "box": bx,
                "cdf": cdf,
                "trend": trend,
                "stats": st,
                "test": info,
            })
        }
        "scatter" => {
            let t = test_ref(&ds, &pp.str("test"))?;
            let t2 = test_ref(&ds, &pp.str("test2"))?;
            let g = set_groups(reg, id, &r, &group_by)
                .unwrap_or_else(|| groups_for(&ds, &r, &group_by));
            let mut f = viz::scatter(&ds, t, t2, &g);
            viz::set_range(
                &mut f,
                "xaxis",
                &ds.tests[t],
                &ds.values_of(t, Some(&r)),
                range,
            );
            viz::set_range(
                &mut f,
                "yaxis",
                &ds.tests[t2],
                &ds.values_of(t2, Some(&r)),
                range,
            );
            json!({ "scatter": f })
        }
        "wafer" if pp.str("coords") == "tests" => {
            // reconstruct maps from datalogged wafer / X / Y test results
            let mode = pp.str_or("mode", "hbin");
            let t = if mode == "test" {
                Some(test_ref(&ds, &pp.str("test"))?)
            } else {
                None
            };
            let txy = TestXy::parse(&ds, pp)?;
            let (tx, ty, tw) = (txy.tx, txy.ty, txy.tw);
            let coord = |row: usize| txy.xy(&ds, row);
            let mut by_wafer: IndexMap<String, Vec<usize>> = IndexMap::new();
            for &row in &r {
                if coord(row).is_none() {
                    continue;
                }
                let Some(w) = txy.wafer(&ds, row) else {
                    continue;
                };
                by_wafer.entry(w).or_default().push(row);
            }
            by_wafer.sort_by(|a, _, b, _| match (a.parse::<f64>(), b.parse::<f64>()) {
                (Ok(x), Ok(y)) => x.partial_cmp(&y).unwrap_or(std::cmp::Ordering::Equal),
                _ => a.cmp(b),
            });
            let wafers: Vec<String> = by_wafer.keys().cloned().collect();
            let w = pp.str("wafer");
            let pick: Vec<(&String, &Vec<usize>)> = if w.is_empty() || w == "all" {
                by_wafer.iter().take(24).collect()
            } else {
                vec![by_wafer
                    .get_key_value(&w)
                    .ok_or_else(|| anyhow!("wafer {w} not found"))?]
            };
            let single = pick.len() == 1;
            let label = |name: &str| match tw {
                Some(tw) => format!("wafer {name} (test {})", ds.tests[tw].num),
                None => format!("X/Y from tests {} / {}", ds.tests[tx].num, ds.tests[ty].num),
            };
            let maps: Vec<Value> = pick
                .into_iter()
                .map(|(name, rw)| {
                    let y = ds.yield_pct(rw);
                    let title = if single { label(name) } else { format!("{} – {:.1}%", label(name), y) };
                    json!({"wafer": name, "yield": y, "n": rw.len(),
                        "figure": viz::wafer_map_xy(&ds, rw, &mode, t, &title, single || mode == "test", &coord)})
                })
                .collect();
            if maps.is_empty() {
                bail!("no parts have results for the X/Y tests");
            }
            json!({"maps": maps, "wafers": wafers})
        }
        "wafer" => {
            let mode = pp.str_or("mode", "hbin");
            let t = if mode == "test" {
                Some(test_ref(&ds, &pp.str("test"))?)
            } else {
                None
            };
            let w = pp.str("wafer");
            if w.is_empty() || w == "all" {
                // gallery: one map per wafer (max 24)
                let mut maps = vec![];
                for (wi, name) in ds.wafers.iter().enumerate().take(24) {
                    let rw: Vec<usize> = r
                        .iter()
                        .copied()
                        .filter(|&i| ds.parts[i].wafer == wi as u32)
                        .collect();
                    if rw.is_empty() {
                        continue;
                    }
                    let y = ds.yield_pct(&rw);
                    maps.push(json!({"wafer": name, "yield": y, "n": rw.len(), "figure": viz::wafer_map_opts(&ds, &rw, &mode, t, &format!("{name} – {:.1}%", y), mode == "test")}));
                }
                json!({"maps": maps, "wafers": ds.wafers})
            } else {
                let wi = ds
                    .wafers
                    .iter()
                    .position(|x| x == &w)
                    .ok_or_else(|| anyhow!("wafer {w} not found"))?;
                let rw: Vec<usize> = r
                    .iter()
                    .copied()
                    .filter(|&i| ds.parts[i].wafer == wi as u32)
                    .collect();
                json!({"maps": [{"wafer": w, "yield": ds.yield_pct(&rw), "n": rw.len(), "figure": viz::wafer_map(&ds, &rw, &mode, t, &w)}], "wafers": ds.wafers})
            }
        }
        "anova_means" => {
            let t = test_ref(&ds, &pp.str("test"))?;
            let factor = pp.str_or("factor", "site");
            let rep = anova::anova(&ds, &r, &factor, &[t], pp.usize("min_group", 5), 0.01);
            let row = rep
                .rows
                .first()
                .ok_or_else(|| anyhow!("not enough data for ANOVA"))?;
            let g = groups_for(&ds, &r, &factor);
            json!({"means": viz::anova_means(row, &factor), "box": viz::boxplot(&ds, t, &g)})
        }
        "compare_test" => {
            let b_id = pp.usize("b", id);
            let (db, _) = reg.get(b_id)?;
            let key = pp.str("test");
            let ta = test_ref(&ds, &key)?;
            let mode = pp.str_or("mode", "datasets");
            let (va, vb, la, lb) = if mode == "group" {
                let sel = pp.str("select");
                let vs = pp.str("vs");
                let (ra, rb) = group_rows(reg, id, &ds, &sel, &vs, &pp.str("filter"))?;
                (
                    ds.values_of(ta, Some(&ra)),
                    ds.values_of(ta, Some(&rb)),
                    if vs.is_empty() {
                        "rest".to_string()
                    } else {
                        vs
                    },
                    sel,
                )
            } else {
                let ra = reg.rows(id, &ds, &pp.str("filter_a"))?;
                let rb = reg.rows(b_id, &db, &pp.str("filter_b"))?;
                let tb = db
                    .test_index
                    .get(&ds.tests[ta].key)
                    .copied()
                    .or_else(|| db.find_test(&key))
                    .ok_or_else(|| anyhow!("test not in B"))?;
                (
                    ds.values_of(ta, Some(&ra)),
                    db.values_of(tb, Some(&rb)),
                    format!("A: {}", reg.name(id)),
                    format!("B: {}", reg.name(b_id)),
                )
            };
            let all: Vec<f64> = va.iter().chain(&vb).copied().collect();
            let g = vec![(la, va), (lb, vb)];
            let info = &ds.tests[ta];
            let mut cdf = viz::cdf_v(info, &g);
            let mut bx = viz::boxplot_v(info, &g);
            viz::set_range(&mut cdf, "xaxis", info, &all, range);
            viz::set_range(&mut bx, "yaxis", info, &all, range);
            json!({"hist": viz::histogram_fine(info, &g, None, true, range), "cdf": cdf, "box": bx})
        }
        "unitcorr" => {
            let b_id = pp.usize("b", 1);
            let (db, _) = reg.get(b_id)?;
            let ra = reg.rows(id, &ds, &pp.str("filter_a"))?;
            let rb = reg.rows(b_id, &db, &pp.str("filter_b"))?;
            let opt = correlation::UnitCorrOptions {
                match_by: pp.str_or("match_by", "auto"),
                r_min: pp.f64("r_min", 0.9),
                offset_pct: pp.f64("offset_pct", 5.0),
                min_n: 5,
            };
            let key = pp.str("test");
            let t = test_ref(&ds, &key)?;
            let rep = correlation::unit_correlation(&ds, &ra, &db, &rb, &opt);
            let tc = rep
                .tests
                .iter()
                .find(|x| x.key == ds.tests[t].key)
                .ok_or_else(|| anyhow!("test not matched"))?;
            json!({"scatter": viz::unit_corr_scatter(tc, &reg.name(id), &reg.name(b_id))})
        }
        _ => bail!("unknown chart kind '{kind}'"),
    })
}

fn u4_at(d: &[u8], o: usize, e: crate::stdf::Endian) -> Option<u32> {
    let b: [u8; 4] = d.get(o..o + 4)?.try_into().ok()?;
    Some(match e {
        crate::stdf::Endian::Little => u32::from_le_bytes(b),
        crate::stdf::Endian::Big => u32::from_be_bytes(b),
    })
}

/// Source file of a dataset and the dataset row of that file's first PRR,
/// when rows map 1:1 to the file's PRRs (no de-duplication).
fn file_of(ds: &Dataset, paths: &[String], file: usize) -> Result<(String, Option<usize>)> {
    if paths.is_empty() {
        bail!("this dataset was created in the app, so it has no STDF file to read raw records from; use the dataset it came from");
    }
    let path = paths
        .get(file)
        .cloned()
        .ok_or_else(|| anyhow!("file {file} is not part of this dataset"))?;
    let base = ds.parts.iter().filter(|p| (p.file as usize) < file).count();
    let n = ds.parts.iter().filter(|p| p.file as usize == file).count() as u64;
    let prrs = ds
        .sources
        .get(file)
        .and_then(|s| s.record_counts.get("PRR").copied())
        .unwrap_or(0);
    Ok((path, (n == prrs).then_some(base)))
}

/// Page through the raw records of one type, optionally only one test number.
fn records_op(reg: &Registry, pp: &P) -> Result<Value> {
    use crate::stdf::{record_code, record_name, records::code, Record, StdfReader};
    let (ds, paths) = reg.get(pp.usize("ds", 0))?;
    let file = pp.usize("file", 0);
    let (path, base) = file_of(&ds, &paths, file)?;
    let rec = pp.str_or("rec", "PRR").to_uppercase();
    let want = record_code(&rec).ok_or_else(|| anyhow!("unknown record type '{rec}'"))?;
    let test = pp.opt_usize("test").map(|t| t as u32);
    let test_off = match want {
        code::PTR | code::MPR | code::FTR => Some(0),
        code::TSR => Some(3),
        _ => None,
    };
    let test = test.filter(|_| test_off.is_some());
    let offset = pp.usize("offset", 0);
    let limit = pp.usize("limit", 200).clamp(1, 5000);
    let counts = ds
        .sources
        .get(file)
        .map(|s| s.record_counts.clone())
        .unwrap_or_default();
    let known_total = test.is_none().then(|| {
        counts
            .get(record_name(want.0, want.1))
            .copied()
            .unwrap_or(0)
    });
    let mut rdr = StdfReader::open(&path)?;
    let endian = rdr.endian;
    let (mut k, mut prr_k) = (0usize, 0usize);
    let mut out = vec![];
    while let Some((h, data)) = rdr.next_raw()? {
        let c = (h.typ, h.sub);
        let part = (c == code::PRR).then_some(prr_k);
        if c == code::PRR {
            prr_k += 1;
        }
        if c != want {
            continue;
        }
        if let (Some(tn), Some(o)) = (test, test_off) {
            if u4_at(data, o, endian) != Some(tn) {
                continue;
            }
        }
        if k >= offset && out.len() < limit {
            let mut v = record_json(&Record::parse(h.typ, h.sub, data, endian));
            v["_n"] = json!(k);
            v["_offset"] = json!(h.offset);
            if let (Some(b), Some(pk)) = (base, part) {
                v["_part"] = json!(b + pk);
            }
            out.push(v);
        }
        k += 1;
        if out.len() >= limit && known_total.is_some() {
            break;
        }
    }
    Ok(json!({
        "rec": rec, "file": file, "path": path, "offset": offset, "total": known_total.unwrap_or(k as u64),
        "rows": out, "counts": counts, "files": paths,
    }))
}

/// All raw records of one part: its PIR … PRR block (tests of that head/site).
fn part_records_op(reg: &Registry, pp: &P) -> Result<Value> {
    use crate::stdf::{records::code, Record, StdfReader};
    let (ds, paths) = reg.get(pp.usize("ds", 0))?;
    let i = pp.usize("index", usize::MAX);
    let p = ds.parts.get(i).ok_or_else(|| anyhow!("no such part"))?;
    let file = p.file as usize;
    let (path, base) = file_of(&ds, &paths, file)?;
    let base = base.ok_or_else(|| {
        anyhow!("raw records need the dataset rows to match the file's PRRs (open it without retest de-duplication)")
    })?;
    let target = i - base;
    let mut rdr = StdfReader::open(&path)?;
    let endian = rdr.endian;
    let mut blocks: std::collections::HashMap<(u8, u8), Vec<Value>> = Default::default();
    let mut prr_k = 0usize;
    while let Some((h, data)) = rdr.next_raw()? {
        let c = (h.typ, h.sub);
        let hs = |o: usize| {
            (
                data.get(o).copied().unwrap_or(1),
                data.get(o + 1).copied().unwrap_or(1),
            )
        };
        let key = match c {
            code::PIR | code::PRR => hs(0),
            code::PTR | code::MPR | code::FTR => hs(4),
            _ => continue,
        };
        if c == code::PIR {
            blocks.insert(key, vec![]);
        }
        let key = if blocks.contains_key(&key) || blocks.len() != 1 {
            key
        } else {
            *blocks.keys().next().unwrap()
        };
        if prr_k <= target {
            let mut v = record_json(&Record::parse(h.typ, h.sub, data, endian));
            v["_rec"] = json!(crate::stdf::record_name(h.typ, h.sub));
            v["_offset"] = json!(h.offset);
            blocks.entry(key).or_default().push(v);
        }
        if c == code::PRR {
            let block = blocks.remove(&key).unwrap_or_default();
            if prr_k == target {
                return Ok(json!({"part": part_json(&ds, i), "path": path, "records": block}));
            }
            prr_k += 1;
        }
    }
    bail!("part not found in {path}")
}

fn set_name(s: &str) -> String {
    let n: String = s
        .trim()
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
        .collect();
    if n.is_empty() {
        "match".into()
    } else {
        n
    }
}

/// Match the parts of two datasets by a key on each side; stores `$name` part
/// sets in both datasets and a link for tracing and paired charts.
fn match_op(reg: &Registry, pp: &P) -> Result<Value> {
    use data::matching::{match_parts, KeySpec};
    let (a, b) = (pp.usize("a", 0), pp.usize("b", 1));
    if a == b {
        bail!("pick two different datasets to match");
    }
    let (da, _) = reg.get(a)?;
    let (db, _) = reg.get(b)?;
    let ks_a = pp.str_or("key_a", "part_txt");
    let ks_b = pp.str_or("key_b", &ks_a);
    let ka = KeySpec::parse(&ks_a, &da)?;
    let kb = KeySpec::parse(&ks_b, &db)?;
    let ra = reg.rows(a, &da, &pp.str("filter_a"))?;
    let rb = reg.rows(b, &db, &pp.str("filter_b"))?;
    let m = match_parts(&da, &ra, &ka, &db, &rb, &kb);
    let name = set_name(&pp.str("name"));
    reg.put_set(a, &name, m.in_a.clone());
    reg.put_set(b, &name, m.in_b.clone());
    let link = {
        let mut v = reg.matches.write().unwrap();
        v.retain(|l| !(l.a == a && l.b == b && l.name == name));
        v.push(MatchLink {
            a,
            b,
            name: name.clone(),
            key_a: ks_a,
            key_b: ks_b,
            pairs: m.pairs.clone(),
        });
        v.len() - 1
    };
    let same_bin = m
        .pairs
        .iter()
        .filter(|(x, y, _)| da.parts[*x].hbin == db.parts[*y].hbin)
        .count();
    let rows: Vec<Value> = m
        .pairs
        .iter()
        .take(pp.usize("limit", 20000))
        .map(|(x, y, k)| {
            let (pa, pb) = (&da.parts[*x], &db.parts[*y]);
            json!({"key": k, "a_row": x, "b_row": y, "a_part_id": pa.part_id, "b_part_id": pb.part_id,
                "a_site": pa.site, "b_site": pb.site, "a_hbin": pa.hbin, "b_hbin": pb.hbin, "a_pass": pa.passed, "b_pass": pb.passed,
                "a_wafer": da.wafer_name(pa), "a_x": pa.x, "a_y": pa.y})
        })
        .collect();
    Ok(json!({
        "link": link, "name": name, "a": a, "b": b, "name_a": reg.name(a), "name_b": reg.name(b),
        "key_a": m.key_a, "key_b": m.key_b, "pairs": m.pairs.len(),
        "matched_a": m.matched_a, "matched_b": m.matched_b, "only_a": m.only_a, "only_b": m.only_b,
        "no_key_a": m.no_key_a, "no_key_b": m.no_key_b, "dup_a": m.dup_a, "dup_b": m.dup_b,
        "n_a": ra.len(), "n_b": rb.len(),
        "bin_agreement_pct": if m.pairs.is_empty() { Value::Null } else { json!(same_bin as f64 * 100.0 / m.pairs.len() as f64) },
        "rows": rows,
    }))
}

/// Bad candidates and their counterparts, as part sets in file 1 (`a`):
///
/// * `$name` (bad_candidate): good in file 1 (`filter_a`), bad in file 2
///   (`filter_b`) and, with a `range`, the file-2 value of `test` inside it;
/// * `$marginal`: the file-2 fails outside the range; they count as
///   candidates only with `include_marginal`, and are listed either way;
/// * `$confirmed_good`: good in file 1 and passing in file 2 (`pass_b`).
///
/// Also keeps the file 1 ↔ file 2 link of the candidates (for tracing and
/// paired charts), like `match`.
fn candidate_sets_op(reg: &Registry, pp: &P) -> Result<Value> {
    use crate::analysis::candidates::{display_unit, Range};
    use data::matching::{match_parts, KeySpec};
    use std::collections::HashMap;
    let (a, b) = (pp.usize("a", 0), pp.usize("b", 1));
    if a == b {
        bail!("pick two different files");
    }
    let (da, _) = reg.get(a)?;
    let (db, _) = reg.get(b)?;
    let ks_a = pp.str_or("key_a", "part_txt");
    let ks_b = pp.str_or("key_b", &ks_a);
    let ka = KeySpec::parse(&ks_a, &da)?;
    let kb = KeySpec::parse(&ks_b, &db)?;
    let ra = reg.rows(a, &da, &pp.str("filter_a"))?;
    let rb = reg.rows(b, &db, &pp.str_or("filter_b", "not pass"))?;
    let name = set_name(&pp.str_or("name", "bad_candidate"));
    let marg_name = set_name(&pp.str_or("marginal", "marginal"));
    let conf_name = set_name(&pp.str_or("confirmed", "confirmed_good"));
    let include = pp.bool("include_marginal", false);
    // the range applies to the failing test's value in file 2
    let range_s = pp.str("range");
    let range = if range_s.trim().is_empty() {
        None
    } else {
        let q = pp.str("test");
        if q.trim().is_empty() {
            bail!("a value range needs the failing test (e.g. 1013)");
        }
        let tb = test_ref(&db, q.split(':').next().unwrap_or(&q).trim())?;
        let ti = &db.tests[tb];
        Some((tb, Range::parse(&range_s, &ti.units, ti.res_scal)?))
    };

    let m = match_parts(&da, &ra, &ka, &db, &rb, &kb);
    // classify each matched unit by its file-2 value
    let mut class: HashMap<&str, bool> = HashMap::new(); // key → in range
    let (mut cand_pairs, mut marg_pairs) = (vec![], vec![]);
    for p in &m.pairs {
        let inside = range
            .as_ref()
            .map(|(t, r)| r.contains(db.value(*t, p.1)))
            .unwrap_or(true);
        class.insert(p.2.as_str(), inside);
        if inside {
            cand_pairs.push(p.clone());
        } else {
            marg_pairs.push(p.clone());
        }
    }
    let mut cand = vec![false; da.n_parts()];
    let mut marg = vec![false; da.n_parts()];
    for &r in &ra {
        if !m.in_a[r] {
            continue;
        }
        match ka.key(&da, r).and_then(|k| class.get(k.as_str()).copied()) {
            Some(true) => cand[r] = true,
            Some(false) => {
                marg[r] = true;
                cand[r] |= include;
            }
            None => {}
        }
    }
    let (mut cand_b, mut marg_b) = (vec![false; db.n_parts()], vec![false; db.n_parts()]);
    for &r in &rb {
        if !m.in_b[r] {
            continue;
        }
        match kb.key(&db, r).and_then(|k| class.get(k.as_str()).copied()) {
            Some(true) => cand_b[r] = true,
            Some(false) => {
                marg_b[r] = true;
                cand_b[r] |= include;
            }
            None => {}
        }
    }
    // good in file 1, passing in file 2, and not among the bad
    let rp = reg.rows(b, &db, &pp.str_or("pass_b", "pass"))?;
    let mc = match_parts(&da, &ra, &ka, &db, &rp, &kb);
    let conf: Vec<bool> = (0..da.n_parts())
        .map(|r| mc.in_a[r] && !cand[r] && !marg[r])
        .collect();
    let conf_b: Vec<bool> = (0..db.n_parts())
        .map(|r| mc.in_b[r] && !cand_b[r] && !marg_b[r])
        .collect();
    let count = |v: &[bool]| v.iter().filter(|&&x| x).count();
    let (n_cand, n_marg, n_conf) = (count(&cand), count(&marg), count(&conf));
    reg.put_set(a, &name, cand);
    reg.put_set(b, &name, cand_b);
    reg.put_set(a, &marg_name, marg);
    reg.put_set(b, &marg_name, marg_b);
    reg.put_set(a, &conf_name, conf);
    reg.put_set(b, &conf_name, conf_b);
    let mut link_pairs = cand_pairs.clone();
    if include {
        link_pairs.extend(marg_pairs.iter().cloned());
        link_pairs.sort_unstable_by_key(|p| p.0);
    }
    let link = {
        let mut v = reg.matches.write().unwrap();
        v.retain(|l| !(l.a == a && l.b == b && l.name == name));
        v.push(MatchLink {
            a,
            b,
            name: name.clone(),
            key_a: ks_a,
            key_b: ks_b,
            pairs: link_pairs,
        });
        v.len() - 1
    };
    // values as the tester shows them (RES_SCAL applied)
    let (disp_units, disp) = range
        .as_ref()
        .map(|(t, _)| display_unit(&db.tests[*t].units, db.tests[*t].res_scal))
        .unwrap_or_default();
    let row = |(x, y, k): &(usize, usize, String)| {
        let (pa, pb) = (&da.parts[*x], &db.parts[*y]);
        json!({"key": k, "a_row": x, "b_row": y, "a_part_id": pa.part_id, "b_part_id": pb.part_id,
            "b_hbin": pb.hbin, "b_value": range.as_ref().map(|(t, _)| n(db.value(*t, *y) * disp)),
            "a_wafer": da.wafer_name(pa), "a_x": pa.x, "a_y": pa.y, "a_site": pa.site})
    };
    let limit = pp.usize("limit", 5000);
    Ok(json!({
        "link": link, "name": name, "marginal": marg_name, "confirmed": conf_name, "a": a, "b": b,
        "key_a": m.key_a, "key_b": m.key_b, "n_a": ra.len(), "n_b": rb.len(), "only_b": m.only_b,
        "matched": m.pairs.len(), "pairs": n_cand, "n_cand": n_cand, "n_marginal": n_marg, "n_confirmed": n_conf,
        "n_pass_b": rp.len(), "include_marginal": include,
        "range": range.as_ref().map(|(t, r)| json!({"test": db.tests[*t].key, "units": disp_units, "stored_units": db.tests[*t].units,
            "res_scal": db.tests[*t].res_scal, "lo": r.lo.map(|v| n(v * disp)), "hi": r.hi.map(|v| n(v * disp))})),
        "rows": cand_pairs.iter().take(limit).map(row).collect::<Vec<_>>(),
        "marginal_rows": marg_pairs.iter().take(limit).map(row).collect::<Vec<_>>(),
    }))
}

/// Paired charts: test X of file A and test Y of file B over the same
/// (matched) parts, colored by a field of either side.
fn match_dual_op(reg: &Registry, pp: &P) -> Result<Value> {
    let (a, b, pairs, _, _) = link_pairs(reg, pp)?;
    let (da, _) = reg.get(a)?;
    let (db, _) = reg.get(b)?;
    let qa = pp.str("test_a");
    let ta = test_ref(&da, &qa)?;
    let qb = pp.str_or("test_b", &qa);
    let tb = test_ref(&db, &qb)?;
    let mode = viz::RangeMode::parse(&pp.str("range"));
    let color_by = pp.str("color_by");
    let color_b = pp.str("color_side") == "b";
    let (na, nb) = (reg.name(a), reg.name(b));
    // groups of pair indices, keyed on the chosen side's field
    let (cds, ct) = if color_b { (&db, tb) } else { (&da, ta) };
    let side_rows: Vec<usize> = pairs
        .iter()
        .map(|p| if color_b { p.1 } else { p.0 })
        .collect();
    let pos: std::collections::HashMap<usize, usize> =
        side_rows.iter().enumerate().map(|(i, &r)| (r, i)).collect();
    let groups: Vec<(String, Vec<usize>)> = groups_for_test(cds, &side_rows, &color_by, Some(ct))
        .into_iter()
        .map(|(g, rs)| (g, rs.iter().map(|r| pos[r]).collect()))
        .collect();
    let (ia, ib) = (&da.tests[ta], &db.tests[tb]);
    let vals = |ds: &Dataset, t: usize, a_side: bool| -> Vec<(String, Vec<f64>)> {
        groups
            .iter()
            .map(|(g, idx)| {
                let v = idx
                    .iter()
                    .map(|&k| ds.value(t, if a_side { pairs[k].0 } else { pairs[k].1 }))
                    .filter(|v| v.is_finite())
                    .collect();
                (g.clone(), v)
            })
            .collect()
    };
    let mut hist_a = viz::histogram_fine(ia, &vals(&da, ta, true), None, groups.len() > 1, mode);
    let mut hist_b = viz::histogram_fine(ib, &vals(&db, tb, false), None, groups.len() > 1, mode);
    hist_a["layout"]["title"]["text"] = json!(format!("A: {na} – {}", ia.label()));
    hist_b["layout"]["title"]["text"] = json!(format!("B: {nb} – {}", ib.label()));
    let pts: Vec<(String, Vec<(f64, f64, String)>)> = groups
        .iter()
        .map(|(g, idx)| {
            let v = idx
                .iter()
                .filter_map(|&k| {
                    let (x, y, key) = &pairs[k];
                    let (vx, vy) = (da.value(ta, *x), db.value(tb, *y));
                    (vx.is_finite() && vy.is_finite()).then(|| (vx, vy, key.clone()))
                })
                .collect();
            (g.clone(), v)
        })
        .collect();
    let mut scatter = viz::pair_scatter(ia, ib, &format!("A: {na}"), &format!("B: {nb}"), &pts);
    let all_x: Vec<f64> = pts.iter().flat_map(|g| g.1.iter().map(|p| p.0)).collect();
    let all_y: Vec<f64> = pts.iter().flat_map(|g| g.1.iter().map(|p| p.1)).collect();
    viz::set_range(&mut scatter, "xaxis", ia, &all_x, mode);
    viz::set_range(&mut scatter, "yaxis", ib, &all_y, mode);
    let diffs: Vec<(String, Vec<f64>)> = pts
        .iter()
        .map(|(g, v)| (g.clone(), v.iter().map(|p| p.1 - p.0).collect()))
        .collect();
    let dinfo = data::TestInfo {
        key: "delta".into(),
        num: ib.num,
        name: format!("B {} − A {}", ib.num, ia.num),
        kind: ib.kind,
        units: ib.units.clone(),
        lo: None,
        hi: None,
        lo_spec: None,
        hi_spec: None,
        res_scal: 0,
        exec_count: 0,
        fail_count: 0,
    };
    let mut delta =
        viz::histogram_fine(&dinfo, &diffs, None, groups.len() > 1, viz::RangeMode::Data);
    delta["layout"]["title"]["text"] = json!(format!(
        "Difference per unit: B {} − A {}{}",
        ib.num,
        ia.num,
        if ia.units != ib.units {
            "  (units differ)"
        } else {
            ""
        }
    ));
    let r = stats::pearson(&all_x, &all_y);
    Ok(json!({
        "n": all_x.len(), "pairs": pairs.len(), "r": n(r), "test_a": ia, "test_b": ib,
        "figures": {"hist_a": hist_a, "hist_b": hist_b, "scatter": scatter, "delta": delta},
    }))
}

/// Find one physical unit by its key (or a row) and list its results in both
/// files side by side.
fn trace_op(reg: &Registry, pp: &P) -> Result<Value> {
    use data::matching::{find, KeySpec};
    let (a, b, ks_a, ks_b) = match pp.opt_usize("link") {
        Some(li) => {
            let m = reg.matches.read().unwrap();
            let l = m
                .get(li)
                .ok_or_else(|| anyhow!("match the two files first"))?;
            (l.a, Some(l.b), l.key_a.clone(), l.key_b.clone())
        }
        None => {
            let k = pp.str_or("key_a", "part_id");
            (
                pp.usize("a", 0),
                pp.opt_usize("b"),
                k.clone(),
                pp.str_or("key_b", &k),
            )
        }
    };
    let (da, _) = reg.get(a)?;
    let ka = KeySpec::parse(&ks_a, &da)?;
    let value = pp.str("value");
    let hits_a: Vec<usize> = match pp.opt_usize("row_a") {
        Some(r) if r < da.n_parts() => vec![r],
        Some(_) => bail!("no such part in A"),
        None if value.trim().is_empty() => bail!("enter the value to look for"),
        None => find(&da, &ka, &value),
    };
    // the key to look up in B: the typed value, or the key of the A part
    let key = if value.trim().is_empty() {
        hits_a
            .last()
            .and_then(|&r| ka.key(&da, r))
            .unwrap_or_default()
    } else {
        value.clone()
    };
    let side_b = match b {
        Some(b) => {
            let (db, _) = reg.get(b)?;
            let kb = KeySpec::parse(&ks_b, &db)?;
            let hits = match pp.opt_usize("row_b") {
                Some(r) if r < db.n_parts() => vec![r],
                _ if key.is_empty() => vec![],
                _ => find(&db, &kb, &key),
            };
            Some((db, hits))
        }
        None => None,
    };
    let ra = hits_a.last().copied();
    let rb = side_b.as_ref().and_then(|(_, h)| h.last().copied());
    let mut tests = vec![];
    let db = side_b.as_ref().map(|s| s.0.clone());
    let val = |ds: &Dataset, t: usize, r: Option<usize>| {
        r.map(|r| ds.value(t, r)).filter(|v| v.is_finite())
    };
    if ra.is_some() || rb.is_some() {
        let mut seen = std::collections::HashSet::new();
        for (t, info) in da.tests.iter().enumerate() {
            let va = val(&da, t, ra);
            let tb = db
                .as_ref()
                .and_then(|d| d.test_index.get(&info.key).copied());
            let vb = match (&db, tb) {
                (Some(d), Some(tb)) => val(d, tb, rb),
                _ => None,
            };
            if va.is_none() && vb.is_none() {
                continue;
            }
            seen.insert(info.key.clone());
            tests.push(json!({"key": info.key, "num": info.num, "units": info.units, "lo": on(info.lo), "hi": on(info.hi),
                "a": on(va), "b": on(vb), "diff": match (va, vb) { (Some(x), Some(y)) => n(y - x), _ => Value::Null },
                "a_pass": va.map(|v| info.within_limits(v)), "b_pass": vb.map(|v| info.within_limits(v))}));
        }
        if let Some(d) = &db {
            for (t, info) in d.tests.iter().enumerate() {
                if seen.contains(&info.key) {
                    continue;
                }
                if let Some(vb) = val(d, t, rb) {
                    tests.push(json!({"key": info.key, "num": info.num, "units": info.units, "lo": on(info.lo), "hi": on(info.hi),
                        "a": Value::Null, "b": vb, "diff": Value::Null, "a_pass": Value::Null, "b_pass": info.within_limits(vb)}));
                }
            }
        }
    }
    Ok(json!({
        "key": key, "key_a": ka.describe(&da),
        "a": ra.map(|r| part_json(&da, r)), "a_hits": hits_a,
        "b": match (&side_b, rb) { (Some((d, _)), Some(r)) => part_json(d, r), _ => Value::Null },
        "b_hits": side_b.as_ref().map(|s| s.1.clone()).unwrap_or_default(),
        "tests": tests,
    }))
}

/// A new in-memory dataset holding the parts of `ds` that match `filter`
/// (e.g. `$match` / `not $match`). It has no source file of its own, so raw
/// records and STDF split are not available on it.
fn subset_op(reg: &Registry, pp: &P) -> Result<Value> {
    let id = pp.usize("ds", 0);
    let (ds, _) = reg.get(id)?;
    let filter = pp.str("filter");
    let r = reg.rows(id, &ds, &filter)?;
    if r.is_empty() {
        bail!("no parts match '{filter}'");
    }
    let mut sub = ds.subset(&r);
    let name = pp.str_or("name", &format!("{} [{filter}]", reg.name(id)));
    sub.name = name.clone();
    let new_id = reg.add(name.clone(), vec![], sub);
    Ok(json!({"id": new_id, "name": name, "parts": r.len()}))
}

/// Marker tests: affected group (B / `select`) against a reference group
/// (A / `vs`), optionally with the same units in a later file for drift.
fn markers_op(reg: &Registry, pp: &P) -> Result<Value> {
    use crate::analysis::markers::{self, Group, Later, MarkerOptions};
    let a_id = pp.usize("a", 0);
    let (da, _) = reg.get(a_id)?;
    let mode = pp.str_or("mode", "datasets");
    let (db, ra, rb, la, lb) = if mode == "group" {
        let sel = pp.str("select");
        if sel.is_empty() {
            bail!("enter a selection expression for the affected group");
        }
        let vs = pp.str("vs");
        let (ra, rb) = group_rows(reg, a_id, &da, &sel, &vs, &pp.str("filter"))?;
        let lab = if vs.is_empty() {
            "rest".to_string()
        } else {
            vs
        };
        (da.clone(), ra, rb, lab, sel)
    } else {
        let b_id = pp.usize("b", a_id);
        let (db, _) = reg.get(b_id)?;
        let ra = reg.rows(a_id, &da, &pp.str("filter_a"))?;
        let rb = reg.rows(b_id, &db, &pp.str("filter_b"))?;
        (db, ra, rb, reg.name(a_id), reg.name(b_id))
    };
    // same units in a later file: pair every part of A with B by identity
    let later_ds;
    let later = match pp.opt_usize("later") {
        Some(l) if mode == "group" && l != a_id => {
            let (dl, _) = reg.get(l)?;
            later_ds = dl;
            let ka_s = pp.str_or("key_a", "part_txt");
            let ka = data::matching::KeySpec::parse(&ka_s, &da)?;
            let kb = data::matching::KeySpec::parse(&pp.str_or("key_b", &ka_s), &later_ds)?;
            let m = data::matching::match_parts(
                &da,
                &da.all_rows(),
                &ka,
                &later_ds,
                &later_ds.all_rows(),
                &kb,
            );
            Some(Later {
                ds: &later_ds,
                pairs: m.pairs.iter().map(|(x, y, _)| (*x, *y)).collect(),
                name: reg.name(l),
            })
        }
        _ => None,
    };
    let opt = MarkerOptions {
        forest: pp.bool("forest", true),
        trees: pp.usize("trees", 150),
        match_by: pp.str_or("match_by", "key"),
        min_n: pp.usize("min_n", 5),
        exclude: parse_nums(&pp.str("exclude"))?,
        methods: methods_param(pp)?,
    };
    let rep = markers::find(
        &Group {
            ds: &da,
            rows: ra.clone(),
        },
        &Group {
            ds: &db,
            rows: rb.clone(),
        },
        later.as_ref(),
        &opt,
    );
    let range = viz::RangeMode::parse(&pp.str("range"));
    // overlays of the top candidates
    let (top_keys, top): (Vec<String>, Vec<Value>) = rep
        .rows
        .iter()
        .take(pp.usize("top_charts", 6))
        .filter_map(|m| {
            let ta = da.find_test(&m.key)?;
            let tb = if std::ptr::eq(&*da, &*db) {
                ta
            } else {
                db.test_index
                    .get(&m.key)
                    .copied()
                    .or_else(|| db.find_test(&m.num.to_string()))?
            };
            let g = vec![
                ("reference".to_string(), da.values_of(ta, Some(&ra))),
                ("affected".to_string(), db.values_of(tb, Some(&rb))),
            ];
            let mut f = viz::histogram_fine(&da.tests[ta], &g, None, true, range);
            f["layout"]["title"]["text"] = json!(format!(
                "{} · {} (score {:.2})",
                da.tests[ta].label(),
                m.why,
                m.score
            ));
            Some((m.key.clone(), f))
        })
        .unzip();
    let bars: Vec<&markers::MarkerRow> = rep.rows.iter().take(20).collect();
    let why_color = |w: &str| match w {
        "separation" => "@s1",
        "tails" => "@s2",
        "drift" => "@s3",
        "correlation break" => "@s7",
        "residual" => "@s4",
        "ANOVA" => "@s6",
        _ => "@s5",
    };
    let mut whys: Vec<&str> = bars.iter().map(|r| r.why.as_str()).collect();
    whys.sort();
    whys.dedup();
    let score_fig = json!({"data": whys.iter().map(|w| {
        let sel: Vec<&&markers::MarkerRow> = bars.iter().filter(|r| r.why == *w).collect();
        json!({"type": "bar", "orientation": "h", "name": w, "y": sel.iter().map(|r| r.key.clone()).collect::<Vec<_>>(),
            "x": sel.iter().map(|r| r.score).collect::<Vec<_>>(), "marker": {"color": why_color(w)},
            "hovertemplate": "%{y}<br>score %{x:.2f}<extra>%{fullData.name}</extra>"})
    }).collect::<Vec<_>>(), "layout": {
        "title": {"text": format!("Top marker candidates ({} affected vs {} reference)", rep.n_aff, rep.n_ref), "x": 0.01, "xanchor": "left", "font": {"size": 15}},
        "xaxis": {"title": {"text": "candidate score (strongest signal)"}, "range": [0, 1.02]},
        "yaxis": {"autorange": "reversed", "type": "category", "categoryorder": "array", "categoryarray": bars.iter().map(|r| r.key.clone()).collect::<Vec<_>>()},
        "barmode": "overlay", "margin": {"l": 190, "r": 20, "t": 64, "b": 50},
        "legend": {"orientation": "h", "x": 1, "xanchor": "right", "y": 1.0, "yanchor": "bottom"}}});
    let cinfo = data::TestInfo {
        key: "sdc".into(),
        num: 0,
        name: "tests beyond ±3σ of reference".into(),
        kind: data::TestKind::Parametric,
        units: "tests per part".into(),
        lo: None,
        hi: None,
        lo_spec: None,
        hi_spec: None,
        res_scal: 0,
        exec_count: 0,
        fail_count: 0,
    };
    let f = |v: &[usize]| v.iter().map(|&x| x as f64).collect::<Vec<_>>();
    let mut sdc = viz::histogram_fine(
        &cinfo,
        &[
            ("reference".into(), f(&rep.sdc.counts_ref)),
            ("affected".into(), f(&rep.sdc.counts_aff)),
        ],
        None,
        true,
        viz::RangeMode::Data,
    );
    sdc["layout"]["title"]["text"] = json!(format!(
        "Sigma-deviation count per part (AUC {:.2})",
        rep.sdc.aff_auc
    ));
    let rows: Vec<Value> = rep
        .rows
        .iter()
        .map(|r| serde_json::to_value(r).unwrap_or(Value::Null))
        .collect();
    Ok(json!({
        "label_ref": la, "label_aff": lb, "n_ref": rep.n_ref, "n_aff": rep.n_aff, "n_tests": rep.n_tests,
        "later": rep.later, "forest_auc": rep.forest_auc.map(n), "forest_error": rep.forest_error,
        "sdc_auc": n(rep.sdc.aff_auc), "rows": rows,
        "methods": opt.methods, "methods_all": markers::METHODS,
        "figures": {"scores": score_fig, "top": top, "top_keys": top_keys, "sdc": sdc},
    }))
}

/// `methods`: the marker score methods to count, as a list or a comma-separated
/// string; missing = the defaults.
fn methods_param(pp: &P) -> Result<Vec<String>> {
    use crate::analysis::markers::{default_methods, METHODS};
    let v: Vec<String> = match pp.0.get("methods") {
        None | Some(Value::Null) => return Ok(default_methods()),
        Some(Value::String(s)) => s
            .split(',')
            .map(|x| x.trim().to_string())
            .filter(|x| !x.is_empty())
            .collect(),
        Some(_) => str_list(pp.0, "methods"),
    };
    for m in &v {
        if !METHODS.contains(&m.as_str()) {
            bail!("unknown score method '{m}' (use {})", METHODS.join(", "));
        }
    }
    if v.is_empty() {
        bail!("tick at least one score method");
    }
    Ok(v)
}

/// CSV export of the part table plus selected (or all numeric) test columns.
pub fn export_csv(ds: &Dataset, rows: &[usize], tests: &[usize]) -> String {
    let mut w = csv::Writer::from_writer(vec![]);
    let mut hdr: Vec<String> = [
        "index", "file", "part_id", "wafer", "head", "site", "x", "y", "hbin", "sbin", "pass",
        "test_t",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    hdr.extend(tests.iter().map(|&t| ds.tests[t].key.clone()));
    let _ = w.write_record(&hdr);
    for &r in rows {
        let p = &ds.parts[r];
        let mut rec = vec![
            r.to_string(),
            ds.sources
                .get(p.file as usize)
                .map(|s| data::short_name(&s.path))
                .unwrap_or_default(),
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
                .map(|b| if b { "1" } else { "0" })
                .unwrap_or("")
                .to_string(),
            p.test_t.to_string(),
        ];
        for &t in tests {
            let v = ds.values[t][r];
            rec.push(if v.is_finite() {
                format!("{v}")
            } else {
                String::new()
            });
        }
        let _ = w.write_record(&rec);
    }
    String::from_utf8(w.into_inner().unwrap_or_default()).unwrap_or_default()
}

pub fn export_csv_op(reg: &Registry, p: &Value) -> Result<String> {
    let pp = P(p);
    let id = pp.usize("ds", 0);
    let (ds, _) = reg.get(id)?;
    let r = reg.rows(id, &ds, &pp.str("filter"))?;
    let tests: Vec<usize> = {
        let t = pp.str("tests");
        if t.is_empty() {
            (0..ds.n_tests()).collect()
        } else {
            t.split(',')
                .filter_map(|q| ds.find_test(q.trim()))
                .collect()
        }
    };
    Ok(export_csv(&ds, &r, &tests))
}

/// Bad-candidate profile: what the parts of set `name` (e.g. good here but bad
/// in a later file) have in common against the rest of the population
/// (`filter`): per-part outlier indicators, factor enrichment (site, wafer,
/// wafer zone, stepping-field position…), where they sit on the wafer, what
/// they failed in the later file, and a per-candidate list.
fn candidates_op(reg: &Registry, pp: &P) -> Result<Value> {
    use crate::analysis::{self, candidates as cand};
    use rayon::prelude::*;
    use std::collections::HashMap;
    let id = pp.usize("a", 0);
    let (ds, _) = reg.get(id)?;
    let name = set_name(&pp.str("name"));
    let flags = reg
        .set(id, &name)
        .ok_or_else(|| anyhow!("part set ${name} does not exist yet: match the files first"))?;
    let pop = reg.rows(id, &ds, &pp.str("filter"))?;
    let aff: Vec<bool> = pop
        .iter()
        .map(|&r| flags.get(r).copied().unwrap_or(false))
        .collect();
    let n_aff = aff.iter().filter(|&&a| a).count();
    if n_aff == 0 {
        bail!("none of the parts in the population is in ${name}");
    }
    if n_aff == pop.len() {
        bail!("every part of the population is in ${name}: nothing left to compare against");
    }
    let exclude = parse_nums(&pp.str("exclude"))?;
    let tests: Vec<usize> = ds
        .numeric_tests()
        .into_iter()
        .filter(|&t| !exclude.contains(&ds.tests[t].num))
        .collect();
    let split = |v: &[f64]| -> (Vec<f64>, Vec<f64>) {
        let (mut r, mut a) = (vec![], vec![]);
        for (x, &is) in v.iter().zip(&aff) {
            if is {
                a.push(*x)
            } else {
                r.push(*x)
            }
        }
        (r, a)
    };

    // sigma-deviation count and worst tests against the reference
    let ref_rows: Vec<usize> = pop
        .iter()
        .zip(&aff)
        .filter(|p| !*p.1)
        .map(|p| *p.0)
        .collect();
    let centers: Vec<(usize, f64, f64)> = tests
        .par_iter()
        .filter_map(|&t| {
            let v = ds.values_of(t, Some(&ref_rows));
            if v.len() < 10 {
                return None;
            }
            let s = analysis::stats::robust_sigma(&v);
            (s > 0.0).then(|| (t, analysis::stats::median(&v), s))
        })
        .collect();
    let zs = |row: usize| -> Vec<(usize, f64)> {
        centers
            .iter()
            .filter_map(|&(t, c, s)| {
                let v = ds.value(t, row);
                v.is_finite().then(|| (t, (v - c) / s))
            })
            .collect()
    };
    let sdc: Vec<f64> = pop
        .par_iter()
        .map(|&r| zs(r).iter().filter(|z| z.1.abs() > 3.0).count() as f64)
        .collect();

    // unsupervised outlier scores over the whole population
    let an = anomaly::detect(
        &ds,
        &pop,
        &anomaly::AnomalyOptions {
            tests: Some(tests.clone()),
            ..Default::default()
        },
    );
    let col =
        |f: &dyn Fn(&anomaly::RowScores) -> f64| -> Vec<f64> { an.per_row.iter().map(f).collect() };
    let mut indicators = vec![];
    for (key, label, v, flag) in [
        (
            "sdc",
            "Sigma-deviation count (tests beyond ±3σ)",
            sdc.clone(),
            None,
        ),
        (
            "robust_z",
            "Largest robust z of any test",
            col(&|r| r.max_robust_z),
            Some(6.0),
        ),
        (
            "pat",
            "Tests outside PAT limits",
            col(&|r| r.n_pat as f64),
            Some(1.0),
        ),
        (
            "nnr",
            "Wafer neighbourhood residual (NNR)",
            col(&|r| r.max_nnr_z),
            Some(5.0),
        ),
        (
            "mahalanobis",
            "Mahalanobis distance (all tests)",
            col(&|r| r.mahal_d2),
            None,
        ),
        (
            "iforest",
            "Isolation Forest score",
            col(&|r| r.iforest),
            Some(0.62),
        ),
        (
            "combined",
            "Combined anomaly score",
            an.scores.clone(),
            None,
        ),
    ] {
        let (r, a) = split(&v);
        if let Some(i) = cand::indicator(key, label, &r, &a, flag) {
            indicators.push(i);
        }
    }
    indicators.sort_by(|a, b| {
        b.auc
            .partial_cmp(&a.auc)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // die positions: PRR X/Y, else rebuilt from logged tests
    let txy = TestXy::parse(&ds, pp).ok();
    let use_prr = pp.str("coords") != "tests" && pop.iter().any(|&r| ds.parts[r].x != -32768);
    let pos: Vec<Option<(String, f64, f64)>> = pop
        .iter()
        .map(|&r| {
            let p = &ds.parts[r];
            if use_prr {
                (p.x != -32768).then(|| {
                    (
                        format!("{}|{}", p.file, ds.wafer_name(p)),
                        p.x as f64,
                        p.y as f64,
                    )
                })
            } else {
                let t = txy.as_ref()?;
                let (x, y) = t.xy(&ds, r)?;
                Some((t.wafer(&ds, r)?, x as f64, y as f64))
            }
        })
        .collect();

    // factors
    let lab = |f: &dyn Fn(usize) -> Option<String>| -> Vec<Option<String>> {
        pop.iter().map(|&r| f(r)).collect()
    };
    let mut fl: Vec<(&str, &str, Vec<Option<String>>)> = vec![
        (
            "site",
            "Test site",
            lab(&|r| Some(format!("site {}", ds.parts[r].site))),
        ),
        (
            "head",
            "Test head",
            lab(&|r| Some(format!("head {}", ds.parts[r].head))),
        ),
        (
            "sbin",
            "Soft bin",
            lab(&|r| Some(format!("sbin {}", ds.parts[r].sbin))),
        ),
        (
            "hbin",
            "Hard bin",
            lab(&|r| Some(format!("hbin {}", ds.parts[r].hbin))),
        ),
        (
            "lot",
            "Lot / file",
            lab(&|r| {
                ds.sources
                    .get(ds.parts[r].file as usize)
                    .map(|s| format!("{} ({})", s.lot_id, s.sublot_id))
            }),
        ),
        (
            "alarm",
            "Tester alarm on any test",
            lab(&|r| {
                Some(
                    if ds.parts[r].alarms.is_empty() {
                        "no alarm"
                    } else {
                        "alarm"
                    }
                    .to_string(),
                )
            }),
        ),
        (
            "wafer",
            "Wafer",
            pos.iter()
                .map(|p| {
                    p.as_ref()
                        .map(|p| p.0.rsplit('|').next().unwrap_or(&p.0).to_string())
                })
                .collect(),
        ),
        ("zone", "Wafer zone (radius)", cand::zones(&pos)),
    ];
    let lbt = data::loadboard_test();
    fl.push((
        "loadboard",
        "Loadboard (file-level test value)",
        lab(&|r| {
            ds.file_value(r, lbt)
                .map(|b| format!("LB {}", data::FieldValue::Num(b).as_group_key()))
        }),
    ));
    fl.push((
        "site_loadboard",
        "Site × loadboard",
        lab(&|r| match ds.field(r, "site_loadboard") {
            data::FieldValue::Str(s) => Some(s),
            _ => None,
        }),
    ));
    if let Some(t) = txy.as_ref().filter(|_| !use_prr) {
        fl.push((
            "field",
            "Position in the stepping field (X/Y before offset)",
            lab(&|r| t.field_xy(&ds, r).map(|(x, y)| format!("({x},{y})"))),
        ));
    }
    let mut factors: Vec<cand::Factor> = fl
        .into_iter()
        .filter_map(|(k, n, l)| cand::factor(k, n, &l, &aff))
        .collect();
    factors.sort_by(|a, b| a.p.partial_cmp(&b.p).unwrap_or(std::cmp::Ordering::Equal));

    // stacked map: parts and candidates per die position over all wafers
    let mut stack: IndexMap<(i64, i64), (usize, usize)> = IndexMap::new();
    for (p, &a) in pos.iter().zip(&aff) {
        if let Some((_, x, y)) = p {
            let e = stack.entry((*x as i64, *y as i64)).or_default();
            e.0 += 1;
            e.1 += a as usize;
        }
    }
    let stack: Vec<Value> = stack
        .iter()
        .map(|(&(x, y), &(n, k))| json!({"x": x, "y": y, "n": n, "k": k}))
        .collect();

    // the later file: what the candidates failed there
    let link = {
        let m = reg.matches.read().unwrap();
        m.iter().find(|l| l.a == id && l.name == name).map(|l| {
            (
                l.b,
                l.pairs
                    .iter()
                    .map(|(x, y, k)| (*x, (*y, k.clone())))
                    .collect::<HashMap<usize, (usize, String)>>(),
            )
        })
    };
    let later = match &link {
        Some((b, pairs)) => Some((reg.get(*b)?.0, pairs)),
        None => None,
    };
    let mut b_fails: IndexMap<String, usize> = IndexMap::new();
    let mut b_bins: IndexMap<u16, usize> = IndexMap::new();
    if let Some((db, pairs)) = &later {
        for (&r, _) in pop.iter().zip(&aff).filter(|p| *p.1) {
            if let Some((rb, _)) = pairs.get(&r) {
                let p = &db.parts[*rb];
                *b_bins.entry(p.hbin).or_default() += 1;
                for &t in &p.fails {
                    *b_fails.entry(db.tests[t as usize].key.clone()).or_default() += 1;
                }
            }
        }
    }
    b_fails.sort_by(|_, a, _, b| b.cmp(a));
    b_bins.sort_by(|_, a, _, b| b.cmp(a));

    // one row per candidate
    let ref_combined: Vec<f64> = analysis::stats::sorted(&split(&an.scores).0);
    let pct_of = |v: f64| {
        if ref_combined.is_empty() || !v.is_finite() {
            return Value::Null;
        }
        json!(ref_combined.partition_point(|&x| x < v) as f64 * 100.0 / ref_combined.len() as f64)
    };
    let parts: Vec<Value> = pop
        .iter()
        .enumerate()
        .filter(|(k, _)| aff[*k])
        .take(5000)
        .map(|(k, &r)| {
            let p = &ds.parts[r];
            let mut z = zs(r);
            z.sort_by(|a, b| b.1.abs().partial_cmp(&a.1.abs()).unwrap_or(std::cmp::Ordering::Equal));
            let worst: Vec<String> = z.iter().take(3).map(|(t, z)| format!("{} ({:+.1}σ)", ds.tests[*t].key, z)).collect();
            let (b_hbin, fails, key) = match &later {
                Some((db, pairs)) => match pairs.get(&r) {
                    Some((rb, key)) => {
                        let q = &db.parts[*rb];
                        let f: Vec<String> = q.fails.iter().take(4).map(|&t| db.tests[t as usize].key.clone()).collect();
                        (json!(q.hbin), f.join(", "), key.clone())
                    }
                    None => (Value::Null, String::new(), String::new()),
                },
                None => (Value::Null, String::new(), String::new()),
            };
            let (wafer, x, y) = match &pos[k] {
                Some((w, x, y)) => (w.rsplit('|').next().unwrap_or(w).to_string(), json!(x), json!(y)),
                None => (String::new(), Value::Null, Value::Null),
            };
            json!({"row": r, "key": key, "part_id": p.part_id, "part_txt": p.part_txt, "wafer": wafer, "x": x, "y": y,
                "site": p.site, "sbin": p.sbin, "b_hbin": b_hbin, "b_fails": fails,
                "anomaly_pct": pct_of(an.scores[k]), "iforest": n(an.per_row[k].iforest), "sdc": sdc[k] as usize,
                "worst": worst.join(", ")})
        })
        .collect();

    Ok(json!({
        "name": name, "n_ref": pop.len() - n_aff, "n_aff": n_aff, "n_tests": centers.len(), "excluded": exclude.len(),
        "later": link.as_ref().map(|l| reg.name(l.0)),
        "indicators": indicators, "factors": factors, "stack": stack, "coords": if use_prr { "prr" } else if txy.is_some() { "tests" } else { "" },
        "b_fails": b_fails.iter().map(|(k, c)| json!({"key": k, "n": c, "share": *c as f64 / n_aff as f64})).collect::<Vec<_>>(),
        "b_bins": b_bins.iter().map(|(k, c)| json!({"bin": k, "n": c})).collect::<Vec<_>>(),
        "parts": parts,
    }))
}

/// Retest trace: follow the parts of run 1 (`runs[0]`, rows matching
/// `filter`, default `not pass`) through the later runs (`runs`: dataset ids
/// in order, each a retest of the previous run's rejects), matched by `key`,
/// then by `link` (optional, e.g. `part_txt`) where the key is missing.
/// Traces the values of `tests` (blank = the most common first fail of the
/// traced parts, leaving out `exclude`) in display units, summarizes each
/// run, and stores `$retest_recovered` / `$retest_still_bad` /
/// `$retest_not_retested` part sets in every run.
fn retest_op(reg: &Registry, pp: &P) -> Result<Value> {
    use crate::analysis::candidates::display_unit;
    use crate::analysis::markers::pair_test;
    use crate::analysis::retest::{self, Outcome, Run, Status};
    use data::matching::KeySpec;
    let ids: Vec<usize> = match pp.0.get("runs") {
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(|v| v.as_u64().or_else(|| v.as_str()?.trim().parse().ok()))
            .map(|v| v as usize)
            .collect(),
        Some(Value::String(s)) => s.split(',').filter_map(|x| x.trim().parse().ok()).collect(),
        _ => vec![],
    };
    if ids.len() < 2 {
        bail!("pick at least two runs: run 1 and its retest");
    }
    let mut seen = std::collections::HashSet::new();
    if !ids.iter().all(|i| seen.insert(*i)) {
        bail!("each run must be a different dataset");
    }
    let dss: Vec<Arc<Dataset>> = ids
        .iter()
        .map(|&i| reg.get(i).map(|d| d.0))
        .collect::<Result<_>>()?;
    let key_s = pp.str_or("key", "part_txt");
    // with a link, a run whose key cannot be read (its test missing) is
    // followed by the link alone
    let link_s = pp.str("link");
    let link_s = link_s.trim();
    let runs: Vec<Run> = dss
        .iter()
        .enumerate()
        .map(|(k, d)| {
            let key = KeySpec::parse(&key_s, d);
            if link_s.is_empty() {
                return Ok(Run { ds: d, key: Some(key?), link: None });
            }
            let link = KeySpec::parse(link_s, d);
            if let (Err(a), Err(b)) = (&key, &link) {
                bail!("run {}: neither the key nor the link can be read: {a}; {b}", k + 1);
            }
            Ok(Run { ds: d, key: key.ok(), link: link.ok() })
        })
        .collect::<Result<_>>()?;
    let key_d = runs.iter().find_map(|r| r.key.as_ref().map(|s| s.describe(r.ds)));
    let link_d = runs.iter().find_map(|r| r.link.as_ref().map(|s| s.describe(r.ds)));
    let key_desc = match (key_d, link_d) {
        (Some(k), Some(l)) => format!("{k}, then {l}"),
        (k, l) => k.or(l).unwrap_or_default(),
    };
    let d1 = &dss[0];
    let filter = pp.str_or("filter", "not pass");
    let start = reg.rows(ids[0], d1, &filter)?;
    if start.is_empty() {
        bail!("no part of run 1 matches '{filter}'");
    }
    let au = retest::audit(&runs);
    let parts = retest::trace(&runs, &start, &au);
    if parts.is_empty() {
        bail!(
            "none of the {} selected parts of run 1 has a key ({})",
            start.len(),
            key_desc
        );
    }

    // traced tests: as given, else the most common first fail
    let exclude = parse_nums(&pp.str("exclude"))?;
    let q = pp.str("tests");
    let tests: Vec<usize> = if q.trim().is_empty() {
        let mut c: IndexMap<usize, usize> = IndexMap::new();
        for p in &parts {
            if let Some(f) = p.rows[0].and_then(|r| d1.parts[r].first_fail) {
                if !exclude.contains(&d1.tests[f as usize].num) {
                    *c.entry(f as usize).or_default() += 1;
                }
            }
        }
        c.sort_by(|_, a, _, b| b.cmp(a));
        let t = c.keys().next().copied().ok_or_else(|| {
            anyhow!("the traced parts have no first fail: enter the test to trace")
        })?;
        vec![t]
    } else {
        q.split([',', ';'])
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| test_ref(d1, s.split(':').next().unwrap_or(s).trim()))
            .collect::<Result<_>>()?
    };
    let by = pp.str_or("match_by", "key");
    // each traced test in each run, with its display scale
    let tmap: Vec<Vec<Option<(usize, f64)>>> = tests
        .iter()
        .map(|&t| {
            dss.iter()
                .map(|d| {
                    let t2 = if Arc::ptr_eq(d, d1) {
                        Some(t)
                    } else {
                        pair_test(d1, d, t, &by).or_else(|| pair_test(d1, d, t, "num"))
                    }?;
                    Some((t2, display_unit(&d.tests[t2].units, d.tests[t2].res_scal).1))
                })
                .collect()
        })
        .collect();
    let fails_on = |ti: usize, k: usize, r: usize| -> bool {
        tmap[ti][k]
            .map(|(t, _)| {
                let v = dss[k].value(t, r);
                dss[k].parts[r].has_fail(t) || (v.is_finite() && !dss[k].tests[t].within_limits(v))
            })
            .unwrap_or(false)
    };
    let summary = retest::summarize(&parts, dss.len(), &|k, r| fails_on(0, k, r));

    // recovery by failing value, per traced test: (value, recovered) points
    // from each part's first fail on the test, or from every retest of a
    // failing part (`value_from` = each); repeatability loops can feed the
    // same points later
    let each = pp.str("value_from") == "each";
    let (max_rate, min_zone) = (pp.f64("max_rate", 0.10), pp.usize("min_zone", 10));
    let recovery: Vec<Value> = (0..tests.len())
        .map(|ti| {
            let val = |k: usize, r: usize| tmap[ti][k].map(|(t, f)| dss[k].value(t, r) * f);
            let mut pts: Vec<(f64, bool, &str, usize)> = vec![];
            for p in &parts {
                if each {
                    for k in 0..dss.len() - 1 {
                        if let (Some(r), Some(_)) = (p.rows[k], p.rows[k + 1]) {
                            if p.status[k] == Status::Fail && fails_on(ti, k, r) {
                                if let Some(v) = val(k, r).filter(|v| v.is_finite()) {
                                    pts.push((v, p.status[k + 1] == Status::Pass, &p.key, k));
                                }
                            }
                        }
                    }
                } else if p.outcome != Outcome::PassedRun1 {
                    let first = (0..dss.len()).find(|&k| {
                        p.status[k] == Status::Fail && p.rows[k].is_some_and(|r| fails_on(ti, k, r))
                    });
                    // not retested at the end still counts once it was retested
                    // after that fail (e.g. the reject chain ended, the next
                    // run retested good parts)
                    let retested_after = |k: usize| p.rows[k + 1..].iter().any(|r| r.is_some());
                    if let Some(k) = first.filter(|&k| p.outcome != Outcome::NotRetested || retested_after(k)) {
                        if let Some(v) = val(k, p.rows[k].unwrap()).filter(|v| v.is_finite()) {
                            pts.push((v, p.outcome == Outcome::Recovered, &p.key, k));
                        }
                    }
                }
            }
            let xy: Vec<(f64, bool)> = pts.iter().map(|p| (p.0, p.1)).collect();
            let lim = tmap[ti][0].map(|(t, f)| {
                let i = &dss[0].tests[t];
                (i.lo.map(|v| v * f), i.hi.map(|v| v * f))
            });
            let (lo, hi) = lim.unwrap_or((None, None));
            json!({
                "value_from": if each { "each" } else { "first" }, "max_rate": max_rate, "min_zone": min_zone,
                "points": pts.iter().map(|p| json!({"key": p.2, "value": n(p.0), "recovered": p.1, "run": p.3})).collect::<Vec<_>>(),
                "bins": retest::recovery_bins(&xy, pp.usize("n_bins", 10)),
                "zones": retest::recovery_zones(&xy, max_rate, min_zone, lo, hi),
            })
        })
        .collect();

    // loadboard per run (a file-level attribute), and retest recovery per board
    let lb = data::loadboard_test();
    let board_of = |k: usize, r: usize| dss[k].file_value(r, lb);
    let mut by_board: IndexMap<(usize, String), (usize, usize)> = IndexMap::new();
    for p in &parts {
        for k in 1..dss.len() {
            if let (true, Some(r)) = (p.due(k), p.rows[k]) {
                let b = board_of(k, r)
                    .map(|b| data::FieldValue::Num(b).as_group_key())
                    .unwrap_or_else(|| "?".into());
                let e = by_board.entry((k, b)).or_default();
                e.0 += 1;
                e.1 += (p.status[k] == Status::Pass) as usize;
            }
        }
    }

    // bad candidates: good in run 1, then failing the (first) traced test on
    // a retest of good parts and never passing after ($rt_bad_candidate);
    // passing again later ($rt_marginal); passing every retest of good parts
    // ($rt_confirmed_good). Marked in run 1, where their values were good.
    let mut cand_sets: IndexMap<&str, Vec<bool>> =
        ["rt_bad_candidate", "rt_marginal", "rt_confirmed_good"]
            .into_iter()
            .map(|n| (n, vec![false; d1.n_parts()]))
            .collect();
    let mut good_runs = std::collections::BTreeSet::new();
    let mut other_fail = 0;
    for p in &parts {
        let (Some(r1), Status::Pass) = (p.rows[0], p.status[0]) else {
            continue;
        };
        let ks: Vec<usize> = (1..dss.len()).filter(|&k| p.retested_good(k)).collect();
        if ks.is_empty() {
            continue;
        }
        good_runs.extend(ks.iter().copied());
        let first_bad = ks
            .iter()
            .copied()
            .find(|&k| p.status[k] == Status::Fail && p.rows[k].is_some_and(|r| fails_on(0, k, r)));
        let set = match first_bad {
            Some(k) if p.status[k + 1..].contains(&Status::Pass) => "rt_marginal",
            Some(_) => "rt_bad_candidate",
            None if ks.iter().all(|&k| p.status[k] == Status::Pass) => "rt_confirmed_good",
            None => {
                other_fail += 1;
                continue;
            }
        };
        cand_sets[set][r1] = true;
    }
    let cand_count = |n: &str| cand_sets[n].iter().filter(|&&b| b).count();
    let candidates = json!({"good_runs": good_runs.iter().collect::<Vec<_>>(), "n_cand": cand_count("rt_bad_candidate"),
        "n_marginal": cand_count("rt_marginal"), "n_confirmed": cand_count("rt_confirmed_good"), "n_other_fail": other_fail});
    for (name, f) in cand_sets {
        reg.put_set(ids[0], name, f);
    }

    // part sets in every run
    for o in [
        Outcome::Recovered,
        Outcome::StillBad,
        Outcome::NotRetested,
        Outcome::PassedRun1,
    ] {
        for (k, d) in dss.iter().enumerate() {
            let mut f = vec![false; d.n_parts()];
            for p in parts.iter().filter(|p| p.outcome == o) {
                if let Some(r) = p.rows[k] {
                    f[r] = true;
                }
            }
            reg.put_set(ids[k], o.set_name(), f);
        }
    }

    // the same outcome sets in other datasets (e.g. the lot's first run), by key
    for v in
        pp.0.get("also")
            .and_then(|x| x.as_array())
            .cloned()
            .unwrap_or_default()
    {
        let Some(id) = v.as_u64().map(|x| x as usize) else {
            continue;
        };
        let (d, _) = reg.get(id)?;
        let k = KeySpec::parse(&key_s, &d);
        let l = (!link_s.is_empty()).then(|| KeySpec::parse(link_s, &d)).transpose();
        let (k, l) = match (k, l) {
            (Err(e), Ok(None)) => return Err(e),
            (Err(_), Err(e)) => return Err(e),
            (k, l) => (k.ok(), l.ok().flatten()),
        };
        let by_key: std::collections::HashMap<&str, Outcome> = parts
            .iter()
            .filter_map(|p| Some((p.ids.0.as_deref()?, p.outcome)))
            .collect();
        let by_link: std::collections::HashMap<&str, Outcome> = parts
            .iter()
            .filter_map(|p| Some((p.ids.1.as_deref()?, p.outcome)))
            .collect();
        let mut flags: IndexMap<&str, Vec<bool>> = IndexMap::new();
        for o in [
            Outcome::Recovered,
            Outcome::StillBad,
            Outcome::NotRetested,
            Outcome::PassedRun1,
        ] {
            flags.insert(o.set_name(), vec![false; d.n_parts()]);
        }
        for r in 0..d.n_parts() {
            let kk = k.as_ref().and_then(|k| k.key(&d, r));
            // by key, else by link when the row has no key
            let o = match &kk {
                Some(key) => by_key.get(key.as_str()).copied(),
                None => l
                    .as_ref()
                    .and_then(|l| l.key(&d, r))
                    .and_then(|x| by_link.get(x.as_str()).copied()),
            };
            if let Some(o) = o {
                flags[o.set_name()][r] = true;
            }
        }
        for (name, f) in flags {
            reg.put_set(id, name, f);
        }
    }

    // flows between consecutive runs, for the Sankey
    let label = |s: Status| match s {
        Status::Pass => "pass",
        Status::Fail => "fail",
        Status::NotRetested => "not retested",
        Status::PassedEarlier => "passed earlier",
    };
    let mut flows: IndexMap<(usize, &str, &str), usize> = IndexMap::new();
    // parts that never failed would swamp the chart: leave them out
    for p in parts.iter().filter(|p| p.outcome != Outcome::PassedRun1) {
        for k in 1..dss.len() {
            // fails moving on, and good parts that a later run retested
            if p.status[k - 1] == Status::Fail || p.rows[k].is_some() {
                *flows
                    .entry((k - 1, label(p.status[k - 1]), label(p.status[k])))
                    .or_default() += 1;
            }
        }
    }
    let ff = |r: usize, d: &Dataset| {
        d.parts[r]
            .first_fail
            .map(|t| d.tests[t as usize].key.clone())
            .unwrap_or_default()
    };
    // part trace fields (Settings > Part identification and trace): the PRR
    // identity plus the datalogged trace tests (`id_tests`: {field: test}),
    // read from the first run each part was found in
    let id_tests: Vec<(String, String)> = pp
        .0
        .get("id_tests")
        .and_then(|x| x.as_object())
        .map(|o| {
            o.iter()
                .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.trim().to_string())))
                .filter(|(_, v)| !v.is_empty())
                .collect()
        })
        .unwrap_or_default();
    let id_idx: Vec<Vec<Option<usize>>> = dss
        .iter()
        .map(|d| id_tests.iter().map(|(_, q)| d.find_test(q)).collect())
        .collect();
    let ids_of = |k: usize, r: usize| -> Value {
        let d = &dss[k];
        let p = &d.parts[r];
        let mut o = serde_json::Map::new();
        o.insert("part_id".into(), json!(p.part_id));
        o.insert("part_txt".into(), json!(p.part_txt));
        o.insert("wafer".into(), json!(d.wafers.get(p.wafer as usize)));
        o.insert("x".into(), json!(p.x));
        o.insert("y".into(), json!(p.y));
        for ((name, _), t) in id_tests.iter().zip(&id_idx[k]) {
            let v = t.map(|t| d.value(t, r)).filter(|v| v.is_finite());
            // the loadboard is a file attribute: parts missing it take the file's board
            let v = v.or_else(|| (name == "lb").then(|| t.and_then(|t| d.file_value(r, d.tests[t].num))).flatten());
            o.insert(name.clone(), v.map(n).unwrap_or(Value::Null));
        }
        Value::Object(o)
    };
    // ID audit: per run the issues and the traced parts on them, the rows of
    // issues and flagged parts as $rt_id_issue in every run
    let mut issue_at: std::collections::HashMap<(usize, usize), &str> = Default::default();
    for i in &au.issues {
        for &r in &i.rows {
            issue_at.entry((i.run, r)).or_insert(i.kind);
        }
    }
    let part_at: std::collections::HashMap<(usize, usize), &str> = parts
        .iter()
        .flat_map(|p| p.rows.iter().enumerate().filter_map(move |(k, r)| Some(((k, (*r)?), p.key.as_str()))))
        .collect();
    let mut id_sets: Vec<Vec<bool>> = dss.iter().map(|d| vec![false; d.n_parts()]).collect();
    for &(k, r) in issue_at.keys() {
        id_sets[k][r] = true;
    }
    for p in parts.iter().filter(|p| !p.flags.is_empty()) {
        for k in p.flags.iter().map(|f| f.run).chain([0]) {
            if let Some(r) = p.rows[k] {
                id_sets[k][r] = true;
            }
        }
    }
    for (k, f) in id_sets.into_iter().enumerate() {
        reg.put_set(ids[k], "rt_id_issue", f);
    }
    let max_issues = pp.usize("max_issues", 2000);
    let audit = json!({
        "checked": runs.iter().any(|r| r.key.is_some() && r.link.is_some()),
        "n_issues": au.issues.len(),
        "dropped_keys": au.bad_keys.iter().flatten().collect::<std::collections::HashSet<_>>().len(),
        "dropped_links": au.bad_links.iter().flatten().collect::<std::collections::HashSet<_>>().len(),
        "n_flagged": parts.iter().filter(|p| !p.flags.is_empty()).count(),
        "issues": au.issues.iter().take(max_issues).map(|i| {
            let mut tp: Vec<&str> = i.rows.iter().filter_map(|&r| part_at.get(&(i.run, r)).copied()).collect();
            tp.dedup();
            json!({"run": i.run, "kind": i.kind, "value": i.value, "pattern": i.pattern, "n_rows": i.rows.len(),
                "rows": i.rows.iter().take(50).collect::<Vec<_>>(), "others": i.others.iter().take(50).collect::<Vec<_>>(),
                "n_others": i.others.len(), "parts": tp.into_iter().take(50).collect::<Vec<_>>()})
        }).collect::<Vec<_>>(),
        "flags": parts.iter().flat_map(|p| p.flags.iter().map(move |f| json!({"run": f.run, "kind": f.kind, "detail": f.detail, "part": p.key}))).take(max_issues).collect::<Vec<_>>(),
    });
    let limit = pp.usize("limit", 20000);
    let part_rows: Vec<Value> = parts
        .iter()
        .take(limit)
        .map(|p| {
            let runs: Vec<Value> = p
                .rows
                .iter()
                .enumerate()
                .map(|(k, r)| match r {
                    Some(r) => {
                        let d = &dss[k];
                        let vals: Vec<Value> = tmap
                            .iter()
                            .map(|m| m[k].map(|(t, f)| n(d.value(t, *r) * f)).unwrap_or(Value::Null))
                            .collect();
                        let fl: Vec<bool> = (0..tests.len()).map(|ti| fails_on(ti, k, *r)).collect();
                        let alarms: Vec<&str> = d.parts[*r].alarms.iter().map(|&t| d.tests[t as usize].key.as_str()).collect();
                        json!({"row": r, "status": p.status[k], "linked": p.linked[k], "id_key": runs[k].key(*r), "id_link": runs[k].link(*r),
                            "id_issue": issue_at.get(&(k, *r)).copied(), "site": d.parts[*r].site, "retest_good": p.retested_good(k), "alarms": alarms, "hbin": d.parts[*r].hbin, "sbin": d.parts[*r].sbin,
                            "first_fail": ff(*r, d), "values": vals, "fails": fl})
                    }
                    None => json!({"row": Value::Null, "status": p.status[k]}),
                })
                .collect();
            let ids = p.rows.iter().enumerate().find_map(|(k, r)| r.map(|r| ids_of(k, r))).unwrap_or(Value::Null);
            json!({"key": p.key, "outcome": p.outcome, "recovered_in": p.recovered_in, "runs": runs, "ids": ids, "flags": p.flags})
        })
        .collect();
    let by_site = retest::by_site(&parts, dss.len(), &|k, r| dss[k].parts[r].site);
    let count = |o: Outcome| parts.iter().filter(|p| p.outcome == o).count();
    Ok(json!({
        "runs": ids.iter().zip(&dss).map(|(&i, d)| {
            let fv = d.file_values(lb);
            json!({"id": i, "name": reg.name(i), "n_parts": d.n_parts(), "loadboard": fv.iter().map(|f| f.value.map(n)).collect::<Vec<_>>(),
                "loadboard_changed": fv.iter().any(|f| f.distinct > 1), "loadboard_missing": fv.iter().map(|f| f.missing).sum::<usize>()})
        }).collect::<Vec<_>>(),
        "loadboard_test": lb, "recovery": recovery,
        "by_board": by_board.iter().map(|((k, b), (n_, k_))| {
            let (l, u) = retest::wilson(*k_, *n_);
            json!({"run": k, "board": b, "retested": n_, "pass": k_, "rate": *k_ as f64 / *n_ as f64, "ci_lo": l, "ci_hi": u})
        }).collect::<Vec<_>>(),
        "key": key_s, "link": link_s, "key_desc": key_desc,
        "n_linked": parts.iter().filter(|p| p.linked.contains(&true)).count(), "filter": filter, "n_start": start.len(), "n_parts": parts.len(),
        "tests": tests.iter().zip(&tmap).map(|(&t, m)| {
            let info = &d1.tests[t];
            json!({"key": info.key, "num": info.num, "units": display_unit(&info.units, info.res_scal).0,
                "limits": m.iter().enumerate().map(|(k, x)| match x {
                    Some((t2, f)) => { let ti = &dss[k].tests[*t2]; json!({"lo": ti.lo.map(|v| n(v * f)), "hi": ti.hi.map(|v| n(v * f))}) }
                    None => Value::Null,
                }).collect::<Vec<_>>()})
        }).collect::<Vec<_>>(),
        "outcomes": {"passed_run1": count(Outcome::PassedRun1), "recovered": count(Outcome::Recovered),
            "still_bad": count(Outcome::StillBad), "not_retested": count(Outcome::NotRetested)},
        "summary": summary, "candidates": candidates,
        "by_site": {"counts": by_site.counts, "p": by_site.p.iter().map(|&v| n(v)).collect::<Vec<_>>()},
        "flows": flows.iter().map(|((k, a, b), c)| json!({"run": k, "from": a, "to": b, "n": c})).collect::<Vec<_>>(),
        "parts": part_rows, "truncated": parts.len() > limit, "audit": audit,
    }))
}

/// Commonality across the lots of an investigation case. Each lot (`lots`:
/// [{name, ds, kind: issue | clean, bad, good, groups: [{label, filter}]}])
/// contrasts a bad group with a good group of its first run (filters). Per
/// factor (site, loadboard, site × loadboard, wafer, wafer zone, reticle
/// position, test-time quartile): the share of every group per level, a χ²
/// test of bad vs good per lot (BH-corrected within the lot), and two flags:
/// the factor sets the bad parts apart in every issue lot but in no clean lot
/// (within lots), or one level holds most bad parts of every issue lot but
/// few parts of the clean lots (across lots, e.g. a loadboard used for the
/// issue lots only). Also the top marker tests of each lot side by side.
fn commonality_op(reg: &Registry, pp: &P) -> Result<Value> {
    use crate::analysis::candidates as cand;
    use crate::analysis::markers::{self, Group, MarkerOptions};
    use std::collections::{BTreeSet, HashMap, HashSet};
    let lots_in =
        pp.0.get("lots")
            .and_then(|x| x.as_array())
            .cloned()
            .unwrap_or_default();
    if lots_in.is_empty() {
        bail!("add at least one lot to the case");
    }
    let exclude = parse_nums(&pp.str("exclude"))?;
    const FACTORS: [(&str, &str); 7] = [
        ("site", "Test site"),
        ("loadboard", "Loadboard"),
        ("site_loadboard", "Site × loadboard"),
        ("wafer", "Wafer"),
        ("zone", "Wafer zone (radius)"),
        ("reticle", "Reticle position"),
        ("test_time", "Test time (quartile)"),
    ];
    struct Lot {
        name: String,
        kind: String,
        ds: Arc<Dataset>,
        bad: Vec<usize>,
        good: Vec<usize>,
        groups: Vec<(String, Vec<usize>)>,
        /// factor → row → level
        labels: Vec<HashMap<usize, String>>,
    }
    let mut lots: Vec<Lot> = vec![];
    for (i, l) in lots_in.iter().enumerate() {
        let lp = P(l);
        let id = lp.usize("ds", usize::MAX);
        let (ds, _) = reg.get(id).with_context(|| format!("lot {}", i + 1))?;
        let name = lp.str_or("name", &reg.name(id));
        let bad = reg
            .rows(id, &ds, &lp.str("bad"))
            .with_context(|| format!("{name}: bad parts"))?;
        let bad_set: HashSet<usize> = bad.iter().copied().collect();
        let good: Vec<usize> = reg
            .rows(id, &ds, &lp.str_or("good", "pass"))
            .with_context(|| format!("{name}: good parts"))?
            .into_iter()
            .filter(|r| !bad_set.contains(r))
            .collect();
        let mut groups = vec![];
        for g in l
            .get("groups")
            .and_then(|x| x.as_array())
            .cloned()
            .unwrap_or_default()
        {
            let gp = P(&g);
            // a group whose part set does not exist yet is left out
            if let Ok(rows) = reg.rows(id, &ds, &gp.str("filter")) {
                groups.push((gp.str("label"), rows));
            }
        }
        // the factor levels of every row in play
        let mut rows: BTreeSet<usize> = bad.iter().chain(&good).copied().collect();
        for g in &groups {
            rows.extend(g.1.iter().copied());
        }
        let rows: Vec<usize> = rows.into_iter().collect();
        let txy = TestXy::parse(&ds, pp).ok();
        let prr = rows.iter().any(|&r| ds.parts[r].x != -32768);
        let pos: Vec<Option<(String, f64, f64)>> = rows
            .iter()
            .map(|&r| {
                let p = &ds.parts[r];
                if prr {
                    (p.x != -32768).then(|| {
                        (
                            format!("{}|{}", p.file, ds.wafer_name(p)),
                            p.x as f64,
                            p.y as f64,
                        )
                    })
                } else {
                    let t = txy.as_ref()?;
                    let (x, y) = t.xy(&ds, r)?;
                    Some((t.wafer(&ds, r)?, x as f64, y as f64))
                }
            })
            .collect();
        let zones = cand::zones(&pos);
        let mut tt: Vec<f64> = rows
            .iter()
            .map(|&r| ds.parts[r].test_t as f64)
            .filter(|&v| v > 0.0)
            .collect();
        tt.sort_by(|a, b| a.total_cmp(b));
        let q = |f: f64| {
            tt.get(((tt.len() as f64 - 1.0) * f).round() as usize)
                .copied()
        };
        let (q1, q2, q3) = (q(0.25), q(0.5), q(0.75));
        let mut labels: Vec<HashMap<usize, String>> = vec![HashMap::new(); FACTORS.len()];
        for (k, &r) in rows.iter().enumerate() {
            let p = &ds.parts[r];
            let lab: [Option<String>; 7] = [
                Some(format!("site {}", p.site)),
                ds.file_value(r, data::loadboard_test())
                    .map(|b| format!("LB {}", data::FieldValue::Num(b).as_group_key())),
                match ds.field(r, "site_loadboard") {
                    data::FieldValue::Str(s) => Some(s),
                    _ => None,
                },
                Some(ds.wafer_name(p).to_string()).filter(|w| !w.is_empty()),
                zones[k].clone(),
                txy.as_ref()
                    .and_then(|t| t.field_xy(&ds, r))
                    .map(|(x, y)| format!("({x},{y})")),
                match (p.test_t as f64, q1, q2, q3) {
                    (v, Some(a), Some(b), Some(c)) if v > 0.0 => Some(
                        if v <= a {
                            "Q1 fastest"
                        } else if v <= b {
                            "Q2"
                        } else if v <= c {
                            "Q3"
                        } else {
                            "Q4 slowest"
                        }
                        .to_string(),
                    ),
                    _ => None,
                },
            ];
            for (f, l) in lab.into_iter().enumerate() {
                if let Some(l) = l {
                    labels[f].insert(r, l);
                }
            }
        }
        lots.push(Lot {
            name,
            kind: lp.str_or("kind", "issue"),
            ds,
            bad,
            good,
            groups,
            labels,
        });
    }

    // bad vs good per lot and factor, BH within the lot
    let mut tests: Vec<Vec<Option<cand::Factor>>> = vec![];
    for l in &lots {
        let rows: Vec<usize> = l.bad.iter().chain(&l.good).copied().collect();
        let aff: Vec<bool> = (0..rows.len()).map(|i| i < l.bad.len()).collect();
        tests.push(
            FACTORS
                .iter()
                .enumerate()
                .map(|(f, (k, n))| {
                    let lab: Vec<Option<String>> =
                        rows.iter().map(|r| l.labels[f].get(r).cloned()).collect();
                    cand::factor(k, n, &lab, &aff)
                })
                .collect(),
        );
    }
    let qs: Vec<Vec<f64>> = tests
        .iter()
        .map(|t| {
            let p: Vec<f64> = t
                .iter()
                .map(|f| f.as_ref().map(|f| f.p).unwrap_or(f64::NAN))
                .collect();
            let present: Vec<f64> = p.iter().copied().filter(|v| v.is_finite()).collect();
            let q = stats::bh_adjust(&present);
            let mut it = q.into_iter();
            p.iter()
                .map(|v| {
                    if v.is_finite() {
                        it.next().unwrap_or(f64::NAN)
                    } else {
                        f64::NAN
                    }
                })
                .collect()
        })
        .collect();

    let share = |l: &Lot, f: usize, rows: &[usize]| -> IndexMap<String, usize> {
        let mut c = IndexMap::new();
        for r in rows {
            if let Some(v) = l.labels[f].get(r) {
                *c.entry(v.clone()).or_default() += 1;
            }
        }
        c
    };
    let issue: Vec<usize> = (0..lots.len())
        .filter(|&i| lots[i].kind != "clean")
        .collect();
    let clean: Vec<usize> = (0..lots.len())
        .filter(|&i| lots[i].kind == "clean")
        .collect();
    let factors: Vec<Value> = FACTORS
        .iter()
        .enumerate()
        .filter(|(f, _)| lots.iter().any(|l| !l.labels[*f].is_empty()))
        .map(|(f, (key, name))| {
            let mut levels: BTreeSet<String> = BTreeSet::new();
            for l in &lots {
                levels.extend(l.labels[f].values().cloned());
            }
            let sig = |i: usize| qs[i][f] < 0.05;
            let within = !issue.is_empty() && issue.iter().all(|&i| sig(i)) && clean.iter().all(|&i| !sig(i));
            // across lots: one level holds most bad parts of every issue lot
            let frac = |c: &IndexMap<String, usize>, lv: &str| {
                let n: usize = c.values().sum();
                if n == 0 { f64::NAN } else { *c.get(lv).unwrap_or(&0) as f64 / n as f64 }
            };
            let across = levels.iter().find_map(|lv| {
                let bad_share = issue
                    .iter()
                    .map(|&i| frac(&share(&lots[i], f, &lots[i].bad), lv))
                    .fold(f64::INFINITY, f64::min);
                let clean_share = clean
                    .iter()
                    .map(|&i| {
                        let l = &lots[i];
                        let all: Vec<usize> = l.bad.iter().chain(&l.good).copied().collect();
                        frac(&share(l, f, &all), lv)
                    })
                    .fold(0.0, f64::max);
                (issue.len() >= 2 && !clean.is_empty() && bad_share >= 0.5 && clean_share <= bad_share / 2.0)
                    .then(|| json!({"level": lv, "issue_share": bad_share, "clean_share": clean_share}))
            });
            json!({
                "key": key, "name": name, "levels": levels,
                "flag_within": within, "flag_across": across,
                "lots": lots.iter().enumerate().map(|(i, l)| {
                    let t = tests[i][f].as_ref();
                    let mut groups = vec![("bad".to_string(), l.bad.clone()), ("good".to_string(), l.good.clone())];
                    groups.extend(l.groups.iter().cloned());
                    json!({
                        "lot": l.name, "kind": l.kind,
                        "test": t.map(|t| json!({"p": n(t.p), "q": n(qs[i][f]), "cramers_v": n(t.cramers_v),
                            "top": t.levels[0].level, "top_rate": n(t.levels[0].rate), "top_lift": n(t.levels[0].lift), "levels": t.levels})),
                        "groups": groups.iter().map(|(g, rows)| json!({"label": g, "n": rows.len(), "counts": share(l, f, rows)})).collect::<Vec<_>>(),
                    })
                }).collect::<Vec<_>>(),
            })
        })
        .collect();

    // top marker tests of each lot, side by side
    let opt = MarkerOptions {
        forest: pp.bool("forest", false),
        trees: pp.usize("trees", 60),
        exclude,
        ..Default::default()
    };
    let mut scores: IndexMap<String, Vec<Option<(f64, String)>>> = IndexMap::new();
    let mut ranked = vec![];
    for (i, l) in lots.iter().enumerate() {
        if l.bad.len() < 5 || l.good.len() < 10 {
            ranked.push(false);
            continue;
        }
        ranked.push(true);
        let rep = markers::find(
            &Group {
                ds: &l.ds,
                rows: l.good.clone(),
            },
            &Group {
                ds: &l.ds,
                rows: l.bad.clone(),
            },
            None,
            &opt,
        );
        for r in rep.rows {
            scores
                .entry(r.key.clone())
                .or_insert_with(|| vec![None; lots.len()])[i] = Some((r.score, r.why));
        }
    }
    let common = |v: &[Option<(f64, String)>]| {
        issue
            .iter()
            .filter(|&&i| ranked[i])
            .map(|&i| v[i].as_ref().map(|s| s.0).unwrap_or(0.0))
            .fold(f64::INFINITY, f64::min)
    };
    let mut mk: Vec<(&String, &Vec<Option<(f64, String)>>)> = scores.iter().collect();
    mk.sort_by(|a, b| common(b.1).total_cmp(&common(a.1)));
    Ok(json!({
        "lots": lots.iter().map(|l| json!({"name": l.name, "kind": l.kind, "n_bad": l.bad.len(), "n_good": l.good.len(),
            "groups": l.groups.iter().map(|g| json!({"label": g.0, "n": g.1.len()})).collect::<Vec<_>>()})).collect::<Vec<_>>(),
        "factors": factors,
        "markers": mk.iter().take(pp.usize("top", 30)).map(|(k, v)| json!({"key": k, "common": n(common(v)),
            "lots": v.iter().map(|s| s.as_ref().map(|(sc, w)| json!({"score": n(*sc), "why": w}))).collect::<Vec<_>>()})).collect::<Vec<_>>(),
        "marker_lots": ranked,
    }))
}

/// Investigation cases are kept as JSON files in the settings folder.
fn cases_dir() -> std::path::PathBuf {
    crate::desktop::config_dir().join("cases")
}

fn case_op(pp: &P, op: &str) -> Result<Value> {
    match op {
        "case_save" => {
            let case =
                pp.0.get("case")
                    .cloned()
                    .ok_or_else(|| anyhow!("nothing to save"))?;
            let name = set_name(case.get("name").and_then(|v| v.as_str()).unwrap_or("case"));
            let path = match pp.str("path") {
                p if p.trim().is_empty() => {
                    std::fs::create_dir_all(cases_dir())?;
                    cases_dir().join(format!("{name}.json"))
                }
                p => std::path::PathBuf::from(p),
            };
            std::fs::write(&path, serde_json::to_string_pretty(&case)?)
                .with_context(|| format!("write {}", path.display()))?;
            Ok(json!({"path": path.to_string_lossy()}))
        }
        "case_load" => {
            let path = pp.str("path");
            let text = std::fs::read_to_string(&path).with_context(|| format!("read {path}"))?;
            Ok(
                json!({"path": path, "case": serde_json::from_str::<Value>(&text).context("not a case file")?}),
            )
        }
        _ => {
            let mut v = vec![];
            if let Ok(rd) = std::fs::read_dir(cases_dir()) {
                for e in rd.flatten() {
                    let p = e.path();
                    if p.extension().is_some_and(|x| x == "json") {
                        let t = e
                            .metadata()
                            .and_then(|m| m.modified())
                            .ok()
                            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                            .map(|d| d.as_secs());
                        v.push(json!({"name": p.file_stem().map(|s| s.to_string_lossy()), "path": p.to_string_lossy(), "modified": t}));
                    }
                }
            }
            v.sort_by(|a, b| b["modified"].as_u64().cmp(&a["modified"].as_u64()));
            Ok(json!(v))
        }
    }
}

/// Signature check: threshold rules learned from part set `name` (bad
/// candidates) in dataset `a` against its normal parts, validated on a pool
/// of good reference parts (clean lots `clean`: [{ds, filter}], and the parts
/// a later file confirmed good, set `confirmed`), combined into one signature
/// and applied to the files in `screen` ([{ds, filter}]).
///
/// The normal parts are the other good parts of the population, or the
/// confirmed good ones when no clean lot is used and there are at least
/// `min_confirmed` of them. `exclude_set` (e.g. `marginal`) leaves its parts
/// out of every group.
fn signature_op(reg: &Registry, pp: &P) -> Result<Value> {
    use crate::analysis::markers::pair_test;
    use crate::analysis::signature::{self as sig, Pop};
    let id = pp.usize("a", 0);
    let (ds, _) = reg.get(id)?;
    let name = set_name(&pp.str("name"));
    let flags = reg.set(id, &name).ok_or_else(|| {
        anyhow!("part set ${name} does not exist yet: find the bad candidates first")
    })?;
    let mut pop = reg.rows(id, &ds, &pp.str("filter"))?;
    let mut notes: Vec<String> = vec![];
    let is = |f: &[bool], r: usize| f.get(r).copied().unwrap_or(false);
    let drop_set = pp.str("exclude_set");
    if !drop_set.trim().is_empty() {
        let dn = set_name(&drop_set);
        if let Some(f) = reg.set(id, &dn) {
            let before = pop.len();
            pop.retain(|&r| !is(&f, r));
            if pop.len() < before {
                notes.push(format!(
                    "{} ${dn} parts are left out of every group",
                    before - pop.len()
                ));
            }
        }
    }
    let (cand_rows, other_rows): (Vec<usize>, Vec<usize>) =
        pop.iter().partition(|&&r| is(&flags, r));
    let conf_name = set_name(&pp.str_or("confirmed", "confirmed_good"));
    let conf_rows: Vec<usize> = reg
        .set(id, &conf_name)
        .map(|f| other_rows.iter().copied().filter(|&r| is(&f, r)).collect())
        .unwrap_or_default();
    let min_n = pp.usize("min_n", 5);
    if cand_rows.len() < min_n {
        bail!(
            "only {} bad candidates in the population; need at least {min_n}",
            cand_rows.len()
        );
    }
    let by = pp.str_or("match_by", "key");
    // other files: (dataset id, filter) → loaded population
    let side = |k: &str| -> Result<Vec<(Arc<Dataset>, Vec<usize>, String)>> {
        let mut v = vec![];
        for (i, s) in
            pp.0.get(k)
                .and_then(|x| x.as_array())
                .cloned()
                .unwrap_or_default()
                .iter()
                .enumerate()
        {
            let sp = P(s);
            let d = sp.usize("ds", usize::MAX);
            let (dd, _) = reg.get(d).with_context(|| format!("{k} file {}", i + 1))?;
            let rows = reg.rows(d, &dd, &sp.str("filter"))?;
            v.push((dd, rows, sp.str_or("label", &reg.name(d))));
        }
        Ok(v)
    };
    let use_clean = pp.bool("use_clean", true);
    let use_conf = pp.bool("use_confirmed", true);
    let min_conf = pp.usize("min_confirmed", 30);
    let clean_in = if use_clean { side("clean")? } else { vec![] };
    let screen_in = side("screen")?;
    // stayed good on retest: good in a run 1 (`filter`, default pass) and
    // passing again on its 100% retest (`b`), matched by `key`; run-1 values
    let mut stayed_in: Vec<(Arc<Dataset>, Vec<usize>, String)> = vec![];
    if pp.bool("use_stayed", true) {
        let items =
            pp.0.get("stayed")
                .and_then(|x| x.as_array())
                .cloned()
                .unwrap_or_default();
        for (i, s) in items.iter().enumerate() {
            use data::matching::{match_parts, KeySpec};
            let sp = P(s);
            let (a, b) = (sp.usize("a", usize::MAX), sp.usize("b", usize::MAX));
            if a == b {
                bail!(
                    "stayed-good source {}: pick a run 1 and a different retest file",
                    i + 1
                );
            }
            let (da, _) = reg
                .get(a)
                .with_context(|| format!("stayed-good source {}", i + 1))?;
            let (db, _) = reg
                .get(b)
                .with_context(|| format!("stayed-good source {}", i + 1))?;
            let ks = sp.str_or("key", "part_txt");
            let (ka, kb) = (KeySpec::parse(&ks, &da)?, KeySpec::parse(&ks, &db)?);
            let ra = reg.rows(a, &da, &sp.str_or("filter", "pass"))?;
            let rb = reg.rows(b, &db, "pass")?;
            let m = match_parts(&da, &ra, &ka, &db, &rb, &kb);
            let rows: Vec<usize> = ra.into_iter().filter(|&r| m.in_a[r]).collect();
            stayed_in.push((da, rows, format!("stayed good: {}", reg.name(a))));
        }
    }
    fn pop_of((d, r, n): &(Arc<Dataset>, Vec<usize>, String)) -> Pop<'_> {
        Pop {
            name: n.clone(),
            ds: d,
            rows: r.clone(),
        }
    }
    let cand = Pop {
        name: "bad candidates".into(),
        ds: &ds,
        rows: cand_rows,
    };
    // the normal parts the rules are learned against: without a clean lot,
    // the parts the later file confirmed good, if there are enough of them
    let conf_label = "confirmed good".to_string();
    let learn_conf = clean_in.is_empty() && stayed_in.is_empty() && conf_rows.len() >= min_conf;
    if clean_in.is_empty() && stayed_in.is_empty() && !learn_conf {
        notes.push(if conf_rows.is_empty() {
            format!("no ${conf_name} parts in the population (run Bad candidates to create them): rules are learned against all other good parts of lot A")
        } else {
            format!("only {} ${conf_name} parts (fewer than {min_conf}): rules are learned against all other good parts of lot A instead", conf_rows.len())
        });
    }
    let normal = if learn_conf {
        Pop {
            name: conf_label.clone(),
            ds: &ds,
            rows: conf_rows.clone(),
        }
    } else {
        Pop {
            name: "lot A normal".into(),
            ds: &ds,
            rows: other_rows,
        }
    };
    // the reference pool: good parts only, from each ticked source
    let mut sources: Vec<Pop> = clean_in.iter().map(pop_of).collect();
    let mut kinds: Vec<&str> = vec!["clean"; sources.len()];
    sources.extend(stayed_in.iter().map(pop_of));
    kinds.resize(sources.len(), "stayed_good");
    if use_conf && !conf_rows.is_empty() {
        sources.push(Pop {
            name: conf_label.clone(),
            ds: &ds,
            rows: conf_rows.clone(),
        });
        kinds.push("confirmed");
        notes.push(format!(
            "${conf_name} parts are also {} the rules are learned against, so their false-alarm rate is in-sample (optimistic)",
            if learn_conf { "the normal parts" } else { "among the normal parts" }
        ));
    }
    if sources.is_empty() {
        bail!(
            "the reference pool is empty: add a clean lot or a stayed-good lot, or tick the confirmed good parts (${conf_name}, created by Bad candidates)"
        );
    }
    let screens: Vec<Pop> = screen_in.iter().map(pop_of).collect();

    let exclude = parse_nums(&pp.str("exclude"))?;
    let tests: Vec<(usize, Vec<Option<usize>>)> = ds
        .numeric_tests()
        .into_iter()
        .filter(|&t| !exclude.contains(&ds.tests[t].num))
        .map(|t| {
            (
                t,
                sources
                    .iter()
                    .map(|c| pair_test(&ds, c.ds, t, &by))
                    .collect(),
            )
        })
        .collect();
    let rows = sig::rules(&cand, &normal, &sources, &tests, min_n);

    // the signature: kept rules added one at a time while they improve it
    let usable: Vec<sig::RuleRow> = rows
        .iter()
        .filter(|r| r.verdict == "keep" || r.verdict == "caution")
        .cloned()
        .collect();
    let pair_in = |rules: &[sig::RuleRow], p: &Pop| -> Vec<Option<usize>> {
        rules
            .iter()
            .map(|r| {
                ds.find_test(&r.key)
                    .and_then(|t| pair_test(&ds, p.ds, t, &by))
            })
            .collect()
    };
    let mat = |p: &Pop| sig::hit_matrix(p, &pair_in(&usable, p), &usable);
    let src_mats: Vec<(String, Vec<Vec<bool>>)> =
        sources.iter().map(|c| (c.name.clone(), mat(c))).collect();
    let signature = sig::select(
        usable.clone(),
        &mat(&cand),
        &mat(&normal),
        &src_mats,
        pp.usize("max_rules", 5),
    );
    let kept = signature.rules.clone();
    let in_a: Vec<Option<usize>> = kept.iter().map(|r| ds.find_test(&r.key)).collect();
    let paired = |p: &Pop| pair_in(&kept, p);
    let h_src: Vec<(String, Vec<usize>)> = sources
        .iter()
        .map(|c| (c.name.clone(), sig::hits(c, &paired(c), &kept)))
        .collect();
    let m = signature.m;
    let count = |h: &[usize]| h.iter().filter(|&&x| x >= m).count();
    let (ref_k, ref_n) = h_src
        .iter()
        .fold((0, 0), |a, h| (a.0 + count(&h.1), a.1 + h.1.len()));
    // lot A as a whole: candidates and every other part of the population
    let h_pop = sig::hits(
        &Pop {
            name: String::new(),
            ds: &ds,
            rows: pop.clone(),
        },
        &in_a,
        &kept,
    );
    let (a_k, a_n) = (count(&h_pop), h_pop.len());

    // screening
    let screen: Vec<Value> = screens
        .iter()
        .map(|p| {
            let t = paired(p);
            let h = sig::hits(p, &t, &kept);
            let k = count(&h);
            let rate = if h.is_empty() { f64::NAN } else { k as f64 / h.len() as f64 };
            let p_ref = sig::fisher_greater(k, h.len(), ref_k, ref_n);
            let rate_a = a_k as f64 / a_n.max(1) as f64;
            let verdict = if kept.is_empty() {
                "no signature"
            } else if p_ref < 0.01 && rate >= 0.5 * rate_a {
                "resembles lot A"
            } else if p_ref < 0.01 {
                "elevated"
            } else {
                "like the reference"
            };
            let per_rule: Vec<Value> = kept
                .iter()
                .zip(&t)
                .map(|(r, t)| {
                    let v = t.map(|t| p.ds.values_of(t, Some(&p.rows))).unwrap_or_default();
                    json!({"key": r.key, "rate": n(sig::rate(&r.rule, &v)), "n": v.len()})
                })
                .collect();
            let parts: Vec<Value> = p
                .rows
                .iter()
                .zip(&h)
                .filter(|(_, &c)| c >= m && m > 0 && !kept.is_empty())
                .take(2000)
                .map(|(&r, &c)| {
                    let q = &p.ds.parts[r];
                    let which: Vec<&str> = kept
                        .iter()
                        .zip(&t)
                        .filter(|(rr, t)| t.map(|t| rr.rule.hit(p.ds.value(t, r))).unwrap_or(false))
                        .map(|(rr, _)| rr.key.as_str())
                        .collect();
                    json!({"row": r, "part_id": q.part_id, "part_txt": q.part_txt, "wafer": p.ds.wafer_name(q), "x": q.x, "y": q.y,
                        "site": q.site, "hbin": q.hbin, "n_rules": c, "rules": which.join(", ")})
                })
                .collect();
            json!({"name": p.name, "n": h.len(), "hits": k, "rate": n(rate), "p_ref": n(p_ref), "verdict": verdict,
                "per_rule": per_rule, "parts": parts})
        })
        .collect();

    // histograms of the leading rules over every population, with the cut
    let range = viz::RangeMode::parse(&pp.str("range"));
    let top_charts = pp.usize("top_charts", 8);
    let figures: Vec<Value> = rows
        .iter()
        .enumerate()
        .filter(|(i, r)| *i < top_charts || r.verdict == "keep" || r.verdict == "caution")
        .take(30)
        .filter_map(|(_, r)| {
            let t = ds.find_test(&r.key)?;
            let mut g = vec![
                (normal.name.clone(), ds.values_of(t, Some(&normal.rows))),
                ("bad candidates".to_string(), ds.values_of(t, Some(&cand.rows))),
            ];
            // each population once (a clean lot may also be screened)
            for p in sources.iter().chain(&screens) {
                if g.len() >= 6 || g.iter().any(|x| x.0 == p.name) {
                    continue;
                }
                if let Some(t2) = pair_test(&ds, p.ds, t, &by) {
                    g.push((p.name.clone(), p.ds.values_of(t2, Some(&p.rows))));
                }
            }
            let mut f = viz::histogram_fine(&ds.tests[t], &g, None, true, range);
            let cut = |x: f64| json!({"type": "line", "xref": "x", "yref": "paper", "x0": x, "x1": x, "y0": 0, "y1": 1, "line": {"color": "@warn", "width": 2, "dash": "dot"}});
            if let Some(sh) = f["layout"]["shapes"].as_array_mut() {
                sh.push(cut(r.rule.a));
                if r.rule.kind == sig::Kind::Outside {
                    sh.push(cut(r.rule.b));
                }
            }
            f["layout"]["title"]["text"] = json!(format!("{} · rule {} · {}", ds.tests[t].label(), r.text, r.verdict));
            Some(json!({"key": r.key, "figure": f}))
        })
        .collect();

    Ok(json!({
        "name": name, "n_cand": cand.rows.len(), "n_normal": normal.rows.len(), "normal": normal.name,
        "n_confirmed": conf_rows.len(), "confirmed": conf_name, "notes": notes,
        "sources": sources.iter().zip(&kinds).map(|(c, k)| json!({"name": c.name, "n": c.rows.len(), "kind": k})).collect::<Vec<_>>(),
        "n_pool": sources.iter().map(|c| c.rows.len()).sum::<usize>(),
        "rows": rows, "signature": signature,
        "rate_a": n(a_k as f64 / a_n.max(1) as f64), "rate_ref": n(ref_k as f64 / ref_n.max(1) as f64),
        "screen": screen, "figures": figures,
    }))
}

/// Dataset ids from a JSON array of numbers (or numeric strings).
fn id_list(v: Option<&Value>) -> Vec<usize> {
    v.and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_u64().or_else(|| x.as_str()?.trim().parse().ok()))
                .map(|x| x as usize)
                .collect()
        })
        .unwrap_or_default()
}

/// Open files of one lot and test step: the same lot (MIR LOT_ID), test
/// code and flow (TEST_COD, FLOW_ID), loaded from single files, in test-time
/// order, with the physical parts any two of them share (by wafer + X/Y when
/// every file has die coordinates, else by part text). Pieces of one test
/// share none; files that do (retests) can still be merged.
fn merge_candidates_op(reg: &Registry) -> Result<Value> {
    use data::matching::KeySpec;
    use std::collections::HashSet;
    struct F {
        id: usize,
        name: String,
        ds: Arc<Dataset>,
    }
    let mut groups: IndexMap<(String, String, String), Vec<F>> = IndexMap::new();
    {
        let v = reg.items.read().unwrap();
        for (id, e) in v.iter().enumerate() {
            let Some(e) = e else { continue };
            if e.paths.len() != 1 || e.ds.sources.len() != 1 {
                continue;
            }
            let s = &e.ds.sources[0];
            let k = (
                s.lot_id.trim().to_string(),
                s.test_cod.trim().to_string(),
                s.flow_id.trim().to_string(),
            );
            groups.entry(k).or_default().push(F {
                id,
                name: e.name.clone(),
                ds: e.ds.clone(),
            });
        }
    }
    // the identity every file of the group has for most parts
    let keys = |ds: &Dataset, spec: &KeySpec| -> Option<HashSet<String>> {
        let k: HashSet<String> = (0..ds.n_parts()).filter_map(|r| spec.key(ds, r)).collect();
        (k.len() * 2 >= ds.n_parts().max(1)).then_some(k)
    };
    let mut out = vec![];
    for ((lot, step, flow), fs) in groups.into_iter().filter(|(_, f)| f.len() > 1) {
        let mut by = "";
        let mut sets = vec![];
        for (name, spec) in [
            ("wafer + X/Y", KeySpec::WaferXy),
            ("part text", KeySpec::PartTxt),
        ] {
            let s: Option<Vec<HashSet<String>>> = fs.iter().map(|f| keys(&f.ds, &spec)).collect();
            if let Some(s) = s {
                by = name;
                sets = s;
                break;
            }
        }
        let file_json = |f: &F| {
            let s = &f.ds.sources[0];
            json!({"id": f.id, "name": f.name, "n_parts": f.ds.n_parts(), "wafers": f.ds.wafers, "start_t": s.start_t,
                "job": s.job_nam, "tester": s.node_nam, "sublot": s.sublot_id})
        };
        let mut order: Vec<usize> = (0..fs.len()).collect();
        order.sort_by_key(|&i| fs[i].ds.sources[0].start_t);
        // files sharing parts (e.g. a retest) are still offered: the page
        // says so and the user decides
        let mut pairs = vec![];
        let mut shared = 0usize;
        if !by.is_empty() {
            for (k, &i) in order.iter().enumerate() {
                for &j in &order[k + 1..] {
                    let n = sets[i].intersection(&sets[j]).count();
                    if n > 0 {
                        shared += n;
                        pairs.push(json!({"a": fs[i].id, "b": fs[j].id, "shared": n}));
                    }
                }
            }
        }
        out.push(json!({"lot": lot, "step": step, "flow": flow, "by": by, "shared": shared, "mergeable": true, "pairs": pairs,
            "files": order.iter().map(|&i| file_json(&fs[i])).collect::<Vec<_>>()}));
    }
    Ok(json!(out))
}
