//! Trim a file: pick the parts to keep (or to remove) by part ID or by row, number
//! the parts that stay 1…N if asked, and write a new STDF file.
//!
//! A selection is numbers, ranges and part IDs separated by commas, semicolons or
//! line breaks, e.g. `1-3,5,2000-2012`. The numbers are matched to the PRR PART_ID
//! (`By::Id`) or to the row, counted from 1 in file order (`By::Row`). The parts
//! go through the split writer, so every kept record is copied byte-for-byte; the
//! bin and pass/fail summaries are regenerated for the parts written.

use crate::data::Dataset;
use crate::split::{self, SplitOptions};
use anyhow::{bail, Context, Result};
use flate2::write::GzEncoder;
use flate2::Compression;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;

/// How the numbers of a selection are read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum By {
    /// the PRR PART_ID
    Id,
    /// the row, 1 for the first part of the file
    Row,
}

impl By {
    pub fn parse(s: &str) -> Result<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "" | "id" | "part_id" => Ok(By::Id),
            "row" | "index" => Ok(By::Row),
            other => bail!("select parts by 'id' or 'row', not '{other}'"),
        }
    }
}

/// What a selection does with the parts it picks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// write the picked parts
    Keep,
    /// write every other part
    Remove,
}

impl Action {
    pub fn parse(s: &str) -> Result<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "" | "keep" => Ok(Action::Keep),
            "remove" | "drop" => Ok(Action::Remove),
            other => bail!("use 'keep' or 'remove', not '{other}'"),
        }
    }
}

/// One item of a selection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Item {
    Num(u64),
    /// both ends included
    Range(u64, u64),
    /// a part ID that is not a number
    Text(String),
}

/// Read a selection such as `1-3,5,2000-2012`. A token that is not a number or a
/// range of numbers is a part ID given as text (e.g. `A-12`).
pub fn parse(text: &str) -> Result<Vec<Item>> {
    let num = |s: &str| s.trim().parse::<u64>().ok();
    let mut items = vec![];
    for tok in text
        .split([',', ';', '\n', '\r', '\t'])
        .map(str::trim)
        .filter(|t| !t.is_empty())
    {
        let item = match tok
            .split_once('-')
            .and_then(|(a, b)| Some((num(a)?, num(b)?)))
        {
            Some((a, b)) if a > b => bail!("'{tok}' ends before it starts"),
            Some((a, b)) => Item::Range(a, b),
            None => match num(tok) {
                Some(n) => Item::Num(n),
                None => Item::Text(tok.to_string()),
            },
        };
        items.push(item);
    }
    if items.is_empty() {
        bail!("enter at least one part ID or number");
    }
    Ok(items)
}

/// What a selection picks, and what it did not find.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct Resolved {
    /// the rows picked, in file order
    pub rows: Vec<usize>,
    /// items that match no part (a range lists how many of its numbers are missing)
    pub missing: Vec<String>,
    /// part IDs that more than one part carries; all of those parts are picked
    pub shared: Vec<String>,
}

/// The rows a selection picks in `ds`.
pub fn resolve(ds: &Dataset, by: By, text: &str) -> Result<Resolved> {
    let items = parse(text)?;
    let mut out = Resolved::default();
    let mut rows = BTreeSet::new();
    match by {
        By::Id => {
            let mut num: BTreeMap<u64, Vec<usize>> = BTreeMap::new();
            let mut txt: HashMap<&str, Vec<usize>> = HashMap::new();
            for (r, p) in ds.parts.iter().enumerate() {
                let id = p.part_id.trim();
                match id.parse::<u64>() {
                    Ok(n) => num.entry(n).or_default().push(r),
                    Err(_) => txt.entry(id).or_default().push(r),
                }
            }
            for item in &items {
                match item {
                    Item::Num(n) => pick(&mut rows, &mut out, n.to_string(), num.get(n)),
                    Item::Text(t) => pick(&mut rows, &mut out, t.clone(), txt.get(t.as_str())),
                    Item::Range(a, b) => {
                        let (a, b) = (*a, *b);
                        let found: Vec<(&u64, &Vec<usize>)> = num.range(a..=b).collect();
                        for &(n, v) in &found {
                            pick(&mut rows, &mut out, n.to_string(), Some(v));
                        }
                        let absent = (b - a).saturating_add(1) - found.len() as u64;
                        if found.is_empty() {
                            out.missing.push(format!("{a}-{b}"));
                        } else if absent > 0 {
                            out.missing.push(format!("{a}-{b} ({absent} not found)"));
                        }
                    }
                }
            }
        }
        By::Row => {
            let n = ds.n_parts() as u64;
            for item in &items {
                match item {
                    Item::Num(k) => {
                        let k = *k;
                        if (1..=n).contains(&k) {
                            rows.insert((k - 1) as usize);
                        } else {
                            out.missing.push(k.to_string());
                        }
                    }
                    Item::Range(a, b) => {
                        let (a, b) = (*a, *b);
                        let (lo, hi) = (a.max(1), b.min(n));
                        let inside = if lo <= hi {
                            rows.extend((lo - 1) as usize..hi as usize);
                            hi - lo + 1
                        } else {
                            0
                        };
                        let outside = (b - a).saturating_add(1) - inside;
                        if outside > 0 {
                            out.missing
                                .push(format!("{a}-{b} ({outside} outside 1-{n})"));
                        }
                    }
                    Item::Text(t) => {
                        bail!("'{t}' is not a row number: enter numbers when selecting by row")
                    }
                }
            }
        }
    }
    out.rows = rows.into_iter().collect();
    out.shared.sort();
    out.shared.dedup();
    Ok(out)
}

/// Pick the rows of one item. A part ID that several parts carry picks all of them.
fn pick(rows: &mut BTreeSet<usize>, out: &mut Resolved, label: String, found: Option<&Vec<usize>>) {
    match found {
        Some(v) => {
            rows.extend(v.iter().copied());
            if v.len() > 1 {
                out.shared.push(format!("{label} ({} parts)", v.len()));
            }
        }
        None => out.missing.push(label),
    }
}

/// Spread `target` over sites that have `avail` parts each: every site takes the same
/// number (the first sites in file order take one more when it does not divide), a
/// site with fewer than that takes all it has, and the rest is shared out. The total
/// is `target`, or all the parts when there are not enough.
pub fn spread(avail: &[usize], target: usize) -> Vec<usize> {
    let mut take = vec![0usize; avail.len()];
    let total: usize = avail.iter().sum();
    let mut remaining = target.min(total);
    let mut order: Vec<usize> = (0..avail.len()).collect();
    order.sort_by_key(|&i| avail[i]);
    let k = order.len();
    for (pos, &i) in order.iter().enumerate() {
        let left = k - pos;
        let fair = remaining / left;
        if avail[i] <= fair {
            take[i] = avail[i];
            remaining -= avail[i];
        } else {
            // this site and the ones after it all have more than the fair share
            let extra = remaining - fair * left;
            for (j, &s) in order[pos..].iter().enumerate() {
                take[s] = fair + usize::from(j < extra);
            }
            break;
        }
    }
    take
}

/// The rows to write: the picked rows, or every other row of the `n` parts.
pub fn rows_to_keep(n: usize, picked: &[usize], action: Action) -> Vec<usize> {
    let set: BTreeSet<usize> = picked.iter().copied().filter(|&r| r < n).collect();
    match action {
        Action::Keep => set.into_iter().collect(),
        Action::Remove => (0..n).filter(|r| !set.contains(r)).collect(),
    }
}

/// What a trimmed file holds.
#[derive(Debug, Clone, serde::Serialize)]
pub struct TrimOutput {
    pub path: String,
    /// parts written (a part cut off before its PRR counts)
    pub kept: u64,
    pub good: u64,
    pub removed: u64,
    /// part ID of the first and last part written ("" when they have none)
    pub first_id: String,
    pub last_id: String,
}

/// Write the parts at `keep` (rows of `ds`, in file order) of `input` to `out`.
///
/// `ds` must be the dataset of `input` loaded without de-duplication, and `out`
/// must be a different file. The file is written beside `out` under a temporary
/// name and moved into place when complete, so `out` is never half written.
pub fn write(
    input: &Path,
    ds: &Dataset,
    keep: &[usize],
    renumber: bool,
    out: &Path,
) -> Result<TrimOutput> {
    let n = ds.n_parts();
    let mut keep = keep.to_vec();
    keep.sort_unstable();
    keep.dedup();
    if let Some(&r) = keep.last() {
        if r >= n {
            bail!("part {} is not in the file", r + 1);
        }
    }
    if keep.is_empty() {
        bail!("no parts are left to write: keep at least one part");
    }
    if same_file(input, out) {
        bail!(
            "the trimmed file cannot replace {}: choose another name",
            input.display()
        );
    }
    if let Some(src) = ds.sources.first() {
        let prr = src.record_counts.get("PRR").copied().unwrap_or(0);
        if prr + src.partial_parts as u64 != n as u64 {
            bail!(
                "this dataset is not the whole file ({n} parts, {prr} PRR records): it was loaded de-duplicated, or the file changed since it was opened"
            );
        }
    }
    let dir = match out.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => Path::new("."),
    };
    let base = out
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "trimmed".into());
    let stem = format!(".{base}.trim");
    let tmp = dir.join(format!("{stem}_trimmed.stdf"));
    let gz = out
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("gz"));
    let tmp_gz = dir.join(format!("{stem}_trimmed.stdf.gz"));

    let mut assign: Vec<Option<String>> = vec![None; n];
    for &r in &keep {
        assign[r] = Some("trimmed".into());
    }
    let opts = SplitOptions {
        renumber,
        strict: true,
    };
    let done = (|| -> Result<split::SplitOutput> {
        let written = split::split_with_options(input, ds, &assign, dir, Some(&stem), &opts)?;
        let o = written.into_iter().next().context("nothing was written")?;
        let moved = if gz {
            gzip(&tmp, &tmp_gz)?;
            &tmp_gz
        } else {
            &tmp
        };
        std::fs::rename(moved, out).with_context(|| format!("write {}", out.display()))?;
        Ok(o)
    })();
    // whatever was left by a failed or finished write
    let _ = std::fs::remove_file(&tmp);
    let _ = std::fs::remove_file(&tmp_gz);
    let o = done?;

    let with_id: Vec<usize> = keep
        .iter()
        .copied()
        .filter(|&r| !ds.parts[r].partial)
        .collect();
    let (first_id, last_id) = if renumber {
        match with_id.len() {
            0 => (String::new(), String::new()),
            k => ("1".to_string(), k.to_string()),
        }
    } else {
        let id = |r: usize| ds.parts[r].part_id.clone();
        (id(keep[0]), id(keep[keep.len() - 1]))
    };
    Ok(TrimOutput {
        path: out.to_string_lossy().to_string(),
        kept: o.parts,
        good: o.good,
        removed: (n as u64).saturating_sub(o.parts),
        first_id,
        last_id,
    })
}

fn gzip(src: &Path, dst: &Path) -> Result<()> {
    let mut enc = GzEncoder::new(
        BufWriter::new(File::create(dst).with_context(|| format!("create {}", dst.display()))?),
        Compression::default(),
    );
    std::io::copy(&mut File::open(src)?, &mut enc)?;
    enc.finish()?.flush()?;
    Ok(())
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(x), Ok(y)) => x == y,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selections_parse_numbers_ranges_and_text() {
        assert_eq!(
            parse("1-3,5,2000-2012").unwrap(),
            vec![Item::Range(1, 3), Item::Num(5), Item::Range(2000, 2012)]
        );
        assert_eq!(
            parse(" 7 ;\n8 - 9\t A-12 , 00042").unwrap(),
            vec![
                Item::Num(7),
                Item::Range(8, 9),
                Item::Text("A-12".into()),
                Item::Num(42)
            ]
        );
        assert!(parse("5-3").is_err(), "a range that ends before it starts");
        assert!(parse(" , ;").is_err(), "nothing to select");
    }

    #[test]
    fn rows_to_keep_inverts_the_picked_rows() {
        assert_eq!(rows_to_keep(5, &[1, 3, 9], Action::Keep), vec![1, 3]);
        assert_eq!(rows_to_keep(5, &[1, 3], Action::Remove), vec![0, 2, 4]);
        assert_eq!(Action::parse("remove").unwrap(), Action::Remove);
        assert!(Action::parse("both").is_err());
    }

    #[test]
    fn spread_shares_the_target_and_caps_short_sites() {
        // the four sites of the example: 25 good parts is 6 or 7 from each
        assert_eq!(spread(&[28, 29, 29, 30], 25), vec![7, 6, 6, 6]);
        assert_eq!(spread(&[28, 29, 29, 30], 25).iter().sum::<usize>(), 25);
        // a short site gives all it has, the others share the rest
        assert_eq!(spread(&[5, 30, 30, 30], 60), vec![5, 19, 18, 18]);
        // not enough parts: all of them
        assert_eq!(spread(&[3, 3], 10), vec![3, 3]);
        assert_eq!(spread(&[10, 10, 10], 0), vec![0, 0, 0]);
        assert!(spread(&[], 5).is_empty());
    }
}
