//! Datalog of picked parts: every chosen test's result for a list of parts,
//! as readable text (one block per part) or CSV (one row per part, or one
//! row per result). Values and limits are in each test's display unit
//! (logged UNITS scaled by RES_SCAL, as the tester shows them). A test the
//! file or a part does not have prints as N/A; each part may carry a mark
//! (e.g. the label written on the physical unit).

use crate::analysis::candidates::display_unit;
use crate::data::Dataset;

/// One column of the datalog: a test of the file, or a requested test the
/// file does not have (printed as N/A).
#[derive(Debug, Clone, PartialEq)]
pub enum Col {
    Test(usize),
    Missing(String),
}

/// Tests from a list such as `1013, 2000`, `1000-1050` or names; a number
/// takes every test with it (all pins of an MPR). Blank = every test.
pub fn resolve_tests(ds: &Dataset, q: &str) -> Vec<Col> {
    let q = q.trim();
    if q.is_empty() {
        return (0..ds.tests.len()).map(Col::Test).collect();
    }
    let mut out: Vec<Col> = vec![];
    for tok in q
        .split([',', ';', '\n'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let head = tok.split(':').next().unwrap_or(tok).trim();
        let range = head.split_once('-').and_then(|(a, b)| {
            Some((a.trim().parse::<u32>().ok()?, b.trim().parse::<u32>().ok()?))
        });
        let hits: Vec<usize> = if let Some(t) = ds.test_index.get(tok) {
            vec![*t]
        } else if let Ok(n) = head.parse::<u32>() {
            (0..ds.tests.len())
                .filter(|&t| ds.tests[t].num == n)
                .collect()
        } else if let Some((a, b)) = range {
            (0..ds.tests.len())
                .filter(|&t| (a..=b).contains(&ds.tests[t].num))
                .collect()
        } else {
            ds.find_tests(tok)
        };
        if hits.is_empty() {
            out.push(Col::Missing(head.to_string()));
        }
        for t in hits {
            if !out.contains(&Col::Test(t)) {
                out.push(Col::Test(t));
            }
        }
    }
    out
}

const NA: &str = "N/A";

/// One result: value (display unit), low, high, result.
fn cell(ds: &Dataset, c: &Col, r: usize) -> [String; 4] {
    let Col::Test(t) = *c else {
        return [NA.into(), String::new(), String::new(), NA.into()];
    };
    let ti = &ds.tests[t];
    let f = display_unit(&ti.units, ti.res_scal).1;
    let v = ds.value(t, r);
    let fail = ds.parts[r].has_fail(t) || (v.is_finite() && !ti.within_limits(v));
    let result = if fail {
        "FAIL"
    } else if v.is_finite() {
        "pass"
    } else {
        NA
    };
    let value = if v.is_finite() {
        num(Some(v * f))
    } else {
        NA.into()
    };
    [
        value,
        num(ti.lo.map(|x| x * f)),
        num(ti.hi.map(|x| x * f)),
        result.into(),
    ]
}

/// Test number, name and display unit of a column.
fn head(ds: &Dataset, c: &Col) -> (String, String, String) {
    match c {
        Col::Test(t) => {
            let ti = &ds.tests[*t];
            let name = ti.key.split_once(':').map(|x| x.1).unwrap_or(&ti.name);
            (
                ti.num.to_string(),
                name.to_string(),
                display_unit(&ti.units, ti.res_scal).0,
            )
        }
        Col::Missing(q) => (q.clone(), "(not in this file)".into(), String::new()),
    }
}

/// 7 significant digits (results are logged as 4-byte floats), no trailing zeros.
fn num(v: Option<f64>) -> String {
    match v {
        Some(0.0) => "0".into(),
        Some(v) if v.is_finite() => {
            let d = (6 - v.abs().log10().floor() as i32).clamp(0, 15) as usize;
            let s = format!("{v:.d$}");
            if s.contains('.') {
                s.trim_end_matches('0').trim_end_matches('.').to_string()
            } else {
                s
            }
        }
        _ => String::new(),
    }
}

fn csv_line(cols: &[String]) -> String {
    let f = |s: &String| {
        if s.contains([',', '"', '\n']) {
            format!("\"{}\"", s.replace('"', "\"\""))
        } else {
            s.clone()
        }
    };
    cols.iter().map(f).collect::<Vec<_>>().join(",") + "\n"
}

const PART_HDR: [&str; 10] = [
    "mark",
    "part_id",
    "part_txt",
    "wafer",
    "x",
    "y",
    "site",
    "hbin",
    "sbin",
    "part_result",
];

/// `format`: `txt`, `csv` (one row per part, a value and a result column per
/// test) or `csv_long` (one row per part and test). `marks[i]` labels
/// `rows[i]` (blank = none).
pub fn build(ds: &Dataset, rows: &[usize], marks: &[String], cols: &[Col], format: &str) -> String {
    let mark = |i: usize| {
        marks
            .get(i)
            .map(|m| m.trim().to_string())
            .unwrap_or_default()
    };
    let part_cols = |i: usize, r: usize| {
        let p = &ds.parts[r];
        let xy = |v: i16| {
            if v == -32768 {
                String::new()
            } else {
                v.to_string()
            }
        };
        vec![
            mark(i),
            p.part_id.clone(),
            p.part_txt.clone(),
            ds.wafer_name(p).to_string(),
            xy(p.x),
            xy(p.y),
            p.site.to_string(),
            p.hbin.to_string(),
            p.sbin.to_string(),
            if p.is_pass() { "pass" } else { "FAIL" }.to_string(),
        ]
    };
    let mut out = String::new();
    match format {
        "csv" => {
            let mut hdr: Vec<String> = PART_HDR.iter().map(|s| s.to_string()).collect();
            for c in cols {
                let (n, name, u) = head(ds, c);
                let k = format!("{n}:{name}");
                hdr.push(if u.is_empty() {
                    k.clone()
                } else {
                    format!("{k} [{u}]")
                });
                hdr.push(format!("{k} result"));
            }
            out += &csv_line(&hdr);
            // the limits as the first two lines under the header
            for (name, which) in [("LO limit", 1), ("HI limit", 2)] {
                let mut row = vec![name.to_string()];
                row.resize(PART_HDR.len(), String::new());
                for c in cols {
                    let lim = match (c, rows.first()) {
                        (Col::Test(_), Some(&r)) => cell(ds, c, r)[which].clone(),
                        _ => String::new(),
                    };
                    row.extend([lim, String::new()]);
                }
                out += &csv_line(&row);
            }
            for (i, &r) in rows.iter().enumerate() {
                let mut row = part_cols(i, r);
                for c in cols {
                    let [v, _, _, res] = cell(ds, c, r);
                    row.extend([v, res]);
                }
                out += &csv_line(&row);
            }
        }
        "csv_long" => {
            let hdr: Vec<String> = PART_HDR
                .iter()
                .chain(&[
                    "test_num",
                    "test_name",
                    "value",
                    "units",
                    "lo",
                    "hi",
                    "result",
                ])
                .map(|s| s.to_string())
                .collect();
            out += &csv_line(&hdr);
            for (i, &r) in rows.iter().enumerate() {
                let pc = part_cols(i, r);
                for c in cols {
                    let (n, name, u) = head(ds, c);
                    let [v, lo, hi, res] = cell(ds, c, r);
                    let mut row = pc.clone();
                    row.extend([n, name, v, u, lo, hi, res]);
                    out += &csv_line(&row);
                }
            }
        }
        _ => {
            out += &format!("STDF Cypher part datalog\nFile:  {}\n", ds.name);
            if let Some(s) = ds.sources.first() {
                out += &format!(
                    "Lot:   {}   step {}   program {} {}   tester {}\n",
                    s.lot_id, s.test_cod, s.job_nam, s.job_rev, s.node_nam
                );
            }
            out += &format!("Parts: {}   Tests: {}\n", rows.len(), cols.len());
            let missing: Vec<String> = cols
                .iter()
                .filter_map(|c| match c {
                    Col::Missing(q) => Some(q.clone()),
                    _ => None,
                })
                .collect();
            if !missing.is_empty() {
                out += &format!("Not in this file (N/A): {}\n", missing.join(", "));
            }
            // column widths from the content
            let rows_txt: Vec<Vec<Vec<String>>> = rows
                .iter()
                .map(|&r| {
                    cols.iter()
                        .map(|c| {
                            let (n, name, u) = head(ds, c);
                            let [v, lo, hi, res] = cell(ds, c, r);
                            vec![n, name, v, u, lo, hi, res]
                        })
                        .collect()
                })
                .collect();
            let hdr = ["Test", "Name", "Value", "Units", "Low", "High", "Result"];
            let mut w: Vec<usize> = hdr.iter().map(|h| h.len()).collect();
            for p in &rows_txt {
                for line in p {
                    for (k, s) in line.iter().enumerate() {
                        w[k] = w[k].max(s.chars().count());
                    }
                }
            }
            let fmt_line = |cols: &[String]| -> String {
                let mut s = String::from("  ");
                for (k, c) in cols.iter().enumerate() {
                    // numbers right-aligned, text left-aligned
                    if (2..=5).contains(&k) {
                        s += &format!("{:>w$}  ", c, w = w[k]);
                    } else {
                        s += &format!("{:<w$}  ", c, w = w[k]);
                    }
                }
                s.trim_end().to_string() + "\n"
            };
            let rule = "=".repeat(w.iter().sum::<usize>() + 2 * w.len() + 2);
            for (pi, &r) in rows.iter().enumerate() {
                let p = &ds.parts[r];
                out += &format!("\n{rule}\n");
                let m = mark(pi);
                let mut id = vec![];
                if !m.is_empty() {
                    id.push(format!("Mark {m}"));
                }
                id.push(format!("Part {}", p.part_id));
                if !p.part_txt.is_empty() {
                    id.push(p.part_txt.clone());
                }
                if p.x != -32768 {
                    id.push(format!("wafer {}  X {}  Y {}", ds.wafer_name(p), p.x, p.y));
                }
                id.push(format!("site {}", p.site));
                id.push(format!("hbin {}  sbin {}", p.hbin, p.sbin));
                let ff = p
                    .first_fail
                    .map(|f| format!(" (first fail {})", ds.tests[f as usize].key))
                    .unwrap_or_default();
                id.push(if p.is_pass() {
                    "PASS".to_string()
                } else {
                    format!("FAIL{ff}")
                });
                out += &id.join("  |  ");
                out += &format!("\n{rule}\n");
                out += &fmt_line(&hdr.iter().map(|s| s.to_string()).collect::<Vec<_>>());
                out += &fmt_line(&w.iter().map(|&n| "-".repeat(n)).collect::<Vec<_>>());
                for line in &rows_txt[pi] {
                    out += &fmt_line(line);
                }
            }
        }
    }
    out
}

/// Die position from tests (wafer label, X, Y) for a row, when the file
/// logs it as results (see the Settings page).
pub type CustomXy<'a> = &'a dyn Fn(usize) -> Option<(String, i16, i16)>;

/// Rows of `ds` matching a search in any of `modes`: `part_id` (exact, or
/// contained when it is the only mode), `part_txt` (contained), `xy` (PRR
/// die X/Y) and `test_xy` (`custom`). X/Y takes "3,-2", "3 -2", "X3Y-2" or
/// "W01 3,-2" (wafer text first).
pub fn find_parts(
    ds: &Dataset,
    q: &str,
    modes: &[&str],
    custom: Option<CustomXy>,
    limit: usize,
) -> Vec<usize> {
    let q = q.trim();
    if q.is_empty() {
        return vec![];
    }
    let ql = q.to_lowercase();
    let xy = parse_xy(q);
    let on = |m: &str| modes.contains(&m);
    let only_id = modes == ["part_id"];
    // "W01" finds wafer "DEMO-W01", and wafer 1 read from a test
    let tail = |s: &str| -> Option<u64> {
        let d: String = s.chars().rev().take_while(|c| c.is_ascii_digit()).collect();
        d.chars().rev().collect::<String>().parse().ok()
    };
    let wafer_ok = |w: &Option<String>, name: &str| {
        w.as_ref().is_none_or(|w| {
            name.to_lowercase().contains(w) || tail(w).is_some() && tail(w) == tail(name)
        })
    };
    let mut out = vec![];
    for (r, p) in ds.parts.iter().enumerate() {
        let id = || {
            let pid = p.part_id.to_lowercase();
            pid == ql || only_id && pid.contains(&ql)
        };
        let txt = || p.part_txt.to_lowercase().contains(&ql);
        let at = || {
            xy.as_ref()
                .is_some_and(|(w, x, y)| p.x == *x && p.y == *y && wafer_ok(w, ds.wafer_name(p)))
        };
        let at_tests = || match (&xy, custom) {
            (Some((w, x, y)), Some(f)) => {
                f(r).is_some_and(|(cw, cx, cy)| cx == *x && cy == *y && wafer_ok(w, &cw))
            }
            _ => false,
        };
        let hit = (on("part_id") && id())
            || (on("part_txt") && txt())
            || (on("xy") && at())
            || (on("test_xy") && at_tests());
        if hit {
            out.push(r);
            if out.len() >= limit {
                break;
            }
        }
    }
    out
}

/// (wafer text, x, y) from "3,-2", "3 -2", "X3Y-2", "x=3 y=-2" or "W01 3,-2".
fn parse_xy(q: &str) -> Option<(Option<String>, i16, i16)> {
    let l = q.to_lowercase();
    let nums: Vec<(usize, i16)> = {
        let b = l.as_bytes();
        let mut v = vec![];
        let mut i = 0;
        while i < b.len() {
            let neg = b[i] == b'-' && i + 1 < b.len() && b[i + 1].is_ascii_digit();
            if b[i].is_ascii_digit() || neg {
                let s = i;
                i += 1;
                while i < b.len() && b[i].is_ascii_digit() {
                    i += 1;
                }
                if let Ok(n) = l[s..i].parse() {
                    v.push((s, n));
                }
            } else {
                i += 1;
            }
        }
        v
    };
    if nums.len() < 2 {
        return None;
    }
    let (sx, x) = nums[nums.len() - 2];
    let y = nums[nums.len() - 1].1;
    // whatever comes before X (minus an "x" / "x=") names the wafer
    let w = l[..sx].trim_end_matches(['x', '=', ' ', ',', ':']).trim();
    Some(((!w.is_empty()).then(|| w.to_string()), x, y))
}

#[cfg(test)]
mod tests {
    #[test]
    fn xy_forms() {
        for q in ["3,-2", "3 -2", "X3Y-2", "x=3 y=-2"] {
            assert_eq!(super::parse_xy(q), Some((None, 3, -2)), "{q}");
        }
        assert_eq!(
            super::parse_xy("W01 3,-2"),
            Some((Some("w01".into()), 3, -2))
        );
        assert_eq!(super::parse_xy("12"), None);
    }

    #[test]
    fn seven_digits() {
        let n = |v: f64| super::num(Some(v));
        assert_eq!(n(48.343399048), "48.3434");
        assert_eq!(n(0.000012345678), "0.00001234568");
        assert_eq!(n(1234567.8), "1234568");
        assert_eq!(n(-2.5), "-2.5");
        assert_eq!(n(0.0), "0");
    }
}
