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
            let s = reg.session.settings.lock().unwrap();
            Ok(json!(s.recent.iter().map(|r| {
                let m = std::fs::metadata(&r.path).ok();
                json!({"path": r.path, "name": data::short_name(&r.path), "exists": m.is_some(), "size": m.map(|m| m.len()).unwrap_or(0), "opened": r.opened})
            }).collect::<Vec<_>>()))
        }
        "forget_recent" => {
            let path = pp.str("path");
            let mut s = reg.session.settings.lock().unwrap();
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
                let ds = data::load_many(&paths, &opt)?;
                ids.push(reg.add(format!("merged: {}", ds.name), paths.clone(), ds));
                ok_paths = paths.clone();
            } else {
                use rayon::prelude::*;
                let results: Vec<(String, Result<Dataset>)> = paths
                    .par_iter()
                    .map(|p| (p.clone(), data::load(p, &opt)))
                    .collect();
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
            Ok(json!({"ids": ids, "errors": errors}))
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
        "match_dual" => match_dual_op(reg, &pp),
        "trace" => trace_op(reg, &pp),
        "markers" => markers_op(reg, &pp),
        "candidates" => candidates_op(reg, &pp),
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
                "hist": viz::histogram_r(info, &viz::values_groups(&ds, t, &g), pp.opt_usize("bins"), pp.bool("density", g.len() > 1), range),
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
            json!({"hist": viz::histogram_r(info, &g, None, true, range), "cdf": cdf, "box": bx})
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
    let mut hist_a = viz::histogram_r(ia, &vals(&da, ta, true), None, groups.len() > 1, mode);
    let mut hist_b = viz::histogram_r(ib, &vals(&db, tb, false), None, groups.len() > 1, mode);
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
    let mut delta = viz::histogram_r(&dinfo, &diffs, None, groups.len() > 1, viz::RangeMode::Data);
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
    let top: Vec<Value> = rep
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
            let mut f = viz::histogram_r(&da.tests[ta], &g, None, true, range);
            f["layout"]["title"]["text"] = json!(format!(
                "{} · {} (score {:.2})",
                da.tests[ta].label(),
                m.why,
                m.score
            ));
            Some(f)
        })
        .collect();
    let bars: Vec<&markers::MarkerRow> = rep.rows.iter().take(20).collect();
    let why_color = |w: &str| match w {
        "separation" => "@s1",
        "tails" => "@s2",
        "drift" => "@s3",
        "correlation break" => "@s7",
        "residual" => "@s4",
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
    let mut sdc = viz::histogram_r(
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
        "figures": {"scores": score_fig, "top": top, "sdc": sdc},
    }))
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
