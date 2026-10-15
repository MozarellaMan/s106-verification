//! OCR confidence for every number a proposal quotes: `s106 ocr-conf` finds each number of a
//! row's `quote` on the OCR lines of the pages the row cites, and records the OCR model's
//! confidence in the line it sits on. A quote check passes on text the OCR got wrong, as long as
//! the reader copied the same wrong text (the verification study), so a low confidence on a quoted
//! number is a sign the page should be read again from the image.
//!
//! Text-layer pages carry no confidence and are reported as `text_layer`. Snapshots written
//! before confidence was recorded carry none either, and are reported as `no_confidence` until they
//! are regenerated.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result};
use serde::Serialize;

use crate::document::{Snapshot, TextSource};

/// One quoted number and where it was found.
#[derive(Debug, Clone, Serialize)]
pub struct Row {
    pub proposal_id: String,
    pub labeller: String,
    pub number: String,
    pub doc_id: String,
    pub page: String,
    /// `ocr`, `text_layer`, `no_confidence`, `not_found` (no cited OCR line holds the number) or
    /// `no_snapshot`
    pub status: String,
    /// the lowest confidence among the cited lines holding the number
    pub conf: String,
    pub line: String,
}

/// The numbers in a quote: runs of digits, with `,` thousands separators and `.` decimals inside.
pub fn numbers(quote: &str) -> Vec<String> {
    let mut out = vec![];
    let mut cur = String::new();
    let chars: Vec<char> = quote.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        let inside = (c == ',' || c == '.')
            && !cur.is_empty()
            && chars.get(i + 1).is_some_and(|n| n.is_ascii_digit());
        if c.is_ascii_digit() || inside {
            cur.push(c);
        } else if !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out.sort();
    out.dedup();
    out
}

/// The pages a row cites, per document: `DOC:4-5,9; DOC2:8`, or a bare list for every `doc_ids`.
pub fn cited(doc_ids: &str, pages: &str) -> BTreeMap<String, Vec<usize>> {
    let docs: Vec<&str> = doc_ids.split_whitespace().collect();
    let mut out: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for part in pages.split(';').map(str::trim).filter(|p| !p.is_empty()) {
        let (targets, list) = match part.split_once(':') {
            Some((d, l)) => (vec![d.trim()], l),
            None => (docs.clone(), part),
        };
        let mut nums = vec![];
        for item in list.split(',').map(str::trim) {
            let item = item.trim_start_matches('p');
            let (a, b) = item.split_once('-').unwrap_or((item, item));
            if let (Ok(a), Ok(b)) = (a.trim().parse::<usize>(), b.trim().parse::<usize>()) {
                nums.extend(a..=b.max(a));
            }
        }
        for d in targets {
            out.entry(d.to_string()).or_default().extend(&nums);
        }
    }
    out
}

/// The snapshot names a doc id may have: itself, and for a Southwark deed id such as
/// `09AP1783-1` or `13AP3815-003` the corpus name `southwark-09_AP_1783-1`.
fn snapshot_names(doc: &str) -> Vec<String> {
    let mut out = vec![doc.to_string()];
    let b = doc.as_bytes();
    if b.len() > 9
        && b[..2].iter().all(u8::is_ascii_digit)
        && &doc[2..4] == "AP"
        && b[4..8].iter().all(u8::is_ascii_digit)
        && b[8] == b'-'
        && let Ok(k) = doc[9..].parse::<u32>()
    {
        out.push(format!("southwark-{}_AP_{}-{k}", &doc[..2], &doc[4..8]));
    }
    out
}

/// Whether `line` holds `number` as a whole number (not inside a longer one).
fn holds(line: &str, number: &str) -> bool {
    // thousands commas are dropped from both, so "1,250" and "1250" match each other
    let (line, number) = (line.replace(',', ""), number.replace(',', ""));
    let digits = |c: char| c.is_ascii_digit();
    line.match_indices(&number).any(|(i, m)| {
        let before = line[..i].chars().next_back();
        let after = line[i + m.len()..].chars().next();
        !before.is_some_and(digits) && !after.is_some_and(digits)
    })
}

/// Every quoted number of every `found` or `ambiguous` row of the proposals file `file`.
pub fn rows(root: &Path, file: &Path) -> Result<Vec<Row>> {
    let mut r = csv::Reader::from_path(root.join(file))
        .with_context(|| format!("opening {}", file.display()))?;
    let mut snaps: BTreeMap<String, Option<Snapshot>> = BTreeMap::new();
    let mut out = vec![];
    for rec in r.deserialize::<BTreeMap<String, String>>() {
        let rec = rec?;
        let get = |k: &str| rec.get(k).cloned().unwrap_or_default();
        if !matches!(get("status").as_str(), "found" | "ambiguous") {
            continue;
        }
        for number in numbers(&get("quote")) {
            for (doc, pages) in cited(&get("doc_ids"), &get("pages")) {
                let snap = snaps.entry(doc.clone()).or_insert_with(|| {
                    snapshot_names(&doc).iter().find_map(|name| {
                        Snapshot::read(&root.join("pages").join(format!("{name}.pages.json"))).ok()
                    })
                });
                let row = |status: &str, page: String, conf: String, line: String| Row {
                    proposal_id: get("proposal_id"),
                    labeller: get("labeller"),
                    number: number.clone(),
                    doc_id: doc.clone(),
                    page,
                    status: status.into(),
                    conf,
                    line,
                };
                let Some(snap) = snap else {
                    out.push(row(
                        "no_snapshot",
                        String::new(),
                        String::new(),
                        String::new(),
                    ));
                    continue;
                };
                // the lowest-confidence cited line holding the number
                let mut best: Option<(usize, &crate::document::Line, TextSource)> = None;
                for p in snap.pages.iter().filter(|p| pages.contains(&p.page)) {
                    for l in p.lines.iter().filter(|l| holds(&l.text, &number)) {
                        let lower = |o: &(usize, &crate::document::Line, TextSource)| {
                            l.conf.unwrap_or(f32::MAX) < o.1.conf.unwrap_or(f32::MAX)
                        };
                        if best.as_ref().is_none_or(lower) {
                            best = Some((p.page, l, p.source));
                        }
                    }
                }
                out.push(match best {
                    None => row("not_found", String::new(), String::new(), String::new()),
                    Some((page, l, source)) => {
                        let status = match (source, l.conf) {
                            (TextSource::TextLayer, _) => "text_layer",
                            (TextSource::Ocr, None) => "no_confidence",
                            (TextSource::Ocr, Some(_)) => "ocr",
                        };
                        row(
                            status,
                            page.to_string(),
                            l.conf.map_or(String::new(), |c| format!("{c:.3}")),
                            l.text.clone(),
                        )
                    }
                });
            }
        }
    }
    Ok(out)
}

/// Writes `out` (relative to `root`) for the proposals file `file`; returns the row count per status.
pub fn run(root: &Path, file: &Path, out: &Path) -> Result<BTreeMap<String, usize>> {
    let rows = rows(root, file)?;
    if let Some(dir) = root.join(out).parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut w = csv::Writer::from_path(root.join(out))?;
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for r in &rows {
        *counts.entry(r.status.clone()).or_default() += 1;
        w.serialize(r)?;
    }
    w.flush()?;
    Ok(counts)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_and_pages_parse() {
        assert_eq!(
            numbers("provide 35% (1,250 sq m) and 40.5 units, clause 4."),
            vec!["1,250", "35", "4", "40.5"]
        );
        let c = cited("A B", "A:4-5,9; B:8");
        assert_eq!(c["A"], vec![4, 5, 9]);
        assert_eq!(c["B"], vec![8]);
        assert_eq!(cited("A B", "p1, 3")["B"], vec![1, 3]);
        assert!(holds("Units 35 and 40", "35"));
        assert!(!holds("Units 135", "35"));
        assert!(holds("1250 sq m", "1,250") && holds("1,250 sq m", "1250"));
        assert_eq!(snapshot_names("13AP3815-003")[1], "southwark-13_AP_3815-3");
        assert_eq!(snapshot_names("19AP2307-decision").len(), 1);
    }
}
