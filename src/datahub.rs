//! The Datahub record as a reader: `s106 pld map` turns each scheme's latest application
//! snapshot into V1 (homes by tenure) and V8 (completion) rows of a proposals file, signed
//! `machine:proposal/datahub-mapping`. The mapping is mechanical, so the same snapshot always
//! gives the same rows; the Datahub is the reader, and its errors are the borough's entries.
//!
//! V1: `total` and the affordable count come from `residential_details`; `market` is total minus
//! affordable; each tenure count is the number of `Gain` records in `residential_units` with that
//! tenure. V8: the application's `actual_completion_date`, and the number of `Gain` records that
//! carry a completion date.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result};
use serde::Serialize;
use serde_json::Value;

pub const LABELLER: &str = "machine:proposal/datahub-mapping";

/// The Datahub's tenure labels as codebook codes; `None` for market homes, which are counted as
/// total minus affordable.
fn code(tenure: &str) -> Option<&'static str> {
    match tenure {
        "Social Rent" => Some("SR"),
        "London Affordable Rent" => Some("LAR"),
        "Affordable Rent (not at LAR benchmark rents)" => Some("AR"),
        "London Shared Ownership" | "Shared Ownership" => Some("SO"),
        "London Living Rent" => Some("LLR"),
        "Discount Market Rent" | "Discount Market Sale" | "Shared Equity" | "Starter Homes" => {
            Some("INT")
        }
        "Intermediate Other" => Some("INT"),
        _ => None,
    }
}

/// One row of a proposals file (`proposals/README.md` gives the columns).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Row {
    pub scheme: String,
    pub variable_id: String,
    pub variable: String,
    pub source: String,
    pub doc_ids: String,
    pub value: String,
    pub basis: String,
    pub pages: String,
    pub status: String,
    pub minutes: String,
    pub notes: String,
    pub labeller: String,
    pub labelled_on: String,
    pub proposal_id: String,
    pub quote: String,
    pub rationale: String,
    pub doubt: String,
    pub confidence: String,
    pub context: String,
}

fn number(v: &Value) -> Option<i64> {
    v.as_i64().or_else(|| v.as_f64().map(|f| f as i64))
}

/// `dd/mm/yyyy` as `yyyy-mm-dd`.
fn iso(d: &str) -> Option<String> {
    chrono::NaiveDate::parse_from_str(d, "%d/%m/%Y")
        .ok()
        .map(|d| d.format("%Y-%m-%d").to_string())
}

/// The V1 and V8 rows for one scheme from its application snapshot's `response`.
pub fn rows(scheme: &str, snapshot: &str, response: &Value, on: &str) -> Vec<Row> {
    let rd = &response["application_details"]["residential_details"];
    let units: Vec<&Value> = rd["residential_units"]
        .as_array()
        .map(|u| u.iter().filter(|u| u["change_type"] == "Gain").collect())
        .unwrap_or_default();
    let row =
        |v: &str, name: &str, value: String, status: &str, pages: &str, quote: Vec<String>| Row {
            scheme: scheme.into(),
            variable_id: v.into(),
            variable: name.into(),
            source: "datahub".into(),
            doc_ids: snapshot.into(),
            value,
            basis: "units".into(),
            pages: pages.into(),
            status: status.into(),
            minutes: String::new(),
            notes: String::new(),
            labeller: LABELLER.into(),
            labelled_on: on.into(),
            proposal_id: format!("{scheme}/{v}/datahub"),
            quote: quote.join("\n"),
            rationale: String::new(),
            doubt: String::new(),
            confidence: "sure".into(),
            context: snapshot.into(),
        };

    let total = number(&rd["total_no_proposed_residential_units"]);
    let affordable = number(&rd["total_no_proposed_residential_affordable_units"]);
    let mut tenures: BTreeMap<&str, i64> = BTreeMap::new();
    let mut labels: BTreeMap<&str, i64> = BTreeMap::new();
    for u in &units {
        let t = u["tenure"].as_str().unwrap_or("");
        *labels.entry(t).or_default() += 1;
        if let Some(c) = code(t) {
            *tenures.entry(c).or_default() += 1;
        }
    }
    let mut v1 = match (total, affordable) {
        (Some(t), Some(a)) => {
            let mut parts = vec![format!("total={t}"), format!("market={}", t - a)];
            // tenure codes in alphabetical order, as the batch-1 mapping wrote them
            parts.extend(tenures.iter().map(|(c, n)| format!("{c}={n}")));
            let mut quote = vec![
                format!("residential_details.total_no_proposed_residential_units = {t}"),
                format!("residential_details.total_no_proposed_residential_affordable_units = {a}"),
                format!(
                    "residential_units: {} records, {} with change_type=Gain",
                    rd["residential_units"].as_array().map_or(0, |u| u.len()),
                    units.len()
                ),
            ];
            quote.extend(labels.iter().map(|(l, n)| format!("  {n} × tenure = {l}")));
            row(
                "V1",
                "Total homes; affordable homes by tenure",
                parts.join("; "),
                "found",
                "residential_details; residential_units",
                quote,
            )
        }
        _ => row(
            "V1",
            "Total homes; affordable homes by tenure",
            String::new(),
            "not_stated",
            "",
            vec!["residential_details has no total or affordable count".into()],
        ),
    };
    v1.rationale = "Total and affordable come from residential_details; market is total minus \
                    affordable; the tenure counts are the Gain records in residential_units mapped \
                    to codebook codes."
        .into();
    v1.doubt = "The tenure labels are the Datahub's, not the deed's, and the unit records may \
                describe a later superseding scheme."
        .into();

    let done = response["actual_completion_date"].as_str().and_then(iso);
    let completed = units
        .iter()
        .filter(|u| {
            u["actual_completion_date"]
                .as_str()
                .is_some_and(|d| !d.is_empty())
        })
        .count();
    let quote = vec![
        format!("status = {}", response["status"].as_str().unwrap_or("")),
        format!(
            "actual_completion_date = {}",
            response["actual_completion_date"]
                .as_str()
                .unwrap_or("None")
        ),
        format!(
            "residential_units (Gain) with an actual_completion_date: {completed} of {}",
            units.len()
        ),
    ];
    let mut v8 = match done {
        Some(d) => row(
            "V8",
            "Completion date and completed homes",
            format!("completed={d}; completed_homes={completed}"),
            "found",
            "actual_completion_date; residential_units",
            quote,
        ),
        None => row(
            "V8",
            "Completion date and completed homes",
            String::new(),
            "not_stated",
            "actual_completion_date",
            quote,
        ),
    };
    v8.rationale = "actual_completion_date on the application; completed_homes counts the Gain \
                    unit records that carry a completion date."
        .into();
    v8.doubt = "A completion date here is the borough's data entry, not an inspection; a Lapsed \
                status can sit beside one."
        .into();
    vec![v1, v8]
}

/// Maps the latest application snapshot of each scheme on `sheet` (relative to `root`) into
/// `out` (relative to `root`). A scheme with no snapshot is reported, not mapped.
pub fn map(root: &Path, sheet: &Path, out: &Path, on: &str) -> Result<Vec<String>> {
    let mut r = csv::Reader::from_path(root.join(sheet))
        .with_context(|| format!("opening {}", sheet.display()))?;
    let mut schemes: Vec<String> = vec![];
    for row in r.deserialize::<BTreeMap<String, String>>() {
        let s = row?.get("scheme").cloned().unwrap_or_default();
        if !s.is_empty() && !schemes.contains(&s) {
            schemes.push(s);
        }
    }
    let mut w = csv::Writer::from_path(root.join(out))?;
    let mut missing = vec![];
    for s in &schemes {
        // snapshots sit in `applications/<borough>-<reference with / as _>/`
        let suffix = format!("-{}", s.replace('/', "_"));
        let apps = crate::pld::snapshots_root(root).join("applications");
        let latest = std::fs::read_dir(&apps)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().ends_with(&suffix))
            .flat_map(|e| std::fs::read_dir(e.path()).into_iter().flatten().flatten())
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "json"))
            .max();
        let Some(path) = latest else {
            missing.push(s.clone());
            continue;
        };
        let rel = path.strip_prefix(&apps).map_or(path.clone(), |p| {
            Path::new("snapshots/pld/applications").join(p)
        });
        let (snap, r) = crate::pld::load(root, &rel)?;
        for row in rows(s, &r.path, &snap.response, on) {
            w.serialize(row)?;
        }
    }
    w.flush()?;
    Ok(missing)
}
