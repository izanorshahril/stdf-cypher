// Shared front-end helpers: theme-aware Plotly rendering, tables, formatting.
"use strict";

const SC = (() => {
  const css = (n) => getComputedStyle(document.documentElement).getPropertyValue(n).trim();
  function tokens() {
    const t = {};
    for (let i = 1; i <= 8; i++) t["@s" + i] = css("--s" + i);
    for (const k of ["pass", "fail", "limit", "muted", "text", "surface", "grid", "warn"]) t["@" + k] = css("--" + k);
    const ramp = (s) => { const c = s.split(",").map((x) => x.trim()); return c.map((col, i) => [i / (c.length - 1), col]); };
    t["@seq"] = ramp(css("--seq"));
    t["@div"] = ramp(css("--div"));
    return t;
  }
  function resolve(v, tk) {
    if (typeof v === "string") return v.startsWith("@") && tk[v] !== undefined ? tk[v] : v;
    if (Array.isArray(v)) return v.map((x) => resolve(x, tk));
    if (v && typeof v === "object") { const o = {}; for (const k in v) o[k] = resolve(v[k], tk); return o; }
    return v;
  }
  function themeLayout(layout) {
    const text = css("--text"), muted = css("--text-2"), grid = css("--grid"), border = css("--border");
    const axis = (a) => Object.assign({ gridcolor: grid, linecolor: border, zerolinecolor: border, tickfont: { color: muted }, automargin: true }, a || {});
    const l = Object.assign({}, layout);
    l.paper_bgcolor = "rgba(0,0,0,0)";
    l.plot_bgcolor = "rgba(0,0,0,0)";
    l.font = Object.assign({ family: css("--sans") || "system-ui", color: text, size: 12 }, l.font || {});
    l.hoverlabel = { bgcolor: css("--surface"), bordercolor: border, font: { color: text } };
    l.xaxis = axis(l.xaxis);
    l.yaxis = axis(l.yaxis);
    if (l.scene) {
      const sax = (a) => Object.assign({ gridcolor: grid, linecolor: border, zerolinecolor: border, backgroundcolor: "rgba(0,0,0,0)", tickfont: { color: muted }, title: { font: { color: muted } } }, a || {});
      l.scene = Object.assign({}, l.scene, { xaxis: sax(l.scene.xaxis), yaxis: sax(l.scene.yaxis), zaxis: sax(l.scene.zaxis), bgcolor: "rgba(0,0,0,0)" });
    }
    if (l.legend) l.legend = Object.assign({ font: { color: muted } }, l.legend);
    return l;
  }
  const plotted = new Set();
  // optional hook that rewrites every figure before drawing (opacity, 3D…)
  let transform = null, afterPlot = null;
  function setTransform(fn, after) { transform = fn; afterPlot = after || null; replotAll(); }
  /** Dense marker-only scatter traces draw several times faster in WebGL (scattergl) with the same look (20k points: about 65 ms against 400 ms in SVG). Lines are faster in SVG at these sizes, so they, fills, error bars and text traces stay SVG. */
  const GL_MIN_POINTS = 5000;
  function glData(data) {
    return data.map((t) => (t.type === "scatter" && t.mode === "markers" && t.x && t.x.length >= GL_MIN_POINTS && !(t.fill || t.error_x || t.error_y || t.stackgroup || t.hoveron) ? Object.assign({}, t, { type: "scattergl" }) : t));
  }
  /** Lists the parts of a box or lasso selection under the chart: the points that carry a part id as customdata. */
  function showSelection(el, ev) {
    let strip = el._strip;
    if (!strip) { strip = el._strip = document.createElement("div"); strip.className = "sel-strip"; el.after(strip); }
    strip.replaceChildren();
    strip.hidden = !ev;
    if (!ev) return;
    // a point is a part; parts with blank or reused ids are told apart by where they sit
    const seen = new Set();
    const rows = [];
    for (const p of ev.points || []) {
      if (typeof p.customdata !== "string") continue;
      const series = (p.data && p.data.name) || "";
      const key = [p.customdata, series, p.x, p.y].join("|");
      if (seen.has(key)) continue;
      seen.add(key);
      rows.push({ part: p.customdata, series, x: p.x, y: p.y });
    }
    if (!rows.length) { strip.append(h("div", {}, "No parts in the selection (this chart does not put one part on each point).")); return; }
    const cols = [{ key: "part", label: "part" }, { key: "series", label: "series" }, { key: "x", label: "x" }, { key: "y", label: "y" }];
    const shown = rows.slice(0, 200);
    strip.append(
      h("div", { class: "row tight" }, h("b", {}, `${rows.length} part${rows.length === 1 ? "" : "s"} selected`), h("button", { class: "small", onclick: () => download("selected_parts.csv", toCSV(cols, rows)) }, "Download CSV"), h("button", { class: "small", onclick: () => { strip.hidden = true; } }, "Hide")),
      h("div", { class: "sel-list" }, shown.map((r) => r.part || "(blank id)").join(", ") + (rows.length > shown.length ? ` … and ${rows.length - shown.length} more` : ""))
    );
  }

  function plot(el, fig) {
    if (!el) return;
    if (typeof el === "string") el = document.getElementById(el);
    el._fig = fig;
    plotted.add(el);
    const tk = tokens();
    let f = resolve(fig, tk);
    if (transform && !el.dataset.raw) f = transform(f, el) || f;
    const layout = themeLayout(f.layout || {});
    const cfg = { responsive: true, displaylogo: false, modeBarButtonsToRemove: ["lasso2d"], toImageButtonOptions: { format: "png", scale: 2 } };
    if (window.Plotly) {
      if (el._dim !== f._dim) { Plotly.purge(el); el._dim = f._dim; el._selBound = false; }
      Plotly.react(el, glData(f.data || []), layout, cfg);
      // box select (the modebar button) lists the parts under the chart
      if (!el._selBound && el.on) { el._selBound = true; el.on("plotly_selected", (ev) => showSelection(el, ev)); el.on("plotly_deselect", () => showSelection(el, null)); }
    }
    else el.innerHTML = '<div class="err">Plotly failed to load (offline?). Re-generate the report with --offline.</div>';
    if (afterPlot && !el.dataset.raw) afterPlot(el, f);
  }
  function replotAll() {
    for (const el of [...plotted]) { if (!document.body.contains(el)) { plotted.delete(el); continue; } plot(el, el._fig); }
  }
  function setTheme(t) {
    if (t) document.documentElement.setAttribute("data-theme", t); else document.documentElement.removeAttribute("data-theme");
    try { localStorage.setItem("sc-theme", t || ""); } catch (e) {}
    replotAll();
  }
  function initTheme() {
    let t = "";
    try { t = localStorage.getItem("sc-theme") || ""; } catch (e) {}
    if (t) document.documentElement.setAttribute("data-theme", t);
    let s = "";
    try { s = localStorage.getItem("sc-skin") || ""; } catch (e) {}
    if (s) document.documentElement.setAttribute("data-skin", s);
    window.matchMedia("(prefers-color-scheme: dark)").addEventListener("change", replotAll);
  }
  /** Colour theme ("" = default, industry, office, work): data-skin on <html>. */
  function setSkin(s) {
    if (s) document.documentElement.setAttribute("data-skin", s); else document.documentElement.removeAttribute("data-skin");
    try { localStorage.setItem("sc-skin", s || ""); } catch (e) {}
    replotAll();
  }
  function getSkin() { return document.documentElement.getAttribute("data-skin") || ""; }
  function toggleTheme() {
    const cur = document.documentElement.getAttribute("data-theme") || (matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light");
    setTheme(cur === "dark" ? "light" : "dark");
  }

  function fmt(v, d) {
    if (v === null || v === undefined || (typeof v === "number" && !isFinite(v))) return "–";
    if (typeof v !== "number") return String(v);
    const a = Math.abs(v);
    if (d !== undefined) return v.toFixed(d);
    if (a !== 0 && (a >= 1e6 || a < 1e-3)) return v.toExponential(3);
    if (Number.isInteger(v)) return String(v);
    return String(+v.toPrecision(5));
  }
  const fmtP = (p) => (p === null || p === undefined ? "–" : p < 1e-4 ? p.toExponential(1) : p.toFixed(4));
  const pct = (v, d = 2) => (v === null || v === undefined ? "–" : v.toFixed(d) + "%");

  function h(tag, attrs, ...kids) {
    const e = document.createElement(tag);
    for (const k in attrs || {}) {
      const v = attrs[k];
      if (v === null || v === undefined || v === false) continue;
      if (k === "class") e.className = v;
      else if (k === "style" && typeof v === "object") Object.assign(e.style, v);
      else if (k.startsWith("on")) e.addEventListener(k.slice(2), v);
      else if (k === "html") e.innerHTML = v;
      else e.setAttribute(k, v === true ? "" : v);
    }
    for (const k of kids.flat()) if (k !== null && k !== undefined && k !== false) e.append(k instanceof Node ? k : document.createTextNode(String(k)));
    return e;
  }
  function kpi(label, value, sub) {
    const long = String(value).length > 10;
    return h("div", { class: "kpi" }, h("div", { class: "l" }, label), h("div", { class: "v", style: long ? { fontSize: "15px", lineHeight: "1.3", overflowWrap: "anywhere" } : null }, value), sub ? h("div", { class: "s" }, sub) : null);
  }
  function chartBox(cls) { const c = h("div", { class: "chart " + (cls || "") }); return [h("div", { class: "card" }, c), c]; }
  function badge(text, kind) { return h("span", { class: "badge " + kind }, h("span", {}, text)); }

  /** Sortable, searchable table. cols: [{key,label,num,fmt,render}]. With
   *  `onClear`, clicking the selected row again (or "Clear selection")
   *  deselects it and calls onClear. */
  function table(container, cols, rows, opts) {
    opts = opts || {};
    let sortKey = opts.sort || null, asc = opts.asc ?? false, q = "", limit = opts.limit || 300, selected = null;
    const wrap = h("div", { class: "tbl-wrap", style: opts.maxHeight ? { maxHeight: opts.maxHeight } : null });
    const count = h("span", { class: "count" });
    const search = h("input", { type: "text", placeholder: opts.placeholder || "Search…", oninput: (e) => { q = e.target.value.toLowerCase(); render(); } });
    const clearBtn = opts.onClear ? h("button", { class: "ghost small", hidden: true, title: "Deselect the row and close what it opened", onclick: () => clear(true) }, "× Clear selection") : null;
    const tools = h("div", { class: "tbl-tools" }, opts.search === false ? null : search, count, clearBtn, opts.tools || null);
    function clear(notify) {
      selected = null;
      wrap.querySelectorAll("tr.sel").forEach((x) => x.classList.remove("sel"));
      if (clearBtn) clearBtn.hidden = true;
      if (notify && opts.onClear) opts.onClear();
    }
    container.replaceChildren(tools, wrap);
    function render() {
      let r = rows;
      if (q) r = r.filter((row) => cols.some((c) => String(row[c.key] ?? "").toLowerCase().includes(q)) || (opts.match && opts.match(row, q)));
      if (sortKey) {
        const c = cols.find((c) => c.key === sortKey);
        r = [...r].sort((a, b) => {
          let x = c && c.sortVal ? c.sortVal(a) : a[sortKey], y = c && c.sortVal ? c.sortVal(b) : b[sortKey];
          if (x === null || x === undefined) return 1;
          if (y === null || y === undefined) return -1;
          if (typeof x === "number" && typeof y === "number") return asc ? x - y : y - x;
          return asc ? String(x).localeCompare(String(y), undefined, { numeric: true }) : String(y).localeCompare(String(x), undefined, { numeric: true });
        });
      }
      count.textContent = `${r.length.toLocaleString()} rows` + (r.length > limit ? ` (showing ${limit})` : "");
      const thead = h("thead", {}, h("tr", {}, cols.map((c) => h("th", { class: c.num ? "num" : "", title: c.title || "", onclick: () => { if (sortKey === c.key) asc = !asc; else { sortKey = c.key; asc = !c.num; } render(); } }, c.label, sortKey === c.key ? h("span", { class: "arrow" }, asc ? " ▲" : " ▼") : null))));
      const tbody = h("tbody");
      for (const row of r.slice(0, limit)) {
        const tr = h("tr", { class: [opts.onClick ? "clickable" + (selected === row ? " sel" : "") : "", opts.rowClass ? opts.rowClass(row) : ""].join(" ").trim() });
        for (const c of cols) {
          const v = row[c.key];
          const content = c.render ? c.render(v, row) : c.fmt ? c.fmt(v, row) : c.num ? fmt(v) : v ?? "";
          tr.append(h("td", { class: c.num ? "num" : "" }, content));
        }
        if (opts.onClick) tr.addEventListener("click", () => {
          if (selected === row && opts.onClear) return clear(true);
          selected = row; [...tbody.children].forEach((x) => x.classList.remove("sel")); tr.classList.add("sel");
          if (clearBtn) clearBtn.hidden = false;
          opts.onClick(row);
        });
        tbody.append(tr);
      }
      const more = r.length > limit ? h("div", { style: { padding: "8px" } }, h("button", { onclick: () => { limit += 1000; render(); } }, "Show more")) : null;
      wrap.replaceChildren(h("table", {}, thead, tbody), more || "");
    }
    render();
    return { setRows: (r) => { rows = r; render(); }, clear: () => clear(false), select: (pred) => { selected = rows.find(pred) || null; if (clearBtn) clearBtn.hidden = !selected; render(); } };
  }

  function toCSV(cols, rows) {
    const esc = (v) => { const s = v === null || v === undefined ? "" : String(v); return /[",\n]/.test(s) ? '"' + s.replace(/"/g, '""') + '"' : s; };
    return [cols.map((c) => esc(c.label)).join(","), ...rows.map((r) => cols.map((c) => esc(r[c.key])).join(","))].join("\n");
  }
  function download(name, text, type) {
    const a = h("a", { href: URL.createObjectURL(new Blob([text], { type: type || "text/csv" })), download: name });
    document.body.append(a); a.click(); a.remove();
  }

  return { tokens, plot, replotAll, setTransform, setTheme, setSkin, getSkin, initTheme, toggleTheme, fmt, fmtP, pct, h, kpi, chartBox, badge, table, toCSV, download };
})();
