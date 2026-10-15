//! Runs the project's agreement-based estimator (`s106::accuracy`, unchanged) on the pilot's
//! votes.csv: each model is its own family, each item one cell (scheme = item id, V1, deed).
//!
//!     estimate <votes.csv> [readers.csv]   > estimate.csv
//!
//! Prints one CSV: `reader` rows (estimate, lower bound, fits, and the measured accuracy from
//! readers.csv when given) and `item` rows (the majority label and its posterior chance of error).

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result, bail};
use s106::accuracy::{Estimate, Kind, Vote, error, estimates};

fn rows(path: &str) -> Result<Vec<BTreeMap<String, String>>> {
    let mut r = csv::Reader::from_path(path).with_context(|| format!("reading {path}"))?;
    Ok(r.deserialize().collect::<Result<_, _>>()?)
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        bail!("usage: estimate <votes.csv> [readers.csv]");
    }
    let mut votes = vec![];
    for row in rows(&args[1])? {
        let get = |c: &str| row.get(c).cloned().unwrap_or_default();
        if get("kind") != "llm" {
            bail!("unexpected kind {}", get("kind"));
        }
        votes.push(Vote {
            cell: (get("scheme"), get("variable"), get("source")),
            reader: get("reader"),
            kind: Kind::Llm,
            family: get("family"),
            status: get("status"),
            fields: BTreeMap::from([("v".to_string(), get("value"))]),
            partial: false,
            value: get("value"),
        });
    }
    let measured: BTreeMap<String, String> = match args.get(2) {
        Some(p) => rows(p)?
            .into_iter()
            .map(|r| (r["reader"].clone(), r["measured_accuracy"].clone()))
            .collect(),
        None => BTreeMap::new(),
    };
    let est = estimates(&votes, &BTreeSet::new());
    let by: BTreeMap<(String, String), &Estimate> =
        est.iter().map(|e| ((e.group.clone(), e.reader.clone()), e)).collect();

    let mut w = csv::Writer::from_writer(std::io::stdout());
    w.write_record([
        "row", "id", "against", "shared", "estimate", "lower", "fits", "passes_gate",
        "measured_accuracy", "shown", "n_votes", "error",
    ])?;
    for e in &est {
        w.write_record([
            "reader",
            &e.reader,
            &e.against.join(" "),
            &e.shared.to_string(),
            &format!("{:.4}", e.accuracy),
            &format!("{:.4}", e.lower),
            &e.fits.to_string(),
            &e.passes().to_string(),
            measured.get(&e.reader).map_or("", |s| s.as_str()),
            "",
            "",
            "",
        ])?;
    }
    let mut cells: BTreeMap<&(String, String, String), Vec<&Vote>> = BTreeMap::new();
    for v in &votes {
        cells.entry(&v.cell).or_default().push(v);
    }
    for (cell, vs) in &cells {
        let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
        for v in vs {
            *counts.entry(v.value.as_str()).or_default() += 1;
        }
        // the majority label (first in label order on a tie), as the shown reading
        let top = counts.iter().max_by(|a, b| a.1.cmp(b.1).then(b.0.cmp(a.0))).map(|(k, _)| *k);
        let shown = vs.iter().find(|v| Some(v.value.as_str()) == top).unwrap();
        let err = error(shown, vs, &by).map_or(String::new(), |x| format!("{x:.4}"));
        w.write_record([
            "item", &cell.0, "", "", "", "", "", "", "", &shown.value, &vs.len().to_string(), &err,
        ])?;
    }
    w.flush()?;
    Ok(())
}
