//! A small expression language for selecting parts.
//!
//! ```text
//! hbin in (2..5, 9) and site != 3
//! part_id in @ids.txt            # one id per line
//! not pass or t[1000] > 0.35     # test by number
//! t["VDD_LEAK"] between 0 and 1  # test by name
//! wafer ~ "^W0[1-3]$"            # regex match
//! $match and not pass            # named part set (e.g. from matching two files)
//! fail(1013) and n_fails = 1     # parts that failed test 1013 (and only that test)
//! first_fail = 1013              # parts whose first failing test is 1013
//! ```
//!
//! Fields: part_id, file, head, site, x, y, xy, hbin, sbin, pass, fail,
//! wafer, lot, sublot, num_test, test_t, part_txt, index, retest.

use super::{Dataset, FieldValue};
use anyhow::{anyhow, bail, Result};
use regex::Regex;
use std::sync::Arc;

/// Looks up a named part set (`$name` in a filter): one flag per dataset row.
pub type SetLookup<'a> = &'a dyn Fn(&str) -> Option<Arc<Vec<bool>>>;

#[derive(Debug, Clone)]
enum Tok {
    Ident(String),
    Num(f64, String),
    Str(String),
    Sym(String),
    At(String),
    Set(String),
}

fn tokenize(s: &str) -> Result<Vec<Tok>> {
    let c: Vec<char> = s.chars().collect();
    let mut i = 0;
    let mut out = vec![];
    while i < c.len() {
        let ch = c[i];
        if ch.is_whitespace() {
            i += 1;
        } else if ch == '"' || ch == '\'' {
            let q = ch;
            i += 1;
            let st = i;
            while i < c.len() && c[i] != q {
                i += 1;
            }
            if i >= c.len() {
                bail!("unterminated string in filter");
            }
            out.push(Tok::Str(c[st..i].iter().collect()));
            i += 1;
        } else if ch == '@' {
            i += 1;
            let st = i;
            while i < c.len() && !c[i].is_whitespace() && c[i] != ')' {
                i += 1;
            }
            out.push(Tok::At(c[st..i].iter().collect()));
        } else if ch == '$' {
            i += 1;
            let st = i;
            while i < c.len() && (c[i].is_alphanumeric() || c[i] == '_' || c[i] == '-') {
                i += 1;
            }
            if st == i {
                bail!("expected a set name after '$'");
            }
            out.push(Tok::Set(c[st..i].iter().collect()));
        } else if ch.is_ascii_digit()
            || (ch == '-' && i + 1 < c.len() && (c[i + 1].is_ascii_digit() || c[i + 1] == '.'))
            || (ch == '.' && i + 1 < c.len() && c[i + 1].is_ascii_digit())
        {
            let st = i;
            i += 1;
            while i < c.len() {
                let d = c[i];
                let range_dots = d == '.' && i + 1 < c.len() && c[i + 1] == '.';
                if range_dots {
                    break;
                }
                if d.is_ascii_alphanumeric()
                    || d == '.'
                    || ((d == '-' || d == '+') && (c[i - 1] == 'e' || c[i - 1] == 'E'))
                    || d == '_'
                {
                    i += 1;
                } else {
                    break;
                }
            }
            let txt: String = c[st..i].iter().collect();
            match txt.parse::<f64>() {
                Ok(v) => out.push(Tok::Num(v, txt)),
                Err(_) => out.push(Tok::Ident(txt)),
            }
        } else if ch.is_alphabetic() || ch == '_' {
            let st = i;
            while i < c.len() && (c[i].is_alphanumeric() || c[i] == '_') {
                i += 1;
            }
            out.push(Tok::Ident(c[st..i].iter().collect()));
        } else {
            let two: String = c[i..(i + 2).min(c.len())].iter().collect();
            if ["==", "!=", "<=", ">=", "..", "!~", "&&", "||"].contains(&two.as_str()) {
                out.push(Tok::Sym(two));
                i += 2;
            } else if "=<>()[],~!".contains(ch) {
                out.push(Tok::Sym(ch.to_string()));
                i += 1;
            } else {
                bail!("unexpected character '{ch}' in filter");
            }
        }
    }
    Ok(out)
}

#[derive(Debug, Clone)]
pub enum Operand {
    Field(String),
    Test(usize),
}

#[derive(Debug, Clone)]
pub enum Item {
    Val(String, Option<f64>),
    Range(f64, f64),
}

#[derive(Debug, Clone)]
pub enum Expr {
    And(Box<Expr>, Box<Expr>),
    Or(Box<Expr>, Box<Expr>),
    Not(Box<Expr>),
    Truthy(Operand),
    Cmp(Operand, String, String, Option<f64>),
    In(Operand, Vec<Item>),
    Regex(Operand, Regex, bool),
    Const(bool),
    Set(Arc<Vec<bool>>),
    /// part failed any of these tests (tester flag or outside limits)
    Fails(Vec<usize>),
}

/// Compiled filter bound to a dataset's test table.
#[derive(Debug, Clone)]
pub struct Filter {
    expr: Expr,
    pub source: String,
}

struct Parser<'a> {
    t: Vec<Tok>,
    i: usize,
    ds: &'a Dataset,
    sets: SetLookup<'a>,
}

impl<'a> Parser<'a> {
    fn peek(&self) -> Option<&Tok> {
        self.t.get(self.i)
    }
    fn next(&mut self) -> Option<Tok> {
        let t = self.t.get(self.i).cloned();
        self.i += 1;
        t
    }
    fn is_kw(&self, kw: &str) -> bool {
        matches!(self.peek(), Some(Tok::Ident(s)) if s.eq_ignore_ascii_case(kw))
    }
    fn is_sym(&self, s: &str) -> bool {
        matches!(self.peek(), Some(Tok::Sym(x)) if x == s)
    }
    fn expect_sym(&mut self, s: &str) -> Result<()> {
        if self.is_sym(s) {
            self.i += 1;
            Ok(())
        } else {
            bail!("expected '{s}' in filter near token {}", self.i)
        }
    }
    fn or(&mut self) -> Result<Expr> {
        let mut l = self.and()?;
        while self.is_kw("or") || self.is_sym("||") {
            self.i += 1;
            let r = self.and()?;
            l = Expr::Or(Box::new(l), Box::new(r));
        }
        Ok(l)
    }
    fn and(&mut self) -> Result<Expr> {
        let mut l = self.not()?;
        while self.is_kw("and") || self.is_sym("&&") {
            self.i += 1;
            let r = self.not()?;
            l = Expr::And(Box::new(l), Box::new(r));
        }
        Ok(l)
    }
    fn not(&mut self) -> Result<Expr> {
        if self.is_kw("not") || self.is_sym("!") {
            self.i += 1;
            return Ok(Expr::Not(Box::new(self.not()?)));
        }
        self.primary()
    }
    fn operand(&mut self) -> Result<Operand> {
        match self.next() {
            Some(Tok::Ident(id)) => {
                if (id == "t" || id == "test") && self.is_sym("[") {
                    self.i += 1;
                    let q = match self.next() {
                        Some(Tok::Num(_, s)) | Some(Tok::Str(s)) | Some(Tok::Ident(s)) => s,
                        _ => bail!("expected test number or name inside t[...]"),
                    };
                    self.expect_sym("]")?;
                    let t = self
                        .ds
                        .find_test(&q)
                        .ok_or_else(|| anyhow!("unknown or ambiguous test '{q}' in filter"))?;
                    return Ok(Operand::Test(t));
                }
                let f = id.to_lowercase();
                const ALIASES: &[&str] = &[
                    "partid", "id", "hard_bin", "soft_bin", "passed", "failed", "fail", "wafer_id",
                    "lot_id", "row", "sublot",
                ];
                if !super::PART_FIELDS.contains(&f.as_str()) && !ALIASES.contains(&f.as_str()) {
                    bail!(
                        "unknown field '{id}'. Known: {}",
                        super::PART_FIELDS.join(", ")
                    );
                }
                Ok(Operand::Field(f))
            }
            other => bail!("expected field or t[..], got {:?}", other),
        }
    }
    fn value(&mut self) -> Result<(String, Option<f64>)> {
        match self.next() {
            Some(Tok::Num(v, s)) => Ok((s, Some(v))),
            Some(Tok::Str(s)) => {
                let n = s.parse::<f64>().ok();
                Ok((s, n))
            }
            Some(Tok::Ident(s)) => {
                let n = match s.to_lowercase().as_str() {
                    "true" | "pass" => Some(1.0),
                    "false" | "fail" => Some(0.0),
                    _ => None,
                };
                Ok((s, n))
            }
            other => bail!("expected value, got {:?}", other),
        }
    }
    fn list(&mut self) -> Result<Vec<Item>> {
        if let Some(Tok::At(path)) = self.peek().cloned() {
            self.i += 1;
            let txt = std::fs::read_to_string(&path)
                .map_err(|e| anyhow!("cannot read list file {path}: {e}"))?;
            return Ok(txt
                .split(['\n', ',', '\r', '\t'])
                .map(|s| s.trim())
                .filter(|s| !s.is_empty() && !s.starts_with('#'))
                .map(|s| Item::Val(s.to_string(), s.parse().ok()))
                .collect());
        }
        let paren = self.is_sym("(") || self.is_sym("[");
        if paren {
            self.i += 1;
        }
        let mut items = vec![];
        loop {
            let (s, n) = self.value()?;
            if self.is_sym("..") {
                self.i += 1;
                let (_, n2) = self.value()?;
                let (a, b) = (
                    n.ok_or_else(|| anyhow!("range needs numbers"))?,
                    n2.ok_or_else(|| anyhow!("range needs numbers"))?,
                );
                items.push(Item::Range(a, b));
            } else {
                items.push(Item::Val(s, n));
            }
            if self.is_sym(",") {
                self.i += 1;
                continue;
            }
            break;
        }
        if paren {
            if self.is_sym(")") || self.is_sym("]") {
                self.i += 1;
            } else {
                bail!("expected ')' to close list");
            }
        }
        Ok(items)
    }
    fn primary(&mut self) -> Result<Expr> {
        if self.is_sym("(") {
            self.i += 1;
            let e = self.or()?;
            self.expect_sym(")")?;
            return Ok(e);
        }
        if self.is_kw("true") || self.is_kw("all") {
            self.i += 1;
            return Ok(Expr::Const(true));
        }
        if self.is_kw("false") || self.is_kw("none") {
            self.i += 1;
            return Ok(Expr::Const(false));
        }
        if (self.is_kw("fail") || self.is_kw("fails") || self.is_kw("failed"))
            && matches!(self.t.get(self.i + 1), Some(Tok::Sym(s)) if s == "(")
        {
            self.i += 2;
            let mut tests = vec![];
            loop {
                let q = match self.next() {
                    Some(Tok::Num(_, s)) | Some(Tok::Str(s)) | Some(Tok::Ident(s)) => s,
                    other => bail!("expected a test number or name in fail(...), got {other:?}"),
                };
                let hits: Vec<usize> = match q.parse::<u32>() {
                    // every test with that number (MPR pins share it)
                    Ok(n) => (0..self.ds.n_tests())
                        .filter(|&t| self.ds.tests[t].num == n)
                        .collect(),
                    Err(_) => self.ds.find_test(&q).into_iter().collect(),
                };
                if hits.is_empty() {
                    bail!("unknown test '{q}' in fail(...)");
                }
                tests.extend(hits);
                if self.is_sym(",") {
                    self.i += 1;
                    continue;
                }
                break;
            }
            self.expect_sym(")")?;
            return Ok(Expr::Fails(tests));
        }
        if let Some(Tok::Set(name)) = self.peek().cloned() {
            self.i += 1;
            let set = (self.sets)(&name).ok_or_else(|| {
                anyhow!("unknown part set '${name}' (sets are created by matching two files)")
            })?;
            if set.len() != self.ds.n_parts() {
                bail!("part set '${name}' belongs to another dataset");
            }
            return Ok(Expr::Set(set));
        }
        let op = self.operand()?;
        // operators
        if self.is_kw("in") {
            self.i += 1;
            return Ok(Expr::In(op, self.list()?));
        }
        if self.is_kw("not") {
            let save = self.i;
            self.i += 1;
            if self.is_kw("in") {
                self.i += 1;
                return Ok(Expr::Not(Box::new(Expr::In(op, self.list()?))));
            }
            self.i = save;
        }
        if self.is_kw("between") {
            self.i += 1;
            let (_, a) = self.value()?;
            if !self.is_kw("and") {
                bail!("expected 'and' in between");
            }
            self.i += 1;
            let (_, b) = self.value()?;
            return Ok(Expr::In(
                op,
                vec![Item::Range(a.unwrap_or(f64::NAN), b.unwrap_or(f64::NAN))],
            ));
        }
        if self.is_sym("~") || self.is_sym("!~") {
            let neg = self.is_sym("!~");
            self.i += 1;
            let (pat, _) = self.value()?;
            return Ok(Expr::Regex(op, Regex::new(&pat)?, neg));
        }
        for s in ["==", "=", "!=", "<=", ">=", "<", ">"] {
            if self.is_sym(s) {
                self.i += 1;
                let (v, n) = self.value()?;
                let s = if s == "==" { "=" } else { s };
                return Ok(Expr::Cmp(op, s.to_string(), v, n));
            }
        }
        Ok(Expr::Truthy(op))
    }
}

impl Filter {
    pub fn parse(src: &str, ds: &Dataset) -> Result<Filter> {
        Self::parse_with(src, ds, &|_| None)
    }

    /// Parse with access to named part sets (`$name`).
    pub fn parse_with(src: &str, ds: &Dataset, sets: SetLookup) -> Result<Filter> {
        let src_t = src.trim();
        if src_t.is_empty() {
            return Ok(Filter {
                expr: Expr::Const(true),
                source: String::new(),
            });
        }
        let toks = tokenize(src_t)?;
        let mut p = Parser {
            t: toks,
            i: 0,
            ds,
            sets,
        };
        let expr = p.or()?;
        if p.i < p.t.len() {
            bail!("unexpected trailing input in filter: {:?}", &p.t[p.i..]);
        }
        Ok(Filter {
            expr,
            source: src_t.to_string(),
        })
    }

    pub fn matches(&self, ds: &Dataset, row: usize) -> bool {
        eval(&self.expr, ds, row)
    }

    pub fn select(&self, ds: &Dataset, rows: &[usize]) -> Vec<usize> {
        rows.iter()
            .copied()
            .filter(|&r| self.matches(ds, r))
            .collect()
    }

    pub fn select_all(&self, ds: &Dataset) -> Vec<usize> {
        (0..ds.parts.len())
            .filter(|&r| self.matches(ds, r))
            .collect()
    }
}

fn operand_value(op: &Operand, ds: &Dataset, row: usize) -> FieldValue {
    match op {
        Operand::Field(f) => ds.field(row, f),
        Operand::Test(t) => {
            let v = ds.value(*t, row);
            if v.is_finite() {
                FieldValue::Num(v)
            } else {
                FieldValue::Missing
            }
        }
    }
}

fn eval(e: &Expr, ds: &Dataset, row: usize) -> bool {
    match e {
        Expr::Const(b) => *b,
        Expr::Set(s) => s.get(row).copied().unwrap_or(false),
        Expr::Fails(ts) => ts.iter().any(|&t| {
            let v = ds.value(t, row);
            ds.parts[row].has_fail(t) || (v.is_finite() && !ds.tests[t].within_limits(v))
        }),
        Expr::And(a, b) => eval(a, ds, row) && eval(b, ds, row),
        Expr::Or(a, b) => eval(a, ds, row) || eval(b, ds, row),
        Expr::Not(a) => !eval(a, ds, row),
        Expr::Truthy(op) => match operand_value(op, ds, row) {
            FieldValue::Bool(b) => b,
            FieldValue::Num(v) => v != 0.0,
            FieldValue::Str(s) => !s.is_empty(),
            FieldValue::Missing => false,
        },
        Expr::Cmp(op, sym, sv, nv) => {
            let v = operand_value(op, ds, row);
            let ord = match (&v, nv) {
                (FieldValue::Num(a), Some(b)) => a.partial_cmp(b),
                (FieldValue::Bool(a), Some(b)) => (*a as u8 as f64).partial_cmp(b),
                (FieldValue::Str(a), _) => {
                    // numeric compare if both parse, else lexical
                    match (a.parse::<f64>(), nv) {
                        (Ok(x), Some(y)) => x.partial_cmp(y),
                        _ => Some(a.as_str().cmp(sv.as_str())),
                    }
                }
                (FieldValue::Missing, _) => return sym == "!=",
                _ => None,
            };
            let Some(o) = ord else { return false };
            use std::cmp::Ordering::*;
            match sym.as_str() {
                "=" => o == Equal,
                "!=" => o != Equal,
                "<" => o == Less,
                "<=" => o != Greater,
                ">" => o == Greater,
                ">=" => o != Less,
                _ => false,
            }
        }
        Expr::In(op, items) => {
            let v = operand_value(op, ds, row);
            items.iter().any(|it| match (it, &v) {
                (Item::Range(a, b), FieldValue::Num(x)) => x >= a && x <= b,
                (Item::Range(a, b), FieldValue::Str(s)) => s
                    .parse::<f64>()
                    .map(|x| x >= *a && x <= *b)
                    .unwrap_or(false),
                (Item::Val(_, Some(n)), FieldValue::Num(x)) => x == n,
                (Item::Val(_, Some(n)), FieldValue::Bool(b)) => (*b as u8 as f64) == *n,
                (Item::Val(s, _), FieldValue::Str(x)) => x == s,
                _ => false,
            })
        }
        Expr::Regex(op, re, neg) => {
            let s = match operand_value(op, ds, row) {
                FieldValue::Str(s) => s,
                FieldValue::Missing => return *neg,
                other => other.as_group_key(),
            };
            re.is_match(&s) != *neg
        }
    }
}
