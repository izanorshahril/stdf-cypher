// Static report renderer: reads the embedded JSON and builds the page.
"use strict";
(function () {
  const { h, plot, fmt, kpi, table } = SC;
  SC.initTheme();
  const R = JSON.parse(document.getElementById("report-data").textContent);
  const head = document.getElementById("head");
  const body = document.getElementById("body");
  document.title = R.title;
  head.append(
    h("div", { style: { display: "flex", justifyContent: "space-between", gap: "12px", alignItems: "flex-start", flexWrap: "wrap" } },
      h("div", {}, h("div", { class: "brand" }, h("span", { class: "dot" }), "stdf·cypher ", h("small", {}, "report")), h("h1", { style: { fontSize: "24px", marginTop: "8px" } }, R.title), h("p", { class: "lead" }, R.subtitle)),
      h("button", { class: "ghost", onclick: SC.toggleTheme, title: "Toggle light/dark" }, "◐ theme")),
    h("div", { class: "toc" }, R.sections.map((s) => h("a", { href: "#" + s.id }, s.title))));
  const pending = [];
  for (const s of R.sections) {
    const sec = h("section", { class: "report-section", id: s.id }, h("h2", {}, s.title));
    if (s.lead) sec.append(h("p", { class: "lead" }, s.lead));
    if (s.kpis && s.kpis.length) sec.append(h("div", { class: "kpis" }, s.kpis.map((k) => kpi(k[0], k[1], k[2]))));
    if (s.figures && s.figures.length) {
      const g = h("div", { class: s.cols === 1 ? "" : s.cols === 3 ? "grid3" : "grid2" });
      for (const f of s.figures) {
        const c = h("div", { class: "chart " + (f.size || "") });
        g.append(h("div", { class: "card", style: s.cols === 1 ? { marginBottom: "12px" } : null }, c));
        pending.push([c, f.fig]);
      }
      sec.append(g);
    }
    for (const t of s.tables || []) {
      const d = h("div");
      if (t.title) sec.append(h("h2", { style: { fontSize: "14px" } }, t.title));
      sec.append(d);
      const cols = t.columns.map((c) => Object.assign({}, c, c.digits !== undefined ? { fmt: (v) => fmt(v, c.digits) } : {}));
      table(d, cols, t.rows, { search: t.rows.length > 15, limit: 200, sort: t.sort, tools: h("button", { onclick: () => SC.download((t.title || s.id).replace(/\W+/g, "_") + ".csv", SC.toCSV(cols, t.rows)) }, "CSV") });
    }
    body.append(sec);
  }
  // render charts as they scroll into view
  const io = new IntersectionObserver((ents) => {
    for (const e of ents) if (e.isIntersecting) { const p = pending.find((x) => x[0] === e.target); if (p) { plot(p[0], p[1]); io.unobserve(e.target); } }
  }, { rootMargin: "400px" });
  for (const [c] of pending) io.observe(c);
  window.addEventListener("beforeprint", () => pending.forEach(([c, f]) => { if (!c._fig) plot(c, f); }));
})();
