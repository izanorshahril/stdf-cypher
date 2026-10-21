//! Self-contained interactive HTML report.

use crate::analysis::{anomaly, anova, compare, correlation, summary};
use crate::data::{short_name, Dataset};
use crate::ml;
use crate::viz;
use anyhow::Result;
use serde_json::{json, Value};

const REPORT_JS: &str = include_str!("../web/report.js");

#[derive(Debug, Clone)]
pub struct ReportOptions {
    pub title: Option<String>,
    pub top_tests: usize,
    pub group_by: String,
    /// embed plotly.js (works offline, +4.5 MB)
    pub offline: bool,
    pub anomaly: bool,
    pub ml: bool,
    pub correlation_tests: usize,
}

impl Default for ReportOptions {
    fn default() -> Self {
        Self {
            title: None,
            top_tests: 12,
            group_by: "site".into(),
            offline: false,
            anomaly: true,
            ml: true,
            correlation_tests: 40,
        }
    }
}

fn col(key: &str, label: &str) -> Value {
    json!({"key": key, "label": label})
}
fn ncol(key: &str, label: &str) -> Value {
    json!({"key": key, "label": label, "num": true})
}
fn dcol(key: &str, label: &str, digits: u32) -> Value {
    json!({"key": key, "label": label, "num": true, "digits": digits})
}
fn fig(f: Value) -> Value {
    json!({"fig": f})
}
fn tall(f: Value) -> Value {
    json!({"fig": f, "size": "tall"})
}
fn n(v: f64) -> Value {
    if v.is_finite() {
        json!(v)
    } else {
        Value::Null
    }
}

/// `files` are the individual datasets (one per input file); `merged` is all of them combined.
pub fn build(files: &[Dataset], merged: &Dataset, opt: &ReportOptions) -> Result<String> {
    let ds = merged;
    let rows = ds.all_rows();
    let mut sections: Vec<Value> = vec![];
    let ov = summary::overview(ds);

    // 1. Summary
    let hb = summary::bins(ds, &rows, true);
    let sb = summary::bins(ds, &rows, false);
    let mut figs = vec![
        fig(viz::bin_pareto(&hb, true)),
        fig(viz::bin_pareto(&sb, false)),
        fig(viz::yield_bar(
            &summary::yield_by(ds, &rows, "site"),
            "Yield by site",
            "site",
        )),
    ];
    if ds.wafers.len() > 1 {
        figs.push(fig(viz::yield_bar(
            &summary::yield_by(ds, &rows, "wafer"),
            "Yield by wafer",
            "wafer",
        )));
    }
    if ds.sources.len() > 1 {
        figs.push(fig(viz::yield_bar(
            &summary::yield_by(ds, &rows, "file"),
            "Yield by file",
            "file",
        )));
    }
    let fp = summary::fail_pareto(ds, &rows);
    sections.push(json!({
        "id": "summary", "title": "Summary",
        "kpis": [
            ["Parts", ov.n_parts.to_string(), format!("{} file(s)", ov.files.len())],
            ["Yield", format!("{:.2}%", ov.yield_pct), format!("{} pass", ov.n_pass)],
            ["Tests", ov.n_tests.to_string(), ""],
            ["Wafers", ov.n_wafers.to_string(), ""],
            ["Sites", ov.sites.to_string(), ""],
            ["Lot", ov.lots.join(", "), ov.part_typ.clone()],
            ["Program", ov.job.clone(), ov.tester.clone()],
        ],
        "figures": figs,
        "tables": [
            {"title": "Fail pareto (first failing test)", "columns": [col("key","Test"), ncol("first_fail","First-fail parts"), dcol("pct_of_fails","% of fails",1), ncol("any_fail","Out-of-limit parts")], "rows": fp.iter().take(40).collect::<Vec<_>>()},
            {"title": "Hard bins", "columns": [ncol("bin","Bin"), col("name","Name"), col("pf","P/F"), ncol("count","Parts"), dcol("pct","%",2)], "rows": hb},
            {"title": "Source files", "columns": [col("path","File"), col("lot_id","Lot"), col("part_typ","Product"), col("job_nam","Program"), col("node_nam","Node"), col("test_cod","Step"), col("tst_temp","Temp")], "rows": ds.sources}
        ]
    }));

    // 2. Wafer maps
    if !ds.wafers.is_empty() {
        let mut maps = vec![];
        for (wi, name) in ds.wafers.iter().enumerate().take(12) {
            let rw: Vec<usize> = rows
                .iter()
                .copied()
                .filter(|&i| ds.parts[i].wafer == wi as u32)
                .collect();
            if !rw.is_empty() {
                maps.push(fig(viz::wafer_map_opts(
                    ds,
                    &rw,
                    "hbin",
                    None,
                    &format!("{name} – {:.1}%", ds.yield_pct(&rw)),
                    false,
                )));
            }
        }
        sections.push(json!({"id": "wafers", "title": "Wafer maps", "lead": format!("Hard-bin maps ({} wafer(s){}); green = pass, colors = failing bins (hover for bin and part).", ds.wafers.len(), if ds.wafers.len() > 12 {", first 12 shown"} else {""}), "figures": maps, "cols": 3}));
    }

    // 3. Top tests
    let tsum = summary::tests(ds, &rows);
    let mut order: Vec<&summary::TestSummary> = tsum
        .iter()
        .filter(|t| t.kind != "Functional" && t.stats.n > 0)
        .collect();
    order.sort_by(|a, b| {
        b.fail_pct.partial_cmp(&a.fail_pct).unwrap().then(
            a.stats
                .cpk
                .unwrap_or(f64::MAX)
                .partial_cmp(&b.stats.cpk.unwrap_or(f64::MAX))
                .unwrap(),
        )
    });
    let groups = viz::fold_groups(
        ds.group_by(&rows, &opt.group_by)
            .into_iter()
            .map(|(k, r)| (format!("{} {k}", opt.group_by), r))
            .collect(),
    );
    let top: Vec<Value> = order
        .iter()
        .take(opt.top_tests)
        .map(|t| fig(viz::histogram(ds, t.idx, &groups, None, groups.len() > 1)))
        .collect();
    let trows: Vec<Value> = tsum.iter().map(|t| json!({"key": t.key, "units": t.units, "lo": t.lo, "hi": t.hi, "n": t.stats.n, "mean": n(t.stats.mean), "std": n(t.stats.std), "min": n(t.stats.min), "max": n(t.stats.max), "cpk": t.stats.cpk, "fail_pct": n(t.fail_pct)})).collect();
    sections.push(json!({
        "id": "tests", "title": "Test distributions",
        "lead": format!("The {} tests with the highest fail rate / lowest Cpk, split by {}. Dashed lines are test limits.", top.len(), opt.group_by),
        "figures": top,
        "tables": [{"title": "All tests", "columns": [col("key","Test"), col("units","Units"), ncol("lo","Lo"), ncol("hi","Hi"), ncol("n","N"), ncol("mean","Mean"), ncol("std","Std"), ncol("min","Min"), ncol("max","Max"), dcol("cpk","Cpk",2), dcol("fail_pct","Fail %",2)], "rows": trows, "sort": "fail_pct"}]
    }));

    // 4. Comparison of first two files
    if files.len() >= 2 {
        let (a, b) = (&files[0], &files[1]);
        let rep = compare::compare(
            &compare::Side::all(a, short_name(&a.sources[0].path)),
            &compare::Side::all(b, short_name(&b.sources[0].path)),
            &Default::default(),
        );
        let mut cfigs = vec![fig(viz::compare_volcano(&rep)), fig(viz::bin_compare(&rep))];
        for t in rep
            .tests
            .iter()
            .filter(|t| t.flag != "OK" && !t.flag.starts_with("ONLY"))
            .take(6)
        {
            if let (Some(ta), Some(tb)) = (a.test_index.get(&t.key), b.test_index.get(&t.key)) {
                let g = vec![
                    (rep.label_a.clone(), a.values_of(*ta, None)),
                    (rep.label_b.clone(), b.values_of(*tb, None)),
                ];
                cfigs.push(fig(viz::histogram_v(&a.tests[*ta], &g, None, true)));
            }
        }
        let crow: Vec<Value> = rep.tests.iter().map(|t| json!({"flag": t.flag, "key": t.key, "mean_a": n(t.a.mean), "mean_b": n(t.b.mean), "shift_sigma": n(t.shift_sigma), "shift_pct_limit": t.shift_pct_limit, "sigma_ratio": n(t.sigma_ratio), "ks_d": n(t.ks_d), "q": n(t.q), "fail_rate_a": t.fail_rate_a, "fail_rate_b": t.fail_rate_b})).collect();
        sections.push(json!({
            "id": "compare", "title": format!("Compare: {} vs {}", rep.label_a, rep.label_b),
            "lead": "Per-test mean shift (pooled σ and % of limit range), spread ratio, Welch t and KS tests with Benjamini–Hochberg q-values.",
            "kpis": [[rep.label_a.clone(), format!("{:.2}%", rep.yield_a), format!("{} parts", rep.n_a)], [rep.label_b.clone(), format!("{:.2}%", rep.yield_b), format!("{} parts", rep.n_b)], ["Flagged tests", rep.n_flagged.to_string(), format!("of {}", rep.tests.len())]],
            "figures": cfigs,
            "tables": [{"title": "Per-test comparison", "columns": [col("flag","Flag"), col("key","Test"), ncol("mean_a","Mean A"), ncol("mean_b","Mean B"), dcol("shift_sigma","Shift σ",2), dcol("shift_pct_limit","Shift % lim",1), dcol("sigma_ratio","σ ratio",2), dcol("ks_d","KS D",3), ncol("q","q"), dcol("fail_rate_a","Fail% A",2), dcol("fail_rate_b","Fail% B",2)], "rows": crow}]
        }));
        let uc =
            correlation::unit_correlation(a, &a.all_rows(), b, &b.all_rows(), &Default::default());
        if uc.matched >= 5 {
            let ufigs: Vec<Value> = uc
                .tests
                .iter()
                .take(4)
                .map(|t| {
                    fig(viz::unit_corr_scatter(
                        t,
                        &short_name(&a.sources[0].path),
                        &short_name(&b.sources[0].path),
                    ))
                })
                .collect();
            let urows: Vec<Value> = uc.tests.iter().map(|t| json!({"status": t.status, "key": t.key, "n": t.n, "r": n(t.r), "slope": n(t.slope), "mean_diff": n(t.mean_diff), "diff_pct_limit": t.diff_pct_limit, "sd_diff": n(t.sd_diff)})).collect();
            let mut k = vec![
                json!([
                    "Matched parts",
                    uc.matched.to_string(),
                    format!("by {}", uc.match_by)
                ]),
                json!(["Bin agreement", format!("{:.1}%", uc.bin_agreement_pct), ""]),
            ];
            for (s, c) in &uc.summary {
                if *c > 0 {
                    k.push(json!([
                        s.to_lowercase().replace('_', " "),
                        c.to_string(),
                        ""
                    ]));
                }
            }
            sections.push(json!({
                "id": "unitcorr", "title": "Unit-level correlation",
                "lead": "Parts present in both files matched one-to-one; each test classified as CORRELATED, OFFSET or UNCORRELATED (r ≥ 0.9, offset ≤ 5% of limit range).",
                "kpis": k, "figures": ufigs,
                "tables": [{"title": "Per-test correlation", "columns": [col("status","Status"), col("key","Test"), ncol("n","N"), dcol("r","r",4), dcol("slope","Slope",3), ncol("mean_diff","Mean B−A"), dcol("diff_pct_limit","Diff % lim",2), ncol("sd_diff","σ diff")], "rows": urows}]
            }));
        }
    }

    // 5. ANOVA
    let numeric = ds.numeric_tests();
    let mut factors = vec!["site".to_string()];
    if ds.sources.len() > 1 {
        factors.push("file".into());
    }
    if ds.wafers.len() > 1 {
        factors.push("wafer".into());
    }
    for f in factors {
        let rep = anova::anova(ds, &rows, &f, &numeric, 5, 0.01);
        if rep.groups.len() < 2 {
            continue;
        }
        let mut afigs = vec![tall(viz::anova_effects(&rep.rows, &f, 20))];
        for r in rep.rows.iter().take(3) {
            afigs.push(fig(viz::anova_means(r, &f)));
        }
        let arows: Vec<Value> = rep.rows.iter().map(|x| json!({"key": x.key, "eta2": n(x.eta2), "f": n(x.f), "p": n(x.p), "q": n(x.q), "kw_p": n(x.kw_p), "levene_p": n(x.levene_p), "range_pct_limit": x.range_pct_limit})).collect();
        sections.push(json!({
            "id": format!("anova-{f}"), "title": format!("ANOVA by {f}"),
            "lead": format!("{} of {} tests differ significantly between {} groups (BH q < 0.01). η² = share of variance explained by {f}.", rep.n_significant, rep.rows.len(), rep.groups.len()),
            "figures": afigs,
            "tables": [{"title": "", "columns": [col("key","Test"), dcol("eta2","η²",4), dcol("f","F",2), ncol("p","p"), ncol("q","q"), ncol("kw_p","Kruskal p"), ncol("levene_p","Levene p"), dcol("range_pct_limit","Mean range % lim",1)], "rows": arows}]
        }));
    }

    // 6. Correlation
    let ct = correlation::pick_tests(ds, &rows, opt.correlation_tests);
    if ct.len() >= 3 {
        let m = correlation::matrix(ds, &rows, &ct, "pearson", 0.7);
        let target: Vec<f64> = rows
            .iter()
            .map(|&i| {
                ds.parts[i]
                    .passed
                    .map(|b| if b { 0.0 } else { 1.0 })
                    .unwrap_or(f64::NAN)
            })
            .collect();
        let tc = correlation::with_target(ds, &rows, &target, "spearman", None);
        let prow: Vec<Value> = m
            .pairs
            .iter()
            .take(200)
            .map(|(a, b, r, k)| json!({"a": a, "b": b, "r": r, "n": k}))
            .collect();
        sections.push(json!({
            "id": "correlation", "title": "Correlation",
            "lead": format!("Pearson matrix of the {} most variable tests, and the tests most correlated with failing.", ct.len()),
            "figures": [tall(viz::corr_heatmap(&m)), tall(viz::target_corr_bar(&tc, "Correlation with fail", 25))],
            "tables": [{"title": "Strong pairs (|r| ≥ 0.7)", "columns": [col("a","Test A"), col("b","Test B"), dcol("r","r",4), ncol("n","N")], "rows": prow}]
        }));
    }

    // 7. Anomalies
    if opt.anomaly {
        let rep = anomaly::detect(ds, &rows, &Default::default());
        let mut afigs = vec![fig(viz::anomaly_scatter(ds, &rep))];
        if let Some(p) = rep.parts.first() {
            if p.x != -32768 {
                afigs.push(fig(viz::anomaly_wafer(ds, &rep, ds.parts[p.row].wafer)));
            }
        }
        let prow: Vec<Value> = rep.parts.iter().take(500).map(|p| json!({"part_id": p.part_id, "wafer": p.wafer, "x": p.x, "y": p.y, "site": p.site, "hbin": p.hbin, "pass": if p.pass {"pass"} else {"fail"}, "methods": p.methods.join(", "), "score": p.score, "top": p.top_tests.first().map(|t| format!("{} (z={:.1})", t.0, t.1)).unwrap_or_default()})).collect();
        let mut k = vec![json!([
            "Flagged parts",
            rep.n_flagged.to_string(),
            format!("of {}", rep.n_parts)
        ])];
        let mut bm: Vec<(&String, &usize)> = rep.by_method.iter().collect();
        bm.sort();
        for (m, c) in bm {
            k.push(json!([m, c.to_string(), ""]));
        }
        let pat: Vec<&anomaly::TestPat> = rep.pat.iter().filter(|t| t.n_outside > 0).collect();
        sections.push(json!({
            "id": "anomaly", "title": "Abnormal parts",
            "lead": "Robust z (|z| ≥ 6), static PAT (passing parts outside robust mean ± 6σ), wafer NNR (|z| ≥ 5), Mahalanobis (p < 1e-6) and Isolation Forest (score ≥ 0.62).",
            "kpis": k, "figures": afigs,
            "tables": [
                {"title": "Flagged parts", "columns": [col("part_id","Part"), col("wafer","Wafer"), ncol("x","X"), ncol("y","Y"), ncol("site","Site"), ncol("hbin","HBin"), col("pass","Result"), col("methods","Methods"), dcol("score","Score",2), col("top","Top test")], "rows": prow},
                {"title": "PAT limits with passing parts outside", "columns": [col("key","Test"), ncol("n_outside","Passing outside"), ncol("pat_lo","PAT lo"), ncol("pat_hi","PAT hi"), ncol("lo","Spec lo"), ncol("hi","Spec hi")], "rows": pat, "sort": "n_outside"}
            ]
        }));
    }

    // 8. ML
    if opt.ml {
        let mut mfigs = vec![];
        let mut k = vec![];
        let mut tables = vec![];
        if let Ok(pr) = ml::pca_report(ds, &rows, &Default::default(), 10, None) {
            mfigs.push(fig(viz::pca_scatter(ds, &pr, "cluster")));
            mfigs.push(fig(viz::pca_scatter(ds, &pr, "pass")));
            if let Some(km) = &pr.kmeans {
                k.push(json!([
                    "Clusters",
                    km.k.to_string(),
                    format!("silhouette {:.3}", km.silhouette)
                ]));
            }
            k.push(json!([
                "PC1+PC2",
                format!(
                    "{:.1}%",
                    (pr.pca.explained_ratio.first().unwrap_or(&0.0)
                        + pr.pca.explained_ratio.get(1).unwrap_or(&0.0))
                        * 100.0
                ),
                "variance explained"
            ]));
        }
        if ds.parts.iter().filter(|p| !p.is_pass()).count() >= 5 {
            if let Ok(fr) = ml::explain(
                ds,
                &rows,
                &ml::TargetSpec::PassFail,
                &Default::default(),
                &Default::default(),
            ) {
                k.push(json!([
                    "RF ROC AUC",
                    format!("{:.3}", fr.auc.unwrap_or(f64::NAN)),
                    "pass/fail, hold-out"
                ]));
                k.push(json!([
                    "RF balanced accuracy",
                    format!("{:.1}%", fr.balanced_accuracy.unwrap_or(0.0) * 100.0),
                    "out-of-bag tuned threshold"
                ]));
                mfigs.push(fig(viz::importance_bar(&fr, 20)));
                mfigs.push(fig(viz::confusion(&fr)));
                let irow: Vec<Value> = fr
                    .importance
                    .iter()
                    .take(50)
                    .map(|(k, v)| json!({"key": k, "imp": v * 100.0}))
                    .collect();
                tables.push(json!({"title": "Random-forest importance (pass/fail)", "columns": [col("key","Test"), dcol("imp","Importance %",2)], "rows": irow}));
            }
        }
        if !mfigs.is_empty() {
            sections.push(json!({"id": "ml", "title": "Machine learning", "lead": "PCA with k-means++ clustering of the robust-standardized test matrix, and a class-balanced random forest predicting pass/fail from parametric tests.", "kpis": k, "figures": mfigs, "tables": tables}));
        }
    }

    let title = opt
        .title
        .clone()
        .unwrap_or_else(|| format!("STDF report – {}", ov.lots.join(", ")));
    let subtitle = format!(
        "{} · {} parts · {} tests · yield {:.2}% · generated {}",
        ov.files.join(", "),
        ov.n_parts,
        ov.n_tests,
        ov.yield_pct,
        chrono::Local::now().format("%Y-%m-%d %H:%M")
    );
    let data = json!({"title": title, "subtitle": subtitle, "sections": sections});
    let data_s = serde_json::to_string(&data)?.replace("</", "<\\/");
    let plotly = if opt.offline {
        format!("<script>{}</script>", crate::server::PLOTLY_JS)
    } else {
        "<script src=\"https://cdnjs.cloudflare.com/ajax/libs/plotly.js/2.35.2/plotly.min.js\"></script>".to_string()
    };
    Ok(format!(
        "<!doctype html>\n<html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"><title>{}</title><style>{}</style>{}</head><body><header class=\"report-header\" id=\"head\"></header><div class=\"report-body\" id=\"body\"></div><script type=\"application/json\" id=\"report-data\">{}</script><script>{}</script><script>{}</script></body></html>",
        html_escape(&title),
        crate::server::STYLE_CSS,
        plotly,
        data_s,
        crate::server::COMMON_JS,
        REPORT_JS
    ))
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}
