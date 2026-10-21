// stdf-cypher interactive dashboard
"use strict";
const { h, plot, fmt, fmtP, pct, kpi, chartBox, badge, table } = SC;

const S = { datasets: [], ds: 0, filter: "", view: "overview", testsCache: {}, scope: "all", co: { range: "both", dim: "2d", opacity: 0.6 } };
try { Object.assign(S.co, JSON.parse(localStorage.getItem("sc-chart") || "{}")); S.scope = localStorage.getItem("sc-scope") || "all"; } catch (e) {}
const main = document.getElementById("main");
const GROUPS = [["", "none"], ["pass", "good / bad"], ["alarm", "alarm"], ["site", "test site"], ["head", "head"], ["hbin", "hard bin"], ["sbin", "soft bin"], ["wafer", "wafer"], ["file", "file / lot"]];

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
  return h("div", { class: "chart-opts" },
    opts.noRange ? null : field("Axis range", range),
    field("View", dim),
    h("label", { class: "field" }, h("span", {}, "Opacity"), h("div", { class: "row tight" }, op, opv)),
    opts.extra || null);
}

/** Applies the chart options to a resolved Plotly figure. */
function chartTransform(f) {
  const op = S.co.opacity;
  for (const t of f.data || []) {
    if (t.type === "bar" && !t.orientation) t.opacity = op;
    else if ((t.type === "scatter" || t.type === "scattergl") && t.mode && t.mode.includes("markers") && t.hoverinfo !== "skip") t.marker = Object.assign({}, t.marker, { opacity: op });
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
SC.setTransform(chartTransform);

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
  match: { label: "Match & trace", fn: viewMatch },
  correlation: { label: "Correlation", fn: viewCorrelation },
  anova: { label: "ANOVA", fn: viewAnova },
  anomaly: { label: "Abnormal parts", fn: viewAnomaly },
  ml: { label: "Machine learning", fn: viewML },
  records: { label: "Records", fn: viewRecords },
  export: { label: "Split & export", fn: viewExport },
};

function renderNav() {
  const nav = document.getElementById("nav");
  nav.replaceChildren();
  for (const [k, v] of Object.entries(VIEWS)) {
    if (k === "overview" || k === "export") nav.append(h("div", { class: "sep" }));
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
      for (const k of ["hist", "box", "cdf", "trend"]) { const [card, c] = chartBox(); g.append(card); plot(c, d[k]); }
      charts.append(h("h2", {}, "Statistics by group"));
      const st = h("div");
      charts.append(st);
      table(st, [{ key: "group", label: "Group" }, { key: "n", label: "N", num: true }, { key: "mean", label: "Mean", num: true }, { key: "std", label: "Std", num: true }, { key: "min", label: "Min", num: true }, { key: "q1", label: "Q1", num: true }, { key: "median", label: "Median", num: true }, { key: "q3", label: "Q3", num: true }, { key: "max", label: "Max", num: true }, { key: "cpk", label: "Cpk", num: true, fmt: (v) => fmt(v, 2) }, { key: "fails", label: "Fails", num: true }], d.stats, { search: false });
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
  el.append(h("h1", {}, "Wafer maps"), h("p", { class: "lead" }, "Bin, pass/fail or test-value maps per wafer. Die coordinates come from the PRR X/Y (wafers from WIR), or are rebuilt from datalogged test results, e.g. wafer, X and Y logged in tests 501, 502 and 503."));
  if (!S.wcoords) S.wcoords = ds && ds.wafers.length ? "prr" : "tests";
  const coords = select([["prr", "PRR X/Y + WIR wafer"], ["tests", "from test results (PTR)"]], S.wcoords, (v) => { S.wcoords = v; S.wafer = "all"; renderCtl(); draw(); });
  const mode = select([["hbin", "hard bin"], ["sbin", "soft bin"], ["pass", "pass / fail"], ["test", "test value"]], S.wmode || "hbin", (v) => { S.wmode = v; draw(); });
  const test = testInput("wt", S.wtest || S.explorerTest || "");
  test.addEventListener("change", () => { S.wtest = test.value; if ((S.wmode || "hbin") === "test") draw(); });
  const small = (inp) => { inp.style.minWidth = "120px"; inp.style.width = "130px"; return inp; };
  const wt = small(testInput("wwt", S.wTestW ?? "501", "none = one map"));
  const xt = small(testInput("wxt", S.wTestX ?? "502", "X test"));
  const yt = small(testInput("wyt", S.wTestY ?? "503", "Y test"));
  for (const i of [wt, xt, yt]) i.addEventListener("change", () => { S.wTestW = wt.value; S.wTestX = xt.value; S.wTestY = yt.value; S.wafer = "all"; draw(); });
  const wafer = select([["all", "all wafers (gallery)"]], "all", (v) => { S.wafer = v; draw(); });
  const ctl = h("div", { class: "controls" });
  function renderCtl() {
    ctl.replaceChildren(field("Coordinates", coords), ...(S.wcoords === "tests" ? [field("Wafer test", wt), field("X test", xt), field("Y test", yt)] : []), field("Color by", mode), field("Test (for test value)", test), field("Wafer", wafer));
  }
  renderCtl();
  const out = h("div");
  el.append(ctl, chartOpts(null, { noRange: true }), out);
  async function draw() {
    loading(out);
    try {
      const d = await api("chart", P({ kind: "wafer", mode: S.wmode || "hbin", test: test.value, wafer: S.wafer || "all", coords: S.wcoords, wafer_test: wt.value, x_test: xt.value, y_test: yt.value }));
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
    const run = h("button", { class: "primary", onclick: () => {
      S.cmpFA = fa.value; S.cmpFB = fb.value; S.cmpSel = sel.value; S.cmpVs = vs.value;
      const params = m === "group" ? { mode: "group", a: S.ds, select: sel.value, vs: vs.value, filter: scoped(S.filter), match_by: matchBy.value } : { mode: "datasets", a: +a.value, b: +b.value, filter_a: scoped(fa.value), filter_b: scoped(fb.value), match_by: matchBy.value };
      runCompare(out, params);
    } }, "Compare");
    ctl.replaceChildren(field("Mode", mode));
    if (m === "group") ctl.append(field("Group (B)", sel), field("Versus (A)", vs));
    else ctl.append(field("A", a), field("Filter A", fa), field("B", b), field("Filter B", fb));
    ctl.append(field("Match tests by", matchBy), run);
    return run;
  }
  const runBtn = render();
  if (S.cmpAuto || (mode.value === "datasets" && S.datasets.length > 1)) { S.cmpAuto = false; runBtn.click(); }
}
async function runCompare(out, params) {
  loading(out);
  try {
    const d = await api("compare", params);
    const r = d.report;
    out.replaceChildren(h("div", { class: "kpis" },
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
      { key: "flag", label: "Flag", render: (v) => badge(v, flagKind(v)) },
      { key: "key", label: "Test" },
      { key: "units", label: "Units" },
      { key: "mean_a", label: "Mean A", num: true, sortVal: (r) => r.a.mean },
      { key: "mean_b", label: "Mean B", num: true, sortVal: (r) => r.b.mean },
      { key: "shift_sigma", label: "Shift σ", num: true, fmt: (v) => fmt(v, 2) },
      { key: "shift_pct_limit", label: "Shift % lim", num: true, fmt: (v) => (v === null ? "–" : v.toFixed(1)) },
      { key: "sigma_ratio", label: "σ ratio", num: true, fmt: (v) => fmt(v, 2) },
      { key: "ks_d", label: "KS D", num: true, fmt: (v) => fmt(v, 3) },
      { key: "q", label: "q", num: true, fmt: fmtP },
      { key: "fail_rate_a", label: "Fail% A", num: true, fmt: (v) => v.toFixed(2) },
      { key: "fail_rate_b", label: "Fail% B", num: true, fmt: (v) => v.toFixed(2) },
    ];
    const rows = r.tests.map((t) => Object.assign({ mean_a: t.a.mean, mean_b: t.b.mean }, t));
    const showTest = async (row, scroll) => {
      loading(detail);
      try {
        const c = await api("chart", Object.assign({}, params, { kind: "compare_test", ds: params.a, test: row.key, range: S.co.range }));
        detail.replaceChildren(h("h2", {}, row.key), chartOpts(() => showTest(row, false)));
        const g2 = h("div", { class: "grid3" });
        detail.append(g2);
        for (const k of ["hist", "cdf", "box"]) { const [card, cc] = chartBox(); g2.append(card); plot(cc, c[k]); }
        if (scroll) detail.scrollIntoView({ behavior: "smooth", block: "start" });
      } catch (e) { showErr(detail, e); }
    };
    table(tb, cols, rows, {
      onClick: (row) => showTest(row, true),
      tools: h("button", { onclick: () => SC.download("compare.csv", SC.toCSV(cols, rows)) }, "Download CSV"),
    });
    out.append(h("h2", {}, "Site yield"));
    const sy = h("div");
    out.append(sy);
    table(sy, [{ key: "0", label: "Site" }, { key: "1", label: "Yield A", num: true, fmt: (v) => pct(v) }, { key: "2", label: "N A", num: true }, { key: "3", label: "Yield B", num: true, fmt: (v) => pct(v) }, { key: "4", label: "N B", num: true }], r.site_yield.map((x) => ({ 0: x[0], 1: x[1], 2: x[2], 3: x[3], 4: x[4] })), { search: false });
  } catch (e) { showErr(out, e); }
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
  const factor = select(GROUPS.filter(([k]) => k && k !== "pass").concat([["x", "die X"], ["y", "die Y"]]), S.anovaFactor || "site", (v) => { S.anovaFactor = v; });
  const ming = h("input", { type: "number", value: 5, min: 2 });
  const out = h("div");
  el.append(h("div", { class: "controls" }, field("Factor", factor), field("Min parts / group", ming), h("button", { class: "primary", onclick: run }, "Run ANOVA")), out);
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
async function partDetail(detail, ds, index, title) {
  loading(detail);
  try {
    const pd = await api("part_detail", { ds, index });
    const raw = h("div");
    const t = h("div");
    detail.replaceChildren(h("h2", {}, title || `Part ${pd.part.part_id}`), h("p", { class: "muted" }, partSummary(pd.part, pd.wafer)),
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
  const cur = S.mk[side] || "part_txt";
  const isTests = !KEYS.some(([k]) => k === cur);
  const sel = select(KEYS, isTests ? "tests" : cur, () => sync());
  const tests = h("input", { type: "text", value: isTests ? cur.replace(/^tests:/, "") : "500", placeholder: "e.g. 500  or  501,502,503", title: "One test with a unique per-part value, or several tests whose values together identify the part (e.g. wafer, X, Y)", style: { width: "190px" }, oninput: () => sync() });
  const value = () => (sel.value === "tests" ? "tests:" + tests.value.trim() : sel.value);
  function sync() { tests.hidden = sel.value !== "tests"; S.mk[side] = value(); }
  sync();
  return { el: h("div", { class: "row tight" }, sel, tests), value };
}
function viewMatch(el) {
  el.append(h("h1", {}, "Match & trace"), h("p", { class: "lead" }, "Find the same physical parts in two files and follow them across: by PRR part text or part ID, by die X/Y, or by datalogged PTR results (one test holding a unique wafer + X/Y code, or wafer, X and Y in separate tests). Matched parts become a part set you can filter on, compare, split out and chart side by side."));
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
  el.append(h("div", { class: "controls" }, field("File A", a), field("Identify A parts by", ka.el), field("Filter A", fa)),
    two ? h("div", { class: "controls" }, field("File B", b), field("Identify B parts by", kb.el), field("Filter B", fb), field("Part set name", name), h("button", { class: "primary", onclick: run }, "Match parts")) : "");
  // trace a single unit by its identity
  const tv = h("input", { type: "text", placeholder: "the A identity value, e.g. LOTA-W01-X3Y-2  or  11253  or  1|3|-2", style: { minWidth: "300px", flex: "1" }, onkeydown: (e) => { if (e.key === "Enter") doTrace(); } });
  const doTrace = () => { sync(); trace(traceBox, { a: st.a, b: two && st.a !== st.b ? st.b : null, key_a: st.ka, key_b: st.kb, value: tv.value }); };
  el.append(h("div", { class: "controls" }, field("Trace a part", tv), h("button", { onclick: doTrace }, "Trace"), h("span", { class: "muted small", style: { paddingBottom: "8px" } }, "Several test values: separate with | in the order of the tests.")), out, traceBox);
  if (S.mres && S.mres.a === st.a && S.mres.b === st.b && ids.includes(st.a) && ids.includes(st.b)) renderMatch(out, S.mres, traceBox);
}
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
    const [dirP, dirF] = pathField("folder", nearDs(d, `${base}_${m.name}_split`), { title: "Choose output folder", placeholder: "output folder" });
    const res = h("div");
    const single = d && d.files === 1;
    return h("div", { class: "card pad" }, h("h2", { style: { marginTop: 0 } }, `${label}: ${dsName(id)}`),
      h("div", { class: "row" },
        h("button", { onclick: () => useFilter(id, set) }, "Analyse matched parts"),
        h("button", { onclick: () => useFilter(id, `not ${set}`) }, "Analyse unmatched parts"),
        h("button", { onclick: () => { runCompare(cmpOut, { mode: "group", a: id, select: set, vs: `not ${set}`, filter: scoped(""), match_by: "key" }); cmpOut.scrollIntoView({ behavior: "smooth" }); } }, "Compare matched vs unmatched")),
      single ? h("div", { class: "row" }, dirF) : null,
      single ? h("div", { class: "row" }, h("button", { class: "primary", onclick: async () => {
        if (!dirP.value) return toast("Choose an output folder", "bad");
        busy("Splitting…");
        try {
          const r = await api("split", { ds: id, by: "groups", groups: [{ name: "matched", expr: set }], rest: "unmatched", out_dir: dirP.value, prefix: base });
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
    tools: h("button", { onclick: () => SC.download(`matched_${m.name}.csv`, SC.toCSV(cols.filter((c) => c.key !== "same"), rows)) }, "Download CSV"),
  });
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
    el.replaceChildren(h("h2", {}, `Trace: ${t.key || "–"}`), h("p", { class: "muted" }, `Identity in A: ${t.key_a}`),
      h("div", { class: twoSided ? "grid2" : "" }, card("A", params.a, t.a, t.a_hits), twoSided ? card("B", params.b, t.b, t.b_hits) : null), tb);
    const pb = (v) => (v === null || v === undefined ? "" : v ? badge("in", "good") : badge("out", "bad"));
    const cols = [{ key: "key", label: "Test" }, { key: "units", label: "Units" }, { key: "lo", label: "Lo", num: true }, { key: "hi", label: "Hi", num: true }, { key: "a", label: "A", num: true }, { key: "a_pass", label: "", render: pb }];
    if (twoSided) cols.push({ key: "b", label: "B", num: true }, { key: "b_pass", label: "", render: pb }, { key: "diff", label: "B − A", num: true });
    if (t.tests.length) table(tb, cols, t.tests, { tools: h("button", { onclick: () => SC.download("trace.csv", SC.toCSV(cols, t.tests)) }, "Download CSV") });
    el.scrollIntoView({ behavior: "smooth", block: "start" });
  } catch (e) { showErr(el, e); }
}

// ---- Home
const FILE_HINT = "STDF v4 files: .stdf, .std or gzip-compressed .gz";
async function viewHome(el) {
  const has = S.datasets.length > 0;
  const merge = h("input", { type: "checkbox", checked: !!S.optMerge, onchange: (e) => (S.optMerge = e.target.checked) });
  const dedup = select([["", "keep every test (incl. retests)"], ["last", "keep the last test of each part"], ["first", "keep the first test of each part"]], S.optDedup || "", (v) => (S.optDedup = v));
  const browse = h("button", { class: "primary big", onclick: openFiles }, "Open STDF files…");
  const drop = h("div", { class: "dropzone", onclick: openFiles },
    h("img", { src: "/static/icon.png", alt: "", width: 64, height: 64 }),
    h("div", { class: "dz-title" }, has ? "Add more files" : "Drop STDF files here"),
    h("div", { class: "muted" }, FILE_HINT),
    h("div", { class: "dz-actions" }, browse, h("button", { onclick: (e) => { e.stopPropagation(); openSamples(); } }, "Try sample data")));
  el.append(h("div", { class: "home-head" },
    h("div", {}, h("h1", {}, has ? "Your data" : "Welcome to STDF Cypher"),
      h("p", { class: "lead" }, has ? "Pick a dataset to analyse, or add more files." : "Fast, private STDF v4 analysis on your own PC: yield, bins, wafer maps, lot and tester comparison, correlation, ANOVA, outlier screening and machine learning.")),
  ));
  const grid = h("div", { class: "home-grid" });
  el.append(grid);
  const left = h("div", {});
  const right = h("div", {});
  grid.append(left, right);
  left.append(drop, h("div", { class: "home-opts" }, h("label", { class: "chk" }, merge, "Combine the files I open into one dataset (lot-to-lot, ANOVA by file)"), field("Retested parts", dedup)));

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
    ["match", "Match & trace", "Same parts in two files by part text, X/Y or logged tests"],
    ["correlation", "Correlation", "Tester-to-tester correlation, test-to-test matrix"],
    ["anova", "ANOVA", "Which tests differ by site, wafer or lot"],
    ["anomaly", "Abnormal parts", "PAT, robust z, wafer NNR, Isolation Forest"],
    ["ml", "Machine learning", "What drives fails? PCA & clustering"],
    ["records", "Records", "Part table and every raw STDF record"],
    ["export", "Split & export", "Split STDF by part/bin, CSV, HTML report"],
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
    const r = await api("load", { paths, merge: !!S.optMerge && paths.length > 1, dedup: S.optDedup || "" });
    await refreshDatasets(r.ids[r.ids.length - 1]);
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
async function uploadFiles(files) {
  files = files.filter((f) => f.size > 0);
  if (!files.length) return;
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
window.addEventListener("drop", (e) => {
  e.preventDefault();
  dragDepth = 0;
  document.body.classList.remove("dragging");
  const files = [...(e.dataTransfer?.files || [])];
  if (files.length) uploadFiles(files);
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
function viewExport(el) {
  const ds = S.datasets.find((d) => d.id === S.ds);
  const base = (ds ? ds.name : "export").replace(/\.(stdf|std)(\.gz)?$/i, "").replace(/\.gz$/i, "");
  const single = ds && ds.files === 1;
  const srcDir = ds && ds.paths.length ? dirname(ds.paths[0]) : "";
  const sep = srcDir.includes("\\") ? "\\" : "/";
  const near = (name) => (srcDir ? srcDir + sep + name : "");
  el.append(h("h1", {}, "Split & export"), h("p", { class: "lead" }, `Dataset: ${ds ? ds.name : "–"}` + (S.filter ? ` · filter: ${S.filter}` : " · all parts (set a part filter at the top to export a subset)")));

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
    el.append(h("div", { class: "card pad section" }, h("h2", {}, "Split / extract STDF"), h("p", { class: "muted" }, "Splitting and extracting work on datasets opened from a single file. Open the file on its own (without 'combine') to use them.")));
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
  try { await refreshDatasets(); } catch (e) { main.append(h("div", { class: "err" }, String(e))); }
  S.view = VIEWS[v] && (S.datasets.length || VIEWS[v].always) ? v : S.datasets.length ? "overview" : "home";
  go(S.view);
  connectEvents();
  await heartbeat();
  setInterval(heartbeat, 5000);
})();
