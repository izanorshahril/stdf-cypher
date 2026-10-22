//! Plotly figure builders. Every function returns a Plotly `{data, layout}`
//! JSON object. Colors are written as role tokens (`@s1`..`@s8`, `@pass`,
//! `@fail`, `@limit`, `@pat`, `@muted`, `@seq`, `@div`) that the front-end
//! resolves for the active light/dark theme.

use crate::analysis::anomaly::AnomalyReport;
use crate::analysis::anova::AnovaRow;
use crate::analysis::compare::CompareReport;
use crate::analysis::correlation::{CorrMatrix, TargetCorr, UnitCorrTest};
use crate::analysis::stats;
use crate::analysis::summary::{BinCount, GroupYield};
use crate::data::{Dataset, TestInfo};
use crate::ml::forest::ForestReport;
use crate::ml::PcaReport;
use serde_json::{json, Value};

pub const MAX_SERIES: usize = 8;
/// categorical slots used for failing bins on maps (green-ish slots are reserved for pass)
const FAIL_SLOTS: [&str; 6] = ["@s2", "@s1", "@s5", "@s4", "@s7", "@s8"];
const MAX_POINTS: usize = 30_000;

pub fn slot(i: usize) -> String {
    format!("@s{}", (i % MAX_SERIES) + 1)
}

/// Series color: semantic colors for good/bad and alarm groups, palette slots otherwise.
pub fn series_color(name: &str, i: usize) -> String {
    match name {
        "good" => "@pass".into(),
        "bad" => "@fail".into(),
        "alarm" => "@warn".into(),
        "no alarm" => "@s1".into(),
        "reference" => "@s1".into(),
        "affected" | "bad candidates" => "@fail".into(),
        "lot A normal" => "@s1".into(),
        "confirmed good" => "@s3".into(),
        // part sets: "$name" and "not $name"
        n if n.starts_with("not $") => "@s1".into(),
        n if n.starts_with('$') => "@fail".into(),
        _ => slot(i),
    }
}

/// How a value axis is scaled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RangeMode {
    /// data range, stretched to a limit only when it is close to the data
    Auto,
    /// test limits (data range on a side without a limit)
    Limits,
    /// data range only
    Data,
    /// data and limits together, ignoring a limit that is far from the data
    Both,
}

impl RangeMode {
    pub fn parse(s: &str) -> RangeMode {
        match s {
            "limits" => RangeMode::Limits,
            "data" | "result" | "results" => RangeMode::Data,
            "both" => RangeMode::Both,
            _ => RangeMode::Auto,
        }
    }
}

/// Unpadded axis range for a test's values under `mode`.
pub fn value_range(info: &TestInfo, vals: &[f64], mode: RangeMode) -> Option<(f64, f64)> {
    let (dmin, dmax) = vals
        .iter()
        .filter(|v| v.is_finite())
        .fold((f64::INFINITY, f64::NEG_INFINITY), |a, &v| {
            (a.0.min(v), a.1.max(v))
        });
    let have = dmin <= dmax;
    let (lo, hi) = match mode {
        RangeMode::Data => (dmin, dmax),
        RangeMode::Limits => (info.lo.unwrap_or(dmin), info.hi.unwrap_or(dmax)),
        RangeMode::Auto | RangeMode::Both => {
            if !have {
                (info.lo.unwrap_or(f64::NAN), info.hi.unwrap_or(f64::NAN))
            } else {
                let span = (dmax - dmin)
                    .max(dmax.abs().max(dmin.abs()) * 1e-6)
                    .max(1e-12);
                let reach = if mode == RangeMode::Auto { 0.5 } else { 10.0 };
                let lo = info
                    .lo
                    .filter(|&l| l < dmin && dmin - l < span * reach)
                    .unwrap_or(dmin);
                let hi = info
                    .hi
                    .filter(|&h| h > dmax && h - dmax < span * reach)
                    .unwrap_or(dmax);
                (lo.min(dmin), hi.max(dmax))
            }
        }
    };
    if !(lo.is_finite() && hi.is_finite()) || lo > hi {
        return have.then_some((dmin, dmax));
    }
    Some((lo, hi))
}

/// Pad a range by 4% per side (or a small absolute amount for a single value).
pub fn padded((lo, hi): (f64, f64)) -> (f64, f64) {
    let pad = if hi > lo {
        (hi - lo) * 0.04
    } else {
        (lo.abs() * 0.05).max(1e-9)
    };
    (lo - pad, hi + pad)
}

/// Fix a figure's x or y axis to the range of `vals` under `mode`.
pub fn set_range(fig: &mut Value, axis: &str, info: &TestInfo, vals: &[f64], mode: RangeMode) {
    if mode == RangeMode::Auto {
        return;
    }
    if let Some(r) = value_range(info, vals, mode) {
        let (a, b) = padded(r);
        fig["layout"][axis]["range"] = json!([a, b]);
        fig["layout"][axis]["autorange"] = json!(false);
    }
}

fn num(v: f64) -> Value {
    if v.is_finite() {
        json!(v)
    } else {
        Value::Null
    }
}

/// Fold groups beyond the palette size into "Other".
pub fn fold_groups(mut g: Vec<(String, Vec<usize>)>) -> Vec<(String, Vec<usize>)> {
    if g.len() <= MAX_SERIES {
        return g;
    }
    g.sort_by(|a, b| b.1.len().cmp(&a.1.len()));
    let rest: Vec<usize> = g.drain(MAX_SERIES - 1..).flat_map(|x| x.1).collect();
    g.push(("Other".into(), rest));
    g
}

fn limit_shapes(lo: Option<f64>, hi: Option<f64>, vertical: bool) -> (Vec<Value>, Vec<Value>) {
    let mut shapes = vec![];
    let mut ann = vec![];
    for (v, name) in [(lo, "LO"), (hi, "HI")] {
        if let Some(v) = v {
            if vertical {
                shapes.push(json!({"type":"line","xref":"x","yref":"paper","x0":v,"x1":v,"y0":0,"y1":1,"line":{"color":"@limit","width":1.5,"dash":"dash"}}));
                ann.push(json!({"xref":"x","yref":"paper","x":v,"y":1,"text":format!("{name} {}", fmt(v)),"showarrow":false,"yanchor":"bottom","font":{"size":11,"color":"@muted"}}));
            } else {
                shapes.push(json!({"type":"line","xref":"paper","yref":"y","x0":0,"x1":1,"y0":v,"y1":v,"line":{"color":"@limit","width":1.5,"dash":"dash"}}));
                ann.push(json!({"xref":"paper","yref":"y","x":1,"y":v,"text":format!("{name} {}", fmt(v)),"showarrow":false,"xanchor":"right","yanchor":"bottom","font":{"size":11,"color":"@muted"}}));
            }
        }
    }
    (shapes, ann)
}

pub fn fmt(v: f64) -> String {
    if !v.is_finite() {
        return "–".into();
    }
    let a = v.abs();
    if a != 0.0 && !(1e-3..1e6).contains(&a) {
        format!("{v:.3e}")
    } else {
        format!("{}", (v * 1e4).round() / 1e4)
    }
}

fn base_layout(title: &str, x: &str, y: &str) -> Value {
    json!({
        "title": {"text": title, "x": 0.01, "xanchor": "left", "font": {"size": 15}},
        "xaxis": {"title": {"text": x}, "zeroline": false},
        "yaxis": {"title": {"text": y}, "zeroline": false},
        "margin": {"l": 60, "r": 20, "t": 64, "b": 50},
        "legend": {"orientation": "h", "x": 1, "xanchor": "right", "y": 1.0, "yanchor": "bottom"},
        "hovermode": "closest"
    })
}

/// Overlaid histograms of one test, one trace per group, with limit lines.
pub fn histogram(
    ds: &Dataset,
    t: usize,
    groups: &[(String, Vec<usize>)],
    nbins: Option<usize>,
    density: bool,
) -> Value {
    histogram_v(&ds.tests[t], &values_groups(ds, t, groups), nbins, density)
}

/// Collect a test's finite values per group.
pub fn values_groups(
    ds: &Dataset,
    t: usize,
    groups: &[(String, Vec<usize>)],
) -> Vec<(String, Vec<f64>)> {
    groups
        .iter()
        .map(|(g, r)| (g.clone(), ds.values_of(t, Some(r))))
        .collect()
}

pub fn histogram_v(
    info: &TestInfo,
    groups: &[(String, Vec<f64>)],
    nbins: Option<usize>,
    density: bool,
) -> Value {
    histogram_r(info, groups, nbins, density, RangeMode::Auto)
}

/// Histogram binned over the value range chosen by `mode`; values outside
/// that range are left out and counted in the title.
pub fn histogram_r(
    info: &TestInfo,
    groups: &[(String, Vec<f64>)],
    nbins: Option<usize>,
    density: bool,
    mode: RangeMode,
) -> Value {
    histogram_rf(info, groups, nbins, density, mode, 1)
}

/// Bins per default bin the dashboard gets, so its bin-size slider can
/// merge them back (coarser) or show them (finer) without a round trip.
pub const FINE_BINS: usize = 4;

/// [`histogram_r`] for the dashboard: with automatic binning, each bin is
/// cut into `FINE_BINS` and the traces carry `_fine` for the slider.
pub fn histogram_fine(
    info: &TestInfo,
    groups: &[(String, Vec<f64>)],
    nbins: Option<usize>,
    density: bool,
    mode: RangeMode,
) -> Value {
    histogram_rf(info, groups, nbins, density, mode, FINE_BINS)
}

fn histogram_rf(
    info: &TestInfo,
    groups: &[(String, Vec<f64>)],
    nbins: Option<usize>,
    density: bool,
    mode: RangeMode,
    fine: usize,
) -> Value {
    let all: Vec<f64> = groups.iter().flat_map(|(_, v)| v.iter().copied()).collect();
    let s = stats::sorted(&all);
    if s.is_empty() {
        return empty("No data for this test");
    }
    let Some((lo, hi)) = value_range(info, &s, mode) else {
        return empty("No data for this test");
    };
    let (lo, hi) = if hi > lo { (lo, hi) } else { padded((lo, hi)) };
    let outside = s.iter().filter(|&&v| v < lo || v > hi).count();
    let inside: Vec<f64> = s.iter().copied().filter(|&v| v >= lo && v <= hi).collect();
    let (edges, _) = stats::histogram(
        if inside.is_empty() { &s } else { &inside },
        nbins,
        Some((lo, hi)),
    );
    let nb0 = edges.len().saturating_sub(1).max(1);
    let fine = if nbins.is_none() && edges.len() > 1 {
        fine.max(1)
    } else {
        1
    };
    let nb = nb0 * fine;
    let w = if edges.len() > 1 {
        (edges[nb0] - edges[0]) / nb as f64
    } else {
        1.0
    };
    let mut data = vec![];
    for (i, (name, v)) in groups.iter().enumerate() {
        let (_, c) = stats::histogram(v, Some(nb), Some((lo, hi)));
        let tot = v.len().max(1) as f64;
        let ys: Vec<f64> = c
            .iter()
            .map(|&k| {
                if density {
                    k as f64 * 100.0 / tot
                } else {
                    k as f64
                }
            })
            .collect();
        let xs: Vec<f64> = (0..nb).map(|k| edges[0] + w * (k as f64 + 0.5)).collect();
        let d = stats::describe(v, info.lo, info.hi);
        let stat = format!(
            "mean {} σ {} Cpk {}<extra></extra>",
            fmt(d.mean),
            fmt(d.std),
            d.cpk.map(fmt).unwrap_or("–".into())
        );
        // raw counts ride along in customdata so the page can switch a
        // normalized histogram back to counts without a round trip
        data.push(json!({
            "type":"bar","name": format!("{name} (n={})", v.len()),"x": xs,"y": ys,"width": w * 0.98,
            "customdata": c, "meta": if density { "pct" } else { "count" }, "_fine": fine,
            "marker":{"color": series_color(name, i), "line":{"width":0}},
            "opacity": if groups.len() > 1 { 0.55 } else { 0.9 },
            "hovertemplate": if density {
                format!("{name}<br>%{{x:.4g}}<br>%{{y:.3g}}% of group (%{{customdata}} parts)<br>{stat}")
            } else {
                format!("{name}<br>%{{x:.4g}}<br>count: %{{y}}<br>{stat}")
            }
        }));
    }
    let (shapes, ann) = limit_shapes(info.lo, info.hi, true);
    let title = if outside > 0 {
        format!("{} ({outside} outside the axis range)", info.label())
    } else {
        info.label()
    };
    let mut l = base_layout(
        &title,
        &info.units,
        if density { "% of parts" } else { "parts" },
    );
    if mode != RangeMode::Auto {
        let (a, b) = padded((lo, hi));
        l["xaxis"]["range"] = json!([a, b]);
    }
    l["barmode"] = json!("overlay");
    l["bargap"] = json!(0);
    l["shapes"] = json!(shapes);
    l["annotations"] = json!(ann);
    json!({"data": data, "layout": l})
}

/// Box plot from precomputed quartiles, one box per group.
pub fn boxplot(ds: &Dataset, t: usize, groups: &[(String, Vec<usize>)]) -> Value {
    boxplot_v(&ds.tests[t], &values_groups(ds, t, groups))
}

pub fn boxplot_v(info: &TestInfo, groups: &[(String, Vec<f64>)]) -> Value {
    let mut data = vec![];
    for (i, (name, v)) in groups.iter().enumerate() {
        if v.is_empty() {
            continue;
        }
        let d = stats::describe(v, info.lo, info.hi);
        let iqr = d.q3 - d.q1;
        let lf = v
            .iter()
            .copied()
            .filter(|&x| x >= d.q1 - 1.5 * iqr)
            .fold(f64::INFINITY, f64::min);
        let uf = v
            .iter()
            .copied()
            .filter(|&x| x <= d.q3 + 1.5 * iqr)
            .fold(f64::NEG_INFINITY, f64::max);
        data.push(json!({
            "type":"box","name": name,"x":[name],"q1":[d.q1],"median":[d.median],"q3":[d.q3],
            "lowerfence":[lf],"upperfence":[uf],"mean":[d.mean],"sd":[d.std],
            "marker":{"color": series_color(name, i)},"line":{"width":1.5},"boxmean":"sd",
            "hovertemplate": format!("{name}: n={} mean {} median {}<extra></extra>", d.n, fmt(d.mean), fmt(d.median))
        }));
        // outliers beyond fences (capped)
        let out: Vec<f64> = v
            .iter()
            .copied()
            .filter(|&x| x < lf || x > uf)
            .take(500)
            .collect();
        if !out.is_empty() {
            data.push(json!({"type":"scatter","mode":"markers","x": vec![name.clone(); out.len()],"y": out,"showlegend":false,"name":format!("{name} outliers"),"marker":{"color": series_color(name, i),"size":5,"opacity":0.6},"hovertemplate":"%{y:.4g}<extra>outlier</extra>"}));
        }
    }
    let (shapes, ann) = limit_shapes(info.lo, info.hi, false);
    let mut l = base_layout(&info.label(), "", &info.units);
    l["shapes"] = json!(shapes);
    l["annotations"] = json!(ann);
    l["showlegend"] = json!(false);
    json!({"data": data, "layout": l})
}

/// Normalized cumulative distribution per group.
pub fn cdf(ds: &Dataset, t: usize, groups: &[(String, Vec<usize>)]) -> Value {
    cdf_v(&ds.tests[t], &values_groups(ds, t, groups))
}

pub fn cdf_v(info: &TestInfo, groups: &[(String, Vec<f64>)]) -> Value {
    let mut data = vec![];
    for (i, (name, v)) in groups.iter().enumerate() {
        let s = stats::sorted(v);
        if s.is_empty() {
            continue;
        }
        let step = (s.len() / 2000).max(1);
        let xs: Vec<f64> = s.iter().step_by(step).copied().collect();
        let ys: Vec<f64> = (0..s.len())
            .step_by(step)
            .map(|k| (k + 1) as f64 * 100.0 / s.len() as f64)
            .collect();
        data.push(json!({"type":"scatter","mode":"lines","name":name,"x":xs,"y":ys,"line":{"color":series_color(name, i),"width":2,"shape":"hv"},"hovertemplate":format!("{name}<br>%{{x:.4g}} → %{{y:.1f}}%<extra></extra>")}));
    }
    let (shapes, ann) = limit_shapes(info.lo, info.hi, true);
    let mut l = base_layout(
        &format!("CDF – {}", info.label()),
        &info.units,
        "cumulative %",
    );
    l["shapes"] = json!(shapes);
    l["annotations"] = json!(ann);
    json!({"data": data, "layout": l})
}

/// Value vs. test order, colored by group.
pub fn trend(ds: &Dataset, t: usize, groups: &[(String, Vec<usize>)]) -> Value {
    let info = &ds.tests[t];
    let total: usize = groups.iter().map(|g| g.1.len()).sum();
    let step = (total / MAX_POINTS).max(1);
    let mut data = vec![];
    for (i, (name, rows)) in groups.iter().enumerate() {
        let (xs, ys): (Vec<usize>, Vec<f64>) = rows
            .iter()
            .step_by(step)
            .filter_map(|&r| {
                let v = ds.value(t, r);
                v.is_finite().then_some((r, v))
            })
            .unzip();
        let ids: Vec<&str> = xs.iter().map(|&r| ds.parts[r].part_id.as_str()).collect();
        data.push(json!({"type":"scattergl","mode":"markers","name":name,"x":xs,"y":ys,"customdata":ids,"marker":{"color":series_color(name, i),"size":5,"opacity":0.75},"hovertemplate":format!("{name}<br>row %{{x}} part %{{customdata}}<br>%{{y:.4g}}<extra></extra>")}));
    }
    let (shapes, ann) = limit_shapes(info.lo, info.hi, false);
    let mut l = base_layout(
        &format!("Trend – {}", info.label()),
        "part (test order)",
        &info.units,
    );
    l["shapes"] = json!(shapes);
    l["annotations"] = json!(ann);
    if step > 1 {
        l["title"]["text"] = json!(format!("Trend – {} (every {step}th part)", info.label()));
    }
    json!({"data": data, "layout": l})
}

/// Scatter of two tests.
pub fn scatter(ds: &Dataset, tx: usize, ty: usize, groups: &[(String, Vec<usize>)]) -> Value {
    let (ix, iy) = (&ds.tests[tx], &ds.tests[ty]);
    let total: usize = groups.iter().map(|g| g.1.len()).sum();
    let step = (total / MAX_POINTS).max(1);
    let mut data = vec![];
    let mut allx = vec![];
    let mut ally = vec![];
    for (i, (name, rows)) in groups.iter().enumerate() {
        let mut xs = vec![];
        let mut ys = vec![];
        let mut ids = vec![];
        for &r in rows.iter().step_by(step) {
            let (a, b) = (ds.value(tx, r), ds.value(ty, r));
            if a.is_finite() && b.is_finite() {
                xs.push(a);
                ys.push(b);
                ids.push(ds.parts[r].part_id.clone());
            }
        }
        allx.extend(&xs);
        ally.extend(&ys);
        data.push(json!({"type":"scattergl","mode":"markers","name":name,"x":xs,"y":ys,"customdata":ids,"marker":{"color":series_color(name, i),"size":6,"opacity":0.7},"hovertemplate":format!("{name} part %{{customdata}}<br>x %{{x:.4g}}<br>y %{{y:.4g}}<extra></extra>")}));
    }
    let r = stats::pearson(&allx, &ally);
    let mut l = base_layout(
        &format!("{} vs {}  (r = {})", iy.name, ix.name, fmt(r)),
        &ix.label(),
        &iy.label(),
    );
    let (mut s1, mut a1) = limit_shapes(ix.lo, ix.hi, true);
    let (s2, a2) = limit_shapes(iy.lo, iy.hi, false);
    s1.extend(s2);
    a1.extend(a2);
    l["shapes"] = json!(s1);
    l["annotations"] = json!(a1);
    json!({"data": data, "layout": l})
}

/// Scatter of matched part pairs: test X of file A against test Y of file B.
pub fn pair_scatter(
    ix: &TestInfo,
    iy: &TestInfo,
    label_x: &str,
    label_y: &str,
    groups: &[(String, Vec<(f64, f64, String)>)],
) -> Value {
    let total: usize = groups.iter().map(|g| g.1.len()).sum();
    let step = (total / MAX_POINTS).max(1);
    let mut data = vec![];
    let (mut allx, mut ally) = (vec![], vec![]);
    for (i, (name, pts)) in groups.iter().enumerate() {
        let sel: Vec<&(f64, f64, String)> = pts.iter().step_by(step).collect();
        allx.extend(pts.iter().map(|p| p.0));
        ally.extend(pts.iter().map(|p| p.1));
        data.push(json!({"type":"scattergl","mode":"markers","name":format!("{name} ({})", pts.len()),
            "x": sel.iter().map(|p| p.0).collect::<Vec<_>>(), "y": sel.iter().map(|p| p.1).collect::<Vec<_>>(),
            "customdata": sel.iter().map(|p| p.2.clone()).collect::<Vec<_>>(),
            "marker":{"color":series_color(name, i),"size":6,"opacity":0.7},
            "hovertemplate":format!("{name}<br>key %{{customdata}}<br>A %{{x:.4g}}<br>B %{{y:.4g}}<extra></extra>")}));
    }
    if allx.is_empty() {
        return empty("No matched parts have both results");
    }
    if ix.units == iy.units {
        let lo = allx
            .iter()
            .chain(&ally)
            .copied()
            .fold(f64::INFINITY, f64::min);
        let hi = allx
            .iter()
            .chain(&ally)
            .copied()
            .fold(f64::NEG_INFINITY, f64::max);
        data.push(json!({"type":"scatter","mode":"lines","name":"y = x","x":[lo,hi],"y":[lo,hi],"line":{"color":"@muted","width":1.5,"dash":"dot"},"hoverinfo":"skip"}));
    }
    let r = stats::pearson(&allx, &ally);
    let mut l = base_layout(
        &format!(
            "Matched parts: B {} vs A {}  (r = {}, n = {})",
            iy.num,
            ix.num,
            fmt(r),
            allx.len()
        ),
        &format!("{label_x} · {}", ix.label()),
        &format!("{label_y} · {}", iy.label()),
    );
    let (mut s1, mut a1) = limit_shapes(ix.lo, ix.hi, true);
    let (s2, a2) = limit_shapes(iy.lo, iy.hi, false);
    s1.extend(s2);
    a1.extend(a2);
    l["shapes"] = json!(s1);
    l["annotations"] = json!(a1);
    json!({"data": data, "layout": l})
}

/// Wafer map. `mode` = "hbin" | "sbin" | "pass" | "test" (requires t).
pub fn wafer_map(ds: &Dataset, rows: &[usize], mode: &str, t: Option<usize>, title: &str) -> Value {
    wafer_map_opts(ds, rows, mode, t, title, true)
}

/// Wafer map drawn as a heatmap so dies scale with the panel. Later tests of
/// the same die (retests) overwrite earlier ones.
pub fn wafer_map_opts(
    ds: &Dataset,
    rows: &[usize],
    mode: &str,
    t: Option<usize>,
    title: &str,
    legend: bool,
) -> Value {
    let prr_xy = |r: usize| {
        let p = &ds.parts[r];
        (p.x != -32768 && p.y != -32768).then_some((p.x, p.y))
    };
    wafer_map_xy(ds, rows, mode, t, title, legend, &prr_xy)
}

/// Wafer map with die coordinates from `xy` (e.g. reconstructed from
/// datalogged X/Y tests); parts without coordinates are left out.
pub fn wafer_map_xy(
    ds: &Dataset,
    rows: &[usize],
    mode: &str,
    t: Option<usize>,
    title: &str,
    legend: bool,
    xy: &dyn Fn(usize) -> Option<(i16, i16)>,
) -> Value {
    let pts: Vec<(usize, i16, i16)> = rows
        .iter()
        .filter_map(|&r| xy(r).map(|(x, y)| (r, x, y)))
        .collect();
    if pts.is_empty() {
        return empty("No X/Y coordinates (final-test data?)");
    }
    let coord: std::collections::HashMap<usize, (i16, i16)> =
        pts.iter().map(|&(r, x, y)| (r, (x, y))).collect();
    let (minx, maxx, miny, maxy) = pts
        .iter()
        .fold((i16::MAX, i16::MIN, i16::MAX, i16::MIN), |a, &(_, x, y)| {
            (a.0.min(x), a.1.max(x), a.2.min(y), a.3.max(y))
        });
    let pts: Vec<usize> = pts.iter().map(|p| p.0).collect();
    let (w, hgt) = ((maxx - minx + 1) as usize, (maxy - miny + 1) as usize);
    let xs: Vec<i16> = (minx..=maxx).collect();
    let ys: Vec<i16> = (miny..=maxy).collect();
    let mut z: Vec<Vec<Value>> = vec![vec![Value::Null; w]; hgt];
    let mut text: Vec<Vec<String>> = vec![vec![String::new(); w]; hgt];
    let mut l = base_layout(title, "X", "Y");
    l["yaxis"]["autorange"] = json!("reversed");
    l["yaxis"]["scaleanchor"] = json!("x");
    l["xaxis"]["showgrid"] = json!(false);
    l["yaxis"]["showgrid"] = json!(false);
    l["xaxis"]["constrain"] = json!("domain");
    l["yaxis"]["constrain"] = json!("domain");
    let cell = |r: usize| {
        let (x, y) = coord[&r];
        ((y - miny) as usize, (x - minx) as usize)
    };
    let tip = |p: &crate::data::Part| {
        format!(
            "part {} · site {} · hbin {} · sbin {}",
            p.part_id, p.site, p.hbin, p.sbin
        )
    };
    match (mode, t) {
        ("test", Some(t)) => {
            let info = &ds.tests[t];
            let mut vals = vec![];
            for &r in &pts {
                let v = ds.value(t, r);
                let p = &ds.parts[r];
                let (yy, xx) = cell(r);
                if v.is_finite() {
                    z[yy][xx] = json!(v);
                    vals.push(v);
                }
                text[yy][xx] = tip(p);
            }
            let s = stats::sorted(&vals);
            let (cmin, cmax) = (
                stats::quantile_sorted(&s, 0.01),
                stats::quantile_sorted(&s, 0.99),
            );
            let data = vec![
                json!({"type":"heatmap","x":xs,"y":ys,"z":z,"text":text,"zmin":cmin,"zmax":cmax,"colorscale":"@seq","xgap":1,"ygap":1,
                "colorbar":{"title":{"text":info.units},"thickness":12},"hovertemplate":"(%{x},%{y}) %{text}<br>%{z:.4g}<extra></extra>"}),
            ];
            l["title"]["text"] = json!(format!("{title} – {}", info.label()));
            json!({"data": data, "layout": l})
        }
        _ => {
            let field = if mode == "sbin" {
                "sbin"
            } else if mode == "pass" {
                "pass"
            } else {
                "hbin"
            };
            let mut groups = ds.group_by(&pts, field);
            // stable colors: rank failing bins by frequency over the WHOLE dataset, so a bin
            // keeps its color on every wafer / selection
            let global_rank: std::collections::HashMap<String, usize> = {
                let all: Vec<usize> = (0..ds.n_parts())
                    .filter(|&r| !ds.parts[r].is_pass())
                    .collect();
                let mut g = ds.group_by(&all, field);
                g.sort_by(|a, b| b.1.len().cmp(&a.1.len()).then(a.0.cmp(&b.0)));
                g.into_iter()
                    .enumerate()
                    .map(|(i, (k, _))| (k, i))
                    .collect()
            };
            groups.sort_by_key(|(k, rs)| {
                (
                    !rs.first().map(|&r| ds.parts[r].is_pass()).unwrap_or(false),
                    global_rank.get(k).copied().unwrap_or(usize::MAX),
                )
            });
            // assign a color + category index per group; overflow folds into "Other"
            let mut cats: Vec<(String, String)> = vec![];
            let mut other_idx: Option<usize> = None;
            for (name, rs) in &groups {
                let passing = rs.first().map(|&r| ds.parts[r].is_pass()).unwrap_or(false);
                let label = match field {
                    "hbin" | "sbin" => {
                        let bn = ds.bin_name(field == "hbin", name.parse().unwrap_or(0));
                        if bn.is_empty() {
                            format!("{field} {name} ({})", rs.len())
                        } else {
                            format!("{field} {name} {bn} ({})", rs.len())
                        }
                    }
                    _ => format!("{name} ({})", rs.len()),
                };
                let (cat, color) = if passing {
                    (cats.len(), "@pass".to_string())
                } else if field == "pass" {
                    (cats.len(), "@fail".to_string())
                } else if let Some(&gi) = global_rank.get(name).filter(|&&gi| gi < FAIL_SLOTS.len())
                {
                    (cats.len(), FAIL_SLOTS[gi].to_string())
                } else {
                    let k = *other_idx.get_or_insert_with(|| {
                        cats.push(("Other".into(), "@muted".into()));
                        cats.len() - 1
                    });
                    for &r in rs {
                        let p = &ds.parts[r];
                        let (yy, xx) = cell(r);
                        z[yy][xx] = json!(k as f64 + 0.5);
                        text[yy][xx] = tip(p);
                    }
                    continue;
                };
                cats.push((label, color));
                for &r in rs {
                    let p = &ds.parts[r];
                    let (yy, xx) = cell(r);
                    z[yy][xx] = json!(cat as f64 + 0.5);
                    text[yy][xx] = tip(p);
                }
            }
            let k = cats.len().max(1) as f64;
            let mut cs: Vec<Value> = vec![];
            for (i, (_, c)) in cats.iter().enumerate() {
                cs.push(json!([i as f64 / k, c]));
                cs.push(json!([(i + 1) as f64 / k, c]));
            }
            let mut data = vec![
                json!({"type":"heatmap","x":xs,"y":ys,"z":z,"text":text,"zmin":0,"zmax":k,"colorscale":cs,"showscale":false,"xgap":1,"ygap":1,
                "hovertemplate":"(%{x},%{y}) %{text}<extra></extra>","showlegend":false}),
            ];
            // legend-only entries
            for (label, color) in &cats {
                data.push(json!({"type":"scatter","mode":"markers","x":[null],"y":[null],"name":label,"marker":{"symbol":"square","size":11,"color":color},"hoverinfo":"skip","showlegend":true}));
            }
            if legend {
                l["legend"] =
                    json!({"orientation":"v","x":1.02,"y":1,"xanchor":"left","yanchor":"top"});
            } else {
                l["legend"] = json!({"orientation":"h","x":0,"xanchor":"left","y":-0.12,"yanchor":"top","font":{"size":10}});
                l["xaxis"]["title"] = json!({"text": ""});
                l["margin"] = json!({"l": 40, "r": 10, "t": 40, "b": 10});
            }
            json!({"data": data, "layout": l})
        }
    }
}

fn wafer_trace(ds: &Dataset, rs: &[usize], label: &str, color: &str, size: f64) -> Value {
    let xs: Vec<i16> = rs.iter().map(|&r| ds.parts[r].x).collect();
    let ys: Vec<i16> = rs.iter().map(|&r| ds.parts[r].y).collect();
    let text: Vec<String> = rs
        .iter()
        .map(|&r| {
            let p = &ds.parts[r];
            format!(
                "part {} site {} hbin {} sbin {}",
                p.part_id, p.site, p.hbin, p.sbin
            )
        })
        .collect();
    json!({"type":"scattergl","mode":"markers","name":label,"x":xs,"y":ys,"text":text,"marker":{"symbol":"square","size":size,"color":color},"hovertemplate":"(%{x},%{y}) %{text}<extra></extra>"})
}

pub fn bin_pareto(bins: &[BinCount], hard: bool) -> Value {
    let xs: Vec<String> = bins
        .iter()
        .map(|b| {
            if b.name.is_empty() {
                format!("{}", b.bin)
            } else {
                format!("{} {}", b.bin, b.name)
            }
        })
        .collect();
    let colors: Vec<&str> = bins
        .iter()
        .map(|b| {
            if b.pf == "P" || (b.pf.trim().is_empty() && b.bin == 1) {
                "@pass"
            } else {
                "@fail"
            }
        })
        .collect();
    let data = vec![
        json!({"type":"bar","x":xs,"y":bins.iter().map(|b| b.count).collect::<Vec<_>>(),"name":"parts","marker":{"color":colors},
        "customdata": bins.iter().map(|b| b.pct).collect::<Vec<_>>(),
        "hovertemplate":"%{x}<br>%{y} parts (%{customdata:.2f}%)<extra></extra>",
        "text": bins.iter().map(|b| format!("{:.1}%", b.pct)).collect::<Vec<_>>(),"textposition":"outside","cliponaxis":false}),
    ];
    let mut l = base_layout(if hard { "Hard bins" } else { "Soft bins" }, "", "parts");
    l["xaxis"]["type"] = json!("category");
    l["showlegend"] = json!(false);
    json!({"data": data, "layout": l})
}

pub fn yield_bar(rows: &[GroupYield], title: &str, field: &str) -> Value {
    let xs: Vec<String> = rows.iter().map(|r| r.group.clone()).collect();
    let data = vec![
        json!({"type":"bar","x":xs,"y":rows.iter().map(|r| r.yield_pct).collect::<Vec<_>>(),"marker":{"color":"@s1"},
        "customdata": rows.iter().map(|r| vec![r.pass, r.n]).collect::<Vec<_>>(),
        "text": rows.iter().map(|r| format!("{:.1}%", r.yield_pct)).collect::<Vec<_>>(),"textposition":"outside","cliponaxis":false,
        "hovertemplate":format!("{field} %{{x}}<br>yield %{{y:.2f}}%<br>%{{customdata[0]}} / %{{customdata[1]}}<extra></extra>")}),
    ];
    let mut l = base_layout(title, field, "yield %");
    l["xaxis"]["type"] = json!("category");
    let minv = rows.iter().map(|r| r.yield_pct).fold(100.0, f64::min);
    l["yaxis"]["range"] = json!([(minv - 10.0).max(0.0).floor(), 102]);
    l["showlegend"] = json!(false);
    json!({"data": data, "layout": l})
}

pub fn corr_heatmap(m: &CorrMatrix) -> Value {
    let z: Vec<Vec<Value>> =
        m.r.iter()
            .map(|row| row.iter().map(|&v| num(v)).collect())
            .collect();
    let data = vec![
        json!({"type":"heatmap","z":z,"x":m.names,"y":m.names,"zmin":-1,"zmax":1,"colorscale":"@div","colorbar":{"title":{"text":"r"},"thickness":12},
        "hovertemplate":"%{y}<br>%{x}<br>r = %{z:.3f}<extra></extra>","xgap":1,"ygap":1}),
    ];
    let mut l = base_layout(
        &format!("{} correlation ({} tests)", m.method, m.names.len()),
        "",
        "",
    );
    l["xaxis"]["showticklabels"] = json!(m.names.len() <= 40);
    l["yaxis"]["showticklabels"] = json!(m.names.len() <= 40);
    l["yaxis"]["autorange"] = json!("reversed");
    l["margin"] = json!({"l": if m.names.len() <= 40 {170} else {30}, "r": 20, "t": 48, "b": if m.names.len() <= 40 {150} else {30}});
    json!({"data": data, "layout": l})
}

pub fn target_corr_bar(v: &[TargetCorr], title: &str, top: usize) -> Value {
    let v: Vec<&TargetCorr> = v.iter().take(top).collect();
    let data = vec![
        json!({"type":"bar","orientation":"h","y": v.iter().map(|t| t.key.clone()).collect::<Vec<_>>(),"x": v.iter().map(|t| t.r).collect::<Vec<_>>(),
        "marker":{"color": v.iter().map(|t| if t.r >= 0.0 {"@s1"} else {"@s2"}).collect::<Vec<_>>()},
        "customdata": v.iter().map(|t| vec![t.p, t.n as f64]).collect::<Vec<_>>(),
        "hovertemplate":"%{y}<br>r = %{x:.3f}<br>p = %{customdata[0]:.2e}, n = %{customdata[1]}<extra></extra>"}),
    ];
    let mut l = base_layout(title, "correlation r", "");
    l["yaxis"]["autorange"] = json!("reversed");
    l["xaxis"]["range"] = json!([-1, 1]);
    l["margin"]["l"] = json!(190);
    l["showlegend"] = json!(false);
    json!({"data": data, "layout": l})
}

pub fn unit_corr_scatter(t: &UnitCorrTest, label_a: &str, label_b: &str) -> Value {
    let step = (t.pairs.len() / MAX_POINTS).max(1);
    let xs: Vec<f64> = t.pairs.iter().step_by(step).map(|p| p.0).collect();
    let ys: Vec<f64> = t.pairs.iter().step_by(step).map(|p| p.1).collect();
    let lo = xs.iter().chain(&ys).copied().fold(f64::INFINITY, f64::min);
    let hi = xs
        .iter()
        .chain(&ys)
        .copied()
        .fold(f64::NEG_INFINITY, f64::max);
    let data = vec![
        json!({"type":"scattergl","mode":"markers","name":"parts","x":xs,"y":ys,"marker":{"color":"@s1","size":6,"opacity":0.7},"hovertemplate":format!("{label_a} %{{x:.4g}}<br>{label_b} %{{y:.4g}}<extra></extra>")}),
        json!({"type":"scatter","mode":"lines","name":"y = x","x":[lo,hi],"y":[lo,hi],"line":{"color":"@muted","width":1.5,"dash":"dot"},"hoverinfo":"skip"}),
        json!({"type":"scatter","mode":"lines","name":"fit","x":[lo,hi],"y":[t.intercept + t.slope*lo, t.intercept + t.slope*hi],"line":{"color":"@s2","width":2},"hoverinfo":"skip"}),
    ];
    let l = base_layout(
        &format!(
            "{} – {} (r = {}, offset = {})",
            t.key,
            t.status,
            fmt(t.r),
            fmt(t.mean_diff)
        ),
        &format!("{label_a} [{}]", t.units),
        &format!("{label_b} [{}]", t.units),
    );
    json!({"data": data, "layout": l})
}

/// Volcano plot: effect size vs significance for a comparison.
pub fn compare_volcano(c: &CompareReport) -> Value {
    let flags = ["OK", "SHIFT", "SPREAD", "SHIFT+SPREAD"];
    let mut data = vec![];
    for (i, f) in flags.iter().enumerate() {
        let pts: Vec<_> = c
            .tests
            .iter()
            .filter(|t| &t.flag == f && t.shift_sigma.is_finite() && t.q.is_finite())
            .collect();
        if pts.is_empty() {
            continue;
        }
        data.push(json!({"type":"scattergl","mode":"markers","name":f,
            "x": pts.iter().map(|t| t.shift_sigma).collect::<Vec<_>>(),
            "y": pts.iter().map(|t| -(t.q.max(1e-300)).log10()).collect::<Vec<_>>(),
            "text": pts.iter().map(|t| t.key.clone()).collect::<Vec<_>>(),
            "marker":{"color": if *f == "OK" {"@muted".to_string()} else {slot(i)},"size":8,"opacity":0.8,"line":{"width":1,"color":"@surface"}},
            "hovertemplate":"%{text}<br>shift %{x:.2f}σ<br>-log10 q %{y:.1f}<extra></extra>"}));
    }
    let l = base_layout(
        &format!("{} vs {} – per-test shift", c.label_b, c.label_a),
        "mean shift (pooled σ, B − A)",
        "−log10(q)",
    );
    json!({"data": data, "layout": l})
}

pub fn bin_compare(c: &CompareReport) -> Value {
    let xs: Vec<String> = c
        .hbins
        .iter()
        .map(|b| {
            if b.name.is_empty() {
                b.bin.to_string()
            } else {
                format!("{} {}", b.bin, b.name)
            }
        })
        .collect();
    let data = vec![
        json!({"type":"bar","name":c.label_a,"x":xs,"y":c.hbins.iter().map(|b| b.pct_a).collect::<Vec<_>>(),"marker":{"color":"@s1"},"hovertemplate":"%{x}: %{y:.2f}%<extra>A</extra>"}),
        json!({"type":"bar","name":c.label_b,"x":xs,"y":c.hbins.iter().map(|b| b.pct_b).collect::<Vec<_>>(),"marker":{"color":"@s2"},"hovertemplate":"%{x}: %{y:.2f}%<extra>B</extra>"}),
    ];
    let mut l = base_layout("Hard-bin distribution", "", "% of parts");
    l["barmode"] = json!("group");
    l["xaxis"]["type"] = json!("category");
    l["yaxis"]["type"] = json!("log");
    json!({"data": data, "layout": l})
}

pub fn anova_means(r: &AnovaRow, factor: &str) -> Value {
    let data = vec![
        json!({"type":"scatter","mode":"markers","x": r.groups.iter().map(|g| g.group.clone()).collect::<Vec<_>>(),
        "y": r.groups.iter().map(|g| num(g.mean)).collect::<Vec<_>>(),
        "error_y":{"type":"data","array": r.groups.iter().map(|g| num(g.std)).collect::<Vec<_>>(),"color":"@muted","thickness":1.5,"width":4},
        "customdata": r.groups.iter().map(|g| g.n).collect::<Vec<_>>(),
        "marker":{"color":"@s1","size":10},"hovertemplate":format!("{factor} %{{x}}<br>mean %{{y:.4g}} ± σ<br>n=%{{customdata}}<extra></extra>")}),
    ];
    let mut l = base_layout(
        &format!(
            "{} by {factor} (F = {}, p = {:.2e}, η² = {:.3})",
            r.key,
            fmt(r.f),
            r.p,
            r.eta2
        ),
        factor,
        &r.units,
    );
    l["xaxis"]["type"] = json!("category");
    l["showlegend"] = json!(false);
    json!({"data": data, "layout": l})
}

pub fn anova_effects(rows: &[AnovaRow], factor: &str, top: usize) -> Value {
    let v: Vec<&AnovaRow> = rows.iter().take(top).collect();
    let data = vec![
        json!({"type":"bar","orientation":"h","y": v.iter().map(|r| r.key.clone()).collect::<Vec<_>>(),"x": v.iter().map(|r| r.eta2).collect::<Vec<_>>(),
        "marker":{"color":"@s1"},"customdata": v.iter().map(|r| vec![num(r.f), num(r.p)]).collect::<Vec<_>>(),
        "hovertemplate":"%{y}<br>η² %{x:.3f}<br>F %{customdata[0]:.2f}, p %{customdata[1]:.2e}<extra></extra>"}),
    ];
    let mut l = base_layout(
        &format!("Effect of {factor} (η², top {})", v.len()),
        "η² (share of variance explained)",
        "",
    );
    l["yaxis"]["autorange"] = json!("reversed");
    l["margin"]["l"] = json!(190);
    l["showlegend"] = json!(false);
    json!({"data": data, "layout": l})
}

pub fn pca_scatter(ds: &Dataset, pr: &PcaReport, color_by: &str) -> Value {
    let rows = &pr.features.rows;
    let groups: Vec<(String, Vec<usize>)> = if color_by == "cluster" && pr.kmeans.is_some() {
        let km = pr.kmeans.as_ref().unwrap();
        (0..km.k)
            .map(|c| {
                (
                    format!("cluster {}", c + 1),
                    (0..rows.len()).filter(|&i| km.labels[i] == c).collect(),
                )
            })
            .collect()
    } else {
        let g = ds.group_by(
            rows,
            if color_by.is_empty() {
                "pass"
            } else {
                color_by
            },
        );
        let pos: std::collections::HashMap<usize, usize> =
            rows.iter().enumerate().map(|(i, &r)| (r, i)).collect();
        g.into_iter()
            .map(|(k, rs)| (k, rs.iter().map(|r| pos[r]).collect()))
            .collect()
    };
    let groups = fold_groups(groups);
    let step = (rows.len() / MAX_POINTS).max(1);
    let mut data = vec![];
    for (i, (name, idx)) in groups.iter().enumerate() {
        let sel: Vec<usize> = idx.iter().step_by(step).copied().collect();
        let color = if color_by == "pass" {
            if name == "pass" {
                "@pass".to_string()
            } else {
                "@fail".to_string()
            }
        } else {
            slot(i)
        };
        data.push(json!({"type":"scattergl","mode":"markers","name":format!("{name} ({})", idx.len()),
            "x": sel.iter().map(|&k| pr.scores[k].first().copied().unwrap_or(0.0)).collect::<Vec<_>>(),
            "y": sel.iter().map(|&k| pr.scores[k].get(1).copied().unwrap_or(0.0)).collect::<Vec<_>>(),
            "customdata": sel.iter().map(|&k| ds.parts[rows[k]].part_id.clone()).collect::<Vec<_>>(),
            "marker":{"color":color,"size":6,"opacity":0.7},"hovertemplate":"part %{customdata}<br>PC1 %{x:.2f}<br>PC2 %{y:.2f}<extra></extra>"}));
    }
    let ev = &pr.pca.explained_ratio;
    let l = base_layout(
        &format!("PCA – {} features", pr.features.p),
        &format!("PC1 ({:.1}%)", ev.first().unwrap_or(&0.0) * 100.0),
        &format!("PC2 ({:.1}%)", ev.get(1).unwrap_or(&0.0) * 100.0),
    );
    json!({"data": data, "layout": l})
}

pub fn pca_scree(pr: &PcaReport) -> Value {
    let ev = &pr.pca.explained_ratio;
    let xs: Vec<String> = (1..=ev.len()).map(|i| format!("PC{i}")).collect();
    let data = vec![
        json!({"type":"bar","name":"explained","x":xs,"y":ev.iter().map(|v| v*100.0).collect::<Vec<_>>(),"marker":{"color":"@s1"},"hovertemplate":"%{x}: %{y:.1f}%<extra></extra>"}),
    ];
    let mut l = base_layout("Variance explained", "", "% variance");
    l["showlegend"] = json!(false);
    json!({"data": data, "layout": l})
}

pub fn importance_bar(fr: &ForestReport, top: usize) -> Value {
    let v: Vec<&(String, f64)> = fr.importance.iter().take(top).collect();
    let data = vec![
        json!({"type":"bar","orientation":"h","y": v.iter().map(|x| x.0.clone()).collect::<Vec<_>>(),"x": v.iter().map(|x| x.1 * 100.0).collect::<Vec<_>>(),"marker":{"color":"@s1"},"hovertemplate":"%{y}<br>%{x:.1f}% of importance<extra></extra>"}),
    ];
    let mut l = base_layout(
        &format!("What drives {} (random forest, top {})", fr.target, v.len()),
        "importance %",
        "",
    );
    l["yaxis"]["autorange"] = json!("reversed");
    l["margin"]["l"] = json!(190);
    l["showlegend"] = json!(false);
    json!({"data": data, "layout": l})
}

pub fn confusion(fr: &ForestReport) -> Value {
    let z: Vec<Vec<usize>> = fr.confusion.clone();
    let data = vec![
        json!({"type":"heatmap","z":z,"x":fr.class_names,"y":fr.class_names,"colorscale":"@seq","showscale":false,"texttemplate":"%{z}","hovertemplate":"actual %{y}<br>predicted %{x}<br>%{z}<extra></extra>","xgap":2,"ygap":2}),
    ];
    let mut l = base_layout("Confusion matrix (hold-out)", "predicted", "actual");
    l["yaxis"]["autorange"] = json!("reversed");
    json!({"data": data, "layout": l})
}

pub fn pred_vs_actual(fr: &ForestReport) -> Value {
    let xs: Vec<f64> = fr.test_pred.iter().map(|p| p.0).collect();
    let ys: Vec<f64> = fr.test_pred.iter().map(|p| p.1).collect();
    let lo = xs.iter().chain(&ys).copied().fold(f64::INFINITY, f64::min);
    let hi = xs
        .iter()
        .chain(&ys)
        .copied()
        .fold(f64::NEG_INFINITY, f64::max);
    let data = vec![
        json!({"type":"scattergl","mode":"markers","name":"hold-out","x":xs,"y":ys,"marker":{"color":"@s1","size":6,"opacity":0.7},"hovertemplate":"actual %{x:.4g}<br>predicted %{y:.4g}<extra></extra>"}),
        json!({"type":"scatter","mode":"lines","name":"ideal","x":[lo,hi],"y":[lo,hi],"line":{"color":"@muted","dash":"dot","width":1.5},"hoverinfo":"skip"}),
    ];
    json!({"data": data, "layout": base_layout(&format!("Predicted vs actual – {} (R² = {})", fr.target, fmt(fr.r2.unwrap_or(f64::NAN))), "actual", "predicted")})
}

pub fn anomaly_scatter(ds: &Dataset, a: &AnomalyReport) -> Value {
    let flagged: std::collections::HashSet<usize> = a.parts.iter().map(|p| p.row).collect();
    let mut normal = (vec![], vec![], vec![]);
    let mut flag = (vec![], vec![], vec![]);
    let step = (a.rows.len() / MAX_POINTS).max(1);
    for (k, &r) in a.rows.iter().enumerate() {
        let is_f = flagged.contains(&r);
        if !is_f && k % step != 0 {
            continue;
        }
        let t = if is_f { &mut flag } else { &mut normal };
        t.0.push(r);
        t.1.push(a.scores[k]);
        t.2.push(ds.parts[r].part_id.clone());
    }
    let data = vec![
        json!({"type":"scattergl","mode":"markers","name":"normal","x":normal.0,"y":normal.1,"customdata":normal.2,"marker":{"color":"@muted","size":5,"opacity":0.5},"hovertemplate":"part %{customdata}<br>score %{y:.2f}<extra></extra>"}),
        json!({"type":"scattergl","mode":"markers","name":"flagged","x":flag.0,"y":flag.1,"customdata":flag.2,"marker":{"color":"@fail","size":9,"line":{"width":1.5,"color":"@surface"}},"hovertemplate":"part %{customdata}<br>score %{y:.2f}<extra>flagged</extra>"}),
    ];
    json!({"data": data, "layout": base_layout("Anomaly score by part", "part (test order)", "combined anomaly score")})
}

/// Wafer map highlighting flagged anomalous dies.
pub fn anomaly_wafer(ds: &Dataset, a: &AnomalyReport, wafer: u32) -> Value {
    let flagged: std::collections::HashSet<usize> = a.parts.iter().map(|p| p.row).collect();
    let rows: Vec<usize> = a
        .rows
        .iter()
        .copied()
        .filter(|&r| ds.parts[r].wafer == wafer && ds.parts[r].x != -32768)
        .collect();
    if rows.is_empty() {
        return empty("No wafer coordinates");
    }
    let (f, n): (Vec<usize>, Vec<usize>) = rows.iter().partition(|r| flagged.contains(r));
    let (np, nf): (Vec<usize>, Vec<usize>) = n.iter().partition(|&&r| ds.parts[r].is_pass());
    let size = 9.0;
    let mut l = base_layout(
        &format!(
            "Anomalies – wafer {}",
            ds.wafers.get(wafer as usize).cloned().unwrap_or_default()
        ),
        "X",
        "Y",
    );
    l["yaxis"]["autorange"] = json!("reversed");
    l["yaxis"]["scaleanchor"] = json!("x");
    let data = vec![
        wafer_trace(ds, &np, "pass", "@pass", size),
        wafer_trace(ds, &nf, "fail", "@muted", size),
        wafer_trace(ds, &f, "anomaly", "@fail", size + 3.0),
    ];
    json!({"data": data, "layout": l})
}

pub fn empty(msg: &str) -> Value {
    json!({"data": [], "layout": {"xaxis":{"visible":false},"yaxis":{"visible":false},"annotations":[{"text":msg,"showarrow":false,"font":{"size":14,"color":"@muted"},"xref":"paper","yref":"paper","x":0.5,"y":0.5}]}})
}
