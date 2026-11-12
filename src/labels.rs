//! How wafer, X, Y and a test are written for people: one setting each (Settings, or the command
//! line), so every page, report and CSV writes them the same way.
//!
//! * Wafer, X, Y: a template with `{w}`, `{x}` and `{y}`, e.g. `W{w}X{x}Y{y}` (W1X118Y126, the
//!   default), `W{w}_X{x}_Y{y}`, `{w}-{x}-{y}`, or `{w}{x3}{y3}` (X and Y zero-padded to three
//!   digits: 1118126, the WXY code).
//! * A test: a template with `{num}` and `{name}`, e.g. `{num} {name}` (the default),
//!   `T#{num}_{name}` or `{num}::{name}`.
//!
//! Matching never depends on these: parts are matched on their values, and a typed value is read
//! back in the chosen form or any of the usual ones.

use anyhow::{bail, Result};
use std::sync::RwLock;

pub const WXY_DEFAULT: &str = "W{w}X{x}Y{y}";
pub const TEST_DEFAULT: &str = "{num} {name}";

static WXY: RwLock<String> = RwLock::new(String::new());
static TEST: RwLock<String> = RwLock::new(String::new());

fn get(l: &RwLock<String>, default: &str) -> String {
    let s = l.read().unwrap();
    if s.is_empty() {
        default.to_string()
    } else {
        s.clone()
    }
}

/// The wafer, X, Y template in use.
pub fn wxy_format() -> String {
    get(&WXY, WXY_DEFAULT)
}

/// The test template in use.
pub fn test_format() -> String {
    get(&TEST, TEST_DEFAULT)
}

/// Sets the templates; an empty one goes back to the default.
pub fn set_formats(wxy: &str, test: &str) -> Result<()> {
    let wxy = wxy.trim();
    let test = test.trim();
    if !wxy.is_empty() {
        let has = |a: &str, b: &str| wxy.contains(a) || wxy.contains(b);
        if !(wxy.contains("{w}") && has("{x}", "{x3}") && has("{y}", "{y3}")) {
            bail!("the wafer, X, Y label needs {{w}}, {{x}} and {{y}} (or {{x3}} and {{y3}})");
        }
        // it must read back as it was written: two fields next to each other need the second
        // one padded ({w}{x3}{y3}); {w}{x}{y} would read 1234 as 1, 23, 4 or 12, 3, 4
        let mut prev_field = false;
        let mut rest = wxy;
        while !rest.is_empty() {
            match rest.find('{') {
                Some(0) => {
                    let end = rest.find('}').unwrap_or(rest.len() - 1);
                    let tag = &rest[1..end];
                    if prev_field && !matches!(tag, "x3" | "y3") {
                        bail!("put a separator between the fields of the wafer, X, Y label, or use {{x3}} and {{y3}} (three digits) after {{w}}");
                    }
                    prev_field = true;
                    rest = &rest[end + 1..];
                }
                Some(i) => {
                    prev_field = false;
                    rest = &rest[i..];
                }
                None => break,
            }
        }
    }
    if !test.is_empty() && !test.contains("{num}") {
        bail!("the test label needs {{num}}");
    }
    *WXY.write().unwrap() = wxy.to_string();
    *TEST.write().unwrap() = test.to_string();
    Ok(())
}

/// A number zero-padded to three digits, keeping its sign: 7 → 007, -7 → -007.
fn pad3(v: &str) -> String {
    match v.strip_prefix('-') {
        Some(d) => format!("-{d:0>3}"),
        None => format!("{v:0>3}"),
    }
}

/// Wafer, X and Y as the chosen template writes them.
pub fn wxy(w: &str, x: &str, y: &str) -> String {
    wxy_with(&wxy_format(), w, x, y)
}

pub fn wxy_with(t: &str, w: &str, x: &str, y: &str) -> String {
    // a padded field holds three digits: X or Y beyond them would read back as another die, so
    // such a part is written in the default form
    let fits = |v: &str| v.trim_start_matches('-').len() <= 3;
    if (t.contains("{x3}") && !fits(x)) || (t.contains("{y3}") && !fits(y)) {
        return wxy_with(WXY_DEFAULT, w, x, y);
    }
    t.replace("{w}", w)
        .replace("{x3}", &pad3(x))
        .replace("{y3}", &pad3(y))
        .replace("{x}", x)
        .replace("{y}", y)
}

/// A test as the chosen template writes it.
pub fn test(num: u32, name: &str) -> String {
    test_with(&test_format(), num, name)
}

pub fn test_with(t: &str, num: u32, name: &str) -> String {
    t.replace("{num}", &num.to_string()).replace("{name}", name)
}

/// Wafer, X and Y read back from text in the chosen form: W1_X118_Y126, 1-118-126 (a negative X is
/// 1--2-126), or a code such as 1118126 when the template writes them without separators.
pub fn parse_wxy(s: &str) -> Option<[String; 3]> {
    parse_with(&wxy_format(), s)
}

pub fn parse_with(t: &str, s: &str) -> Option<[String; 3]> {
    let s = s.trim();
    let mut re = String::from("(?i)^");
    let mut order = vec![];
    let mut rest = t;
    while let Some(i) = rest.find('{') {
        re.push_str(&regex::escape(&rest[..i]));
        let end = rest[i..].find('}')? + i;
        let tag = &rest[i + 1..end];
        let (pat, slot) = match tag {
            "w" => (r"(-?\d+?)", 0),
            "x" => (r"(-?\d+)", 1),
            "y" => (r"(-?\d+)", 2),
            "x3" => (r"(-?\d{3})", 1),
            "y3" => (r"(-?\d{3})", 2),
            _ => return None,
        };
        re.push_str(pat);
        order.push(slot);
        rest = &rest[end + 1..];
    }
    re.push_str(&regex::escape(rest));
    re.push('$');
    let caps = regex::Regex::new(&re).ok()?.captures(s)?;
    let mut out: [String; 3] = Default::default();
    for (i, &slot) in order.iter().enumerate() {
        let v: i64 = caps.get(i + 1)?.as_str().parse().ok()?;
        out[slot] = v.to_string();
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn templates_write_and_read_back() {
        for (t, text) in [
            ("W{w}X{x}Y{y}", "W1X118Y126"),
            ("W{w}_X{x}_Y{y}", "W1_X118_Y126"),
            ("{w}-{x}-{y}", "1-118-126"),
            ("{w}{x3}{y3}", "1118126"),
        ] {
            assert_eq!(wxy_with(t, "1", "118", "126"), text);
            assert_eq!(parse_with(t, text).unwrap(), ["1", "118", "126"], "{t}");
        }
        // a negative X with a dash between: 1--2-126
        assert_eq!(wxy_with("{w}-{x}-{y}", "1", "-2", "126"), "1--2-126");
        assert_eq!(
            parse_with("{w}-{x}-{y}", "1--2-126").unwrap(),
            ["1", "-2", "126"]
        );
        // padded: X 7 is 007, and a two-digit wafer stays whole
        assert_eq!(wxy_with("{w}{x3}{y3}", "24", "7", "146"), "24007146");
        assert_eq!(
            parse_with("{w}{x3}{y3}", "24007146").unwrap(),
            ["24", "7", "146"]
        );
        assert!(parse_with("W{w}X{x}Y{y}", "W1X2").is_none());
        // beyond three digits a padded field would read back as another die: the default form
        assert_eq!(wxy_with("{w}{x3}{y3}", "1", "1000", "2"), "W1X1000Y2");
    }

    #[test]
    fn a_template_needs_its_fields() {
        // refused before anything is set, so other tests keep the defaults
        assert!(set_formats("W{w}X{x}", "").is_err());
        // fields next to each other cannot be read back unless padded
        assert!(set_formats("{w}{x}{y}", "").is_err());
        assert!(set_formats("{w}{x3}{y}", "").is_err());
        assert!(set_formats("", "T{name}").is_err());
        for (t, text) in [
            ("T#{num}_{name}", "T#72199_VDD"),
            ("P{num}-{name}", "P72199-VDD"),
            ("{num}::{name}", "72199::VDD"),
        ] {
            assert_eq!(test_with(t, 72199, "VDD"), text);
        }
    }
}
