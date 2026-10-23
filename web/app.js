// stdf-cypher interactive dashboard
"use strict";
const { h, plot, fmt, fmtP, pct, kpi, chartBox, badge, table } = SC;

const S = { datasets: [], ds: 0, filter: "", view: "overview", testsCache: {}, scope: "all", co: { range: "both", dim: "2d", opacity: 0.6, hist: "pct" } };
try { Object.assign(S.co, JSON.parse(localStorage.getItem("sc-chart") || "{}")); S.scope = localStorage.getItem("sc-scope") || "all"; } catch (e) {}
// settings kept in this browser profile (Settings page); `sc-sort` predates it
const CFG_DEFAULT = { sort: "time", dedup: "", merge: false, key: "part_txt", lbTest: "5100", ignoreIds: true, wxy: "500", rev: "", others: [], card: ["part_txt", "wxy", "wxy_t"] };
const WCAL_DEFAULT = { w: "80100", x: "80101", y: "80102", ox: "80103", oy: "80104", mode: "add", fx: false, fy: false };
S.cfg = Object.assign({}, CFG_DEFAULT);
try {
  const old = localStorage.getItem("sc-sort");
  if (old) S.cfg.sort = old;
  Object.assign(S.cfg, JSON.parse(localStorage.getItem("sc-settings") || "{}"));
  const w = JSON.parse(localStorage.getItem("sc-wcal") || "null");
  if (w) S.wcal = Object.assign({}, WCAL_DEFAULT, w);
} catch (e) {}
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
/** The identity / trace tests (Settings > Part identification and trace). */
function idTests() {
  const w = waferCal();
  return { w: w.w, x_t: w.x, y_t: w.y, ox: w.ox, oy: w.oy, wxy: S.cfg.wxy, rev: S.cfg.rev, lb: S.lbTest || "", ...Object.fromEntries((S.cfg.others || []).map((o, i) => [`o${i}`, o.test])) };
}
/** Tests left out of rankings and first-fail picks: the identity tests, unless Settings keeps them in. */
function idExclude() {
  if (S.cfg.ignoreIds === false) return "";
  return Object.values(idTests()).map((v) => String(v || "").trim()).filter((v) => /^\d+(\s*-\s*\d+)?$/.test(v)).join(", ");
}
/** Part trace fields a part can show (mini card, search): [key, label]. */
function traceFields() {
  const w = waferCal();
  return [["part_txt", "Part text"], ["part_id", "Part ID"], ["prr_xy", "PRR X/Y"], ["wafer", "Wafer (WIR)"],
    ...(S.cfg.wxy ? [["wxy", "WXY"]] : []), ...(w.w || w.x || w.y ? [["wxy_t", "Wafer, X, Y"]] : []),
    ...(S.cfg.rev ? [["rev", "Program rev"]] : []), ...(S.lbTest ? [["lb", "Loadboard"]] : []),
    ...(S.cfg.others || []).filter((o) => o.test).map((o, i) => [`o${i}`, o.name || `ID ${o.test}`])];
}
/** A part's trace field values as text, from the PRR fields and trace tests in `ids`. */
function traceValues(ids) {
  if (!ids) return {};
  const w = waferCal(), num = (v) => (v == null ? "" : Number.isInteger(v) ? String(v) : fmt(v, 6));
  const die = (v, o) => (v == null ? null : o == null ? v : w.mode === "sub" ? v - o : v + o);
  const dx = die(ids.x_t, ids.ox), dy = die(ids.y_t, ids.oy);
  const out = { part_txt: ids.part_txt || "", part_id: ids.part_id || "", prr_xy: ids.x != null && ids.x !== NO_XY ? `${ids.x}, ${ids.y}` : "", wafer: ids.wafer || "",
    wxy: num(ids.wxy), wxy_t: [ids.w, dx, dy].some((v) => v != null) ? [ids.w, dx, dy].map(num).join("|") : "", rev: num(ids.rev), lb: num(ids.lb) };
  for (const k of Object.keys(ids)) if (/^o\d+$/.test(k)) out[k] = num(ids[k]);
  return out;
}
const main = document.getElementById("main");
const GROUPS = [["", "none"], ["pass", "good / bad"], ["alarm", "alarm"], ["site", "test site"], ["head", "head"], ["hbin", "hard bin"], ["sbin", "soft bin"], ["wafer", "wafer"], ["file", "file / lot"], ["loadboard", "loadboard"], ["site_loadboard", "site × loadboard"]];
/** Loadboard test number input (a session setting: the `loadboard` field reads this test's most common value per file). */
function loadboardField() {
  const i = h("input", { type: "text", value: S.lbTest || "5100", style: { width: "80px" }, title: "Test number holding the loadboard (DIB) ID. Read per file (its most common value), so parts missing it still get the file's board. Usable as the field `loadboard` in filters, color-by, ANOVA and commonality, and `site_loadboard` for site × board.",
    onchange: async (e) => { const v = e.target.value.trim(); try { const r = await api("loadboard", { test: v }); S.lbTest = String(r.test); saveCfg(); S.testsCache = {}; toast(`Loadboard = test ${r.test}`, "good"); } catch (err) { toast(err.message, "bad"); } } });
  return field("Loadboard test", i);
}

async function api(op, params) {
  const r = await fetch("/api/" + op, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(params || {}) });
  const j = await r.json();
  if (!j.ok) throw new Error(j.error || "request failed");
  return j.data;
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
  match: { label: "Match, merge & trace", fn: viewMatch },
  investigation: { label: "Failure investigation", fn: viewInvestigation },
  candidates: { label: "Bad candidates", fn: viewCandidates },
  signature: { label: "Signature check", fn: viewSignature },
  retest: { label: "Retest trace", fn: viewRetest },
  correlation: { label: "Correlation", fn: viewCorrelation },
  anova: { label: "ANOVA", fn: viewAnova },
  anomaly: { label: "Abnormal parts", fn: viewAnomaly },
  ml: { label: "Machine learning", fn: viewML },
  records: { label: "Records", fn: viewRecords },
  datalog: { label: "Part datalog", fn: viewDatalog },
  export: { label: "Merge, split & export", fn: viewExport },
  settings: { label: "Settings", fn: viewSettings, always: true },
};

// a heading above the first view of each group ("" = a plain separator)
const NAV_GROUPS = { overview: "Explore", match: "Investigate", correlation: "Statistics", records: "Data & files", settings: "" };
function renderNav() {
  const nav = document.getElementById("nav");
  nav.replaceChildren();
  for (const [k, v] of Object.entries(VIEWS)) {
    if (NAV_GROUPS[k] !== undefined) nav.append(NAV_GROUPS[k] ? h("div", { class: "nav-group" }, NAV_GROUPS[k]) : h("div", { class: "sep" }));
    const disabled = !v.always && !S.datasets.length;
    nav.append(h("button", { class: (S.view === k ? "active" : "") + (disabled ? " dim" : ""), onclick: () => go(k), title: disabled ? "Open an STDF file first" : "" }, v.label));
  }
  nav.append(h("div", { class: "hint" }, S.datasets.length ? "The part filter and population at the top apply to every view." : "Open an STDF file to unlock the analyses."));
}
function go(view) {
  if (view === "parts") view = "records";
  S.view = view;
  try { history.replaceState(null, "", "#" + view); } catch (e) {}
  renderNav();
  main.replaceChildren();
  if (!S.datasets.length && !VIEWS[view].always) { S.view = "home"; renderNav(); return viewHome(main); }
  VIEWS[view].fn(main);
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
    const d = await api("overview", P());
    const o = d.overview;
    body.replaceChildren();
    body.append(h("div", { class: "kpis" },
      kpi("Parts", o.n_parts.toLocaleString(), S.filter ? `${d.selected.toLocaleString()} selected` : `${o.files.length} file(s)`),
      kpi("Yield", pct(S.filter ? d.selected_yield : o.yield_pct), `${o.n_pass.toLocaleString()} pass`),
      kpi("Tests", o.n_tests.toLocaleString()),
      kpi("Wafers", o.n_wafers || "–"),
      kpi("Sites", o.sites),
      kpi("Retests", o.retests.toLocaleString()),
      kpi("Lot", o.lots.join(", ") || "–", o.part_typ),
      kpi("Program", o.job || "–", o.tester)));
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
    table(src, [{ key: "path", label: "File" }, { key: "lot_id", label: "Lot" }, { key: "sublot_id", label: "Sublot" }, { key: "part_typ", label: "Product" }, { key: "job_nam", label: "Program" }, { key: "node_nam", label: "Node" }, { key: "test_cod", label: "Step" }, { key: "tst_temp", label: "Temp" }, { key: "endian", label: "Endian" }], d.sources, { search: false });
  } catch (e) { showErr(body, e); }
}

// ---- Test explorer
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
      { key: "cpk", label: "Cpk", num: true, fmt: (v) => fmt(v, 2) },
      { key: "fail_pct", label: "Fail %", num: true, fmt: (v) => (v ? v.toFixed(2) : "0") },
    ], tests, { onClick: (r) => { S.explorerTest = r.key; draw(); }, sort: "fail_pct", maxHeight: "760px", placeholder: "Search tests…" });
    if (!S.explorerTest) {
      const first = [...tests].filter((t) => t.kind !== "Functional").sort((a, b) => (b.fail_pct || 0) - (a.fail_pct || 0))[0];
      S.explorerTest = first && first.key;
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
  el.append(h("h1", {}, "Wafer maps"), h("p", { class: "lead" }, "Bin, pass/fail or test-value maps per wafer. Die coordinates come from the PRR X/Y (wafers from WIR), or are rebuilt from datalogged test results, e.g. wafer, X, Y and an X/Y offset logged in tests 80100–80104 (die X = X + X offset). If the map still looks off, try subtracting the offset or flipping an axis."));
  if (!S.wcoords) S.wcoords = ds && ds.wafers.length ? "prr" : "tests";
  const coords = select([["prr", "PRR X/Y + WIR wafer"], ["tests", "from test results (PTR)"]], S.wcoords, (v) => { S.wcoords = v; S.wafer = "all"; renderCtl(); draw(); });
  const mode = select([["hbin", "hard bin"], ["sbin", "soft bin"], ["pass", "pass / fail"], ["test", "test value"]], S.wmode || "hbin", (v) => { S.wmode = v; draw(); });
  const test = testInput("wt", S.wtest || S.explorerTest || "");
  test.addEventListener("change", () => { S.wtest = test.value; if ((S.wmode || "hbin") === "test") draw(); });
  const small = (inp) => { inp.style.minWidth = "120px"; inp.style.width = "130px"; return inp; };
  const wc = waferCal();
  const wt = small(testInput("wwt", wc.w, "none = one map"));
  const xt = small(testInput("wxt", wc.x, "X test"));
  const yt = small(testInput("wyt", wc.y, "Y test"));
  const oxt = small(testInput("woxt", wc.ox, "none"));
  const oyt = small(testInput("woyt", wc.oy, "none"));
  const omode = select([["add", "X + offset"], ["sub", "X − offset"]], wc.mode, () => cal());
  const fx = h("input", { type: "checkbox", checked: wc.fx, onchange: () => cal() });
  const fy = h("input", { type: "checkbox", checked: wc.fy, onchange: () => cal() });
  function cal() { Object.assign(wc, { w: wt.value, x: xt.value, y: yt.value, ox: oxt.value, oy: oyt.value, mode: omode.value, fx: fx.checked, fy: fy.checked }); saveCfg(); S.wafer = "all"; draw(); }
  for (const i of [wt, xt, yt, oxt, oyt]) i.addEventListener("change", cal);
  const wafer = select([["all", "all wafers (gallery)"]], "all", (v) => { S.wafer = v; draw(); });
  const ctl = h("div", { class: "controls" });
  function renderCtl() {
    ctl.replaceChildren(field("Coordinates", coords), ...(S.wcoords === "tests" ? [field("Wafer test", wt), field("X test", xt), field("Y test", yt), field("X offset test", oxt), field("Y offset test", oyt), field("Offset", omode), h("label", { class: "chk" }, fx, "flip X"), h("label", { class: "chk" }, fy, "flip Y")] : []), field("Color by", mode), field("Test (for test value)", test), field("Wafer", wafer));
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
    const later = select([["", "none"], ...dsOptions().filter(([v]) => +v !== S.ds)], S.cmpLater ?? "", (v) => { S.cmpLater = v; keys.hidden = !v; });
    const ka = keyPicker("a"), kb = keyPicker("b");
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
  el.append(h("p", { class: "lead" }, "The same physical parts tested in two files (e.g. golden units on two testers, or a re-test) are matched by part ID or wafer X/Y. Each test is CORRELATED, OFFSET (good r but mean offset beyond tolerance) or UNCORRELATED."));
  const a = select(dsOptions(), String(S.ucA ?? S.ds), (v) => (S.ucA = +v));
  const dA = S.datasets.find((d) => d.id === (S.ucA ?? S.ds));
  const twin = S.datasets.find((d) => d.id !== dA.id && d.files === 1 && dA.wafers.length && d.wafers.some((w) => dA.wafers.includes(w)));
  const b = select(dsOptions(), String(S.ucB ?? (twin ? twin.id : otherDs(dA.id))), (v) => (S.ucB = +v));
  const mb = select([["auto", "auto"], ["part_id", "part ID"], ["xy", "wafer + X/Y"]], "auto");
  const rmin = h("input", { type: "number", step: "0.01", value: 0.9 });
  const off = h("input", { type: "number", step: "0.5", value: 5 });
  const out = h("div");
  el.append(h("div", { class: "controls" }, field("File A", a), field("File B", b), field("Match by", mb), field("min r", rmin), field("max offset % limit", off), h("button", { class: "primary", onclick: run }, "Correlate")), out);
  async function run() {
    loading(out);
    const params = { a: +a.value, b: +b.value, match_by: mb.value, r_min: +rmin.value, offset_pct: +off.value, filter_a: scoped(""), filter_b: scoped("") };
    try {
      const d = await api("unitcorr", params);
      const r = d.report;
      out.replaceChildren(h("div", { class: "kpis" }, kpi("Matched parts", r.matched.toLocaleString(), `by ${r.match_by}`), kpi("Only in A / B", `${r.only_a} / ${r.only_b}`), kpi("Bin agreement", pct(r.bin_agreement_pct, 1)),
        ...Object.entries(r.summary).filter(([, v]) => v > 0).map(([k, v]) => kpi(k.replace("_", " ").toLowerCase(), v))));
      if (!r.matched) { out.append(h("div", { class: "err" }, "No parts matched. Unit correlation needs the same physical parts in both files: identical part IDs (final test) or identical wafer IDs + X/Y (wafer sort). Try another 'Match by', or pick files from the same lot.")); return; }
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
  el.append(h("h1", {}, "Records"), h("p", { class: "lead" }, "Every STDF record of the current dataset. The part table summarises each PRR with its first failing test; the record viewer shows any record type field by field, straight from the file."));
  const tabs = h("div", { class: "tabs" });
  const body = h("div");
  el.append(tabs, body);
  const modes = [["parts", "Part table (PRR)"], ["raw", "Record viewer (all types)"]];
  const setMode = (m) => { S.recMode = m; tabs.replaceChildren(...modes.map(([k, l]) => h("button", { class: k === m ? "active" : "", onclick: () => setMode(k) }, l))); body.replaceChildren(); (m === "raw" ? recRaw : recParts)(body); };
  setMode(S.recMode || "parts");
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
    h("button", { onclick: () => { location.href = `/api/export.csv?ds=${S.ds}&filter=${encodeURIComponent(scoped(S.filter))}`; } }, "Download CSV (all tests)"),
    h("button", { onclick: () => go("export") }, "Split, extract or save report…")));
  const out = h("div");
  el.append(out);
  loading(out);
  try {
    const d = await api("parts", P({ limit: 20000 }));
    const detail = h("div");
    const tb = h("div");
    out.replaceChildren(h("p", { class: "muted" }, `${d.total.toLocaleString()} parts` + (d.total > d.rows.length ? ` (first ${d.rows.length} listed)` : "")), tb, detail);
    const hasTxt = d.rows.some((r) => r.part_txt);
    table(tb, [{ key: "index", label: "#", num: true }, { key: "part_id", label: "Part ID" }, hasTxt ? { key: "part_txt", label: "Part text" } : null, { key: "wafer", label: "Wafer" }, { key: "x", label: "X", num: true }, { key: "y", label: "Y", num: true }, { key: "head", label: "Head", num: true }, { key: "site", label: "Site", num: true }, { key: "hbin", label: "HBin", num: true }, { key: "sbin", label: "SBin", num: true }, { key: "pass", label: "Result", render: (v) => (v === null ? badge("n/a", "neutral") : v ? badge("good", "good") : badge("bad", "bad")) }, { key: "alarm", label: "Alarm", render: (v) => (v ? badge("alarm", "warn") : "") }, { key: "first_fail", label: "First fail" }, { key: "test_t", label: "Test time ms", num: true }].filter(Boolean), d.rows, {
      onClick: (row) => partDetail(detail, S.ds, row.index),
      onClear: () => detail.replaceChildren(),
    });
  } catch (e) { showErr(out, e); }
}

const TEST_RECS = ["PTR", "MPR", "FTR", "TSR"];
function recRaw(el) {
  const ds = S.datasets.find((d) => d.id === S.ds);
  if (!S.rec || S.rec.ds !== S.ds) S.rec = { ds: S.ds, file: 0, type: "PRR", test: "", offset: 0, limit: 200 };
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
  ctl.append(ds && ds.paths.length > 1 ? field("File", fileSel) : "", field("Record type", typeSel), testField, field("Rows per page", size));
  async function load() {
    loading(out, "Reading records…");
    testField.hidden = !TEST_RECS.includes(st.type);
    try {
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
const KEYS = [["part_txt", "PRR part text (PART_TXT)"], ["part_id", "PRR part ID"], ["wafer_xy", "PRR wafer + X/Y"], ["xy", "PRR X/Y only"], ["tests", "PTR test result(s)…"]];
function keyPicker(side, ds) {
  S.mk = S.mk || {};
  const cur = S.mk[side] || S.cfg.key;
  const isTests = !KEYS.some(([k]) => k === cur);
  const sel = select(KEYS, isTests ? "tests" : cur, () => sync());
  const tests = h("input", { type: "text", value: isTests ? cur.replace(/^tests:/, "") : "500", placeholder: "e.g. 500  or  501,502,503", title: "One test with a unique per-part value, or several tests whose values together identify the part (e.g. wafer, X, Y)", style: { width: "190px" }, oninput: () => sync() });
  const value = () => (sel.value === "tests" ? "tests:" + tests.value.trim() : sel.value);
  function sync() { tests.hidden = sel.value !== "tests"; S.mk[side] = value(); }
  sync();
  return { el: h("div", { class: "row tight" }, sel, tests), value };
}
function viewMatch(el) {
  el.append(h("h1", {}, "Match, merge & trace"), mergeCard(), h("p", { class: "lead" }, "Find the same physical parts in two files and follow them across: by PRR part text or part ID, by die X/Y, or by datalogged PTR results (one test holding a unique wafer + X/Y code, or wafer, X and Y in separate tests). Matched parts become a part set you can filter on, compare, split out and chart side by side."));
  const ids = S.datasets.map((d) => d.id);
  const st = S.m || (S.m = { a: S.ds, b: otherDs(S.ds), name: "match", fa: "", fb: "" });
  if (!ids.includes(st.a)) st.a = S.ds;
  if (!ids.includes(st.b) || st.b === st.a) st.b = otherDs(st.a);
  const two = S.datasets.length > 1;
  const a = select(dsOptions(), String(st.a), (v) => (st.a = +v));
  const b = select(dsOptions(), String(st.b), (v) => (st.b = +v));
  const ka = keyPicker("a"), kb = keyPicker("b");
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
  if (!two) el.append(h("div", { class: "help" }, "Open a second STDF file (without 'combine') to match parts between files. You can still trace a part by its identity in this file below."));
  el.append(h("div", { class: "controls" }, field("File A", a), field("Identify A parts by", ka.el), field("Filter A", fa), quickFilter(fa, "qa")),
    two ? h("div", { class: "controls" }, field("File B", b), field("Identify B parts by", kb.el), field("Filter B", fb), quickFilter(fb, "qb"), field("Part set name", name), h("button", { class: "primary", onclick: run }, "Match parts")) : "",
    two ? h("div", { class: "help" }, "Example – parts that were bin 1 in file A but failed test 1013 in file B: Filter A ", h("code", {}, "hbin = 1"), ", Filter B ", h("code", {}, "fail(1013)"), " (or ", h("code", {}, "first_fail = 1013"), ", or ", h("code", {}, "fail(1013) and n_fails = 1"), " for that test only). Then use “Find marker tests” on file A.") : "");
  // trace a single unit by its identity
  const tv = h("input", { type: "text", placeholder: "the A identity value, e.g. LOTA-W01-X3Y-2  or  11253  or  1|3|-2", style: { minWidth: "300px", flex: "1" }, onkeydown: (e) => { if (e.key === "Enter") doTrace(); } });
  const doTrace = () => { sync(); trace(traceBox, { a: st.a, b: two && st.a !== st.b ? st.b : null, key_a: st.ka, key_b: st.kb, value: tv.value }); };
  el.append(h("div", { class: "controls" }, field("Trace a part", tv), h("button", { onclick: doTrace }, "Trace"), h("span", { class: "muted small", style: { paddingBottom: "8px" } }, "Several test values: separate with | in the order of the tests.")), out, traceBox);
  if (S.mres && S.mres.a === st.a && S.mres.b === st.b && ids.includes(st.a) && ids.includes(st.b)) renderMatch(out, S.mres, traceBox);
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
    const [dirP, dirF] = pathField("folder", nearDs(d, `${base}_${m.name}_split`), { title: "Choose output folder", placeholder: "output folder" });
    const res = h("div");
    const single = d && d.files === 1 && !d.derived;
    const within = h("input", { type: "checkbox", checked: !!sideFilter });
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
      single ? h("div", { class: "row" }, dirF) : null,
      single && sideFilter ? h("label", { class: "chk" }, within, ` write only parts within Filter ${label}`) : null,
      single ? h("div", { class: "row" }, h("button", { class: "primary", onclick: async () => {
        if (!dirP.value) return toast("Choose an output folder", "bad");
        busy("Splitting…");
        try {
          const r = await api("split", { ds: id, by: "groups", groups: [{ name: "matched", expr: set }], rest: "unmatched", out_dir: dirP.value, prefix: base, filter: within.checked ? sideFilter : "" });
          const t = h("div");
          res.replaceChildren(resultLine(`${r.outputs.length} file(s) written to ${r.out_dir}`, r.out_dir), t);
          table(t, [{ key: "name", label: "Output" }, { key: "parts", label: "Parts", num: true }, { key: "good", label: "Good", num: true }, { key: "path", label: "File" }], r.outputs, { search: false });
        } catch (e) { toast(e.message, "bad"); } finally { busy(null); }
      } }, "Split: matched / unmatched STDF")) : h("p", { class: "muted small" }, "Splitting needs a dataset opened from a single file."),
      res);
  };
  out.append(h("div", { class: "grid2" }, sideCard(m.a, "A"), sideCard(m.b, "B")), cmpOut);

  // dual view
  const ta = testInput("dta", S.dualA || S.explorerTest || "", "test in A", m.a);
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
  out.append(h("h2", {}, "Dual view: the same units in both files"),
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
  const a = select(dsOptions(), String(st.a), (v) => (st.a = +v));
  const b = select(dsOptions(), String(st.b), (v) => (st.b = +v));
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
  const ka = keyPicker("ca"), kb = keyPicker("cb");
  const name = h("input", { type: "text", value: st.name, style: { width: "130px" }, title: "Part set name, usable as $name in every filter" });
  const coords = select([["auto", "auto (PRR X/Y, else tests)"], ["tests", "from tests (Wafer maps settings)"]], st.coords);
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
    const goodF = scoped(st.good), set = "$" + st.name;
    loading(out, "Matching file 2's bad parts to file 1's good parts…");
    try {
      const m = await api("candidate_sets", { a: st.a, b: st.b, key_a: ka.value(), key_b: kb.value(), filter_a: goodF, filter_b: badFilter(), test: st.test.replace(/:.*/, ""), range: st.range, include_marginal: st.incMarg, name: st.name });
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
  el.append(
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
  return S.inv || (S.inv = { name: "case 1", test: "", loadboard: S.lbTest || "5100", key: "part_txt", good: "pass",
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
  const keySel = select(KEYS.filter(([k]) => k !== "tests"), c.key || "part_txt", (v) => (c.key = v));
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
  el.append(h("h1", {}, "Retest trace"), h("p", { class: "lead" }, "Follow parts through a chain of runs: run 1, then a retest of its rejects, then a retest of those rejects, and so on; a run may also retest good parts (e.g. the good parts of runs 1 and 2), and their rejects are followed on. Pick the starting parts in run 1 (e.g. the ones failing test 1013); each later run is searched for the same physical part, so you see per run whether it passed or failed, its bin and the traced test's value, and how the population shrinks."));
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
  const kp = keyPicker("rt");
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
      const d = await api("retest_trace", { runs: st.runs, key: kp.value(), filter: scoped(filter), tests: st.tests, exclude: idExclude(), id_tests: idTests(), value_from: st.valueFrom || "first", max_rate: (st.maxRate ?? 10) / 100, min_zone: st.minZone ?? 10 });
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
        h("div", { class: "controls" }, field("Identify parts by", kp.el)),
        h("p", { class: "muted small" }, "Part text (or wafer + X/Y) is safest: part IDs often restart at 1 in every run."),
        h("div", { class: "controls" }, field("Start with", h("div", { class: "row tight" }, mode, custom))),
        h("div", { class: "controls" }, field("Trace test(s)", h("div", { class: "row tight" }, tests, unitHint)), loadboardField(), h("button", { class: "primary", onclick: run }, "Trace")))),
    out, traceBox);
  if (S.rtLast) renderRetest(out, traceBox, S.rtLast);
}
/** Parts table search: any trace value, and wafer X Y typed as 3|12|-4 (or with commas, spaces or slashes). */
function rtSearch(r, q) {
  if (r._hay.includes(q)) return true;
  const m = /^\s*(-?\d+)[\s,;/|]+(-?\d+)[\s,;/|]+(-?\d+)\s*$/.exec(q);
  return !!m && r._trace.wxy_t === `${m[1]}|${m[2]}|${m[3]}`;
}
/** The part trace card: the trace fields picked here (kept in Settings), for one part. */
function partCard(p, redraw) {
  if (!p) return null;
  const tv = traceValues(p.ids), all = traceFields();
  const pick = (S.cfg.card || []).filter((k) => all.some(([x]) => x === k));
  const chips = all.filter(([k]) => pick.includes(k)).map(([k, l]) => h("span", { class: "trace-chip" }, h("span", { class: "muted small" }, l), h("b", {}, tv[k] || "–")));
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
    kpi("Matched by", d.key_desc, "")),
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
  out.append(h("h2", {}, "Flow and trajectories"), g);
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
  const focusBar = () => h("div", { style: { padding: "8px 12px 0" } }, h("div", { class: "row tight", style: { justifyContent: "space-between" } },
    S.rtHi ? h("span", { class: "row tight" }, h("span", { class: "muted small" }, "Highlighted:"), h("b", {}, S.rtHi), h("button", { class: "ghost small", title: "Clear the highlight", onclick: () => focusPart(null) }, "× clear"), h("button", { class: "small", title: "All results of this part in run 1 and the last run it was found in", onclick: tracePart }, "Trace this part"))
      : h("span", { class: "muted small" }, "Click a line or point to highlight that part and narrow the parts table to it (click it again to clear). Drag to zoom, double-click to zoom out."),
    h("button", { class: "small", title: "Clear the highlighted part, the parts table search and every chart zoom", onclick: resetView }, "↺ Reset view")),
    S.rtHi ? partCard(d.parts.find((x) => x.key === S.rtHi), () => drawTraj()) : null);
  const onPick = (el) => el.on && el.on("plotly_click", (ev) => { const k = ev.points && ev.points[0] && ev.points[0].customdata; if (typeof k === "string") focusPart(k, "chart"); });
  let ti = Math.min(S.rtTest || 0, d.tests.length - 1);
  const clip = (s, n) => (s.length > n ? s.slice(0, n - 1) + "…" : s);
  const jit = (key) => { let x = 0; for (const c of key) x = (x * 31 + c.charCodeAt(0)) | 0; return ((x >>> 0) % 1000) / 1000 * 0.24 - 0.12; };
  const drawTraj = () => {
    const t = d.tests[ti];
    const sel = d.tests.length > 1 ? select(d.tests.map((x, i) => [String(i), x.key]), String(ti), (v) => { ti = S.rtTest = +v; drawTraj(); drawRec(); drawDelta(); }) : null;
    const [card, c] = chartBox("tall");
    trajBox.replaceChildren(focusBar(), ...(sel ? [h("div", { class: "controls", style: { padding: "8px 12px 0" } }, field("Test", sel))] : []), card.firstChild);
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
    const o2 = { key: p.key, outcome: p.outcome, _trace: tv, _hay: Object.values(tv).filter(Boolean).join(" ").toLowerCase() };
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
  const popF = c.n_marginal ? "pass and not $rt_marginal" : "pass";
  const box = h("div"), mkBox = h("div");
  out.append(h("h2", {}, "Bad candidates from the retests of good parts"),
    h("p", { class: "lead" }, `${c.good_runs.map(rl).join(", ")} retested good parts. Parts good in run 1 that failed ${t0.key} there and never passed again are the bad candidates ($rt_bad_candidate in run 1); those that passed again later are marginal ($rt_marginal, left out of the comparison); those passing every retest of good parts are confirmed good ($rt_confirmed_good). Find candidate tests ranks every run-1 test by how well it sets the candidates apart from the other good parts, as on the Bad candidates page.`), box, mkBox);
  box.append(h("div", { class: "kpis" },
    kpi("Bad candidates", c.n_cand.toLocaleString(), `good in run 1, failed ${t0.key} on a retest of good parts, never passed after`),
    kpi("Marginal", c.n_marginal.toLocaleString(), "failed it there, passed again later"),
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
    h("button", { title: "Profile run 1 against the first retest of good parts on the Bad candidates page (outlier scores, sites, wafer zones, focus tests)", onclick: () => {
      S.bc = Object.assign(S.bc || {}, { a, b: later, good: "pass", test: String(t0.num), how: "fail", range: "", rangeNote: `from Retest trace: ${rl(c.good_runs[0])} retested good parts`, incMarg: false, extra: "", name: "bad_candidate", coords: "auto" });
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
    h("div", { class: "dz-actions" }, browse, folder, h("button", { onclick: (e) => { e.stopPropagation(); openSamples(); } }, "Try sample data")));
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

  right.append(h("h2", {}, "What can I do?"));
  const tips = [
    ["overview", "Overview", "Yield, bin pareto, site & wafer yield, fail pareto"],
    ["explorer", "Test explorer", "Histogram, box, CDF and trend of any test vs limits"],
    ["wafer", "Wafer maps", "Bin or test-value maps per wafer"],
    ["compare", "Compare", "Lot vs lot, or a group of parts vs the rest"],
    ["match", "Match, merge & trace", "Same parts in two files; merge pieces of one lot"],
    ["investigation", "Failure investigation", "One case: recovery window, bad candidates, signature, screening"],
    ["candidates", "Bad candidates", "Good parts that went bad, and which tests set them apart"],
    ["signature", "Signature check", "Learn rules from bad candidates, check them on good parts"],
    ["retest", "Retest trace", "Follow parts through up to 8 runs, by site and value"],
    ["correlation", "Correlation", "Tester-to-tester correlation, test-to-test matrix"],
    ["anova", "ANOVA", "Which tests differ by site, wafer or lot"],
    ["anomaly", "Abnormal parts", "PAT, robust z, wafer NNR, Isolation Forest"],
    ["ml", "Machine learning", "What drives fails? PCA & clustering"],
    ["records", "Records", "Part table and every raw STDF record"],
    ["datalog", "Part datalog", "Chosen tests of chosen parts as text or CSV"],
    ["export", "Merge, split & export", "Merge or split STDF, CSV, HTML report"],
  ];
  right.append(h("div", { class: "tips" }, tips.map(([k, t, d]) => h("button", { class: "tip", disabled: !has, onclick: () => go(k) }, h("b", {}, t), h("span", {}, d)))));
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
async function openSamples() {
  busy("Creating sample data…");
  try {
    const r = await api("sample");
    S.optMerge = false;
    await loadPaths(r.paths, { quiet: true });
    toast("Loaded 3 demo files: lot A, a shifted lot B, and lot A re-tested on a 2nd tester. Try Compare and Correlation!", "good", 8000);
  } catch (e) { toast(e.message, "bad"); } finally { busy(null); }
}
async function loadPaths(paths, opts) {
  opts = opts || {};
  if (!paths.length) return;
  busy(`Loading ${paths.length === 1 ? paths[0].split(/[\\/]/).pop() : paths.length + " files"}…`);
  try {
    const r = await api("load", { paths, merge: !!S.optMerge && paths.length > 1, dedup: S.optDedup || "", sort: S.optSort || "time" });
    await refreshDatasets(r.ids[r.ids.length - 1]);
    if (r.skipped) toast(`Skipped ${r.skipped} file${r.skipped > 1 ? "s" : ""} that ${r.skipped > 1 ? "are" : "is"} not STDF`, "", 6000);
    for (const e of r.errors || []) toast(`Could not read ${e.name}: ${e.error}`, "bad", 10000);
    if (!opts.quiet) toast(`Opened ${r.ids.length} dataset${r.ids.length > 1 ? "s" : ""}`, "good");
    go("overview");
  } catch (e) {
    toast(e.message, "bad", 10000);
  } finally { busy(null); }
}
function uploadOne(file, onProgress) {
  return new Promise((resolve, reject) => {
    const x = new XMLHttpRequest();
    x.open("POST", "/api/upload?name=" + encodeURIComponent(file.name));
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
function pathField(kind, value, opts) {
  opts = opts || {};
  const inp = h("input", { type: "text", value: value || "", placeholder: opts.placeholder || "", style: { minWidth: "320px", flex: "1" } });
  const btn = h("button", { onclick: async () => {
    try {
      const r = kind === "folder" ? await api("pick_folder", { title: opts.title }) : await api("pick_save", { title: opts.title, name: opts.name || "", ext: opts.ext || "" });
      if (r.path) { inp.value = r.path; inp.dispatchEvent(new Event("change")); }
    } catch (e) { toast(e.message, "bad"); }
  } }, "Browse…");
  if (!(S.info && S.info.native_dialogs)) btn.hidden = true;
  return [inp, h("div", { class: "pathfield" }, inp, btn)];
}
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
        h("button", { class: "primary", onclick: async () => {
          if (!path.value) return toast("Choose where to save the merged file", "bad");
          busy("Writing merged STDF…");
          try {
            const r = await api("merge_files", { ds: list.map((f) => f.id), path: path.value, renumber: renumber.checked });
            res.replaceChildren(resultLine(`Saved ${r.path}: ${r.parts.toLocaleString()} parts (${r.good.toLocaleString()} good) from ${r.files} files`, r.path), h("button", { onclick: () => loadPaths([r.path]) }, "Open it"));
          } catch (e) { toast(e.message, "bad"); } finally { busy(null); }
        } }, "Save merged STDF"),
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
// ---- Part datalog: pick parts, pick tests, write their results
const DL_MODES = [["part_id", "Part ID"], ["xy", "X/Y"], ["part_txt", "Part text"], ["test_xy", "Die X/Y from tests"]];
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
  el.append(h("h1", {}, "Part datalog"), h("p", { class: "lead" }, `Pick parts of ${ds ? ds.name : "the dataset"} and the tests you want, then get every result with its limits as easy-to-read text or CSV. Switch the dataset at the top to use another file.`));
  const wc = waferCal();
  const calParams = () => ({ wafer_test: wc.w, x_test: wc.x, y_test: wc.y, ox_test: wc.ox, oy_test: wc.oy, offset_mode: wc.mode, flip_x: wc.fx, flip_y: wc.fy });
  const findParts = (q, limit) => api("find_parts", Object.assign({ ds: S.ds, q, by: st.modes.join(","), limit }, calParams()));
  const label = (r) => [`part ${r.part_id}`, r.part_txt, r.x != null ? `${r.wafer ? r.wafer + " " : ""}X${r.x} Y${r.y}` : "", r.test_xy ? `tests: ${r.test_xy.wafer} X${r.test_xy.x} Y${r.test_xy.y}` : "", `site ${r.site}`, `bin ${r.hbin}`].filter(Boolean).join(" · ");
  const has = (r) => st.parts.some((p) => p.index === r.index);
  const add = (rows) => { let n = 0; for (const r of rows) if (!has(r)) { st.parts.push(Object.assign({ mark: "" }, r)); n++; } drawList(); return n; };
  // 1 · find parts: ticked ways to match, live top 3, full list, pasted list
  const modes = h("div", { class: "row tight" }, DL_MODES.map(([k, l]) => h("label", { class: "chk", title: k === "test_xy" ? `Wafer, X and Y from tests ${[wc.w, wc.x, wc.y].filter(Boolean).join(" / ")}${wc.ox ? `, offsets ${wc.ox} / ${wc.oy}` : ""} (change them on the Settings page)` : "" },
    h("input", { type: "checkbox", checked: st.modes.includes(k), onchange: (e) => { st.modes = DL_MODES.map(([m]) => m).filter((m) => (m === k ? e.target.checked : st.modes.includes(m))); } }), l)),
    h("button", { class: "ghost small", onclick: () => go("settings"), title: "Tests for die X/Y" }, "Settings…"));
  const q = h("input", { type: "text", class: "filter-input", placeholder: "type a part ID (233), X/Y (3,-2  ·  W01 3,-2  ·  X3Y-2) or part text", style: { minWidth: "460px" } });
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
      const p = await api("pick_save", { title: "Save datalog", name: `${base()}.${ext()}`, ext: ext() });
      if (!p.path) return;
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
// ---- Settings: defaults used across the views, kept in this browser profile
function viewSettings(el) {
  el.append(h("h1", {}, "Settings"), h("p", { class: "lead" }, "Defaults used across the views, saved on this PC. A view's own fields still override them for that analysis."));
  const sec = (title, note, ...kids) => h("div", { class: "card pad section" }, h("h2", { style: { marginTop: 0 } }, title), note ? h("p", { class: "muted small" }, note) : null, ...kids);
  const saved = () => { saveCfg(); toast("Saved", "good", 1500); };
  const sort = select([["time", "by test start time (MIR), else file time"], ["name", "by file name (run2 before run10)"], ["none", "as given"]], S.optSort, (v) => { S.optSort = v; saved(); });
  const dedup = select([["", "keep every test (incl. retests)"], ["last", "keep the last test of each part"], ["first", "keep the first test of each part"]], S.optDedup || "", (v) => { S.optDedup = v; saved(); });
  const merge = h("input", { type: "checkbox", checked: !!S.optMerge, onchange: (e) => { S.optMerge = e.target.checked; saved(); } });
  const key = select(KEYS.filter(([k]) => k !== "tests"), S.cfg.key, (v) => { S.cfg.key = v; S.mk = {}; saved(); });
  const ignore = h("input", { type: "checkbox", checked: S.cfg.ignoreIds !== false, onchange: (e) => { S.cfg.ignoreIds = e.target.checked; saved(); } });
  const cfgIn = (k, ph, w) => h("input", { type: "text", value: S.cfg[k] || "", placeholder: ph, style: { width: w || "110px" }, onchange: (e) => { S.cfg[k] = e.target.value.trim(); saved(); } });
  const othersBox = h("div");
  const drawOthers = () => {
    const o = S.cfg.others || (S.cfg.others = []);
    othersBox.replaceChildren(...o.map((x, i) => h("div", { class: "row tight", style: { marginBottom: "6px" } },
      h("input", { type: "text", value: x.name, placeholder: "name, e.g. Die ID", style: { width: "150px" }, onchange: (e) => { x.name = e.target.value.trim(); saved(); } }),
      h("input", { type: "text", value: x.test, placeholder: "test", style: { width: "110px" }, onchange: (e) => { x.test = e.target.value.trim(); saved(); } }),
      h("button", { title: "Remove", onclick: () => { o.splice(i, 1); saved(); drawOthers(); } }, "×"))),
      h("button", { onclick: () => { o.push({ name: "", test: "" }); drawOthers(); } }, "+ Add other ID"));
  };
  drawOthers();
  const lb = h("input", { type: "text", value: S.lbTest || "5100", style: { width: "90px" }, onchange: async (e) => { try { const r = await api("loadboard", { test: e.target.value.trim() }); S.lbTest = String(r.test); S.testsCache = {}; saved(); } catch (err) { toast(err.message, "bad"); } } });
  const wc = waferCal();
  const inp = (k, ph) => h("input", { type: "text", value: wc[k], placeholder: ph, style: { width: "110px" }, onchange: (e) => { wc[k] = e.target.value.trim(); saved(); } });
  const omode = select([["add", "X + offset"], ["sub", "X − offset"]], wc.mode, (v) => { wc.mode = v; saved(); });
  const flip = (k, l) => h("label", { class: "chk" }, h("input", { type: "checkbox", checked: !!wc[k], onchange: (e) => { wc[k] = e.target.checked; saved(); } }), l);
  const range = select([["limits", "within limits"], ["data", "results"], ["both", "both (adaptive)"]], S.co.range, (v) => { S.co.range = v; saveChartOpts(); toast("Saved", "good", 1500); });
  const hist = select([["pct", "% of group"], ["count", "counts"]], S.co.hist, (v) => { S.co.hist = v; saveChartOpts(); toast("Saved", "good", 1500); });
  const skin = select([["", "Default"], ["industry", "Industry (steel blue, serif headings)"], ["office", "Office (navy, sky blue, yellow)"], ["work", "Work (burgundy, magenta)"]], SC.getSkin(), (v) => { SC.setSkin(v); document.getElementById("skin").value = v; toast("Saved", "good", 1500); });
  const reset = () => {
    S.cfg = Object.assign({}, CFG_DEFAULT, { others: [], card: CFG_DEFAULT.card.slice() }); S.wcal = Object.assign({}, WCAL_DEFAULT);
    S.optSort = S.cfg.sort; S.optDedup = ""; S.optMerge = false; S.mk = {};
    if (S.lbTest !== "5100") api("loadboard", { test: "5100" }).catch(() => {});
    S.lbTest = "5100"; saveCfg(); go("settings");
  };
  el.append(
    sec("Opening files", null, h("div", { class: "controls" }, field("Order of opened files", sort), field("Retested parts", dedup), h("label", { class: "chk" }, merge, "Combine the files I open into one dataset"))),
    sec("Part identification and trace", "The PRR part ID, part text, X/Y and wafer are always available for tracing (some products leave part text or X/Y empty). The datalogged (PTR) identity tests below add to them: they show in the part trace card and the parts search of Retest trace, and rebuild die X/Y for Wafer maps, Bad candidates and the Part datalog. Leave a test blank if the program does not log it.",
      h("div", { class: "controls" }, field("Identify parts by", key), h("label", { class: "chk", title: "Identity tests differ by position or board only, so they would top marker rankings and first-fail picks for no physical reason" }, ignore, "Ignore the identification tests in analytics (rankings, signatures, first fails)")),
      h("h3", {}, "Part trace"),
      h("div", { class: "controls" }, field("Wafer test", inp("w", "none")), field("X test", inp("x", "X")), field("Y test", inp("y", "Y")), field("X offset test", inp("ox", "none")), field("Y offset test", inp("oy", "none")), field("Offset", omode), flip("fx", "flip X"), flip("fy", "flip Y")),
      h("div", { class: "controls" }, field("WXY test (wafer, X, Y in one result)", cfgIn("wxy", "none"))),
      h("h3", {}, "Other trace"),
      h("div", { class: "controls" }, field("Test program revision test", cfgIn("rev", "none")), field("Loadboard test", lb)),
      field("Other IDs", othersBox)),
    sec("Look", "Colour theme for the whole app; ◐ at the top switches light and dark. Each histogram also has its own bin-size slider above the chart.", h("div", { class: "controls" }, field("Theme", skin), field("Axis range", range), field("Histogram", hist))),
    h("div", { class: "row" }, h("button", { onclick: reset }, "Reset to defaults")));
}
function viewExport(el) {
  const ds = S.datasets.find((d) => d.id === S.ds);
  const base = (ds ? ds.name : "export").replace(/\.(stdf|std)(\.gz)?$/i, "").replace(/\.gz$/i, "");
  const single = ds && ds.files === 1 && !ds.derived;
  const srcDir = ds && ds.paths.length ? dirname(ds.paths[0]) : "";
  const sep = srcDir.includes("\\") ? "\\" : "/";
  const near = (name) => (srcDir ? srcDir + sep + name : "");
  el.append(h("h1", {}, "Merge, split & export"), mergeCard(), h("p", { class: "lead" }, `Dataset: ${ds ? ds.name : "–"}` + (S.filter ? ` · filter: ${S.filter}` : " · all parts (set a part filter at the top to export a subset)")));

  // report
  const [repPath, repField] = pathField("save", near(base + "_report.html"), { title: "Save HTML report", name: base + "_report.html", ext: "html" });
  const repOut = h("div");
  el.append(h("div", { class: "card pad section" }, h("h2", {}, "HTML report"),
    h("p", { class: "muted" }, "A single self-contained file (works offline) with all analyses. Share it by e-mail or on a drive."),
    h("div", { class: "row" }, repField, h("button", { class: "primary", onclick: async () => {
      if (!repPath.value) return toast("Choose where to save the report", "bad");
      busy("Building report…");
      try { const r = await api("save_report", P({ path: repPath.value })); repOut.replaceChildren(resultLine(`Saved ${r.path}`, r.path)); } catch (e) { toast(e.message, "bad"); } finally { busy(null); }
    } }, "Save report")), repOut));

  // CSV
  const [csvPath, csvField] = pathField("save", near(base + ".csv"), { title: "Save CSV", name: base + ".csv", ext: "csv" });
  const csvTests = h("input", { type: "text", placeholder: "all tests, or e.g. 1000, 1010, VDD_LEAK", style: { minWidth: "260px" } });
  const csvOut = h("div");
  el.append(h("div", { class: "card pad section" }, h("h2", {}, "CSV (parts × tests)"),
    h("p", { class: "muted" }, "One row per part with PRR fields and test values. Opens in Excel, JMP, Python…"),
    h("div", { class: "row" }, field("Tests", csvTests)),
    h("div", { class: "row" }, csvField, h("button", { class: "primary", onclick: async () => {
      if (!csvPath.value) { location.href = `/api/export.csv?ds=${S.ds}&filter=${encodeURIComponent(S.filter)}&tests=${encodeURIComponent(csvTests.value)}`; return; }
      busy("Writing CSV…");
      try { const r = await api("export_csv_file", { ds: String(S.ds), filter: S.filter, tests: csvTests.value, path: csvPath.value }); csvOut.replaceChildren(resultLine(`Saved ${r.path}`, r.path)); } catch (e) { toast(e.message, "bad"); } finally { busy(null); }
    } }, "Export CSV")), csvOut));

  if (!single) {
    el.append(h("div", { class: "card pad section" }, h("h2", {}, "Split / extract STDF"), h("p", { class: "muted" }, ds && ds.derived ? "This dataset was created in the app, so it has no STDF file of its own. Split the file it came from instead (e.g. with its part set in “my own groups”)." : "Splitting and extracting work on datasets opened from a single file. Open the file on its own (without 'combine') to use them.")));
    return;
  }
  // extract
  const [exPath, exField] = pathField("save", near(base + "_selection.stdf"), { title: "Save selected parts as STDF", name: base + "_selection.stdf", ext: "stdf" });
  const exOut = h("div");
  el.append(h("div", { class: "card pad section" }, h("h2", {}, "Extract selected parts to a new STDF"),
    h("p", { class: "muted" }, S.filter ? `Parts matching: ${S.filter}` : "Set a part filter at the top first (e.g. part_id in (12, 57) or hbin = 5)."),
    h("div", { class: "row" }, exField, h("button", { class: "primary", disabled: !S.filter, onclick: async () => {
      if (!exPath.value) return toast("Choose where to save the file", "bad");
      busy("Writing STDF…");
      try { const r = await api("export_stdf", P({ path: exPath.value })); exOut.replaceChildren(resultLine(`Wrote ${r.parts} parts to ${r.path}`, r.path)); } catch (e) { toast(e.message, "bad"); } finally { busy(null); }
    } }, "Extract")), exOut));

  // split
  const by = select([["hbin", "hard bin"], ["sbin", "soft bin"], ["site", "site"], ["head", "head"], ["wafer", "wafer"], ["pass", "pass / fail"], ["part_id", "part ID (one file per part)"], ["part_txt", "part text (one file per PART_TXT)"], ["xy", "die X,Y (one file per die)"], ["test", "PTR result (one file per value of a test)…"], ["list", "a list of parts I paste…"], ["groups", "my own groups (filters)…"]], "hbin", () => renderGroups());
  const splitTest = testInput("spt", "", "e.g. 501 (a logged wafer number)");
  const listKey = select([["part_id", "PRR part ID"], ["part_txt", "PRR part text"], ["xy", "PRR X/Y (x|y)"], ["tests", "PTR test result(s)…"]], "part_id", () => { listTests.hidden = listKey.value !== "tests"; });
  const listTests = h("input", { type: "text", value: "500", placeholder: "e.g. 500 or 501,502,503", style: { width: "170px" }, hidden: true });
  const listBox = h("textarea", { rows: 5, class: "paths", placeholder: "one part per line (or comma separated)\n12\n57\n103" });
  const listName = h("input", { type: "text", value: "listed", style: { width: "140px" } });
  const [dirPath, dirField] = pathField("folder", near(base + "_split"), { title: "Choose output folder", placeholder: "output folder" });
  const groupsBox = h("div");
  const groups = [{ name: "group1", expr: "site in (1,2)" }, { name: "fails", expr: "not pass" }];
  const rest = h("input", { type: "text", value: "rest", placeholder: "(leave empty to drop unmatched parts)" });
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
      if (!dirPath.value) return toast("Choose an output folder", "bad");
      busy("Splitting…");
      try {
        const r = await api("split", { ds: S.ds, filter: S.filter, by: by.value, groups, rest: rest.value, out_dir: dirPath.value, prefix: base, test: splitTest.value, list: listBox.value, list_key: listKey.value === "tests" ? "tests:" + listTests.value : listKey.value, list_name: listName.value });
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
window.addEventListener("hashchange", () => { const v = location.hash.slice(1); if (VIEWS[v] && v !== S.view) go(v); });
window.addEventListener("keydown", (e) => { if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "o") { e.preventDefault(); openFiles(); } });
// live connection: keeps the desktop app running while this window is open,
// and receives files opened from Explorer while the app is running
function connectEvents() {
  try {
    const es = new EventSource("/api/events");
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
  try { await refreshDatasets(); } catch (e) { main.append(h("div", { class: "err" }, String(e))); }
  S.view = VIEWS[v] && (S.datasets.length || VIEWS[v].always) ? v : S.datasets.length ? "overview" : "home";
  go(S.view);
  connectEvents();
  await heartbeat();
  setInterval(heartbeat, 5000);
})();
