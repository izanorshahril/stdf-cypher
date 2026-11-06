//! Instrument alarms that testers write into GDR or DTR records.
//!
//! STDF V4 has no record for an instrument alarm: a tester sets the alarm bit of a test result
//! (TEST_FLG bit 0) and some vendors add the details as datalog text (DTR) or generic data (GDR).
//! There is no standard layout, so a record counts as an alarm when its text has the word ALARM,
//! and its details are read from `KEY=value` pairs (`SITE=2 TEST=1030 INSTR=PPMU12 CODE=0x1203`)
//! or, where a key is missing, from free text (`site 2 ... pin VDD1 (code 0x2201)`).
//! An alarm with a site is tied to the part tested on that site when the record was written: the
//! first part of that site whose PRR comes after it.

use super::{DatalogEntry, Dataset};
use serde::Serialize;

/// An instrument alarm read from a GDR or DTR record.
#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct InstAlarm {
    pub head: Option<u8>,
    pub site: Option<u8>,
    pub test: Option<u32>,
    /// instrument, resource, channel or pin, as the record names it
    pub instrument: String,
    pub code: String,
    pub severity: String,
    pub message: String,
    /// the part (row) the alarm happened on, when its site is known
    pub row: Option<usize>,
}

/// Whether the text is an alarm record: it has the word ALARM (ALARM, ALARMS, INSTR_ALARM...).
fn is_alarm(text: &str) -> bool {
    let up = text.to_ascii_uppercase();
    up.match_indices("ALARM").any(|(i, _)| {
        let before = up[..i].chars().next_back();
        // "NOALARM" is not an alarm, "INST_ALARM" and "ALARM:" are
        before.is_none_or(|c| !c.is_ascii_alphanumeric() || c == '_')
    }) && !up.contains("NO ALARM")
}

/// The `KEY=value` pairs of a text. A value runs to the next key or to a `;` or `|`.
fn pairs(text: &str) -> Vec<(String, String)> {
    let b = text.as_bytes();
    let word = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    // (key start, '=' position) of every key
    let mut keys: Vec<(usize, usize)> = vec![];
    for (i, &c) in b.iter().enumerate() {
        if c != b'=' || i == 0 {
            continue;
        }
        let mut s = i;
        while s > 0 && word(b[s - 1]) {
            s -= 1;
        }
        let starts_ok = s == 0 || matches!(b[s - 1], b' ' | b'\t' | b';' | b',' | b'|' | b':' | b'(' | b'[');
        if s < i && starts_ok && !b[s].is_ascii_digit() {
            keys.push((s, i));
        }
    }
    let mut out = vec![];
    for (k, &(s, eq)) in keys.iter().enumerate() {
        let end = keys.get(k + 1).map_or(b.len(), |n| n.0);
        let raw = &text[eq + 1..end];
        let cut = raw.find([';', '|']).unwrap_or(raw.len());
        let mut v = raw[..cut].trim().trim_end_matches(',').trim();
        // a closing bracket belongs to the value only when the value opened it
        while (v.ends_with(')') && v.matches('(').count() < v.matches(')').count())
            || (v.ends_with(']') && v.matches('[').count() < v.matches(']').count())
        {
            v = v[..v.len() - 1].trim_end();
        }
        out.push((text[s..eq].to_ascii_uppercase(), v.trim_matches('"').to_string()));
    }
    out
}

/// The value after a word in free text: `site 2`, `site#2`, `site2`, `pin VDD1`.
fn after_word(text: &str, names: &[&str]) -> Option<String> {
    let toks: Vec<&str> = text
        .split(|c: char| c.is_whitespace() || matches!(c, ',' | ';' | '(' | ')' | '[' | ']' | '|'))
        .filter(|t| !t.is_empty())
        .collect();
    for (i, t) in toks.iter().enumerate() {
        let low = t.to_ascii_lowercase();
        let low = low.trim_end_matches(':');
        for n in names {
            if low == *n {
                if let Some(v) = toks.get(i + 1) {
                    let v = v.trim_start_matches(['#', ':', '=']).trim_end_matches([':', '.']);
                    // "INSTRUMENT ALARM:" names no instrument
                    if !v.is_empty() && !v.to_ascii_uppercase().contains("ALARM") {
                        return Some(v.to_string());
                    }
                }
            } else if let Some(rest) = low.strip_prefix(n) {
                // site2, site#2, site:2
                let rest = rest.trim_start_matches(['#', ':', '=']);
                if !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit()) {
                    return Some(rest.to_string());
                }
            }
        }
    }
    None
}

fn number<T: std::str::FromStr>(v: &str) -> Option<T> {
    v.trim().trim_start_matches('#').parse().ok()
}

/// Reads an alarm from the text of a GDR or DTR record, or None when it is not one.
pub fn parse(text: &str) -> Option<InstAlarm> {
    if !is_alarm(text) {
        return None;
    }
    let mut a = InstAlarm::default();
    let mut instr = vec![];
    for (k, v) in pairs(text) {
        if v.is_empty() {
            continue;
        }
        match k.as_str() {
            "SITE" | "SITE_NUM" | "SITE_NO" => a.site = a.site.or(number(&v)),
            "HEAD" | "HEAD_NUM" => a.head = a.head.or(number(&v)),
            "TEST" | "TEST_NUM" | "TNUM" | "TEST_NO" => a.test = a.test.or(number(&v)),
            "INSTR" | "INSTRUMENT" | "RESOURCE" | "BOARD" | "SLOT" | "MODULE" | "CHANNEL" | "CH" | "PIN" => {
                instr.push(v)
            }
            "CODE" | "ALARM_ID" | "ALARM_CODE" | "ERR" | "ERROR" | "ERROR_CODE" | "ID"
                if a.code.is_empty() =>
            {
                a.code = v
            }
            "SEVERITY" | "SEV" | "LEVEL" if a.severity.is_empty() => a.severity = v.to_ascii_uppercase(),
            "MSG" | "MESSAGE" | "TEXT" | "DESC" | "DESCRIPTION" if a.message.is_empty() => a.message = v,
            _ => {}
        }
    }
    // free text for what the keys did not give
    if a.site.is_none() {
        a.site = after_word(text, &["site"]).and_then(|v| number(&v));
    }
    if a.head.is_none() {
        a.head = after_word(text, &["head"]).and_then(|v| number(&v));
    }
    if a.test.is_none() {
        a.test = after_word(text, &["test"]).and_then(|v| number(&v));
    }
    if instr.is_empty() {
        for n in [&["instrument", "instr", "resource", "board"][..], &["pin", "channel", "ch"][..]] {
            if let Some(v) = after_word(text, n) {
                instr.push(v);
            }
        }
    }
    a.instrument = instr.join(" / ");
    if a.code.is_empty() {
        a.code = after_word(text, &["code"]).unwrap_or_default();
    }
    if a.severity.is_empty() {
        let up = text.to_ascii_uppercase();
        a.severity = ["FATAL", "ERROR", "WARNING", "WARN", "INFO"]
            .iter()
            .find(|s| up.split(|c: char| !c.is_ascii_alphanumeric()).any(|w| w == **s))
            .map(|s| if *s == "WARNING" { "WARN" } else { s })
            .unwrap_or_default()
            .to_string();
    }
    if a.message.is_empty() {
        // free text: what follows the word ALARM
        let up = text.to_ascii_uppercase();
        let at = up.find("ALARM").map_or(0, |i| i + 5);
        let rest = text[at..].trim_start_matches(|c: char| c.is_ascii_alphabetic() || c == '_');
        let mut msg = rest.trim_start_matches([':', '-', ' ']).trim();
        // "on site 4, test 1010, resource DCVI6: settling timeout (code 0x13F2)": the message is
        // what follows the last ": ", without a closing "(code ...)"
        if let Some(i) = msg.rfind(": ") {
            if !msg[i + 2..].trim().is_empty() {
                msg = msg[i + 2..].trim();
            }
        }
        if let Some(i) = msg.rfind('(') {
            if msg.ends_with(')') && msg[i..].to_ascii_lowercase().contains("code") {
                msg = msg[..i].trim_end();
            }
        }
        a.message = msg.to_string();
    }
    Some(a)
}

/// How far ahead (in parts) an alarm looks for the part of its site.
const LOOK_AHEAD: usize = 4096;

/// Reads the alarms of every datalog entry of `ds`, ties each to its part and counts them on the
/// parts.
pub fn attach(ds: &mut Dataset) {
    let mut datalog = std::mem::take(&mut ds.datalog);
    for e in datalog.iter_mut() {
        e.alarm = parse(&e.text).map(|mut a| {
            a.row = a.site.and_then(|site| part_of(ds, e, site, a.head));
            a
        });
        if let Some(row) = e.alarm.as_ref().and_then(|a| a.row) {
            let p = &mut ds.parts[row];
            p.inst_alarms = p.inst_alarms.saturating_add(1);
        }
    }
    ds.datalog = datalog;
}

/// The first part of `site` (and `head`, when given) whose PRR comes after the record.
fn part_of(ds: &Dataset, e: &DatalogEntry, site: u8, head: Option<u8>) -> Option<usize> {
    let end = ds.parts.len().min(e.after_part + LOOK_AHEAD);
    (e.after_part..end).find(|&r| {
        let p = &ds.parts[r];
        p.site == site && head.is_none_or(|h| p.head == h)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_value_alarms() {
        let a = parse("ALARM SITE=2 TEST=1030 INSTR=PPMU12 CODE=0x1203 SEVERITY=warn MSG=clamp current reached").unwrap();
        assert_eq!(a.site, Some(2));
        assert_eq!(a.test, Some(1030));
        assert_eq!(a.instrument, "PPMU12");
        assert_eq!(a.code, "0x1203");
        assert_eq!(a.severity, "WARN");
        assert_eq!(a.message, "clamp current reached");
        // a GDR as stdf-cypher shows it: fields joined by "; "
        let g = parse("INST_ALARM; SITE=3; HEAD=1; INSTR=DPS3; PIN=VDD1; CODE=E22; MSG=over current").unwrap();
        assert_eq!((g.head, g.site), (Some(1), Some(3)));
        assert_eq!(g.instrument, "DPS3 / VDD1");
        assert_eq!(g.message, "over current");
    }

    #[test]
    fn free_text_alarms() {
        let a = parse("INSTRUMENT ALARM: site 3 DPS overcurrent on pin VDD1 (code 0x2201) ERROR").unwrap();
        assert_eq!(a.site, Some(3));
        assert_eq!(a.instrument, "VDD1");
        assert_eq!(a.code, "0x2201");
        assert_eq!(a.severity, "ERROR");
        assert!(a.message.starts_with("site 3 DPS overcurrent"));
        let b = parse("Alarm on site#4, test 2000: PMU clamp").unwrap();
        assert_eq!((b.site, b.test), (Some(4), Some(2000)));
        assert_eq!(b.message, "PMU clamp");
        let c = parse("WARN: instrument alarm on site 4, test 1010, resource DCVI6: settling timeout (code 0x13F2)").unwrap();
        assert_eq!((c.instrument.as_str(), c.message.as_str(), c.code.as_str()), ("DCVI6", "settling timeout", "0x13F2"));
        // a bracket the value opened stays
        let d = parse("INST_ALARM SITE=2 MSG=fail-safe trip (BG_TRIM_0)").unwrap();
        assert_eq!(d.message, "fail-safe trip (BG_TRIM_0)");
    }

    #[test]
    fn other_text_is_not_an_alarm() {
        assert!(parse("LOT START SYNTH_A").is_none());
        assert!(parse("MIR_ADD; GUI_NAME=x").is_none());
        assert!(parse("NO ALARM on site 1").is_none());
        assert!(parse("NOALARMS").is_none());
    }
}
