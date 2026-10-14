//! Human labels: what a person read off each source, and the figures that follow from them.
//!
//! Labels are a layer on their own. Nothing else in the crate reads them, and nothing waits for
//! them: `s106 labels` picks up whatever submissions exist under `<data>/labels/`, and with none
//! it says so and writes the same (empty) report. Each submission is a CSV in the source-audit
//! codebook's columns (`runs/2026-10-14-source-audit-batch1/codebook.md` in s106-data), one row
//! per scheme × variable × source, signed with `labeller` and `labelled_on`. Submissions are
//! append-only: a correction is a new file, and for each labeller and cell the latest
//! `labelled_on` (then file name) wins.
//!
//! Three things come out, each with its n:
//! - agreement between labellers, where two people labelled the same cell (Cohen's kappa on status);
//! - agreement between sources, as the labels record them (the deed against the committee report, ...);
//! - the pipeline's accuracy: its own readings of the same cells (recorded extractions, Datahub
//!   snapshots) scored field by field against the labels, as precision and recall.
//!
//! A disagreement here is a discrepancy for another reader to settle, never a finding about a scheme.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use rust_decimal::Decimal;
use serde::Serialize;

use crate::schema::{AffordableBasis, Housing, Tenure};

/// The codebook's statuses.
pub const STATUSES: [&str; 8] = [
    "found",
    "not_stated",
    "ambiguous",
    "inconsistent",
    "source_missing",
    "no_source",
    "unreadable",
    "not_applicable",
];

/// Labeller prefix for the pipeline's own readings. Rejected in submissions: an agent's reading
/// is never a label.
pub const MACHINE: &str = "machine:";

/// How a person arrived at a label (the optional `method` column; empty means `read`):
/// `read` and `blind` are independent readings of the source; `audited` and `corrected` are
/// decisions on a shown proposal and its evidence, so they are anchored on the proposal.
pub const METHODS: [&str; 4] = ["read", "blind", "audited", "corrected"];

/// The two kinds of label the accuracy tables keep apart.
pub fn label_kind(method: &str) -> &'static str {
    match method {
        "audited" | "corrected" => "anchored",
        _ => "independent",
    }
}

const REQUIRED: [&str; 8] = [
    "scheme",
    "variable_id",
    "source",
    "value",
    "basis",
    "status",
    "labeller",
    "labelled_on",
];

/// One reading of one cell: a scheme's variable as one source states it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Reading {
    pub scheme: String,
    pub variable_id: String,
    pub source: String,
    pub value: String,
    pub basis: String,
    pub status: String,
    pub labeller: String,
    pub labelled_on: String,
    pub minutes: Option<u32>,
    /// the documents and pages (or Datahub fields) the value was read from, as submitted
    pub doc_ids: String,
    pub pages: String,
    /// one of [`METHODS`] for a label; empty for a machine reading
    pub method: String,
    /// `<file>:<line>` for a label; what it was read from for a machine reading
    pub origin: String,
}

type Cell = (String, String, String);

impl Reading {
    fn cell(&self) -> Cell {
        (
            self.scheme.clone(),
            self.variable_id.clone(),
            self.source.clone(),
        )
    }

    /// The value and basis as the report shows them.
    fn shown(&self) -> String {
        match self.basis.as_str() {
            "" => format!("`{}`", self.value),
            b => format!("`{}` ({b})", self.value),
        }
    }

    /// The value as fields: `total=120; SR=24` -> {total: 120, sr: 24}, plus `basis` when stated.
    /// Numbers compare as numbers (`35%`, `35.0` and `35` are one value); free text compares with
    /// case and spacing ignored.
    pub fn fields(&self) -> BTreeMap<String, String> {
        let mut out = BTreeMap::new();
        if self.status != "found" {
            return out;
        }
        if self.value.contains('=') {
            for part in self.value.split(';') {
                if let Some((k, v)) = part.split_once('=') {
                    let k = normal(k);
                    // `other=<label>:N` may repeat, one per label, so the label is part of the key
                    match v.rsplit_once(':').filter(|_| k == "other") {
                        Some((label, n)) => {
                            out.insert(format!("other:{}", normal(label)), normal(n))
                        }
                        None => out.insert(k, normal(v)),
                    };
                }
            }
        } else if !self.value.trim().is_empty() {
            out.insert("value".into(), normal(&self.value));
        }
        if !self.basis.trim().is_empty() && self.basis.trim() != "not_stated" {
            out.insert("basis".into(), normal(&self.basis));
        }
        out
    }
}

fn normal(s: &str) -> String {
    let words = s
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    let number = words
        .trim_start_matches('£')
        .trim_end_matches('%')
        .replace(',', "");
    match number.parse::<Decimal>() {
        Ok(d) => d.normalize().to_string(),
        Err(_) => words,
    }
}

/// A submitted row that couldn't be used, and why. Reported, never fatal.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Rejected {
    pub origin: String,
    pub reason: String,
}

// ------------------------------------------------------------------------------------ loading

/// Every `*.csv` under `dir` (recursively), in path order. Rows with an empty `status` are cells
/// not labelled yet and are skipped. A missing directory is no labels, not an error.
pub fn load(dir: &Path) -> Result<(Vec<Reading>, Vec<Rejected>)> {
    let mut files = vec![];
    collect_csv(dir, &mut files)?;
    files.sort();
    let mut readings = vec![];
    let mut rejected = vec![];
    for f in &files {
        let name = f.strip_prefix(dir).unwrap_or(f).display().to_string();
        load_file(f, &name, false, &mut readings, &mut rejected)?;
    }
    Ok((readings, rejected))
}

/// Every `*.csv` under `<root>/proposals/`: an agent's proposed labels, in the codebook's columns,
/// signed `machine:<producer>`. They are scored like any other machine reading and are never
/// labels. A missing directory is no proposals.
pub fn proposals(root: &Path) -> Result<(Vec<Reading>, Vec<Rejected>)> {
    let dir = root.join("proposals");
    let mut files = vec![];
    collect_csv(&dir, &mut files)?;
    files.sort();
    let mut readings = vec![];
    let mut rejected = vec![];
    for f in &files {
        let name = format!("proposals/{}", f.strip_prefix(&dir).unwrap_or(f).display());
        load_file(f, &name, true, &mut readings, &mut rejected)?;
    }
    Ok((readings, rejected))
}

fn proposal_problem(r: &Reading) -> Option<String> {
    if r.scheme.is_empty() || r.variable_id.is_empty() || r.source.is_empty() {
        return Some("scheme, variable_id and source are all needed".into());
    }
    if !STATUSES.contains(&r.status.as_str()) {
        return Some(format!("status `{}` is not in the codebook", r.status));
    }
    if !r.labeller.starts_with(MACHINE) {
        return Some(format!(
            "a proposal is signed `{MACHINE}<producer>`, not `{}`",
            r.labeller
        ));
    }
    if !r.method.is_empty() {
        return Some("a proposal has no method; that column is for labels".into());
    }
    if r.status == "found" && r.value.is_empty() {
        return Some("status found but no value".into());
    }
    None
}

fn collect_csv(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    if !dir.is_dir() {
        return Ok(());
    }
    for entry in std::fs::read_dir(dir).with_context(|| format!("listing {}", dir.display()))? {
        let path = entry?.path();
        if path.is_dir() {
            collect_csv(&path, out)?;
        } else if path.extension().is_some_and(|e| e == "csv") {
            out.push(path);
        }
    }
    Ok(())
}

/// `proposals` reads a proposals file (see [`proposals`]) instead of a submission: the rows must
/// be signed `machine:` and carry no `method`.
fn load_file(
    path: &Path,
    name: &str,
    proposals: bool,
    readings: &mut Vec<Reading>,
    rejected: &mut Vec<Rejected>,
) -> Result<()> {
    let mut r = csv::Reader::from_path(path).with_context(|| format!("opening {name}"))?;
    let headers = r.headers()?.clone();
    let col = |n: &str| headers.iter().position(|h| h.trim() == n);
    let missing: Vec<&str> = REQUIRED.into_iter().filter(|c| col(c).is_none()).collect();
    if !missing.is_empty() {
        rejected.push(Rejected {
            origin: name.into(),
            reason: format!("whole file: no column {}", missing.join(", ")),
        });
        return Ok(());
    }
    for (i, row) in r.records().enumerate() {
        // line 1 is the header
        let origin = format!("{name}:{}", i + 2);
        let row = match row {
            Ok(row) => row,
            Err(e) => {
                rejected.push(Rejected {
                    origin,
                    reason: e.to_string(),
                });
                continue;
            }
        };
        let get = |n: &str| {
            col(n)
                .and_then(|c| row.get(c))
                .unwrap_or("")
                .trim()
                .to_string()
        };
        let reading = Reading {
            // the same form as the pipeline's readings, so `ex/2` and `EX/2` are one scheme
            scheme: get("scheme").to_uppercase(),
            variable_id: get("variable_id").to_uppercase(),
            source: get("source").to_lowercase(),
            value: get("value"),
            basis: get("basis"),
            status: get("status"),
            labeller: get("labeller"),
            labelled_on: get("labelled_on"),
            minutes: get("minutes").parse().ok(),
            doc_ids: get("doc_ids"),
            pages: get("pages"),
            method: match get("method").to_lowercase() {
                m if m.is_empty() && !proposals => "read".into(),
                m => m,
            },
            origin: origin.clone(),
        };
        if reading.status.is_empty() {
            continue;
        }
        let problem = if proposals {
            proposal_problem(&reading)
        } else {
            problem(&reading)
        };
        match problem {
            Some(reason) => rejected.push(Rejected { origin, reason }),
            None => readings.push(reading),
        }
    }
    Ok(())
}

fn problem(r: &Reading) -> Option<String> {
    if r.scheme.is_empty() || r.variable_id.is_empty() || r.source.is_empty() {
        return Some("scheme, variable_id and source are all needed".into());
    }
    if !STATUSES.contains(&r.status.as_str()) {
        return Some(format!("status `{}` is not in the codebook", r.status));
    }
    if r.labeller.is_empty() {
        return Some("no labeller".into());
    }
    if r.labeller.starts_with(MACHINE) {
        return Some("a machine reading is not a label".into());
    }
    if !METHODS.contains(&r.method.as_str()) {
        return Some(format!(
            "method `{}` is not one of {}",
            r.method,
            METHODS.join(", ")
        ));
    }
    if chrono::NaiveDate::parse_from_str(&r.labelled_on, "%Y-%m-%d").is_err() {
        return Some(format!(
            "labelled_on `{}` is not a YYYY-MM-DD date",
            r.labelled_on
        ));
    }
    if r.status == "found" && r.value.is_empty() {
        return Some("status found but no value".into());
    }
    None
}

/// For each labeller and cell, the latest reading by `labelled_on`; on the same date, the one
/// that comes later in `readings` (as `load` returns them: by file path, then row). Earlier ones
/// stay on disk; they are superseded, not deleted.
pub fn latest(readings: &[Reading]) -> Vec<Reading> {
    let mut by: BTreeMap<(Cell, String), &Reading> = BTreeMap::new();
    for r in readings {
        let key = (r.cell(), r.labeller.clone());
        let newer = by
            .get(&key)
            .is_none_or(|old| r.labelled_on >= old.labelled_on);
        if newer {
            by.insert(key, r);
        }
    }
    by.into_values().cloned().collect()
}

// --------------------------------------------------------------------------- machine readings

/// The pipeline's readings of the cells it can fill, in the codebook's format: deed V1 and V2
/// from every recorded extraction that replays from `<data>/pages/`, Datahub V1 and V8 from the
/// latest snapshot of each application. Anything that can't be read is a note, not an error.
pub fn machine_readings(root: &Path, model: &str) -> (Vec<Reading>, Vec<String>) {
    let mut readings = vec![];
    let mut notes = vec![];
    deed_readings(root, model, &mut readings, &mut notes);
    datahub_readings(root, &mut readings, &mut notes);
    (merge_duplicates(readings), notes)
}

fn sorted_entries(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|d| d.filter_map(|e| e.ok().map(|e| e.path())).collect())
        .unwrap_or_default();
    v.sort();
    v
}

fn deed_readings(root: &Path, model: &str, out: &mut Vec<Reading>, notes: &mut Vec<String>) {
    let labeller = format!("{MACHINE}extract/{model}");
    for path in sorted_entries(&root.join("pages")) {
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        let Some(stem) = name.strip_suffix(".pages.json") else {
            continue;
        };
        let agreement = crate::document::Snapshot::read(&path).and_then(|s| {
            let doc = s.document();
            crate::extract::extract(&doc, model, crate::extract::Mode::Replay, None)
        });
        let a = match agreement {
            Ok((a, _)) => a,
            Err(e) => {
                notes.push(format!("deed {stem}: no reading ({e:#})"));
                continue;
            }
        };
        if a.instrument.kind.is_variation() {
            // The codebook records the original obligation for V1-V6.
            notes.push(format!("deed {stem}: a variation, not read for V1/V2"));
            continue;
        }
        if a.metadata.planning_references.is_empty() {
            notes.push(format!("deed {stem}: names no planning reference"));
        }
        let origin = format!("extractions/{stem}/{model}.extract.v2.json");
        for scheme in &a.metadata.planning_references {
            let reading = |variable_id: &str, value: String, basis: &str| Reading {
                scheme: scheme.trim().to_uppercase(),
                variable_id: variable_id.into(),
                source: "deed".into(),
                status: if value.is_empty() {
                    "not_stated"
                } else {
                    "found"
                }
                .into(),
                value,
                basis: basis.into(),
                labeller: labeller.clone(),
                labelled_on: String::new(),
                minutes: None,
                doc_ids: stem.to_string(),
                pages: String::new(),
                method: String::new(),
                origin: origin.clone(),
            };
            out.push(reading("V1", deed_v1(&a.housing), "units"));
            let (share, basis) = deed_v2(&a.housing);
            out.push(reading("V2", share, basis));
        }
    }
}

fn deed_v1(h: &Housing) -> String {
    let mut parts = vec![];
    if let Some(n) = h.total_units {
        parts.push(format!("total={n}"));
    }
    if let Some(n) = h.market_units {
        parts.push(format!("market={n}"));
    }
    let tenures = h
        .tenures
        .iter()
        .filter_map(|t| Some((t.tenure, t.units?)))
        .collect::<Vec<_>>();
    parts.extend(tenure_parts(&tenures));
    parts.join("; ")
}

fn deed_v2(h: &Housing) -> (String, &'static str) {
    let Some(pct) = h.claimed_affordable_pct.as_ref().and_then(|p| p.ok()) else {
        return (String::new(), "");
    };
    let basis = match h.claimed_affordable_basis {
        Some(AffordableBasis::Units) => "units",
        Some(AffordableBasis::HabitableRooms) => "habitable_rooms",
        Some(AffordableBasis::Floorspace) => "floorspace",
        None => "not_stated",
    };
    (format!("share={}%", pct.normalize()), basis)
}

/// The codebook's tenure codes; zero counts are left out ("only the tenures the source names").
fn tenure_parts(tenures: &[(Tenure, u32)]) -> Vec<String> {
    tenures
        .iter()
        .filter(|(_, n)| *n > 0)
        .map(|(t, n)| match code(*t) {
            Some(c) => format!("{c}={n}"),
            None => format!("other={}:{n}", t.label()),
        })
        .collect()
}

fn code(t: Tenure) -> Option<&'static str> {
    Some(match t {
        Tenure::SocialRent => "SR",
        Tenure::LondonAffordableRent => "LAR",
        Tenure::AffordableRent => "AR",
        Tenure::LondonLivingRent => "LLR",
        Tenure::LondonSharedOwnership => "LSO",
        Tenure::SharedOwnership => "SO",
        Tenure::DiscountMarketRent => "DMR",
        Tenure::Intermediate => "INT",
        Tenure::DiscountMarketSale | Tenure::FirstHomes | Tenure::Other => return None,
    })
}

fn datahub_readings(root: &Path, out: &mut Vec<Reading>, notes: &mut Vec<String>) {
    let labeller = format!(
        "{MACHINE}pld-mapping/v{}",
        crate::reconcile::PLD_MAPPING_VERSION
    );
    let apps = crate::pld::snapshots_root(root).join("applications");
    // `_search/` holds search results, not one application each
    let dirs = sorted_entries(&apps).into_iter().filter(|d| {
        d.is_dir()
            && !d
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .starts_with('_')
    });
    for dir in dirs {
        let Some(latest) = sorted_entries(&dir).into_iter().rfind(|f| f.is_file()) else {
            continue;
        };
        let rel = latest.strip_prefix(root).unwrap_or(&latest).to_path_buf();
        let delivery = crate::pld::load(root, &rel)
            .and_then(|(snap, at)| crate::reconcile::delivery_from_pld(&snap, at));
        let d = match delivery {
            Ok(d) => d,
            Err(e) => {
                notes.push(format!("datahub {}: no reading ({e:#})", rel.display()));
                continue;
            }
        };
        let Some(scheme) = d.lpa_app_no.as_deref().map(|s| s.trim().to_uppercase()) else {
            notes.push(format!("datahub {}: no lpa_app_no", rel.display()));
            continue;
        };
        let p = &d.permitted;
        let mut v1 = vec![];
        if let Some(n) = p.total_units {
            v1.push(format!("total={n}"));
        }
        if let Some(n) = p.market_units {
            v1.push(format!("market={n}"));
        }
        v1.extend(tenure_parts(&p.tenure_units));
        let v8 = d
            .completed_on
            .map(|c| format!("completed={c}"))
            .unwrap_or_default();
        for (variable_id, value, basis) in [("V1", v1.join("; "), "units"), ("V8", v8, "")] {
            out.push(Reading {
                scheme: scheme.clone(),
                variable_id: variable_id.into(),
                source: "datahub".into(),
                status: if value.is_empty() {
                    "not_stated"
                } else {
                    "found"
                }
                .into(),
                value,
                basis: basis.into(),
                labeller: labeller.clone(),
                labelled_on: String::new(),
                minutes: None,
                doc_ids: String::new(),
                pages: String::new(),
                method: String::new(),
                origin: d.snapshot.path.clone(),
            });
        }
    }
}

/// Two readings of one cell by the same producer (two deed files naming one scheme): kept when
/// they agree, otherwise one `ambiguous` reading, which is not scored.
fn merge_duplicates(readings: Vec<Reading>) -> Vec<Reading> {
    let mut by: BTreeMap<(Cell, String), Reading> = BTreeMap::new();
    for r in readings {
        let key = (r.cell(), r.labeller.clone());
        match by.get_mut(&key) {
            None => {
                by.insert(key, r);
            }
            Some(old) if old.fields() == r.fields() && old.status == r.status => {
                old.origin = format!("{} + {}", old.origin, r.origin);
            }
            Some(old) => {
                old.value = format!("{} OR {}", old.value, r.value);
                old.status = "ambiguous".into();
                old.origin = format!("{} + {}", old.origin, r.origin);
            }
        }
    }
    by.into_values().collect()
}

// ------------------------------------------------------------------------------------ figures

#[derive(Debug, Serialize)]
pub struct Report {
    pub labels: Coverage,
    pub rejected: Vec<Rejected>,
    pub labellers: Vec<LabellerAgreement>,
    pub sources: Vec<SourceAgreement>,
    pub accuracy: Vec<Accuracy>,
    pub machine_notes: Vec<String>,
}

#[derive(Debug, Default, Serialize)]
pub struct Coverage {
    /// rows in the submissions, including superseded ones
    pub rows: usize,
    /// cells with at least one current label
    pub cells: usize,
    pub schemes: usize,
    pub labellers: Vec<String>,
    pub by_status: BTreeMap<String, usize>,
    /// current labels per method (`read`, `blind`, `audited`, `corrected`)
    pub by_method: BTreeMap<String, usize>,
    pub minutes: u32,
}

/// Two labellers on the cells both labelled.
#[derive(Debug, Serialize)]
pub struct LabellerAgreement {
    pub a: String,
    pub b: String,
    pub cells: usize,
    pub same_status: usize,
    /// Cohen's kappa on status; `None` when it is undefined (one status throughout)
    pub kappa: Option<f64>,
    pub both_found: usize,
    pub same_value: usize,
    pub differing: Vec<String>,
}

/// Two sources for one variable, on the schemes where the labels have both as `found`.
#[derive(Debug, Serialize)]
pub struct SourceAgreement {
    pub variable_id: String,
    pub a: String,
    pub b: String,
    pub schemes: usize,
    /// every field both sources state is equal
    pub agree: usize,
    pub differing: Vec<String>,
    /// schemes where the two state no field in common besides `basis`
    pub not_comparable: usize,
    /// schemes left out because the labellers disagree on a cell
    pub unsettled: usize,
}

/// The pipeline against the labels for one producer, variable and source. A field the label
/// states and the pipeline gets right is a true positive; a field the pipeline gets wrong or
/// misses lowers recall; a field the pipeline states that the label doesn't (or gets wrong)
/// lowers precision.
#[derive(Debug, Serialize)]
pub struct Accuracy {
    pub producer: String,
    pub variable_id: String,
    pub source: String,
    /// `independent` (labels read or blind) or `anchored` (audited or corrected proposals)
    pub labels: &'static str,
    /// labelled cells the pipeline also read
    pub cells: usize,
    /// cells left out: the label or the reading is ambiguous, inconsistent, or the source wasn't read
    pub not_scored: usize,
    pub true_pos: usize,
    pub false_pos: usize,
    pub false_neg: usize,
    pub precision: Option<Rate>,
    pub recall: Option<Rate>,
    pub errors: Vec<String>,
}

/// A proportion with its Wilson 95% interval.
#[derive(Debug, Serialize)]
pub struct Rate {
    pub k: usize,
    pub n: usize,
    pub value: f64,
    pub low: f64,
    pub high: f64,
}

impl Rate {
    pub fn of(k: usize, n: usize) -> Option<Rate> {
        if n == 0 {
            return None;
        }
        let (kf, nf, z) = (k as f64, n as f64, 1.96_f64);
        let p = kf / nf;
        let centre = (p + z * z / (2.0 * nf)) / (1.0 + z * z / nf);
        let half = z * (p * (1.0 - p) / nf + z * z / (4.0 * nf * nf)).sqrt() / (1.0 + z * z / nf);
        Some(Rate {
            k,
            n,
            value: p,
            low: (centre - half).max(0.0),
            high: (centre + half).min(1.0),
        })
    }
}

pub fn report(
    submitted: &[Reading],
    rejected: Vec<Rejected>,
    machine: &[Reading],
    machine_notes: Vec<String>,
) -> Report {
    let current = latest(submitted);
    let mut by_cell: BTreeMap<Cell, Vec<&Reading>> = BTreeMap::new();
    for r in &current {
        by_cell.entry(r.cell()).or_default().push(r);
    }
    let mut by_status = BTreeMap::new();
    let mut by_method = BTreeMap::new();
    for r in &current {
        *by_status.entry(r.status.clone()).or_insert(0) += 1;
        *by_method.entry(r.method.clone()).or_insert(0) += 1;
    }
    let labels = Coverage {
        rows: submitted.len(),
        cells: by_cell.len(),
        schemes: current
            .iter()
            .map(|r| &r.scheme)
            .collect::<BTreeSet<_>>()
            .len(),
        labellers: current
            .iter()
            .map(|r| r.labeller.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect(),
        by_status,
        by_method,
        minutes: current.iter().filter_map(|r| r.minutes).sum(),
    };
    let settled = settle(&by_cell);
    Report {
        labellers: labeller_agreement(&by_cell, &labels.labellers),
        sources: source_agreement(&by_cell, &settled),
        accuracy: accuracy(&settled, &by_cell, machine),
        labels,
        rejected,
        machine_notes,
    }
}

/// One reading per cell where the labellers agree (status and fields); cells where they don't are
/// left out of everything that uses labels as truth.
fn settle<'a>(by_cell: &BTreeMap<Cell, Vec<&'a Reading>>) -> BTreeMap<Cell, &'a Reading> {
    by_cell
        .iter()
        .filter(|(_, rs)| {
            rs.iter()
                .all(|r| r.status == rs[0].status && r.fields() == rs[0].fields())
        })
        .map(|(c, rs)| (c.clone(), rs[0]))
        .collect()
}

fn labeller_agreement(
    by_cell: &BTreeMap<Cell, Vec<&Reading>>,
    labellers: &[String],
) -> Vec<LabellerAgreement> {
    let mut out = vec![];
    for (i, a) in labellers.iter().enumerate() {
        for b in &labellers[i + 1..] {
            let mut pairs = vec![];
            for (cell, rs) in by_cell {
                let ra = rs.iter().find(|r| &r.labeller == a);
                let rb = rs.iter().find(|r| &r.labeller == b);
                if let (Some(ra), Some(rb)) = (ra, rb) {
                    pairs.push((cell, *ra, *rb));
                }
            }
            if pairs.is_empty() {
                continue;
            }
            let statuses: Vec<(&str, &str)> = pairs
                .iter()
                .map(|(_, ra, rb)| (ra.status.as_str(), rb.status.as_str()))
                .collect();
            let found: Vec<_> = pairs
                .iter()
                .filter(|(_, ra, rb)| ra.status == "found" && rb.status == "found")
                .collect();
            let differing = pairs
                .iter()
                .filter(|(_, ra, rb)| ra.status != rb.status || ra.fields() != rb.fields())
                .map(|(c, ra, rb)| {
                    format!(
                        "{} {} {}: {a} {} {}, {b} {} {}",
                        c.0,
                        c.1,
                        c.2,
                        ra.status,
                        ra.shown(),
                        rb.status,
                        rb.shown()
                    )
                })
                .collect();
            out.push(LabellerAgreement {
                a: a.clone(),
                b: b.clone(),
                cells: pairs.len(),
                same_status: statuses.iter().filter(|(x, y)| x == y).count(),
                kappa: kappa(&statuses),
                both_found: found.len(),
                same_value: found
                    .iter()
                    .filter(|(_, ra, rb)| ra.fields() == rb.fields())
                    .count(),
                differing,
            });
        }
    }
    out
}

/// Cohen's kappa for two raters' categories.
pub fn kappa(pairs: &[(&str, &str)]) -> Option<f64> {
    let n = pairs.len() as f64;
    if pairs.is_empty() {
        return None;
    }
    let observed = pairs.iter().filter(|(a, b)| a == b).count() as f64 / n;
    let categories: BTreeSet<&str> = pairs.iter().flat_map(|(a, b)| [*a, *b]).collect();
    let expected: f64 = categories
        .iter()
        .map(|c| {
            let pa = pairs.iter().filter(|(a, _)| a == c).count() as f64 / n;
            let pb = pairs.iter().filter(|(_, b)| b == c).count() as f64 / n;
            pa * pb
        })
        .sum();
    if (1.0 - expected).abs() < f64::EPSILON {
        return None;
    }
    Some((observed - expected) / (1.0 - expected))
}

fn source_agreement(
    by_cell: &BTreeMap<Cell, Vec<&Reading>>,
    settled: &BTreeMap<Cell, &Reading>,
) -> Vec<SourceAgreement> {
    // (variable, scheme) -> source -> reading, for settled `found` cells
    let mut found: BTreeMap<(&str, &str), BTreeMap<&str, &Reading>> = BTreeMap::new();
    let mut unsettled: BTreeSet<(&str, &str)> = BTreeSet::new();
    for cell in by_cell.keys() {
        let key = (cell.1.as_str(), cell.0.as_str());
        match settled.get(cell) {
            Some(r) if r.status == "found" => {
                found.entry(key).or_default().insert(cell.2.as_str(), r);
            }
            Some(_) => {}
            None => {
                unsettled.insert(key);
            }
        }
    }
    let mut pairs: BTreeMap<(&str, &str, &str), SourceAgreement> = BTreeMap::new();
    for ((variable, scheme), sources) in &found {
        let names: Vec<&str> = sources.keys().copied().collect();
        for (i, a) in names.iter().enumerate() {
            for b in &names[i + 1..] {
                let entry = pairs
                    .entry((variable, a, b))
                    .or_insert_with(|| SourceAgreement {
                        variable_id: variable.to_string(),
                        a: a.to_string(),
                        b: b.to_string(),
                        schemes: 0,
                        agree: 0,
                        differing: vec![],
                        not_comparable: 0,
                        unsettled: 0,
                    });
                let (fa, fb) = (sources[a].fields(), sources[b].fields());
                let shared: Vec<&String> = fa.keys().filter(|k| fb.contains_key(*k)).collect();
                entry.schemes += 1;
                if !shared.iter().any(|k| *k != "basis") {
                    entry.not_comparable += 1;
                } else if shared.iter().all(|k| fa[*k] == fb[*k]) {
                    entry.agree += 1;
                } else {
                    entry.differing.push(format!(
                        "{scheme}: {a} {}, {b} {}",
                        sources[a].shown(),
                        sources[b].shown()
                    ));
                }
            }
        }
    }
    for e in pairs.values_mut() {
        e.unsettled = unsettled
            .iter()
            .filter(|(v, _)| *v == e.variable_id)
            .count();
    }
    pairs.into_values().collect()
}

/// Fields a producer reads for a variable, when it reads only some of the codebook's: the label's
/// other fields are not held against it. The PLD mapping's V8 is the completion date only.
pub fn reads_only(producer: &str, variable_id: &str) -> Option<&'static [&'static str]> {
    match (
        producer.starts_with(&format!("{MACHINE}pld-mapping/")),
        variable_id,
    ) {
        (true, "V8") => Some(&["completed"]),
        _ => None,
    }
}

/// Statuses that say what a source holds; the rest (missing, unreadable, ambiguous, ...) can't
/// score a reading.
fn scoreable(status: &str) -> bool {
    status == "found" || status == "not_stated"
}

fn accuracy(
    settled: &BTreeMap<Cell, &Reading>,
    by_cell: &BTreeMap<Cell, Vec<&Reading>>,
    machine: &[Reading],
) -> Vec<Accuracy> {
    let mut out: BTreeMap<(String, String, String, &str), Accuracy> = BTreeMap::new();
    for m in machine {
        let cell = m.cell();
        let Some(labels) = by_cell.get(&cell) else {
            continue; // nobody has labelled this cell yet
        };
        // Labellers who disagree on the kind are a split cell; the settled label decides below.
        let kind = settled
            .get(&cell)
            .map_or_else(|| label_kind(&labels[0].method), |l| label_kind(&l.method));
        let a = out
            .entry((m.labeller.clone(), cell.1.clone(), cell.2.clone(), kind))
            .or_insert_with(|| Accuracy {
                producer: m.labeller.clone(),
                variable_id: cell.1.clone(),
                source: cell.2.clone(),
                labels: kind,
                cells: 0,
                not_scored: 0,
                true_pos: 0,
                false_pos: 0,
                false_neg: 0,
                precision: None,
                recall: None,
                errors: vec![],
            });
        a.cells += 1;
        let Some(label) = settled.get(&cell).filter(|l| scoreable(&l.status)) else {
            a.not_scored += 1;
            continue;
        };
        if !scoreable(&m.status) {
            a.not_scored += 1;
            continue;
        }
        let mut truth = label.fields();
        if let Some(only) = reads_only(&m.labeller, &m.variable_id) {
            truth.retain(|k, _| only.contains(&k.as_str()));
        }
        let mut got = m.fields();
        // A basis is scored only where the label states one.
        if !truth.contains_key("basis") {
            got.remove("basis");
        }
        let mut wrong = vec![];
        for (k, v) in &truth {
            match got.get(k) {
                Some(g) if g == v => a.true_pos += 1,
                Some(g) => {
                    a.false_pos += 1;
                    a.false_neg += 1;
                    wrong.push(format!("{k} {g} (label {v})"));
                }
                None => {
                    a.false_neg += 1;
                    wrong.push(format!("{k} missed (label {v})"));
                }
            }
        }
        for (k, g) in &got {
            if !truth.contains_key(k) {
                a.false_pos += 1;
                wrong.push(format!("{k} {g} (label states none)"));
            }
        }
        if !wrong.is_empty() {
            a.errors.push(format!("{}: {}", cell.0, wrong.join(", ")));
        }
    }
    let mut out: Vec<Accuracy> = out.into_values().collect();
    for a in &mut out {
        a.precision = Rate::of(a.true_pos, a.true_pos + a.false_pos);
        a.recall = Rate::of(a.true_pos, a.true_pos + a.false_neg);
    }
    out
}

// ------------------------------------------------------------------------------------- output

fn rate(r: &Option<Rate>) -> String {
    match r {
        Some(r) => format!(
            "{:.0}% ({}/{}; 95% CI {:.0}-{:.0}%)",
            r.value * 100.0,
            r.k,
            r.n,
            r.low * 100.0,
            r.high * 100.0
        ),
        None => "-".into(),
    }
}

pub fn markdown(r: &Report) -> String {
    let mut s = String::from(
        "# Human labels: agreement and accuracy\n\n\
         Generated by `s106 labels` from the submissions in `labels/`. Do not edit; rerun it.\n\
         A disagreement below is a discrepancy for another reader to settle, never a finding about a scheme.\n\n",
    );
    let c = &r.labels;
    if c.cells == 0 {
        s.push_str(
            "**No human labels yet.** Nothing below is a measured accuracy: every figure the \
             pipeline reports is still consistent-with-its-own-citation only.\n\n",
        );
    } else {
        s.push_str(&format!(
            "{} labelled cells ({} rows accepted) across {} scheme(s) by {}; {} minutes recorded.\n\n",
            c.cells,
            c.rows,
            c.schemes,
            c.labellers.join(", "),
            c.minutes
        ));
        s.push_str("| status | labels |\n|---|---|\n");
        for (k, v) in &c.by_status {
            s.push_str(&format!("| {k} | {v} |\n"));
        }
        s.push('\n');
        s.push_str(
            "By method (`read` and `blind` are independent readings; `audited` and `corrected` \
             are decisions on a shown proposal, so anchored on it):\n\n| method | labels |\n|---|---|\n",
        );
        for (k, v) in &c.by_method {
            s.push_str(&format!("| {k} | {v} |\n"));
        }
        s.push('\n');
    }
    if !r.rejected.is_empty() {
        s.push_str("## Rows not used\n\n");
        for x in &r.rejected {
            s.push_str(&format!("- `{}`: {}\n", x.origin, x.reason));
        }
        s.push('\n');
    }

    s.push_str("## Pipeline against the labels\n\n");
    if r.accuracy.is_empty() {
        s.push_str("No labelled cell has a pipeline reading yet.\n\n");
    } else {
        s.push_str(
            "Field by field (`total`, `SR`, `share`, `basis`, ...). Cells where labellers disagree \
             or a source wasn't read are not scored. `basis` counts only where the label states \
             one. The Datahub mapping's V8 scores the completion date only. `labels` says what \
             the row rests on: `independent` labels were read without seeing a proposal; \
             `anchored` ones are confirmations or corrections of a shown proposal, so a \
             `machine:proposal/...` row against anchored labels is how often the person changed \
             the proposal, not an independent accuracy.\n\n\
             | producer | variable | source | labels | cells | not scored | precision | recall |\n\
             |---|---|---|---|---|---|---|---|\n",
        );
        for a in &r.accuracy {
            s.push_str(&format!(
                "| {} | {} | {} | {} | {} | {} | {} | {} |\n",
                a.producer,
                a.variable_id,
                a.source,
                a.labels,
                a.cells,
                a.not_scored,
                rate(&a.precision),
                rate(&a.recall)
            ));
        }
        s.push('\n');
        for a in r.accuracy.iter().filter(|a| !a.errors.is_empty()) {
            s.push_str(&format!(
                "{} {} {} ({} labels):\n",
                a.producer, a.variable_id, a.source, a.labels
            ));
            for e in &a.errors {
                s.push_str(&format!("- {e}\n"));
            }
            s.push('\n');
        }
    }

    s.push_str("## Labellers against each other\n\n");
    if r.labellers.is_empty() {
        s.push_str("No cell has two labellers yet.\n\n");
    } else {
        s.push_str(
            "| labellers | cells | same status | kappa (status) | both found | same value |\n\
             |---|---|---|---|---|---|\n",
        );
        for l in &r.labellers {
            s.push_str(&format!(
                "| {} / {} | {} | {} | {} | {} | {} |\n",
                l.a,
                l.b,
                l.cells,
                l.same_status,
                l.kappa.map_or("-".into(), |k| format!("{k:.2}")),
                l.both_found,
                l.same_value
            ));
        }
        s.push('\n');
        for l in r.labellers.iter().filter(|l| !l.differing.is_empty()) {
            s.push_str(&format!("{} / {} differ on:\n", l.a, l.b));
            for d in &l.differing {
                s.push_str(&format!("- {d}\n"));
            }
            s.push('\n');
        }
    }

    s.push_str("## Sources against each other\n\n");
    if r.sources.is_empty() {
        s.push_str("No scheme has two sources labelled `found` for one variable yet.\n\n");
    } else {
        s.push_str(
            "Schemes where the labels have both sources as `found`; they agree when every field \
             both state is equal.\n\n\
             | variable | sources | schemes | agree | nothing in common | labellers disagree |\n\
             |---|---|---|---|---|---|\n",
        );
        for x in &r.sources {
            s.push_str(&format!(
                "| {} | {} / {} | {} | {} | {} | {} |\n",
                x.variable_id, x.a, x.b, x.schemes, x.agree, x.not_comparable, x.unsettled
            ));
        }
        s.push('\n');
        for x in r.sources.iter().filter(|x| !x.differing.is_empty()) {
            s.push_str(&format!("{} {} / {} differ:\n", x.variable_id, x.a, x.b));
            for d in &x.differing {
                s.push_str(&format!("- {d}\n"));
            }
            s.push('\n');
        }
    }

    if !r.machine_notes.is_empty() {
        s.push_str("## Pipeline readings not made\n\n");
        for n in &r.machine_notes {
            s.push_str(&format!("- {n}\n"));
        }
    }
    s
}

/// Read every submission under `<root>/labels/`, score the pipeline's readings against them,
/// and write `<root>/reports/labels/labels.{md,json}`. Never fails for want of labels.
pub fn run(root: &Path, model: &str) -> Result<Report> {
    let (submitted, mut rejected) = load(&root.join("labels"))?;
    let (mut machine, notes) = machine_readings(root, model);
    let (proposed, bad) = proposals(root)?;
    machine.extend(proposed);
    rejected.extend(bad);
    let r = report(&submitted, rejected, &machine, notes);
    let dir = root.join("reports/labels");
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join("labels.md"), markdown(&r))?;
    std::fs::write(
        dir.join("labels.json"),
        serde_json::to_string_pretty(&r)? + "\n",
    )?;
    Ok(r)
}
