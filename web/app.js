// stdf-cypher interactive dashboard
"use strict";
const { h, plot, fmt, fmtP, pct, kpi, chartBox, badge, table } = SC;

const S = { datasets: [], ds: 0, filter: "", view: "overview", testsCache: {}, scope: "all", co: { range: "both", dim: "2d", opacity: 0.6, hist: "pct" } };
try { Object.assign(S.co, JSON.parse(localStorage.getItem("sc-chart") || "{}")); S.scope = localStorage.getItem("sc-scope") || "all"; } catch (e) {}
// settings kept in this browser profile (Settings page); `sc-sort` predates it
// how parts can be identified, in the default order (Settings can change it; the first is the default)
// wafer, X, Y from tests first (read from the part's own memory), then part text; the others are
// shown in their column but never link parts (a single WXY test can be wrong, part IDs restart)
const ID_ORDER = ["w_x_y", "part_txt", "wafer_xy", "xy", "wxy", "part_id"];
// the IDs that belong to one physical part, so finding them in two files means shared parts
const ID_STRONG = ["w_x_y", "part_txt"];
const CFG_DEFAULT = { sort: "time", dedup: "", merge: false, idOrder: ID_ORDER.slice(), lbTest: "5100", ignoreIds: true, wxy: "80149", rev: "", others: [], card: ["part_txt", "wxy", "wxy_t"], touch: { on: false, base: 1, key: "auto" } };
const WCAL_DEFAULT = { w: "80100", x: "80101", y: "80102", ox: "80103", oy: "80104", mode: "add", fx: false, fy: false };
S.cfg = Object.assign({}, CFG_DEFAULT);
try {
  const old = localStorage.getItem("sc-sort");
  if (old) S.cfg.sort = old;
  Object.assign(S.cfg, JSON.parse(localStorage.getItem("sc-settings") || "{}"));
  const w = JSON.parse(localStorage.getItem("sc-wcal") || "null");
  if (w) S.wcal = Object.assign({}, WCAL_DEFAULT, w);
} catch (e) {}
// the WXY test default moved from 500 to 80149: settings saved before that move to it once
if ((S.cfg.cfgVer || 0) < 2) { if (String(S.cfg.wxy) === "500") S.cfg.wxy = CFG_DEFAULT.wxy; S.cfg.cfgVer = 2; }
// "Identify parts by" became the first ID of the order: a saved choice stays the default
if ((S.cfg.cfgVer || 0) < 3) { if (S.cfg.key && S.cfg.key !== "part_txt") S.cfg.idOrder = [S.cfg.key, ...ID_ORDER.filter((k) => k !== S.cfg.key)]; delete S.cfg.key; S.cfg.cfgVer = 3; }
// version 4: wafer, X, Y from tests first, then part text (a single WXY test proved unreliable)
if ((S.cfg.cfgVer || 0) < 4) { S.cfg.idOrder = ID_ORDER.slice(); S.cfg.cfgVer = 4; }
// before 0.3.5 the identity tests were a free "leave out tests" list: keep
// the tests no trace field covers as other IDs
if (typeof S.cfg.exclude === "string") {
  const known = new Set([...Object.values(Object.assign({}, WCAL_DEFAULT, S.wcal || {})), S.cfg.wxy, S.cfg.rev, S.cfg.lbTest].map(String));
  const left = S.cfg.exclude.split(/[,;\s]+/).map((x) => x.trim()).filter((x) => x && !known.has(x) && x !== "80100-80104");
  S.cfg.others = (S.cfg.others || []).concat(left.map((t) => ({ name: `ID ${t}`, test: t })));
  delete S.cfg.exclude;
}
// order of opened files (dataset order, e.g. retest runs): time | name | none
S.optSort = S.cfg.sort;
S.optDedup = S.cfg.dedup;
S.optMerge = S.cfg.merge;
S.lbTest = S.cfg.lbTest;
function saveCfg() {
  Object.assign(S.cfg, { sort: S.optSort, dedup: S.optDedup || "", merge: !!S.optMerge, lbTest: S.lbTest || "5100" });
  try { localStorage.setItem("sc-settings", JSON.stringify(S.cfg)); localStorage.setItem("sc-sort", S.cfg.sort); if (S.wcal) localStorage.setItem("sc-wcal", JSON.stringify(S.wcal)); } catch (e) {}
}
/** Whether a trace test is switched on (its "trace" box in Settings). Off, the test is left out of tracing only. */
function traceOn(k) { return !(S.cfg.traceOff || {})[k]; }
/** How WXY is checked and the Wafer, X, Y chip read: raw (the W, X, Y tests as logged, default) or die (the offset tests added). */
function wxyBasis() { return S.cfg.wxyBasis === "die" ? "die" : "raw"; }
/** The identity / trace tests (Settings > Part identification and trace). `trace`: the tests switched off for tracing are blank. */
function idTests(trace) {
  const w = waferCal(), on = (k, v) => (trace && !traceOn(k) ? "" : v);
  return { w: on("w", w.w), x_t: on("x", w.x), y_t: on("y", w.y), ox: on("ox", w.ox), oy: on("oy", w.oy), wxy: on("wxy", S.cfg.wxy), rev: on("rev", S.cfg.rev), lb: on("lb", S.lbTest || ""), ...Object.fromEntries((S.cfg.others || []).map((o, i) => [`o${i}`, trace && o.off ? "" : o.test])) };
}
/** Tests left out of rankings and first-fail picks: the identity tests, unless Settings keeps them in. */
function idExclude() {
  if (S.cfg.ignoreIds === false) return "";
  return Object.values(idTests()).map((v) => String(v || "").trim()).filter((v) => /^\d+(\s*-\s*\d+)?$/.test(v)).join(", ");
}
/** A "trace" box beside a trace test in Settings: untick it when the program does not log that test. */
function traceBox(on, setOn) {
  return h("span", { class: "trace-on", title: "Trace parts with this test (part trace card, parts search, Retest trace). Untick when the program does not log it; wafer maps and analytics keep their own settings." },
    h("input", { type: "checkbox", checked: on, onchange: (e) => { setOn(e.target.checked); S.rtLast = null; saveCfg(); toast("Saved", "good", 1500); } }), "trace");
}
/** Wafer, X and Y as one ID: W12X-2Y25 (letters, not |, which reads as "or" in a filter). */
// how wafer, X, Y and a test are written everywhere (Settings, Labels; the server writes reports the same way)
const WXY_FORMATS = [["W{w}X{x}Y{y}", "W1X118Y126"], ["W{w}_X{x}_Y{y}", "W1_X118_Y126"], ["{w}-{x}-{y}", "1-118-126"], ["{w}_{x}_{y}", "1_118_126"], ["{w}{x3}{y3}", "1118126 (X and Y as 3 digits)"]];
const TEST_FORMATS = [["{num} {name}", "72199 TESTNAME"], ["T#{num}_{name}", "T#72199_TESTNAME"], ["T{num}_{name}", "T72199_TESTNAME"], ["P{num}-{name}", "P72199-TESTNAME"], ["{num}::{name}", "72199::TESTNAME"]];
const pad3 = (v) => { const s = String(v); return s.startsWith("-") ? "-" + s.slice(1).padStart(3, "0") : s.padStart(3, "0"); };
const wxyWith = (t, w, x, y) => t.replace("{w}", w).replace("{x3}", pad3(x)).replace("{y3}", pad3(y)).replace("{x}", x).replace("{y}", y);
const testWith = (t, num, name) => t.replace("{num}", num).replace("{name}", name);
/** Wafer, X and Y as one ID, in the form set in Settings (default W12X-2Y25; letters, not |, which reads as "or" in a filter). */
const wxyText = (w, x, y) => wxyWith(S.cfg.wxyFmt || WXY_FORMATS[0][0], w, x, y);
/** Part trace fields a part can show (mini card, search): [key, label]. */
function traceFields() {
  const t = idTests(true);
  return [["part_txt", "Part text"], ["part_id", "Part ID"], ["prr_xy", "PRR X/Y"], ["wafer", "Wafer (WIR)"],
    ...(t.wxy ? [["wxy", "WXY"]] : []), ...(t.w || t.x_t || t.y_t ? [["wxy_t", wxyBasis() === "die" ? "Wafer, die X, Y" : "Wafer, X, Y"]] : []),
    ...(t.rev ? [["rev", "Program rev"]] : []), ...(t.lb ? [["lb", "Loadboard"]] : []),
    ...(S.cfg.others || []).flatMap((o, i) => (o.test && !o.off ? [[`o${i}`, o.name || `ID ${o.test}`]] : []))];
}
/** A part's trace field values as text, from the PRR fields and trace tests in `ids`. */
function traceValues(ids) {
  if (!ids) return {};
  const w = waferCal(), num = (v) => (v == null ? "" : Number.isInteger(v) ? String(v) : fmt(v, 6));
  const die = (v, o) => (v == null ? null : o == null ? v : w.mode === "sub" ? v - o : v + o);
  const dx = die(ids.x_t, ids.ox), dy = die(ids.y_t, ids.oy), [tx, ty] = wxyBasis() === "die" ? [dx, dy] : [ids.x_t, ids.y_t];
  const out = { part_txt: ids.part_txt || "", part_id: ids.part_id || "", prr_xy: ids.x != null && ids.x !== NO_XY ? `${ids.x}, ${ids.y}` : "", wafer: ids.wafer || "",
    wxy: num(ids.wxy), wxy_t: [ids.w, tx, ty].every((v) => v != null) ? wxyText(...[ids.w, tx, ty].map(num)) : "", rev: num(ids.rev), lb: num(ids.lb) };
  for (const k of Object.keys(ids)) if (/^o\d+$/.test(k)) out[k] = num(ids[k]);
  return out;
}
const main = document.getElementById("main");
const GROUPS = [["", "none"], ["pass", "good / bad"], ["alarm", "alarm"], ["site", "test site"], ["head", "head"], ["hbin", "hard bin"], ["sbin", "soft bin"], ["wafer", "wafer"], ["file", "file / lot"], ["loadboard", "loadboard"], ["site_loadboard", "site × loadboard"]];
/** Loadboard test number input (a session setting: the `loadboard` field reads this test's most common value per file). */
/** The loadboard test the page uses: one shared setting, changed in Settings only. */
function loadboardField() {
  return field("Loadboard test", h("div", { class: "row tight" }, h("span", { class: "small", title: "Test number holding the loadboard (DIB) ID, read per file (its most common value). The field `loadboard` in filters, color-by and ANOVA." }, `test ${S.lbTest || "5100"}`),
    h("button", { class: "ghost small", title: "One setting for every page", onclick: () => go("settings") }, "Change in Settings…")));
}

async function api(op, params) {
  const r = await fetch("/api/" + op, { method: "POST", headers: { "content-type": "application/json", "x-stdf-token": window.STDF_TOKEN }, body: JSON.stringify(params || {}) });
  const j = await r.json();
  if (!j.ok) throw new Error(j.error || "request failed");
  return j.data;
}
/** Keeps what is typed or picked in `el` under `key` for this session, so switching tabs or pages
 *  does not lose it; restores it when the field is made again. Returns `el`. */
function draft(key, el) {
  const d = (S.drafts ||= {});
  const get = () => (el.type === "checkbox" ? el.checked : el.value);
  if (d[key] !== undefined) { if (el.type === "checkbox") el.checked = d[key]; else el.value = d[key]; }
  for (const ev of ["input", "change"]) el.addEventListener(ev, () => (d[key] = get()));
  return el;
}
/** Which parts an action uses: the part filter and the population at the top, or the whole file. */
function scopeLine(what, whole) {
  const f = scoped(S.filter);
  return h("div", { class: "row tight small scope-line" }, badge(whole ? "whole file" : f ? "filtered" : "all parts", whole ? "neutral" : f ? "warn" : "neutral"),
    whole ? `${what}: every part of the file; the part filter and population at the top do not apply here.` : f ? `${what} only the parts matching ${f} (the part filter and population at the top).` : `${what} every part (no part filter, all parts).`);
}
/** A filter limited to the chosen population (all parts or good parts only). */
const scoped = (f) => (S.scope === "good" ? (f && f.trim() ? `(${f}) and pass` : "pass") : f || "");
const P = (extra) => Object.assign({ ds: S.ds, filter: scoped(S.filter), range: S.co.range }, extra || {});
const dsName = (id) => (S.datasets.find((d) => d.id === id) || {}).name || `dataset ${id}`;

// ---- chart options: axis range, 2D/3D, opacity (shared by every chart view)
function saveChartOpts() { try { localStorage.setItem("sc-chart", JSON.stringify(S.co)); } catch (e) {} }
/** Option bar for chart views. `onRange` re-fetches (range is computed on the server). */
function chartOpts(onRange, opts) {
  opts = opts || {};
  const range = select([["limits", "within limits"], ["data", "results"], ["both", "both (adaptive)"]], S.co.range, (v) => { S.co.range = v; saveChartOpts(); onRange && onRange(); });
  const dim = h("div", { class: "seg" }, ["2d", "3d"].map((d) => h("button", { class: S.co.dim === d ? "active" : "", onclick: (e) => { S.co.dim = d; saveChartOpts(); [...e.target.parentNode.children].forEach((b) => b.classList.toggle("active", b === e.target)); SC.replotAll(); } }, d.toUpperCase())));
  const opv = h("span", { class: "muted small" }, Math.round(S.co.opacity * 100) + "%");
  const op = h("input", { type: "range", min: 0.1, max: 1, step: 0.05, value: S.co.opacity, oninput: (e) => { S.co.opacity = +e.target.value; opv.textContent = Math.round(S.co.opacity * 100) + "%"; saveChartOpts(); SC.replotAll(); } });
  const hist = opts.hist ? h("div", { class: "seg", title: "% of group puts groups of different size on one scale; counts show how many parts are really in each bin" }, [["pct", "% of group"], ["count", "counts"]].map(([v, l]) => h("button", { class: S.co.hist === v ? "active" : "", onclick: (e) => { S.co.hist = v; saveChartOpts(); for (const seg of document.querySelectorAll(".seg-hist")) [...seg.children].forEach((b, i) => b.classList.toggle("active", ["pct", "count"][i] === v)); SC.replotAll(); } }, l))) : null;
  if (hist) hist.classList.add("seg-hist");
  return h("div", { class: "chart-opts" },
    opts.noRange ? null : field("Axis range", range),
    hist ? field("Histogram", hist) : null,
    field("View", dim),
    h("label", { class: "field" }, h("span", {}, "Opacity"), h("div", { class: "row tight" }, op, opv)),
    opts.extra || null);
}

/** Histogram bins: the server cuts every automatic bin into `_fine` parts;
 *  a chart's slider (scale ×¼ … ×4 the default count) merges them back. */
const BIN_SCALES = [0.25, 0.5, 1, 2, 4];
function rebin(f, scale) {
  for (const t of f.data || []) {
    if (t.type === "histogram") { t.nbinsx = Math.max(2, Math.round((t.nbinsx || 30) * scale)); continue; }
    if (t.type !== "bar" || !t._fine || !Array.isArray(t.x) || t.x.length < 2) continue;
    const m = Math.max(1, Math.round(t._fine / scale));
    const w = (t.x[t.x.length - 1] - t.x[0]) / (t.x.length - 1);
    if (m > 1) {
      const xs = [], ys = [], cs = [];
      for (let i = 0; i < t.x.length; i += m) {
        const n = Math.min(m, t.x.length - i);
        let y = 0, c = 0;
        for (let j = i; j < i + n; j++) { y += t.y[j] || 0; c += (t.customdata && t.customdata[j]) || 0; }
        xs.push(t.x[i] - w / 2 + (w * n) / 2); ys.push(y); cs.push(c);
      }
      t.x = xs; t.y = ys; if (t.customdata) t.customdata = cs;
    }
    t.width = w * m * 0.98;
  }
  return f;
}
const hasHist = (f) => (f.data || []).some((t) => t.type === "histogram" || (t.type === "bar" && t._fine));
/** A bin-size slider above every histogram chart (once per chart element). */
function histSlider(el, f) {
  const prev = el.previousElementSibling;
  const has = prev && prev.classList && prev.classList.contains("bin-slider");
  if (!hasHist(f) || f._dim === "3d") { if (has) prev.remove(); return; }
  if (has) return;
  const i0 = BIN_SCALES.indexOf(el._binScale || 1);
  const lab = h("span", { class: "muted small" });
  const show = (k) => { const s = BIN_SCALES[k]; lab.textContent = s === 1 ? "(default)" : s < 1 ? `(${1 / s}× wider bins)` : `(${s}× finer bins)`; };
  const r = h("input", { type: "range", min: 0, max: BIN_SCALES.length - 1, step: 1, value: i0 < 0 ? 2 : i0, title: "Bin size: wider bins smooth the shape, narrower bins show detail",
    oninput: (e) => { el._binScale = BIN_SCALES[+e.target.value]; show(+e.target.value); plot(el, el._fig); } });
  show(i0 < 0 ? 2 : i0);
  el.before(h("div", { class: "bin-slider row tight small" }, h("span", { class: "muted" }, "Bins: wide"), r, h("span", { class: "muted" }, "fine"), lab));
}

/** Box plot with its groups shown as "A" / "B" (the part before ":") so
 *  small charts keep their room; the hover keeps the full name. */
function shortBoxes(fig) {
  const short = (s) => String(s).split(":")[0];
  return Object.assign({}, fig, {
    data: (fig.data || []).map((t) => Object.assign({}, t, { x: (t.x || []).map(short) })),
    layout: Object.assign({}, fig.layout, { margin: Object.assign({}, (fig.layout || {}).margin, { b: 30 }) }),
  });
}
/** A histogram with the box plot of the same groups underneath, horizontal,
 *  sharing the value axis. */
function histWithBox(hist, box) {
  if (!hist || !box || !(box.data || []).length) return hist;
  const bx = box.data.map((t) => {
    const c = Object.assign({}, t, { xaxis: "x", yaxis: "y2" });
    if (t.type === "box") { c.orientation = "h"; c.y = t.x; delete c.x; c.hovertemplate = t.hovertemplate; }
    else { c.x = t.y; c.y = t.x; c.hovertemplate = "%{x:.4g}<extra>outlier</extra>"; }
    return c;
  });
  const nb = box.data.filter((t) => t.type === "box").length;
  const hb = Math.min(0.4, 0.1 + 0.05 * nb);
  const l = Object.assign({}, hist.layout);
  l.yaxis = Object.assign({}, l.yaxis, { domain: [hb + 0.06, 1] });
  l.yaxis2 = { domain: [0, hb], type: "category", automargin: true, showgrid: false, autorange: "reversed" };
  l.xaxis = Object.assign({}, l.xaxis, { anchor: "y2" });
  return { data: [...hist.data, ...bx], layout: l };
}

/** Applies the chart options to a resolved Plotly figure. */
function chartTransform(f, el) {
  rebin(f, (el && el._binScale) || 1);
  const op = S.co.opacity;
  // figures marked _normable (compare views) follow the % / counts switch
  if (f._normable && S.co.hist === "count") {
    let hit = false;
    for (const t of f.data || []) {
      if (t.type !== "bar" || t.meta !== "pct" || !t.customdata) continue;
      t.y = t.customdata;
      t.hovertemplate = String(t.hovertemplate || "").replace(/%\{y:\.3g\}% of group \(%\{customdata\} parts\)/, "count: %{y}");
      hit = true;
    }
    if (hit && f.layout) f.layout = Object.assign({}, f.layout, { yaxis: Object.assign({}, f.layout.yaxis, { title: { text: "parts" } }) });
  }
  for (const t of f.data || []) {
    if (t.type === "bar" && !t.orientation) t.opacity = op;
    // `_dim` scales a trace back (e.g. the parts not highlighted)
    else if ((t.type === "scatter" || t.type === "scattergl") && t.mode && t.mode.includes("markers") && t.hoverinfo !== "skip") t.marker = Object.assign({}, t.marker, { opacity: op * (t._dim ?? 1) });
  }
  return S.co.dim === "3d" ? to3D(f) : f;
}
/** 2D → 3D: overlaid histograms become bar layers, series get their own depth
 *  lane, value heatmaps (wafer test maps) become surfaces. Other figures stay 2D. */
function to3D(f) {
  const data = f.data || [], L = f.layout || {};
  const title = (a) => (a && a.title && (a.title.text ?? a.title)) || "";
  const scene = (zTitle, lanes) => ({
    xaxis: { title: { text: title(L.xaxis) }, range: L.xaxis && L.xaxis.range },
    yaxis: lanes ? { title: { text: "" }, tickvals: lanes.map((_, i) => i), ticktext: lanes } : { title: { text: title(L.yaxis) }, autorange: "reversed" },
    zaxis: { title: { text: zTitle } },
    camera: { eye: { x: 1.5, y: -1.6, z: 0.9 } },
    aspectmode: "manual", aspectratio: lanes ? { x: 1.6, y: Math.min(1.2, 0.35 + 0.15 * lanes.length), z: 0.7 } : { x: 1, y: 1, z: 0.45 },
  });
  const out = (traces, sc) => ({ _dim: "3d", data: traces, layout: { title: L.title, margin: { l: 0, r: 0, t: 48, b: 0 }, legend: L.legend, showlegend: L.showlegend, scene: sc } });
  const hm = data.find((t) => t.type === "heatmap");
  if (hm) {
    if (hm.showscale === false) return f; // categorical (bin) map
    return out([{ type: "surface", x: hm.x, y: hm.y, z: hm.z, text: hm.text, colorscale: hm.colorscale, cmin: hm.zmin, cmax: hm.zmax, colorbar: hm.colorbar, opacity: Math.max(S.co.opacity, 0.35), hovertemplate: "(%{x},%{y}) %{text}<br>%{z:.4g}<extra></extra>", connectgaps: false }], scene(title(hm.colorbar) || "value"));
  }
  if (data.some((t) => t.type === "box" || t.orientation === "h")) return f;
  const bars = data.filter((t) => t.type === "bar");
  const lanesOf = (ts) => ts.map((t) => String(t.name || "").replace(/ \(n=\d+\)$/, ""));
  const limitFrames = (zmin, zmax, nl, xr) => (L.shapes || []).filter((s) => s.type === "line").map((s) => {
    const c = (s.line && s.line.color) || "red";
    if (s.xref === "x") return { type: "scatter3d", mode: "lines", x: Array(5).fill(s.x0), y: [-0.5, nl - 0.5, nl - 0.5, -0.5, -0.5], z: [zmin, zmin, zmax, zmax, zmin], line: { color: c, width: 4, dash: "dash" }, showlegend: false, hoverinfo: "skip" };
    if (s.yref === "y" && xr) return { type: "scatter3d", mode: "lines", x: [xr[0], xr[1], xr[1], xr[0], xr[0]], y: [-0.5, -0.5, nl - 0.5, nl - 0.5, -0.5], z: Array(5).fill(s.y0), line: { color: c, width: 4, dash: "dash" }, showlegend: false, hoverinfo: "skip" };
    return null;
  }).filter(Boolean);
  if (bars.length) {
    let zmax = 0;
    const traces = bars.map((t, i) => {
      const x = [], y = [], z = [], I = [], J = [], K = [];
      const w = (t.width || 1) / 2;
      t.x.forEach((c, k) => {
        const hgt = t.y[k];
        if (!(hgt > 0)) return;
        zmax = Math.max(zmax, hgt);
        const b = x.length;
        for (const [dx, dy, dz] of [[-w, -0.38, 0], [w, -0.38, 0], [w, 0.38, 0], [-w, 0.38, 0], [-w, -0.38, hgt], [w, -0.38, hgt], [w, 0.38, hgt], [-w, 0.38, hgt]]) { x.push(c + dx); y.push(i + dy); z.push(dz); }
        for (const [p, q, r] of [[0, 1, 2], [0, 2, 3], [4, 5, 6], [4, 6, 7], [0, 1, 5], [0, 5, 4], [1, 2, 6], [1, 6, 5], [2, 3, 7], [2, 7, 6], [3, 0, 4], [3, 4, 7]]) { I.push(b + p); J.push(b + q); K.push(b + r); }
      });
      const color = (t.marker && t.marker.color) || undefined;
      return { type: "mesh3d", name: t.name, x, y, z, i: I, j: J, k: K, color: Array.isArray(color) ? color[0] : color, opacity: op3(), flatshading: true, showlegend: true, hoverinfo: "x+z+name" };
    });
    return out(traces.concat(limitFrames(0, zmax, bars.length)), scene(title(L.yaxis), lanesOf(bars)));
  }
  const series = data.filter((t) => (t.type === "scatter" || t.type === "scattergl") && t.hoverinfo !== "skip" && t.x && t.x.length && t.x[0] !== null);
  if (!series.length) return f;
  const ys = series.flatMap((t) => t.y);
  const xs = series.flatMap((t) => t.x).filter((v) => typeof v === "number");
  const zs = ys.filter((v) => typeof v === "number");
  const zmin = Math.min(...zs), zmax = Math.max(...zs);
  const traces = series.map((t, i) => ({ type: "scatter3d", mode: t.mode, name: t.name, x: t.x, y: t.x.map(() => i), z: t.y, customdata: t.customdata, hovertemplate: t.hovertemplate, line: t.line ? Object.assign({}, t.line, { width: 4 }) : undefined, marker: t.marker ? Object.assign({}, t.marker, { size: 3, line: undefined }) : undefined }));
  const xr = xs.length ? [Math.min(...xs), Math.max(...xs)] : null;
  return out(traces.concat(limitFrames(zmin, zmax, series.length, xr).filter((tr) => tr.z.every((v) => isFinite(v)) && (!xr || tr.x.every((v) => v >= xr[0] && v <= xr[1])) && tr.z.every((v) => v >= zmin && v <= zmax))), scene(title(L.yaxis), lanesOf(series)));
}
const op3 = () => Math.max(0.15, S.co.opacity);
SC.setTransform(chartTransform, histSlider);

/** Modal text prompt; resolves to the text or null. */
function askText(title, value, hint) {
  return new Promise((resolve) => {
    const inp = h("input", { type: "text", value: value || "", style: { width: "100%" } });
    const done = (v) => { dlg.close(); dlg.remove(); resolve(v); };
    const dlg = h("dialog", { class: "dlg" }, h("form", { method: "dialog", onsubmit: (e) => { e.preventDefault(); done(inp.value.trim() || null); } },
      h("h2", { style: { marginTop: 0 } }, title), hint ? h("p", { class: "muted small" }, hint) : null, inp,
      h("div", { class: "row", style: { justifyContent: "flex-end", marginTop: "12px" } }, h("button", { type: "button", class: "ghost", onclick: () => done(null) }, "Cancel"), h("button", { type: "submit", class: "primary" }, "OK"))));
    dlg.addEventListener("cancel", () => done(null));
    document.body.append(dlg);
    dlg.showModal();
    inp.select();
  });
}
async function renameDs(id) {
  const name = await askText("Rename dataset", dsName(id), "A short name makes files easier to tell apart in Compare, Match and charts.");
  if (!name) return;
  try { await api("rename", { ds: id, name }); await refreshDatasets(); go(S.view); toast(`Renamed to ${name}`, "good"); } catch (e) { toast(e.message, "bad"); }
}
function loading(el, msg) { el.replaceChildren(h("div", { class: "loading" }, msg || "Computing…")); }
function showErr(el, e) { el.replaceChildren(h("div", { class: "err" }, String(e.message || e))); }
function select(options, value, onchange, attrs) {
  const s = h("select", Object.assign({ onchange: (e) => onchange && onchange(e.target.value) }, attrs || {}), options.map(([v, l]) => h("option", { value: v, selected: v === value }, l)));
  return s;
}
function field(label, input) { return h("label", { class: "field" }, h("span", {}, label), input); }
function dsOptions() { return S.datasets.map((d) => [String(d.id), `${d.id}: ${d.name}`]); }

async function testsFor(ds, filter) {
  const k = ds + "|" + (filter || "");
  if (!S.testsCache[k]) S.testsCache[k] = await api("tests", { ds, filter: filter || "" });
  return S.testsCache[k];
}
/** Test picker; `ds` selects another dataset's test list (default: the current one). */
function testInput(id, value, placeholder, ds) {
  let list = "tests-dl";
  if (ds !== undefined && ds !== S.ds) {
    list = "tests-dl-" + ds;
    if (!document.getElementById(list)) {
      const dl = document.body.appendChild(h("datalist", { id: list }));
      testsFor(ds, "").then((t) => dl.replaceChildren(...t.map((x) => h("option", { value: x.key })))).catch(() => {});
    }
  } else if (!document.getElementById("tests-dl")) document.body.appendChild(h("datalist", { id: "tests-dl" }));
  return h("input", { type: "text", id, list, value: value || "", placeholder: placeholder || "test number, name or key", style: { minWidth: "240px" } });
}
async function fillTestList() {
  const dl = document.getElementById("tests-dl") || document.body.appendChild(h("datalist", { id: "tests-dl" }));
  try {
    const t = await testsFor(S.ds, "");
    dl.replaceChildren(...t.map((x) => h("option", { value: x.key })));
  } catch (e) {}
}

// ---------------------------------------------------------------- views
const VIEWS = {
  home: { label: "Home", fn: viewHome, always: true },
  overview: { label: "Overview", fn: viewOverview },
  explorer: { label: "Test explorer", fn: viewExplorer },
  wafer: { label: "Wafer maps", fn: viewWafer },
  compare: { label: "Compare", fn: viewCompare },
  trace: { label: "Trace", fn: viewTrace },
  qual: { label: "Drift report", fn: viewQual },
  correlation: { label: "Correlation", fn: viewCorrelation },
  anova: { label: "ANOVA", fn: viewAnova },
  anomaly: { label: "Abnormal parts", fn: viewAnomaly },
  ml: { label: "Machine learning", fn: viewML },
  records: { label: "Records", fn: viewRecords },
  data: { label: "Edit, merge & export", fn: viewData },
  investigation: { label: "Failure investigation", fn: viewInvestigation },
  candidates: { label: "Bad candidates", fn: viewCandidates },
  signature: { label: "Signature check", fn: viewSignature },
  retest: { label: "Retest recovery", fn: viewRetest },
  settings: { label: "Settings", fn: viewSettings, always: true },
};
// pages that became a tab of another page: [page, tab]
const VIEW_TABS = {
  parttrace: ["trace"], findpart: ["trace"],
  parts: ["records", "raw"], datalog: ["records", "datalog"], raw: ["records", "raw"],
  trim: ["data", "trim"], export: ["data", "export"], match: ["data", "match"],
};

// a heading above the first view of each group ("" = a plain separator)
const NAV_GROUPS = { overview: "Explore", qual: "Investigate", records: "Data & files", investigation: "Experimental", settings: "" };
function renderNav() {
  const nav = document.getElementById("nav");
  nav.replaceChildren();
  for (const [k, v] of Object.entries(VIEWS)) {
    if (NAV_GROUPS[k] !== undefined) nav.append(NAV_GROUPS[k] ? h("div", { class: "nav-group" }, NAV_GROUPS[k]) : h("div", { class: "sep" }));
    const disabled = !v.always && !S.datasets.length;
    nav.append(h("button", { class: (S.view === k ? "active" : "") + (disabled ? " dim" : ""), onclick: () => go(k), title: disabled ? "Open an STDF file first" : "" }, v.label));
  }
  nav.append(h("div", { class: "hint" }, S.datasets.length ? "The part filter and population at the top apply to every analysis and export; raw records, the part datalog and Edit & trim work on the whole file, and say so." : "Open an STDF file to unlock the analyses."));
}
/** A view name, a tab name (parttrace) or page/tab (trace/retest) as [page, tab or undefined]; null when unknown. */
function viewOf(name) {
  const [v, t] = String(name || "").split("/");
  if (VIEW_TABS[v]) return VIEW_TABS[v];
  return VIEWS[v] ? [v, t] : null;
}
function go(name) {
  const [view, tab] = viewOf(name) || ["home"];
  if (tab) (S.tabs ||= {})[view] = tab;
  S.view = view;
  try { history.replaceState(null, "", "#" + view + (S.tabs && S.tabs[view] ? "/" + S.tabs[view] : "")); } catch (e) {}
  renderNav();
  main.replaceChildren();
  if (!S.datasets.length && !VIEWS[view].always) { S.view = "home"; renderNav(); return viewHome(main); }
  VIEWS[view].fn(main);
}
/** A page of tabs [id, label, fn(el)]: the open tab is kept per page and in the address. Each tab's own h1 is hidden under the page's. */
function tabbedPage(el, page, title, lead, tabs) {
  const S_ = (S.tabs ||= {});
  const bar = h("div", { class: "tabs page-tabs", role: "tablist" });
  const body = h("div", { class: "tab-body" });
  el.append(h("h1", {}, title), lead ? h("p", { class: "lead" }, lead) : "", bar, body);
  const open = (k) => {
    S_[page] = k;
    try { history.replaceState(null, "", `#${page}/${k}`); } catch (e) {}
    bar.replaceChildren(...tabs.map(([id, label]) => h("button", { class: id === k ? "active" : "", role: "tab", "aria-selected": String(id === k), onclick: () => open(id) }, label)));
    body.replaceChildren();
    tabs.find(([id]) => id === k)[2](body);
  };
  open(tabs.some(([k]) => k === S_[page]) ? S_[page] : tabs[0][0]);
}
function viewData(el) {
  tabbedPage(el, "data", "Edit, merge & export", "", [
    ["trim", "Edit & trim", viewTrim],
    ["export", "Merge, split & export", viewExport],
    ["match", "Match parts", viewMatch],
  ]);
}
function otherDs(id) {
  const o = S.datasets.find((d) => d.id !== id);
  return o ? o.id : id;
}

// ---- Overview
async function viewOverview(el) {
  el.append(h("h1", {}, "Overview"));
  const body = h("div");
  el.append(body);
  loading(body);
  try {
    const d = await api("overview", P({ by_site: !!S.cfg.ovSite }));
    const o = d.overview;
    body.replaceChildren();
    body.append(h("div", { class: "kpis" },
      kpi("Parts", o.n_parts.toLocaleString(), S.filter ? `${d.selected.toLocaleString()} selected` : `${o.files.length} file(s)`),
      o.partial_parts ? kpi("Cut off", o.partial_parts.toLocaleString(), `no PRR record: kept without bin or verdict. Yield ${pct(o.yield_pct)} counting them, ${pct(o.yield_without_partial_pct)} without`) : "",
      kpi("Yield", pct(S.filter ? d.selected_yield : o.yield_pct), o.yield_without_partial_pct != null ? `${o.n_pass.toLocaleString()} pass · ${pct(o.yield_without_partial_pct)} without the cut-off part` : `${o.n_pass.toLocaleString()} pass`),
      kpi("Tests", o.n_tests.toLocaleString()),
      kpi("Wafers", o.n_wafers || "–"),
      kpi("Sites", o.sites),
      kpi("Retests", o.retests.toLocaleString()),
      kpi("Lot", o.lots.join(", ") || "–", o.part_typ),
      kpi("Program", o.job || "–", o.tester)));
    // more than one site: the bin and yield bars can be split by site
    if (o.sites > 1) body.append(h("div", { class: "row tight" }, h("label", { class: "chk", title: "Bins as the % of each site's parts, and yield by wafer and file, one bar per site" },
      h("input", { type: "checkbox", checked: !!S.cfg.ovSite, onchange: (e) => { S.cfg.ovSite = e.target.checked; saveCfg(); go("overview"); } }), `Split the bars by site (${o.sites} sites)`)));
    const g = h("div", { class: "grid2" });
    for (const k of ["hbin", "sbin", "site", "wafer", "file"]) {
      if (!d.figures[k]) continue;
      const [card, c] = chartBox();
      g.append(card);
      setTimeout(() => plot(c, d.figures[k]));
    }
    body.append(g);
    if (d.figures.wafermap) {
      body.append(h("h2", {}, "Stacked bin map"));
      const [card, c] = chartBox("tall");
      body.append(card);
      plot(c, d.figures.wafermap);
    }
    body.append(h("h2", {}, "Fail pareto"), h("p", { class: "lead" }, "First failing test per part (what binned the part out) and the total number of parts outside limits for each test. Click a row to open it in the test explorer."));
    const fp = h("div");
    body.append(fp);
    table(fp, [
      { key: "key", label: "Test" },
      { key: "first_fail", label: "First-fail parts", num: true },
      { key: "pct_of_fails", label: "% of fails", num: true, fmt: (v) => pct(v, 1) },
      { key: "any_fail", label: "Out-of-limit parts", num: true },
    ], d.fail_pareto, { onClick: (r) => { S.explorerTest = r.key; go("explorer"); } });
    body.append(h("h2", {}, "Bins"));
    const bins = h("div", { class: "grid2" });
    const hb = h("div"), sb = h("div");
    bins.append(h("div", { class: "card pad" }, h("b", {}, "Hard bins"), hb), h("div", { class: "card pad" }, h("b", {}, "Soft bins"), sb));
    body.append(bins);
    const bc = [{ key: "bin", label: "Bin", num: true }, { key: "name", label: "Name" }, { key: "pf", label: "P/F" }, { key: "count", label: "Parts", num: true }, { key: "pct", label: "%", num: true, fmt: (v) => pct(v) }];
    table(hb, bc, d.hbins, { search: false });
    table(sb, bc, d.sbins, { search: false });
    body.append(h("h2", {}, "Source files"));
    const src = h("div");
    body.append(src);
    body.append(h("h2", {}, "Datalog"), h("p", { class: "lead" }, "GDR and DTR records of the file (datalog text and generic data), with the number of parts before each one. Opened on demand."));
    const dl = h("details", { class: "adv" }, h("summary", {}, "Show the datalog records"), h("div"));
    dl.addEventListener("toggle", async () => {
      if (!dl.open || dl.dataset.loaded) return;
      dl.dataset.loaded = "1";
      const rows = await api("datalog", P({}));
      table(dl.lastChild, [{ key: "kind", label: "Kind" }, { key: "after_part", label: "After part", num: true }, { key: "text", label: "Text" }], rows, { search: true });
    });
    body.append(dl);
    // instrument alarms some testers log as GDR or DTR records, decoded and tied to their parts
    const al = h("div");
    body.append(al);
    api("inst_alarms", { ds: S.ds }).then((r) => {
      if (!r.alarms.length) return;
      al.append(h("h2", {}, "Instrument alarms"),
        h("p", { class: "lead" }, `${r.alarms.length.toLocaleString()} alarm records in the GDR and DTR records, on ${r.parts.toLocaleString()} parts. Site, test, instrument, code and severity are read from KEY=value pairs or the record's text; an alarm is tied to the part of its site tested when it was logged. The part filter field inst_alarm selects these parts.`),
        h("div", { class: "row" }, h("button", { onclick: () => useFilter(S.ds, andF(S.filter, "inst_alarm")) }, "Filter to parts with an instrument alarm")));
      const t = h("div");
      al.append(t);
      table(t, [{ key: "kind", label: "Record" }, { key: "part_id", label: "Part ID" }, { key: "site", label: "Site", num: true }, { key: "test", label: "Test", num: true }, { key: "test_name", label: "Test name" }, { key: "instrument", label: "Instrument" }, { key: "code", label: "Code" }, { key: "severity", label: "Severity" }, { key: "message", label: "Message" }, { key: "after_part", label: "After part", num: true }], r.alarms, { search: true });
    }).catch(() => {});
    table(src, [{ key: "path", label: "File" }, { key: "lot_id", label: "Lot" }, { key: "sublot_id", label: "Sublot" }, { key: "part_typ", label: "Product" }, { key: "job_nam", label: "Program" }, { key: "node_nam", label: "Node" }, { key: "test_cod", label: "Step" }, { key: "tst_temp", label: "Temp" }, { key: "endian", label: "Endian" }], d.sources, { search: false });
  } catch (e) { showErr(body, e); }
}

// ---- Test explorer
/** Cpk cell: amber below 1.33, red below 1.00 (the rows are marked the same way). */
const cpkFlag = (v) => (Number.isFinite(v) ? h("span", { class: v < 1 ? "flag-fail" : v < 1.33 ? "flag-warn" : "" }, fmt(v, 2)) : "");

async function viewExplorer(el) {
  el.append(h("h1", {}, "Test explorer"), h("p", { class: "lead" }, "Pick a test to see its distribution against limits, split by any factor. Add a second test for a correlation scatter."));
  const left = h("div", { class: "card pad" });
  const right = h("div");
  el.append(h("div", { class: "split" }, left, right));
  loading(left, "Loading tests…");
  const groupSel = select(GROUPS, S.explorerGroup || "", (v) => { S.explorerGroup = v; draw(); });
  const t2 = testInput("t2", S.explorerTest2 || "", "optional 2nd test for scatter");
  t2.addEventListener("change", () => { S.explorerTest2 = t2.value; draw(); });
  const dens = h("input", { type: "checkbox", checked: S.explorerDensity ?? true, onchange: (e) => { S.explorerDensity = e.target.checked; draw(); } });
  right.append(h("div", { class: "controls" }, field("Color parts by", groupSel), field("Scatter vs", t2), h("label", { class: "chk" }, dens, "normalize groups (%)")), chartOpts(() => draw()));
  const charts = h("div");
  right.append(charts);
  try {
    const tests = await testsFor(S.ds, scoped(S.filter));
    table(left, [
      { key: "key", label: "Test" },
      { key: "units", label: "Units" },
      { key: "n", label: "N", num: true },
      { key: "cpk", label: "Cpk", num: true, render: (v) => cpkFlag(v) },
      { key: "fail_pct", label: "Fail %", num: true, render: (v) => h("span", { class: v ? "flag-fail" : "" }, v ? v.toFixed(2) : "0") },
    ], tests, { onClick: (r) => { S.explorerTest = r.key; draw(); }, sort: "fail_pct", maxHeight: "760px", placeholder: "Search tests…", rowClass: (r) => (Number.isFinite(r.cpk) ? (r.cpk < 1 ? "row-fail" : r.cpk < 1.33 ? "row-warn" : "") : "") });
    left.append(h("p", { class: "muted small" }, "Cpk amber below 1.33 and red below 1.00; fail % is red where a part failed."));
    // opens on the test most parts fail (of the latest file when the dataset holds several of one lot)
    if (!S.explorerTest) {
      const f = await featuredTest([S.ds]);
      const first = [...tests].filter((t) => t.kind !== "Functional").sort((a, b) => (b.fail_pct || 0) - (a.fail_pct || 0))[0];
      S.explorerTest = (f && f.key && tests.some((t) => t.key === f.key) ? f.key : null) || (first && first.key);
    }
    draw();
  } catch (e) { showErr(left, e); }
  async function draw() {
    if (!S.explorerTest) return;
    loading(charts);
    try {
      const d = await api("chart", P({ kind: "hist", test: S.explorerTest, group_by: S.explorerGroup || "", density: dens.checked }));
      const t = d.test;
      charts.replaceChildren(h("div", { class: "kpis" },
        kpi("Test", `${t.num}`, t.name), kpi("Limits", `${fmt(t.lo)} … ${fmt(t.hi)}`, t.units || ""), kpi("Results", d.stats.reduce((a, g) => a + g.n, 0).toLocaleString(), S.scope === "good" || S.filter ? "in the selection" : ""), kpi("Outside limits", d.stats.reduce((a, g) => a + g.fails, 0).toLocaleString())));
      const g = h("div", { class: "grid2" });
      charts.append(g);
      // groups unticked in the statistics table are left off the charts (kept per grouping)
      const multi = d.stats.length > 1;
      const hidKey = S.explorerGroup || "";
      const hidden = (S.explorerHidden ||= {})[hidKey] ||= new Set();
      const isOf = (tr, name) => tr.name === name || (tr.name || "").startsWith(`${name} (n=`) || tr.name === `${name} outliers`;
      const shown = (k) => (multi && hidden.size ? Object.assign({}, d[k], { data: d[k].data.filter((tr) => !d.stats.some((x) => hidden.has(x.group) && isOf(tr, x.group))) }) : d[k]);
      const plotAll = () => {
        g.replaceChildren();
        // histogram with its box plot underneath on one value axis, then CDF and trend
        const [hc, c0] = chartBox("tall");
        hc.style.gridColumn = "1 / -1";
        g.append(hc);
        plot(c0, histWithBox(shown("hist"), shown("box")));
        for (const k of ["cdf", "trend"]) {
          const [card, c] = chartBox();
          g.append(card);
          plot(c, shown(k));
        }
      };
      plotAll();
      const toggle = (name, on) => { if (on) hidden.delete(name); else hidden.add(name); plotAll(); };
      const allBox = multi ? h("span", { class: "row tight small" },
        h("button", { onclick: () => { hidden.clear(); st.querySelectorAll("input[data-group]").forEach((x) => (x.checked = true)); plotAll(); } }, "All"),
        h("button", { onclick: () => { d.stats.forEach((x) => hidden.add(x.group)); st.querySelectorAll("input[data-group]").forEach((x) => (x.checked = false)); plotAll(); } }, "None"),
        h("span", { class: "muted" }, "tick the groups to show on the charts")) : null;
      charts.append(h("div", { class: "row" }, h("h2", {}, "Statistics by group"), allBox));
      const st = h("div");
      charts.append(st);
      const groupCol = multi
        ? { key: "group", label: "Group", render: (v) => h("label", { class: "chk" }, h("input", { type: "checkbox", "data-group": v, checked: !hidden.has(v), onclick: (e) => e.stopPropagation(), onchange: (e) => toggle(v, e.target.checked) }), String(v)) }
        : { key: "group", label: "Group" };
      table(st, [groupCol, { key: "n", label: "N", num: true }, { key: "mean", label: "Mean", num: true }, { key: "std", label: "Std", num: true }, { key: "min", label: "Min", num: true }, { key: "q1", label: "Q1", num: true }, { key: "median", label: "Median", num: true }, { key: "q3", label: "Q3", num: true }, { key: "max", label: "Max", num: true }, { key: "cpk", label: "Cpk", num: true, fmt: (v) => fmt(v, 2) }, { key: "fails", label: "Fails", num: true }], d.stats, { search: false });
      if (t2.value) {
        const s = await api("chart", P({ kind: "scatter", test: S.explorerTest, test2: t2.value, group_by: S.explorerGroup || "" }));
        const [card, c] = chartBox("tall");
        charts.append(h("h2", {}, "Scatter"), card);
        plot(c, s.scatter);
      }
    } catch (e) { showErr(charts, e); }
  }
}

// ---- Wafer maps
async function viewWafer(el) {
  const ds = S.datasets.find((d) => d.id === S.ds);
  el.append(h("h1", {}, "Wafer maps"), h("p", { class: "lead" }, "Bin, pass/fail or test-value maps per wafer. Die coordinates come from the PRR X/Y (wafers from WIR), or are rebuilt from datalogged test results, e.g. wafer, X, Y and an X/Y offset logged in tests 80100–80104 (die X = X + X offset). The tests and offsets are set once in Settings. If the map still looks off, try subtracting the offset or flipping an axis."));
  if (!S.wcoords) S.wcoords = ds && ds.wafers.length ? "prr" : "tests";
  const coords = select([["prr", "PRR X/Y + WIR wafer"], ["tests", "die X/Y from tests"]], S.wcoords, (v) => { S.wcoords = v; S.wafer = "all"; renderCtl(); draw(); });
  const mode = select([["hbin", "hard bin"], ["sbin", "soft bin"], ["pass", "pass / fail"], ["test", "test value"]], S.wmode || "hbin", (v) => { S.wmode = v; draw(); });
  const test = testInput("wt", S.wtest || S.explorerTest || "");
  test.addEventListener("change", () => { S.wtest = test.value; if ((S.wmode || "hbin") === "test") draw(); });
  if (!test.value) featuredTest([S.ds]).then((f) => { if (f && f.key && !test.value) { test.value = f.key; S.wtest = f.key; if ((S.wmode || "hbin") === "test") draw(); } });
  // the tests and offsets are one shared setting (Settings), shown here; flips only turn the map
  const wc = waferCal();
  const fx = h("input", { type: "checkbox", checked: wc.fx, onchange: () => cal() });
  const fy = h("input", { type: "checkbox", checked: wc.fy, onchange: () => cal() });
  function cal() { Object.assign(wc, { fx: fx.checked, fy: fy.checked }); saveCfg(); S.wafer = "all"; draw(); }
  const calText = () => {
    const off = (t, o) => (o ? `${t} ${wc.mode === "sub" ? "−" : "+"} ${o}` : t || "–");
    return `wafer ${wc.w || "none (one map)"} · X ${off(wc.x, wc.ox)} · Y ${off(wc.y, wc.oy)}`;
  };
  const calField = field("Die X/Y from tests (shared setting)", h("div", { class: "row tight" }, h("span", { class: "small" }, calText()),
    h("button", { class: "ghost small", title: "The wafer, X, Y and offset tests are set once, in Settings, for every page", onclick: () => go("settings") }, "Change in Settings…")));
  const wafer = select([["all", "all wafers (gallery)"]], "all", (v) => { S.wafer = v; draw(); });
  const ctl = h("div", { class: "controls" });
  function renderCtl() {
    ctl.replaceChildren(field("Coordinates", coords), ...(S.wcoords === "tests" ? [calField, h("label", { class: "chk", title: "Display orientation of the map" }, fx, "flip X"), h("label", { class: "chk", title: "Display orientation of the map" }, fy, "flip Y")] : []), field("Color by", mode), field("Test (for test value)", test), field("Wafer", wafer));
  }
  renderCtl();
  const out = h("div");
  el.append(ctl, chartOpts(null, { noRange: true }), out);
  async function draw() {
    loading(out);
    try {
      const d = await api("chart", P({ kind: "wafer", mode: S.wmode || "hbin", test: test.value, wafer: S.wafer || "all", coords: S.wcoords, wafer_test: wc.w, x_test: wc.x, y_test: wc.y, ox_test: wc.ox, oy_test: wc.oy, offset_mode: wc.mode, flip_x: wc.fx, flip_y: wc.fy }));
      const names = d.wafers || [];
      if (S.wafer && S.wafer !== "all" && !names.includes(S.wafer)) S.wafer = "all";
      wafer.replaceChildren(...[["all", "all wafers (gallery)"], ...names.map((w) => [w, w])].map(([v, l]) => h("option", { value: v, selected: v === (S.wafer || "all") }, l)));
      out.replaceChildren();
      if (names.length > 24 && (S.wafer || "all") === "all") out.append(h("p", { class: "muted" }, `Showing the first 24 of ${names.length} wafers; pick one above to see the others.`));
      const gal = h("div", { class: d.maps.length > 1 ? "gallery" : "" });
      out.append(gal);
      for (const m of d.maps) { const [card, c] = chartBox(d.maps.length > 1 ? "" : "tall"); gal.append(card); plot(c, m.figure); }
    } catch (e) { showErr(out, e); }
  }
  draw();
}

// ---- Compare
async function viewCompare(el) {
  el.append(h("h1", {}, "Compare"), h("p", { class: "lead" }, "Two populations, every test: mean shift (in σ and % of limit range), spread ratio, Welch t-test, Kolmogorov–Smirnov, BH-corrected q-values; plus bin and site-yield deltas. Use two files, or a group of parts against the rest of one file."));
  const mode = select([["datasets", "File A vs file B"], ["group", "Group vs rest (same file)"]], S.cmpMode || (S.datasets.length > 1 ? "datasets" : "group"), (v) => { S.cmpMode = v; render(); });
  const ctl = h("div", { class: "controls" });
  const out = h("div");
  el.append(ctl, out);
  function render() {
    const m = mode.value;
    const a = select(dsOptions(), String(S.cmpA ?? S.ds), (v) => (S.cmpA = +v));
    const b = select(dsOptions(), String(S.cmpB ?? otherDs(S.cmpA ?? S.ds)), (v) => (S.cmpB = +v));
    const fa = h("input", { type: "text", class: "filter-input", placeholder: "filter A (optional)", value: S.cmpFA || "" });
    const fb = h("input", { type: "text", class: "filter-input", placeholder: "filter B (optional)", value: S.cmpFB || "" });
    const sel = h("input", { type: "text", class: "filter-input", placeholder: "group, e.g. site = 3  or  $match  or  part_id in @ids.txt", value: S.cmpSel || "", style: { minWidth: "300px" } });
    const vs = h("input", { type: "text", class: "filter-input", placeholder: "versus (blank = all other parts)", value: S.cmpVs || "" });
    const matchBy = select([["key", "number + name"], ["num", "test number"], ["name", "test name"]], S.cmpMatch || "key", (v) => (S.cmpMatch = v));
    const later = select([["", "none"], ...dsOptions().filter(([v]) => +v !== S.ds)], S.cmpLater ?? "", (v) => { S.cmpLater = v; keys.hidden = !v; if (v) kb.forDs(+v); });
    const ka = keyPicker("cmp_a", S.ds), kb = keyPicker("cmp_b", S.cmpLater ? +S.cmpLater : null);
    const keys = h("div", { class: "row tight", hidden: !S.cmpLater }, ka.el, h("span", { class: "muted small" }, "↔"), kb.el);
    const run = h("button", { class: "primary", onclick: () => {
      S.cmpFA = fa.value; S.cmpFB = fb.value; S.cmpSel = sel.value; S.cmpVs = vs.value;
      const params = m === "group" ? { mode: "group", a: S.ds, select: sel.value, vs: vs.value, filter: scoped(S.filter), match_by: matchBy.value } : { mode: "datasets", a: +a.value, b: +b.value, filter_a: scoped(fa.value), filter_b: scoped(fb.value), match_by: matchBy.value };
      if (m === "group" && later.value) Object.assign(params, { later: +later.value, key_a: ka.value(), key_b: kb.value() });
      if (idExclude()) params.exclude = idExclude();
      runCompare(out, params);
    } }, "Compare");
    ctl.replaceChildren(field("Mode", mode));
    if (m === "group") ctl.append(field("Group (B)", sel), field("Versus (A)", vs), field("Same units in file (drift)", later), field("identified by", keys));
    else ctl.append(field("A", a), field("Filter A", fa), field("B", b), field("Filter B", fb));
    ctl.append(field("Match tests by", matchBy), run);
    return run;
  }
  const runBtn = render();
  if (S.cmpAuto || (mode.value === "datasets" && S.datasets.length > 1)) { S.cmpAuto = false; runBtn.click(); }
}
async function runCompare(out, params) {
  loading(out);
  const mkBox = h("div");
  const mkP = api("markers", Object.assign({}, params, { range: S.co.range, methods: mkMethods() })).catch((e) => e);
  try {
    const d = await api("compare", params);
    const r = d.report;
    let pick = null;
    mkBox.replaceChildren(h("h2", {}, "Marker candidates"), h("div", { class: "loading" }, "Ranking every test (statistics + random forest)…"));
    const fetchTop = (key) => api("chart", Object.assign({}, params, { kind: "compare_test", ds: params.a, test: key, range: S.co.range })).then((c) => c.hist);
    mkP.then((mk) => (mk instanceof Error ? showErr(mkBox, mk) : renderMarkers(mkBox, mk, (row) => pick && pick(row), { fetchTop })));
    // box plots of the top tests, A and B side by side per test
    const boxes = h("div");
    const boxKeys = mkP.then((mk) => (mk instanceof Error || !mk.rows ? r.tests.map((t) => t.key) : mk.rows.map((x) => x.key)));
    let shownBoxes = 0;
    const moreBoxes = async (n) => {
      const keys = (await boxKeys).slice(shownBoxes, shownBoxes + n);
      shownBoxes += keys.length;
      const grid = boxes.querySelector(".grid4");
      for (const key of keys) {
        const [card, c] = chartBox();
        grid.append(card);
        api("chart", Object.assign({}, params, { kind: "compare_test", ds: params.a, test: key, range: S.co.range })).then((x) => plot(c, shortBoxes(x.box))).catch((e) => showErr(c, e));
      }
      more.hidden = shownBoxes >= (await boxKeys).length;
    };
    const more = h("button", { onclick: () => moreBoxes(8) }, "Show 8 more");
    boxes.append(h("h2", {}, "Box plots"), h("p", { class: "lead" }, "The top marker tests (or the most severe tests), each with A and B side by side on one value axis. Dashed lines are the limits."), h("div", { class: "grid4" }), more);
    moreBoxes(8);
    out.replaceChildren(chartOpts(() => runCompare(out, params), { hist: true }), mkBox, boxes, h("h2", {}, "Full comparison"), h("div", { class: "kpis" },
      kpi(r.label_a, pct(r.yield_a), `${r.n_a.toLocaleString()} parts`),
      kpi(r.label_b, pct(r.yield_b), `${r.n_b.toLocaleString()} parts`),
      kpi("Yield delta", (r.yield_b - r.yield_a >= 0 ? "+" : "") + (r.yield_b - r.yield_a).toFixed(2) + " pt"),
      kpi("Flagged tests", r.n_flagged, `of ${r.tests.length}`)));
    const g = h("div", { class: "grid2" });
    out.append(g);
    for (const k of ["volcano", "bins"]) { const [card, c] = chartBox(); g.append(card); plot(c, d.figures[k]); }
    const detail = h("div");
    out.append(h("h2", {}, "Per-test results"), h("p", { class: "lead" }, "Sorted by severity. Click a test to overlay the two distributions."));
    const tb = h("div");
    out.append(tb, detail);
    const flagKind = (f) => (f === "OK" ? "good" : f.startsWith("ONLY") ? "neutral" : f.includes("+") ? "bad" : "warn");
    const cols = [
      { key: "flag", label: "Flag", render: (v) => badge(v, flagKind(v)), title: "SHIFT: mean moved ≥ 0.5 σ (Cohen's d) or ≥ 5 % of the limit range; SPREAD: σ ratio ≥ 1.5 or ≤ 1/1.5; both need Welch q < 0.01. ONLY_A / ONLY_B: the test exists in one population only." },
      { key: "key", label: "Test" },
      { key: "units", label: "Units" },
      { key: "mean_a", label: "Mean A", num: true, sortVal: (r) => r.a.mean },
      { key: "mean_b", label: "Mean B", num: true, sortVal: (r) => r.b.mean },
      { key: "shift_sigma", label: "Shift σ", num: true, fmt: (v) => fmt(v, 2), title: "Cohen's d: mean B − mean A in pooled standard deviations" },
      { key: "shift_pct_limit", label: "Shift % lim", num: true, fmt: (v) => (v === null ? "–" : v.toFixed(1)), title: "mean shift as a % of the limit range (hi − lo): how much of the test window it eats" },
      { key: "sigma_ratio", label: "σ ratio", num: true, fmt: (v) => fmt(v, 2), title: "standard deviation B ÷ A" },
      { key: "ks_d", label: "KS D", num: true, fmt: (v) => fmt(v, 3), title: mkTip("ks_d") },
      { key: "q", label: "q", num: true, fmt: fmtP, title: "Welch t-test p-value (unequal variances), Benjamini–Hochberg corrected across all tests" },
      { key: "fail_rate_a", label: "Fail% A", num: true, fmt: (v) => v.toFixed(2) },
      { key: "fail_rate_b", label: "Fail% B", num: true, fmt: (v) => v.toFixed(2) },
    ];
    const rows = r.tests.map((t) => Object.assign({ mean_a: t.a.mean, mean_b: t.b.mean }, t));
    const showTest = async (row, scroll) => {
      loading(detail);
      try {
        const c = await api("chart", Object.assign({}, params, { kind: "compare_test", ds: params.a, test: row.key, range: S.co.range }));
        detail.replaceChildren(h("h2", {}, row.key), chartOpts(() => showTest(row, false), { hist: true }));
        normable(c.hist);
        const g2 = h("div", { class: "grid3" });
        detail.append(g2);
        for (const k of ["hist", "cdf", "box"]) { const [card, cc] = chartBox(); g2.append(card); plot(cc, c[k]); }
        if (scroll) detail.scrollIntoView({ behavior: "smooth", block: "start" });
      } catch (e) { showErr(detail, e); }
    };
    pick = (row) => showTest(row, true);
    table(tb, cols, rows, {
      onClick: (row) => showTest(row, true),
      onClear: () => detail.replaceChildren(),
      tools: h("button", { onclick: () => SC.download("compare.csv", SC.toCSV(cols, rows)) }, "Download CSV"),
    });
    out.append(h("h2", {}, "Site yield"));
    const sy = h("div");
    out.append(sy);
    table(sy, [{ key: "0", label: "Site" }, { key: "1", label: "Yield A", num: true, fmt: (v) => pct(v) }, { key: "2", label: "N A", num: true }, { key: "3", label: "Yield B", num: true, fmt: (v) => pct(v) }, { key: "4", label: "N B", num: true }], r.site_yield.map((x) => ({ 0: x[0], 1: x[1], 2: x[2], 3: x[3], 4: x[4] })), { search: false });
  } catch (e) { showErr(out, e); }
}

// ---- Marker candidates
/** Method notes for the marker table: [name, what it tells you about a test]. */
const MK_HELP = {
  score: ["Candidate score", "The strongest of the ticked sub-scores (separation, tails, PAT, ANOVA, drift, correlation break, residual, random forest), each scaled 0–1. A test ranks high if it is a strong marker in any one view."],
  why: ["Strongest signal", "Which method produced the score."],
  auc: ["Rank AUC (Mann–Whitney)", "Chance that a random affected part reads higher on this test than a random reference part. 0.5 = no difference, 0 or 1 = the groups do not overlap at all. It uses ranks, so outliers and non-normal results do not distort it, and it is fair to very unequal group sizes (e.g. 5,000 vs 250 parts)."],
  separation: ["Separation", "|2·AUC − 1|: 0 = same distribution, 1 = every affected part sits on one side of every reference part."],
  shift_sigma: ["Robust shift", "(median affected − median reference) ÷ robust σ of the reference (1.4826 × MAD). How far the result moved, in units of the reference's normal spread; the sign gives the direction."],
  ks_d: ["Kolmogorov–Smirnov D", "Largest gap between the two cumulative distributions (0–1). Catches shape changes such as a second mode or a stretched tail that a median shift can miss."],
  q: ["Mann–Whitney q", "p-value of the rank test, corrected for testing hundreds of tests at once (Benjamini–Hochberg false-discovery rate). q < 0.05 means the difference is unlikely to be chance."],
  sigma_ratio: ["Spread ratio", "Robust σ affected ÷ robust σ reference. Above 1 = the affected parts scatter more on this test."],
  levene_p: ["Levene test (ANOVA on spread)", "One-way ANOVA on each part's distance from its group median: a small p means the spread really differs, even when the centre does not."],
  tail_ref: ["±3σ share, reference", "Share of reference parts beyond median ± 3 robust σ of the reference (sigma-deviation counting, as used in latent-defect screening)."],
  tail_aff: ["±3σ share, affected", "Share of affected parts beyond the same ±3σ band of the reference. A marker can hide in a few outliers while the bulk looks unchanged."],
  tail_excess: ["Tails (±3σ)", "±3σ share of affected minus that of reference."],
  pat_ref: ["PAT share, reference", "Share of reference parts outside static Part Average Testing limits: median ± 6 robust σ of the reference, never wider than the test limits (AEC-Q001 style)."],
  pat_aff: ["PAT share, affected", "Share of affected parts outside the same PAT limits: parts that pass the test limits but are statistical outliers."],
  pat_excess: ["PAT", "PAT share of affected minus that of reference."],
  anova_p: ["One-way ANOVA p", "F test of the group means (affected vs reference). Assumes roughly normal results; read it next to the rank-based q."],
  eta2: ["ANOVA η²", "Share of the test's variance explained by the group (0–1): how much of the spread is the affected-vs-reference difference."],
  fail_ref: ["Fail %, reference", "Share of reference results outside the test limits."],
  fail_aff: ["Fail %, affected", "Share of affected results outside the test limits."],
  drift_sep: ["Drift separation", "Same units in a second file: the per-unit change (file 2 − file 1) of affected vs reference units, scored like separation."],
  drift_shift_sigma: ["Drift shift", "Median per-unit change of affected minus reference, in robust σ of the reference units' change."],
  r_ref: ["r, reference", "Pearson correlation of file-1 vs file-2 readings for reference units: how repeatable the test is for good units."],
  r_aff: ["r, affected", "The same correlation for affected units (needs ≥ 8 matched units)."],
  corr_break: ["Correlation break", "r reference − r affected: the test stops tracking the same unit between files for the affected parts."],
  resid_sigma: ["Residual", "Fit file 2 vs file 1 on the reference units, then measure how far affected units sit from that line, in robust σ of the reference residuals."],
  resid_abs: ["Residual", "|residual σ|: affected units' distance from the reference file-1 → file-2 fit."],
  importance: ["Random-forest importance", "A class-balanced random forest learns affected vs reference from all tests at once; importance is how much each test contributes to its splits, relative to the top test. Finds markers that only work in combination with other tests."],
};
const mkTip = (k) => (MK_HELP[k] ? `${MK_HELP[k][0]}: ${MK_HELP[k][1]}` : "");
/** How each method becomes a 0–1 sub-score; the candidate score is the largest of the ticked ones. */
const MK_SCORE = [
  ["separation", "Separation", "|2·AUC − 1|, halved when q ≥ 0.05 so chance-level differences rank lower"],
  ["tails", "Tails (±3σ)", "2 × (affected ±3σ share − reference ±3σ share), clipped to 0–1: 50 extra points of outliers = 1"],
  ["PAT", "PAT", "4 × (affected PAT share − reference PAT share), clipped to 0–1: 25 extra points outside PAT = 1"],
  ["ANOVA", "ANOVA", "η = √η², the correlation ratio (share of the test's spread explained by the group, on the same 0–1 footing as separation), halved when the ANOVA p ≥ 0.05. Off by default: it repeats the separation view under a normality assumption"],
  ["drift", "Drift", "drift separation, |2·AUC − 1| of the per-unit change between files"],
  ["correlation break", "Correlation break", "r reference − r affected, clipped to 0–1"],
  ["residual", "Residual", "|residual σ| ÷ 6, clipped to 1: a 6σ offset from the reference fit = 1"],
  ["random forest", "Random forest", "importance × model skill, where skill = 2 × model AUC − 1, so a model that cannot tell the groups apart cannot promote its noise tests"],
];
const MK_UNIT = ["drift", "correlation break", "residual"];
const MK_DEFAULT = MK_SCORE.map((m) => m[0]).filter((m) => m !== "ANOVA");
/** The ticked score methods (shared by Compare and Bad candidates). */
const mkMethods = () => (S.mkMethods && S.mkMethods.length ? S.mkMethods : MK_DEFAULT);
/** Score and strongest signal of one marker row from its sub-scores, over the ticked methods. */
function mkRescore(r, methods) {
  let best = [0, ""];
  for (const [k] of MK_SCORE) {
    const v = r.subs && r.subs[k];
    if (!methods.includes(k) || v === undefined || v === null) continue;
    if (!best[1] || v > best[0]) best = [v, k];
  }
  return Object.assign({}, r, { score: best[0], why: best[1] || "–" });
}
function markerHelp(hasDrift, methods, avail) {
  const li = (name, text) => h("li", {}, h("b", {}, name), " — ", text);
  const note = (k) => (!avail.includes(k) ? (MK_UNIT.includes(k) && !hasDrift ? " (not available here: needs the same units measured in both files)" : " (not available in this run)") : !methods.includes(k) ? " (not ticked: not counted)" : "");
  return h("details", { class: "adv help" },
    h("summary", {}, "ⓘ Methods and how the score is calculated"),
    h("p", { class: "small" }, "Reference = A (or the rest of the file), affected = B (or the group). Every numeric test is scored on its own; hover a column header or tab for its note."),
    h("p", { class: "small" }, h("b", {}, "Score = the largest of the ticked sub-scores"), ` (each 0–1; now: ${methods.map((k) => MK_SCORE.find((m) => m[0] === k)[1]).join(", ")}). Taking the maximum instead of a sum means a test surfaces when it is a clear marker in any one view, and is not boosted for showing the same difference in several. The badge names the sub-score that won. Untick a method to rank without it; the table, chart and badges follow without re-running.`),
    h("ul", { class: "small" }, ...MK_SCORE.filter(([k]) => methods.includes(k)).map(([k, n, t]) => li(n, t + note(k)))),
    MK_SCORE.some(([k]) => !methods.includes(k)) ? h("p", { class: "small muted" }, "Not counted: ", MK_SCORE.filter(([k]) => !methods.includes(k)).map(([k, n]) => n + note(k)).join("; "), ".") : null,
    h("p", { class: "small" }, h("b", {}, "Methods")),
    h("ul", { class: "small" }, ...["auc", "separation", "q", "shift_sigma", "ks_d", "sigma_ratio", "levene_p", "anova_p", "eta2", "tail_aff", "pat_aff", "drift_sep", "corr_break", "resid_sigma", "importance"].map((k) => li(MK_HELP[k][0], MK_HELP[k][1]))),
    h("p", { class: "small muted" }, "Summary tiles: Model AUC is the random forest's hold-out AUC (can all tests together tell the groups apart?). Outlier-count AUC counts, per part, how many tests fall beyond ±3σ of the reference and compares those counts between the groups; it is high when affected parts are marginal across many tests rather than on one."));
}
/** Marks a figure as following the "% of group / counts" histogram switch. */
const normable = (f) => { if (f) f._normable = true; return f; };
const MK_COLOR = { separation: "@s1", tails: "@s2", drift: "@s3", "correlation break": "@s7", residual: "@s4", ANOVA: "@s6" };
/** Bar chart of the top 20 scores, one trace per strongest signal. */
function markerScoreFig(mk, rows) {
  const bars = rows.slice(0, 20);
  const whys = [...new Set(bars.map((r) => r.why))].sort();
  return { data: whys.map((w) => { const sel = bars.filter((r) => r.why === w); return { type: "bar", orientation: "h", name: w, y: sel.map((r) => r.key), x: sel.map((r) => r.score), marker: { color: MK_COLOR[w] || "@s5" }, hovertemplate: "%{y}<br>score %{x:.2f}<extra>%{fullData.name}</extra>" }; }),
    layout: { title: { text: `Top marker candidates (${mk.n_aff} affected vs ${mk.n_ref} reference)`, x: 0.01, xanchor: "left", font: { size: 15 } }, xaxis: { title: { text: "candidate score (strongest ticked signal)" }, range: [0, 1.02] },
      yaxis: { autorange: "reversed", type: "category", categoryorder: "array", categoryarray: bars.map((r) => r.key) }, barmode: "overlay", margin: { l: 190, r: 20, t: 64, b: 50 },
      legend: { orientation: "h", x: 1, xanchor: "right", y: 1.0, yanchor: "bottom" } } };
}
/**
 * Marker ranking with score-method checkboxes. `opts.fetchTop(key)` returns
 * (a promise of) the reference-vs-affected histogram of a test the server did
 * not draw; `opts.onRescore(rows)` hears every re-ranking.
 */
function renderMarkers(box, mk, onPick, opts) {
  opts = opts || {};
  const hasDrift = !!mk.later;
  const p1 = (v) => (v === null || v === undefined ? "–" : (v * 100).toFixed(1));
  const f2 = (v) => (v === null || v === undefined ? "–" : fmt(v, 2));
  const whyKind = (w) => ({ separation: "bad", tails: "warn", PAT: "warn", ANOVA: "warn", drift: "warn", "correlation break": "warn", residual: "warn" })[w] || "neutral";
  const cols = [
    { key: "rank", label: "#", num: true },
    { key: "key", label: "Test" },
    { key: "why", label: "Strongest signal", render: (v) => { const b = badge(v, whyKind(v)); const m = MK_SCORE.find((x) => x[0] === v); b.title = m ? `${m[1]} sub-score: ${m[2]}` : ""; return b; } },
    { key: "score", label: "Score", num: true, fmt: f2 },
    { key: "auc", label: "AUC", num: true, fmt: f2 },
    { key: "shift_sigma", label: "Shift σ", num: true, fmt: f2 },
    { key: "ks_d", label: "KS D", num: true, fmt: f2 },
    { key: "q", label: "q", num: true, fmt: fmtP },
    { key: "sigma_ratio", label: "σ ratio", num: true, fmt: f2 },
    { key: "levene_p", label: "Levene p", num: true, fmt: fmtP },
    { key: "tail_ref", label: "±3σ ref %", num: true, fmt: p1 },
    { key: "tail_aff", label: "±3σ aff %", num: true, fmt: p1 },
    { key: "pat_ref", label: "PAT ref %", num: true, fmt: p1 },
    { key: "pat_aff", label: "PAT aff %", num: true, fmt: p1 },
    { key: "anova_p", label: "ANOVA p", num: true, fmt: fmtP },
    { key: "eta2", label: "η²", num: true, fmt: f2 },
    { key: "fail_ref", label: "Fail ref %", num: true, fmt: p1 },
    { key: "fail_aff", label: "Fail aff %", num: true, fmt: p1 },
  ];
  if (hasDrift) cols.push(
    { key: "drift_sep", label: "Drift sep", num: true, fmt: f2 },
    { key: "drift_shift_sigma", label: "Drift σ", num: true, fmt: f2 },
    { key: "r_ref", label: "r ref", num: true, fmt: f2 },
    { key: "r_aff", label: "r aff", num: true, fmt: f2 },
    { key: "resid_sigma", label: "Resid σ", num: true, fmt: f2 });
  cols.push({ key: "importance", label: "RF imp", num: true, fmt: f2 });
  const views = [["score", "Overall"], ["separation", "Separation"], ["tail_excess", "Tails (±3σ)"], ["pat_excess", "PAT"], ["eta2", "ANOVA η²"]];
  if (hasDrift) views.push(["drift_sep", "Drift between files"], ["corr_break", "Correlation break"], ["resid_abs", "Residual"]);
  views.push(["importance", "Random forest"]);
  for (const c of cols) c.title = mkTip(c.key);
  const base = mk.rows.map((r) => Object.assign({ resid_abs: r.resid_sigma === null ? null : Math.abs(r.resid_sigma) }, r));
  // methods some test has a sub-score for (drift needs a second file, the forest a model)
  const avail = MK_SCORE.map((m) => m[0]).filter((k) => base.some((r) => r.subs && r.subs[k] !== undefined));
  let rows = base;
  const tabs = h("div", { class: "tabs" });
  const tb = h("div");
  const setView = (k) => {
    S.mkView = k;
    tabs.replaceChildren(...views.map(([v, l]) => h("button", { class: v === k ? "active" : "", title: mkTip(v), onclick: () => setView(v) }, l)));
    const sorted = rows.filter((r) => r[k] !== null && r[k] !== undefined).sort((a, b) => b[k] - a[k]).map((r, i) => Object.assign({}, r, { rank: i + 1 }));
    table(tb, cols, sorted, { limit: 10, search: true, placeholder: "Search tests…", onClick: onPick, tools: h("button", { onclick: () => SC.download("marker_tests.csv", SC.toCSV(cols, sorted)) }, "Download CSV") });
  };
  const k = [
    kpi("Affected", mk.n_aff.toLocaleString(), mk.label_aff),
    kpi("Reference", mk.n_ref.toLocaleString(), mk.label_ref),
    kpi("Tests ranked", mk.n_tests.toLocaleString()),
    kpi("Model AUC", mk.forest_auc == null ? "–" : fmt(mk.forest_auc, 3), mk.forest_error || "random forest, hold-out: can all tests together tell the groups apart?"),
    kpi("Outlier-count AUC", fmt(mk.sdc_auc, 3), "tests beyond ±3σ per part"),
  ];
  k[3].title = "Hold-out AUC of a class-balanced random forest trained on all tests: 0.5 = cannot tell the groups apart, 1 = perfect. Also sets how much random-forest importance counts in the score (skill = 2 × AUC − 1).";
  k[4].title = "Per part, count the tests beyond ±3 robust σ of the reference; this AUC compares those counts between affected and reference parts.";
  if (hasDrift) k.push(kpi("Drift file", mk.later, "same units, matched by identity"));
  const methodBox = h("div", { class: "controls" });
  const helpBox = h("div");
  box.replaceChildren(h("h2", {}, "Marker candidates"),
    h("p", { class: "lead" }, "Every test ranked by how well it separates the affected parts from the reference: rank AUC and shift, tails beyond ±3σ of the reference, PAT, ANOVA" + (hasDrift ? ", how the same units changed in the other file (drift, residual, correlation break)" : "") + ", and random-forest importance. The score is the strongest of the ticked signals. Click a test for its distributions."),
    h("div", { class: "kpis" }, ...k), methodBox, helpBox);
  const g = h("div", { class: "grid2" });
  box.append(g);
  const [scoreCard, scoreChart] = chartBox("tall"), [sdcCard, sdcChart] = chartBox("tall");
  g.append(scoreCard, sdcCard);
  plot(sdcChart, normable(mk.figures.sdc));
  const g3 = h("div", { class: "grid3" });
  box.append(h("h3", {}, "Top 10"), tabs, tb, h("h3", {}, "Top candidates: reference vs affected"), g3);
  const serverTop = Object.fromEntries((mk.figures.top_keys || []).map((key, i) => [key, mk.figures.top[i]]));
  const drawTop = () => {
    g3.replaceChildren();
    for (const r of rows.slice(0, mk.figures.top.length || 6)) {
      const [card, c] = chartBox();
      g3.append(card);
      const title = (f) => { f = JSON.parse(JSON.stringify(f)); f.layout = f.layout || {}; f.layout.title = Object.assign({ x: 0.01, xanchor: "left", font: { size: 15 } }, f.layout.title, { text: `${r.key} · ${r.why} (score ${fmt(r.score, 2)})` }); return normable(f); };
      if (serverTop[r.key]) plot(c, title(serverTop[r.key]));
      else if (opts.fetchTop) { c.append(h("div", { class: "loading" }, "Loading…")); Promise.resolve(opts.fetchTop(r.key)).then((f) => { c.replaceChildren(); plot(c, title(f)); }).catch((e) => showErr(c, e)); }
    }
  };
  const rescore = () => {
    const methods = mkMethods().filter((m) => MK_SCORE.some((x) => x[0] === m));
    rows = base.map((r) => mkRescore(r, methods)).sort((a, b) => b.score - a.score);
    methodBox.replaceChildren(h("span", { class: "muted small", style: { paddingBottom: "6px" } }, "Score methods:"), ...MK_SCORE.map(([key, name, text]) => {
      const cb = h("input", { type: "checkbox", checked: methods.includes(key), disabled: !avail.includes(key), onchange: (e) => {
        const next = MK_SCORE.map((m) => m[0]).filter((m) => (m === key ? e.target.checked : methods.includes(m)));
        if (!next.length) { e.target.checked = true; return toast("Keep at least one score method", "bad"); }
        S.mkMethods = next;
        rescore();
      } });
      return h("label", { class: "chk", title: avail.includes(key) ? text : (MK_UNIT.includes(key) ? "Needs the same units measured in a second file" : "Not available in this run") }, cb, name);
    }));
    helpBox.replaceChildren(markerHelp(hasDrift, methods, avail));
    plot(scoreChart, markerScoreFig(mk, rows));
    setView(views.some(([v]) => v === S.mkView) ? S.mkView : "score");
    drawTop();
    if (opts.onRescore) opts.onRescore(rows);
  };
  rescore();
}

// ---- Correlation
async function viewCorrelation(el) {
  el.append(h("h1", {}, "Correlation"));
  const tabs = h("div", { class: "tabs" });
  const body = h("div");
  el.append(tabs, body);
  const modes = [["unit", "Unit correlation (tester ↔ tester)"], ["matrix", "Test ↔ test matrix"], ["target", "Drivers of a target"]];
  const cur = S.corrMode || (S.datasets.length > 1 ? "unit" : "matrix");
  const setMode = (m) => { S.corrMode = m; tabs.replaceChildren(...modes.map(([k, l]) => h("button", { class: k === m ? "active" : "", onclick: () => setMode(k) }, l))); body.replaceChildren(); ({ unit: corrUnit, matrix: corrMatrix, target: corrTarget })[m](body); };
  setMode(cur);
}
function corrUnit(el) {
  el.append(h("p", { class: "lead" }, "The same physical parts tested in two files (e.g. golden units on two testers, or a re-test) are paired by the IDs picked for each file, as on Match parts. Each test is CORRELATED, OFFSET (good r but mean offset beyond tolerance) or UNCORRELATED."));
  const a = select(dsOptions(), String(S.ucA ?? S.ds), (v) => { S.ucA = +v; ka.forDs(S.ucA); });
  const dA = S.datasets.find((d) => d.id === (S.ucA ?? S.ds));
  const twin = S.datasets.find((d) => d.id !== dA.id && d.files === 1 && dA.wafers.length && d.wafers.some((w) => dA.wafers.includes(w)));
  const b = select(dsOptions(), String(S.ucB ?? (twin ? twin.id : otherDs(dA.id))), (v) => { S.ucB = +v; kb.forDs(S.ucB); });
  // the same ID pickers as Match parts, so both pages pair the same units
  const ka = keyPicker("uc_a", +a.value), kb = keyPicker("uc_b", +b.value);
  const rmin = h("input", { type: "number", step: "0.01", value: 0.9 });
  const off = h("input", { type: "number", step: "0.5", value: 5 });
  const out = h("div");
  el.append(h("div", { class: "controls" }, field("File A", a), field("Identify A parts by", ka.el), field("File B", b), field("Identify B parts by", kb.el), field("min r", rmin), field("max offset % limit", off), h("button", { class: "primary", onclick: run }, "Correlate")), out);
  async function run() {
    loading(out);
    const params = { a: +a.value, b: +b.value, key_a: ka.value(), key_b: kb.value(), r_min: +rmin.value, offset_pct: +off.value, filter_a: scoped(""), filter_b: scoped("") };
    try {
      const d = await api("unitcorr", params);
      const r = d.report;
      out.replaceChildren(h("div", { class: "kpis" }, kpi("Matched parts", r.matched.toLocaleString(), `by ${r.match_by}`), kpi("Only in A / B", `${r.only_a} / ${r.only_b}`), kpi("Bin agreement", pct(r.bin_agreement_pct, 1)),
        ...Object.entries(r.summary).filter(([, v]) => v > 0).map(([k, v]) => kpi(k.replace("_", " ").toLowerCase(), v))));
      if (!r.matched) { out.append(h("div", { class: "err" }, "No parts matched. Unit correlation needs the same physical parts in both files, with the same ID in each. Pick another ID (the list shows how many parts of each file have it), or files from the same lot.")); return; }
      const [card, c] = chartBox("tall");
      out.append(card);
      if (d.first) plot(c, d.first);
      const tb = h("div");
      out.append(h("h2", {}, "Per-test correlation"), tb);
      const kind = (s) => (s === "CORRELATED" ? "good" : s === "OFFSET" ? "warn" : s === "UNCORRELATED" ? "bad" : "neutral");
      const cols = [{ key: "status", label: "Status", render: (v) => badge(v, kind(v)) }, { key: "key", label: "Test" }, { key: "n", label: "N", num: true }, { key: "r", label: "r", num: true, fmt: (v) => fmt(v, 4) }, { key: "slope", label: "Slope", num: true, fmt: (v) => fmt(v, 3) }, { key: "mean_diff", label: "Mean B−A", num: true }, { key: "diff_pct_limit", label: "Diff % lim", num: true, fmt: (v) => (v === null ? "–" : v.toFixed(2)) }, { key: "sd_diff", label: "σ diff", num: true }, { key: "max_abs_diff", label: "Max |diff|", num: true }];
      table(tb, cols, r.tests, { onClick: async (row) => { const s = await api("chart", Object.assign({ kind: "unitcorr", ds: params.a, test: row.key }, params)); plot(c, s.scatter); c.scrollIntoView({ behavior: "smooth", block: "center" }); }, tools: h("button", { onclick: () => SC.download("unit_correlation.csv", SC.toCSV(cols, r.tests)) }, "Download CSV") });
      if (r.bin_matrix.length) {
        const bm = h("div");
        out.append(h("h2", {}, "Hard-bin agreement (A → B)"), bm);
        table(bm, [{ key: "a", label: "Bin A", num: true }, { key: "b", label: "Bin B", num: true }, { key: "n", label: "Parts", num: true }, { key: "same", label: "", render: (v) => (v ? badge("match", "good") : badge("mismatch", "bad")) }], r.bin_matrix.map(([x, y, n]) => ({ a: x, b: y, n, same: x === y })), { search: false, sort: "n" });
      }
    } catch (e) { showErr(out, e); }
  }
  if (S.datasets.length > 1) run();
}
function corrMatrix(el) {
  el.append(h("p", { class: "lead" }, "Pairwise-complete correlation between tests. Tests are auto-picked by relative spread (or list your own). Strong pairs flag redundant tests or shared physical causes; click a pair for its scatter."));
  const method = select([["pearson", "Pearson"], ["spearman", "Spearman (rank)"]], "pearson");
  const maxT = h("input", { type: "number", value: 60, min: 2 });
  const thr = h("input", { type: "number", value: 0.7, step: 0.05 });
  const list = h("input", { type: "text", placeholder: "optional: comma-separated tests", style: { minWidth: "260px" } });
  const out = h("div");
  el.append(h("div", { class: "controls" }, field("Method", method), field("Max tests", maxT), field("|r| ≥", thr), field("Tests", list), h("button", { class: "primary", onclick: run }, "Run")), out);
  async function run() {
    loading(out);
    try {
      const tests = list.value.split(",").map((s) => s.trim()).filter(Boolean);
      const d = await api("correlation", P({ method: method.value, max_tests: +maxT.value, threshold: +thr.value, tests }));
      out.replaceChildren();
      const [card, c] = chartBox("tall");
      out.append(card);
      plot(c, d.figure);
      const sc = h("div");
      const tb = h("div");
      out.append(h("h2", {}, `Strong pairs (|r| ≥ ${thr.value})`), tb, sc);
      table(tb, [{ key: "a", label: "Test A" }, { key: "b", label: "Test B" }, { key: "r", label: "r", num: true, fmt: (v) => fmt(v, 4) }, { key: "n", label: "N", num: true }], d.pairs, {
        onClick: async (row) => { const s = await api("chart", P({ kind: "scatter", test: row.a, test2: row.b, group_by: "site" })); const [cd, cc] = chartBox("tall"); sc.replaceChildren(cd); plot(cc, s.scatter); },
        onClear: () => sc.replaceChildren(),
      });
    } catch (e) { showErr(out, e); }
  }
  run();
}
function corrTarget(el) {
  el.append(h("p", { class: "lead" }, "Rank every test by correlation with a target: fail (point-biserial) or another test. A fast first look at what drives yield loss or a key parameter; see Machine learning for a multivariate model."));
  const tgt = testInput("tgt", "", "pass  (or a test)");
  tgt.value = "pass";
  const method = select([["spearman", "Spearman"], ["pearson", "Pearson"]], "spearman");
  const out = h("div");
  el.append(h("div", { class: "controls" }, field("Target", tgt), field("Method", method), h("button", { class: "primary", onclick: run }, "Run")), out);
  async function run() {
    loading(out);
    try {
      const d = await api("target_corr", P({ target: tgt.value || "pass", method: method.value }));
      const [card, c] = chartBox("tall");
      const tb = h("div");
      out.replaceChildren(card, h("h2", {}, "All tests"), tb);
      plot(c, d.figure);
      table(tb, [{ key: "key", label: "Test" }, { key: "r", label: "r", num: true, fmt: (v) => fmt(v, 4) }, { key: "p", label: "p", num: true, fmt: fmtP }, { key: "n", label: "N", num: true }], d.rows, { onClick: (r) => { S.explorerTest = r.key; go("explorer"); } });
    } catch (e) { showErr(out, e); }
  }
  run();
}

// ---- ANOVA
async function viewAnova(el) {
  el.append(h("h1", {}, "ANOVA"), h("p", { class: "lead" }, "One-way ANOVA of every test against a factor, with Kruskal–Wallis (non-parametric) and Brown–Forsythe (equal-variance) checks and Benjamini–Hochberg q-values. η² is the share of variance the factor explains. Use 'file / lot' on merged files for lot-to-lot, or site for site-to-site."));
  const factorSel = select(GROUPS.filter(([k]) => k && k !== "pass").concat([["x", "die X"], ["y", "die Y"], ["test:", "a test's value…"]]), S.anovaFactor || "site", (v) => { S.anovaFactor = v; ftest.hidden = v !== "test:"; });
  const ftest = h("input", { type: "text", value: S.anovaFactorTest || "5100", placeholder: "test #", style: { width: "80px" }, hidden: (S.anovaFactor || "site") !== "test:", title: "Group parts by this test's value (e.g. a datalogged board, socket or trim code)", oninput: (e) => (S.anovaFactorTest = e.target.value.trim()) });
  const factor = { get value() { return factorSel.value === "test:" ? "test:" + ftest.value.trim() : factorSel.value; } };
  const ming = h("input", { type: "number", value: 5, min: 2 });
  const out = h("div");
  el.append(h("div", { class: "controls" }, field("Factor", h("div", { class: "row tight" }, factorSel, ftest)), field("Min parts / group", ming), loadboardField(), h("button", { class: "primary", onclick: run }, "Run ANOVA")), out);
  async function run() {
    loading(out);
    try {
      const d = await api("anova", P({ factor: factor.value, min_group: +ming.value }));
      out.replaceChildren(h("div", { class: "kpis" }, kpi("Groups", d.groups.length, d.groups.slice(0, 6).map((g) => `${g[0]} (${g[1]})`).join(", ")), kpi("Significant tests", d.n_significant, `q < 0.01 of ${d.rows.length}`)));
      const g = h("div", { class: "grid2" });
      const [c1, p1] = chartBox("tall");
      const right = h("div", { class: "card" });
      const p2 = h("div", { class: "chart" }), p3 = h("div", { class: "chart" });
      right.append(p2, p3);
      g.append(c1, right);
      out.append(g);
      plot(p1, d.figure);
      if (d.first) plot(p2, d.first);
      const tb = h("div");
      out.append(h("h2", {}, "All tests"), tb);
      const show = async (row) => { const c = await api("chart", P({ kind: "anova_means", test: row.key, factor: factor.value, min_group: +ming.value })); plot(p2, c.means); plot(p3, c.box); };
      if (d.rows[0]) show(d.rows[0]);
      const cols = [{ key: "key", label: "Test" }, { key: "eta2", label: "η²", num: true, fmt: (v) => fmt(v, 4) }, { key: "f", label: "F", num: true, fmt: (v) => fmt(v, 2) }, { key: "p", label: "p", num: true, fmt: fmtP }, { key: "q", label: "q (BH)", num: true, fmt: fmtP }, { key: "kw_p", label: "Kruskal p", num: true, fmt: fmtP }, { key: "levene_p", label: "Levene p", num: true, fmt: fmtP }, { key: "range_pct_limit", label: "Mean range % lim", num: true, fmt: (v) => (v === null ? "–" : v.toFixed(1)) }, { key: "n", label: "N", num: true }];
      table(tb, cols, d.rows, { onClick: show, tools: h("button", { onclick: () => SC.download("anova.csv", SC.toCSV(cols, d.rows)) }, "Download CSV") });
    } catch (e) { showErr(out, e); }
  }
  run();
}

// ---- Anomaly
async function viewAnomaly(el) {
  el.append(h("h1", {}, "Abnormal parts"), h("p", { class: "lead" }, "Outlier screening that combines univariate robust z-scores, static PAT limits (AEC-Q001 style, passing parts only), wafer nearest-neighbour residuals (NNR), multivariate Mahalanobis distance and an Isolation Forest model."));
  const rk = h("input", { type: "number", value: 6, step: 0.5 });
  const pk = h("input", { type: "number", value: 6, step: 0.5 });
  const nk = h("input", { type: "number", value: 5, step: 0.5 });
  const it = h("input", { type: "number", value: 0.62, step: 0.01 });
  const meth = {};
  const chks = ["robust", "pat", "nnr", "mahalanobis", "iforest"].map((m) => { meth[m] = h("input", { type: "checkbox", checked: true }); return h("label", { class: "chk" }, meth[m], m); });
  const onlyPass = h("input", { type: "checkbox" });
  const out = h("div");
  el.append(h("div", { class: "controls" }, field("robust k", rk), field("PAT k", pk), field("NNR k", nk), field("iForest ≥", it), ...chks, h("label", { class: "chk" }, onlyPass, "passing parts only"), h("button", { class: "primary", onclick: run }, "Detect")), out);
  async function run(wafer) {
    loading(out);
    try {
      const methods = Object.keys(meth).filter((k) => meth[k].checked).join(",");
      const params = P({ robust_k: +rk.value, pat_k: +pk.value, nnr_k: +nk.value, iforest_threshold: +it.value, methods, only_passing: onlyPass.checked });
      if (typeof wafer === "number") params.wafer = wafer;
      const d = await api("anomaly", params);
      out.replaceChildren(h("div", { class: "kpis" }, kpi("Flagged parts", d.n_flagged, `of ${d.n_parts.toLocaleString()}`), kpi("Tests analysed", d.n_tests), ...Object.entries(d.by_method).map(([k, v]) => kpi(k, v))));
      const g = h("div", { class: "grid2" });
      out.append(g);
      const [c1, p1] = chartBox("tall");
      g.append(c1);
      plot(p1, d.figures.scores);
      if (d.figures.wafer) {
        const [c2, p2] = chartBox("tall");
        g.append(c2);
        plot(p2, d.figures.wafer);
        const ds = S.datasets.find((x) => x.id === S.ds);
        if (ds && ds.wafers.length > 1) c2.prepend(h("div", { style: { padding: "4px 8px" } }, select(ds.wafers.map((w, i) => [String(i), w]), String(params.wafer ?? ""), (v) => run(+v))));
      }
      const detail = h("div");
      const tb = h("div");
      out.append(h("h2", {}, "Flagged parts"), tb, detail);
      const cols = [{ key: "part_id", label: "Part" }, { key: "wafer", label: "Wafer" }, { key: "x", label: "X", num: true }, { key: "y", label: "Y", num: true }, { key: "site", label: "Site", num: true }, { key: "hbin", label: "HBin", num: true }, { key: "pass", label: "Result", render: (v) => (v ? badge("pass", "good") : badge("fail", "bad")) }, { key: "methods", label: "Methods", fmt: (v) => v.join(", ") }, { key: "score", label: "Score", num: true, fmt: (v) => fmt(v, 2) }, { key: "max_robust_z", label: "max |z|", num: true, fmt: (v) => fmt(v, 1) }, { key: "max_nnr_z", label: "max NNR z", num: true, fmt: (v) => fmt(v, 1) }, { key: "iforest", label: "iForest", num: true, fmt: (v) => fmt(v, 3) }, { key: "mahal_p", label: "Mahal p", num: true, fmt: fmtP }, { key: "top", label: "Top test", sortVal: (r) => r.top_tests[0] && r.top_tests[0][0] }];
      const rows = d.parts.map((p) => Object.assign({ top: p.top_tests[0] ? `${p.top_tests[0][0]} (z=${p.top_tests[0][1].toFixed(1)})` : "" }, p));
      table(tb, cols, rows, {
        onClick: (row) => partDetail(detail, S.ds, row.row, `Part ${row.part_id} — all tests`),
        onClear: () => detail.replaceChildren(),
        tools: [h("button", { onclick: () => SC.download("anomalies.csv", SC.toCSV(cols, rows)) }, "Download CSV"), h("button", { onclick: () => { document.getElementById("filter").value = rows.length ? `index in (${rows.map((r) => r.row).join(",")})` : ""; } }, "Use as filter")],
      });
      if (d.pat.length) {
        const pt = h("div");
        out.append(h("h2", {}, "PAT limits with passing parts outside"), pt);
        table(pt, [{ key: "key", label: "Test" }, { key: "n_outside", label: "Passing parts outside", num: true }, { key: "pat_lo", label: "PAT lo", num: true }, { key: "pat_hi", label: "PAT hi", num: true }, { key: "lo", label: "Spec lo", num: true }, { key: "hi", label: "Spec hi", num: true }, { key: "center", label: "Robust mean", num: true }, { key: "robust_sigma", label: "Robust σ", num: true }], d.pat, { sort: "n_outside" });
      }
    } catch (e) { showErr(out, e); }
  }
  run();
}

// ---- Machine learning
async function viewML(el) {
  el.append(h("h1", {}, "Machine learning"));
  const tabs = h("div", { class: "tabs" });
  const body = h("div");
  el.append(tabs, body);
  const modes = [["explain", "Random forest: what drives…"], ["pca", "PCA + clustering"]];
  const setMode = (m) => { S.mlMode = m; tabs.replaceChildren(...modes.map(([k, l]) => h("button", { class: k === m ? "active" : "", onclick: () => setMode(k) }, l))); body.replaceChildren(); (m === "pca" ? mlPca : mlExplain)(body); };
  setMode(S.mlMode || "explain");
}
function mlExplain(el) {
  el.append(h("p", { class: "lead" }, "Trains a random forest (class-balanced, 25% hold-out) to predict pass/fail, hard bin, soft bin, site — or any test value (regression) — from all parametric tests. Impurity importance shows which tests carry the signal."));
  const tgt = testInput("mlt", "pass", "pass | hbin | sbin | site | a test");
  const trees = h("input", { type: "number", value: 100, min: 10 });
  const depth = h("input", { type: "number", value: 12, min: 2 });
  const out = h("div");
  el.append(h("div", { class: "controls" }, field("Target", tgt), field("Trees", trees), field("Max depth", depth), h("button", { class: "primary", onclick: run }, "Train")), out);
  run();
  async function run() {
    loading(out, "Training…");
    try {
      const d = await api("explain", P({ target: tgt.value || "pass", trees: +trees.value, depth: +depth.value }));
      const r = d.report;
      const k = r.kind === "classification" ? [kpi("ROC AUC", fmt(r.auc, 3), r.class_names.length > 2 ? "macro one-vs-rest" : "hold-out, threshold-free"), kpi("Accuracy", pct(r.accuracy * 100, 1), "hold-out"), kpi("Balanced accuracy", pct(r.balanced_accuracy * 100, 1), r.threshold != null ? `at P ≥ ${r.threshold.toFixed(3)} (out-of-bag)` : "mean class recall"), kpi("Classes", r.class_names.length, r.class_names.map((c, i) => `${c}: ${r.class_counts[i]}`).slice(0, 4).join(", "))] : [kpi("R²", fmt(r.r2, 3), "hold-out"), kpi("RMSE", fmt(r.rmse)), kpi("MAE", fmt(r.mae))];
      out.replaceChildren(h("div", { class: "kpis" }, ...k, kpi("Features", r.n_features), kpi("Train / test", `${r.n_train} / ${r.n_test}`)));
      const g = h("div", { class: "grid2" });
      out.append(g);
      const [c1, p1] = chartBox("tall"); const [c2, p2] = chartBox("tall");
      g.append(c1, c2);
      plot(p1, d.figures.importance); plot(p2, d.figures.eval);
      const tb = h("div");
      out.append(h("h2", {}, "Feature importance"), tb);
      table(tb, [{ key: "0", label: "Test" }, { key: "1", label: "Importance %", num: true, fmt: (v) => (v * 100).toFixed(2) }], r.importance.map((x) => ({ 0: x[0], 1: x[1] })), { onClick: (row) => { S.explorerTest = row[0]; go("explorer"); } });
    } catch (e) { showErr(out, e); }
  }
}
function mlPca(el) {
  el.append(h("p", { class: "lead" }, "Principal components of the robust-standardized test matrix, with k-means++ clustering (k chosen by silhouette when 'auto'). Separate clusters often reveal sub-populations: a site, a wafer region, a process split."));
  const color = select([["pass", "pass/fail"], ["cluster", "cluster"], ["site", "site"], ["hbin", "hard bin"], ["wafer", "wafer"], ["file", "file / lot"]], "cluster");
  const k = h("input", { type: "text", value: "auto", style: { width: "70px" } });
  const mf = h("input", { type: "number", value: 150 });
  const out = h("div");
  el.append(h("div", { class: "controls" }, field("Color by", color), field("Clusters", k), field("Max features", mf), h("button", { class: "primary", onclick: run }, "Run PCA")), out);
  async function run() {
    loading(out);
    try {
      const d = await api("pca", P({ color_by: color.value, clusters: k.value, max_features: +mf.value }));
      const cl = d.clusters;
      out.replaceChildren(h("div", { class: "kpis" }, kpi("Features", d.n_features), kpi("PC1 + PC2", pct((d.explained[0] + (d.explained[1] || 0)) * 100, 1), "variance explained"), cl ? kpi("Clusters", cl.k, `silhouette ${cl.silhouette.toFixed(3)}`) : null, cl ? kpi("Cluster sizes", cl.sizes.join(" / ")) : null));
      const g = h("div", { class: "grid2" });
      out.append(g);
      const [c1, p1] = chartBox("tall"); const [c2, p2] = chartBox("tall");
      g.append(c1, c2);
      plot(p1, d.figures.scatter); plot(p2, d.figures.scree);
      out.append(h("h2", {}, "Top loadings"));
      const lg = h("div", { class: "grid3" });
      out.append(lg);
      for (const l of d.loadings) { const t = h("div", { class: "card pad" }, h("b", {}, `PC${l.pc}`)); const tt = h("div"); t.append(tt); lg.append(t); table(tt, [{ key: "0", label: "Test" }, { key: "1", label: "Loading", num: true, fmt: (v) => v.toFixed(3) }], l.top.map((x) => ({ 0: x[0], 1: x[1] })), { search: false }); }
    } catch (e) { showErr(out, e); }
  }
  run();
}

// ---- Records
function viewRecords(el) {
  tabbedPage(el, "records", "Records", "Every STDF record of the current dataset, by record type. PRR opens as the part table (each part with its first failing test; click one for its results), or as its raw fields. The part datalog writes the results of the parts and tests you pick.", [
    ["raw", "Records", recRaw],
    ["datalog", "Part datalog", viewDatalog],
  ]);
}

const NO_XY = -32768;
function partSummary(p, wafer) {
  return [p.part_id && `part ${p.part_id}`, p.part_txt && `text ${p.part_txt}`, wafer && `wafer ${wafer}`, p.x !== NO_XY && p.x !== null && `X ${p.x} Y ${p.y}`, `site ${p.site}`, `hbin ${p.hbin}`, `sbin ${p.sbin}`, p.passed === true || p.pass === true ? "good" : p.passed === false || p.pass === false ? "bad" : null].filter(Boolean).join(" · ");
}
const TEST_COLS = [{ key: "key", label: "Test" }, { key: "value", label: "Value", num: true }, { key: "units", label: "Units" }, { key: "lo", label: "Lo", num: true }, { key: "hi", label: "Hi", num: true }, { key: "robust_z", label: "Robust z", num: true, fmt: (v) => fmt(v, 2) }, { key: "pass", label: "", render: (v) => (v ? badge("in", "good") : badge("out", "bad")) }];
/** All test results of one part, with its raw records on demand. */
/** "× Close" that empties a detail box (and runs `after`, e.g. clearing a highlight). */
function closeBtn(box, after) {
  return h("button", { class: "ghost small", title: "Close", onclick: () => { box.replaceChildren(); if (after) after(); } }, "× Close");
}
async function partDetail(detail, ds, index, title) {
  loading(detail);
  try {
    const pd = await api("part_detail", { ds, index });
    const raw = h("div");
    const t = h("div");
    detail.replaceChildren(h("div", { class: "row tight", style: { justifyContent: "space-between" } }, h("h2", {}, title || `Part ${pd.part.part_id}`), closeBtn(detail)), h("p", { class: "muted" }, partSummary(pd.part, pd.wafer)),
      h("div", { class: "row" }, h("button", { onclick: () => partRaw(raw, ds, index) }, "Show raw records (PIR … PRR)")), t, raw);
    table(t, TEST_COLS, pd.tests);
    detail.scrollIntoView({ behavior: "smooth", block: "start" });
  } catch (e) { showErr(detail, e); }
}
function cellVal(v) {
  if (v === null || v === undefined) return "";
  if (typeof v === "object") { const s = JSON.stringify(v); return s.length > 90 ? h("span", { title: s }, s.slice(0, 88) + "…") : s; }
  return typeof v === "number" ? fmt(v) : String(v);
}
/** Raw records of one part, one row per record. */
async function partRaw(el, ds, index) {
  loading(el, "Reading the file…");
  try {
    const d = await api("part_records", { ds, index });
    const mainKeys = ["_rec", "test_num", "test_txt", "result", "test_flg", "lo_limit", "hi_limit", "units"];
    const rows = d.records.map((r, i) => {
      const rest = Object.fromEntries(Object.entries(r).filter(([k, v]) => !mainKeys.includes(k) && k !== "_offset" && v !== null && !(Array.isArray(v) && !v.length) && v !== ""));
      return Object.assign({ i: i + 1, other: rest }, r);
    });
    const tb = h("div");
    el.replaceChildren(h("h2", {}, `Raw records (${rows.length})`), h("p", { class: "muted small" }, d.path), tb);
    table(tb, [{ key: "i", label: "#", num: true }, { key: "_rec", label: "Record" }, { key: "test_num", label: "Test", num: true }, { key: "test_txt", label: "Name" }, { key: "result", label: "Result", num: true, render: cellVal }, { key: "test_flg", label: "TEST_FLG", num: true }, { key: "lo_limit", label: "LO", num: true, render: cellVal }, { key: "hi_limit", label: "HI", num: true, render: cellVal }, { key: "units", label: "Units", render: cellVal }, { key: "other", label: "Other fields", render: cellVal }], rows, { limit: 2000 });
  } catch (e) { showErr(el, e); }
}

async function recParts(el) {
  el.append(h("div", { class: "controls" },
    h("button", { onclick: () => { location.href = `/api/export.csv?ds=${S.ds}&filter=${encodeURIComponent(scoped(S.filter))}&t=${encodeURIComponent(window.STDF_TOKEN)}`; } }, "Download CSV (all tests)"),
    h("button", { onclick: () => go("trim") }, "Edit & trim…"),
    h("button", { onclick: () => go("export") }, "Split, extract or save report…")), scopeLine("The part table and its CSV list"));
  const out = h("div");
  el.append(out);
  loading(out);
  try {
    const d = await api("parts", P({ limit: 20000 }));
    const est = touchOn() ? await touchEstimate(S.ds) : null;
    if (est) d.rows.forEach((r) => { r.touchdown = est.touch[r.index]; r.loop = est.loop[r.index]; });
    const detail = h("div");
    const tb = h("div");
    out.replaceChildren(h("p", { class: "muted" }, `${d.total.toLocaleString()} parts` + (d.total > d.rows.length ? ` (first ${d.rows.length} listed)` : "")), ...(est ? [touchCard(est)] : []), tb, detail);
    const hasTxt = d.rows.some((r) => r.part_txt);
    table(tb, [{ key: "index", label: "#", num: true }, { key: "part_id", label: "Part ID" }, hasTxt ? { key: "part_txt", label: "Part text" } : null, { key: "wafer", label: "Wafer" }, { key: "x", label: "X", num: true }, { key: "y", label: "Y", num: true }, { key: "head", label: "Head", num: true }, { key: "site", label: "Site", num: true }, { key: "hbin", label: "HBin", num: true }, { key: "sbin", label: "SBin", num: true }, { key: "pass", label: "Result", render: (v) => (v === null ? badge("n/a", "neutral") : v ? badge("good", "good") : badge("bad", "bad")) }, { key: "alarm", label: "Alarm", render: (v) => (v ? badge("alarm", "warn") : "") }, { key: "first_fail", label: "First fail" }, { key: "test_t", label: "Test time ms", num: true }, ...(est ? [{ key: "touchdown", label: "Touchdown", num: true }, { key: "loop", label: "Loop", num: true }] : [])].filter(Boolean), d.rows, {
      onClick: (row) => partDetail(detail, S.ds, row.index),
      onClear: () => detail.replaceChildren(),
    });
  } catch (e) { showErr(out, e); }
}

const TEST_RECS = ["PTR", "MPR", "FTR", "TSR"];
function recRaw(el) {
  const ds = S.datasets.find((d) => d.id === S.ds);
  if (!S.rec || S.rec.ds !== S.ds) S.rec = { ds: S.ds, file: 0, type: "PRR", test: "", offset: 0, limit: 200, prr: S.rec ? S.rec.prr : "parts" };
  const st = S.rec;
  const ctl = h("div", { class: "controls" });
  const pager = h("div", { class: "row" });
  const out = h("div");
  const detail = h("div");
  el.append(ctl, pager, out, detail);
  const typeSel = h("select", { onchange: (e) => { st.type = e.target.value; st.offset = 0; load(); } });
  const fileSel = h("select", { onchange: (e) => { st.file = +e.target.value; st.offset = 0; load(); } });
  const test = testInput("rt", st.test, "all tests");
  test.style.minWidth = "160px";
  test.addEventListener("change", () => { st.test = test.value; st.offset = 0; load(); });
  const size = select([["100", "100"], ["200", "200"], ["500", "500"], ["1000", "1000"]], String(st.limit), (v) => { st.limit = +v; st.offset = 0; load(); });
  const testField = field("Test number", test);
  // PRR as the part table (first fail, alarms, part filter, click for results) or as raw fields
  const prrView = select([["parts", "part table"], ["raw", "raw fields"]], st.prr || "parts", (v) => { st.prr = v; load(); });
  const prrField = field("PRR as", prrView);
  const fileField = ds && ds.paths.length > 1 ? field("File", fileSel) : "";
  const sizeField = field("Rows per page", size);
  ctl.append(fileField, field("Record type", typeSel), prrField, testField, sizeField);
  const wholeNote = scopeLine("Raw records", true);
  el.insertBefore(wholeNote, pager);
  async function load() {
    loading(out, "Reading records…");
    testField.hidden = !TEST_RECS.includes(st.type);
    prrField.hidden = st.type !== "PRR";
    const asParts = st.type === "PRR" && (st.prr || "parts") === "parts";
    sizeField.hidden = asParts;
    wholeNote.hidden = asParts;
    if (fileField) fileField.hidden = asParts;
    try {
      if (asParts) {
        // the record types and their counts, then the part table
        const c = await api("records", { ds: S.ds, file: st.file, rec: "PRR", test: null, offset: 0, limit: 1 });
        typeSel.replaceChildren(...Object.entries(c.counts).map(([k, n]) => h("option", { value: k, selected: k === st.type }, `${k} (${n.toLocaleString()})`)));
        pager.replaceChildren();
        detail.replaceChildren();
        out.replaceChildren();
        await recParts(out);
        return;
      }
      const tn = parseInt(String(st.test).trim(), 10);
      const d = await api("records", { ds: S.ds, file: st.file, rec: st.type, test: TEST_RECS.includes(st.type) && isFinite(tn) ? tn : null, offset: st.offset, limit: st.limit });
      const types = Object.entries(d.counts);
      if (!types.some(([k]) => k === st.type)) types.push([st.type, 0]);
      typeSel.replaceChildren(...types.map(([k, n]) => h("option", { value: k, selected: k === st.type }, `${k} (${n.toLocaleString()})`)));
      fileSel.replaceChildren(...d.files.map((p, i) => h("option", { value: i, selected: i === st.file }, p.split(/[\\/]/).pop())));
      const total = d.total, from = Math.min(st.offset + 1, total), to = st.offset + d.rows.length;
      const jump = h("input", { type: "number", min: 1, value: from, style: { width: "100px" }, onchange: (e) => { st.offset = Math.max(0, Math.min(total - 1, +e.target.value - 1)); load(); } });
      pager.replaceChildren(
        h("button", { disabled: st.offset <= 0, onclick: () => { st.offset = Math.max(0, st.offset - st.limit); load(); } }, "◀ Previous"),
        h("button", { disabled: to >= total, onclick: () => { st.offset += st.limit; load(); } }, "Next ▶"),
        h("span", { class: "muted" }, total ? `${st.type} records ${from.toLocaleString()}–${to.toLocaleString()} of ${total.toLocaleString()}` : `No ${st.type} records` + (st.test ? ` for test ${st.test}` : "")),
        total > st.limit ? h("label", { class: "row tight muted" }, "go to #", jump) : null);
      const keys = [];
      for (const r of d.rows) for (const k of Object.keys(r)) if (!keys.includes(k)) keys.push(k);
      const order = ["_n", ...keys.filter((k) => !k.startsWith("_")), ...keys.filter((k) => k.startsWith("_") && k !== "_n")];
      const label = (k) => ({ _n: "#", _offset: "byte offset", _part: "dataset row" })[k] || k;
      const cols = order.map((k) => { const v0 = d.rows.map((r) => r[k]).find((v) => v !== null && v !== undefined); return { key: k, label: label(k), num: typeof v0 === "number", render: k === "_n" ? (v) => String(v + 1) : cellVal }; });
      table(out, cols, d.rows, {
        limit: st.limit,
        placeholder: "Search this page…",
        onClick: (row) => {
          const box = h("div");
          detail.replaceChildren(h("h2", {}, `${st.type} record #${row._n + 1}`), row._part !== undefined ? h("div", { class: "row" }, h("button", { class: "primary", onclick: () => partDetail(box, S.ds, row._part) }, "Show this part's test results")) : null, h("pre", { class: "json" }, JSON.stringify(row, null, 2)), box);
          detail.scrollIntoView({ behavior: "smooth", block: "start" });
        },
      });
    } catch (e) { showErr(out, e); }
  }
  load();
}

// ---- Match & trace
/** Every ID, in the order set in Settings. */
function idOrderAll() { return [...new Set([...(S.cfg.idOrder || []).filter((k) => ID_ORDER.includes(k)), ...ID_ORDER])]; }
/** The IDs in the order set in Settings: [id, API key, label], leaving out those with no test set or with trace off. */
function idChoices() {
  const t = idTests(true), num = (v) => String(v || "").trim(), wxy = num(t.wxy), wt = [t.w, t.x_t, t.y_t].map(num);
  // on the die basis the ID holds die X and Y: the X and Y tests with their offsets applied
  const off = wxyBasis() === "die" && num(t.ox) && num(t.oy) ? [t.ox, t.oy].map(num) : [];
  const wk = [...wt, ...off];
  const all = {
    part_txt: ["part_txt", "PRR part text (ULT)"],
    wxy: wxy ? [`tests:${wxy}`, `PTR WXY (test ${wxy})`] : null,
    w_x_y: wt.every(Boolean) ? [`w_x_y${off.length && waferCal().mode === "sub" ? "_sub" : ""}:${wk.join(",")}`, `PTR wafer, X, Y${off.length ? " with offsets" : ""} (tests ${wk.join(", ")}), as W12X-2Y25`] : null,
    part_id: ["part_id", "PRR part ID"],
    wafer_xy: ["wafer_xy", "PRR wafer + X/Y"],
    xy: ["xy", "PRR X/Y only"],
  };
  return idOrderAll().filter((k) => all[k]).map((k) => [k, ...all[k]]);
}
/** The ways to identify parts for a picker: [API key, label]. */
function keyOptions() { return [...idChoices().map(([, k, l]) => [k, l]), ["tests", "Other PTR test(s)…"]]; }
function defaultKey() { return idChoices()[0][1]; }
/** Whether open files share parts by part text, WXY or wafer, X, Y (any of them): { label, found }, or null. */
async function findSharedId(filter) {
  const keys = idChoices().filter(([id]) => ID_STRONG.includes(id)).map(([, k]) => k);
  if (!keys.length) return null;
  const found = await api("part_trace_detect", { keys, filter });
  if (!found.groups.length) return null;
  const by = found.titles.filter((_, j) => found.by_id[j] > 0);
  return { label: by.length ? by.join(", ") : found.titles.join(", "), found };
}
/** The ID picker of one page and side: its choice is kept for that page alone. With `ds` (a dataset id,
 *  or a function giving it) each ID shows how many of that file's parts have it, an ID the file lacks is
 *  marked, and a page opened for the first time starts on the first ID the file has. Call `.forDs(id)`
 *  when the page's file changes. */
function keyPicker(side, ds) {
  S.mk = S.mk || {};
  const chosen = () => S.mk[side] !== undefined;
  const cur = S.mk[side] || defaultKey();
  const opts = keyOptions();
  const isTests = !opts.some(([k]) => k === cur && k !== "tests");
  const sel = select(opts, isTests ? "tests" : cur, () => { picked = true; sync(); });
  let picked = chosen();
  const tests = h("input", { type: "text", value: isTests ? cur.replace(/^tests:/, "") : "", placeholder: "e.g. 500  or  501,502,503", title: "One test with a unique per-part value, or several tests whose values together identify the part (e.g. wafer, X, Y)", style: { width: "190px" }, oninput: () => { picked = true; sync(); } });
  const value = () => (sel.value === "tests" ? "tests:" + tests.value.trim() : sel.value);
  function sync() { tests.hidden = sel.value !== "tests"; if (picked) S.mk[side] = value(); }
  sync();
  const set = (k) => { sel.value = k; picked = true; sync(); };
  // coverage in the page's file
  let seq = 0;
  const forDs = async (id) => {
    if (id == null) return;
    const mine = ++seq;
    try {
      const c = await api("id_coverage", { ds: id, keys: opts.filter(([k]) => k !== "tests").map(([k]) => k) });
      if (mine !== seq) return;
      const n = Object.fromEntries(c.keys.map((x) => [x.key, x]));
      for (const o of sel.options) {
        const x = n[o.value], base = (opts.find(([k]) => k === o.value) || [])[1] || o.textContent;
        o.textContent = !x ? base : x.parts ? `${base} · ${x.parts.toLocaleString()} of ${c.parts.toLocaleString()} parts` : `${base} · not in this file`;
      }
      if (!picked) {
        const first = c.keys.find((x) => x.parts > 0);
        if (first && first.key !== sel.value) { sel.value = first.key; sync(); }
      }
    } catch (e) {}
  };
  forDs(typeof ds === "function" ? ds() : ds);
  return { el: h("div", { class: "row tight" }, sel, tests), value, set, forDs };
}
function viewMatch(el) {
  el.append(h("h1", {}, "Match parts"), h("p", { class: "lead" }, "Find the same physical parts in two files: by PRR part text or part ID, by die X/Y, or by datalogged PTR results (one test holding a unique wafer + X/Y code, or wafer, X and Y in separate tests). Matched parts become a part set you can filter on, compare, split out, merge and chart side by side. To follow one part, or many parts over several files, use the Trace page."),
    h("div", { class: "row" }, h("button", { onclick: () => go("trace") }, "Find one part…"), h("button", { onclick: () => go("trace") }, "Part trace report…"), h("button", { onclick: () => go("export") }, "Merge files…")));
  const ids = S.datasets.map((d) => d.id);
  const st = S.m || (S.m = { a: S.ds, b: otherDs(S.ds), name: "match", fa: "", fb: "" });
  if (!ids.includes(st.a)) st.a = S.ds;
  if (!ids.includes(st.b) || st.b === st.a) st.b = otherDs(st.a);
  const two = S.datasets.length > 1;
  const a = select(dsOptions(), String(st.a), (v) => { st.a = +v; ka.forDs(st.a); });
  const b = select(dsOptions(), String(st.b), (v) => { st.b = +v; kb.forDs(st.b); });
  const ka = keyPicker("match_a", st.a), kb = keyPicker("match_b", st.b);
  const name = h("input", { type: "text", value: st.name, style: { width: "110px" }, title: "Used as $name in filters" });
  const fa = h("input", { type: "text", class: "filter-input", value: st.fa, placeholder: "optional, e.g. wafer = \"W01\"" });
  const fb = h("input", { type: "text", class: "filter-input", value: st.fb, placeholder: "optional" });
  const out = h("div");
  const traceBox = h("div");
  const sync = () => { st.a = +a.value; st.b = +b.value; st.name = name.value.trim() || "match"; st.fa = fa.value; st.fb = fb.value; st.ka = ka.value(); st.kb = kb.value(); };
  const run = async () => {
    sync();
    if (st.a === st.b) return toast("Pick two different files", "bad");
    loading(out, "Matching parts…");
    try { S.mres = await api("match", { a: st.a, b: st.b, key_a: st.ka, key_b: st.kb, filter_a: st.fa, filter_b: st.fb, name: st.name }); renderMatch(out, S.mres, traceBox); } catch (e) { showErr(out, e); }
  };
  if (!two) { el.append(h("div", { class: "help" }, "Open a second STDF file (without 'combine') to match parts between files.")); return; }
  el.append(h("div", { class: "controls" }, field("File A", a), field("Identify A parts by", ka.el), field("Filter A", fa), quickFilter(fa, "qa")),
    two ? h("div", { class: "controls" }, field("File B", b), field("Identify B parts by", kb.el), field("Filter B", fb), quickFilter(fb, "qb"), field("Part set name", name), h("button", { class: "primary", onclick: run }, "Match parts")) : "",
    two ? h("div", { class: "help" }, "Example – parts that were bin 1 in file A but failed test 1013 in file B: Filter A ", h("code", {}, "hbin = 1"), ", Filter B ", h("code", {}, "fail(1013)"), " (or ", h("code", {}, "first_fail = 1013"), ", or ", h("code", {}, "fail(1013) and n_fails = 1"), " for that test only). Then use “Find marker tests” on file A.") : "");
  el.append(out, traceBox);
  if (S.mres && S.mres.a === st.a && S.mres.b === st.b && ids.includes(st.a) && ids.includes(st.b)) renderMatch(out, S.mres, traceBox);
}
/** A part as the part searches list it. */
function partHitLabel(r) {
  return [`part ${r.part_id}`, r.part_txt, r.x != null ? `${r.wafer ? r.wafer + " " : ""}X${r.x} Y${r.y}` : "", r.wxy != null ? `WXY ${r.wxy}` : "", r.test_xy ? testXyLabel(r.test_xy) : "", `site ${r.site}`, `bin ${r.hbin}`].filter(Boolean).join(" · ");
}
/** The wafer, X and Y tests the part searches read (Settings). */
function partSearchParams() {
  const wc = waferCal();
  return { wafer_test: wc.w, x_test: wc.x, y_test: wc.y, ox_test: wc.ox, oy_test: wc.oy, offset_mode: wc.mode, flip_x: wc.fx, flip_y: wc.fy, wxy_test: idTests(true).wxy || "" };
}
/** Adds a found part to the Part datalog's list (that file's), and says so. */
function addToDatalog(ds, r) {
  if (!S.dl || S.dl.ds !== ds) S.dl = { ds, parts: [], tests: S.dl ? S.dl.tests : [], modes: S.dl ? S.dl.modes : ["part_id", "xy", "part_txt"], format: S.dl ? S.dl.format : "txt" };
  if (S.dl.parts.some((p) => p.index === r.index)) return toast("Already in the part datalog");
  S.dl.parts.push(Object.assign({ mark: "" }, r));
  setDs(ds);
  toast(`Added part ${r.part_id} to the part datalog (${S.dl.parts.length} part${S.dl.parts.length > 1 ? "s" : ""})`, "good");
}
/** Builds a filter like  not pass and fail(1013)  into `input`. */
function quickFilter(input, key) {
  const st = (S.qf = S.qf || {})[key] || (S.qf[key] = { parts: "", test: "", how: "fail" });
  const parts = select([["", "any part"], ["pass", "good parts"], ["not pass", "bad parts"], ["hbin = 1", "hard bin 1"]], st.parts, (v) => { st.parts = v; apply(); });
  const test = h("input", { type: "text", value: st.test, placeholder: "test #", style: { width: "90px" }, onchange: (e) => { st.test = e.target.value.trim(); apply(); } });
  const how = select([["fail", "fails it"], ["first", "fails it first"], ["only", "fails only it"]], st.how, (v) => { st.how = v; apply(); });
  function apply() {
    const t = st.test.replace(/:.*/, "");
    const tc = !t ? "" : st.how === "first" ? `first_fail = ${t}` : st.how === "only" ? `fail(${t}) and n_fails = 1` : `fail(${t})`;
    input.value = [st.parts, tc].filter(Boolean).join(" and ");
  }
  return field("Quick filter", h("div", { class: "row tight" }, parts, h("span", { class: "muted small" }, "failing test"), test, how));
}
const andF = (base, x) => (base && base.trim() ? `(${base}) and ${x}` : x);
function useFilter(id, expr) {
  setDs(id);
  document.getElementById("filter").value = expr;
  S.filter = expr;
  document.getElementById("filter").classList.add("active");
  go("overview");
}
function baseName(d) { return (d ? d.name : "export").replace(/\.(stdf|std)(\.gz)?$/i, "").replace(/\.gz$/i, "").replace(/[^\w.-]+/g, "_"); }
function nearDs(d, name) {
  const dir = d && d.paths.length ? dirname(d.paths[0]) : "";
  return dir ? dir + (dir.includes("\\") ? "\\" : "/") + name : "";
}
function renderMatch(out, m, traceBox) {
  const A = dsName(m.a), B = dsName(m.b), set = "$" + m.name;
  out.replaceChildren(h("div", { class: "kpis" },
    kpi("Matched units", m.pairs.toLocaleString(), `${m.key_a} ↔ ${m.key_b}`),
    kpi("A: " + A, `${m.matched_a.toLocaleString()} / ${m.n_a.toLocaleString()}`, `${m.only_a.toLocaleString()} only in A` + (m.no_key_a ? ` · ${m.no_key_a} without key` : "")),
    kpi("B: " + B, `${m.matched_b.toLocaleString()} / ${m.n_b.toLocaleString()}`, `${m.only_b.toLocaleString()} only in B` + (m.no_key_b ? ` · ${m.no_key_b} without key` : "")),
    kpi("Repeat keys", `${m.dup_a} / ${m.dup_b}`, "retests in A / B (last test is paired)"),
    kpi("Bin agreement", m.bin_agreement_pct == null ? "–" : pct(m.bin_agreement_pct, 1), "same hard bin in A and B")));
  if (!m.pairs) {
    out.append(h("div", { class: "err" }, "No parts matched. Both sides must produce the same identity text: e.g. the same PART_TXT, or tests whose results are equal (12 and 12.0 compare equal). PRR wafer IDs such as \"LOT-W01\" never equal a logged wafer number; match logged tests against logged tests instead."));
    return;
  }
  out.append(h("div", { class: "help" }, "Part set ", h("code", {}, set), " now exists in both files. Use it in the part filter (", h("code", {}, set), ", ", h("code", {}, `not ${set}`), ", ", h("code", {}, `${set} and not pass`), "), in Compare → group mode, or in Split & export → my own groups."));
  const cmpOut = h("div");
  const sideCard = (id, label) => {
    const d = S.datasets.find((x) => x.id === id);
    const base = baseName(d);
    const isA = id === m.a;
    const sideFilter = ((isA ? S.m.fa : S.m.fb) || "").trim();
    const other = isA ? m.b : m.a;
    const keys = isA ? { key_a: S.m.ka, key_b: S.m.kb } : { key_a: S.m.kb, key_b: S.m.ka };
    const res = h("div");
    const single = d && d.files === 1 && !d.derived;
    const scopeNote = sideFilter ? h("p", { class: "muted small" }, "Unmatched = parts within Filter ", label, " (", h("code", {}, sideFilter), ") that did not match.") : null;
    const markers = () => {
      runCompare(cmpOut, Object.assign({ mode: "group", a: id, select: set, vs: `not ${set}`, filter: scoped(sideFilter), match_by: "key", later: other }, keys));
      cmpOut.scrollIntoView({ behavior: "smooth" });
    };
    const mkDatasets = async (thenCompare) => {
      busy("Creating datasets…");
      const made = [];
      try {
        for (const [what, f] of [["matched", andF(sideFilter, set)], ["unmatched", andF(sideFilter, `not ${set}`)]]) {
          try { made.push(Object.assign({ what }, await api("subset_ds", { ds: id, filter: f, name: `${dsName(id)} · ${m.name} ${what}` }))); }
          catch (e) { toast(`No ${what} dataset: ${e.message}`, "bad", 8000); }
        }
        await refreshDatasets(S.ds);
        if (made.length) toast("Created " + made.map((x) => `“${x.name}” (${x.parts})`).join(" and "), "good", 7000);
        if (thenCompare && made.length === 2) { S.cmpMode = "datasets"; S.cmpA = made[1].id; S.cmpB = made[0].id; S.cmpFA = ""; S.cmpFB = ""; S.cmpAuto = true; go("compare"); }
      } finally { busy(null); }
    };
    return h("div", { class: "card pad" }, h("h2", { style: { marginTop: 0 } }, `${label}: ${dsName(id)}`), scopeNote,
      h("div", { class: "row" },
        h("button", { class: "primary", onclick: markers, title: "Rank every test: matched vs unmatched parts of this file, plus drift of the same units in the other file" }, "Find marker tests"),
        h("button", { onclick: () => mkDatasets(true) }, "New datasets: matched / unmatched → compare"),
        h("button", { onclick: () => mkDatasets(false) }, "Only create the datasets")),
      h("div", { class: "row" },
        h("button", { onclick: () => useFilter(id, andF(sideFilter, set)) }, "Analyse matched parts"),
        h("button", { onclick: () => useFilter(id, andF(sideFilter, `not ${set}`)) }, "Analyse unmatched parts")),
      single ? h("div", { class: "row" }, h("button", { title: "Opens Split on Merge, split & export with the groups filled in", onclick: () => {
        // one split tool: its groups are filled in for this file
        const d = (S.drafts ||= {}), k = (x) => `export:${id}:${x}`;
        d[k("by")] = "groups";
        d[k("groups")] = sideFilter ? [{ name: "matched", expr: andF(sideFilter, set) }, { name: "unmatched", expr: andF(sideFilter, `not ${set}`) }] : [{ name: "matched", expr: set }];
        d[k("rest")] = sideFilter ? "" : "unmatched";
        setDs(id); go("export");
      } }, "Split into matched / unmatched STDF…")) : h("p", { class: "muted small" }, "Splitting needs a dataset opened from a single file."),
      res);
  };
  out.append(h("div", { class: "grid2" }, sideCard(m.a, "A"), sideCard(m.b, "B")), cmpOut);

  // dual view
  const ta = testInput("dta", S.dualA || "", "test in A", m.a);
  const tb = testInput("dtb", S.dualB || "", "test in B (blank = same)", m.b);
  const color = select(GROUPS, S.dualColor || "pass", (v) => { S.dualColor = v; draw(); });
  const side = select([["a", "A's"], ["b", "B's"]], S.dualSide || "a", (v) => { S.dualSide = v; draw(); });
  const charts = h("div");
  async function draw() {
    S.dualA = ta.value; S.dualB = tb.value;
    if (!ta.value) { charts.replaceChildren(h("p", { class: "muted" }, "Pick a test in A (and optionally a different one in B).")); return; }
    loading(charts);
    try {
      const d = await api("match_dual", { link: m.link, test_a: ta.value, test_b: tb.value, color_by: S.dualColor || "pass", color_side: S.dualSide || "a", range: S.co.range, filter_a: scoped(""), filter_b: scoped("") });
      charts.replaceChildren(h("div", { class: "kpis" }, kpi("Units with both results", d.n.toLocaleString(), `of ${d.pairs.toLocaleString()} matched`), kpi("A test", String(d.test_a.num), d.test_a.name), kpi("B test", String(d.test_b.num), d.test_b.name), kpi("Correlation r", fmt(d.r, 4))));
      const g = h("div", { class: "grid2" });
      charts.append(g);
      for (const k of ["hist_a", "hist_b", "scatter", "delta"]) { const [card, c] = chartBox(k === "scatter" ? "tall" : ""); g.append(card); plot(c, d.figures[k]); }
    } catch (e) { showErr(charts, e); }
  }
  ta.addEventListener("change", draw);
  tb.addEventListener("change", draw);
  if (!ta.value) featuredTest([m.a, m.b]).then((f) => { if (f && f.key && !ta.value) { ta.value = f.key; draw(); } });
  out.append(h("div", { class: "row" }, h("h2", {}, "Dual view: the same units in both files"),
    h("button", { title: "Every test, CORRELATED / OFFSET / UNCORRELATED, over the same units paired by the same IDs", onclick: () => {
      Object.assign(S, { ucA: m.a, ucB: m.b, corrMode: "unit" }); Object.assign((S.mk ||= {}), { uc_a: S.m.ka, uc_b: S.m.kb }); go("correlation");
    } }, "Unit correlation of these units →")),
    h("p", { class: "lead" }, "Each matched unit contributes its result for the A test and for the B test, so e.g. test 101 in A can be viewed against test 102 in B for exactly the same dies. Colors follow a field of either file, e.g. B's results colored by A's site."),
    h("div", { class: "controls" }, field("Test in A", ta), field("Test in B", tb), field("Color parts by", color), field("using the field of", side), h("button", { class: "primary", onclick: draw }, "Show")),
    chartOpts(draw), charts);
  draw();

  // pairs
  const tbl = h("div");
  out.append(h("h2", {}, "Matched units"), h("p", { class: "muted" }, "Click a row to trace that unit: all its results in A and B side by side."), tbl);
  const pf = (v) => (v === null ? badge("n/a", "neutral") : v ? badge("good", "good") : badge("bad", "bad"));
  const cols = [{ key: "key", label: "Identity" }, { key: "a_part_id", label: "A part" }, { key: "b_part_id", label: "B part" }, { key: "a_site", label: "A site", num: true }, { key: "b_site", label: "B site", num: true }, { key: "a_hbin", label: "A hbin", num: true }, { key: "b_hbin", label: "B hbin", num: true }, { key: "a_pass", label: "A", render: pf }, { key: "b_pass", label: "B", render: pf }, { key: "same", label: "Bins", render: (v) => (v ? badge("same", "good") : badge("differ", "warn")) }];
  const rows = m.rows.map((r) => Object.assign({ same: r.a_hbin === r.b_hbin }, r));
  table(tbl, cols, rows, {
    onClick: (r) => trace(traceBox, { a: m.a, b: m.b, key_a: S.m.ka, key_b: S.m.kb, row_a: r.a_row, row_b: r.b_row }),
    onClear: () => traceBox.replaceChildren(),
    tools: h("button", { onclick: () => SC.download(`matched_${m.name}.csv`, SC.toCSV(cols.filter((c) => c.key !== "same"), rows)) }, "Download CSV"),
  });
}
// ---- Signature check
/** Editable list of {ds, filter} populations. */
function popList(list, label, emptyNote) {
  const box = h("div");
  const draw = () => {
    box.replaceChildren(...list.map((it, i) => h("div", { class: "row tight", style: { marginBottom: "6px" } },
      select(dsOptions(), String(it.ds), (v) => (it.ds = +v)),
      h("input", { type: "text", class: "filter-input", value: it.filter, placeholder: "good parts, e.g. hbin = 1", oninput: (e) => (it.filter = e.target.value) }),
      h("button", { title: "Remove", onclick: () => { list.splice(i, 1); draw(); } }, "×"))),
      list.length ? "" : h("p", { class: "muted small" }, emptyNote),
      h("button", { onclick: () => { list.push({ ds: S.datasets[S.datasets.length - 1].id, filter: "hbin = 1" }); draw(); } }, "+ " + label));
  };
  draw();
  return box;
}
const SIG_VERDICT = { keep: "good", caution: "warn", drop: "bad", untested: "neutral", weak: "neutral", "no data": "neutral" };
const SCREEN_VERDICT = { "resembles lot A": "bad", elevated: "warn", "like the reference": "good", "no signature": "neutral" };
/** Signature check settings, seeded from Bad candidates. */
function sigState() {
  const bc = S.bc || {};
  return S.sg || (S.sg = { a: bc.a ?? S.ds, name: bc.name || "bad_candidate", filter: bc.good || "hbin = 1", max: 5, clean: [], screen: [], stayed: [], useClean: true, useConf: true, useStayed: true, confirmed: "confirmed_good", noMarginal: false });
}
function viewSignature(el) {
  el.append(h("h1", {}, "Signature check"), h("p", { class: "lead" }, "Learn a failure signature from the bad candidates of the lot with the issue (lot A), throw out what known-good parts also show, and screen other files with what is left. Run Bad candidates first: it creates the part sets the signature is learned from ($bad_candidate) and checked against ($confirmed_good)."));
  if (S.datasets.length < 2) { el.append(h("div", { class: "help" }, "Open lot A's files (and a clean lot, if you have one) without 'combine'.")); return; }
  const bc = S.bc || {};
  const ids = S.datasets.map((d) => d.id);
  const st = sigState();
  if (!ids.includes(st.a)) st.a = S.ds;
  for (const k of ["clean", "screen"]) st[k] = st[k].filter((x) => ids.includes(x.ds));
  if (!st.clean.length && !st.cleanSeeded) { st.cleanSeeded = true; const o = S.datasets.find((d) => d.id !== st.a && d.id !== bc.b); if (o) st.clean.push({ ds: o.id, filter: "hbin = 1" }); }
  const a = select(dsOptions(), String(st.a), (v) => (st.a = +v));
  const name = h("input", { type: "text", value: st.name, style: { width: "130px" }, oninput: (e) => (st.name = e.target.value) });
  const filter = h("input", { type: "text", class: "filter-input", value: st.filter, oninput: (e) => (st.filter = e.target.value) });
  const noMarg = h("input", { type: "checkbox", checked: !!st.noMarginal, onchange: (e) => (st.noMarginal = e.target.checked) });
  const max = h("input", { type: "number", min: 1, max: 20, value: st.max, style: { width: "70px" }, oninput: (e) => (st.max = +e.target.value || 5) });
  const useClean = h("input", { type: "checkbox", checked: st.useClean !== false, onchange: (e) => (st.useClean = e.target.checked) });
  const useConf = h("input", { type: "checkbox", checked: st.useConf !== false, onchange: (e) => (st.useConf = e.target.checked) });
  st.stayed = (st.stayed || []).filter((x) => ids.includes(x.a) && ids.includes(x.b));
  const useStayed = h("input", { type: "checkbox", checked: st.useStayed !== false, onchange: (e) => (st.useStayed = e.target.checked) });
  const stayedBox = h("div");
  const drawStayed = () => stayedBox.replaceChildren(...st.stayed.map((it, i) => h("div", { class: "row tight", style: { marginBottom: "6px" } },
    select(dsOptions(), String(it.a), (v) => (it.a = +v)), h("span", { class: "muted small" }, "good, and pass again in"), select(dsOptions(), String(it.b), (v) => (it.b = +v)),
    h("button", { title: "Remove", onclick: () => { st.stayed.splice(i, 1); drawStayed(); } }, "×"))),
    h("button", { onclick: () => { st.stayed.push({ a: S.datasets[0].id, b: S.datasets[S.datasets.length - 1].id }); drawStayed(); } }, "+ Add run 1 → 100% retest"));
  drawStayed();
  const conf = h("input", { type: "text", value: st.confirmed || "confirmed_good", style: { width: "130px" }, oninput: (e) => (st.confirmed = e.target.value) });
  const srcRow = { display: "flex", gap: "6px", alignItems: "center", margin: "8px 0" };
  const out = h("div");
  const run = async () => {
    const clean = st.useClean !== false ? st.clean : [];
    const stayed = st.useStayed !== false ? st.stayed : [];
    if (stayed.some((x) => x.a === x.b)) return toast("A stayed-good source needs two different files: run 1 and its retest", "bad");
    if (!clean.length && !stayed.length && st.useConf === false) return toast("Tick at least one reference source: a clean lot, a stayed-good lot or the confirmed good parts", "bad");
    loading(out, "Learning rules in lot A, checking them against the reference pool…");
    try {
      const d = await api("signature", { a: st.a, name: st.name, filter: scoped(st.filter), exclude: idExclude(), max_rules: st.max, range: S.co.range, exclude_set: st.noMarginal ? "marginal" : "",
        use_clean: st.useClean !== false, use_confirmed: st.useConf !== false, confirmed: st.confirmed || "confirmed_good",
        use_stayed: st.useStayed !== false, stayed: stayed.map((x) => ({ a: x.a, b: x.b, filter: scoped("pass") })),
        clean: clean.map((c) => ({ ds: c.ds, filter: scoped(c.filter) })), screen: st.screen.map((c) => ({ ds: c.ds, filter: scoped(c.filter) })) });
      S.sgLast = d;
      renderSignature(out, d, st);
    } catch (e) { showErr(out, e); }
  };
  el.append(
    h("div", { class: "card pad" }, h("b", {}, "1 · Learn from lot A"), h("div", { class: "controls" }, field("File 1 of lot A", a), field("Bad candidates (part set)", h("div", { class: "row tight" }, h("span", {}, "$"), name)), field("Population", filter),
      h("label", { class: "chk", title: "Leave the $marginal parts (file-2 fails outside the Bad candidates value range) out of every group" }, noMarg, "Leave out $marginal"), h("button", { onclick: () => go("candidates") }, "Bad candidates…")),
      h("p", { class: "muted small" }, "Normal parts: without a clean lot, the confirmed good parts (if there are at least 30), else every other good part of lot A.")),
    h("div", { class: "grid2" },
      h("div", { class: "card pad" }, h("b", {}, "2 · Reference pool (good parts only)"), h("p", { class: "muted small" }, "Every reference part that matches a rule is a false alarm. Each source's rate is shown; the verdicts use the pooled rate."),
        h("label", { class: "chk", style: srcRow }, useConf, "Confirmed good from file 2:", h("span", {}, "$"), conf, h("span", { class: "muted small" }, "(their lot-A values)")),
        h("label", { class: "chk", style: srcRow }, useClean, "Clean lots (no such failures):"),
        popList(st.clean, "Add clean lot", "Optional: add a validated lot (e.g. lot B)."),
        h("label", { class: "chk", style: srcRow, title: "Parts good in a run 1 that pass again on its 100% retest (matched by part text), e.g. lot C; their run-1 values are used" }, useStayed, "Stayed good on retest:"),
        stayedBox),
      h("div", { class: "card pad" }, h("b", {}, "3 · Files to screen"), h("p", { class: "muted small" }, "Does their hit rate look like lot A or like the reference?"), popList(st.screen, "Add file to screen", "Optional: add the files you want to check."))),
    h("div", { class: "controls" }, field("Rules in the signature (max)", max), h("button", { class: "primary", onclick: run }, "Check signature")),
    out);
  if (S.sgLast) renderSignature(out, S.sgLast, st);
}
function renderSignature(out, d, st) {
  const pctS = (v, n) => (v == null ? "–" : (v * 100).toFixed(n ?? 1) + "%");
  const sg = d.signature;
  const nk = d.rows.filter((r) => r.verdict === "keep").length, nc = d.rows.filter((r) => r.verdict === "caution").length;
  const srcs = d.sources || [];
  out.replaceChildren(h("div", { class: "kpis" },
    kpi("Bad candidates", d.n_cand.toLocaleString(), `$${d.name}`),
    kpi("Normal parts", d.n_normal.toLocaleString(), d.normal === "confirmed good" ? `$${d.confirmed}: the rules are learned against them` : "other good parts of lot A"),
    kpi("Reference pool", d.n_pool.toLocaleString(), srcs.map((c) => `${c.name} ${c.n.toLocaleString()}`).join(" + ")),
    kpi("Rules kept", `${nk} + ${nc}`, `keep + caution, of ${d.rows.length} tests`)));
  if (d.notes && d.notes.length) out.append(h("ul", { class: "small muted" }, ...d.notes.map((t) => h("li", {}, t))));
  // the signature
  const card = h("div", { class: "card pad" });
  out.append(h("h2", {}, "Signature"), card);
  if (!sg.rules.length) card.append(h("p", {}, "No rule survives the reference pool: whatever separates lot A's candidates also shows in good parts without the failure. There is no specific signature to screen with yet."));
  else {
    card.append(h("p", {}, `A part matches when it hits at least ${sg.m} of these ${sg.rules.length} rule${sg.rules.length > 1 ? "s" : ""}:`),
      h("p", { class: "muted small" }, `Built one rule at a time from ${sg.considered} usable (keep / caution) rule${sg.considered === 1 ? "" : "s"}: ` + sg.steps.map((x, i) => `${i ? "+ " : ""}${x.key} → score ${fmt(x.score, 3)} (m = ${x.m})`).join(", ") + (sg.rules.length < sg.considered ? "; no other rule improves the score." : ".")),
      h("ul", { class: "findings" }, ...sg.rules.map((r) => h("li", {}, h("b", {}, r.key), ` ${r.text} ${r.units || ""} `, badge(r.verdict, SIG_VERDICT[r.verdict]), h("span", { class: "muted small" }, ` capture ${pctS(r.capture, 0)}, normal ${pctS(r.fa_a)}, ` + r.sources.map((c) => `${c.lot} ${pctS(c.rate)}`).join(", ") + `, pooled ${pctS(r.fa_ref)}`)))),
      h("div", { class: "kpis" },
        kpi("Catches", pctS(sg.capture, 0), "of lot A's bad candidates"),
        kpi("Normal parts", pctS(sg.fa_a), "that match"),
        ...sg.sources.map((c) => kpi(c.name, pctS(c.rate), `false alarms (${c.n.toLocaleString()} good parts)`)),
        kpi("Reference pool", pctS(sg.fa_ref), "pooled false-alarm rate")));
    if (sg.filter) card.append(h("p", { class: "small" }, "As a part filter: ", h("code", {}, sg.filter), " ",
      h("button", { onclick: () => { navigator.clipboard && navigator.clipboard.writeText(sg.filter); toast("Copied", "good"); } }, "Copy")));
  }
  // screening
  if (d.screen.length) {
    const det = h("div");
    out.append(h("h2", {}, "Screening"), h("p", { class: "lead" }, `Signature hit rate in each file against the reference pool (${pctS(d.rate_ref, 2)}) and lot A (${pctS(d.rate_a, 2)}). p: one-sided Fisher exact test, file vs reference pool. Click a file for its at-risk parts.`));
    const tb = h("div");
    out.append(tb, det);
    const showFile = (f) => {
      det.replaceChildren(h("h3", {}, `${f.name}: ${f.hits.toLocaleString()} parts match`),
        h("p", { class: "muted small" }, "Per rule: " + f.per_rule.map((r) => `${r.key} ${pctS(r.rate)}`).join(" · ")));
      const sel = S.datasets.find((x) => x.name === f.name);
      if (sg.filter && sel) det.append(h("div", { class: "row" }, h("button", { onclick: () => useFilter(sel.id, sg.filter) }, "Analyse them"), h("button", { onclick: () => { setDs(sel.id); document.getElementById("filter").value = ""; S.filter = ""; S.wmode = "pass"; go("wafer"); } }, "Wafer maps")));
      const t = h("div");
      det.append(t);
      const cols = [{ key: "part_txt", label: "Part text" }, { key: "part_id", label: "Part" }, { key: "wafer", label: "Wafer" }, { key: "x", label: "X", num: true }, { key: "y", label: "Y", num: true }, { key: "site", label: "Site", num: true }, { key: "n_rules", label: "Rules hit", num: true }, { key: "rules", label: "Which" }];
      table(t, cols, f.parts, { tools: h("button", { onclick: () => SC.download(`signature_${baseName({ name: f.name })}.csv`, SC.toCSV(cols, f.parts)) }, "Download CSV") });
    };
    table(tb, [
      { key: "name", label: "File" },
      { key: "verdict", label: "Verdict", render: (v) => badge(v, SCREEN_VERDICT[v] || "neutral") },
      { key: "n", label: "Parts", num: true },
      { key: "hits", label: "Match", num: true },
      { key: "rate", label: "Rate", num: true, fmt: (v) => pctS(v, 2) },
      { key: "p_ref", label: "p vs reference", num: true, fmt: fmtP, title: "one-sided Fisher exact test: is this file's rate higher than the reference pool's?" },
    ], d.screen, { search: false, onClick: showFile });
    showFile(d.screen[0]);
  }
  // all rules
  const figs = Object.fromEntries(d.figures.map((f) => [f.key, f.figure]));
  const chart = h("div");
  const showRule = (r) => {
    if (!figs[r.key]) { chart.replaceChildren(h("p", { class: "muted" }, `No chart for ${r.key} (only the leading and kept rules are drawn).`)); return; }
    const [c1, c] = chartBox("tall");
    chart.replaceChildren(h("p", { class: "small" }, h("b", {}, r.key), " — ", r.why, "."), c1);
    plot(c, normable(figs[r.key]));
  };
  const tabs = h("div", { class: "tabs" }), tb2 = h("div");
  const cols = [
    { key: "rank", label: "#", num: true },
    { key: "key", label: "Test" },
    { key: "verdict", label: "Verdict", render: (v, r) => { const b = badge(v, SIG_VERDICT[v]); b.title = r.why; return b; } },
    { key: "drop_by", label: "Dropped by", title: "the reference source whose own false-alarm rate dropped the rule (each source can drop a rule on its own, so a large pool cannot outvote it)", fmt: (v) => v || "" },
    { key: "text", label: "Rule" },
    { key: "capture", label: "Capture", num: true, fmt: (v) => pctS(v, 0), title: "share of lot A's bad candidates the rule catches" },
    { key: "fa_a", label: "Normal", num: true, fmt: (v) => pctS(v), title: d.normal === "confirmed good" ? "share of the confirmed good parts (the learning group) that match" : "share of lot A's other good parts that match (unknown status: some were never retested)" },
    ...srcs.map((c, i) => ({ key: `src${i}`, label: c.name, num: true, fmt: (v) => pctS(v), title: `false-alarm rate in ${c.kind === "confirmed" ? "the parts file 2 confirmed good (their lot-A values)" : "this clean lot's good parts"}` })),
    { key: "fa_ref", label: "Pooled ref.", num: true, fmt: (v) => pctS(v), title: "false-alarm rate of the whole reference pool: the verdict and the score use this" },
    { key: "shift", label: "Ref. shift σ", num: true, fmt: (v) => fmt(v, 2), title: "reference pool median − normal median, in normal σ, positive toward the rule's side. Well below 0 = the pool never reaches the rule's region" },
    { key: "limits", label: "Limits", title: "a clean lot's limits differ from lot A's" },
    { key: "score", label: "Score", num: true, fmt: (v) => fmt(v, 2), title: "capture − the larger false-alarm rate (normal parts or pooled reference)" },
  ];
  const rows = d.rows.map((r, i) => {
    const o = Object.assign({ rank: i + 1, shift: r.pool ? r.pool.shift_sigma : null, limits: r.pool && r.pool.limits_changed ? "changed" : "" }, r);
    srcs.forEach((c, j) => { const x = r.sources.find((y) => y.lot === c.name); o[`src${j}`] = x ? x.rate : null; });
    return o;
  });
  const views = [["all", "All"], ["keep", "Keep"], ["caution", "Caution"], ["drop", "Drop"], ["untested", "Not tested"], ["weak", "Weak"]];
  const setView = (v) => {
    S.sgView = v;
    tabs.replaceChildren(...views.map(([k, l]) => h("button", { class: k === v ? "active" : "", onclick: () => setView(k) }, `${l} (${k === "all" ? rows.length : rows.filter((r) => r.verdict === k).length})`)));
    const sel = v === "all" ? rows : rows.filter((r) => r.verdict === v);
    table(tb2, cols, sel, { onClick: showRule, placeholder: "Search tests…", tools: h("button", { onclick: () => SC.download("signature_rules.csv", SC.toCSV(cols.filter((c) => c.key !== "verdict").concat([{ key: "verdict", label: "Verdict" }, { key: "why", label: "Why" }, { key: "filter", label: "Filter" }]), sel)) }, "Download CSV") });
  };
  out.append(h("h2", {}, "Rules per test"), h("p", { class: "lead" }, "Each test's best threshold rule in lot A and how the reference pool judges it. Click a test to see the normal parts, the bad candidates and every other file against the rule's cut (dotted line)."), signatureHelp(), chartOpts(() => { }, { noRange: true, hist: true }), tabs, tb2, chart);
  setView(views.some(([k]) => k === S.sgView) ? S.sgView : "all");
  const first = rows.find((r) => figs[r.key]);
  if (first) showRule(first);
}
function signatureHelp() {
  const li = (n, t) => h("li", {}, h("b", {}, n), " — ", t);
  return h("details", { class: "adv help" }, h("summary", {}, "ⓘ How rules, verdicts and the screening work"),
    h("ul", { class: "small" },
      li("Normal parts", "the group the rules are learned against. Without a clean lot: the parts file 2 confirmed good ($confirmed_good: good in lot A, pass in file 2), if there are at least 30; otherwise every other good part of lot A (some of those were never retested, so a few may be real failures too)."),
      li("Rule (learn)", "for each test, the cut that best separates lot A's bad candidates from the normal parts: above a value, below a value, or outside a band around the normal parts (median ± k robust σ). The cut maximises Youden's J = capture − false-alarm rate. Learned on the same parts it is scored on, so the capture is optimistic; the reference pool is the check."),
      li("Reference pool", "good parts only, from the ticked sources: clean lots (no such failures), parts that stayed good on a 100% retest (good in run 1 and passing again, their run-1 values) and the confirmed good parts (their lot-A values). Each source's false-alarm rate is listed; any one source can drop a rule, and otherwise the pooled rate (all their parts together) drives the verdict and the score. Confirmed good parts are also among the normal parts, so their rate is in-sample; a clean lot is the independent check."),
      li("Capture / Normal / sources / Pooled", "share of the bad candidates, of the normal parts, of each reference source and of the whole pool that match the rule."),
      li("Score", "capture − the larger of the normal and pooled false-alarm rates. High = catches the candidates and stays quiet in every good population."),
      li("Drop", "a reference source (each clean lot and the confirmed good parts, each on its own) or the whole pool matches the rule in at least half as many parts as the candidates, or at least twice as often as the normal parts and significantly so (Fisher p < 0.01). The pattern exists without the failure, so it is not the signature. Because every source can drop a rule by itself, a large pool cannot outvote a small lot that shows the pattern; Dropped by names it."),
      li("Caution", "the pool matches it significantly more often than the normal parts, but less than twice as often. Still used."),
      li("Not tested", "the pool sits more than 0.5σ away from the rule's side, so it never reaches the region: few hits there prove nothing either way."),
      li("Weak", "the rule barely separates the candidates from the normal parts (capture − normal false alarms < 0.2)."),
      li("Signature", "built one rule at a time: start with the kept rule that scores best on its own, then keep adding the rule that most improves capture − the larger of the normal and pooled false-alarm rates (with m, the number of rules a part must hit, re-chosen at each step), and stop when no rule improves it or the maximum is reached. It is therefore never worse than its best single rule."),
      li("Screening", "a file's signature hit rate against the reference pool (one-sided Fisher exact test). Resembles lot A = significantly above the pool and at least half of lot A's rate; elevated = significantly above the pool only; like the reference = not significantly different."),
      li("Limits", "lots on the same test program can carry slightly different limits; rules use absolute values, and the Limits column flags tests whose limits changed.")));
}

// ---- Bad candidates
/** Wafer-coordinate settings shared with the wafer view (tests 80100–80104 by default). */
function waferCal() { return S.wcal || (S.wcal = Object.assign({}, WCAL_DEFAULT)); }
function viewCandidates(el) {
  el.append(h("h1", {}, "Bad candidates"), h("p", { class: "lead" }, "Parts that were good in file 1 but are bad in file 2 (e.g. failing one test, optionally only with its file-2 value in a range), found by matching the two files by part text. Instead of splitting anything, the matched good parts of file 1 are marked as bad candidates and compared with every other good part of file 1: which tests, outlier scores, sites, wafer zones or field positions set them apart, and which tests to focus on. Good parts that pass file 2 are marked as confirmed good."));
  if (S.datasets.length < 2) { el.append(h("div", { class: "help" }, "Open both files (without 'combine'): file 1 with most parts, file 2 with the parts that became bad.")); return; }
  const st = S.bc || (S.bc = { a: S.ds, b: otherDs(S.ds), good: "hbin = 1", test: "", how: "fail", range: "", incMarg: false, extra: "", name: "bad_candidate", coords: "auto" });
  const ids = S.datasets.map((d) => d.id);
  if (!ids.includes(st.a)) st.a = S.ds;
  if (!ids.includes(st.b) || st.b === st.a) st.b = otherDs(st.a);
  const a = select(dsOptions(), String(st.a), (v) => { st.a = +v; ka.forDs(st.a); });
  const b = select(dsOptions(), String(st.b), (v) => { st.b = +v; kb.forDs(st.b); });
  const good = h("input", { type: "text", class: "filter-input", value: st.good, placeholder: "e.g. hbin = 1  or  pass" });
  const test = h("input", { type: "text", value: st.test, placeholder: "e.g. 1013 (blank = any fail)", style: { width: "170px" } });
  const how = select([["fail", "fails it"], ["first", "fails it first"], ["only", "fails only it"]], st.how);
  const range = h("input", { type: "text", value: st.range || "", placeholder: "pick the test first (optional)", style: { width: "150px" }, title: "Hard bad only: the failing test's value in file 2 must be inside this range (ends included; either end may be left open, e.g. 10..). Type plain numbers: they are read in the unit shown next to the box, the test's display unit (its logged UNITS scaled by RES_SCAL, as the tester shows it). An explicit SI suffix (10uA..20uA) overrides that. File-2 fails outside the range become $marginal." });
  // the selected test's display unit in file 2: plain numbers in the range are read in it
  const unitHint = h("b", { class: "small" });
  const updHint = async () => {
    const q = test.value.trim().replace(/:.*/, "");
    unitHint.title = "";
    if (!q) { unitHint.textContent = ""; range.placeholder = "pick the test first (optional)"; return; }
    let t;
    try { t = (await testsFor(+b.value, "")).find((x) => x.key === q || String(x.num) === q || x.name === q); } catch (e) { t = null; }
    if (!t) { unitHint.textContent = ""; unitHint.title = ""; range.placeholder = `test ${q} not in file 2`; return; }
    const du = t.disp_units || t.units || "";
    range.placeholder = du ? `e.g. 10..20 (${du})` : "e.g. 10..20";
    unitHint.textContent = du || "(no unit)";
    unitHint.title = `Plain numbers are in ${du || "the test's units"}` + (t.res_scal ? `: logged in ${t.units || "?"} with RES_SCAL ${t.res_scal}, shown in ${du}` : (t.units ? `, the unit the test is logged in` : "")) + ". An SI suffix (e.g. 10u) overrides it.";
  };
  test.addEventListener("input", updHint);
  b.addEventListener("change", updHint);
  updHint();
  const incMarg = h("input", { type: "checkbox", checked: !!st.incMarg });
  const rangeNote = h("div", { class: "muted small", style: { maxWidth: "320px" } }, st.rangeNote || "");
  range.addEventListener("input", () => { st.rangeNote = ""; rangeNote.textContent = ""; });
  const extra = h("input", { type: "text", class: "filter-input", value: st.extra, placeholder: "optional extra filter on file 2" });
  const ka = keyPicker("ca", st.a), kb = keyPicker("cb", st.b);
  const name = h("input", { type: "text", value: st.name, style: { width: "130px" }, title: "Part set name, usable as $name in every filter" });
  const coords = select([["auto", "auto (PRR X/Y, else die X/Y from tests)"], ["tests", "die X/Y from tests (Settings)"]], st.coords);
  const out = h("div"), traceBox = h("div");
  const badFilter = () => {
    const t = test.value.trim().replace(/:.*/, "");
    const tc = !t ? "" : how.value === "first" ? `first_fail = ${t}` : how.value === "only" ? `fail(${t}) and n_fails = 1` : `fail(${t})`;
    return ["not pass", tc, extra.value.trim() ? `(${extra.value.trim()})` : ""].filter(Boolean).join(" and ");
  };
  const run = async () => {
    Object.assign(st, { a: +a.value, b: +b.value, good: good.value.trim(), test: test.value.trim(), how: how.value, range: range.value.trim(), incMarg: incMarg.checked, extra: extra.value, name: name.value.trim() || "bad_candidate", coords: coords.value });
    if (st.a === st.b) return toast("Pick two different files", "bad");
    if (st.range && !st.test) return toast("A value range needs the failing test", "bad");
    const goodF0 = scoped(st.good), set = "$" + st.name;
    // a set handed over (Retest trace) keeps its parts out of the reference too
    const goodF = st.keep && st.leave ? andF(goodF0, `not $${st.leave}`) : goodF0;
    loading(out, st.keep ? `Using $${st.keep} as the candidates…` : "Matching file 2's bad parts to file 1's good parts…");
    try {
      const m = await api("candidate_sets", { a: st.a, b: st.b, key_a: ka.value(), key_b: kb.value(), filter_a: goodF, filter_b: badFilter(), test: st.test.replace(/:.*/, ""), range: st.keep ? "" : st.range, include_marginal: st.incMarg, name: st.name, keep_set: st.keep || "", leave_set: st.leave || "" });
      if (!m.n_cand) { out.replaceChildren(h("div", { class: "err" }, `None of the ${m.n_b.toLocaleString()} bad parts of file 2 (${badFilter()}) matched a good part of file 1 (${goodF || "all parts"})` + (m.n_marginal ? `, except ${m.n_marginal.toLocaleString()} marginal fails outside the range ${st.range} (tick "Include marginal fails" to use them).` : `. Check the identity (${m.key_a} ↔ ${m.key_b}) and the filters.`))); return; }
      // marginal fails are left out of both groups unless they count as candidates
      const popF = m.n_marginal && !m.include_marginal ? (goodF ? `(${goodF}) and not $${m.marginal}` : `not $${m.marginal}`) : goodF;
      const w = waferCal();
      const common = { a: st.a, filter: popF, exclude: idExclude(), range: S.co.range };
      const cal = { coords: st.coords, wafer_test: w.w, x_test: w.x, y_test: w.y, ox_test: w.ox, oy_test: w.oy, offset_mode: w.mode, flip_x: w.fx, flip_y: w.fy };
      loading(out, `${m.n_cand.toLocaleString()} bad candidates found; ranking every test and outlier score…`);
      const [mk, cd] = await Promise.all([
        api("markers", Object.assign({ mode: "group", select: set, vs: "", later: st.b, key_a: ka.value(), key_b: kb.value(), methods: mkMethods() }, common)).catch((e) => e),
        api("candidates", Object.assign({ name: st.name }, common, cal)),
      ]);
      renderCandidates(out, traceBox, { m, mk, cd, st, popF, set, keys: { key_a: ka.value(), key_b: kb.value() }, badF: badFilter() });
    } catch (e) { showErr(out, e); }
  };
  // candidates handed over from Retest trace: shown, and dropped with ×
  const keepBox = st.keep ? h("div", { class: "row tight small", style: { margin: "6px 0" } }, badge(`candidates: $${st.keep}`, "warn"), st.keepNote || "",
    st.leave ? ` · $${st.leave} left out of every group` : "",
    h("button", { class: "ghost small", title: "Find the candidates from file 2 instead", onclick: () => { st.keep = ""; st.leave = ""; st.keepNote = ""; S.bcLast = null; go("candidates"); } }, "× Find them from file 2 instead")) : "";
  el.append(
    keepBox,
    h("div", { class: "controls" }, field("File 1 (good parts)", a), field("Good parts are", good), field("Identify parts by", ka.el)),
    h("div", { class: "controls" }, field("File 2 (became bad)", b), field("Bad parts failing test", h("div", { class: "row tight" }, test, how)), field("with its file-2 value in", h("div", {}, h("div", { class: "row tight" }, range, unitHint), rangeNote)), h("label", { class: "chk", title: "Off: file-2 fails outside the range ($marginal) are left out of both the candidates and the reference. On: they count as candidates." }, incMarg, "Include marginal fails as candidates"), field("Extra filter", extra), field("Identify parts by", kb.el)),
    h("div", { class: "controls" }, field("Mark them as", name), field("Die X/Y", coords), loadboardField(), h("button", { class: "primary", onclick: run }, "Find bad candidates")),
    out, traceBox);
  if (S.bcLast && ids.includes(S.bcLast.st.a) && ids.includes(S.bcLast.st.b)) renderCandidates(out, traceBox, S.bcLast);
}
/** One line on how the candidates differ on a test, in the terms of its strongest signal. */
function focusNote(r, pctS) {
  const parts = [];
  if (r.why === "tails") parts.push(`${pctS(r.tail_aff, 0)} of candidates beyond ±3σ vs ${pctS(r.tail_ref)} of the others`);
  else if (r.why === "PAT") parts.push(`${pctS(r.pat_aff, 0)} of candidates outside PAT limits vs ${pctS(r.pat_ref)} of the others`);
  else if (r.why === "random forest") parts.push(`random-forest importance ${fmt(r.importance, 2)}: it separates them together with other tests`);
  else if (r.drift_sep != null && ["drift", "residual", "correlation break"].includes(r.why)) parts.push(`the same units changed differently between the files (drift ${fmt(r.drift_shift_sigma, 2)}σ, residual ${fmt(r.resid_sigma, 2)}σ)`);
  parts.push(`median ${r.shift_sigma >= 0 ? "higher" : "lower"} by ${fmt(Math.abs(r.shift_sigma), 2)}σ, AUC ${fmt(r.auc, 2)}`);
  // how close the typical candidate sits to a limit, as a share of the limit window
  if (r.lo != null && r.hi != null && r.hi > r.lo) {
    const near = (m) => { const dl = (m - r.lo) / (r.hi - r.lo), dh = (r.hi - m) / (r.hi - r.lo); return dl < dh ? ["LO", dl] : ["HI", dh]; };
    const [lim, da] = near(r.median_aff);
    const dr = lim === "LO" ? (r.median_ref - r.lo) / (r.hi - r.lo) : (r.hi - r.median_ref) / (r.hi - r.lo);
    if (dr - da >= 0.05) parts.push(`median ${(da * 100).toFixed(0)}% of the limit window from ${lim} (others ${(dr * 100).toFixed(0)}%)`);
  }
  return parts.join("; ") + ".";
}
function renderCandidates(out, traceBox, R) {
  S.bcLast = R;
  const { m, mk, cd, st, popF, set } = R;
  const A = dsName(st.a), B = dsName(st.b);
  const pctS = (v, d) => (v == null ? "–" : (v * 100).toFixed(d ?? 1) + "%");
  const rng = m.range ? `${m.range.test} in ${m.range.lo ?? "−∞"} … ${m.range.hi ?? "+∞"} ${m.range.units || ""}`.trim() : "";
  out.replaceChildren(h("div", { class: "kpis" },
    kpi("Bad in file 2", m.n_b.toLocaleString(), R.badF),
    kpi("Bad candidates", m.n_cand.toLocaleString(), `good in ${A}, ${set}` + (rng ? `; ${rng} in ${B}` : "")),
    m.range ? kpi("Marginal", (m.n_marginal || 0).toLocaleString(), `$${m.marginal}: fails outside the range, ${m.include_marginal ? "counted as candidates" : "left out of both groups"}`) : "",
    kpi("Confirmed good", (m.n_confirmed || 0).toLocaleString(), `$${m.confirmed}: good in ${A}, pass in ${B}`),
    kpi("Not found as good", m.only_b.toLocaleString(), "bad in file 2 without a good match in file 1"),
    kpi("Reference", cd.n_ref.toLocaleString(), `other good parts (${popF || "all"})`),
    kpi("Tests ranked", cd.n_tests.toLocaleString(), cd.excluded ? `${cd.excluded} left out` : "")));
  out.append(h("div", { class: "row" },
    h("button", { onclick: () => useFilter(st.a, set) }, "Analyse the candidates (filter " + set + ")"),
    h("button", { onclick: () => { S.cmpMode = "group"; S.cmpSel = set; S.cmpVs = ""; S.ds = st.a; S.filter = popF; go("compare"); } }, "Open in Compare"),
    h("button", { onclick: () => { S.wmode = "pass"; S.ds = st.a; go("wafer"); } }, "Wafer maps"),
    h("button", { onclick: () => { Object.assign(sigState(), { a: st.a, name: st.name, filter: st.good, confirmed: m.confirmed || "confirmed_good", noMarginal: !!(m.n_marginal && !m.include_marginal) }); go("signature"); }, title: "Learn a signature from these candidates, check it against a reference pool of good parts (a clean lot and/or the confirmed good parts) and screen other files" }, "Signature check →")));
  if (m.marginal_rows && m.marginal_rows.length) {
    const mt = h("div");
    const mcols = [{ key: "key", label: "Identity" }, { key: "a_wafer", label: "Wafer" }, { key: "a_x", label: "X", num: true }, { key: "a_y", label: "Y", num: true }, { key: "a_site", label: "Site", num: true }, { key: "b_hbin", label: "Hbin in 2", num: true }, { key: "b_value", label: `${m.range.test} in 2 (${m.range.units || "–"})`, num: true, fmt: (v) => fmt(v, 4) }];
    out.append(h("details", { class: "adv" }, h("summary", {}, `Marginal: ${m.n_marginal.toLocaleString()} parts failed ${m.range.test} in ${B} outside the range (${m.include_marginal ? "counted as candidates" : "left out of the candidates and the reference"})`), mt));
    table(mt, mcols, m.marginal_rows, { tools: h("button", { onclick: () => SC.download(`${m.marginal}.csv`, SC.toCSV(mcols, m.marginal_rows)) }, "Download CSV"), onClick: (r) => trace(traceBox, Object.assign({ a: st.a, b: st.b, row_a: r.a_row }, R.keys, { value: r.key })) });
  }

  // plain-language summary of what they share (follows the ticked score methods)
  const sum = h("ul", { class: "findings" });
  const summarize = (rows) => {
    sum.replaceChildren();
    const top = rows.filter((r, i) => i === 0 || r.score >= 0.3).slice(0, 3);
    for (const r of top) sum.append(h("li", {}, h("b", {}, r.key), ` (${r.why}, score ${fmt(r.score, 2)}): `, focusNote(r, pctS)));
    const failed = cd.b_fails[0];
    if (failed && rows.length) {
      const i = rows.findIndex((r) => r.key === failed.key);
      const r = rows[i];
      sum.append(h("li", {}, `The test they fail in ${B}, `, h("b", {}, failed.key), i < 0 ? ` is not ranked in ${A} (no results there, or left out).`
        : r.score >= 0.3 ? ` already sets them apart in ${A}: rank #${i + 1}, score ${fmt(r.score, 2)}. They were marginal on it before failing.`
        : ` does not set them apart in ${A} (rank #${i + 1}, score ${fmt(r.score, 2)}): no early sign on the same test, so look at the other focus tests.`));
    }
    const ind = cd.indicators[0];
    if (ind) sum.append(h("li", {}, h("b", {}, ind.name), `: AUC ${fmt(ind.auc, 2)}, ${pctS(ind.capture, 0)} of candidates fall in the top 5 % of good parts (${fmt(ind.lift, 1)}× chance).`));
    const fac = cd.factors.find((f) => f.p < 0.01);
    if (fac) { const l = fac.levels[0]; sum.append(h("li", {}, h("b", {}, fac.name), `: ${l.level} has ${pctS(l.rate, 2)} candidates vs ${pctS(cd.n_aff / (cd.n_ref + cd.n_aff), 2)} overall (${fmt(l.lift, 1)}×, p ${fmtP(fac.p)}).`)); }
    else sum.append(h("li", {}, "No site, wafer, zone or bin stands out (no factor with p < 0.01)."));
    if (cd.b_fails.length) sum.append(h("li", {}, `In ${B} they fail: `, cd.b_fails.slice(0, 4).map((f) => `${f.key} (${pctS(f.share, 0)})`).join(", "), "."));
  };
  summarize([]);
  out.append(h("h2", {}, "What the candidates have in common"), sum);

  // focus tests (marker ranking)
  const focus = h("div");
  const mkBox = h("div");
  out.append(chartOpts(() => showFocus(S.bcFocus), { hist: true }), mkBox, focus);
  const showFocus = async (key) => {
    if (!key) return;
    S.bcFocus = key;
    loading(focus);
    try {
      const second = (R.ranked || mk.rows || []).map((r) => r.key).find((k) => k !== key);
      const [d, dual, sc] = await Promise.all([
        api("chart", { kind: "hist", ds: st.a, filter: popF, test: key, group_by: set, density: true, range: S.co.range }),
        api("match_dual", { link: m.link, test_a: key, test_b: key, color_by: "pass", color_side: "b", range: S.co.range, filter_a: "", filter_b: "" }).catch((e) => e),
        second ? api("chart", { kind: "scatter", ds: st.a, filter: popF, test: key, test2: second, group_by: set, range: S.co.range }).catch((e) => e) : null,
      ]);
      focus.replaceChildren(h("h2", {}, `Focus: ${key}`), h("p", { class: "lead" }, `${A}: bad candidates against the other good parts. Below, the same candidates' results in ${A} against ${B}.`));
      const g = h("div", { class: "grid2" });
      focus.append(g);
      for (const k of ["hist", "box", "cdf", "trend"]) { const [card, c] = chartBox(); g.append(card); plot(c, k === "hist" ? normable(d[k]) : d[k]); }
      const g2 = h("div", { class: "grid2" });
      focus.append(g2);
      if (sc && !(sc instanceof Error)) { const [card, c] = chartBox("tall"); g2.append(card); plot(c, sc.scatter); }
      if (dual && !(dual instanceof Error)) { const [card, c] = chartBox("tall"); g2.append(card); plot(c, dual.figures.scatter); }
      focus.scrollIntoView({ behavior: "smooth", block: "start" });
    } catch (e) { showErr(focus, e); }
  };
  if (mk instanceof Error) showErr(mkBox, mk);
  else {
    const fetchTop = (key) => api("chart", { kind: "compare_test", mode: "group", ds: st.a, select: set, vs: "", filter: popF, test: key, range: S.co.range }).then((c) => c.hist);
    renderMarkers(mkBox, mk, (row) => showFocus(row.key), { fetchTop, onRescore: (rows) => { R.ranked = rows; summarize(rows); } });
    mkBox.querySelector("h2").textContent = "Focus tests";
  }

  // indicators
  const indBox = h("div");
  out.append(h("h2", {}, "Outlier indicators"), h("p", { class: "lead" }, "Would a per-part outlier score have caught the candidates in file 1? AUC 0.5 = no better than chance. Capture = share of candidates above the 95th percentile of the other good parts; lift = capture ÷ 5 %. Flagged = share over the method's usual threshold."), indBox);
  const ig = h("div", { class: "grid2" });
  const it = h("div");
  indBox.append(ig);
  {
    const [card, c] = chartBox();
    ig.append(card, it);
    const rows = [...cd.indicators].reverse();
    plot(c, { data: [{ type: "bar", orientation: "h", y: rows.map((r) => r.name), x: rows.map((r) => r.auc), marker: { color: rows.map((r) => (r.auc >= 0.7 ? "@fail" : "@s1")) }, text: rows.map((r) => `capture ${pctS(r.capture, 0)}`), textposition: "auto", hovertemplate: "%{y}<br>AUC %{x:.2f}<br>%{text}<extra></extra>" }],
      layout: { title: { text: "How well each score separates the candidates (AUC)", x: 0.01, xanchor: "left", font: { size: 15 } }, xaxis: { range: [0, 1], title: { text: "AUC" } }, yaxis: { automargin: true }, margin: { l: 250, r: 20, t: 56, b: 44 }, shapes: [{ type: "line", x0: 0.5, x1: 0.5, yref: "paper", y0: 0, y1: 1, line: { color: "@muted", dash: "dot" } }] } });
    table(it, [
      { key: "name", label: "Indicator" },
      { key: "auc", label: "AUC", num: true, fmt: (v) => fmt(v, 2), title: "chance that a candidate scores higher than another good part" },
      { key: "capture", label: "Capture", num: true, fmt: (v) => pctS(v, 0), title: "candidates above the 95th percentile of the other good parts" },
      { key: "lift", label: "Lift", num: true, fmt: (v) => fmt(v, 1) + "×", title: "capture ÷ 5 %" },
      { key: "flagged_aff", label: "Flagged cand.", num: true, fmt: (v) => pctS(v), title: "candidates over the method's usual threshold" },
      { key: "flagged_ref", label: "Flagged good", num: true, fmt: (v) => pctS(v), title: "other good parts over the same threshold" },
    ], cd.indicators, { search: false });
  }

  // common factors
  const fBox = h("div"), fChart = h("div");
  out.append(h("h2", {}, "Common factors"), h("p", { class: "lead" }, "Are the candidates concentrated somewhere? χ² test of the candidate rate across the levels of each factor (Cramér's V = strength, 0–1). Wafer zone splits each wafer by radius: centre < 50 %, middle < 80 %, edge. Click a factor for its levels."), h("div", { class: "grid2" }, fBox, fChart));
  const showFactor = (f) => {
    const lv = f.levels.slice(0, 30);
    const [card, c] = chartBox();
    fChart.replaceChildren(card);
    plot(c, { data: [{ type: "bar", x: lv.map((l) => l.level), y: lv.map((l) => l.rate * 100), customdata: lv.map((l) => [l.n_aff, l.n]), marker: { color: lv.map((l) => (l.lift >= 2 ? "@fail" : "@s1")) }, hovertemplate: "%{x}<br>%{y:.2f}% candidates (%{customdata[0]} of %{customdata[1]})<extra></extra>" }],
      layout: { title: { text: `${f.name}: candidate rate per level`, x: 0.01, xanchor: "left", font: { size: 15 } }, xaxis: { type: "category", title: { text: "" } }, yaxis: { title: { text: "% candidates" } }, margin: { l: 60, r: 20, t: 56, b: 80 },
        shapes: [{ type: "line", xref: "paper", x0: 0, x1: 1, y0: (100 * cd.n_aff) / (cd.n_ref + cd.n_aff), y1: (100 * cd.n_aff) / (cd.n_ref + cd.n_aff), line: { color: "@muted", dash: "dot" } }] } });
  };
  const frows = cd.factors.map((f) => Object.assign({ top: f.levels[0].level, top_rate: f.levels[0].rate, top_lift: f.levels[0].lift, n_levels: f.levels.length }, f));
  table(fBox, [
    { key: "name", label: "Factor" },
    { key: "n_levels", label: "Levels", num: true },
    { key: "p", label: "p", num: true, fmt: fmtP, title: "χ² test: is the candidate rate the same on every level?" },
    { key: "cramers_v", label: "V", num: true, fmt: (v) => fmt(v, 2), title: "Cramér's V: strength of the association, 0–1" },
    { key: "top", label: "Highest level" },
    { key: "top_rate", label: "Rate", num: true, fmt: (v) => pctS(v, 2) },
    { key: "top_lift", label: "Lift", num: true, fmt: (v) => fmt(v, 1) + "×", title: "rate ÷ overall candidate rate" },
  ], frows, { search: false, onClick: showFactor });
  if (frows.length) showFactor(frows[0]);
  else fChart.replaceChildren(h("p", { class: "muted" }, "No factor with more than one level."));

  // stacked wafer map
  if (cd.stack.length) {
    const xs = [...new Set(cd.stack.map((p) => p.x))].sort((p, q) => p - q), ys = [...new Set(cd.stack.map((p) => p.y))].sort((p, q) => p - q);
    const z = ys.map(() => xs.map(() => null)), tx = ys.map(() => xs.map(() => ""));
    for (const p of cd.stack) { const i = ys.indexOf(p.y), j = xs.indexOf(p.x); z[i][j] = p.k ? (100 * p.k) / p.n : 0; tx[i][j] = `${p.k} of ${p.n}`; }
    const [card, c] = chartBox("tall");
    out.append(h("h2", {}, "Where on the wafer"), h("p", { class: "lead" }, `All wafers stacked: share of good parts at each die position that became bad candidates (die X/Y from ${cd.coords === "prr" ? "PRR" : "the wafer-map test settings"}).`), card);
    plot(c, { data: [{ type: "heatmap", x: xs, y: ys, z, text: tx, colorscale: "@seq", zmin: 0, xgap: 1, ygap: 1, colorbar: { title: { text: "% cand." }, thickness: 12 }, hovertemplate: "(%{x},%{y})<br>%{text} candidates<br>%{z:.1f}%<extra></extra>" }],
      layout: { title: { text: "Bad-candidate rate per die position", x: 0.01, xanchor: "left", font: { size: 15 } }, xaxis: { title: { text: "X" } }, yaxis: { title: { text: "Y" }, scaleanchor: "x", autorange: "reversed" }, margin: { l: 50, r: 20, t: 56, b: 44 } } });
  }

  // candidates
  const tb = h("div");
  out.append(h("h2", {}, "Candidates"), h("p", { class: "muted" }, "Anomaly %ile = rank of the combined anomaly score among the other good parts (99 = more abnormal than 99 % of them). Worst tests = largest deviations from the other good parts, in robust σ. Click a row to trace the part in both files."), tb);
  const cols = [
    { key: "key", label: "Identity" }, { key: "wafer", label: "Wafer" }, { key: "x", label: "X", num: true }, { key: "y", label: "Y", num: true }, { key: "site", label: "Site", num: true }, { key: "sbin", label: "Sbin", num: true },
    { key: "b_hbin", label: "Hbin in 2", num: true }, { key: "b_fails", label: "Fails in 2" },
    { key: "anomaly_pct", label: "Anomaly %ile", num: true, fmt: (v) => (v == null ? "–" : v.toFixed(0)) },
    { key: "sdc", label: "Tests >3σ", num: true }, { key: "worst", label: "Worst tests" },
  ];
  table(tb, cols, cd.parts, {
    sort: "anomaly_pct",
    onClick: (r) => trace(traceBox, Object.assign({ a: st.a, b: st.b, row_a: r.row }, R.keys, { value: r.key })),
    onClear: () => traceBox.replaceChildren(),
    tools: h("button", { onclick: () => SC.download(`${st.name}.csv`, SC.toCSV(cols, cd.parts)) }, "Download CSV"),
  });
  if (!(mk instanceof Error) && mk.rows.length) showFocus(S.bcFocus && mk.rows.some((r) => r.key === S.bcFocus) ? S.bcFocus : (R.ranked || mk.rows)[0].key).then(() => window.scrollTo(0, 0));
}

async function trace(el, params) {
  loading(el, "Tracing…");
  try {
    const t = await api("trace", params);
    const twoSided = params.b !== null && params.b !== undefined;
    const card = (label, id, p, hits) => {
      const raw = h("div");
      if (!p) return h("div", { class: "card pad" }, h("b", {}, `${label}: ${dsName(id)}`), h("p", { class: "muted" }, "Not found in this file."));
      return h("div", { class: "card pad" }, h("b", {}, `${label}: ${dsName(id)}`), h("p", {}, partSummary(p, p.wafer)),
        hits.length > 1 ? h("p", { class: "muted small" }, `${hits.length} test records of this unit (retests); showing the last (row ${p.index}).`) : null,
        h("div", { class: "row" }, h("button", { onclick: () => partRaw(raw, id, p.index) }, "Raw records")), raw);
    };
    const tb = h("div");
    el.replaceChildren(h("div", { class: "row tight", style: { justifyContent: "space-between" } }, h("h2", {}, `Trace: ${t.key || "–"}`), closeBtn(el)), h("p", { class: "muted" }, `Identity in A: ${t.key_a}`),
      h("div", { class: twoSided ? "grid2" : "" }, card("A", params.a, t.a, t.a_hits), twoSided ? card("B", params.b, t.b, t.b_hits) : null), tb);
    const pb = (v) => (v === null || v === undefined ? "" : v ? badge("in", "good") : badge("out", "bad"));
    const cols = [{ key: "key", label: "Test" }, { key: "units", label: "Units" }, { key: "lo", label: "Lo", num: true }, { key: "hi", label: "Hi", num: true }, { key: "a", label: "A", num: true }, { key: "a_pass", label: "", render: pb }];
    if (twoSided) cols.push({ key: "b", label: "B", num: true }, { key: "b_pass", label: "", render: pb }, { key: "diff", label: "B − A", num: true });
    if (t.tests.length) table(tb, cols, t.tests, { tools: h("button", { onclick: () => SC.download("trace.csv", SC.toCSV(cols, t.tests)) }, "Download CSV") });
    el.scrollIntoView({ behavior: "smooth", block: "start" });
  } catch (e) { showErr(el, e); }
}

// ---- Failure investigation (a case: files with roles, settings, steps)
function invCase() {
  const ids = S.datasets.map((d) => d.id);
  return S.inv || (S.inv = { name: "case 1", test: "", loadboard: S.lbTest || "5100", key: defaultKey(), good: "pass",
    lots: [{ name: "Lot A", kind: "issue", run1: ids[0], retest: ids[1] ?? ids[0], chain: [] }, { name: "Lot B", kind: "clean", run1: ids[2] ?? ids[0] }],
    screen: [], window: { from: "auto", mode: "strict", tol: 5, unit: "pct" }, maxRate: 10, minZone: 10 });
}
const INV_STEPS = [
  ["recovery", "1 · Retest and recovery window", "Follow each issue lot's rejects through its reject retests and find the value band that never recovers."],
  ["bad", "2 · Bad candidates", "Per issue lot: good in run 1, failing the test in the retest with the value inside the hard-bad window ($case_bad); outside it $case_marginal; passing the retest $case_good."],
  ["common", "3 · Commonality", "One table across the lots: site, loadboard, site × loadboard, wafer, zone, reticle position and test time for every group, flags for what all issue lots share and the clean lots do not, and the top marker tests side by side."],
  ["signature", "4 · Signature check", "Rules learned from the first issue lot's bad candidates, checked against the clean lots, the parts that stayed good on retest in the other issue lots and the confirmed good parts."],
  ["screen", "5 · Screen", "The signature applied to the other issue lots' unrecoverable and recovered parts and to the files to screen."],
];
function viewInvestigation(el) {
  el.append(h("h1", {}, "Failure investigation"), h("p", { class: "lead" }, "Set the case up once: which files are the issue lots (run 1, the retest where good parts failed, and the retests of those rejects), which are clean lots and which to screen, plus the failing test. Each step then reuses it, shows its result here and links to its full page. Use files from one tester type throughout (e.g. V93K only)."));
  if (S.datasets.length < 2) { el.append(h("div", { class: "help" }, "Open the case's files first (without 'combine'), or load a saved case below.")); }
  const c = invCase();
  const ids = S.datasets.map((d) => d.id);
  const fix = (x) => (ids.includes(x) ? x : ids[0]);
  for (const l of c.lots) { l.run1 = fix(l.run1); if (l.kind === "issue") { l.retest = fix(l.retest); l.chain = (l.chain || []).filter((x) => ids.includes(x)); } }
  c.screen = c.screen.filter((x) => ids.includes(x));
  const R = S.invRes || (S.invRes = {});
  const top = h("div"), files = h("div"), settings = h("div"), steps = h("div");
  el.append(top, files, settings, steps);

  // save / load
  const nameIn = h("input", { type: "text", value: c.name, style: { width: "160px" }, oninput: (e) => (c.name = e.target.value) });
  const savedSel = select([["", "saved cases…"]], "", () => {});
  api("case_list").then((l) => savedSel.replaceChildren(h("option", { value: "" }, l.length ? "saved cases…" : "no saved case"), ...l.map((x) => h("option", { value: x.path }, x.name)))).catch(() => {});
  const pathIn = h("input", { type: "text", placeholder: "or a case file path", style: { width: "220px" } });
  top.append(h("div", { class: "controls card pad" }, field("Case", nameIn),
    h("button", { onclick: () => saveCase(false) }, "Save"), h("button", { onclick: () => saveCase(true), title: "Save as <name>.stdf-case.json in the folder of the first lot's run 1" }, "Save next to the files"),
    field("Load", h("div", { class: "row tight" }, savedSel, pathIn)), h("button", { onclick: () => loadCase(pathIn.value.trim() || savedSel.value) }, "Load")));
  const pathOf = (id) => { const d = S.datasets.find((x) => x.id === id); return d && d.paths.length === 1 ? d.paths[0] : null; };
  async function saveCase(next) {
    const f = (id) => ({ path: pathOf(id), name: dsName(id) });
    const out = JSON.parse(JSON.stringify(c));
    out.lots = c.lots.map((l) => Object.assign({}, l, { run1: f(l.run1) }, l.kind === "issue" ? { retest: f(l.retest), chain: l.chain.map(f) } : {}));
    out.screen = c.screen.map(f);
    const missing = [out.lots.flatMap((l) => [l.run1, l.retest, ...(l.chain || [])]), out.screen].flat().filter((x) => x && !x.path);
    if (missing.length) return toast(`Only files loaded from disk can be saved in a case: ${missing.map((x) => x.name).join(", ")}`, "bad");
    let path = "";
    if (next) { const p = out.lots[0].run1.path; path = p.replace(/[\\/][^\\/]*$/, "") + "/" + c.name.replace(/[^\w-]+/g, "_") + ".stdf-case.json"; }
    try { const r = await api("case_save", { case: out, path }); toast(`Saved ${r.path}`, "good"); } catch (e) { toast(e.message, "bad"); }
  }
  async function loadCase(path) {
    if (!path) return toast("Pick a saved case or enter a path", "bad");
    try {
      const { case: k } = await api("case_load", { path });
      const all = [k.lots.flatMap((l) => [l.run1, l.retest, ...(l.chain || [])]), k.screen || []].flat().filter((x) => x && x.path);
      const open = (p) => S.datasets.find((d) => d.paths.length === 1 && d.paths[0] === p);
      const need = [...new Set(all.map((x) => x.path).filter((p) => !open(p)))];
      if (need.length) { toast(`Opening ${need.length} file(s)…`); await api("load", { paths: need }); await refreshDatasets(); }
      const id = (x) => (x && x.path && open(x.path) ? open(x.path).id : undefined);
      k.lots = k.lots.map((l) => Object.assign({}, l, { run1: id(l.run1) }, l.kind === "issue" ? { retest: id(l.retest), chain: (l.chain || []).map(id).filter((x) => x !== undefined) } : {}));
      k.screen = (k.screen || []).map(id).filter((x) => x !== undefined);
      S.inv = k; S.invRes = {};
      if (k.loadboard) { await api("loadboard", { test: k.loadboard }); S.lbTest = String(k.loadboard); }
      toast(`Loaded ${k.name}`, "good");
      go("investigation");
    } catch (e) { toast(e.message, "bad"); }
  }

  // files and roles
  const drawFiles = () => {
    const dsSel = (v, set) => select(dsOptions(), String(v), (x) => set(+x));
    const lotRows = c.lots.map((l, i) => {
      const chainBox = h("div", { class: "row tight" }, ...(l.chain || []).map((x, j) => h("span", { class: "row tight" }, dsSel(x, (v) => (l.chain[j] = v)), h("button", { title: "Remove", onclick: () => { l.chain.splice(j, 1); drawFiles(); } }, "×"))),
        h("button", { onclick: () => { l.chain.push(ids[ids.length - 1]); drawFiles(); } }, "+ reject retest"));
      return h("div", { class: "card pad", style: { marginBottom: "8px" } },
        h("div", { class: "controls" },
          field("Lot", h("input", { type: "text", value: l.name, style: { width: "110px" }, oninput: (e) => (l.name = e.target.value) })),
          field("Role", select([["issue", "issue lot"], ["clean", "clean lot"]], l.kind, (v) => { l.kind = v; if (v === "issue") { l.retest = l.retest ?? l.run1; l.chain = l.chain || []; } drawFiles(); })),
          field(l.kind === "issue" ? "Run 1 (parts good here)" : "Run 1", dsSel(l.run1, (v) => (l.run1 = v))),
          ...(l.kind === "issue" ? [field("Retest (good parts failed here)", dsSel(l.retest, (v) => (l.retest = v))), field("Retests of those rejects, in order", chainBox)] : []),
          h("button", { title: "Remove this lot", onclick: () => { c.lots.splice(i, 1); drawFiles(); } }, "Remove lot")));
    });
    const scr = h("div", { class: "row tight" }, ...c.screen.map((x, j) => h("span", { class: "row tight" }, dsSel(x, (v) => (c.screen[j] = v)), h("button", { onclick: () => { c.screen.splice(j, 1); drawFiles(); } }, "×"))),
      h("button", { onclick: () => { c.screen.push(ids[ids.length - 1]); drawFiles(); } }, "+ file to screen"));
    files.replaceChildren(h("h2", {}, "Files and roles"), ...lotRows,
      h("div", { class: "row" }, h("button", { onclick: () => { c.lots.push({ name: `Lot ${String.fromCharCode(65 + c.lots.length)}`, kind: "issue", run1: ids[0], retest: ids[0], chain: [] }); drawFiles(); } }, "+ Issue lot"),
        h("button", { onclick: () => { c.lots.push({ name: `Lot ${String.fromCharCode(65 + c.lots.length)}`, kind: "clean", run1: ids[0] }); drawFiles(); } }, "+ Clean lot")),
      h("div", { class: "controls" }, field("Files to screen (good parts)", scr)));
  };
  drawFiles();

  // settings
  const testIn = h("input", { type: "text", value: c.test, placeholder: "e.g. 1013", style: { width: "110px" }, oninput: (e) => (c.test = e.target.value.trim()) });
  const keySel = select(keyOptions().filter(([k]) => k !== "tests"), c.key || "part_txt", (v) => (c.key = v));
  settings.append(h("h2", {}, "Settings"), h("div", { class: "controls" }, field("Failing test", testIn), field("Identify parts by", keySel), field("Good parts are", h("input", { type: "text", class: "filter-input", value: c.good, oninput: (e) => (c.good = e.target.value) })),
    loadboardField(),
    h("button", { class: "primary", onclick: () => runAll() }, "Run all steps")));

  // steps
  const boxes = {};
  for (const [k, title, what] of INV_STEPS) {
    const status = h("span"), body = h("div");
    boxes[k] = { status, body };
    steps.append(h("div", { class: "card pad", style: { marginTop: "12px" } }, h("div", { class: "row" }, h("b", {}, title), status,
      h("button", { onclick: () => run(k) }, k === "screen" ? "Run (with step 4)" : "Run"), h("button", { onclick: () => openFull(k), title: "Open the full page with this case's settings" }, "Open page →")),
      h("p", { class: "muted small" }, what), body));
  }
  const issue = () => c.lots.filter((l) => l.kind === "issue"), clean = () => c.lots.filter((l) => l.kind === "clean");
  const g4 = (v) => (v == null || !isFinite(v) ? "–" : String(Number(v.toPrecision(5))));
  const pctS = (v, n) => (v == null || !isFinite(v) ? "–" : (v * 100).toFixed(n ?? 1) + "%");
  const setStatus = (k, s, msg) => { R[k + "_status"] = [s, msg]; boxes[k].status.replaceChildren(s ? badge(msg || s, s === "done" ? "good" : s === "error" ? "bad" : "neutral") : ""); };
  const need = () => { if (!c.test) throw new Error("Enter the failing test in Settings"); };

  async function stepRecovery() {
    need();
    R.recovery = [];
    for (const l of issue()) {
      if (!l.chain.length) { R.recovery.push({ lot: l.name, note: "no reject retest: add the retests of the rejects to find the recovery window" }); continue; }
      const d = await api("retest_trace", { runs: [l.retest, ...l.chain], key: c.key, filter: `fail(${c.test})`, tests: c.test, also: [l.run1], max_rate: c.maxRate / 100, min_zone: c.minZone });
      R.recovery.push({ lot: l.name, d });
    }
    pickWindow();
  }
  function pickWindow() {
    const cands = (R.recovery || []).filter((x) => x.d).map((x) => ({ lot: x.lot, z: x.d.recovery[0].zones.find((z) => z.kind === "hard_bad"), units: x.d.tests[0].units })).filter((x) => x.z);
    const w = c.window;
    const pick = w.from === "auto" ? cands.sort((a, b) => b.z.n - a.z.n)[0] : cands.find((x) => x.lot === w.from);
    R.window = pick ? Object.assign({ lot: pick.lot, units: pick.units, how: w.mode === "strict" ? "strict (observed)" : w.mode === "widened" ? "widened to the nearest recovered part" : `strict + ${w.tol}${w.unit === "pct" ? "% of band width" : " " + (pick.units || "")}` }, hardBadRange(pick.z, w.mode, w.tol, w.unit)) : null;
  }
  function drawRecovery() {
    const b = boxes.recovery.body;
    if (!R.recovery) return b.replaceChildren();
    const rows = R.recovery.map((x) => {
      if (!x.d) return { lot: x.lot, note: x.note };
      const z = (k) => x.d.recovery[0].zones.find((q) => q.kind === k);
      const f = (q) => (q ? `${g4(q.lo)} … ${g4(q.hi)}: ${q.recovered}/${q.n}` : "–");
      return { lot: x.lot, traced: x.d.n_parts, recovered: x.d.outcomes.recovered, still: x.d.outcomes.still_bad, units: x.d.tests[0].units, hard: f(z("hard_bad")), marginal: f(z("marginal")), extreme: f(z("extreme")) };
    });
    const t = h("div");
    const w = c.window;
    const fromSel = select([["auto", "auto (most hard-bad parts)"], ...issue().map((l) => [l.name, l.name])], w.from, (v) => { w.from = v; pickWindow(); drawRecovery(); });
    const modeSel = select([["strict", "strict (observed)"], ["widened", "widened to the nearest recovered part"], ["tolerance", "strict + tolerance"]], w.mode, (v) => { w.mode = v; pickWindow(); drawRecovery(); });
    const tolIn = h("input", { type: "number", min: 0, step: "any", value: w.tol, style: { width: "70px" }, hidden: w.mode !== "tolerance", onchange: (e) => { w.tol = +e.target.value || 0; pickWindow(); drawRecovery(); } });
    const unitSel = select([["pct", "% of band width"], ["abs", "absolute"]], w.unit, (v) => { w.unit = v; pickWindow(); drawRecovery(); });
    unitSel.hidden = w.mode !== "tolerance";
    b.replaceChildren(t, h("div", { class: "controls" }, field("Window from", fromSel), field("Range", h("div", { class: "row tight" }, modeSel, tolIn, unitSel)),
      h("span", { style: { paddingBottom: "8px" } }, R.window ? h("b", {}, `Hard-bad window: ${c.test} in ${g4(R.window.lo)} … ${g4(R.window.hi)} ${R.window.units || ""} (${R.window.how}, from ${R.window.lot})`) : h("span", { class: "muted" }, "No hard-bad window found: step 2 then takes every fail of the test."))));
    table(t, [{ key: "lot", label: "Lot" }, { key: "traced", label: "Rejects traced", num: true }, { key: "recovered", label: "Recovered", num: true }, { key: "still", label: "Still bad", num: true },
      { key: "marginal", label: "Marginal" }, { key: "hard", label: "Hard bad" }, { key: "extreme", label: "Extreme" }, { key: "note", label: "" }], rows, { search: false });
  }

  async function stepBad() {
    need();
    R.bad = [];
    const range = R.window ? `${g4(R.window.lo)}..${g4(R.window.hi)}` : "";
    for (const l of issue()) {
      const m = await api("candidate_sets", { a: l.run1, b: l.retest, key_a: c.key, key_b: c.key, filter_a: scoped(c.good), filter_b: `not pass and fail(${c.test})`, test: c.test, range, include_marginal: false, name: "case_bad", marginal: "case_marginal", confirmed: "case_good" });
      R.bad.push({ lot: l.name, m });
    }
  }
  function drawBad() {
    const b = boxes.bad.body;
    if (!R.bad) return b.replaceChildren();
    const t = h("div");
    b.replaceChildren(t);
    table(t, [{ key: "lot", label: "Lot" }, { key: "n_b", label: `Fail ${c.test} in retest`, num: true }, { key: "n_cand", label: "Hard bad ($case_bad)", num: true }, { key: "n_marginal", label: "Marginal", num: true }, { key: "n_confirmed", label: "Confirmed good", num: true }, { key: "rng", label: "Range" }],
      R.bad.map((x) => Object.assign({}, x.m, { lot: x.lot, rng: x.m.range ? `${g4(x.m.range.lo)} … ${g4(x.m.range.hi)} ${x.m.range.units || ""}` : "any fail" })), { search: false });
  }

  async function stepCommon() {
    need();
    const w = waferCal();
    const lots = c.lots.map((l) => l.kind === "issue"
      ? { name: l.name, ds: l.run1, kind: "issue", bad: "$case_bad", good: "$case_good", groups: [{ label: "marginal", filter: "$case_marginal" }, { label: "unrecoverable", filter: "$retest_still_bad" }, { label: "recovered", filter: "$retest_recovered" }] }
      : { name: l.name, ds: l.run1, kind: "clean", bad: `fail(${c.test})`, good: scoped(c.good) });
    R.common = await api("commonality", { lots, exclude: idExclude(), wafer_test: w.w, x_test: w.x, y_test: w.y, ox_test: w.ox, oy_test: w.oy, offset_mode: w.mode, flip_x: w.fx, flip_y: w.fy });
  }
  function drawCommon() {
    const b = boxes.common.body;
    const d = R.common;
    if (!d) return b.replaceChildren();
    const lots = d.lots;
    const ft = h("div"), detail = h("div"), mt = h("div");
    const flagged = d.factors.filter((f) => f.flag_within || f.flag_across);
    b.replaceChildren(h("div", { class: "kpis" }, ...lots.map((l) => kpi(l.name, `${l.n_bad} / ${l.n_good}`, `${l.kind === "clean" ? `fail ${c.test}` : "hard bad"} / good`))),
      h("ul", { class: "findings" }, ...(flagged.length ? flagged.map((f) => h("li", {}, h("b", {}, f.name), f.flag_across ? `: ${f.flag_across.level} holds ${f.flag_across.issue_share >= 1 ? "all" : "at least " + pctS(f.flag_across.issue_share, 0)} of every issue lot's bad parts, but only ${pctS(f.flag_across.clean_share, 0)} of the clean lots' parts.` : "", f.flag_within ? " Sets the bad parts apart within every issue lot (q < 0.05) and in no clean lot." : ""))
        : [h("li", {}, "No factor is shared by all issue lots and absent from the clean lots.")])),
      h("h3", {}, "Factors"), h("p", { class: "muted small" }, "Per lot: χ² test of bad vs good across the factor's levels (BH-corrected within the lot), the level with the highest bad rate and its lift. Click a factor for every group's share per level."), ft, detail,
      h("h3", {}, "Top marker tests, side by side"), h("p", { class: "muted small" }, "Each lot's bad vs good parts ranked as on Bad candidates (no random forest here); sorted by the lowest score across the issue lots, so tests that mark every issue lot come first."), mt);
    const frows = d.factors.map((f) => {
      const o = { key: f.key, name: f.name, flag: [f.flag_across ? `across: ${f.flag_across.level}` : "", f.flag_within ? "within" : ""].filter(Boolean).join(", ") };
      f.lots.forEach((l, i) => { o[`q${i}`] = l.test ? l.test.q : null; o[`t${i}`] = l.test ? `${l.test.top} (${fmt(l.test.top_lift, 1)}×)` : ""; });
      return o;
    });
    const showFactor = (fr) => {
      const f = d.factors.find((x) => x.key === fr.key);
      const cols = [{ key: "level", label: f.name }];
      const series = [];
      f.lots.forEach((l, i) => l.groups.forEach((g, j) => { cols.push({ key: `g${i}_${j}`, label: `${l.lot}: ${g.label} (${g.n})`, num: true, fmt: (v) => pctS(v) }); series.push([`${l.lot}: ${g.label}`, l.kind, g]); }));
      const rows = f.levels.map((lv) => { const o = { level: lv }; f.lots.forEach((l, i) => l.groups.forEach((g, j) => (o[`g${i}_${j}`] = g.n ? (g.counts[lv] || 0) / g.n : null))); return o; });
      const tt = h("div"), [card, ch] = chartBox();
      detail.replaceChildren(h("div", { class: "grid2" }, card, tt));
      const shown = series.filter(([n]) => / bad$| good$|unrecoverable|recovered/.test(n));
      plot(ch, { data: shown.map(([n, kind, g]) => ({ type: "bar", name: n, x: f.levels, y: f.levels.map((lv) => (g.n ? (100 * (g.counts[lv] || 0)) / g.n : 0)), hovertemplate: `${n}<br>%{x}: %{y:.1f}%<extra></extra>` })),
        layout: { title: { text: `${f.name}: share of each group per level`, x: 0.01, xanchor: "left", font: { size: 15 } }, barmode: "group", xaxis: { type: "category" }, yaxis: { title: { text: "% of group" } }, margin: { l: 55, r: 20, t: 56, b: 80 }, legend: { orientation: "h", y: -0.25 } } });
      table(tt, cols, rows, { search: false });
    };
    const fcols = [{ key: "name", label: "Factor" }, { key: "flag", label: "Flag", render: (v) => (v ? badge(v, "bad") : "") }];
    lots.forEach((l, i) => fcols.push({ key: `q${i}`, label: `${l.name} q`, num: true, fmt: fmtP, title: `χ² bad vs good in ${l.name}, BH-corrected` }, { key: `t${i}`, label: `${l.name} top level` }));
    table(ft, fcols, frows, { search: false, onClick: showFactor });
    if (frows.length) showFactor(flagged.length ? frows.find((r) => r.key === flagged[0].key) : frows[0]);
    const mcols = [{ key: "key", label: "Test" }, { key: "common", label: "Lowest issue-lot score", num: true, fmt: (v) => fmt(v, 2) }];
    lots.forEach((l, i) => d.marker_lots[i] && mcols.push({ key: `s${i}`, label: `${l.name} score`, num: true, fmt: (v, r) => (v == null ? "–" : `${fmt(v, 2)} ${r[`w${i}`]}`) }));
    table(mt, mcols, d.markers.map((m) => { const o = { key: m.key, common: m.common }; m.lots.forEach((s, i) => { o[`s${i}`] = s ? s.score : null; o[`w${i}`] = s ? s.why : ""; }); return o; }), { limit: 15, placeholder: "Search tests…" });
  }

  async function stepSignature() {
    need();
    const iss = issue();
    if (!iss.length) throw new Error("Add an issue lot");
    const first = iss[0], others = iss.slice(1);
    const screen = [...others.flatMap((l) => [{ ds: l.run1, filter: "$retest_still_bad", label: `${l.name} unrecoverable` }, { ds: l.run1, filter: "$retest_recovered", label: `${l.name} recovered` }]), ...c.screen.map((x) => ({ ds: x, filter: scoped(c.good) }))];
    R.sig = await api("signature", { a: first.run1, name: "case_bad", filter: scoped(c.good), exclude: idExclude(), exclude_set: "case_marginal", confirmed: "case_good", use_confirmed: true, max_rules: 5, range: S.co.range,
      clean: clean().map((l) => ({ ds: l.run1, filter: scoped(c.good) })), stayed: others.map((l) => ({ a: l.run1, b: l.retest, key: c.key, filter: scoped(c.good) })), screen });
    R.sigLot = first.name;
  }
  function drawSignature() {
    const b = boxes.signature.body, d = R.sig;
    if (!d) return b.replaceChildren();
    const sg = d.signature;
    b.replaceChildren(h("div", { class: "kpis" }, kpi("Learned from", R.sigLot, `${d.n_cand} bad candidates vs ${d.n_normal} ${d.normal}`), kpi("Reference pool", d.n_pool.toLocaleString(), d.sources.map((x) => x.name).join(" + ")),
      kpi("Rules kept", `${d.rows.filter((r) => r.verdict === "keep").length} + ${d.rows.filter((r) => r.verdict === "caution").length}`, `of ${d.rows.length} tests`)),
      sg.rules.length ? h("p", {}, `Signature: at least ${sg.m} of `, ...sg.rules.map((r, i) => h("span", {}, i ? ", " : "", h("b", {}, r.key), ` ${r.text} ${r.units || ""}`)), ` · catches ${pctS(sg.capture, 0)}, normal ${pctS(sg.fa_a)}, pooled reference ${pctS(sg.fa_ref)}`)
        : h("p", {}, "No rule survives the reference pool: nothing in run 1 sets the bad candidates apart from good parts that stayed good."));
  }
  function drawScreen() {
    const b = boxes.screen.body, d = R.sig;
    if (!d) return b.replaceChildren();
    const t = h("div");
    b.replaceChildren(t);
    if (!d.screen.length) { t.append(h("p", { class: "muted" }, "Nothing to screen: add a second issue lot or files to screen.")); return; }
    table(t, [{ key: "name", label: "Screened" }, { key: "verdict", label: "Verdict", render: (v) => badge(v, SCREEN_VERDICT[v] || "neutral") }, { key: "n", label: "Parts", num: true }, { key: "hits", label: "Match", num: true }, { key: "rate", label: "Rate", num: true, fmt: (v) => pctS(v, 1) }, { key: "p_ref", label: "p vs reference", num: true, fmt: fmtP }], d.screen, { search: false });
  }
  const STEP = { recovery: [stepRecovery, drawRecovery], bad: [stepBad, drawBad], common: [stepCommon, drawCommon], signature: [stepSignature, drawSignature], screen: [stepSignature, drawScreen] };
  async function run(k) {
    setStatus(k, "running", "running…");
    try {
      await STEP[k][0]();
      setStatus(k, "done", "done");
      STEP[k][1]();
      if (k === "recovery") drawBad();
      if (k === "signature" || k === "screen") { setStatus("signature", "done", "done"); setStatus("screen", "done", "done"); drawSignature(); drawScreen(); }
      return true;
    } catch (e) { setStatus(k, "error", "error"); boxes[k].body.replaceChildren(h("div", { class: "err" }, e.message)); return false; }
  }
  async function runAll() { for (const k of ["recovery", "bad", "common", "signature"]) if (!(await run(k))) break; }
  function openFull(k) {
    const iss = issue(), l = iss[0];
    if (k === "recovery" && l) { const x = (R.recovery || []).find((y) => y.d); const lot = iss.find((y) => x && y.name === x.lot) || l; S.rt = Object.assign(S.rt || {}, { runs: [lot.retest, ...lot.chain], mode: "test", tests: c.test }); S.rtLast = x ? x.d : null; go("retest"); }
    else if (k === "bad" && l) { S.bc = Object.assign(S.bc || {}, { a: l.run1, b: l.retest, good: c.good, test: c.test, how: "fail", range: R.window ? `${g4(R.window.lo)}..${g4(R.window.hi)}` : "", rangeNote: R.window ? `from the case's recovery window (${R.window.how}, ${R.window.lot})` : "", incMarg: false, extra: "", name: "case_bad", coords: "auto" }); S.bcLast = null; go("candidates"); }
    else if ((k === "signature" || k === "screen") && l) { Object.assign(sigState(), { a: l.run1, name: "case_bad", filter: c.good, confirmed: "case_good", noMarginal: true, clean: clean().map((x) => ({ ds: x.run1, filter: c.good })), stayed: iss.slice(1).map((x) => ({ a: x.run1, b: x.retest })) }); S.sgLast = R.sig || null; go("signature"); }
    else if (k === "common") toast("Commonality is shown here; open a lot in Bad candidates for its own common factors", "neutral");
  }
  // redraw earlier results
  for (const [k] of INV_STEPS) { const s = R[k + "_status"]; if (s) setStatus(k, s[0], s[1]); }
  drawRecovery(); drawBad(); drawCommon(); drawSignature(); drawScreen();
}

// ---- Retest trace
const RT_OUTCOME = { recovered: ["recovered", "@pass", "good"], still_bad: ["still bad", "@fail", "bad"], not_retested: ["not retested", "@warn", "warn"], passed_run1: ["stayed good", "@s1", "neutral"] };
const RT_STATUS = { pass: "pass", fail: "fail", not_retested: "not retested", passed_earlier: "passed earlier" };
/** A color token with transparency, for Sankey links. */
function tokA(name, a) {
  const c = getComputedStyle(document.documentElement).getPropertyValue("--" + name).trim();
  const m = /^#?([0-9a-f]{2})([0-9a-f]{2})([0-9a-f]{2})/i.exec(c);
  return m ? `rgba(${parseInt(m[1], 16)},${parseInt(m[2], 16)},${parseInt(m[3], 16)},${a})` : `rgba(128,128,128,${a})`;
}
/** The hard-bad range to send: strict (observed), widened (to the nearest recovered part) or strict + tolerance (never past it). */
function hardBadRange(z, mode, tol, unit) {
  if (mode === "widened") return { lo: z.safe_lo ?? z.lo, hi: z.safe_hi ?? z.hi };
  if (mode === "tolerance") {
    const d = unit === "abs" ? tol : ((z.hi - z.lo) * tol) / 100;
    return { lo: z.safe_lo != null ? Math.max(z.lo - d, z.safe_lo) : z.lo - d, hi: z.safe_hi != null ? Math.min(z.hi + d, z.safe_hi) : z.hi + d };
  }
  return { lo: z.lo, hi: z.hi };
}
function viewRetest(el) {
  el.append(h("h1", {}, "Retest recovery"), h("p", { class: "lead" }, "Recovery analysis of a retest chain (experimental; the Trace page follows parts across any files). Follow parts through a chain of runs: run 1, then a retest of its rejects, then a retest of those rejects, and so on; a run may also retest good parts (e.g. the good parts of runs 1 and 2), and their rejects are followed on. Pick the starting parts in run 1 (e.g. the ones failing test 1013); each later run is searched for the same physical part, so you see per run whether it passed or failed, its bin and the traced test's value, and how the population shrinks."));
  if (S.datasets.length < 2) { el.append(h("div", { class: "help" }, "Open run 1 and at least one retest file (without 'combine').")); return; }
  const ids = S.datasets.map((d) => d.id);
  const st = S.rt || (S.rt = { runs: ids.slice(0, 3), mode: "reject", custom: "", tests: "", valueFrom: "first", maxRate: 10, minZone: 10 });
  st.runs = st.runs.filter((x) => ids.includes(x));
  while (st.runs.length < 2) { const o = ids.find((x) => !st.runs.includes(x)); if (o === undefined) break; st.runs.push(o); }
  const runsBox = h("div");
  const drawRuns = () => {
    runsBox.replaceChildren(...st.runs.map((id, i) => h("div", { class: "row tight", style: { marginBottom: "6px" } },
      h("span", { style: { width: "54px" } }, `Run ${i + 1}`),
      select(dsOptions(), String(id), (v) => { st.runs[i] = +v; updHint(); }),
      h("button", { title: "Move up", disabled: i === 0, onclick: () => { [st.runs[i - 1], st.runs[i]] = [st.runs[i], st.runs[i - 1]]; drawRuns(); updHint(); } }, "↑"),
      h("button", { title: "Move down", disabled: i === st.runs.length - 1, onclick: () => { [st.runs[i + 1], st.runs[i]] = [st.runs[i], st.runs[i + 1]]; drawRuns(); updHint(); } }, "↓"),
      h("button", { title: "Remove", disabled: st.runs.length <= 2, onclick: () => { st.runs.splice(i, 1); drawRuns(); updHint(); } }, "×"),
      h("span", { class: "muted small" }, i === 0 ? "first run: the starting parts" : "a retest: of the rejects, or of good parts"))),
      h("button", { onclick: () => { st.runs.push(ids.find((x) => !st.runs.includes(x)) ?? ids[0]); drawRuns(); } }, "+ Add run"));
  };
  const kp = keyPicker("rt", () => st.runs[0]);
  if (st.link == null) st.link = "part_txt";
  const link = select([["", "nothing (key only)"], ...keyOptions().filter(([k]) => k !== "tests")], st.link, (v) => (st.link = v));
  link.title = "Where a part has no key in a run (e.g. it failed before the WXY test), find it by this ID instead, from the last run it was found in. Never joins parts whose keys differ.";
  const mode = select([["reject", "rejects of run 1 (not pass)"], ["all", "all parts of run 1 (when a run retests good parts)"], ["test", "failing the traced test"], ["custom", "custom filter"]], st.mode, (v) => { st.mode = v; custom.hidden = v !== "custom"; });
  const custom = h("input", { type: "text", class: "filter-input", value: st.custom, placeholder: "e.g. fail(1013) and site = 2", hidden: st.mode !== "custom", oninput: (e) => (st.custom = e.target.value) });
  const tests = h("input", { type: "text", value: st.tests, placeholder: "blank = most common first fail", style: { width: "190px" }, title: "Test(s) to trace, e.g. 1013 or 1013, 1290. Values are shown in each test's display unit (logged UNITS scaled by RES_SCAL).", oninput: (e) => { st.tests = e.target.value; updHint(); } });
  const unitHint = h("b", { class: "small" });
  const out = h("div"), traceBox = h("div");
  async function updHint() {
    const q = tests.value.split(",")[0].trim().replace(/:.*/, "");
    unitHint.textContent = "";
    if (!q) return;
    try { const t = (await testsFor(st.runs[0], "")).find((x) => x.key === q || String(x.num) === q || x.name === q); unitHint.textContent = t ? t.disp_units || t.units || "" : "not in run 1"; } catch (e) { }
  }
  const run = async () => {
    const first = st.tests.split(",")[0].trim().replace(/:.*/, "");
    if (st.mode === "test" && !first) return toast("Enter the test to trace first", "bad");
    const filter = st.mode === "reject" ? "not pass" : st.mode === "all" ? "all" : st.mode === "test" ? `fail(${first})` : st.custom.trim() || "not pass";
    loading(out, "Following the parts through the runs…");
    try {
      const d = await api("retest_trace", { runs: st.runs, key: kp.value(), link: st.link, filter: scoped(filter), tests: st.tests, exclude: idExclude(), id_tests: idTests(true), value_from: st.valueFrom || "first", max_rate: (st.maxRate ?? 10) / 100, min_zone: st.minZone ?? 10 });
      S.rtLast = d;
      renderRetest(out, traceBox, d);
    } catch (e) { showErr(out, e); }
  };
  S.rtRun = run;
  drawRuns();
  updHint();
  el.append(
    h("div", { class: "grid2" },
      h("div", { class: "card pad" }, h("b", {}, "Runs, in test order"), runsBox),
      h("div", { class: "card pad" }, h("b", {}, "Parts and tests"),
        h("div", { class: "controls" }, field("Identify parts by", kp.el), field("Then link by", link)),
        h("p", { class: "muted small" }, "Part text (or wafer + X/Y) is safest: part IDs often restart at 1 in every run. The link follows parts on where the key is missing, e.g. a part that failed before its WXY test."),
        h("div", { class: "controls" }, field("Start with", h("div", { class: "row tight" }, mode, custom))),
        h("div", { class: "controls" }, field("Trace test(s)", h("div", { class: "row tight" }, tests, unitHint)), loadboardField(), h("button", { class: "primary", onclick: run }, "Trace")))),
    out, traceBox);
  if (S.rtLast) renderRetest(out, traceBox, S.rtLast);
}
/** ID audit kinds: label, chart color, badge kind, what it means. */
const RT_ID = {
  key_shared: ["key shared", "@fail", "bad", "One key (e.g. WXY) on parts with different part texts in one run: a corrupt EEPROM or a constant value. Not used for matching; the part text matches instead."],
  link_shared: ["link shared", "@warn", "warn", "One link (e.g. part text) on parts with different keys in one run: a 2D code misread or copied to another part. Not used for matching; the key matches instead."],
  suspect_key: ["placeholder key", "@s3", "warn", "A key that looks like a reader default (all zeros, all ones, one character repeated). Not used for matching when there is a link."],
  suspect_link: ["placeholder link", "@s4", "warn", "A link that looks like a reader default. Not used for matching when there is a key."],
  link_changed: ["link changed", "@s1", "warn", "Found by its key, but with another part text than before: camera out of sync or a wrong sort. The part keeps its first part text."],
  key_conflict: ["key conflict", "@s7", "bad", "Its part text is on a row with another key in this run: not joined, so it may show as not retested."],
  ambiguous: ["ambiguous", "@s6", "bad", "Not found in this run because its own key or part text is on several parts there: not guessed, so it may show as not retested."],
};
const rtId = (k) => RT_ID[k] || [k, "@muted", "neutral", ""];
/** ID audit: per run, IDs shared by different parts or placeholder values, and parts whose IDs disagree between runs. */
function renderIdAudit(out, d, pick) {
  const a = d.audit;
  if (!a) return;
  const rl = (k) => `Run ${k + 1}`;
  const box = h("div");
  out.append(h("h2", {}, "ID audit"), h("p", { class: "muted" }, "Part text and WXY should each belong to one part. A corrupt EEPROM can give many parts the same WXY (e.g. all zeros), and a camera out of sync or a wrong sort can put a 2D code on another part. Such values are kept out of matching when the other ID can do it, so they never merge different parts. Click a row to highlight its part."), box);
  if (!a.checked) box.append(h("div", { class: "help" }, "Pick a link (Then link by) to check the key and the link against each other. Without one, only placeholder values are found."));
  const rows = [
    ...a.issues.map((i) => ({ run: i.run, kind: i.kind, value: i.value, pattern: i.pattern || "", n: i.n_rows, others: i.others.join(", ") + (i.n_others > i.others.length ? ` +${i.n_others - i.others.length}` : ""), parts: i.parts })),
    ...a.flags.map((f) => ({ run: f.run, kind: f.kind, value: f.detail, pattern: "", n: 1, others: "", parts: [f.part] })),
  ];
  if (!rows.length) { box.append(h("p", { class: "muted small" }, "No ID problems found.")); return; }
  box.append(h("div", { class: "kpis" },
    kpi("ID issues", a.n_issues.toLocaleString(), "shared or placeholder values, over all runs"),
    kpi("Parts flagged", a.n_flagged.toLocaleString(), "link changed or key conflict between runs"),
    kpi("Left out of matching", `${a.dropped_keys} key${a.dropped_keys === 1 ? "" : "s"}, ${a.dropped_links} link${a.dropped_links === 1 ? "" : "s"}`, "distinct values, in the runs they were found in")),
  h("div", { class: "row" }, h("button", { onclick: () => useFilter(d.runs[0].id, "$rt_id_issue") }, "Analyse ID issues ($rt_id_issue)")));
  const [card, c] = chartBox();
  box.append(card);
  const xs = d.runs.map((_, k) => rl(k));
  plot(c, { data: Object.entries(RT_ID).filter(([k]) => rows.some((r) => r.kind === k)).map(([k, [lab, col]]) => ({ type: "bar", name: lab, x: xs, y: d.runs.map((_, run) => rows.filter((r) => r.kind === k && r.run === run).length), marker: { color: col }, hovertemplate: `%{x}: %{y} ${lab}<extra></extra>` })),
    layout: { title: { text: "ID problems per run", x: 0.01, xanchor: "left", font: { size: 15 } }, barmode: "stack", xaxis: { type: "category" }, yaxis: { title: { text: "issues / flagged parts" } }, margin: { l: 55, r: 20, t: 56, b: 40 }, legend: { orientation: "h", x: 1, xanchor: "right", y: 1.0, yanchor: "bottom" } } });
  const tb = h("div");
  box.append(tb);
  table(tb, [
    { key: "run", label: "Run", fmt: (v) => rl(v) },
    { key: "kind", label: "Problem", render: (v) => h("span", { title: rtId(v)[3] }, badge(rtId(v)[0], rtId(v)[2])) },
    { key: "value", label: "Value" },
    { key: "pattern", label: "Pattern" },
    { key: "n", label: "Rows", num: true },
    { key: "others", label: "Other IDs on those rows" },
    { key: "parts", label: "Traced parts", fmt: (v) => (v.length > 5 ? `${v.slice(0, 5).join(", ")} +${v.length - 5}` : v.join(", ")) },
  ], rows, { placeholder: "Search value, part text, WXY…", onClick: (r) => { if (r.parts.length) pick(r.parts[0]); } });
}
/** The highlighted part's IDs in every run, with how it was found; values that disagree with its first ones are marked. */
function idTrail(d, p) {
  if (!p) return null;
  // a value the audit found shared or a placeholder is struck through; the
  // others are compared with the part's first good value
  const BAD = { id_key: ["key_shared", "suspect_key"], id_link: ["link_shared", "suspect_link"] };
  const bad = (r, f) => BAD[f].includes(r.id_issue);
  const first = (f) => p.runs.filter((r) => !bad(r, f)).map((r) => r[f]).find((v) => v != null);
  const k0 = first("id_key"), l0 = first("id_link");
  const cell = (r, f, ref) => { const v = r[f]; return h("td", { class: v == null ? "" : bad(r, f) ? "id-bad" : ref != null && v !== ref ? "id-diff" : "" }, v ?? "–"); };
  const flag = (k) => (p.flags || []).filter((f) => f.run === k);
  const checks = (r, k) => [
    ...(r.id_issue ? [[r.id_issue, ""]] : []),
    ...flag(k).map((f) => [f.kind, ` (${f.detail})`]),
  ].map(([x, more]) => h("span", { title: rtId(x)[3] + more }, badge(rtId(x)[0], rtId(x)[2])));
  const found = (r, k) => (r.row == null ? (flag(k).length ? "not joined" : "–") : r.linked ? "link" : k ? "key" : "start");
  const row = (label, cells) => h("tr", {}, h("th", {}, label), ...cells);
  return h("div", { class: "id-trail" }, h("table", {},
    h("thead", {}, row("IDs", p.runs.map((_, k) => h("th", {}, `R${k + 1}`)))),
    h("tbody", {},
      row("Key", p.runs.map((r) => cell(r, "id_key", k0))),
      d.link ? row("Link", p.runs.map((r) => cell(r, "id_link", l0))) : null,
      row("Found by", p.runs.map((r, k) => h("td", { class: "muted small" }, found(r, k)))),
      row("Check", p.runs.map((r, k) => h("td", {}, ...checks(r, k)))))));
}
/** Parts table search: any trace value, and wafer X Y typed as W3X12Y-4 or 3, 12, -4 (or with spaces, slashes or |). */
function rtSearch(r, q) {
  if (r._hay.includes(q)) return true;
  const m = /^\s*(-?\d+)[\s,;/|]+(-?\d+)[\s,;/|]+(-?\d+)\s*$/.exec(q);
  return !!m && r._trace.wxy_t === wxyText(m[1], m[2], m[3]);
}
/** The WXY test against the wafer, X and Y tests, on the basis chosen in Settings: raw (the tests as logged, default) or die (the offset tests added). Reads as wafer, then X and Y zero-padded to three digits (17113138 = 17|113|138). Null without WXY and the three tests. */
function wxyCheck(ids) {
  if (!ids || ids.wxy == null || ids.w == null || ids.x_t == null || ids.y_t == null) return null;
  const w = waferCal(), i3 = (v) => String(Math.round(v)).padStart(3, "0");
  const fits = (v) => Math.round(v) >= 0 && Math.round(v) <= 999;
  const enc = (wf, x, y) => (fits(x) && fits(y) ? `${Math.round(wf)}${i3(x)}${i3(y)}` : "");
  const die = (v, o) => (v == null ? null : o == null ? v : w.mode === "sub" ? v - o : v + o);
  const wxy = String(Math.round(ids.wxy)), raw = enc(ids.w, ids.x_t, ids.y_t), dies = enc(ids.w, die(ids.x_t, ids.ox), die(ids.y_t, ids.oy));
  const basis = wxyBasis(), want = basis === "die" ? dies : raw, other = basis === "die" ? raw : dies;
  if (want && wxy === want) return { ok: true, text: `= ${basis} W,X,Y`, title: basis === "die" ? "WXY matches the wafer, die X and Y (offset tests applied)." : "WXY matches the wafer, X and Y tests as logged." };
  const hint = other && wxy === other ? ` It matches the ${basis === "die" ? "raw" : "die"} W,X,Y: change Check WXY against in Settings.` : "";
  return { ok: false, text: `≠ ${basis} W,X,Y`, title: `WXY ${wxy} is not the ${basis} W,X,Y (${want || "no 2-digit wafer, 3-digit X and Y"}).${hint}` };
}
/** The WXY check as a badge for the part card. */
function wxyBadge(ids) {
  const c = wxyCheck(ids);
  if (!c) return null;
  const b = badge(c.text, c.ok ? "good" : "bad");
  b.title = c.title;
  return b;
}
/** The part trace card: the trace fields picked here (kept in Settings), for one part. */
function partCard(p, redraw) {
  if (!p) return null;
  const tv = traceValues(p.ids), all = traceFields();
  const pick = (S.cfg.card || []).filter((k) => all.some(([x]) => x === k));
  const chips = all.filter(([k]) => pick.includes(k)).map(([k, l]) => {
    const c = h("span", { class: "trace-chip" }, h("span", { class: "muted small" }, l), h("b", {}, tv[k] || "–"), k === "wxy" ? wxyBadge(p.ids) : null);
    if (k === "wxy_t") c.title = wxyBasis() === "die" ? "Die X and Y: the X and Y tests with the offset tests applied. WXY is checked against these." : "X and Y as the tests logged them (offset tests not added). WXY is checked against these.";
    return c;
  });
  const menu = h("details", { class: "trace-pick", open: !!S.tracePickOpen }, h("summary", { title: "Pick the trace fields to show", onclick: (e) => (S.tracePickOpen = !e.currentTarget.parentNode.open) }, "Fields"),
    h("div", { class: "trace-menu" }, all.map(([k, l]) => h("label", { class: "chk" }, h("input", { type: "checkbox", checked: pick.includes(k), onchange: (e) => {
      S.cfg.card = e.target.checked ? [...pick, k] : pick.filter((x) => x !== k); saveCfg(); redraw();
    } }), l))));
  return h("div", { class: "trace-card" }, chips.length ? chips : h("span", { class: "muted small" }, "No trace field picked."), menu);
}
function renderRetest(out, traceBox, d) {
  const pctS = (v, n) => (v == null || !isFinite(v) ? "–" : (v * 100).toFixed(n ?? 1) + "%");
  const N = d.runs.length, o = d.outcomes;
  const rl = (k) => `Run ${k + 1}`;
  out.replaceChildren(h("div", { class: "kpis" },
    kpi("Traced parts", d.n_parts.toLocaleString(), `run 1: ${d.filter}`),
    kpi("Recovered", o.recovered.toLocaleString(), `${pctS(o.recovered / ((o.recovered + o.still_bad + o.not_retested) || 1))} of the parts that failed passed a later retest`),
    kpi("Still bad", o.still_bad.toLocaleString(), `failed in ${rl(N - 1)}`),
    kpi("Not retested", o.not_retested.toLocaleString(), "failed, then no match in a later run"),
    o.passed_run1 ? kpi("Stayed good", o.passed_run1.toLocaleString(), "passed run 1 and never failed after") : "",
    kpi("Matched by", d.key_desc, d.link ? `${(d.n_linked || 0).toLocaleString()} parts found by the link in a run without their key` : "")),
  h("div", { class: "row" },
    h("button", { onclick: () => useFilter(d.runs[0].id, "$retest_recovered") }, "Analyse recovered ($retest_recovered)"),
    h("button", { onclick: () => useFilter(d.runs[0].id, "$retest_still_bad") }, "Analyse still bad ($retest_still_bad)"),
    o.not_retested ? h("button", { onclick: () => useFilter(d.runs[0].id, "$retest_not_retested") }, "Not retested ($retest_not_retested)") : ""),
  retestHelp());

  // per-run summary
  const t0 = d.tests[0];
  const lbOf = (r) => { const v = [...new Set((r.loadboard || []).filter((x) => x != null))]; return v.length ? `LB ${v.join("/")}${r.loadboard_changed ? " (changes in file!)" : ""}` : ""; };
  const srows = d.summary.map((s, k) => Object.assign({ name: `${rl(k)}: ${d.runs[k].name}`, lb: lbOf(d.runs[k]) }, s));
  const sb = h("div");
  out.append(h("h2", {}, "Run by run"), sb);
  table(sb, [
    { key: "name", label: "Run" },
    { key: "lb", label: "Loadboard", title: `test ${d.loadboard_test}: the most common value in the file (parts without it take the file's board)` },
    { key: "due", label: "Due", num: true, title: "parts still bad coming into this run (run 1: every traced part)" },
    { key: "retested", label: "Retested", num: true, title: "of those, found in this run" },
    { key: "pass", label: "Pass", num: true, title: "passed in this run (recovered, from run 2 on)" },
    { key: "fail", label: "Fail", num: true },
    { key: "fail_test", label: `Fail ${t0.key}`, num: true, title: "failing the traced test here" },
    { key: "fail_other", label: "Fail other", num: true, title: "failing, but not on the traced test" },
    { key: "not_retested", label: "Not retested", num: true, title: "still bad but not found in this run" },
    { key: "retest_good", label: "Retest of good", num: true, fmt: (v, r) => (v ? `${v} (${r.retest_good_fail} fail)` : "–"), title: "parts that had passed in an earlier run and were tested again here; those failing are due for the next run" },
    { key: "cum_recovery_rate", label: "Cum. recovered", num: true, fmt: (v, r) => (r.run ? `${r.cum_recovered} (${pctS(v)})` : "–"), title: "parts that passed a retest up to this run, over run 1's fails" },
  ], srows, { search: false });
  if (d.by_board.length) {
    const bb = h("div");
    out.append(h("h3", {}, "Recovery by loadboard"), h("p", { class: "muted small" }, `Parts due for a retest, by the loadboard (test ${d.loadboard_test}) of the run that retested them. 95% Wilson interval in brackets. Each run only gets the previous run's rejects, so boards on different runs see different parts: compare boards at the same run position (e.g. run 2 of two lots).`), bb);
    table(bb, [
      { key: "run", label: "Run", fmt: (v) => rl(v) }, { key: "board", label: "Loadboard" }, { key: "retested", label: "Retested", num: true }, { key: "pass", label: "Recovered", num: true },
      { key: "rate", label: "Recovery", num: true, fmt: (v, r) => `${pctS(v)} [${pctS(r.ci_lo, 0)}–${pctS(r.ci_hi, 0)}]` },
    ], d.by_board, { search: false });
  }

  // an audit row highlights its part and shows its ID trail (above the charts)
  renderIdAudit(out, d, (k) => { if (S.rtHi !== k) focusPart(k, "chart"); focusBox.scrollIntoView({ behavior: "smooth", block: "start" }); });
  renderRetestCandidates(out, d);

  // flow
  const nodes = [], idx = {};
  // flows carry labels ("passed earlier"); the lookups use status keys
  const node = (k, s) => { s = s.replace(" ", "_"); const n = `${rl(k)} ${RT_STATUS[s]}`; if (!(n in idx)) { idx[n] = nodes.length; nodes.push([n, s]); } return idx[n]; };
  const links = d.flows.map((f) => ({ s: node(f.run, f.from), t: node(f.run + 1, f.to), n: f.n, to: f.to.replace(" ", "_") }));
  const nodeCol = { pass: "@pass", fail: "@fail", not_retested: "@warn", passed_earlier: "@muted" };
  const linkCol = { pass: "pass", fail: "fail", not_retested: "warn", passed_earlier: "muted" };
  // stacked full width so up to 8 runs stay readable
  const g = h("div", { class: "grid1" });
  // the highlighted part, its trace card and a part search, above the charts
  const focusBox = h("div", { class: "card pad focus-box" });
  out.append(h("h2", {}, "Flow and trajectories"), focusBox, g);
  {
    const [card, c] = chartBox("tall");
    g.append(card);
    plot(c, { data: [{ type: "sankey", arrangement: "snap", node: { label: nodes.map(([n]) => n), color: nodes.map(([, s]) => nodeCol[s]), pad: 18, thickness: 16, line: { width: 0 } },
      link: { source: links.map((l) => l.s), target: links.map((l) => l.t), value: links.map((l) => l.n), color: links.map((l) => tokA(linkCol[l.to], 0.35)), hovertemplate: "%{source.label} → %{target.label}: %{value}<extra></extra>" } }],
      layout: { title: { text: "Where the fails went (parts that never failed left out)", x: 0.01, xanchor: "left", font: { size: 15 } }, margin: { l: 10, r: 10, t: 56, b: 20 } } });
  }
  // trajectory: one line per part over the runs, box per run behind
  const trajBox = h("div", { class: "card" });
  g.append(trajBox);
  // one highlighted part across the charts and the parts table; click its
  // line or a point again (or "× clear") to drop it, Reset view for all
  // a chart click only highlights the part and narrows the parts table to
  // it (the trend stays in view); a click in the table also traces it
  let tblApi = null, flat = [], onlyNote = null;
  const tracePart = () => {
    const p = d.parts.find((x) => x.key === S.rtHi);
    if (!p) return;
    const last = p.runs.map((x) => x.row != null).lastIndexOf(true);
    const kb = last > 0 ? last : 1;
    trace(traceBox, Object.assign({ a: d.runs[0].id, b: d.runs[kb].id, row_a: p.runs[0].row }, p.runs[kb].row != null ? { row_b: p.runs[kb].row } : {}, { key_a: d.key, key_b: d.key, value: S.rtHi }));
  };
  const showOnly = (key) => {
    S.rtOnly = key || null;
    if (!tblApi) return;
    tblApi.setRows(S.rtOnly ? flat.filter((r) => r.key === S.rtOnly) : flat);
    if (S.rtHi) tblApi.select((r) => r.key === S.rtHi); else tblApi.clear();
    onlyNote.replaceChildren(...(S.rtOnly ? [h("span", { class: "muted small" }, "Showing only the part clicked on the chart:"), h("b", {}, S.rtOnly), h("button", { class: "ghost small", onclick: () => showOnly(null) }, "Show all parts")] : []));
    onlyNote.hidden = !S.rtOnly;
  };
  const focusPart = (key, from) => {
    S.rtHi = key && key !== S.rtHi ? key : null;
    drawTraj(); drawDelta();
    if (from === "chart" || !S.rtHi) showOnly(from === "chart" ? S.rtHi : null);
    else if (tblApi) tblApi.select((r) => r.key === S.rtHi);
    if (!S.rtHi || from === "chart") { traceBox.replaceChildren(); return; }
    tracePart();
  };
  const resetView = () => { S.rtHi = null; S.rtOnly = null; traceBox.replaceChildren(); renderRetest(out, traceBox, d); };
  // typing a part works like clicking it on a chart: exact matches (key,
  // any trace value, wafer|X|Y) first, else parts containing the text
  const findIn = h("input", { type: "text", placeholder: "part, part text, WXY or 3|12|-4", style: { width: "240px" }, onkeydown: (e) => { if (e.key === "Enter") findPart(); } });
  const hits = h("span", { class: "row tight" }), focusDyn = h("span", { class: "row tight" }), cardSlot = h("div");
  const findPart = () => {
    const q = findIn.value.trim().toLowerCase();
    hits.replaceChildren();
    if (!q) return;
    const exact = flat.filter((r) => r.key.toLowerCase() === q || Object.values(r._trace).some((v) => String(v).toLowerCase() === q) || (!r._hay.includes(q) && rtSearch(r, q)));
    const m = exact.length ? exact : flat.filter((r) => r.key.toLowerCase().includes(q) || rtSearch(r, q));
    if (!m.length) { hits.append(h("span", { class: "muted small" }, "No traced part matches.")); return; }
    if (m[0].key !== S.rtHi) focusPart(m[0].key, "chart");
    if (m.length > 1) hits.append(h("span", { class: "muted small" }, `${m.length.toLocaleString()} parts match:`), ...m.slice(0, 8).map((r) => h("button", { class: "ghost small", onclick: () => { if (r.key !== S.rtHi) focusPart(r.key, "chart"); } }, r.key)), m.length > 8 ? h("span", { class: "muted small" }, "…") : null);
  };
  const drawFocus = () => {
    focusDyn.replaceChildren(...(S.rtHi ? [h("span", { class: "muted small" }, "Highlighted:"), h("b", {}, S.rtHi), h("button", { class: "ghost small", title: "Clear the highlight", onclick: () => focusPart(null) }, "× clear"), h("button", { class: "small", title: "All results of this part in run 1 and the last run it was found in", onclick: tracePart }, "Trace this part")]
      : [h("span", { class: "muted small" }, "Find a part, or click a line or point, to highlight it and narrow the parts table to it (click it again to clear). Drag to zoom, double-click to zoom out.")]));
    const hp = S.rtHi ? d.parts.find((x) => x.key === S.rtHi) : null;
    cardSlot.replaceChildren(...(hp ? [partCard(hp, drawFocus), idTrail(d, hp)] : []));
  };
  focusBox.append(h("div", { class: "row tight", style: { justifyContent: "space-between" } },
    h("span", { class: "row tight" }, field("Find in traced parts", h("div", { class: "row tight" }, findIn, h("button", { onclick: findPart }, "Find"))), hits),
    h("button", { class: "small", title: "Clear the highlighted part, the parts table search and every chart zoom", onclick: resetView }, "↺ Reset view")),
    h("div", { class: "row tight", style: { marginTop: "6px" } }, focusDyn), cardSlot);
  const onPick = (el) => el.on && el.on("plotly_click", (ev) => { const k = ev.points && ev.points[0] && ev.points[0].customdata; if (typeof k === "string") focusPart(k, "chart"); });
  let ti = Math.min(S.rtTest || 0, d.tests.length - 1);
  const clip = (s, n) => (s.length > n ? s.slice(0, n - 1) + "…" : s);
  const jit = (key) => { let x = 0; for (const c of key) x = (x * 31 + c.charCodeAt(0)) | 0; return ((x >>> 0) % 1000) / 1000 * 0.24 - 0.12; };
  const drawTraj = () => {
    const t = d.tests[ti];
    const sel = d.tests.length > 1 ? select(d.tests.map((x, i) => [String(i), x.key]), String(ti), (v) => { ti = S.rtTest = +v; drawTraj(); drawRec(); drawDelta(); }) : null;
    const [card, c] = chartBox("tall");
    drawFocus();
    trajBox.replaceChildren(...(sel ? [h("div", { class: "controls", style: { padding: "8px 12px 0" } }, field("Test", sel))] : []), card.firstChild);
    const bx = [], by = [];
    const lines = {}, pts = { pass: [[], [], [], []], fail: [[], [], [], []], other: [[], [], [], []] };
    for (const p of d.parts) {
      const j = jit(p.key);
      const L = lines[p.outcome] || (lines[p.outcome] = [[], [], []]);
      p.runs.forEach((r, k) => {
        const v = r.values ? r.values[ti] : null;
        if (v == null) return;
        bx.push(k + 1); by.push(v);
        L[0].push(k + 1 + j); L[1].push(v); L[2].push(p.key);
        const c = rtClass(r, ti);
        const P = pts[c];
        P[3].push(p.key);
        P[0].push(k + 1 + j); P[1].push(v); P[2].push(`${p.key}<br>${rl(k)}: ${RT_STATUS[r.status]}${r.retest_good ? " (retest of good)" : ""}, site ${r.site}, bin ${r.hbin}${c === "other" ? `<br>passes ${t.key}; ${rtOther(r)}` : ""}`);
      });
      L[0].push(null); L[1].push(null); L[2].push(null);
    }
    // with a part highlighted, the others fade back
    const dim = S.rtHi ? 0.3 : 1;
    const data = [{ type: "box", x: bx, y: by, name: "distribution", boxpoints: false, width: 0.55, fillcolor: tokA("muted", 0.12), line: { color: tokA("muted", 0.6), width: 1 }, hoverinfo: "y", showlegend: false }];
    for (const [k, [label, col]] of Object.entries(RT_OUTCOME)) if (lines[k]) data.push({ type: "scatter", mode: "lines", x: lines[k][0], y: lines[k][1], customdata: lines[k][2], name: label, line: { color: col, width: 1 }, opacity: S.rtHi ? 0.12 : 0.35, hoverinfo: "none", connectgaps: false });
    data.push({ type: "scatter", mode: "markers", x: pts.fail[0], y: pts.fail[1], text: pts.fail[2], customdata: pts.fail[3], name: "fail in that run", marker: { color: "@fail", size: 6 }, hovertemplate: "%{text}<br>%{y:.4g}<extra></extra>", _dim: dim });
    data.push({ type: "scatter", mode: "markers", x: pts.pass[0], y: pts.pass[1], text: pts.pass[2], customdata: pts.pass[3], name: "pass in that run", marker: { color: "@pass", size: 6, symbol: "circle-open", line: { width: 2 } }, hovertemplate: "%{text}<br>%{y:.4g}<extra></extra>", _dim: dim });
    if (pts.other[0].length) data.push({ type: "scatter", mode: "markers", x: pts.other[0], y: pts.other[1], text: pts.other[2], customdata: pts.other[3], name: `passes ${t.key}, fails another test or alarm`, marker: { color: "@warn", size: 7, symbol: "diamond" }, hovertemplate: "%{text}<br>%{y:.4g}<extra></extra>", _dim: dim });
    const hi = S.rtHi && d.parts.find((p) => p.key === S.rtHi);
    if (hi) {
      const xs = [], ys = [];
      hi.runs.forEach((r, k) => { const v = r.values ? r.values[ti] : null; if (v != null) { xs.push(k + 1 + jit(hi.key)); ys.push(v); } });
      data.push({ type: "scatter", mode: "lines+markers", x: xs, y: ys, customdata: xs.map(() => hi.key), name: hi.key, line: { color: "@text", width: 5 }, marker: { color: "@text", size: 11 }, hovertemplate: `${hi.key}<br>%{y:.4g}<br>click to clear<extra></extra>` });
    }
    const shapes = [];
    t.limits.forEach((l, k) => { if (!l) return; for (const y of [l.lo, l.hi]) if (y != null) shapes.push({ type: "line", x0: k + 0.55, x1: k + 1.45, y0: y, y1: y, line: { color: "@limit", width: 1.5, dash: "dash" } }); });
    plot(c, { data, layout: { title: { text: `${t.key}${t.units ? ` [${t.units}]` : ""} per run (dashed: limits)`, x: 0.01, xanchor: "left", font: { size: 15 } },
      xaxis: { tickvals: d.runs.map((_, k) => k + 1), ticktext: d.runs.map((r, k) => `${rl(k)}<br>${clip(r.name, N > 4 ? 16 : 28)}`), range: [0.4, N + 0.6], zeroline: false },
      yaxis: { title: { text: t.units || "" } }, shapes, margin: { l: 60, r: 20, t: 56, b: 60 },
      legend: { orientation: "h", x: 1, xanchor: "right", y: 1.0, yanchor: "bottom" } } });
    onPick(c);
  };
  // recovery by failing value
  const recBox = h("div");
  out.append(h("h2", {}, "Recovery by failing value"), h("p", { class: "lead" }, "Did a part's failing value decide whether a retest could recover it? Each part's value where it first failed the test (or, optionally, its value at every retest), colored by whether it passed again. The values are cut into three bands where the recovery rate changes (best fit of a separate rate per band); the middle band is hard bad when its recovery rate's 95% upper bound stays at or below the threshold. Marginal parts sit nearer the limits, extreme ones (e.g. clamped readings) farther out."), recBox);
  const drawRec = () => {
    const t = d.tests[ti], rc = d.recovery[ti], st = S.rt;
    const vf = select([["first", "first failing value"], ["each", "value at every retest"]], st.valueFrom || "first", (v) => (st.valueFrom = v));
    const mr = h("input", { type: "number", min: 1, max: 50, step: 1, value: st.maxRate ?? 10, style: { width: "70px" }, oninput: (e) => (st.maxRate = +e.target.value || 10) });
    const mz = h("input", { type: "number", min: 3, step: 1, value: st.minZone ?? 10, style: { width: "70px" }, oninput: (e) => (st.minZone = +e.target.value || 10) });
    const tol = select([["strict", "strict (observed)"], ["widened", "widened to the nearest recovered part"], ["tolerance", "strict + tolerance"]], st.rangeMode || "strict", (v) => { st.rangeMode = v; tolBox.hidden = v !== "tolerance"; preview(); });
    tol.title = "Which range 'Use as hard-bad range' sends to Bad candidates. Strict (default): only values seen never to recover, the cleanest candidates. Widened: halfway to the nearest part that did recover. Strict + tolerance: both edges moved out by a share of the band width or a fixed amount, never past the nearest recovered part.";
    const tolV = h("input", { type: "number", min: 0, step: "any", value: st.tolValue ?? 5, style: { width: "70px" }, oninput: (e) => { st.tolValue = +e.target.value || 0; preview(); } });
    const tolU = select([["pct", "% of band width"], ["abs", `${t.units || "units"} (absolute)`]], st.tolUnit || "pct", (v) => { st.tolUnit = v; preview(); });
    const tolBox = h("div", { class: "row tight", hidden: (st.rangeMode || "strict") !== "tolerance" }, tolV, tolU);
    const prev = h("span", { class: "muted small", style: { paddingBottom: "8px" } });
    const u = t.units ? ` ${t.units}` : "";
    const g4 = (v) => (v == null || !isFinite(v) ? "–" : String(Number(v.toPrecision(5))));
    const pts = rc.points;
    const ok = pts.filter((p) => p.recovered), bad = pts.filter((p) => !p.recovered);
    const zcol = { marginal: "warn", hard_bad: "fail", extreme: "muted" };
    const zname = { marginal: "marginal", hard_bad: "hard bad", extreme: "extreme" };
    const shapes = rc.zones.map((z) => ({ type: "rect", xref: "x", yref: "paper", x0: z.lo, x1: z.hi, y0: 0, y1: 1, fillcolor: tokA(zcol[z.kind], 0.16), line: { width: 0 }, layer: "below" }));
    // the hard-bad band widened halfway to the nearest part outside it
    for (const z of rc.zones.filter((x) => x.kind === "hard_bad")) for (const x of [z.safe_lo, z.safe_hi]) if (x != null) shapes.push({ type: "line", xref: "x", yref: "paper", x0: x, x1: x, y0: 0, y1: 1, line: { color: "@fail", dash: "dot", width: 1 } });
    const lim = t.limits[0] || {};
    for (const x of [lim.lo, lim.hi]) if (x != null) shapes.push({ type: "line", xref: "x", yref: "paper", x0: x, x1: x, y0: 0, y1: 1, line: { color: "@limit", dash: "dash", width: 1.5 } });
    const ann = rc.zones.map((z, i) => ({ x: (z.lo + z.hi) / 2, y: 1 - 0.07 * (i % 2), xref: "x", yref: "paper", yanchor: "bottom", showarrow: false, text: `${zname[z.kind]}: ${z.recovered}/${z.n} recovered`, font: { size: 11 }, bgcolor: tokA("surface", 0.85) }));
    const [c1, p1] = chartBox("tall"), [c2, p2] = chartBox("tall");
    const zt = h("div");
    recBox.replaceChildren(h("div", { class: "controls" }, field("Value", vf), field("Hard bad: recovery ≤ (%)", mr), field("Min parts per zone", mz), h("button", { onclick: () => S.rtRun && S.rtRun() }, "Recompute"), field("Hard-bad range to use", h("div", { class: "row tight" }, tol, tolBox)), prev),
      pts.length ? h("div", { class: "grid2" }, c1, c2) : h("p", { class: "muted" }, "No part failed this test and was retested."), zt);
    if (!pts.length) return;
    plot(p1, { data: [
      { type: "histogram", x: bad.map((p) => p.value), name: `never recovered (${bad.length})`, marker: { color: "@fail" }, nbinsx: 50 },
      { type: "histogram", x: ok.map((p) => p.value), name: `recovered (${ok.length})`, marker: { color: "@pass" }, nbinsx: 50 }],
      layout: { title: { text: `${t.key}: ${rc.value_from === "each" ? "value before each retest" : "first failing value"}`, x: 0.01, xanchor: "left", font: { size: 15 } }, barmode: "stack", xaxis: { title: { text: t.units || "" } }, yaxis: { title: { text: "parts" } }, shapes, annotations: ann, margin: { l: 55, r: 20, t: 70, b: 50 },
        legend: { orientation: "h", x: 1, xanchor: "right", y: 1.08, yanchor: "bottom" } } });
    const b = rc.bins;
    plot(p2, { data: [{ type: "scatter", mode: "markers+lines", x: b.map((x) => (x.lo + x.hi) / 2), y: b.map((x) => x.rate * 100), text: b.map((x) => `${g4(x.lo)} … ${g4(x.hi)}${u}<br>${x.recovered} of ${x.n} recovered`),
      error_y: { type: "data", symmetric: false, array: b.map((x) => (x.ci_hi - x.rate) * 100), arrayminus: b.map((x) => (x.rate - x.ci_lo) * 100), color: "@muted" }, line: { color: "@s1" }, marker: { size: 8, color: "@s1" }, hovertemplate: "%{text}<br>%{y:.0f}%<extra></extra>", name: "recovery rate" }],
      layout: { title: { text: "Recovery rate per value bin (95% Wilson)", x: 0.01, xanchor: "left", font: { size: 15 } }, xaxis: { title: { text: t.units || "" } }, yaxis: { title: { text: "% recovered" }, range: [-3, 103] }, shapes, margin: { l: 55, r: 20, t: 56, b: 50 }, showlegend: false } });
    const runOf = () => { const c = {}; for (const p of pts) c[p.run] = (c[p.run] || 0) + 1; return +Object.entries(c).sort((a, z) => z[1] - a[1])[0][0]; };
    function sendRange(z) { return hardBadRange(z, st.rangeMode, st.tolValue ?? 5, st.tolUnit || "pct"); }
    function rangeHow() { const m = st.rangeMode || "strict"; return m === "strict" ? "strict (observed)" : m === "widened" ? "widened to the nearest recovered part" : `strict + ${st.tolValue ?? 5}${(st.tolUnit || "pct") === "pct" ? "% of band width" : u} tolerance`; }
    function preview() { const hb = rc.zones.find((x) => x.kind === "hard_bad"); if (!hb) { prev.textContent = ""; return; } const z = sendRange(hb); prev.textContent = `sends ${g4(z.lo)} … ${g4(z.hi)}${u}`; }
    preview();
    const useRange = (z0) => {
      const z = sendRange(z0), how = rangeHow();
      const bId = d.runs[runOf()].id;
      const bc = S.bc || (S.bc = { a: S.ds, b: bId, good: "hbin = 1", test: "", how: "fail", range: "", incMarg: false, extra: "", name: "bad_candidate", coords: "auto" });
      Object.assign(bc, { b: bId, test: String(t.num), how: "fail", range: `${g4(z.lo)}..${g4(z.hi)}`, rangeNote: `from Retest trace, ${how}: ${rc.zones.find((x) => x.kind === "hard_bad").recovered} of ${rc.zones.find((x) => x.kind === "hard_bad").n} recovered in ${d.runs.map((r) => r.name).join(" → ")}` });
      if (bc.a === bId) bc.a = otherDs(bId);
      S.bcLast = null;
      toast(`Bad candidates: test ${t.num} in ${g4(z.lo)}..${g4(z.hi)}${u} (${how}) on ${d.runs[runOf()].name} (file 2). Pick file 1 (where they were good).`, "good");
      go("candidates");
    };
    table(zt, [
      { key: "kind", label: "Zone", render: (v) => badge(zname[v], v === "hard_bad" ? "bad" : v === "marginal" ? "warn" : "neutral") },
      { key: "lo", label: `Range${u}`, fmt: (v, r) => `${g4(r.lo)} … ${g4(r.hi)}` },
      { key: "safe_lo", label: "Up to the nearest recovered", fmt: (v, r) => (r.kind === "hard_bad" ? `${g4(r.safe_lo)} … ${g4(r.safe_hi)}` : ""), title: "the hard-bad band widened to halfway to the nearest part outside it" },
      { key: "n", label: "Parts", num: true }, { key: "recovered", label: "Recovered", num: true },
      { key: "rate", label: "Recovery", num: true, fmt: (v, r) => `${pctS(v)} [${pctS(r.ci_lo, 0)}–${pctS(r.ci_hi, 0)}]`, title: "recovered share, 95% Wilson interval" },
      { key: "use", label: "", render: (v, r) => (r.kind === "hard_bad" ? h("button", { onclick: (e) => { e.stopPropagation(); useRange(r); } }, "Use as hard-bad range") : "") },
    ], rc.zones, { search: false });
    if (!rc.zones.some((z) => z.kind === "hard_bad")) zt.append(h("p", { class: "muted small" }, `No middle value band with at least ${rc.min_zone} parts recovers less than its neighbours with a 95% upper bound under ${Math.round(rc.max_rate * 100)}%: lower the minimum or raise the threshold, or there is no unrecoverable band.`));
  };

  // run k vs run k+1, same parts
  const deltaBox = h("div");
  out.append(h("h2", {}, "Run to run"), h("p", { class: "lead" }, "The same parts' values in two consecutive runs: on the dotted y = x line the reading repeated; above or below it, it moved. Dashed lines are each run's limits."), deltaBox);
  let pk = 0;
  const drawDelta = () => {
    const t = d.tests[ti];
    const pairSel = select(d.runs.slice(0, -1).map((_, k) => [String(k), `${rl(k)} → ${rl(k + 1)}`]), String(pk), (v) => { pk = +v; drawDelta(); });
    const P = { pass: [[], [], [], []], fail: [[], [], [], []], other: [[], [], [], []] };
    let hiPt = null;
    for (const p of d.parts) {
      const a = p.runs[pk], b = p.runs[pk + 1];
      const x = a.values ? a.values[ti] : null, y = b.values ? b.values[ti] : null;
      if (x == null || y == null) continue;
      const c = rtClass(b, ti);
      const Q = P[c];
      Q[0].push(x); Q[1].push(y); Q[2].push(`${p.key}<br>site ${a.site} → ${b.site}${c === "other" ? `<br>passes ${t.key}; ${rtOther(b)}` : ""}`);
      Q[3].push(p.key);
      if (p.key === S.rtHi) hiPt = [x, y];
    }
    const dimD = S.rtHi ? 0.3 : 1;
    const all = [...P.pass[0], ...P.fail[0], ...P.other[0]], allY = [...P.pass[1], ...P.fail[1], ...P.other[1]];
    const n = all.length;
    const dl = allY.map((y, i) => y - all[i]).sort((a, b) => a - b);
    const mean = (v) => v.reduce((a, b) => a + b, 0) / v.length;
    let r = null;
    if (n > 2) { const mx = mean(all), my = mean(allY); let sxy = 0, sxx = 0, syy = 0; all.forEach((x, i) => { sxy += (x - mx) * (allY[i] - my); sxx += (x - mx) ** 2; syy += (allY[i] - my) ** 2; }); r = sxy / Math.sqrt(sxx * syy); }
    const lo = Math.min(...all, ...allY), hi = Math.max(...all, ...allY);
    const shapes = [{ type: "line", x0: lo, x1: hi, y0: lo, y1: hi, line: { color: "@muted", dash: "dot" } }];
    const la = t.limits[pk], lb = t.limits[pk + 1];
    for (const x of [la && la.lo, la && la.hi]) if (x != null) shapes.push({ type: "line", x0: x, x1: x, yref: "paper", y0: 0, y1: 1, line: { color: "@limit", dash: "dash", width: 1 } });
    for (const y of [lb && lb.lo, lb && lb.hi]) if (y != null) shapes.push({ type: "line", y0: y, y1: y, xref: "paper", x0: 0, x1: 1, line: { color: "@limit", dash: "dash", width: 1 } });
    const [card, c] = chartBox("tall");
    deltaBox.replaceChildren(h("div", { class: "controls" }, field("Runs", pairSel), h("span", { class: "muted small", style: { paddingBottom: "8px" } }, n ? `${n} parts in both · median change ${fmt(dl[Math.floor(n / 2)], 4)} ${t.units || ""} · r ${fmt(r, 3)}` : "no part has values in both runs")), card);
    plot(c, { data: [
      { type: "scatter", mode: "markers", x: P.fail[0], y: P.fail[1], text: P.fail[2], customdata: P.fail[3], name: `fail in ${rl(pk + 1)}`, marker: { color: "@fail", size: 7 }, _dim: dimD, hovertemplate: "%{text}<br>%{x:.4g} → %{y:.4g}<extra></extra>" },
      { type: "scatter", mode: "markers", x: P.pass[0], y: P.pass[1], text: P.pass[2], customdata: P.pass[3], name: `pass in ${rl(pk + 1)}`, marker: { color: "@pass", size: 7 }, _dim: dimD, hovertemplate: "%{text}<br>%{x:.4g} → %{y:.4g}<extra></extra>" },
      { type: "scatter", mode: "markers", x: P.other[0], y: P.other[1], text: P.other[2], customdata: P.other[3], name: `passes ${t.key}, fails another test or alarm`, marker: { color: "@warn", size: 8, symbol: "diamond" }, _dim: dimD, hovertemplate: "%{text}<br>%{x:.4g} → %{y:.4g}<extra></extra>" },
      ...(hiPt ? [{ type: "scatter", mode: "markers", x: [hiPt[0]], y: [hiPt[1]], customdata: [S.rtHi], name: S.rtHi, marker: { color: "rgba(0,0,0,0)", size: 18, line: { color: "@text", width: 3 } }, hovertemplate: `${S.rtHi}<br>click to clear<extra></extra>` }] : [])],
      layout: { title: { text: `${t.key}: ${rl(pk)} vs ${rl(pk + 1)}`, x: 0.01, xanchor: "left", font: { size: 15 } }, xaxis: { title: { text: `${rl(pk)} ${t.units || ""}` } }, yaxis: { title: { text: `${rl(pk + 1)} ${t.units || ""}` } }, shapes, margin: { l: 60, r: 20, t: 56, b: 50 },
        legend: { orientation: "h", x: 1, xanchor: "right", y: 1.0, yanchor: "bottom" } } });
    onPick(c);
  };
  drawTraj();
  drawRec();
  drawDelta();

  // one row per part
  flat = d.parts.map((p) => {
    const tv = traceValues(p.ids);
    const idc = [...p.runs.map((r, k) => (r.id_issue ? `${rtId(r.id_issue)[0]} R${k + 1}` : "")), ...(p.flags || []).map((f) => `${rtId(f.kind)[0]} R${f.run + 1}`)].filter(Boolean).join("; ");
    const o2 = { key: p.key, outcome: p.outcome, linked: p.runs.map((r, k) => (r.linked ? `R${k + 1}` : "")).filter(Boolean).join(", "), idcheck: idc, _trace: tv, _hay: Object.values(tv).filter(Boolean).join(" ").toLowerCase() };
    p.runs.forEach((r, k) => {
      o2[`s${k}`] = r.row != null && rtClass(r, 0) === "other" ? (r.status === "pass" ? "pass, alarm" : `fail, not ${d.tests[0].key}`) : RT_STATUS[r.status];
      o2[`site${k}`] = r.site ?? null;
      o2[`b${k}`] = r.hbin ?? null;
      o2[`f${k}`] = r.first_fail || "";
      d.tests.forEach((_, i) => (o2[`v${k}_${i}`] = r.values ? r.values[i] : null));
    });
    return o2;
  });
  const cols = [{ key: "key", label: "Part" }, { key: "outcome", label: "Outcome", render: (v) => badge(RT_OUTCOME[v][0], RT_OUTCOME[v][2]) }];
  if (d.link) cols.push({ key: "linked", label: "Linked in", title: "Runs where the part was found by the link, its key missing there" });
  if (d.audit && (d.audit.n_issues || d.audit.n_flagged)) cols.push({ key: "idcheck", label: "ID check", title: "ID problems on this part (see ID audit)", render: (v) => (v ? badge(v, "warn") : "") });
  d.runs.forEach((_, k) => {
    cols.push({ key: `s${k}`, label: `R${k + 1}`, title: `yellow: passes ${d.tests[0].key} but fails another test or has an alarm`, render: (v) => (v === "pass" ? badge("pass", "good") : v === "fail" ? badge("fail", "bad") : v && (v.startsWith("pass,") || v.startsWith("fail,")) ? badge(v, "warn") : h("span", { class: "muted small" }, v || "")) },
      { key: `site${k}`, label: `R${k + 1} site`, num: true }, { key: `b${k}`, label: `R${k + 1} bin`, num: true });
    d.tests.forEach((t, i) => cols.push({ key: `v${k}_${i}`, label: `R${k + 1} ${t.key}${t.units ? ` (${t.units})` : ""}`, num: true, fmt: (v) => (v == null ? "" : fmt(v, 4)) }));
    cols.push({ key: `f${k}`, label: `R${k + 1} 1st fail` });
  });
  const tb = h("div");
  onlyNote = h("div", { class: "row tight", hidden: true });
  out.append(h("h2", {}, "Parts"), h("p", { class: "muted" }, "Click a part to highlight it on the trajectory and run-to-run charts and trace it (all its results in run 1 and the last run it was found in); click it again to clear. Search matches the part key, part ID, part text, WXY (one test, or wafer|X|Y from the separate tests, e.g. 3|12|-4) and the other trace IDs."), onlyNote, tb);
  tblApi = table(tb, cols, flat, {
    placeholder: "Search part, part text, WXY…",
    match: (r, q) => rtSearch(r, q),
    onClick: (r) => { if (S.rtHi !== r.key) focusPart(r.key, "table"); },
    onClear: () => focusPart(null),
    tools: h("button", { onclick: () => SC.download("retest_trace.csv", SC.toCSV(cols.map((c) => (c.key === "outcome" ? { key: "outcome", label: "Outcome" } : c.render ? { key: c.key, label: c.label } : c)), flat)) }, "Download CSV"),
  });
  if (S.rtOnly) showOnly(S.rtOnly); else if (S.rtHi) tblApi.select((r) => r.key === S.rtHi);
  if (d.truncated) out.append(h("p", { class: "muted small" }, "Only the first 20,000 parts are listed."));
  renderRetestSites(out, d, pctS);
}
// bad candidates straight from the chain: good in run 1, failing the traced
// test on a retest of good parts and never passing after; then which run-1
// tests set them apart from the other good parts
function renderRetestCandidates(out, d) {
  const c = d.candidates;
  if (!c || !c.good_runs.length) return;
  const rl = (k) => `Run ${k + 1}`, t0 = d.tests[0], a = d.runs[0].id;
  const later = d.runs[c.good_runs[0]].id;
  const popF = c.n_marginal ? "pass and not $rt_passed_later" : "pass";
  const box = h("div"), mkBox = h("div");
  out.append(h("h2", {}, "Bad candidates from the retests of good parts"),
    h("p", { class: "lead" }, `${c.good_runs.map(rl).join(", ")} retested good parts. Parts good in run 1 that failed ${t0.key} there and never passed again are the bad candidates ($rt_bad_candidate in run 1); those that passed again later are left out of the comparison ($rt_passed_later); those passing every retest of good parts are confirmed good ($rt_confirmed_good). Find candidate tests ranks every run-1 test by how well it sets the candidates apart from the other good parts, as on the Bad candidates page.`), box, mkBox);
  box.append(h("div", { class: "kpis" },
    kpi("Bad candidates", c.n_cand.toLocaleString(), `good in run 1, failed ${t0.key} on a retest of good parts, never passed after`),
    kpi("Passed again later", c.n_marginal.toLocaleString(), `failed it there, passed a later retest ($rt_passed_later)`),
    kpi("Confirmed good", c.n_confirmed.toLocaleString(), "passed every retest of good parts"),
    c.n_other_fail ? kpi("Failed another test", c.n_other_fail.toLocaleString(), `on a retest of good parts, but not ${t0.key}`) : ""));
  if (!c.n_cand) { box.append(h("p", { class: "muted" }, `No good part of run 1 failed ${t0.key} on a retest of good parts without passing again.`)); return; }
  const find = async () => {
    loading(mkBox, "Ranking every run-1 test…");
    try {
      const mk = await api("markers", { mode: "group", a, select: "$rt_bad_candidate", vs: "", filter: popF, later, key_a: d.key, key_b: d.key, exclude: idExclude(), range: S.co.range, methods: mkMethods() });
      mkBox.replaceChildren();
      const fetchTop = (key) => api("chart", { kind: "compare_test", mode: "group", ds: a, select: "$rt_bad_candidate", vs: "", filter: popF, test: key, range: S.co.range }).then((x) => x.hist);
      renderMarkers(mkBox, mk, null, { fetchTop });
    } catch (e) { showErr(mkBox, e); }
  };
  box.append(h("div", { class: "row" },
    h("button", { class: "primary", onclick: find }, "Find candidate tests"),
    h("button", { onclick: () => useFilter(a, "$rt_bad_candidate") }, "Analyse the candidates ($rt_bad_candidate)"),
    h("button", { title: "Profile these same candidates on the Bad candidates page (outlier scores, sites, wafer zones, focus tests): $rt_bad_candidate as found over the whole chain, with $rt_passed_later left out", onclick: () => {
      S.bc = Object.assign(S.bc || {}, { a, b: later, good: "pass", test: String(t0.num), how: "fail", range: "", rangeNote: "", incMarg: false, extra: "", name: "bad_candidate", coords: "auto",
        keep: "rt_bad_candidate", leave: c.n_marginal ? "rt_passed_later" : "", keepNote: `from Retest trace: good in run 1, failed ${t0.key} on a retest of good parts (${c.good_runs.map(rl).join(", ")}), never passed after` });
      S.bcLast = null; go("candidates");
    } }, "Full profile in Bad candidates →")));
}
// pass/fail of the traced parts per test site, per run and over all runs
function renderRetestSites(out, d, pctS) {
  const bs = d.by_site;
  if (!bs || !bs.counts.length) return;
  const N = d.runs.length;
  const sites = [...new Set(bs.counts.map((c) => c.site))].sort((a, b) => a - b);
  const at = (k, s) => bs.counts.find((c) => c.run === k && c.site === s);
  const pS = (p) => (p == null ? "–" : p < 0.001 ? "< 0.001" : p.toFixed(3));
  const cell = (c) => (c ? `${c.pass}/${c.tested} (${pctS(c.rate, 0)})` : "–");
  const rows = sites.map((s) => {
    const r = { site: s };
    for (let k = 0; k < N; k++) r[`r${k}`] = at(k, s);
    r.all = at(null, s);
    r.rate = r.all ? r.all.rate : null;
    return r;
  });
  const cols = [{ key: "site", label: "Site", num: true }];
  for (let k = 0; k < N; k++) cols.push({ key: `r${k}`, label: `Run ${k + 1}`, num: true, fmt: (v) => cell(v), sortVal: (r) => (r[`r${k}`] ? r[`r${k}`].rate : -1), title: `pass / tested (pass rate) on this site in run ${k + 1}; χ² p = ${pS(bs.p[k])}` });
  cols.push({ key: "rate", label: "All runs", num: true, fmt: (v, r) => (r.all ? `${r.all.pass}/${r.all.tested} ${pctS(v)} [${pctS(r.all.ci_lo, 0)}–${pctS(r.all.ci_hi, 0)}]` : "–"), title: "every test of the traced parts on this site, over all runs (95% Wilson interval)" });
  const tb = h("div");
  const pRow = [`All runs p = ${pS(bs.p[N])}`, ...d.runs.map((_, k) => `run ${k + 1}: ${pS(bs.p[k])}`)].join(" · ");
  out.append(h("h2", {}, "Pass/fail by test site"),
    h("p", { class: "lead" }, "Could the test site cause it? Every traced part tested in a run counts under the site it was tested on there (rejects and retested good parts alike). χ² p-value of pass/fail against site: a small value (e.g. below 0.05) means the pass rate differs between sites more than chance explains."),
    h("p", { class: "muted small" }, pRow), tb);
  table(tb, cols, rows, { search: false });
  const [card, c] = chartBox("");
  out.append(card);
  plot(c, { data: sites.map((s, i) => {
    const pts = d.runs.map((_, k) => at(k, s));
    return { type: "bar", name: `site ${s}`, x: d.runs.map((_, k) => `Run ${k + 1}`), y: pts.map((p) => (p ? p.rate * 100 : null)), marker: { color: `@s${(i % 8) + 1}` },
      text: pts.map((p) => (p ? `${p.pass}/${p.tested}` : "")), textposition: "outside",
      error_y: { type: "data", symmetric: false, array: pts.map((p) => (p ? (p.ci_hi - p.rate) * 100 : 0)), arrayminus: pts.map((p) => (p ? (p.rate - p.ci_lo) * 100 : 0)), color: "@muted" },
      hovertemplate: `site ${s}, %{x}<br>%{text} pass (%{y:.0f}%)<extra></extra>` };
  }), layout: { title: { text: "Pass rate per site and run (95% Wilson)", x: 0.01, xanchor: "left", font: { size: 15 } }, barmode: "group", yaxis: { title: { text: "% pass" }, range: [0, 110] }, margin: { l: 55, r: 20, t: 56, b: 40 }, legend: { orientation: "h", x: 1, xanchor: "right", y: 1.0, yanchor: "bottom" } } });
}
// a run result: "pass", "fail" (fails the traced test ti) or "other" (passes
// it, but the part fails another test or carries an alarm)
function rtClass(r, ti) {
  const alarm = (r.alarms || []).length > 0;
  if (r.status === "pass") return alarm ? "other" : "pass";
  return r.fails && r.fails[ti] ? "fail" : "other";
}
function rtOther(r) {
  const a = (r.alarms || []);
  return [r.status === "fail" ? `fails ${r.first_fail || "another test"}` : "", a.length ? `alarm on ${a.slice(0, 3).join(", ")}${a.length > 3 ? "…" : ""}` : ""].filter(Boolean).join("; ");
}
function retestHelp() {
  const li = (n, t) => h("li", {}, h("b", {}, n), " — ", t);
  return h("details", { class: "adv help" }, h("summary", {}, "ⓘ How parts are followed and how to read the charts"),
    h("ul", { class: "small" },
      li("Matching", "each starting part of run 1 is looked up in every later run by the chosen identity (the last row with that identity, so a part retested twice in one file counts once, with its last result)."),
      li("Per run", "pass or fail (the part's bin), its bin, the traced test's value in display units (logged UNITS scaled by RES_SCAL), whether it failed that test, and its first failing test. Not retested = still bad but not found in that run; passed earlier = passed in an earlier run (good parts are usually not retested)."),
      li("Outcome", "recovered = failed somewhere, then passed in the last run it was found in; still bad = failed in the last run; not retested = failed, then never found again; stayed good = passed run 1 and never failed after (also when a later run retested it good)."),
      li("Flow", "where run 1's fails went at every retest: pass, fail again or not retested. Bands are proportional to part counts."),
      li("Trajectories", "one thin line per part across the runs (spread slightly sideways so they do not hide each other), colored by outcome; markers show pass (open green), fail on the traced test (red) or passing the traced test while failing another test or carrying an alarm (yellow diamond) in that run; a box per run shows the distribution of the parts tested there, and dashed lines each run's limits. A part that recovers because its reading came back inside the limits drops below the limit line; one that passes with an unchanged value points at another cause."),
      li("Recovery by failing value", "the value where each part first failed the traced test (or its value before every retest), recovered vs never. The sorted values are cut at the two points that best fit a separate recovery rate per band (binomial maximum likelihood), so the cuts fall where the recovery rate changes; the middle band is hard bad when it has at least the minimum number of parts, recovers less often than both sides, and its 95% Wilson upper bound is at or below the threshold. Parts nearer the limits are marginal, those farther out extreme (e.g. clamped readings). Use as hard-bad range sends the band to Bad candidates as the file-2 value range of the test."),
      li("Loadboard", "read from the loadboard test (default 5100) as a file-level value: the most common value in each file, so parts missing it still get their file's board; a file whose board changes is flagged."),
      li("Run to run", "the same parts' values in two consecutive runs. Points on y = x repeated; a cloud shifted off the line is a tester or condition offset; a wide cloud is poor repeatability.")));
}

// ---- Home
const FILE_HINT = "STDF v4 files: .stdf, .std or gzip-compressed .gz";
async function viewHome(el) {
  const has = S.datasets.length > 0;
  const merge = h("input", { type: "checkbox", checked: !!S.optMerge, onchange: (e) => { S.optMerge = e.target.checked; saveCfg(); } });
  const dedup = select([["", "keep every test (incl. retests)"], ["last", "keep the last test of each part"], ["first", "keep the first test of each part"]], S.optDedup || "", (v) => { S.optDedup = v; saveCfg(); });
  const browse = h("button", { class: "primary big", onclick: openFiles }, "Open STDF files…");
  const folder = S.info && S.info.native_dialogs ? h("button", { onclick: (e) => { e.stopPropagation(); openFolder(); } }, "Open folder…") : null;
  const sortSel = select([["time", "by test start time (MIR), else file time"], ["name", "by file name (run2 before run10)"], ["none", "as given"]], S.optSort, (v) => { S.optSort = v; saveCfg(); });
  const drop = h("div", { class: "dropzone", onclick: openFiles },
    h("img", { src: "/static/icon.png", alt: "", width: 64, height: 64 }),
    h("div", { class: "dz-title" }, has ? "Add more files" : "Drop STDF files or folders here"),
    h("div", { class: "muted" }, FILE_HINT),
    h("div", { class: "dz-actions" }, browse, folder));
  el.append(h("div", { class: "home-head" },
    h("div", {}, h("h1", {}, has ? "Your data" : "Welcome to STDF Cypher"),
      h("p", { class: "lead" }, has ? "Pick a dataset to analyse, or add more files." : "Fast, private STDF v4 analysis on your own PC: yield, bins, wafer maps, lot and tester comparison, correlation, ANOVA, outlier screening and machine learning.")),
  ));
  const grid = h("div", { class: "home-grid" });
  el.append(grid);
  const left = h("div", {});
  const right = h("div", {});
  grid.append(left, right);
  left.append(drop, h("div", { class: "home-opts" }, h("label", { class: "chk" }, merge, "Combine the files I open into one dataset (lot-to-lot, ANOVA by file)"), field("Retested parts", dedup), field("Order of opened files", sortSel)));

  if (has) {
    left.append(h("h2", {}, "Open datasets"));
    const cards = h("div", { class: "ds-cards" });
    for (const d of S.datasets) {
      cards.append(h("div", { class: "ds-card" + (d.id === S.ds ? " current" : "") },
        h("div", { class: "ds-main", onclick: () => { setDs(d.id); go("overview"); } },
          h("div", { class: "ds-name" }, d.name),
          h("div", { class: "ds-meta" }, `${d.n_parts.toLocaleString()} parts · ${d.n_tests} tests · yield ${pct(d.yield, 1)}` + (d.wafers.length ? ` · ${d.wafers.length} wafer(s)` : "") + (d.lots && d.lots.length ? ` · lot ${[...new Set(d.lots)].join(", ")}` : ""))),
        h("div", { class: "ds-actions" },
          h("button", { class: "primary", onclick: () => { setDs(d.id); go("overview"); } }, "Analyse"),
          h("button", { class: "ghost", title: "Rename this dataset", onclick: () => renameDs(d.id) }, "Rename"),
          h("button", { class: "ghost", title: "Close this dataset", onclick: async () => { await api("remove", { ds: d.id }); toast(`Closed ${d.name}`); await refreshDatasets(); go("home"); } }, "Close"))));
    }
    left.append(cards);
    if (S.datasets.filter((d) => !d.derived).length > 1) {
      const shared = h("div");
      left.append(shared);
      findSharedId("").then((r) => {
        if (!r) return;
        const n = r.found.groups.reduce((a, g) => a + g.length, 0);
        shared.replaceChildren(h("div", { class: "card pad", style: { marginTop: "10px" } }, h("div", { class: "row tight" },
          h("span", {}, `${n} open files share parts by ${r.label}: ${r.found.in_several.toLocaleString()} parts are in more than one file.`),
          h("button", { onclick: () => go("trace") }, "Trace them in a Part trace report"))));
      }).catch(() => {});
    }
  }

  // recent files
  right.append(h("h2", { style: { marginTop: 0 } }, "Recent files"));
  const rec = h("div", { class: "recent" }, h("div", { class: "muted" }, "Loading…"));
  right.append(rec);
  api("recent").then((list) => {
    if (!list.length) { rec.replaceChildren(h("div", { class: "muted small" }, "Files you open will be listed here.")); return; }
    rec.replaceChildren(...list.map((r) => h("div", { class: "recent-row" + (r.exists ? "" : " missing"), title: r.path },
      h("button", { class: "recent-open", disabled: !r.exists, onclick: () => loadPaths([r.path]) },
        h("span", { class: "recent-name" }, r.name),
        h("span", { class: "recent-path" }, r.exists ? `${dirname(r.path)} · ${fmtSize(r.size)}` : "file not found")),
      h("button", { class: "ghost x", title: "Remove from list", onclick: async () => { await api("forget_recent", { path: r.path }); go("home"); } }, "×"))));
    if (list.filter((r) => r.exists).length > 1) rec.append(h("button", { class: "ghost small", onclick: () => loadPaths(list.filter((r) => r.exists).slice(0, 5).map((r) => r.path)) }, `Open the ${Math.min(5, list.filter((r) => r.exists).length)} most recent`));
  }).catch(() => rec.replaceChildren());

  const pathBox = h("textarea", { rows: 3, class: "paths", placeholder: "C:\\data\\lot1.stdf\nC:\\data\\lot2.stdf.gz" });
  right.append(h("details", { class: "card pad adv" }, h("summary", {}, "Open by path"),
    pathBox, h("button", { onclick: () => loadPaths(pathBox.value.split(/\r?\n/).map((x) => x.trim().replace(/^"|"$/g, "")).filter(Boolean)) }, "Open")));

  // a few next steps for what is open (the side bar lists every page)
  const files = S.datasets.filter((d) => !d.derived);
  const tips = !has ? [
    ["", "Open STDF files", "Ctrl+O, drag them here, or drop them on the .exe; try synthetic_samples/ without data"],
    ["settings", "Settings", "Part identification, die X/Y tests and offsets, loadboard test"],
  ] : [
    ["overview", "Overview", "Yield, bins, sites and fail pareto of the current file"],
    ["explorer", "Test explorer", "Distribution, Cpk and trend of any test"],
    ...(files.length > 1 ? [["compare", "Compare files", "Lot vs lot or tester vs tester, every test ranked"], ["trace", "Trace parts across files", "Each part's history as an Excel report, or a retest chain"]] : [["records", "Records", "Every part and raw record of the file"]]),
  ];
  right.append(h("h2", {}, "Next steps"));
  right.append(h("div", { class: "tips" }, tips.map(([k, t, d]) => h("button", { class: "tip", onclick: () => (k ? go(k) : openFiles()) }, h("b", {}, t), h("span", {}, d)))));
}

function dirname(p) { const i = Math.max(p.lastIndexOf("/"), p.lastIndexOf("\\")); return i > 0 ? p.slice(0, i) : p; }
function fmtSize(b) { return b > 1e9 ? (b / 1e9).toFixed(1) + " GB" : b > 1e6 ? (b / 1e6).toFixed(1) + " MB" : Math.max(1, Math.round(b / 1e3)) + " KB"; }

// ---- opening files
async function openFiles() {
  if (S.info && S.info.native_dialogs) {
    try {
      const r = await api("pick_files");
      if (r.paths.length) await loadPaths(r.paths);
    } catch (e) { toast(e.message, "bad"); }
  } else {
    const inp = h("input", { type: "file", multiple: true, accept: ".stdf,.std,.gz,.stdf.gz", style: { display: "none" } });
    inp.addEventListener("change", () => { uploadFiles([...inp.files]); inp.remove(); });
    document.body.append(inp);
    inp.click();
  }
}
async function openFolder() {
  try {
    const r = await api("pick_folder", { title: "Open every STDF file in a folder" });
    if (r.path) await loadPaths([r.path]);
  } catch (e) { toast(e.message, "bad"); }
}
async function loadPaths(paths, opts) {
  opts = opts || {};
  if (!paths.length) return;
  busy(`Loading ${paths.length === 1 ? paths[0].split(/[\\/]/).pop() : paths.length + " files"}…`);
  try {
    const r = await api("load", { paths, merge: !!S.optMerge && paths.length > 1, dedup: S.optDedup || "", sort: S.optSort || "time" });
    await refreshDatasets(r.ids[r.ids.length - 1]);
    const again = [...new Set(r.already || [])];
    if (again.length) toast(`Already open: ${again.map(dsName).join(", ")}`, "", 6000);
    if (r.skipped) toast(`Skipped ${r.skipped} file${r.skipped > 1 ? "s" : ""} that ${r.skipped > 1 ? "are" : "is"} not STDF`, "", 6000);
    for (const e of r.errors || []) toast(`Could not read ${e.name}: ${e.error}`, "bad", 10000);
    const fresh = r.ids.filter((id) => !again.includes(id)).length;
    if (!opts.quiet && fresh) toast(`Opened ${fresh} dataset${fresh > 1 ? "s" : ""}`, "good");
    // on the home screen the user picks the page: stay there, with the new dataset listed
    go(S.view === "home" ? "home" : "overview");
  } catch (e) {
    toast(e.message, "bad", 10000);
  } finally { busy(null); }
}
function uploadOne(file, onProgress) {
  return new Promise((resolve, reject) => {
    const x = new XMLHttpRequest();
    x.open("POST", "/api/upload?name=" + encodeURIComponent(file.name));
    x.setRequestHeader("x-stdf-token", window.STDF_TOKEN);
    x.upload.onprogress = (e) => onProgress(e.loaded);
    x.onload = () => { try { const j = JSON.parse(x.responseText); j.ok ? resolve(j.data.path) : reject(new Error(j.error)); } catch (e) { reject(e); } };
    x.onerror = () => reject(new Error("upload failed"));
    x.send(file);
  });
}
const STDF_EXT = /\.(stdf|std|stdf_gz|stdfz|gz)$/i;
async function uploadFiles(files, skipped) {
  files = files.filter((f) => f.size > 0);
  skipped = skipped || 0;
  if (!files.length) { if (skipped) toast(`No STDF files found (${skipped} other file(s) skipped)`, "bad"); return; }
  // copies lose their file time, so put them in time order before uploading
  if ((S.optSort || "time") === "time") files.sort((a, b) => a.lastModified - b.lastModified);
  if (skipped) toast(`Skipped ${skipped} file${skipped > 1 ? "s" : ""} that ${skipped > 1 ? "are" : "is"} not STDF`, "", 6000);
  // a file dropped again while its copy is open: not copied or opened twice
  const open = [];
  for (const f of [...files]) {
    try {
      const r = await api("open_upload", { name: f.name, size: f.size });
      if (r.ds != null) { open.push(r.ds); files.splice(files.indexOf(f), 1); }
    } catch (e) {}
  }
  if (open.length) toast(`Already open: ${[...new Set(open)].map(dsName).join(", ")}`, "", 6000);
  if (!files.length) { setDs(open[open.length - 1]); return go(S.view === "home" ? "home" : "overview"); }
  const total = files.reduce((a, f) => a + f.size, 0);
  let done = 0;
  const paths = [];
  try {
    for (const f of files) {
      busy(`Reading ${f.name}…`, 0);
      paths.push(await uploadOne(f, (n) => busy(`Reading ${f.name}…`, (done + n) / total)));
      done += f.size;
    }
  } catch (e) { busy(null); return toast(e.message, "bad"); }
  busy(null);
  await loadPaths(paths);
}

// drag & drop anywhere
let dragDepth = 0;
window.addEventListener("dragenter", (e) => { if ([...(e.dataTransfer?.types || [])].includes("Files")) { dragDepth++; document.body.classList.add("dragging"); e.preventDefault(); } });
window.addEventListener("dragleave", () => { dragDepth = Math.max(0, dragDepth - 1); if (!dragDepth) document.body.classList.remove("dragging"); });
window.addEventListener("dragover", (e) => e.preventDefault());
window.addEventListener("drop", async (e) => {
  e.preventDefault();
  dragDepth = 0;
  document.body.classList.remove("dragging");
  // folders: read them (with subfolders) and keep the STDF files only
  const entries = [...(e.dataTransfer?.items || [])].map((i) => i.webkitGetAsEntry && i.webkitGetAsEntry()).filter(Boolean);
  if (!entries.some((x) => x.isDirectory)) {
    const files = [...(e.dataTransfer?.files || [])];
    if (files.length) uploadFiles(files);
    return;
  }
  busy("Reading folder…");
  const files = [];
  let skipped = 0;
  const fileOf = (en) => new Promise((res) => en.file(res, () => res(null)));
  const list = (dir) => new Promise((res) => { const r = dir.createReader(), all = []; const next = () => r.readEntries((b) => { if (!b.length) return res(all); all.push(...b); next(); }, () => res(all)); next(); });
  const walk = async (en, depth) => {
    if (en.isDirectory) { if (depth < 8) for (const c of await list(en)) await walk(c, depth + 1); return; }
    if (!STDF_EXT.test(en.name)) { skipped++; return; }
    const f = await fileOf(en);
    if (f) files.push(f);
  };
  try { for (const en of entries) await walk(en, 0); } finally { busy(null); }
  uploadFiles(files, skipped);
});

// ---- busy overlay & toasts
function busy(msg, frac) {
  const b = document.getElementById("busy");
  if (!msg) { b.hidden = true; return; }
  b.hidden = false;
  b.querySelector(".busy-msg").textContent = msg;
  const bar = b.querySelector(".busy-bar");
  bar.hidden = frac === undefined;
  if (frac !== undefined) bar.firstChild.style.width = Math.round(frac * 100) + "%";
}
function toast(msg, kind, ms) {
  const t = h("div", { class: "toast " + (kind || "") }, h("span", {}, msg), h("button", { class: "ghost x", onclick: () => t.remove() }, "×"));
  document.getElementById("toasts").append(t);
  setTimeout(() => t.remove(), ms || 4500);
}

// ---- Split & export
/** Where to save: with the app's own dialogs there is no path box, the save button asks (askPath) and
 *  saves at once; in a browser without them, a path box. Returns [input holding the path, element]. */
function pathField(kind, value, opts) {
  opts = opts || {};
  const inp = h("input", { type: "text", value: value || "", placeholder: opts.placeholder || "", style: { minWidth: "320px", flex: "1" } });
  inp._pick = { kind, opts };
  if (nativeDialogs()) return [inp, h("span", { hidden: true })];
  return [inp, h("div", { class: "pathfield" }, inp)];
}
const nativeDialogs = () => !!(S.info && S.info.native_dialogs);
/** The folder the last file was saved in (or the last folder chosen), kept for next time. */
function lastDir() { try { return localStorage.getItem("sc-last-dir") || ""; } catch (e) { return ""; } }
function setLastDir(d) { try { if (d) localStorage.setItem("sc-last-dir", d); } catch (e) {} }
const baseOf = (p) => String(p || "").split(/[\\/]/).pop();
/** Asks where to save for a path field: the save dialog (or folder picker) opens in the last folder used,
 *  else the field's default folder, with the default file name. Returns the path, or "" when cancelled.
 *  Without the app's dialogs, returns the typed path (a toast when it is empty). */
async function askPath(inp) {
  const { kind, opts } = inp._pick || { kind: "save", opts: {} };
  if (!nativeDialogs()) {
    if (!inp.value.trim()) toast(kind === "folder" ? "Choose an output folder" : "Choose where to save the file", "bad");
    return inp.value.trim();
  }
  const dir = lastDir() || (kind === "folder" ? dirname(inp.value) : dirname(inp.value));
  try {
    const r = kind === "folder"
      ? await api("pick_folder", { title: opts.title, dir })
      : await api("pick_save", { title: opts.title, name: baseOf(inp.value) || opts.name || "", ext: opts.ext || "", dir });
    if (!r.path) return "";
    setLastDir(kind === "folder" ? r.path : dirname(r.path));
    inp.value = r.path;
    inp.dispatchEvent(new Event("change"));
    return r.path;
  } catch (e) { toast(e.message, "bad"); return ""; }
}
/** A save button with an arrow: the button saves the file; the arrow also offers "Save and open".
 *  `run(open)` does the save and opens the saved file when `open` is true. */
function saveSplit(label, run) {
  const menu = h("div", { class: "save-menu", hidden: true },
    h("button", { onclick: () => { menu.hidden = true; run(false); } }, "Save only"),
    h("button", { onclick: () => { menu.hidden = true; run(true); } }, "Save and open in the app"));
  const arrow = h("button", { class: "primary caret", title: "More save options", "aria-haspopup": "menu", onclick: (e) => { e.stopPropagation(); menu.hidden = !menu.hidden; } }, "▾");
  return h("div", { class: "save-split" }, h("button", { class: "primary", onclick: () => run(false) }, label), arrow, menu);
}
document.addEventListener("click", () => document.querySelectorAll(".save-menu").forEach((m) => (m.hidden = true)));
function resultLine(msg, path) {
  return h("div", { class: "result-line" }, badge("done", "good"), h("span", {}, msg), path && S.info && S.info.app_mode ? h("button", { class: "ghost", onclick: () => api("reveal", { path }) }, "Show in folder") : null);
}
// files of one lot and test step (same lot, test code and flow): combine them
// in the app or write one STDF file, in the order and with the part IDs chosen
function mergeCard() {
  const box = h("div");
  const card = h("div", { class: "card pad section" }, h("h2", { style: { marginTop: 0 } }, "Merge"),
    h("p", { class: "muted" }, "Open files of the same lot and test step (MIR lot, test code and flow) are offered for merging, e.g. wafers tested on two testers. Merge them into one dataset here, or into one STDF file in the order you set. Files that share parts (a retest) can be merged too: the page says how many parts they share."), box);
  const files = () => S.datasets.filter((d) => d.files === 1 && !d.derived);
  const dsPaths = (ids) => ids.map((id) => S.datasets.find((d) => d.id === id).paths[0]);
  const combine = async (ids) => {
    busy("Combining…");
    try { const r = await api("load", { paths: dsPaths(ids), merge: true, sort: "none" }); await refreshDatasets(r.ids[0]); toast("Combined into one dataset", "good"); go("overview"); } catch (e) { toast(e.message, "bad"); } finally { busy(null); }
  };
  const fileLine = (f) => [f.n_parts != null ? `${f.n_parts.toLocaleString()} parts` : "", f.wafers && f.wafers.length ? `${f.wafers.length} wafer(s): ${f.wafers.slice(0, 6).join(", ")}${f.wafers.length > 6 ? "…" : ""}` : "", f.tester || ""].filter(Boolean).join(" · ");
  // `list`: [{id, name, …}] in merge order; ↑ ↓ change it
  const writer = (list, label) => {
    list = list.slice();
    const first = S.datasets.find((d) => d.id === list[0].id);
    const dir = dirname(first.paths[0]), sep = dir.includes("\\") ? "\\" : "/";
    const base = (label || first.name).replace(/\.(stdf|std)(\.gz)?$/i, "").replace(/[^\w.-]+/g, "_");
    const [path, pf] = pathField("save", dir + sep + base + "_merged.stdf", { title: "Save merged STDF", name: base + "_merged.stdf", ext: "stdf" });
    const renumber = h("input", { type: "checkbox" });
    const order = h("ol", { class: "small merge-order" });
    const drawOrder = () => order.replaceChildren(...list.map((f, i) => h("li", {}, h("b", {}, f.name), fileLine(f) ? ` · ${fileLine(f)}` : "", " ",
      h("button", { class: "ghost", title: "Move up", disabled: i === 0, onclick: () => { [list[i - 1], list[i]] = [list[i], list[i - 1]]; drawOrder(); } }, "↑"),
      h("button", { class: "ghost", title: "Move down", disabled: i === list.length - 1, onclick: () => { [list[i + 1], list[i]] = [list[i], list[i + 1]]; drawOrder(); } }, "↓"))));
    drawOrder();
    const res = h("div");
    return h("div", {}, h("div", { class: "muted small" }, "Merge order (the first file's header opens the merged file):"), order,
      h("label", { class: "chk", title: "Each later file's part IDs are raised by the highest part ID written before it (e.g. file 1 ends at 481: file 2's part 1 becomes 482). Part IDs that are not numbers are counted up instead." }, renumber, "Renumber part IDs: each file continues from the previous file's last part ID"),
      h("div", { class: "row" }, pf,
        saveSplit("Save merged STDF", async (open) => {
          if (!(await askPath(path))) return;
          busy("Writing merged STDF…");
          try {
            const r = await api("merge_files", { ds: list.map((f) => f.id), path: path.value, renumber: renumber.checked });
            res.replaceChildren(resultLine(`Saved ${r.path}: ${r.parts.toLocaleString()} parts (${r.good.toLocaleString()} good) from ${r.files} files`, r.path), h("button", { onclick: () => loadPaths([r.path]) }, "Open it"));
            if (open) loadPaths([r.path]);
          } catch (e) { toast(e.message, "bad"); } finally { busy(null); }
        }),
        h("button", { onclick: () => combine(list.map((f) => f.id)) }, "Combine in the app")), res);
  };
  const draw = (groups) => {
    const rows = groups.map((g) => {
      const label = [g.lot && `lot ${g.lot}`, g.step && `step ${g.step}`, g.flow && `flow ${g.flow}`].filter(Boolean).join(" · ") || "no lot or step in the files";
      const verdict = !g.by ? badge("no wafer + X/Y or part text to compare parts", "neutral")
        : g.shared ? badge(`${g.shared.toLocaleString()} shared parts (by ${g.by})`, "warn")
        : badge(`no shared parts (by ${g.by}): pieces of one test`, "good");
      const pairs = g.pairs && g.pairs.length ? h("ul", { class: "small" }, g.pairs.map((x) => h("li", {}, `${dsName(x.a)} and ${dsName(x.b)} share ${x.shared.toLocaleString()} parts (a retest?): merging keeps both results of those parts`))) : null;
      return h("div", { class: "card pad", style: { marginBottom: "8px" } }, h("div", { class: "row tight" }, h("b", {}, label), verdict), pairs, writer(g.files, `${g.lot || "lot"}${g.step ? "_" + g.step : ""}`));
    });
    // any other selection, for files the check does not group
    const pick = files().map((d) => [d.id, h("input", { type: "checkbox" })]);
    const sel = h("div");
    const manual = h("details", { class: "adv" }, h("summary", {}, "Merge other files"),
      h("div", { class: "row" }, ...pick.map(([id, c]) => h("label", { class: "chk" }, c, dsName(id)))),
      h("button", { onclick: () => { const ids = pick.filter(([, c]) => c.checked).map(([id]) => id); if (ids.length < 2) return toast("Tick at least two files", "bad"); sel.replaceChildren(writer(ids.map((id) => ({ id, name: dsName(id) })))); } }, "Merge the ticked files…"), sel);
    box.replaceChildren(...(rows.length ? rows : [h("p", { class: "muted small" }, files().length > 1 ? "No two open files share a lot and test step." : "Open the files of a lot to merge them.")]), files().length > 1 ? manual : "");
  };
  if (files().length > 1) api("merge_candidates").then(draw).catch((e) => showErr(box, e));
  else draw([]);
  return card;
}
// ---- Part trace: each part's history across files, written to an Excel workbook
// ---- Trace: the same physical parts across files, as a report and as charts
/** The test most parts fail, as a test key ("1013:VDD_LEAK"): of the latest file when the files are of one lot,
 *  else the highest fail rate of any of them (cached per set of datasets). Null when nothing fails. */
async function featuredTest(ids) {
  const k = [...(ids || [])].sort((a, b) => a - b).join(",") + "|" + idExclude();
  const c = (S.featured ||= {});
  if (!(k in c)) {
    try { c[k] = await api("featured_test", { ds: ids || [], exclude: idExclude() }); } catch (e) { c[k] = { key: null }; }
  }
  return c[k];
}
const TR_STATUS = { pass: ["pass", "@pass", "good"], fail: ["fail", "@fail", "bad"], absent: ["not tested", "@warn", "warn"] };
const TR_OUTCOME = { recovered: ["recovered", "@pass", "good"], still_bad: ["still bad", "@fail", "bad"], not_retested: ["not retested", "@warn", "warn"], good: ["never failed", "@s1", "neutral"] };
function viewTrace(el) {
  el.append(h("h1", {}, "Trace"), h("p", { class: "lead" }, "The same physical parts across files, in any order and from any lot: where the fails went, each part's results run by run, and an Excel report of it all. Parts are found by every ID ticked below at once, so where one is missing (no part text in an engineering file, a part that failed before its WXY test) another fills the gap. Untick a run to leave it out of the charts, the parts and the report."));
  const st = S.pt || (S.pt = { order: [], on: {}, names: {}, last: {}, seen: [], path: "", ids: {} });
  st.ids ||= {};
  if (st.show === undefined) st.show = "fail";
  const live = S.datasets.map((d) => d.id);
  st.order = [...st.order.filter((id) => live.includes(id)), ...live.filter((id) => !st.order.includes(id))];
  const choices = idChoices();
  // per ID: [files that log it, parts with it], from one look over every ID
  let avail = null, info = null, V = null;
  const idOn = (id) => st.ids[id] ?? (avail ? ID_STRONG.includes(id) && avail[id][1] > 0 : ID_STRONG.includes(id));
  const tickedIds = () => choices.filter(([id]) => idOn(id) && (!avail || avail[id][0] > 0));
  // only wafer, X, Y and part text link parts across files; the others are shown in their column
  const linksOf = () => tickedIds().map(([id]) => ID_STRONG.includes(id));
  const keyParams = () => ({ keys: tickedIds().map(([, k]) => k), links: linksOf() });
  const fileInfo = (id) => info && info.files.find((f) => f.id === id);
  const label = (id) => st.names[id] ?? dsName(id);
  const ticked = () => st.order.filter((id) => st.on[id]);
  const note = h("div", { class: "muted small" });
  const idBox = h("div", { class: "id-pick" });
  const dupes = h("div"), conflictBox = h("div"), auditBox = h("div");
  const runChips = h("div", { class: "row tight run-chips" });
  const viewBox = h("div");
  let auditSeq = 0;
  // the audit compares only the ticked IDs among part text, WXY and wafer, X, Y
  const idSpec = () => {
    const t = idTests(true), on = (id) => tickedIds().some(([x]) => x === id);
    return { wxy: "", w: on("w_x_y") ? t.w : "", x: t.x_t, y: t.y_t, ox: t.ox, oy: t.oy, die: wxyBasis() === "die", sub: waferCal().mode === "sub" };
  };
  const runAudit = async () => {
    const mine = ++auditSeq, ids = ticked();
    if (!ids.length || tickedIds().filter(([id]) => ID_STRONG.includes(id)).length < 2) return auditBox.replaceChildren();
    try {
      const a = await api("part_trace_audit", { ds: ids, ids: idSpec(), filter: scoped(S.filter) });
      if (mine !== auditSeq) return;
      if (!a.problems) {
        auditBox.replaceChildren(h("div", { class: "muted small", style: { margin: "8px 0" } }, a.compared ? `ID audit: the ticked IDs agree on all ${a.compared.toLocaleString()} tests that have two or more of them. Missing values are not counted as problems.` : "ID audit: no part has two IDs to compare."));
        return;
      }
      const hint = (a.wxy_differs ? " If WXY differs on most parts, check Check WXY against in Settings." : "") + (a.id_differs ? " If one W,X,Y has many part texts, X and Y probably need their offsets: set Check WXY against to die X/Y in Settings." : "");
      auditBox.replaceChildren(h("div", { class: "small", style: { margin: "8px 0" } },
        h("div", { class: "row tight" }, badge("ID audit", "bad"), `${a.problems.toLocaleString()} problems on ${a.parts.toLocaleString()} tests of parts: their ID cells are red in the report, and the ID audit sheet lists every one.${hint}`),
        h("details", {}, h("summary", {}, a.problems > a.first.length ? `The first ${a.first.length}` : "Show them"),
          h("ul", {}, a.first.map((f) => h("li", {}, f.text, " (", f.files.map(dsName).join(", "), ")"))))));
    } catch (e) { if (mine === auditSeq) auditBox.replaceChildren(h("div", { class: "muted small" }, `ID audit failed: ${e.message}`)); }
  };
  function drawIds() {
    const n = st.order.length;
    idBox.replaceChildren(h("b", {}, "Identify parts by"), h("span", { class: "muted small" }, " wafer, X, Y and part text link parts across files. The others are optional: ticked, they are a column of the parts and the report, but never link parts."),
      h("div", { class: "id-checks" }, choices.map(([id, , lab], j) => {
        const a = avail && avail[id];
        const none = a && !a[0];
        const k = info ? info.keys.indexOf(choices[j][1]) : -1;
        const linked = k >= 0 ? info.by_id[k] : null;
        const links = ID_STRONG.includes(id);
        const what = !a ? "looking…" : none ? "not in any open file" : [`in ${a[0]} of ${n} files`, `${a[1].toLocaleString()} parts`, links && linked != null ? `${linked.toLocaleString()} found in two or more files by it` : "", links ? "" : "column only, never links parts"].filter(Boolean).join(" · ");
        return h("label", { class: "chk id-check" + (none ? " dim" : ""), title: links ? "Links the same part across files" : "Shown in its own column only: never used to link parts across files (a single WXY test can be wrong; part IDs and X/Y restart in every file)" },
          h("input", { type: "checkbox", checked: idOn(id) && !none, disabled: !!none, onchange: (e) => { st.ids[id] = e.target.checked; detect(); } }),
          h("span", {}, lab), h("span", { class: "muted small" }, what));
      })),
      h("div", { class: "muted small" }, "Test numbers are set in ", h("a", { href: "#settings", onclick: (e) => { e.preventDefault(); go("settings"); } }, "Settings, Part identification"), ". When wafer, X, Y and part text disagree, wafer, X, Y decides."));
  }
  const list = h("div", { class: "tbl-wrap" });
  // a box picked on the flow names a run by its place: it goes when the runs change
  const move = (i, j) => { [st.order[i], st.order[j]] = [st.order[j], st.order[i]]; st.pick = null; draw(); refreshView(); };
  const groupOf = (id) => (info ? info.groups.findIndex((g) => g.includes(id)) : -1);
  const shareText = (f) => {
    if (!f) return "made in the app";
    if (!f.ids.has.some(Boolean)) return "has none of the ticked IDs: left out";
    const parts = [`${f.ids.shared.toLocaleString()} of ${f.ids.distinct.toLocaleString()} parts in other files`];
    if (f.ids.no_id) parts.push(`${f.ids.no_id.toLocaleString()} without an ID (left out)`);
    if (f.ids.placeholder) parts.push(`${f.ids.placeholder.toLocaleString()} placeholder IDs (not matched)`);
    return parts.join(" · ");
  };
  const idsOf = (f) => (!f ? "" : info.titles.map((t, j) => (f.ids.has[j] ? badge(t, f.ids.with[j] ? "neutral" : "warn") : null)).filter(Boolean));
  const setOn = (id, on) => { st.on[id] = on; st.pick = null; draw(); runAudit(); refreshView(); };
  function draw() {
    drawIds();
    const many = info && info.groups.length > 1;
    // the runs in their order, ticked or not: a quick way to compare runs back and forth
    runChips.replaceChildren(h("span", { class: "muted small" }, "Runs:"), ...st.order.map((id) => {
      const i = ticked().indexOf(id);
      return h("label", { class: "chk run-chip" + (st.on[id] ? " on" : ""), title: dsName(id) }, h("input", { type: "checkbox", checked: !!st.on[id], onchange: (e) => setOn(id, e.target.checked) }), (i >= 0 ? `R${i + 1} ` : "") + label(id));
    }));
    list.replaceChildren(h("table", {},
      h("thead", {}, h("tr", {}, ["", "Order", "Run", "Name in the report", "Same part again", "File", "Lot", "Tester", "Parts", "IDs", "Shared parts"].map((t) => h("th", {}, t)))),
      h("tbody", {}, st.order.map((id, i) => {
        const f = fileInfo(id), g = groupOf(id);
        const on = st.on[id];
        return h("tr", {},
          h("td", {}, h("input", { type: "checkbox", checked: !!on, title: "Include this run", "aria-label": `Include ${dsName(id)}`, onchange: (e) => setOn(id, e.target.checked) })),
          h("td", { style: { whiteSpace: "nowrap" } },
            h("button", { class: "ghost", title: "Move up", "aria-label": `Move ${dsName(id)} up`, disabled: i === 0, onclick: () => move(i, i - 1) }, "↑"),
            h("button", { class: "ghost", title: "Move down", "aria-label": `Move ${dsName(id)} down`, disabled: i === st.order.length - 1, onclick: () => move(i, i + 1) }, "↓")),
          h("td", {}, on ? `R${ticked().indexOf(id) + 1}` : ""),
          h("td", {}, h("input", { type: "text", value: label(id), style: { width: "180px" }, title: "The name over this file's columns", "aria-label": `Name of ${dsName(id)} in the report`, oninput: (e) => (st.names[id] = e.target.value), onchange: () => { draw(); refreshView(); } })),
          h("td", { style: { whiteSpace: "nowrap" } }, f && f.ids.repeats ? [
            badge(`${f.ids.repeats.toLocaleString()} extra`, "warn"), " ",
            h("label", { class: "chk", style: { display: "inline-flex" }, title: "Keep only the last test of each part in this file. Leave it off to show every test, one more row per extra test." }, h("input", { type: "checkbox", checked: !!st.last[id], onchange: (e) => { st.last[id] = e.target.checked; draw(); refreshView(); } }), "Last test only"),
          ] : ""),
          h("td", {}, dsName(id)),
          h("td", {}, f ? f.lot : ""),
          h("td", {}, f ? f.tester : ""),
          h("td", {}, f ? f.ids.parts.toLocaleString() : ""),
          h("td", { class: "small", style: { whiteSpace: "nowrap" } }, idsOf(f)),
          h("td", { class: "small" }, many && g >= 0 ? [badge(`group ${String.fromCharCode(65 + g)}`, "neutral"), " "] : "", f && !f.ids.shared ? badge("no shared parts", "warn") : "", f && !f.ids.shared ? " " : "", shareText(f)));
      }))));
    if (!info) { dupes.replaceChildren(); conflictBox.replaceChildren(); return; }
    const files = ticked().map(fileInfo).filter(Boolean);
    const open = files.filter((f) => f.ids.repeats && !st.last[f.id]);
    const rounded = files.filter((f) => f.ids.rounded);
    const warn = (label, text) => h("div", { class: "row tight small", style: { margin: "8px 0" } }, badge(label, "warn"), text);
    const ptr = tickedIds().some(([, k]) => /^(tests|w_x_y)/.test(k));
    dupes.replaceChildren(
      ...(rounded.length ? [warn("IDs may not be unique", `${rounded.map((f) => `${dsName(f.id)}: ${f.ids.rounded.toLocaleString()} parts`).join(" · ")} have a PTR ID above 16,777,216. STDF stores PTR results as 32-bit numbers, so these are rounded and different parts can share an ID. Keep part text or wafer, X, Y ticked as well.`)] : []),
      ...(open.length ? [warn("same part again", `${open.map((f) => `${dsName(f.id)}: ${f.ids.repeats.toLocaleString()} extra tests`).join(" · ")}. The charts and parts use each part's last test in a file; the report gives each extra test a row. If the repeats are intended, tick Last test only for that file.${ptr ? " With a PTR ID, first check these are repeats and not different parts with the same result, such as X and Y that need offsets: Last test only would merge them." : ""}`)] : []));
    // tests whose IDs disagree, warned before anything is written
    const apart = info.conflicts - info.joined - (info.shared || 0);
    const cList = (kind) => info.first_conflicts.filter((c) => c.kind === kind);
    const cBox = (n, kind, text) => (n ? [h("div", { class: "small", style: { margin: "8px 0" } },
      h("div", { class: "row tight" }, badge(kind === "joined" ? "joined by the trusted ID" : kind === "shared" ? "ID on several parts" : "kept apart", kind === "apart" ? "bad" : "warn"), text),
      cList(kind).length ? h("details", {}, h("summary", {}, n > cList(kind).length ? `The first ${cList(kind).length}` : "Show them"),
        h("ul", {}, cList(kind).map((c) => h("li", {}, c.text, " (", [...new Set(c.files)].map(dsName).join(", "), ")")))) : "")] : []);
    conflictBox.replaceChildren(
      ...cBox(info.joined, "joined", `${info.joined.toLocaleString()} tests share a more trusted ID with a part but differ on a less trusted one, e.g. the same W,X,Y with another part text: a part text recorded on the wrong part. They are traced as that part, with red ID cells and an ID check note giving the other value.`),
      ...cBox(info.shared || 0, "shared", `${(info.shared || 0).toLocaleString()} tests have an ID value that is on several parts of their own file (the same W,X,Y with other part texts, a copied memory, a part text on two dies). That value does not tell the parts apart, so it is not used to match them in that file: they are found by their other ID, with red ID cells and an ID check note. Many of them usually mean a setting: X and Y without their offsets, for example.`),
      ...cBox(apart, "apart", `${apart.toLocaleString()} tests share an ID with a part but are a different part: a more trusted ID differs (the same part text with another W,X,Y), or the shared ID is on several parts, so it is not unique (one W,X,Y on many part texts). Each keeps its own row, right after the other, with red ID cells and an ID check note.`),
      ...(info.conflicts ? [h("div", { class: "muted small" }, "When IDs disagree, W,X,Y (read from the part's own memory) decides over part text (read by a camera as the handler picks the part, so it can fall out of step). An all-zeros or all-ones W,X,Y is a corrupt read and matches nothing. When most parts disagree, a setting is usually wrong: X and Y without their offsets, for example (Settings, Check WXY against).")] : []));
    const pairs = info.pairs.map((x) => `${dsName(x.a)} and ${dsName(x.b)}: ${x.shared.toLocaleString()}`).join(" · ");
    note.replaceChildren(!tickedIds().length ? "Tick at least one ID." : info.groups.length ? `${info.in_several.toLocaleString()} parts are in more than one file. Shared parts by pair: ${pairs}.` : "No two open files share a part by the ticked IDs. Tick files to trace them anyway, or tick another ID.");
  }
  let latest = 0;
  const detect = async () => {
    const mine = ++latest;
    try {
      if (!avail) {
        note.textContent = "Looking for the IDs in the open files…";
        const all = await api("part_trace_detect", { keys: choices.map(([, k]) => k), links: choices.map(() => false), filter: scoped(S.filter) });
        if (mine !== latest) return;
        avail = Object.fromEntries(choices.map(([id], j) => [id, [all.files.filter((f) => f.ids.has[j]).length, all.files.reduce((a, f) => a + f.ids.with[j], 0)]]));
      }
      const keys = tickedIds().map(([, k]) => k);
      if (!linksOf().some(Boolean)) { info = null; V = null; viewSeq++; draw(); note.textContent = "Tick wafer, X, Y or part text: they link parts across files."; viewBox.replaceChildren(); return; }
      note.textContent = "Looking for shared parts…";
      const found = await api("part_trace_detect", Object.assign({ filter: scoped(S.filter) }, keyParams()));
      if (mine !== latest) return;
      info = Object.assign(found, { keys });
      // a file seen for the first time is ticked when it shares parts; after that the tick is the user's
      for (const f of info.files) {
        if (st.seen.includes(f.id)) continue;
        st.seen.push(f.id);
        st.on[f.id] = groupOf(f.id) >= 0;
      }
      runAudit();
      draw();
      refreshView();
    } catch (e) { info = null; note.textContent = ""; toast(e.message, "bad"); draw(); }
  };
  const startOf = (id) => { const f = fileInfo(id); return f && f.start_t ? f.start_t : Infinity; };
  const sorts = h("div", { class: "row tight" },
    h("button", { title: "Earliest test start (MIR) first", onclick: () => { st.order.sort((a, b) => startOf(a) - startOf(b)); st.pick = null; draw(); refreshView(); } }, "Sort by test start time"),
    h("button", { onclick: () => { st.order.sort((a, b) => dsName(a).localeCompare(dsName(b), undefined, { numeric: true })); st.pick = null; draw(); refreshView(); } }, "Sort by file name"),
    h("button", { title: "Name the ticked files STDF_1, STDF_2… in their current order", onclick: () => { ticked().forEach((id, i) => (st.names[id] = `STDF_${i + 1}`)); draw(); refreshView(); } }, "Name by order"),
    h("button", { title: "Back to each file's own name", onclick: () => { st.names = {}; draw(); refreshView(); } }, "Reset names"));
  // the report
  const first = S.datasets.find((d) => d.paths && d.paths.length);
  if (!st.path && first) { const dir = dirname(first.paths[0]); st.path = dir + (dir.includes("\\") ? "\\" : "/") + "part_trace.xlsx"; }
  const [path, pf] = pathField("save", st.path, { title: "Save part trace", name: "part_trace.xlsx", ext: "xlsx" });
  path.addEventListener("change", () => (st.path = path.value));
  const res = h("div");
  const runsParam = () => ticked().map((id) => ({ ds: id, name: label(id), last_only: !!st.last[id] }));
  const save = async () => {
    const ids = ticked();
    if (!ids.length) return toast("Tick at least one run", "bad");
    if (!linksOf().some(Boolean)) return toast("Tick wafer, X, Y or part text", "bad");
    if (!(await askPath(path))) return;
    st.path = path.value;
    busy("Writing the part trace…");
    try {
      const r = await api("part_trace_export", Object.assign({ runs: runsParam() }, keyParams(), { filter: scoped(S.filter), path: path.value, ids: idSpec() }));
      const s = r.summary;
      const left = s.runs.reduce((a, x) => a + x.left_out, 0);
      res.replaceChildren(resultLine(`Saved ${r.path}: ${s.parts.toLocaleString()} parts, ${s.in_several.toLocaleString()} in more than one file, ${s.rows.toLocaleString()} rows${left ? `, ${left.toLocaleString()} earlier tests left out` : ""}${s.conflicts ? `, ${s.conflicts.toLocaleString()} with IDs that disagree` : ""}${s.id_problems ? `, ${s.id_problems.toLocaleString()} ID audit problems` : ""}`, r.path));
    } catch (e) { toast(e.message, "bad"); } finally { busy(null); }
  };

  // ---- charts and parts
  const testIn = testInput("trace-test", st.test || "", "optional: a test to follow, e.g. 1013");
  testIn.title = "Its value in every run on the trajectory chart and in the parts. Blank: pass / fail only. Opens on the test most parts fail.";
  testIn.addEventListener("change", () => { st.test = testIn.value.trim(); st.testSet = true; refreshView(); });
  const show = select([["fail", "parts that failed in a run"], ["all", "all parts"]], st.show, (v) => { st.show = v; st.pick = null; renderView(); });
  let viewSeq = 0;
  async function refreshView() {
    const mine = ++viewSeq;
    if (!info || !ticked().length || !linksOf().some(Boolean)) { V = null; viewBox.replaceChildren(); return; }
    // a box picked on the flow belongs to these runs only
    if (st.pick && st.pick.sig !== ticked().join(",")) st.pick = null;
    // the test most parts fail, until one is chosen
    if (!st.testSet) {
      const f = await featuredTest(ticked());
      if (mine !== viewSeq) return;
      if (!f || !f.key) { st.test = ""; testIn.value = ""; testIn.title = "No test fails in these runs: pass / fail only."; }
      else { st.test = f.key; testIn.value = f.key; testIn.title = `Opened on ${f.key}: failed by ${f.fails.toLocaleString()} of ${f.parts.toLocaleString()} parts (${(f.rate * 100).toFixed(1)}%) in ${f.file}${f.one_lot ? ", the latest file of the lot" : ", the highest fail rate of these lots"}. Clear it for pass / fail only.`; }
    }
    loading(viewBox, "Tracing the parts…");
    try {
      const d = await api("part_trace_view", Object.assign({ runs: runsParam(), filter: scoped(S.filter), test: st.test || "" }, keyParams()));
      if (mine !== viewSeq) return;
      V = d;
      prepare(V);
      renderView();
    } catch (e) { if (mine === viewSeq) showErr(viewBox, e); }
  }
  // each part's status in each run, and how it ended
  function prepare(d) {
    const R = d.runs.length;
    for (const p of d.parts) {
      p.status = p.runs.map((c) => (!c ? "absent" : c.pass === true ? "pass" : "fail"));
      p.failed = p.status.includes("fail");
      const present = p.status.map((s, k) => (s === "absent" ? -1 : k)).filter((k) => k >= 0);
      const last = present[present.length - 1];
      p.outcome = !p.failed ? "good" : p.status[last] === "pass" ? "recovered" : last === R - 1 ? "still_bad" : "not_retested";
      p._hay = [...p.ids.filter(Boolean), ...p.runs.filter(Boolean).map((c) => c.part_id)].join(" ").toLowerCase();
    }
  }
  const rl = (k) => `R${k + 1}`;
  function renderView() {
    if (!V) return;
    const d = V, R = d.runs.length;
    const filt = () => d.parts.filter((p) => (st.show === "all" || p.failed) && (!st.pick || p.status[st.pick.run] === st.pick.status) && (!st.outcome || p.outcome === st.outcome));
    const shown = filt();
    const count = (o) => d.parts.filter((p) => p.outcome === o).length;
    const chip = (o) => { const [lab, , kind] = TR_OUTCOME[o]; const n = count(o); return n ? h("button", { class: "ghost small" + (st.outcome === o ? " active" : ""), title: "Show only these parts (click again for all)", onclick: () => { st.outcome = st.outcome === o ? null : o; renderView(); } }, badge(`${lab} ${n.toLocaleString()}`, kind)) : null; };
    const filters = [st.pick ? `${rl(st.pick.run)} ${TR_STATUS[st.pick.status][0]}` : "", st.outcome ? TR_OUTCOME[st.outcome][0] : ""].filter(Boolean);
    viewBox.replaceChildren(
      h("div", { class: "kpis" },
        kpi("Parts", d.parts.length.toLocaleString(), `${d.parts.filter((p) => p.failed).length.toLocaleString()} failed in a run`),
        ...["recovered", "still_bad", "not_retested"].map((o) => kpi(TR_OUTCOME[o][0], count(o).toLocaleString(), o === "recovered" ? "failed, then passed its last run" : o === "still_bad" ? `failed its last run, ${d.runs[R - 1] ? rl(R - 1) : ""}` : "failed, then not in a later run")),
        d.test ? kpi("Following", d.test.key, d.test.units || "") : kpi("Following", "pass / fail", "no test chosen")),
      h("div", { class: "row tight" }, h("span", { class: "muted small" }, "Show only:"), ...["recovered", "still_bad", "not_retested", "good"].map(chip),
        filters.length ? h("button", { class: "ghost small", onclick: () => { st.pick = null; st.outcome = null; renderView(); } }, `× ${filters.join(", ")}`) : null));
    // run by run
    const runRows = d.runs.map((r, k) => {
      const inRun = d.parts.filter((p) => p.status[k] !== "absent");
      const earlier = (p) => p.status.slice(0, k).includes("fail");
      return { run: `${rl(k)}: ${r.name}`, parts: inRun.length, pass: inRun.filter((p) => p.status[k] === "pass").length, fail: inRun.filter((p) => p.status[k] === "fail").length,
        retested: inRun.filter(earlier).length, recovered: inRun.filter((p) => earlier(p) && p.status[k] === "pass").length, new_: inRun.filter((p) => !p.status.slice(0, k).some((s) => s !== "absent")).length };
    });
    const rb = h("div");
    viewBox.append(h("h2", {}, "Run by run"), rb);
    table(rb, [{ key: "run", label: "Run" }, { key: "parts", label: "Parts", num: true }, { key: "pass", label: "Pass", num: true }, { key: "fail", label: "Fail", num: true },
      { key: "retested", label: "Failed before", num: true, title: "parts here that failed in an earlier run" }, { key: "recovered", label: "Recovered here", num: true, title: "failed in an earlier run, pass here" },
      { key: "new_", label: "First seen here", num: true }], runRows, { search: false });
    // where the fails went: click a box to list its parts
    const g = h("div", { class: "grid1" });
    viewBox.append(h("h2", {}, "Where the fails went"), h("p", { class: "muted small" }, "Parts that failed in at least one run, from run to run. Click a box to list its parts below; click it again to show all."), g);
    if (R > 1) {
      const flow = d.parts.filter((p) => p.failed);
      const nodes = [], idx = {}, links = {};
      const node = (k, s) => { const n = `${rl(k)} ${TR_STATUS[s][0]}`; if (!(n in idx)) { idx[n] = nodes.length; nodes.push([n, s, k]); } return idx[n]; };
      for (const p of flow) for (let k = 0; k + 1 < R; k++) { const key = `${node(k, p.status[k])}>${node(k + 1, p.status[k + 1])}>${p.status[k + 1]}`; links[key] = (links[key] || 0) + 1; }
      const L = Object.entries(links).map(([k, n]) => { const [s, t, to] = k.split(">"); return { s: +s, t: +t, n, to }; });
      const [card, c] = chartBox("tall");
      g.append(card);
      plot(c, { data: [{ type: "sankey", arrangement: "snap", node: { label: nodes.map(([n]) => n), color: nodes.map(([, s]) => TR_STATUS[s][1]), customdata: nodes.map(([, , k]) => d.runs[k].name), hovertemplate: "%{label}: %{value} parts<br>%{customdata}<extra></extra>", pad: 18, thickness: 16, line: { width: 0 } },
        link: { source: L.map((l) => l.s), target: L.map((l) => l.t), value: L.map((l) => l.n), color: L.map((l) => tokA(l.to === "pass" ? "pass" : l.to === "fail" ? "fail" : "warn", 0.35)), hovertemplate: "%{source.label} → %{target.label}: %{value}<extra></extra>" } }],
        layout: { margin: { l: 10, r: 10, t: 20, b: 20 } } });
      if (c.on) c.on("plotly_click", (ev) => {
        const pt = ev.points && ev.points[0];
        if (!pt || pt.pointNumber == null || pt.source) return;
        const nd = nodes[pt.pointNumber];
        if (!nd) return;
        const same = st.pick && st.pick.run === nd[2] && st.pick.status === nd[1];
        st.pick = same ? null : { run: nd[2], status: nd[1], sig: ticked().join(",") };
        if (st.show !== "all" && !same) st.show = "fail";
        renderView();
        partsBox.scrollIntoView({ behavior: "smooth", block: "start" });
      });
    } else g.append(h("p", { class: "muted small" }, "Tick two runs or more to see where the fails went."));
    // trajectory of the followed test
    if (d.test && R > 1) {
      const lines = shown.filter((p) => p.runs.some((c) => c && c.value != null)).slice(0, 600);
      const xs = d.runs.map((_, k) => rl(k));
      const data = Object.entries(TR_OUTCOME).map(([o, [lab, col]]) => {
        const sel = lines.filter((p) => p.outcome === o);
        const x = [], y = [], cd = [];
        for (const p of sel) { p.runs.forEach((c, k) => { x.push(xs[k]); y.push(c && c.value != null ? c.value : null); cd.push(d.parts.indexOf(p)); }); x.push(null); y.push(null); cd.push(null); }
        return { type: "scattergl", mode: "lines+markers", name: `${lab} (${sel.length})`, x, y, customdata: cd, connectgaps: false, line: { color: col, width: 1 }, marker: { color: col, size: 5 }, opacity: 0.6, hovertemplate: "%{x}: %{y}<extra>" + lab + "</extra>" };
      }).filter((t) => t.x.length);
      const [card, c] = chartBox("tall");
      g.append(card);
      plot(c, { data, layout: { title: { text: `${d.test.key} run by run${lines.length < shown.length ? ` (first ${lines.length} parts)` : ""}`, x: 0.01, xanchor: "left", font: { size: 15 } }, xaxis: { type: "category" }, yaxis: { title: { text: d.test.units || "" } }, margin: { l: 60, r: 20, t: 56, b: 40 } } });
      if (c.on) c.on("plotly_click", (ev) => { const i = ev.points && ev.points[0] && ev.points[0].customdata; if (i != null && d.parts[i]) showPart(d.parts[i]); });
    }
    // the parts, as in the report
    const partsBox = h("div"), detail = h("div");
    viewBox.append(h("div", { class: "row" }, h("h2", {}, "Parts"), h("span", { class: "muted small" }, "Show"), show,
      h("span", { class: "muted small" }, "Search finds part text, WXY, W,X,Y (W24X111Y146 or 24, 111, 146) or part ID in any run. Click a part for its results.")), partsBox, detail);
    const cell = (k) => (c, p) => {
      if (!c) return h("span", { class: "muted small" }, "–");
      const st_ = p.status[k];
      return h("span", { class: "tr-cell" }, badge(st_ === "pass" ? `pass · bin ${c.hbin}` : `bin ${c.hbin}`, st_ === "pass" ? "good" : "bad"),
        c.first ? h("span", { class: "small", title: "First and last failing test" }, c.first === c.last ? ` ${c.first}` : ` ${c.first} → ${c.last}`) : "",
        c.value != null ? h("span", { class: "small muted" }, ` · ${fmt(c.value)}`) : "",
        c.n > 1 ? h("span", { class: "small muted", title: "Tested more than once in this run: the last test is shown" }, ` ×${c.n}`) : "");
    };
    const rows = shown.map((p) => Object.assign({ _p: p, outcome: TR_OUTCOME[p.outcome][0], note: p.note || "" }, Object.fromEntries(d.titles.map((t, j) => [`id${j}`, p.ids[j] || ""])), Object.fromEntries(p.runs.map((c, k) => [`r${k}`, c]))));
    const cols = [...d.titles.map((t, j) => ({ key: `id${j}`, label: t, render: (v, r) => h("span", { class: r._p.note ? "flag-fail" : "" }, v) })),
      { key: "outcome", label: "Outcome", render: (v, r) => badge(v, TR_OUTCOME[r._p.outcome][2]) },
      ...d.runs.map((r, k) => ({ key: `r${k}`, label: `${rl(k)} ${r.name}`, render: (v, row) => cell(k)(v, row._p) })),
      ...(d.parts.some((p) => p.note) ? [{ key: "note", label: "ID check" }] : [])];
    const wxyQ = (q) => { const m = /^\s*(-?\d+)[\s,;/|]+(-?\d+)[\s,;/|]+(-?\d+)\s*$/.exec(q); return m ? wxyText(m[1], m[2], m[3]).toLowerCase() : null; };
    const csvCols = [...d.titles.map((t, j) => ({ key: `id${j}`, label: t })), { key: "outcome", label: "Outcome" },
      ...d.runs.flatMap((r, k) => [{ key: `r${k}_hbin`, label: `${r.name} Hard Bin` }, { key: `r${k}_first`, label: `${r.name} First Fail Test` }, { key: `r${k}_last`, label: `${r.name} Last Fail Test` }, ...(d.test ? [{ key: `r${k}_value`, label: `${r.name} ${d.test.key}` }] : [])])];
    const csvRows = () => rows.map((r) => Object.assign({}, r, ...d.runs.map((_, k) => { const c = r[`r${k}`]; return c ? { [`r${k}_hbin`]: c.hbin, [`r${k}_first`]: c.first || "", [`r${k}_last`]: c.last || "", [`r${k}_value`]: c.value ?? "" } : {}; })));
    table(partsBox, cols, rows, {
      placeholder: "Find a part…",
      match: (r, q) => r._p._hay.includes(q) || (wxyQ(q) && r._p._hay.includes(wxyQ(q))),
      onClick: (r) => showPart(r._p), onClear: () => detail.replaceChildren(),
      tools: h("button", { onclick: () => SC.download("part_trace.csv", SC.toCSV(csvCols, csvRows())) }, "Download CSV"),
    });
    function showPart(p) {
      const box = h("div");
      detail.replaceChildren(h("div", { class: "card pad" },
        h("div", { class: "row tight", style: { justifyContent: "space-between" } }, h("b", {}, p.ids.filter(Boolean).join(" · ") || "part"), closeBtn(detail)),
        p.note ? h("p", { class: "small flag-fail" }, p.note) : null,
        h("div", { class: "row" }, ...p.runs.map((c, k) => (c ? h("button", { onclick: () => partDetail(box, d.runs[k].id, c.row, `${rl(k)} ${d.runs[k].name}: part ${c.part_id}`) }, `Results in ${rl(k)}`) : null)),
          ...p.runs.map((c, k) => (c ? h("button", { class: "ghost small", title: `Add part ${c.part_id} of ${d.runs[k].name} to the part datalog`, onclick: () => addToDatalog(d.runs[k].id, { index: c.row, part_id: c.part_id, site: c.site, hbin: c.hbin }) }, `+ datalog ${rl(k)}`) : null))),
        box));
      const k0 = p.runs.findIndex(Boolean);
      if (k0 >= 0) partDetail(box, d.runs[k0].id, p.runs[k0].row, `${rl(k0)} ${d.runs[k0].name}: part ${p.runs[k0].part_id}`);
      detail.scrollIntoView({ behavior: "smooth", block: "start" });
    }
  }
  const scope = scoped(S.filter);
  el.append(h("div", { class: "card pad section" },
    idBox,
    scope ? h("div", { class: "row tight small" }, badge("part filter on", "warn"), `Only parts matching ${scope} are traced.`) : "",
    sorts, list, conflictBox, dupes, auditBox, note,
    h("div", { class: "row", style: { marginTop: "10px" } }, pf, h("button", { class: "primary", onclick: save }, "Export to Excel")),
    res),
    h("div", { class: "card pad section trace-controls" }, runChips,
      h("div", { class: "controls" }, field("Follow test (optional)", testIn), h("button", { class: "ghost", title: "Back to the test most parts fail", onclick: () => { st.testSet = false; refreshView(); } }, "Most failed test"))),
    viewBox);
  draw();
  detect();
}
// ---- Drift report (was Qualification drift): endurance check of one file, 25x loop compare of two, per test and site
function viewQual(el) {
  el.append(h("h1", {}, "Drift report"), h("p", { class: "lead" }, "Mean drift of every test on every site, written to Excel. Each site holds its own looped unit, so each site is its own trial. Drift = ABS(mean1 - mean2) / (USL - LSL)."));
  const st = S.qual || (S.qual = { e: {}, c: {} });
  const files = () => S.datasets.filter((d) => d.paths && d.paths.length);
  const num = (v, step, w) => h("input", { type: "number", value: v, step: step || 1, min: 0, style: { width: (w || 70) + "px" } });
  const defPath = (name) => { const f = files()[0]; if (!f) return name; const dir = dirname(f.paths[0]); return dir + (dir.includes("\\") ? "\\" : "/") + name; };
  const filt = () => S.filter || "";
  const pctTxt = (v) => (v == null ? "" : v.toFixed(2) + "%");
  const topTable = (top, drift) => top.length ? h("div", { class: "tbl-wrap" }, h("table", {},
    h("thead", {}, h("tr", {}, ["Test", "Unit", "Site", "Drift %", "Comment"].map((x) => h("th", {}, x)))),
    h("tbody", {}, top.map((r) => h("tr", {}, h("td", {}, r.test), h("td", {}, r.units), h("td", {}, r.site),
      h("td", {}, r.drift > drift ? badge(r.drift.toFixed(2), "bad") : r.drift.toFixed(2)), h("td", { class: "small" }, r.comment)))))) : "";
  const scopeNote = () => (S.scope === "good" ? h("div", { class: "row tight small" }, badge("population ignored", "warn"), "The yield check needs every loop, so these reports use all parts. The part filter still applies.") : "");
  const fileOpts = () => files().map((d) => [String(d.id), d.name]);
  const statusBadge = (s) => badge(s.status, s.ok ? "good" : "bad");

  // -- endurance check
  const eDs = select(fileOpts(), String(st.e.ds ?? (files()[0] || {}).id), (v) => (st.e.ds = +v));
  const eRuns = num(st.e.runs ?? 25), eDrift = num(st.e.drift ?? 5, 0.5), eYield = num(st.e.yield ?? 95, 0.5), eCpk = num(st.e.cpk ?? 1.33, 0.01);
  const ePick = select([["clean", "Clean: first and last good loops"], ["raw", "Raw: loops as they are (all must pass)"]], st.e.pick || "clean", (v) => (st.e.pick = v));
  const eSkip = num(st.e.skip ?? 0), eLoops = num(st.e.max_loops ?? 1000, 1, 80), eMin = num(st.e.max_minutes ?? 90, 1, 80);
  const eMaxTrim = num(st.e.max_trim ?? 20, 1);
  const eTrim = select([["off", "Off"], ["ends", "Ends: failing loops at the start and end"], ["any", "Ends, then bad loops between"]], st.e.trim || "off", (v) => (st.e.trim = v));
  const ePathT = (() => { const f = files().find((d) => d.id === +eDs.value) || files()[0]; if (!f) return "trimmed.stdf"; return f.paths[0].replace(/(\.stdf|\.std)?(\.gz)?$/i, "") + "_trimmed.stdf"; })();
  const [eTPath, eTPf] = pathField("save", st.e.tpath || ePathT, { title: "Save the trimmed STDF", name: "trimmed.stdf", ext: "stdf" });
  const [ePath, ePf] = pathField("save", st.e.path || defPath("endurance_drift.xlsx"), { title: "Save endurance report", name: "endurance_drift.xlsx", ext: "xlsx" });
  const eOut = h("div"), eRes = h("div");
  const eParams = () => {
    Object.assign(st.e, { ds: +eDs.value, runs: +eRuns.value, drift: +eDrift.value, yield: +eYield.value, cpk: +eCpk.value, pick: ePick.value, skip: +eSkip.value, max_loops: +eLoops.value, max_minutes: +eMin.value, trim: eTrim.value, max_trim: +eMaxTrim.value, path: ePath.value, tpath: eTPath.value });
    return { ds: st.e.ds, runs: st.e.runs, drift: st.e.drift, yield: st.e.yield, cpk: st.e.cpk, pick: st.e.pick, skip: st.e.skip, max_loops: st.e.max_loops, max_minutes: st.e.max_minutes, trim: st.e.trim, max_trim: st.e.max_trim, filter: filt() };
  };
  const eCheck = async () => {
    if (!files().length) return;
    loading(eOut);
    try {
      const p = eParams(), d = await api("endurance_check", p);
      const ok = d.sites.filter((s) => s.ok).length;
      eOut.replaceChildren(
        h("div", { class: "kpis" }, kpi("Sites compared", `${ok} of ${d.sites.length}`, ok === d.sites.length ? "all sites pass" : "see the dropped sites below"),
          kpi("Test-site rows", d.rows.toLocaleString(), `${d.tests.toLocaleString()} tests${d.short_rows ? `, ${d.short_rows} with fewer than ${p.runs} values` : ""}`),
          kpi(`Drift above ${p.drift}%`, d.over.toLocaleString(), "rows to comment"), kpi(`Cpk below ${p.cpk}`, d.low_cpk.toLocaleString(), "rows"),
          p.trim !== "off" ? kpi("Loops trimmed", (d.trim.start + d.trim.end + d.trim.between).toLocaleString(), `${d.trim.start} start, ${d.trim.end} end, ${d.trim.between} between`) : ""),
        p.trim !== "off" ? h("div", { class: "muted small", style: { margin: "6px 0" } }, d.trim.loops ? `Loops left out for every site: ${d.trim.loops}. Trimmed for sites ${d.trim.sites.join(", ") || "none"}; the other sites could not reach the yield or the good loops needed and are dropped.` : "The trim left out no loop.") : "",
        h("div", { class: "tbl-wrap" }, h("table", {},
          h("thead", {}, h("tr", {}, ["Site", "Loops", "In window", "Cut by", "Good", "Window yield", "Trimmed", "Kept yield", "Span yield", `First ${p.runs}`, `Last ${p.runs}`, "Skipped start / end", "Status"].map((x) => h("th", {}, x)))),
          h("tbody", {}, d.sites.map((s) => h("tr", {}, h("td", {}, s.label), h("td", {}, s.loops), h("td", {}, s.window), h("td", {}, s.cut_by), h("td", {}, s.good), h("td", {}, pctTxt(s.window_yield)), h("td", {}, s.trimmed || ""), h("td", {}, pctTxt(s.kept_yield)), h("td", {}, pctTxt(s.span_yield)),
            h("td", {}, s.first_loops), h("td", {}, s.last_loops), h("td", {}, s.first_loops ? `${s.skipped_lead} / ${s.skipped_tail}` : ""), h("td", {}, statusBadge(s))))))),
        d.top.length ? h("h3", {}, "Largest drifts") : "", topTable(d.top, p.drift));
    } catch (e) { showErr(eOut, e); }
  };
  const eSave = async () => {
    if (!(await askPath(ePath))) return;
    busy("Writing the endurance report…");
    try {
      const r = await api("endurance_export", Object.assign(eParams(), { path: ePath.value }));
      eRes.replaceChildren(resultLine(`Saved ${r.path}: ${r.rows.toLocaleString()} test-site rows on ${r.sites} sites${r.dropped ? ` (${r.dropped} dropped)` : ""}, ${r.over.toLocaleString()} above the drift threshold`, r.path));
    } catch (e) { toast(e.message, "bad"); } finally { busy(null); }
  };
  const eTRes = h("div");
  const eTrimSave = async () => {
    if (!(await askPath(eTPath))) return;
    busy("Writing the trimmed STDF…");
    try {
      const r = await api("endurance_trim_save", Object.assign(eParams(), { path: eTPath.value }));
      eTRes.replaceChildren(resultLine(`Saved ${r.path}: ${r.kept.toLocaleString()} parts (the compared sites, first to last compared loop, without the trimmed loops)`, r.path), h("button", { onclick: () => loadPaths([r.path]) }, "Open it"));
    } catch (e) { toast(e.message, "bad"); } finally { busy(null); }
  };
  el.append(h("div", { class: "card pad section" }, h("h2", { style: { marginTop: 0 } }, "Endurance check (one file)"),
    h("p", { class: "muted small" }, "The same unit loops on each site for 1,000 runs or 1.5 hours, and the first and last loops of each site are compared. Clean skips failing loops (an unstable start, noisy loops) and compares the first and last good loops, as long as the yield over the compared span reaches the threshold; a site below it is dropped. The window cut leaves out loops past 1,000 runs or 90 minutes (time estimated from the file start and finish), where a long run degrades the unit. 0 = no cut."),
    scopeNote(),
    h("div", { class: "controls" }, field("File", eDs), field("Runs at each end", eRuns), field("Drift threshold %", eDrift), field("Yield threshold %", eYield), field("Cpk minimum", eCpk)),
    h("div", { class: "controls" }, field("Compared loops", ePick), field("Skip first loops", eSkip), field("Window ends after loops", eLoops), field("or minutes", eMin), field("Auto trim to reach the yield", eTrim), field("Trim at most % of loops", eMaxTrim), h("button", { onclick: eCheck }, "Check sites")),
    h("p", { class: "muted small" }, "Auto trim leaves out loops for every site at once (loop k is the k-th part of each site, one touchdown): first the loops at the start and end in which any site fails, then, with Ends then bad loops between, the failing loops between until every site reaches the yield. A site that still cannot reach it, or would need more loops left out than the limit (0 = no limit), is dropped instead and does not decide what is trimmed."),
    eOut, h("div", { class: "row", style: { marginTop: "10px" } }, ePf, h("button", { class: "primary", onclick: eSave }, "Export to Excel")), eRes,
    h("div", { class: "row", style: { marginTop: "6px" } }, eTPf, h("button", { onclick: eTrimSave, title: "The compared sites from their first to last compared loop, without the trimmed loops, as a new STDF. The source file is not changed." }, "Save trimmed STDF")), eTRes));

  // -- 25x loop compare
  const ids = files().map((d) => d.id);
  const cA = select(fileOpts(), String(st.c.a ?? ids[0]), (v) => (st.c.a = +v));
  const cB = select(fileOpts(), String(st.c.b ?? ids[1] ?? ids[0]), (v) => (st.c.b = +v));
  const nA = h("input", { type: "text", value: st.c.name_a || "", placeholder: "e.g. Rev 1.002", style: { width: "150px" } });
  const nB = h("input", { type: "text", value: st.c.name_b || "", placeholder: "e.g. Rev 1.003", style: { width: "150px" } });
  const cRuns = num(st.c.runs ?? 25), cDrift = num(st.c.drift ?? 5, 0.5), cCpk = num(st.c.cpk ?? 1.33, 0.01), cSkip = num(st.c.skip ?? 0);
  const cTake = select([["first", "First good loops"], ["last", "Last good loops"]], st.c.take || "first", (v) => (st.c.take = v));
  const [cPath, cPf] = pathField("save", st.c.path || defPath("loop_compare_drift.xlsx"), { title: "Save 25x loop compare", name: "loop_compare_drift.xlsx", ext: "xlsx" });
  const cOut = h("div"), cRes = h("div");
  const cParams = () => {
    Object.assign(st.c, { a: +cA.value, b: +cB.value, name_a: nA.value, name_b: nB.value, runs: +cRuns.value, drift: +cDrift.value, cpk: +cCpk.value, skip: +cSkip.value, take: cTake.value, path: cPath.value });
    return { a: st.c.a, b: st.c.b, name_a: st.c.name_a, name_b: st.c.name_b, runs: st.c.runs, drift: st.c.drift, cpk: st.c.cpk, skip: st.c.skip, take: st.c.take, filter: filt() };
  };
  const cCheck = async () => {
    if (files().length < 2) return showErr(cOut, new Error("Open the old and the new file"));
    if (cA.value === cB.value) return showErr(cOut, new Error("Pick two different files"));
    loading(cOut);
    try {
      const p = cParams(), d = await api("loop_compare_check", p);
      const ok = d.sites.filter((s) => s.ok).length;
      const nm = [p.name_a || dsName(p.a), p.name_b || dsName(p.b)];
      cOut.replaceChildren(
        h("div", { class: "kpis" }, kpi("Sites compared", `${ok} of ${d.sites.length}`, `${p.runs} good loops in both files`),
          kpi("Tests paired", d.pairs.toLocaleString(), `${d.only_old} only old, ${d.only_new} only new`),
          kpi("Changed", `${d.limits_changed} limits`, `${d.renamed} names`),
          kpi(`Drift above ${p.drift}%`, d.over.toLocaleString(), `of ${d.rows.toLocaleString()} test-site rows${d.short_rows ? `, ${d.short_rows} with fewer than ${p.runs} values` : ""}`)),
        h("div", { class: "tbl-wrap" }, h("table", {},
          h("thead", {}, h("tr", {}, ["Site", `${nm[0]} good`, `${nm[0]} loops used`, `${nm[1]} good`, `${nm[1]} loops used`, "Status"].map((x) => h("th", {}, x)))),
          h("tbody", {}, d.sites.map((s) => h("tr", {}, h("td", {}, s.label), h("td", {}, `${s.good[0]} of ${s.loops[0]}`), h("td", {}, s.used[0]), h("td", {}, `${s.good[1]} of ${s.loops[1]}`), h("td", {}, s.used[1]), h("td", {}, statusBadge(s))))))),
        d.top.length ? h("h3", {}, "Largest drifts") : "", topTable(d.top, p.drift));
    } catch (e) { showErr(cOut, e); }
  };
  const cSave = async () => {
    if (cA.value === cB.value) return toast("Pick two different files", "bad");
    if (!(await askPath(cPath))) return;
    busy("Writing the 25x loop compare…");
    try {
      const r = await api("loop_compare_export", Object.assign(cParams(), { path: cPath.value }));
      cRes.replaceChildren(resultLine(`Saved ${r.path}: ${r.rows.toLocaleString()} test-site rows on ${r.sites} sites${r.dropped ? ` (${r.dropped} dropped)` : ""}, ${r.over.toLocaleString()} above the drift threshold`, r.path));
    } catch (e) { toast(e.message, "bad"); } finally { busy(null); }
  };
  el.append(h("div", { class: "card pad section" }, h("h2", { style: { marginTop: 0 } }, "25x loop compare (old vs new file)"),
    h("p", { class: "muted small" }, "The same units loop on the old and the new test program. Each site takes its first (or last) 25 good loops from each file; a site without 25 good loops in both files (an unstable or inactive site) is dropped, and extra loops are left out. Tests pair by number and name, else by number. Drift uses the old file's limits."),
    scopeNote(),
    h("div", { class: "controls" }, field("Old (reference)", cA), field("Name", nA), field("New", cB), field("Name", nB)),
    h("div", { class: "controls" }, field("Loops per site", cRuns), field("Take", cTake), field("Skip first loops", cSkip), field("Drift threshold %", cDrift), field("Cpk minimum", cCpk), h("button", { onclick: cCheck }, "Check sites")),
    cOut, h("div", { class: "row", style: { marginTop: "10px" } }, cPf, h("button", { class: "primary", onclick: cSave }, "Export to Excel")), cRes));
  if (files().length) eCheck();
  if (files().length > 1 && cA.value !== cB.value) cCheck();
}
// ---- Part datalog: pick parts, pick tests, write their results
const DL_MODES = [["part_id", "Part ID"], ["xy", "PRR X/Y"], ["part_txt", "Part text"], ["wxy", "WXY test"], ["test_xy", "W,X,Y from tests"]];
/** Wafer, X and Y from tests as the part list shows them: the raw values as logged, W24X111Y146 (no offsets). */
function testXyLabel(t) {
  if (!t.raw) return "";
  const [x, y] = t.raw;
  return t.wafer && t.wafer !== "all" ? wxyText(t.wafer, x, y) : `X${x}Y${y}`;
}
/** A search box that shows the top matches while typing; clicking one (or
 *  Enter for the first) hands it to `pick`. `find(q)` resolves to
 *  [{label, value}]. */
function typeahead(input, find, pick) {
  const list = h("div", { class: "ta-list", hidden: true });
  let items = [], seq = 0, timer = null;
  const render = () => {
    list.hidden = !items.length;
    list.replaceChildren(...items.map((it) => h("button", { class: "ta-item", onmousedown: (e) => { e.preventDefault(); choose(it); } }, h("span", { class: "ta-add" }, "＋ Add"), h("span", {}, it.label))));
  };
  const choose = (it) => { pick(it.value); input.value = ""; items = []; render(); input.focus(); };
  input.addEventListener("input", () => {
    clearTimeout(timer);
    const q = input.value.trim(), my = ++seq;
    if (!q) { items = []; render(); return; }
    timer = setTimeout(async () => { try { const r = await find(q); if (my === seq) { items = r; render(); } } catch (e) { items = []; render(); } }, 150);
  });
  input.addEventListener("keydown", (e) => { if (e.key === "Enter" && items.length) { e.preventDefault(); choose(items[0]); } });
  input.addEventListener("blur", () => setTimeout(() => { list.hidden = true; }, 150));
  input.addEventListener("focus", () => { if (items.length) list.hidden = false; });
  return h("div", { class: "ta" }, input, list);
}
function viewDatalog(el) {
  const ds = S.datasets.find((d) => d.id === S.ds);
  if (!S.dl || S.dl.ds !== S.ds) S.dl = { ds: S.ds, parts: [], tests: S.dl ? S.dl.tests : [], modes: S.dl ? S.dl.modes : ["part_id", "xy", "part_txt"], format: S.dl ? S.dl.format : "txt" };
  const st = S.dl;
  if (typeof st.tests === "string") st.tests = st.tests.split(",").map((x) => x.trim()).filter(Boolean);
  el.append(h("h1", {}, "Part datalog"), h("p", { class: "lead" }, `Pick parts of ${ds ? ds.name : "the dataset"} and the tests you want, then get every result with its limits as easy-to-read text or CSV. Switch the dataset at the top to use another file.`), scopeLine("Part search", true));
  const wc = waferCal();
  const calParams = partSearchParams;
  const findParts = (q, limit) => api("find_parts", Object.assign({ ds: S.ds, q, by: st.modes.join(","), limit }, calParams()));
  const label = partHitLabel;
  const has = (r) => st.parts.some((p) => p.index === r.index);
  const add = (rows) => { let n = 0; for (const r of rows) if (!has(r)) { st.parts.push(Object.assign({ mark: "" }, r)); n++; } drawList(); return n; };
  // 1 · find parts: ticked ways to match, live top 3, full list, pasted list
  const modes = h("div", { class: "row tight" }, DL_MODES.map(([k, l]) => h("label", { class: "chk", title: k === "wxy" ? `The WXY test ${idTests(true).wxy || "(set it in Settings)"}: its value, e.g. 1118126` : k === "test_xy" ? `Wafer, X and Y from tests ${[wc.w, wc.x, wc.y].filter(Boolean).join(" / ")}, the raw values as logged, e.g. W24X111Y146 or 24, 111, 146 (change the tests on the Settings page)` : k === "xy" ? "X and Y of the PRR record (empty in files that log X and Y as tests)" : "" },
    h("input", { type: "checkbox", checked: st.modes.includes(k), onchange: (e) => { st.modes = DL_MODES.map(([m]) => m).filter((m) => (m === k ? e.target.checked : st.modes.includes(m))); } }), l)),
    h("button", { class: "ghost small", onclick: () => go("settings"), title: "Tests for die X/Y" }, "Settings…"));
  const q = h("input", { type: "text", class: "filter-input", placeholder: `type a part ID (233), PRR X/Y (3,-2), part text, WXY (1118126) or W,X,Y (${wxyText(1, 118, 126)})`, style: { minWidth: "460px" } });
  const hits = h("div"), listBox = h("div"), out = h("div");
  const partSearch = typeahead(q, async (text) => {
    if (!st.modes.length) return [];
    const r = await findParts(text, 3);
    return r.rows.map((x) => ({ label: label(x) + (has(x) ? "  (in the list)" : ""), value: x }));
  }, (row) => { if (add([row])) toast(`Added part ${row.part_id}`, "good", 1500); });
  const showAll = async () => {
    if (!q.value.trim()) return;
    try {
      const r = await findParts(q.value, 500);
      if (!r.rows.length) { hits.replaceChildren(h("p", { class: "muted small" }, "No part matches.")); return; }
      const tb = h("div");
      hits.replaceChildren(h("div", { class: "row tight" }, h("span", { class: "muted small" }, `${r.total.toLocaleString()} match(es)${r.total > r.rows.length ? `, first ${r.rows.length} shown` : ""}`),
        h("button", { onclick: () => { toast(`Added ${add(r.rows)} part(s)`, "good"); } }, "＋ Add all shown")), tb);
      table(tb, [
        { key: "add", label: "", render: (v, row) => h("button", { class: "ghost", onclick: (e) => { e.stopPropagation(); add([row]); e.target.textContent = "added"; e.target.disabled = true; } }, has(row) ? "added" : "＋ Add") },
        { key: "part_id", label: "Part ID" }, { key: "part_txt", label: "Part text" }, { key: "wafer", label: "Wafer" }, { key: "x", label: "X", num: true }, { key: "y", label: "Y", num: true },
        { key: "site", label: "Site", num: true }, { key: "hbin", label: "HBin", num: true }, { key: "sbin", label: "SBin", num: true }, { key: "first_fail", label: "First fail" },
      ], r.rows, { search: false, limit: 50 });
    } catch (e) { showErr(hits, e); }
  };
  const paste = h("textarea", { rows: 4, class: "paths", placeholder: "one part per line: part IDs, X/Y or part text", style: { minWidth: "460px" } });
  const addPasted = async () => {
    try {
      const r = await findParts(paste.value, 5000);
      const n = add(r.rows);
      hits.replaceChildren(h("p", { class: "muted small" }, `Added ${n} part(s)` + (r.missing.length ? `; not found: ${r.missing.slice(0, 20).join(", ")}${r.missing.length > 20 ? "…" : ""}` : "")));
    } catch (e) { showErr(hits, e); }
  };
  // 2 · the list, each part with an optional mark (the label on the physical unit)
  function drawList() {
    if (!st.parts.length) { listBox.replaceChildren(h("p", { class: "muted small" }, "No parts yet: type above and click a match to add it.")); return; }
    listBox.replaceChildren(h("div", { class: "row tight" }, h("b", {}, `${st.parts.length} part(s)`), h("span", { class: "muted small" }, "Printed in this order (↑ ↓ to move). Mark: the label on the physical unit (1, 2, A, B1…), printed with the part"), h("button", { class: "ghost", onclick: () => { st.parts = []; drawList(); } }, "Clear")),
      h("table", { class: "dl-parts" }, h("tbody", {}, st.parts.map((p, i) => h("tr", {},
        h("td", {}, h("input", { type: "text", value: p.mark || "", placeholder: "mark", style: { width: "80px" }, oninput: (e) => (p.mark = e.target.value) })),
        h("td", {}, label(p)),
        // the datalog prints the parts in this order
        h("td", { style: { whiteSpace: "nowrap" } },
          h("button", { class: "ghost", title: "Move up", disabled: i === 0, onclick: () => { [st.parts[i - 1], st.parts[i]] = [st.parts[i], st.parts[i - 1]]; drawList(); } }, "↑"),
          h("button", { class: "ghost", title: "Move down", disabled: i === st.parts.length - 1, onclick: () => { [st.parts[i + 1], st.parts[i]] = [st.parts[i], st.parts[i + 1]]; drawList(); } }, "↓"),
          h("button", { class: "ghost x", title: "Remove", onclick: () => { st.parts.splice(i, 1); drawList(); } }, "×")))))));
  }
  // 3 · tests: live top 3 from the file's tests, chips, any number typed (N/A when missing)
  const testsBox = h("div", { class: "row tight", style: { flexWrap: "wrap" } });
  const drawTests = () => testsBox.replaceChildren(...(st.tests.length ? st.tests.map((t, i) => h("span", { class: "badge neutral", style: { display: "inline-flex", gap: "6px", alignItems: "center" } }, t,
    h("button", { class: "ghost x", title: "Remove", onclick: () => { st.tests.splice(i, 1); drawTests(); } }, "×"))) : [h("span", { class: "muted small" }, "No tests picked: every test is written.")]),
    st.tests.length ? h("button", { class: "ghost", onclick: () => { st.tests = []; drawTests(); } }, "Clear") : "");
  const addTests = (s) => { for (const t of String(s).split(/[,;]/).map((x) => x.trim()).filter(Boolean)) if (!st.tests.includes(t)) st.tests.push(t); drawTests(); };
  const tq = h("input", { type: "text", placeholder: "type a test number or name (1013, DUMMY…); Enter adds what you typed", style: { minWidth: "360px" } });
  const testSearch = typeahead(tq, async (text) => {
    const ts = await testsFor(S.ds, "");
    const l = text.toLowerCase().replace(/:.*/, "");
    // numbers starting with what was typed first, then names containing it
    const rank = (t) => (String(t.num) === l ? 0 : String(t.num).startsWith(l) ? 1 : t.key.toLowerCase().includes(l) ? 2 : 9);
    const m = ts.filter((t) => rank(t) < 9).sort((a, b) => rank(a) - rank(b)).slice(0, 3);
    return m.map((t) => ({ label: `${t.key}${t.units ? ` [${t.disp_units || t.units}]` : ""}${st.tests.includes(t.key) ? "  (picked)" : ""}`, value: t.key }));
  }, addTests);
  tq.addEventListener("keydown", (e) => { if (e.key === "Enter" && tq.value.trim()) { e.preventDefault(); addTests(tq.value); tq.value = ""; } });
  const format = select([["txt", "easy-to-read text (one block per part)"], ["csv", "CSV, one row per part"], ["csv_long", "CSV, one row per result"]], st.format, (v) => (st.format = v));
  const ext = () => (st.format === "txt" ? "txt" : "csv");
  const base = () => `${(ds ? ds.name : "datalog").replace(/\.(stdf|std)(\.gz)?$/i, "")}_datalog`;
  const make = (path) => api("part_datalog", { ds: S.ds, rows: st.parts.map((p) => p.index), marks: st.parts.map((p) => p.mark || ""), tests: st.tests.join(", "), format: format.value, path: path || "" });
  const note = (r) => `${r.n_parts} part(s) × ${r.n_tests} test(s)` + (r.missing && r.missing.length ? ` · not in this file, written as N/A: ${r.missing.join(", ")}` : "");
  const preview = async () => {
    try {
      const r = await make();
      const lines = r.text.split("\n");
      out.replaceChildren(h("p", { class: "muted small" }, note(r) + (lines.length > 400 ? ` · first 400 of ${lines.length} lines shown` : "")),
        h("pre", { class: "card pad", style: { overflow: "auto", maxHeight: "560px", fontSize: "12px" } }, lines.slice(0, 400).join("\n")));
    } catch (e) { showErr(out, e); }
  };
  const download = async () => { try { const r = await make(); SC.download(`${base()}.${ext()}`, r.text, ext() === "csv" ? "text/csv" : "text/plain"); } catch (e) { toast(e.message, "bad"); } };
  const native = S.info && S.info.native_dialogs;
  const saveAs = async () => {
    try {
      const p = await api("pick_save", { title: "Save datalog", name: `${base()}.${ext()}`, ext: ext(), dir: lastDir() || (ds && ds.paths.length ? dirname(ds.paths[0]) : "") });
      if (!p.path) return;
      setLastDir(dirname(p.path));
      const r = await make(p.path);
      out.replaceChildren(resultLine(`Saved ${r.path}`, r.path), h("p", { class: "muted small" }, note(r)));
    } catch (e) { toast(e.message, "bad"); }
  };
  el.append(h("div", { class: "card pad section" }, h("h2", { style: { marginTop: 0 } }, "1 · Find parts"),
      h("div", { class: "row tight" }, h("span", { class: "muted small" }, "Match by"), modes),
      h("div", { class: "row tight" }, partSearch, h("button", { onclick: showAll }, "Show all matches")),
      h("details", { class: "adv" }, h("summary", {}, "Paste a list of parts"), paste, h("button", { onclick: addPasted }, "＋ Add the list")), hits),
    h("div", { class: "card pad section" }, h("h2", { style: { marginTop: 0 } }, "2 · Parts in the datalog"), listBox),
    h("div", { class: "card pad section" }, h("h2", { style: { marginTop: 0 } }, "3 · Tests and output"),
      h("div", { class: "row tight" }, testSearch), testsBox,
      h("div", { class: "controls" }, field("Format", format),
        h("button", { onclick: preview }, "Preview"), native ? h("button", { class: "primary", onclick: saveAs }, "Save as…") : null, h("button", { class: native ? "" : "primary", onclick: download }, "Download")), out));
  drawList();
  drawTests();
}
// ---- Experiments (Settings, "Experiments"): touchdown and loop per part; off unless switched on
function touchOn() { return !!(S.cfg.touch && S.cfg.touch.on); }
function touchCfg() { return Object.assign({ base: 1, key: "auto" }, S.cfg.touch || {}); }
function setTouch(patch) { S.cfg.touch = Object.assign({ on: false, base: 1, key: "auto" }, S.cfg.touch, patch); saveCfg(); }
function touchEstimate(ds) { const c = touchCfg(); return api("touch_estimate", { ds, base: Number(c.base), key: c.key }); }
const LOOP_NAMES = { Id: "part ID", Text: "part text", Off: "no repeats" };
/** Numbers and ranges such as "3-5,9", as a set. */
function numSet(text) {
  const out = new Set();
  for (const tok of String(text).split(/[,;\s]+/).filter(Boolean)) {
    const m = tok.match(/^(\d+)\s*-\s*(\d+)$/);
    if (m) { for (let k = Number(m[1]); k <= Number(m[2]); k++) out.add(k); } else if (/^\d+$/.test(tok)) out.add(Number(tok));
  }
  return out;
}
/** The touchdown summary of an estimate, for the Records view. */
function touchCard(est) {
  const rows = est.touchdowns.map((t) => ({ touch: t.touch, sites: t.sites.join(", "), parts: t.parts, good: t.good, bad: t.bad, cut_off: t.cut_off, test_ms: t.test_ms_median == null ? "" : t.test_ms_median.toFixed(0), loops: t.loop_min === t.loop_max ? String(t.loop_min) : `${t.loop_min}–${t.loop_max}` }));
  const tb = h("div");
  table(tb, [{ key: "touch", label: "Touchdown", num: true }, { key: "sites", label: "Sites" }, { key: "parts", label: "Parts", num: true }, { key: "good", label: "Good", num: true }, { key: "bad", label: "Bad", num: true }, { key: "cut_off", label: "Cut off", num: true }, { key: "test_ms", label: "Test time ms", num: true }, { key: "loops", label: "Loops", num: true }], rows, { search: rows.length > 15, maxHeight: "360px" });
  return h("div", { class: "card pad section" }, h("h2", { style: { marginTop: 0 } }, "Touchdowns (experiment)"),
    h("p", { class: "muted small" }, `${est.n_touchdowns} touchdown(s) of up to ${est.max_sites} sites, found from ${est.method}. Loops are counted by ${LOOP_NAMES[est.loop_key] || est.loop_key}: a sample tested again on each loop. Estimated, not read from the tester: check it against your own records before you rely on it.`),
    tb);
}

// ---- Edit & trim: pick parts by ID or row, keep or remove them, number the rest, save a new STDF
const TRIM_PAGE = 200;
function viewTrim(el) {
  const ds = S.datasets.find((d) => d.id === S.ds);
  el.append(h("h1", {}, "Edit & trim"), h("p", { class: "lead" }, `Choose the parts of ${ds ? ds.name : "the dataset"} to keep or remove, number the parts that stay if you like, and save a new STDF file. The file you opened is not changed.`), scopeLine("The part list", true));
  if (!ds) { el.append(h("p", { class: "muted" }, "Open an STDF file first.")); return; }
  if (!S.trim || S.trim.ds !== S.ds) S.trim = { ds: S.ds, sel: new Set(), anchor: null, page: 0, q: "", show: "all", action: "keep", renumber: false, method: "rules", op: "replace", numbers: "", by: "id", filter: "", result: "any", sites: null, touchText: "", quota: "", quotaDecided: "" };
  const body = h("div");
  el.append(body);
  loading(body, "Reading the parts…");
  Promise.all([api("trim_list", { ds: S.ds }), api("site_yield", { ds: S.ds }), touchOn() ? touchEstimate(S.ds) : null])
    .then(([data, yields, est]) => drawTrim(body, ds, data, yields, est))
    .catch((e) => showErr(body, e));
}
function drawTrim(body, ds, data, yields, est) {
  body.replaceChildren();
  if (!data.editable) { body.append(h("div", { class: "card pad" }, "Trimming works on a dataset opened from one STDF file. This dataset was combined or made in the app: open its file on its own to trim it.")); return; }
  const st = S.trim, sel = st.sel, rows = data.rows, n = rows.length;
  // row: [index, part ID, part text, wafer, X, Y, site, hbin, sbin, pass, cut off, first fail], then touchdown and loop when estimated
  if (est) rows.forEach((r, i) => r.push(est.touch[i], est.loop[i]));
  const sites = yields.sites || [];
  if (!st.sites) st.sites = new Set(sites.map((s) => s.site));
  const hay = rows.map((r) => `${r[1]} ${r[2]} ${r[3]} ${r[4] ?? ""} ${r[5] ?? ""}`.toLowerCase());
  const tn = () => (est && st.touchText.trim() ? numSet(st.touchText) : null);
  const resultOk = (r) => st.result === "any" || (!r[10] && (st.result === "good" ? r[9] === true : r[9] === false));
  const written = () => (st.action === "keep" ? [...sel].sort((a, b) => a - b) : rows.map((_, i) => i).filter((i) => !sel.has(i)));
  const quotaKey = () => `${st.quota}|${[...st.sites].sort((a, b) => a - b).join(",")}|${st.filter}|${st.touchText}`;

  // the operation's live message, and the candidates (the preview) of the chosen method
  const say = h("div", { class: "muted small", role: "status", "aria-live": "polite" });
  const preview = h("div", { class: "muted small" });
  let cand = [];
  let plan = null;
  let seq = 0, timer = null;
  let fcache = { expr: null, set: null };
  const filterSet = async (expr) => {
    if (!expr.trim()) return null;
    if (fcache.expr !== expr) { const r = await api("trim_resolve", { ds: S.ds, filter: expr }); fcache = { expr, set: new Set(r.rows) }; }
    return fcache.set;
  };
  const schedule = (ms) => { clearTimeout(timer); timer = setTimeout(computePreview, ms || 0); };
  const computePreview = async () => {
    const my = ++seq;
    try {
      if (st.method === "numbers") {
        if (!st.numbers.trim()) { preview.textContent = "Type part IDs or row numbers."; cand = []; }
        else {
          const r = await api("trim_resolve", { ds: S.ds, text: st.numbers, by: st.by });
          if (my !== seq) return;
          cand = r.rows;
          preview.textContent = `${r.count.toLocaleString()} part(s) for these numbers` + (r.missing.length ? `; not in the file: ${r.missing.join(", ")}` : "") + (r.shared.length ? `; part IDs on more than one part (all included): ${r.shared.join(", ")}` : "");
        }
        plan = null;
      } else if (st.method === "rules") {
        const fs = await filterSet(st.filter);
        if (my !== seq) return;
        cand = rows.filter((r) => (!fs || fs.has(r[0])) && st.sites.has(r[6]) && (!tn() || tn().has(r[12])) && resultOk(r)).map((r) => r[0]);
        preview.textContent = !st.sites.size ? "No site is included: tick sites in the yield table." : `${cand.length.toLocaleString()} part(s) match these rules`;
        plan = null;
      } else {
        const target = Number(st.quota);
        if (!(target > 0)) { cand = []; plan = null; preview.textContent = "Enter how many good parts per site."; }
        else {
          const fs = await filterSet(st.filter);
          if (my !== seq) return;
          const pool = rows.filter((r) => r[9] === true && !r[10] && (!fs || fs.has(r[0])) && st.sites.has(r[6]) && (!tn() || tn().has(r[12]))).map((r) => r[0]);
          const q = await api("trim_quota", { ds: S.ds, target, candidates: pool, result: "good", drop: [] });
          if (my !== seq) return;
          plan = q;
          cand = q.rows;
          preview.textContent = `${q.taken.toLocaleString()} good part(s): ${target.toLocaleString()} from each of ${q.per_site.length} site(s)` + (q.shortfall ? `; ${q.shortfall.toLocaleString()} short, where a site has fewer than ${target.toLocaleString()}` : "");
          // a site short of its share: ask whether to drop it, unless the user already answered for this setting
          if (q.short.length && st.quotaDecided !== quotaKey()) {
            const names = q.short.map((s) => `site ${s}`).join(", ");
            shortBox.replaceChildren(h("p", {}, `${names} ${q.short.length > 1 ? "have" : "has"} fewer than ${target.toLocaleString()} good parts, the number wanted from each site. Drop ${q.short.length > 1 ? "them" : "it"} from this quota pick? Keeping ${q.short.length > 1 ? "them" : "it"} takes all the good parts ${q.short.length > 1 ? "they have" : "it has"}.`),
              h("div", { class: "row tight" },
                h("button", { class: "primary", onclick: () => { for (const s of q.short) st.sites.delete(s); st.quotaDecided = ""; syncSites(); schedule(0); } }, `Yes, drop ${names}`),
                h("button", { onclick: () => { st.quotaDecided = quotaKey(); schedule(0); } }, `No, keep ${names}`)));
            shortBox.hidden = false;
          } else shortBox.hidden = true;
        }
      }
    } catch (e) { if (my === seq) { preview.textContent = e.message; cand = []; } }
    if (my === seq) paint();
  };

  // apply the candidate rows to the selection: replace, add or take out
  const applyRows = (list) => {
    let added = 0, removed = 0;
    if (st.op === "replace") {
      const want = new Set(list);
      for (const i of [...sel]) if (!want.has(i)) { sel.delete(i); removed++; }
      for (const i of list) if (!sel.has(i)) { sel.add(i); added++; }
    } else if (st.op === "add") {
      for (const i of list) if (!sel.has(i)) { sel.add(i); added++; }
    } else {
      for (const i of list) if (sel.delete(i)) removed++;
    }
    st.anchor = null;
    say.textContent = `Added ${added.toLocaleString()}, removed ${removed.toLocaleString()}. ${sel.size.toLocaleString()} part(s) selected.`;
    paint();
  };
  const applyNow = async () => { await computePreview(); apply(); };
  const apply = () => {
    if (!cand.length) { say.textContent = "Nothing matched: the selection is unchanged."; return; }
    applyRows(cand);
  };

  // the committed selection, always visible
  const stripCount = h("b", {});
  const stripKinds = h("span", { class: "muted small" });
  const strip = h("div", { class: "trim-strip card pad" },
    h("div", { class: "row tight", style: { flexWrap: "wrap" } }, h("span", { class: "strip-title" }, "Selected"), stripCount, stripKinds,
      h("button", { class: "ghost small", title: "list only the selected parts", onclick: () => { st.show = "selected"; st.page = 0; showSel.value = "selected"; paint(); } }, "Review selected"),
      h("button", { class: "ghost small", onclick: () => { sel.clear(); st.anchor = null; say.textContent = "Selection cleared."; paint(); } }, "Clear selection")));

  // 1 · site eligibility, in the yield table
  const siteChecks = new Map();
  const totalYield = yields.total.yield_pct;
  const lowYield = (y) => y !== null && totalYield !== null && y < totalYield - 10;
  const syncSites = () => { for (const [s, b] of siteChecks) b.checked = st.sites.has(s); };
  const yieldTable = h("div", { class: "tbl-wrap" }, h("table", {},
    h("thead", {}, h("tr", {}, ...["In", "Site", "Parts", "Good", "Bad", "Cut off", "Yield", ""].map((c) => h("th", { class: ["Site", "Parts", "Good", "Bad", "Cut off", "Yield"].includes(c) ? "num" : "" }, c)))),
    h("tbody", {}, ...sites.map((s) => {
      const box = h("input", { type: "checkbox", "aria-label": `include site ${s.site} in the rules and the quota`, onchange: (e) => { if (e.target.checked) st.sites.add(s.site); else st.sites.delete(s.site); st.quotaDecided = ""; schedule(0); } });
      siteChecks.set(s.site, box);
      return h("tr", {},
        h("td", {}, box),
        h("td", { class: "num" }, s.site),
        h("td", { class: "num" }, s.parts.toLocaleString()),
        h("td", { class: "num" }, s.good.toLocaleString()),
        h("td", { class: "num" }, s.bad.toLocaleString()),
        h("td", { class: "num" }, s.cut_off ? s.cut_off.toLocaleString() : ""),
        h("td", { class: `num ${lowYield(s.yield_pct) ? "flag-fail" : ""}` }, s.yield_pct === null ? "–" : `${s.yield_pct.toFixed(1)} %`),
        h("td", {}, lowYield(s.yield_pct) ? h("span", { class: "flag-fail" }, "low yield") : ""));
    }),
    h("tfoot", {}, h("tr", {}, h("td", {}), h("td", {}, "All"), h("td", { class: "num" }, yields.total.parts.toLocaleString()), h("td", { class: "num" }, yields.total.good.toLocaleString()), h("td", { class: "num" }, yields.total.bad.toLocaleString()), h("td", { class: "num" }, yields.total.cut_off ? yields.total.cut_off.toLocaleString() : ""), h("td", { class: "num" }, totalYield === null ? "–" : `${totalYield.toFixed(1)} %`), h("td", {}))))));
  const allSites = h("button", { class: "ghost small", onclick: () => { st.sites = new Set(sites.map((s) => s.site)); st.quotaDecided = ""; schedule(0); } }, "All sites");

  // 2 · choose parts: one method at a time, then one operation
  const methods = [["numbers", "By numbers"], ["rules", "By rules"], ["quota", "Quota"]];
  const methodBtns = methods.map(([k, label]) => [k, h("button", { class: "ghost", onclick: () => { st.method = k; plan = null; schedule(0); } }, label)]);
  const numIn = h("input", { type: "text", value: st.numbers, placeholder: "e.g. 1-3,5,2000-2012", style: { minWidth: "300px" }, oninput: (e) => { st.numbers = e.target.value; schedule(250); }, onkeydown: (e) => { if (e.key === "Enter") { e.preventDefault(); applyNow(); } } });
  const byEl = select([["id", "part IDs"], ["row", "row numbers (1 = first part)"]], st.by, (v) => { st.by = v; schedule(0); });
  const numPanel = h("fieldset", { class: "trim-panel" }, h("legend", {}, "By numbers"), field("Part IDs or row numbers", numIn), field("Read the numbers as", byEl), h("p", { class: "muted small" }, "Ranges and lists, e.g. 1-3,5,2000-2012. Enter applies the operation."));
  const filterIn = h("input", { type: "text", value: st.filter, placeholder: "e.g. hbin = 1 and not pass", style: { minWidth: "300px" }, oninput: (e) => { st.filter = e.target.value; schedule(250); }, onkeydown: (e) => { if (e.key === "Enter") { e.preventDefault(); applyNow(); } } });
  const resultSel = select([["any", "all results (incl. cut off and unknown)"], ["good", "good only"], ["bad", "bad only"]], st.result, (v) => { st.result = v; schedule(0); });
  const touchIn = est ? h("input", { type: "text", value: st.touchText, placeholder: "e.g. 3-5,9", style: { minWidth: "180px" }, oninput: (e) => { st.touchText = e.target.value; schedule(250); } }) : null;
  const rulesPanel = h("fieldset", { class: "trim-panel" }, h("legend", {}, "By rules"), field("Filter expression", filterIn), field("Result", resultSel), touchIn ? field("Touchdown numbers", touchIn) : null,
    h("p", { class: "muted small" }, "Rules that are set must all hold. Sites are ticked in the yield table above. Blank rules do not restrict."));
  const quotaIn = h("input", { type: "number", min: "1", value: st.quota, placeholder: "e.g. 25", style: { width: "100px" }, oninput: (e) => { st.quota = e.target.value; st.quotaDecided = ""; schedule(250); }, onkeydown: (e) => { if (e.key === "Enter") { e.preventDefault(); applyNow(); } } });
  const shortBox = h("div", { class: "quota-ask card pad", hidden: true, role: "alertdialog", "aria-label": "Sites short of their share" });
  const quotaTable = h("div", {});
  const quotaPanel = h("fieldset", { class: "trim-panel" }, h("legend", {}, "Quota"), field("Good parts per site", quotaIn),
    h("p", { class: "muted small" }, "This many good parts from every site ticked in the yield table, taken in file order and within the filter and touchdown rules. A site with fewer good parts gives all it has."),
    shortBox, quotaTable);
  // the operation, then apply
  const opRadios = [["replace", "Replace selection"], ["add", "Add to selection"], ["take", "Take out of selection"]].map(([k, label]) =>
    h("label", { class: "chk" }, h("input", { type: "radio", name: "trim-op", value: k, checked: st.op === k, onchange: () => { st.op = k; } }), label));
  const applyBtn = h("button", { class: "primary", onclick: apply }, "Apply to selection");

  // 3 · review the parts: search, show, paged table, bulk apply
  const search = h("input", { type: "text", value: st.q, placeholder: "search ID, text, wafer, X/Y", "aria-label": "search this table", style: { minWidth: "240px" }, oninput: (e) => { st.q = e.target.value; st.page = 0; paint(); } });
  const showSel = select([["all", "all parts"], ["selected", "selected only"], ["not", "not selected"]], st.show, (v) => { st.show = v; st.page = 0; paint(); });
  const listBody = h("tbody");
  const pageLbl = h("span", { class: "muted small" });
  const prev = h("button", { class: "ghost small", onclick: () => { st.page--; paint(); } }, "‹ Previous");
  const next = h("button", { class: "ghost small", onclick: () => { st.page++; paint(); } }, "Next ›");
  const shown = () => {
    const q = st.q.trim().toLowerCase(), list = [];
    for (let i = 0; i < n; i++) {
      if (st.show === "selected" && !sel.has(i)) continue;
      if (st.show === "not" && sel.has(i)) continue;
      if (q && !hay[i].includes(q)) continue;
      list.push(i);
    }
    return list;
  };
  let curList = [], rowKey = "", rowEls = [], rowBoxes = [];
  const click = (i, e) => {
    const list = curList;
    const a = st.anchor === null ? -1 : list.indexOf(st.anchor), b = list.indexOf(i);
    if (e.shiftKey && a >= 0 && b >= 0) {
      if (!(e.ctrlKey || e.metaKey)) sel.clear();
      for (let k = Math.min(a, b); k <= Math.max(a, b); k++) sel.add(list[k]);
    } else if (e.ctrlKey || e.metaKey) {
      if (sel.has(i)) sel.delete(i); else sel.add(i);
      st.anchor = i;
    } else {
      sel.clear();
      sel.add(i);
      st.anchor = i;
    }
    paint();
  };
  const bulkBtn = h("button", { class: "ghost small", onclick: () => applyRows(shown()) }, "Apply operation to all listed");
  const listCount = h("span", { class: "muted small" });

  // 4 · output and save
  const actionSel = select([["keep", "keep the selected parts"], ["remove", "remove the selected parts"]], st.action, (v) => { st.action = v; paint(); });
  const idSel = select([["keep", "keep the part IDs as they are"], ["backfill", "backfill: renumber the parts that stay 1…N"]], st.renumber ? "backfill" : "keep", (v) => { st.renumber = v === "backfill"; paint(); });
  const writeLine = h("div", { class: "muted" });
  const idLine = h("div", { class: "muted small" });
  const out = h("div");
  const base = ds.name.replace(/\.(stdf|std)(\.gz)?$/i, "").replace(/\.gz$/i, "");
  const srcDir = ds.paths && ds.paths.length ? dirname(ds.paths[0]) : "";
  const sep = srcDir.includes("\\") ? "\\" : "/";
  const [pathBox, pathF] = pathField("save", srcDir ? srcDir + sep + base + "_trim.stdf" : "", { title: "Save trimmed STDF", name: base + "_trim.stdf", ext: "stdf" });
  const saveBtnGroup = saveSplit("Save trimmed STDF", async (open) => {
    if (!(await askPath(pathBox))) return;
    busy("Writing trimmed STDF…");
    try {
      const r = await api("trim_save", { ds: S.ds, rows: [...sel], action: st.action, renumber: st.renumber, path: pathBox.value });
      out.replaceChildren(resultLine(`Saved ${r.path}: ${r.kept.toLocaleString()} parts (${r.good.toLocaleString()} good), removed ${r.removed.toLocaleString()}${r.first_id ? `, IDs ${r.first_id} … ${r.last_id}` : ""}`, r.path), h("button", { onclick: () => loadPaths([r.path]) }, "Open it"));
      if (open) loadPaths([r.path]);
    } catch (e) { toast(e.message, "bad"); } finally { busy(null); }
  });

  // paint: everything that depends on the selection, the method and the page
  const paint = () => {
    methodBtns.forEach(([k, b]) => { b.className = st.method === k ? "primary" : "ghost"; b.setAttribute("aria-pressed", String(st.method === k)); });
    numPanel.hidden = st.method !== "numbers";
    rulesPanel.hidden = st.method !== "rules";
    quotaPanel.hidden = st.method !== "quota";
    applyBtn.disabled = !shortBox.hidden;
    syncSites();

    const list = shown();
    curList = list;
    const pages = Math.max(1, Math.ceil(list.length / TRIM_PAGE));
    st.page = Math.max(0, Math.min(st.page, pages - 1));
    const at = st.page * TRIM_PAGE;
    const slice = list.slice(at, at + TRIM_PAGE);
    const key = slice.join(",");
    if (key !== rowKey) {
      rowKey = key;
      rowBoxes = [];
      rowEls = slice.map((i) => {
        const r = rows[i];
        const box = h("input", { type: "checkbox", "aria-label": `select part ${r[1]}, row ${i + 1}`, onclick: (e) => { e.stopPropagation(); if (sel.has(i)) sel.delete(i); else sel.add(i); st.anchor = i; paint(); } });
        rowBoxes.push(box);
        const cells = [h("td", {}, box), h("td", { class: "num" }, i + 1), h("td", {}, r[1]), h("td", {}, r[2]), h("td", {}, r[3]),
          h("td", { class: "num" }, r[4]), h("td", { class: "num" }, r[5]), h("td", { class: "num" }, r[6]),
          h("td", { class: "num" }, r[7]), h("td", { class: "num" }, r[8]),
          h("td", {}, r[10] ? "cut off" : r[9] === null ? "n/a" : r[9] ? "good" : "bad"), h("td", {}, r[11])];
        if (est) cells.push(h("td", { class: "num" }, r[12]), h("td", { class: "num" }, r[13]));
        return h("tr", { class: "clickable", onclick: (e) => click(i, e) }, ...cells);
      });
      listBody.replaceChildren(...rowEls);
    }
    slice.forEach((i, k) => {
      rowEls[k].className = sel.has(i) ? "clickable sel" : "clickable";
      rowBoxes[k].checked = sel.has(i);
    });
    prev.disabled = st.page === 0;
    next.disabled = st.page >= pages - 1;
    pageLbl.textContent = `page ${st.page + 1} of ${pages.toLocaleString()} · ${list.length.toLocaleString()} part(s) listed`;
    listCount.textContent = `${list.length.toLocaleString()} listed`;
    bulkBtn.disabled = !list.length;

    const goodN = [...sel].filter((i) => rows[i][9] === true).length;
    const badN = [...sel].filter((i) => rows[i][9] === false).length;
    const cutN = [...sel].filter((i) => rows[i][10]).length;
    stripCount.textContent = `${sel.size.toLocaleString()} of ${n.toLocaleString()} parts`;
    stripKinds.textContent = `${goodN.toLocaleString()} good · ${badN.toLocaleString()} bad · ${cutN.toLocaleString()} cut off`;

    // quota: the allocation per site, before it is applied
    if (st.method === "quota" && plan && plan.per_site.length) {
      quotaTable.replaceChildren(h("table", {},
        h("thead", {}, h("tr", {}, h("th", {}, "Site"), h("th", { class: "num" }, "Good available"), h("th", { class: "num" }, "Taken"))),
        h("tbody", {}, ...plan.per_site.map((p) => h("tr", {}, h("td", {}, p.site), h("td", { class: "num" }, p.available), h("td", { class: "num" }, p.take))))));
    } else quotaTable.replaceChildren();

    const w = written(), removed = n - w.length;
    const good = w.filter((i) => rows[i][9] === true).length, cut = w.filter((i) => rows[i][10]).length;
    writeLine.textContent = w.length
      ? `Will write ${w.length.toLocaleString()} part(s) (${good.toLocaleString()} good) and remove ${removed.toLocaleString()}.` + (cut ? ` ${cut} part(s) cut off before their PRR are written without bin or verdict.` : "")
      : "Nothing would be written: keep at least one part.";
    const idOf = (i) => rows[i][1] || "–";
    idLine.textContent = !w.length ? "" : st.renumber
      ? `Part IDs will run 1 … ${w.length.toLocaleString()}.`
      : `Part IDs keep their numbers, ${idOf(w[0])} … ${idOf(w[w.length - 1])}${removed ? ", with gaps where parts were removed" : ""}.`;
    saveBtnGroup.querySelectorAll("button").forEach((b) => (b.disabled = !w.length));
  };

  body.append(
    strip,
    h("div", { class: "card pad section" }, h("h2", { style: { marginTop: 0 } }, "1 · Sites and yield"),
      h("p", { class: "muted small" }, "Tick the sites the rules and the quota may use. Yield is good ÷ (good + bad), cut-off parts left out. A site more than 10 points below the file's yield is flagged."),
      h("div", { class: "row tight" }, allSites),
      yieldTable),
    h("div", { class: "card pad section" }, h("h2", { style: { marginTop: 0 } }, "2 · Choose parts"),
      h("div", { class: "row tight", role: "group", "aria-label": "method" }, ...methodBtns.map(([, b]) => b)),
      numPanel, rulesPanel, quotaPanel,
      h("div", { class: "row tight", style: { flexWrap: "wrap" }, role: "radiogroup", "aria-label": "operation" }, h("span", { class: "muted small" }, "Then:"), ...opRadios, applyBtn),
      preview, say),
    h("div", { class: "card pad section" }, h("h2", { style: { marginTop: 0 } }, "3 · Review parts"),
      h("div", { class: "row tight", style: { flexWrap: "wrap" } }, search, showSel, bulkBtn, listCount),
      h("div", { class: "tbl-wrap trim-list" }, h("table", {},
        h("thead", {}, h("tr", {}, ...["", "#", "Part ID", "Part text", "Wafer", "X", "Y", "Site", "HBin", "SBin", "Result", "First fail", ...(est ? ["Touchdown", "Loop"] : [])].map((c, k) => h("th", { class: (k >= 5 && k <= 9) || c === "Touchdown" || c === "Loop" ? "num" : "", "aria-label": c === "" ? "select" : null }, c)))),
        listBody)),
      h("div", { class: "row tight" }, prev, pageLbl, next),
      h("p", { class: "muted small" }, "Click a part to select only it. Ctrl+click adds or removes one part, Shift+click selects the range from the last part clicked. The box on each row selects it from the keyboard too. The search only finds parts; it does not change the selection.")),
    h("div", { class: "card pad section" }, h("h2", { style: { marginTop: 0 } }, "4 · Output and save"),
      h("div", { class: "row tight", style: { flexWrap: "wrap" } }, field("Selected parts", actionSel), field("Part IDs", idSel)),
      writeLine, idLine,
      h("p", { class: "muted" }, "Records are copied byte-for-byte; the bin and pass/fail summaries are rebuilt for the parts written. Test synopsis (TSR) records are not written, as in Merge, split & export."),
      h("div", { class: "row" }, pathF, saveBtnGroup), out));
  if (!st.sites.size) say.textContent = "No site is included: tick sites in the yield table.";
  schedule(0);
  paint();
}

// ---- Settings: defaults used across the views, kept in this browser profile
/** A label form for Settings: a preset or your own template, checked by the server (which writes the reports). */
function labelPick(key, title, presets, example, help) {
  const cur = S.cfg[key] || presets[0][0];
  const isPreset = presets.some(([t]) => t === cur);
  const sel = select([...presets.map(([t, ex]) => [t, ex]), ["custom", "Custom…"]], isPreset ? cur : "custom", () => sync());
  const own = h("input", { type: "text", value: isPreset ? "" : cur, placeholder: help, style: { width: "200px" }, hidden: isPreset, onchange: () => sync() });
  const shown = h("span", { class: "muted small" }, `e.g. ${example(cur)}`);
  async function sync() {
    own.hidden = sel.value !== "custom";
    const t = sel.value === "custom" ? own.value.trim() : sel.value;
    if (!t) return;
    const next = Object.assign({ wxy: S.cfg.wxyFmt || "", test: S.cfg.testFmt || "" }, key === "wxyFmt" ? { wxy: t } : { test: t });
    try {
      await api("label_formats", next);
      S.cfg[key] = t; saveCfg(); S.featured = {};
      shown.textContent = `e.g. ${example(t)}`;
      toast("Saved", "good", 1500);
    } catch (e) { toast(e.message, "bad"); }
  }
  return field(title, h("div", { class: "row tight" }, sel, own, shown));
}
function viewSettings(el) {
  el.append(h("h1", {}, "Settings"), h("p", { class: "lead" }, "Shared settings, saved on this PC. Every page uses them: change one here and it changes everywhere. Pages show the value they use, with a link back here."));
  const sec = (title, note, ...kids) => h("div", { class: "card pad section" }, h("h2", { style: { marginTop: 0 } }, title), note ? h("p", { class: "muted small" }, note) : null, ...kids);
  const saved = () => { S.rtLast = null; saveCfg(); toast("Saved", "good", 1500); };
  // threads for loading several files: 0 = one per core (the server keeps the setting)
  const threadHint = h("span", { class: "muted small" }, "");
  const threads = h("input", { type: "number", min: "0", max: "256", value: "0", style: { width: "80px" }, title: "Threads used to read several files at once. 0 = one per core on this PC.",
    onchange: async (e) => { try { const r = await api("parse_threads", { threads: Number(e.target.value) || 0 }); threads.value = String(r.threads); threadHint.textContent = `0 = auto (${r.cores} cores here)`; toast(`Loading uses ${r.using} thread(s)`, "good", 1500); } catch (err) { toast(err.message, "bad"); } } });
  api("parse_threads", {}).then((r) => { threads.value = String(r.threads); threadHint.textContent = `0 = auto (${r.cores} cores here)`; }).catch(() => {});
  const ignore = h("input", { type: "checkbox", checked: S.cfg.ignoreIds !== false, onchange: (e) => { S.cfg.ignoreIds = e.target.checked; saved(); } });
  // the IDs may change: pickers forget their choice, and the order list shows what is usable
  const idChanged = () => { S.mk = {}; paintIds(); };
  const cfgIn = (k, ph, w) => h("input", { type: "text", value: S.cfg[k] || "", placeholder: ph, style: { width: w || "110px" }, onchange: (e) => { S.cfg[k] = e.target.value.trim(); idChanged(); saved(); } });
  const traced = (k, control) => h("span", { class: "row tight" }, control, traceBox(traceOn(k), (v) => { const off = Object.assign({}, S.cfg.traceOff); if (v) delete off[k]; else off[k] = true; S.cfg.traceOff = off; idChanged(); }));
  const othersBox = h("div");
  const drawOthers = () => {
    const o = S.cfg.others || (S.cfg.others = []);
    othersBox.replaceChildren(...o.map((x, i) => h("div", { class: "row tight", style: { marginBottom: "6px" } },
      h("input", { type: "text", value: x.name, placeholder: "name, e.g. Die ID", style: { width: "150px" }, onchange: (e) => { x.name = e.target.value.trim(); saved(); } }),
      h("input", { type: "text", value: x.test, placeholder: "test", style: { width: "110px" }, onchange: (e) => { x.test = e.target.value.trim(); saved(); } }),
      traceBox(!x.off, (v) => { x.off = v ? undefined : true; }), h("button", { title: "Remove", onclick: () => { o.splice(i, 1); saved(); drawOthers(); } }, "×"))),
      h("button", { onclick: () => { o.push({ name: "", test: "" }); drawOthers(); } }, "+ Add other ID"));
  };
  drawOthers();
  const lb = h("input", { type: "text", value: S.lbTest || "5100", style: { width: "90px" }, onchange: async (e) => { try { const r = await api("loadboard", { test: e.target.value.trim() }); S.lbTest = String(r.test); S.testsCache = {}; saved(); } catch (err) { toast(err.message, "bad"); } } });
  const wc = waferCal();
  const inp = (k, ph) => h("input", { type: "text", value: wc[k], placeholder: ph, style: { width: "110px" }, onchange: (e) => { wc[k] = e.target.value.trim(); idChanged(); saved(); } });
  const omode = select([["add", "X + offset"], ["sub", "X − offset"]], wc.mode, (v) => { wc.mode = v; saved(); });
  const flip = (k, l) => h("label", { class: "chk" }, h("input", { type: "checkbox", checked: !!wc[k], onchange: (e) => { wc[k] = e.target.checked; saved(); } }), l);
  const range = select([["limits", "within limits"], ["data", "results"], ["both", "both (adaptive)"]], S.co.range, (v) => { S.co.range = v; saveChartOpts(); toast("Saved", "good", 1500); });
  const hist = select([["pct", "% of group"], ["count", "counts"]], S.co.hist, (v) => { S.co.hist = v; saveChartOpts(); toast("Saved", "good", 1500); });
  const skin = select([["", "Default"], ["industry", "Industry (steel blue, serif headings)"], ["office", "Office (navy, sky blue, yellow)"], ["work", "Work (burgundy, magenta)"]], SC.getSkin(), (v) => { SC.setSkin(v); document.getElementById("skin").value = v; toast("Saved", "good", 1500); });
  const ID_ROWS = {
    part_txt: { title: "PRR part text (ULT)", note: "Unit level traceability, the code read from the unit" },
    wxy: { title: "PTR WXY", note: "One test: wafer, X and Y in one result", ctl: () => [field("Test", traced("wxy", cfgIn("wxy", "none"))), field("Check WXY against", select([["raw", "raw W, X, Y tests"], ["die", "die X/Y (offsets added)"]], wxyBasis(), (v) => { S.cfg.wxyBasis = v; idChanged(); saved(); }))] },
    w_x_y: { title: "PTR wafer + X + Y", note: "A combination: one test each, all must be logged. Written as W12X-2Y25; with Check WXY against die X/Y, X and Y are the die X and Y (the offset tests applied)", ctl: () => [field("Wafer test", traced("w", inp("w", "none"))), field("X test", traced("x", inp("x", "X"))), field("Y test", traced("y", inp("y", "Y")))] },
    part_id: { title: "PRR part ID", note: "Often restarts per file: use within one lot only" },
    wafer_xy: { title: "PRR wafer + X/Y", note: "Needs X/Y in the PRR" },
    xy: { title: "PRR X/Y only", note: "Same die position, on any wafer" },
  };
  const idBox = h("div", { class: "id-order" });
  const idBadges = {};
  function paintIds() {
    const usable = idChoices().map(([id]) => id);
    for (const [id, cell] of Object.entries(idBadges)) cell.replaceChildren(!usable.includes(id) ? badge("not in use: test blank or trace off", "neutral") : id === usable[0] ? badge("default", "good") : "");
  }
  function drawIds() {
    const order = idOrderAll();
    idBox.replaceChildren(...order.map((id, i) => {
      const r = ID_ROWS[id];
      idBadges[id] = h("span");
      const move = (j) => { [order[i], order[j]] = [order[j], order[i]]; S.cfg.idOrder = order; S.mk = {}; saved(); drawIds(); };
      return h("div", { class: "id-row" },
        h("span", { class: "id-rank" }, String(i + 1)),
        h("div", { class: "id-what" }, h("b", {}, r.title), " ", idBadges[id], h("div", { class: "muted small" }, r.note)),
        h("div", { class: "id-tests" }, r.ctl ? r.ctl() : ""),
        h("span", { class: "row tight id-move" },
          h("button", { class: "ghost", title: "Move up", "aria-label": `Move ${r.title} up`, disabled: i === 0, onclick: () => move(i - 1) }, "↑"),
          h("button", { class: "ghost", title: "Move down", "aria-label": `Move ${r.title} down`, disabled: i === order.length - 1, onclick: () => move(i + 1) }, "↓")));
    }));
    paintIds();
  }
  const reset = () => {
    S.cfg = Object.assign({}, CFG_DEFAULT, { others: [], card: CFG_DEFAULT.card.slice(), idOrder: ID_ORDER.slice() }); S.wcal = Object.assign({}, WCAL_DEFAULT);
    S.optSort = S.cfg.sort; S.optDedup = ""; S.optMerge = false; S.mk = {};
    if (S.lbTest !== "5100") api("loadboard", { test: "5100" }).catch(() => {});
    api("label_formats", { wxy: "", test: "" }).catch(() => {});
    S.lbTest = "5100"; saveCfg(); go("settings");
  };
  el.append(
    sec("Opening files", "The order of opened files, retested parts and combining files are set on Home, next to where files are opened.", h("div", { class: "controls" }, field("Threads for loading files", h("span", { class: "row tight" }, threads, threadHint)), h("button", { class: "ghost", onclick: () => go("home") }, "Opening options on Home…"))),
    sec("Part identification", "How the same physical part is found across files, in this order. The first usable ID is the default on pages that match by one ID. On Trace, wafer, X, Y and part text link parts across files (wafer, X, Y decides when they disagree); the others are optional columns that never link parts. Leave a test blank if the program does not log it, or untick trace to leave it out of tracing only.", "How the same physical part is found across files, in this order. The first usable ID is the default on pages that match by one ID. The Part trace report uses part text, WXY and wafer, X, Y together: each one the files log is ticked, and a part is found by any of them. Leave a test blank if the program does not log it, or untick trace to leave it out of tracing only.",
      idBox,
      h("label", { class: "chk", style: { marginTop: "10px" }, title: "Identity tests differ by position or board only, so they would top marker rankings and first-fail picks for no physical reason" }, ignore, "Ignore the identification tests in analytics (rankings, signatures, first fails)")),
    sec("Labels", "How wafer, X, Y and a test are written on every page, in the Excel reports and in CSV files. Matching does not change: a part is still found by its values, and a search reads W,X,Y typed in this form.",
      h("div", { class: "controls" }, labelPick("wxyFmt", "Wafer, X, Y", WXY_FORMATS, (t) => wxyWith(t, 1, 118, 126), "{w} {x} {y}, or {x3} {y3} for 3 digits"),
        labelPick("testFmt", "Test number and name", TEST_FORMATS, (t) => testWith(t, 72199, "TESTNAME"), "{num} {name}"))),
    sec("Die coordinates from tests", "Offsets added to the X and Y tests rebuild die X/Y for Wafer maps, Bad candidates and the Part datalog. They change how parts are identified only when Check WXY against is die X/Y: then wafer, X, Y is the die position.",
      h("div", { class: "controls" }, field("X offset test", traced("ox", inp("ox", "none"))), field("Y offset test", traced("oy", inp("oy", "none"))), field("Offset", omode), flip("fx", "flip X"), flip("fy", "flip Y"))),
    sec("Other trace fields", "Shown in the part trace card and the parts search of Retest trace.",
      h("div", { class: "controls" }, field("Test program revision test", traced("rev", cfgIn("rev", "none"))), field("Loadboard test", traced("lb", lb))),
      field("Other IDs", othersBox)),
    sec("Experiments", "Off unless switched on. Experimental features are not validated on real tester files; they show only while switched on here.",
      h("div", { class: "controls" },
        h("label", { class: "chk" }, h("input", { type: "checkbox", checked: touchOn(), onchange: (e) => setTouch({ on: e.target.checked }) }), "Touchdown and loop per part"),
        field("Touchdown numbers start at", select([["1", "1"], ["0", "0"]], String(touchCfg().base), (v) => setTouch({ base: Number(v) }))),
        field("Count loops by", select([["auto", "whichever repeats more"], ["id", "part ID"], ["text", "part text"], ["off", "no loops"]], touchCfg().key, (v) => setTouch({ key: v })))),
      h("p", { class: "muted small" }, "Touchdown: the parts tested together. Estimated from the PIR records of each touchdown when the file has them, otherwise from the site order and the test times (the parts of one touchdown finish together). Loop: how often the same part ID or part text has come up before in the file, e.g. one DUT set tested again on every loop. Records lists both per part, and Edit & trim lists them and can select by touchdown.")),
    sec("Look", "Colour theme for the whole app; ◐ at the top switches light and dark. Each histogram also has its own bin-size slider above the chart.", h("div", { class: "controls" }, field("Theme", skin), field("Axis range", range), field("Histogram", hist))),
    h("div", { class: "row" }, h("button", { onclick: reset }, "Reset to defaults")));
  drawIds();
}
function viewExport(el) {
  const ds = S.datasets.find((d) => d.id === S.ds);
  const base = (ds ? ds.name : "export").replace(/\.(stdf|std)(\.gz)?$/i, "").replace(/\.gz$/i, "");
  const single = ds && ds.files === 1 && !ds.derived;
  const srcDir = ds && ds.paths.length ? dirname(ds.paths[0]) : "";
  const sep = srcDir.includes("\\") ? "\\" : "/";
  const near = (name) => (srcDir ? srcDir + sep + name : "");
  el.append(h("h1", {}, "Merge, split & export"), mergeCard(), h("p", { class: "lead" }, `Dataset: ${ds ? ds.name : "–"}`), scopeLine("The report, CSV, extract and split below write"));

  // report
  const [repPath, repField] = pathField("save", near(base + "_report.html"), { title: "Save HTML report", name: base + "_report.html", ext: "html" });
  const repOut = h("div");
  el.append(h("div", { class: "card pad section" }, h("h2", {}, "HTML report"),
    h("p", { class: "muted" }, "A single self-contained file (works offline) with all analyses. Share it by e-mail or on a drive."),
    h("div", { class: "row" }, repField, h("button", { class: "primary", onclick: async () => {
      if (!(await askPath(repPath))) return;
      busy("Building report…");
      try { const r = await api("save_report", P({ path: repPath.value })); repOut.replaceChildren(resultLine(`Saved ${r.path}`, r.path)); } catch (e) { toast(e.message, "bad"); } finally { busy(null); }
    } }, "Save report")), repOut));

  // CSV
  const [csvPath, csvField] = pathField("save", near(base + ".csv"), { title: "Save CSV", name: base + ".csv", ext: "csv" });
  const dk = (k) => `export:${S.ds}:${k}`;
  const csvTests = draft(dk("csvTests"), h("input", { type: "text", placeholder: "all tests, or e.g. 1000, 1010, VDD_LEAK", style: { minWidth: "260px" } }));
  const csvOut = h("div");
  el.append(h("div", { class: "card pad section" }, h("h2", {}, "CSV (parts × tests)"),
    h("p", { class: "muted" }, "One row per part with PRR fields and test values. Opens in Excel, JMP, Python…"),
    h("div", { class: "row" }, field("Tests", csvTests)),
    h("div", { class: "row" }, csvField, h("button", { class: "primary", onclick: async () => {
      if (!nativeDialogs() && !csvPath.value.trim()) { location.href = `/api/export.csv?ds=${S.ds}&filter=${encodeURIComponent(scoped(S.filter))}&tests=${encodeURIComponent(csvTests.value)}&t=${encodeURIComponent(window.STDF_TOKEN)}`; return; }
      if (!(await askPath(csvPath))) return;
      busy("Writing CSV…");
      try { const r = await api("export_csv_file", { ds: String(S.ds), filter: scoped(S.filter), tests: csvTests.value, path: csvPath.value }); csvOut.replaceChildren(resultLine(`Saved ${r.path}`, r.path)); } catch (e) { toast(e.message, "bad"); } finally { busy(null); }
    } }, "Export CSV")), csvOut));

  if (!single) {
    el.append(h("div", { class: "card pad section" }, h("h2", {}, "Split / extract STDF"), h("p", { class: "muted" }, ds && ds.derived ? "This dataset was created in the app, so it has no STDF file of its own. Split the file it came from instead (e.g. with its part set in “my own groups”)." : "Splitting and extracting work on datasets opened from a single file. Open the file on its own (without 'combine') to use them.")));
    return;
  }
  // extract
  const [exPath, exField] = pathField("save", near(base + "_selection.stdf"), { title: "Save selected parts as STDF", name: base + "_selection.stdf", ext: "stdf" });
  const exOut = h("div");
  el.append(h("div", { class: "card pad section" }, h("h2", {}, "Extract selected parts to a new STDF"),
    h("p", { class: "muted" }, S.filter ? `Parts matching: ${scoped(S.filter)}` : "Set a part filter at the top first (e.g. part_id in (12, 57) or hbin = 5).", " To pick parts one by one, by row or by quota instead, use ", h("a", { href: "#data/trim", onclick: (e) => { e.preventDefault(); go("trim"); } }, "Edit & trim"), "."),
    h("div", { class: "row" }, exField, (() => { const b = saveSplit("Extract", async (open) => {
      if (!(await askPath(exPath))) return;
      busy("Writing STDF…");
      try { const r = await api("export_stdf", P({ path: exPath.value })); exOut.replaceChildren(resultLine(`Wrote ${r.parts} parts to ${r.path}`, r.path)); if (open) loadPaths([r.path]); } catch (e) { toast(e.message, "bad"); } finally { busy(null); }
    }); b.querySelectorAll("button").forEach((x) => (x.disabled = !S.filter)); return b; })()), exOut));

  // split
  const by = select([["hbin", "hard bin"], ["sbin", "soft bin"], ["site", "site"], ["head", "head"], ["wafer", "wafer"], ["pass", "pass / fail"], ["part_id", "part ID (one file per part)"], ["part_txt", "part text (one file per PART_TXT)"], ["xy", "die X,Y (one file per die)"], ["test", "PTR result (one file per value of a test)…"], ["list", "a list of parts I paste…"], ["groups", "my own groups (filters)…"]], "hbin", () => renderGroups());
  draft(dk("by"), by);
  const splitTest = draft(dk("splitTest"), testInput("spt", "", "e.g. 501 (a logged wafer number)"));
  const listKey = draft(dk("listKey"), select([["part_id", "PRR part ID"], ["part_txt", "PRR part text"], ["xy", "PRR X/Y"], ["tests", "PTR test result(s)…"]], "part_id", () => { listTests.hidden = listKey.value !== "tests"; }));
  const listTests = draft(dk("listTests"), h("input", { type: "text", value: "500", placeholder: "e.g. 500 or 501,502,503", style: { width: "170px" }, hidden: listKey.value !== "tests" }));
  const listBox = draft(dk("listBox"), h("textarea", { rows: 5, class: "paths", placeholder: "one part per line (or comma separated)\n12\n57\n103" }));
  const listName = draft(dk("listName"), h("input", { type: "text", value: "listed", style: { width: "140px" } }));
  const [dirPath, dirField] = pathField("folder", near(base + "_split"), { title: "Choose output folder", placeholder: "output folder" });
  const groupsBox = h("div");
  // the groups are kept as they are edited, per dataset
  const groups = ((S.drafts ||= {})[dk("groups")] ||= [{ name: "group1", expr: "site in (1,2)" }, { name: "fails", expr: "not pass" }]);
  const rest = draft(dk("rest"), h("input", { type: "text", value: "rest", placeholder: "(leave empty to drop unmatched parts)" }));
  function renderGroups() {
    groupsBox.replaceChildren();
    if (by.value === "test") { groupsBox.append(h("div", { class: "row" }, field("Test", splitTest), field("Parts without a result go to", rest)), h("div", { class: "muted small" }, "One file per distinct result, e.g. split by a datalogged wafer number.")); return; }
    if (by.value === "list") { groupsBox.append(h("div", { class: "row" }, field("Parts are identified by", h("div", { class: "row tight" }, listKey, listTests)), field("Listed parts go to", listName), field("Other parts go to", rest)), listBox); return; }
    if (by.value !== "groups") return;
    for (const g of groups) {
      const n = h("input", { type: "text", value: g.name, oninput: (e) => (g.name = e.target.value), style: { width: "140px" } });
      const ex = h("input", { type: "text", value: g.expr, oninput: (e) => (g.expr = e.target.value), class: "filter-input", style: { flex: "1" } });
      groupsBox.append(h("div", { class: "row" }, n, ex, h("button", { class: "ghost x", onclick: () => { groups.splice(groups.indexOf(g), 1); renderGroups(); } }, "×")));
    }
    groupsBox.append(h("div", { class: "row" }, h("button", { onclick: () => { groups.push({ name: "group" + (groups.length + 1), expr: "" }); renderGroups(); } }, "+ Add group"), field("Unmatched parts go to", rest)), h("div", { class: "muted small" }, "First matching group wins. Same filter syntax as the top bar, including part sets from Match & trace (e.g. $match)."));
  }
  const spOut = h("div");
  el.append(h("div", { class: "card pad section" }, h("h2", {}, "Split into several STDF files"),
    h("p", { class: "muted" }, "Records are copied byte-for-byte; bin/part-count summaries are recalculated for each file." + (S.filter ? " Only parts matching the current filter are written." : "")),
    h("div", { class: "row" }, field("Split by", by)), groupsBox,
    h("div", { class: "row" }, dirField, h("button", { class: "primary", onclick: async () => {
      if (!(await askPath(dirPath))) return;
      busy("Splitting…");
      try {
        const r = await api("split", { ds: S.ds, filter: scoped(S.filter), by: by.value, groups, rest: rest.value, out_dir: dirPath.value, prefix: base, test: splitTest.value, list: listBox.value, list_key: listKey.value === "tests" ? "tests:" + listTests.value : listKey.value, list_name: listName.value });
        const t = h("div");
        spOut.replaceChildren(resultLine(`${r.outputs.length} file(s) written to ${r.out_dir}`, r.out_dir), t);
        table(t, [{ key: "name", label: "Output" }, { key: "parts", label: "Parts", num: true }, { key: "good", label: "Good", num: true }, { key: "path", label: "File" }], r.outputs, { search: r.outputs.length > 15 });
      } catch (e) { toast(e.message, "bad"); } finally { busy(null); }
    } }, "Split")), spOut));
  renderGroups();
}

// ---------------------------------------------------------------- boot
async function refreshDatasets(select_id) {
  S.datasets = await api("datasets");
  S.testsCache = {};
  const sel = document.getElementById("ds");
  sel.replaceChildren(...S.datasets.map((d) => h("option", { value: d.id }, `${d.name} (${d.n_parts.toLocaleString()} parts)`)));
  sel.disabled = !S.datasets.length;
  if (!S.datasets.length) sel.replaceChildren(h("option", {}, "no data open"));
  if (select_id !== undefined && S.datasets.some((d) => d.id === select_id)) setDs(select_id);
  else if (S.datasets.length) setDs(S.datasets.some((d) => d.id === S.ds) ? S.ds : S.datasets[S.datasets.length - 1].id);
  renderNav();
}
function setDs(id) {
  S.ds = +id;
  document.getElementById("ds").value = String(id);
  fillTestList();
}
function applyFilter() {
  const f = document.getElementById("filter");
  S.filter = f.value.trim();
  f.classList.toggle("active", !!S.filter);
  if (S.view === "home") go("overview"); else go(S.view);
}
document.getElementById("ds").addEventListener("change", (e) => { setDs(+e.target.value); S.explorerTest = null; go(S.view === "home" ? "overview" : S.view); });
document.getElementById("apply").addEventListener("click", applyFilter);
document.getElementById("clear").addEventListener("click", () => { document.getElementById("filter").value = ""; applyFilter(); });
document.getElementById("filter").addEventListener("keydown", (e) => { if (e.key === "Enter") applyFilter(); });
document.getElementById("theme").addEventListener("click", SC.toggleTheme);
{
  const sk = document.getElementById("skin");
  sk.value = SC.getSkin();
  sk.addEventListener("change", () => SC.setSkin(sk.value));
}
document.getElementById("rename").addEventListener("click", () => S.datasets.length && renameDs(S.ds));
{
  const sc = document.getElementById("scope");
  sc.value = S.scope;
  sc.classList.toggle("active", S.scope === "good");
  sc.addEventListener("change", () => { S.scope = sc.value; sc.classList.toggle("active", S.scope === "good"); try { localStorage.setItem("sc-scope", S.scope); } catch (e) {} S.testsCache = {}; go(S.view === "home" ? "overview" : S.view); });
}
document.getElementById("open").addEventListener("click", openFiles);
document.getElementById("homebtn").addEventListener("click", () => go("home"));
window.addEventListener("hashchange", () => { const v = location.hash.slice(1), w = viewOf(v); if (w && (w[0] !== S.view || (w[1] && w[1] !== (S.tabs || {})[w[0]]))) go(v); });
window.addEventListener("keydown", (e) => { if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "o") { e.preventDefault(); openFiles(); } });
// live connection: keeps the desktop app running while this window is open,
// and receives files opened from Explorer while the app is running
function connectEvents() {
  try {
    const es = new EventSource("/api/events?t=" + encodeURIComponent(window.STDF_TOKEN));
    es.addEventListener("open", (e) => { try { const p = JSON.parse(e.data); if (p.length) loadPaths(p); } catch (x) {} });
    es.onopen = () => document.body.classList.remove("offline");
    es.onerror = () => { if (es.readyState === EventSource.CLOSED) document.body.classList.add("offline"); };
  } catch (e) {}
}
async function heartbeat() {
  try {
    const r = await api("ping");
    if (r.pending && r.pending.length) await loadPaths(r.pending);
    document.body.classList.remove("offline");
  } catch (e) { document.body.classList.add("offline"); }
}
SC.initTheme();
(async () => {
  const v = (location.hash || "").slice(1);
  try { S.info = await api("info"); } catch (e) { S.info = {}; }
  if (S.info.version) document.getElementById("ver").textContent = "v" + S.info.version;
  // a saved loadboard test other than the default is set again on the server
  if (S.lbTest && S.lbTest !== "5100") api("loadboard", { test: S.lbTest }).catch(() => {});
  // the labels the server writes in reports, as set in Settings
  if (S.cfg.wxyFmt || S.cfg.testFmt) api("label_formats", { wxy: S.cfg.wxyFmt || "", test: S.cfg.testFmt || "" }).catch(() => {});
  try { await refreshDatasets(); } catch (e) { main.append(h("div", { class: "err" }, String(e))); }
  const w = viewOf(v);
  go(w && (S.datasets.length || VIEWS[w[0]].always) ? v : S.datasets.length ? "overview" : "home");
  connectEvents();
  await heartbeat();
  setInterval(heartbeat, 5000);
})();
