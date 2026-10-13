//! Evaluate the extractor on a set of deeds against the national S106 register.
//!
//! Port of `s106-audit/run_eval.py`. Per deed: k extractions (the first is recorded as a replay
//! fixture, the rest are live), each verified, then
//!   * register scoring: extracted fixed sums vs register amounts, affordable units vs register units
//!   * run agreement: do the k runs report the same housing values, sums, flags and trigger labels?
//!   * gate outcome: pass / review / fail / skip counts and error classes per run
//!
//! The register is a reference, not ground truth: councils split sums into instalments, duplicate
//! rows, and record indexed or partial figures. So a sum "matches" when it equals one register
//! amount (±£1) or the total of one purpose's amounts, and misses are listed for a human to check.
//!
//! Inputs: `data/reference/{candidates.tsv,developer-agreement*.csv}`, `data/raw/eval/<id>.pdf`.
//! Writes `data/processed/eval/<id>/run<k>.audit.json` and `data/processed/eval/summary.{json,md}`.
//! Runs are sequential: PDFium sits behind a process-wide lock and `extract` calls the API synchronously.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::str::FromStr;
use std::time::Instant;

use anyhow::{Context, Result};
use regex::Regex;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::document::{self, Config};
use crate::extract::{self, Mode};
use crate::llm;
use crate::report::AuditJson;
use crate::schema::{Agreement, Housing};
use crate::verify::{self, CheckResult, ErrorClass, Status};

/// A sum matches a register amount when they differ by at most this many pounds.
pub fn tolerance() -> Decimal {
    Decimal::ONE
}

#[derive(Debug, Clone)]
pub struct EvalArgs {
    pub runs: usize,
    pub model: String,
    /// re-verify each deed's recorded fixture instead of calling the API (one run per deed)
    pub replay: bool,
    /// candidate ids to run (all when empty)
    pub only: Vec<String>,
}

// --------------------------------------------------------------------------- register

#[derive(Debug, Clone, Default)]
struct AgreementRow {
    reference: String,
    kind: String,
}

#[derive(Debug, Clone)]
struct ContributionRow {
    purpose: String,
    amount: Option<Decimal>,
    units: Option<Decimal>,
}

/// The planning.data.gov.uk register: agreements by planning reference, contribution rows by agreement reference.
#[derive(Debug, Clone, Default)]
pub struct Register {
    agreements: BTreeMap<String, Vec<AgreementRow>>,
    contributions: BTreeMap<String, Vec<ContributionRow>>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Reference {
    /// (purpose, amount) for every positive register amount
    pub amounts: Vec<(String, Decimal)>,
    pub purpose_totals: BTreeMap<String, Decimal>,
    pub affordable_units: Vec<u32>,
    pub agreement_types: Vec<String>,
}

#[derive(Deserialize)]
struct RawAgreement {
    #[serde(rename = "planning-application", default)]
    planning_application: String,
    #[serde(rename = "developer-agreement-type", default)]
    kind: String,
    #[serde(default)]
    reference: String,
}

#[derive(Deserialize)]
struct RawContribution {
    #[serde(rename = "developer-agreement", default)]
    agreement: String,
    #[serde(rename = "contribution-purpose", default)]
    purpose: String,
    #[serde(default)]
    amount: String,
    #[serde(default)]
    units: String,
}

fn parse_decimal(s: &str) -> Option<Decimal> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    Decimal::from_str(s).or_else(|_| Decimal::from_scientific(s)).ok()
}

pub fn load_register(reference_dir: &Path) -> Result<Register> {
    let mut reg = Register::default();
    let path = reference_dir.join("developer-agreement.csv");
    let mut rdr = csv::ReaderBuilder::new().flexible(true).from_path(&path).with_context(|| format!("open {}", path.display()))?;
    for row in rdr.deserialize::<RawAgreement>() {
        let a = row.with_context(|| format!("read {}", path.display()))?;
        reg.agreements.entry(a.planning_application.trim().to_string()).or_default().push(AgreementRow { reference: a.reference, kind: a.kind });
    }
    let path = reference_dir.join("developer-agreement-contribution.csv");
    let mut rdr = csv::ReaderBuilder::new().flexible(true).from_path(&path).with_context(|| format!("open {}", path.display()))?;
    for row in rdr.deserialize::<RawContribution>() {
        let c = row.with_context(|| format!("read {}", path.display()))?;
        reg.contributions.entry(c.agreement).or_default().push(ContributionRow {
            purpose: c.purpose,
            amount: parse_decimal(&c.amount),
            units: parse_decimal(&c.units),
        });
    }
    Ok(reg)
}

impl Register {
    /// Build a register from in-memory rows: `(planning ref, agreement ref, type)` and
    /// `(agreement ref, purpose, amount, units)`. Used by tests.
    pub fn from_rows(agreements: &[(&str, &str, &str)], contributions: &[(&str, &str, Option<Decimal>, Option<Decimal>)]) -> Register {
        let mut reg = Register::default();
        for (planning, reference, kind) in agreements {
            reg.agreements.entry(planning.trim().to_string()).or_default()
                .push(AgreementRow { reference: reference.to_string(), kind: kind.to_string() });
        }
        for (agreement, purpose, amount, units) in contributions {
            reg.contributions.entry(agreement.to_string()).or_default()
                .push(ContributionRow { purpose: purpose.to_string(), amount: *amount, units: *units });
        }
        reg
    }
}

/// What the register says about one planning reference. CIL is a separate levy, not an obligation
/// in the deed, so those agreements are excluded.
pub fn register_reference(planning_ref: &str, register: &Register) -> Reference {
    let deeds: Vec<&AgreementRow> = register.agreements.get(planning_ref).map_or(vec![], |v| v.iter().collect())
        .into_iter().filter(|a| a.kind != "community-infrastructure-levy").collect();
    let rows: Vec<&ContributionRow> = deeds.iter().flat_map(|a| register.contributions.get(&a.reference).into_iter().flatten()).collect();
    let amounts: Vec<(String, Decimal)> = rows.iter()
        .filter_map(|c| c.amount.filter(|a| *a > Decimal::ZERO).map(|a| (c.purpose.clone(), a))).collect();
    let mut purpose_totals: BTreeMap<String, Decimal> = BTreeMap::new();
    for (purpose, amount) in &amounts {
        *purpose_totals.entry(purpose.clone()).or_default() += *amount;
    }
    let affordable_units: BTreeSet<u32> = rows.iter()
        .filter(|c| c.purpose == "affordable-housing")
        .filter_map(|c| c.units.filter(|u| *u > Decimal::ZERO))
        .filter_map(|u| u.trunc().to_string().parse::<u32>().ok())
        .collect();
    Reference {
        amounts,
        purpose_totals,
        affordable_units: affordable_units.into_iter().collect(),
        agreement_types: deeds.iter().map(|a| a.kind.clone()).collect::<BTreeSet<_>>().into_iter().collect(),
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct RegisterScore {
    pub extracted_sums: usize,
    pub extracted_sums_matched: usize,
    pub register_amounts: usize,
    pub register_amounts_found: usize,
    pub unmatched_extracted: Vec<String>,
    pub missed_register: Vec<String>,
    pub affordable_units: Option<u32>,
    pub register_affordable_units: Vec<u32>,
    pub affordable_units_match: Option<bool>,
}

pub fn score_against_register(agreement: &Agreement, reference: &Reference) -> RegisterScore {
    let sums: Vec<Decimal> = agreement.contributions.iter().filter_map(|c| c.amount.value_gbp).collect();
    let reg_amounts: Vec<Decimal> = reference.amounts.iter().map(|(_, a)| *a).collect();
    let reg_totals: Vec<Decimal> = reference.purpose_totals.values().copied().collect();
    let tol = tolerance();
    let close = |x: Decimal, ys: &[Decimal]| ys.iter().any(|y| (x - *y).abs() <= tol);

    let matched: Vec<bool> = sums.iter().map(|s| close(*s, &reg_amounts) || close(*s, &reg_totals)).collect();
    // a register amount is "found" if some extracted sum equals it, or equals its purpose total
    let found: Vec<bool> = reference.amounts.iter()
        .map(|(p, a)| close(*a, &sums) || reference.purpose_totals.get(p).is_some_and(|t| close(*t, &sums)))
        .collect();
    let unmatched_extracted = sums.iter().zip(&matched).filter(|(_, m)| !**m).map(|(s, _)| s.to_string()).collect();
    let missed_register = reference.amounts.iter().zip(&found).filter(|(_, f)| !**f).map(|((p, a), _)| format!("{p}={a}")).collect();
    let ah = agreement.housing.affordable_units;
    let units_ok = if reference.affordable_units.is_empty() {
        None
    } else {
        Some(ah.is_some_and(|n| reference.affordable_units.contains(&n) || n == reference.affordable_units.iter().sum::<u32>()))
    };
    RegisterScore {
        extracted_sums: sums.len(),
        extracted_sums_matched: matched.iter().filter(|m| **m).count(),
        register_amounts: reg_amounts.len(),
        register_amounts_found: found.iter().filter(|f| **f).count(),
        unmatched_extracted,
        missed_register,
        affordable_units: ah,
        register_affordable_units: reference.affordable_units.clone(),
        affordable_units_match: units_ok,
    }
}

// --------------------------------------------------------------------------- gate

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct GateSummary {
    pub pass: usize,
    pub review: usize,
    pub fail: usize,
    pub skip: usize,
    pub parser_errors: usize,
    pub document_discrepancies: usize,
    pub failures: Vec<String>,
    pub reviews: Vec<String>,
}

pub fn gate_summary(results: &[CheckResult]) -> GateSummary {
    let n = |s: Status| results.iter().filter(|r| r.status == s).count();
    let e = |c: ErrorClass| results.iter().filter(|r| r.error_class == Some(c)).count();
    let lines = |s: Status| results.iter().filter(|r| r.status == s)
        .map(|r| format!("{} {}: {}", r.check_id, r.description, r.detail)).collect::<Vec<_>>();
    GateSummary {
        pass: n(Status::Pass),
        review: n(Status::Review),
        fail: n(Status::Fail),
        skip: n(Status::Skip),
        parser_errors: e(ErrorClass::ParserError),
        document_discrepancies: e(ErrorClass::DocumentDiscrepancy),
        failures: lines(Status::Fail),
        reviews: lines(Status::Review),
    }
}

// --------------------------------------------------------------------------- run agreement

/// One contribution's reported values, in the order of `CONTRIBUTION_FIELDS`.
type ContributionPrint = [String; 5];
const CONTRIBUTION_FIELDS: [&str; 5] = ["amount", "index_linked", "structure", "conditionality", "trigger"];

#[derive(Debug, Clone, PartialEq)]
struct Fingerprint {
    housing: Vec<(&'static str, String)>,
    sums: Vec<String>,
    contributions: BTreeMap<String, ContributionPrint>,
}

fn housing_print(h: &Housing) -> Vec<(&'static str, String)> {
    let mut tenures: Vec<(&str, Option<u32>, Option<u32>)> = h.tenures.iter().map(|t| (t.tenure.label(), t.units, t.habitable_rooms)).collect();
    tenures.sort();
    vec![
        ("total_units", format!("{:?}", h.total_units)),
        ("market_units", format!("{:?}", h.market_units)),
        ("affordable_units", format!("{:?}", h.affordable_units)),
        ("affordable_habitable_rooms", format!("{:?}", h.affordable_habitable_rooms)),
        ("tenures", format!("{tenures:?}")),
        ("claimed_affordable_pct", format!("{:?}", h.claimed_affordable_pct.map(|d| d.normalize().to_string()))),
    ]
}

fn amount_text(d: Option<Decimal>) -> String {
    d.map_or("None".into(), |d| d.normalize().to_string())
}

/// The run's reported values, minus evidence snippets and free text that may legitimately vary.
fn fingerprint(a: &Agreement) -> Fingerprint {
    let name_re = Regex::new(r"\W+").expect("static regex");
    let mut sums: Vec<String> = a.contributions.iter().filter_map(|c| c.amount.value_gbp).map(|d| d.normalize().to_string()).collect();
    sums.sort();
    let contributions = a.contributions.iter().map(|c| {
        let name = name_re.replace_all(&c.name.to_lowercase(), " ").trim().to_string();
        let trigger = if c.payment.instalments.is_empty() {
            c.payment.trigger_event.map_or("None".to_string(), |t| t.label().to_string())
        } else {
            c.payment.instalments.iter().map(|i| i.trigger_event.map_or("?", |t| t.label())).collect::<Vec<_>>().join(" | ")
        };
        (name, [
            amount_text(c.amount.value_gbp),
            c.amount.index_linked.to_string(),
            format!("{:?}", c.amount.structure),
            format!("{:?}", c.conditionality.kind),
            trigger,
        ])
    }).collect();
    Fingerprint { housing: housing_print(&a.housing), sums, contributions }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ContributionNames {
    pub union: usize,
    pub in_every_run: usize,
    pub only_some_runs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct AcrossRuns {
    pub runs_compared: usize,
    /// the remaining fields are meaningful only when `runs_compared >= 2`
    pub housing_identical: bool,
    pub housing_diffs: Vec<String>,
    pub sums_identical: bool,
    pub contribution_names: ContributionNames,
    pub field_diffs: Vec<String>,
}

impl AcrossRuns {
    pub fn compared(&self) -> bool {
        self.runs_compared >= 2
    }

    /// Every compared value matched in every run.
    pub fn all_agree(&self) -> bool {
        self.housing_identical && self.sums_identical && self.field_diffs.is_empty() && self.contribution_names.only_some_runs.is_empty()
    }
}

/// Do the runs report the same housing values, sums, contribution set, flags and trigger labels?
pub fn agreement_across(agreements: &[&Agreement]) -> AcrossRuns {
    let fps: Vec<Fingerprint> = agreements.iter().map(|a| fingerprint(a)).collect();
    let mut out = AcrossRuns {
        runs_compared: fps.len(),
        housing_identical: true,
        housing_diffs: vec![],
        sums_identical: true,
        contribution_names: ContributionNames { union: 0, in_every_run: 0, only_some_runs: vec![] },
        field_diffs: vec![],
    };
    if fps.len() < 2 {
        return out;
    }
    let names: BTreeSet<&String> = fps.iter().flat_map(|f| f.contributions.keys()).collect();
    let common: Vec<&String> = names.iter().copied().filter(|n| fps.iter().all(|f| f.contributions.contains_key(*n))).collect();
    for n in &common {
        let vals: Vec<&ContributionPrint> = fps.iter().map(|f| &f.contributions[*n]).collect();
        for (i, field) in CONTRIBUTION_FIELDS.iter().enumerate() {
            if vals.iter().any(|v| v[i] != vals[0][i]) {
                out.field_diffs.push(format!("{n}: {field} {:?}", vals.iter().map(|v| &v[i]).collect::<Vec<_>>()));
            }
        }
    }
    out.housing_diffs = fps[0].housing.iter().enumerate()
        .filter(|(i, _)| fps.iter().any(|f| f.housing[*i].1 != fps[0].housing[*i].1))
        .map(|(_, (k, _))| k.to_string()).collect();
    out.housing_identical = out.housing_diffs.is_empty();
    out.sums_identical = fps.iter().all(|f| f.sums == fps[0].sums);
    out.contribution_names = ContributionNames {
        union: names.len(),
        in_every_run: common.len(),
        only_some_runs: names.iter().filter(|n| !common.contains(n)).map(|n| n.to_string()).collect(),
    };
    out
}

// --------------------------------------------------------------------------- runs

#[derive(Debug, Clone, Serialize)]
pub struct RunRow {
    pub run: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gate: Option<GateSummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub register: Option<RegisterScore>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contributions: Option<usize>,
    pub seconds: f64,
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub cost_usd: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct DeedSummary {
    pub id: String,
    pub council: String,
    pub planning_ref: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub register_types: Vec<String>,
    pub pages: Option<usize>,
    pub text_source: Option<String>,
    pub runs: Vec<RunRow>,
    pub agreement: AcrossRuns,
}

#[derive(Debug, Clone, Deserialize)]
struct Candidate {
    id: String,
    #[serde(default)]
    council: String,
    #[serde(default)]
    planning_ref: String,
    #[serde(rename = "type", default)]
    kind: String,
}

fn error_row(run: usize, error: String, seconds: f64) -> RunRow {
    RunRow { run, error: Some(error), gate: None, register: None, contributions: None, seconds, tokens_in: 0, tokens_out: 0, cost_usd: 0.0 }
}

fn round(x: f64, dp: i32) -> f64 {
    let m = 10f64.powi(dp);
    (x * m).round() / m
}

/// One extraction + verification; writes the run's audit JSON. Failure becomes an error row,
/// so one failed run does not sink the deed's other runs.
fn run_once(doc: &document::Document, run: usize, model: &str, mode: Mode, out: &Path, client: Option<&rust_claude_sdk::Client>,
            reference: &Reference) -> (RunRow, Option<Agreement>) {
    let t0 = Instant::now();
    let attempt = || -> Result<(RunRow, Agreement)> {
        let (mut agreement, info) = extract::extract(doc, model, mode, client)?;
        let results = verify::verify(&agreement, &doc.pages());
        let (tin, tout) = info.usage.as_ref().map_or((0, 0), |u| (u.input_tokens, u.output_tokens));
        let cost = match (&info.usage, mode) {
            (Some(u), Mode::Live | Mode::Record) => llm::cost_usd(&info.model, u),
            _ => 0.0,
        };
        if let Some(u) = &info.usage {
            agreement.warnings.push(format!("{}: {} in / {} out tokens, ~${:.3}{}", info.source, u.input_tokens, u.output_tokens,
                llm::cost_usd(&info.model, u), if info.source == "live" { "" } else { " when recorded" }));
        }
        std::fs::write(out, serde_json::to_string_pretty(&AuditJson { agreement: &agreement, verification: &results })?)
            .with_context(|| format!("write {}", out.display()))?;
        let row = RunRow {
            run,
            error: None,
            gate: Some(gate_summary(&results)),
            register: Some(score_against_register(&agreement, reference)),
            contributions: Some(agreement.contributions.len()),
            seconds: round(t0.elapsed().as_secs_f64(), 1),
            tokens_in: tin,
            tokens_out: tout,
            cost_usd: round(cost, 4),
        };
        Ok((row, agreement))
    };
    match attempt() {
        Ok((row, agreement)) => (row, Some(agreement)),
        Err(e) => {
            let mut msg = format!("{e:#}");
            msg.truncate(msg.char_indices().nth(400).map_or(msg.len(), |(i, _)| i));
            (error_row(run, msg, round(t0.elapsed().as_secs_f64(), 1)), None)
        }
    }
}

fn evaluate(row: &Candidate, runs: usize, model: &str, register: &Register, client: Option<&rust_claude_sdk::Client>, replay: bool,
            root: &Path, cfg: &Config) -> Result<DeedSummary> {
    let pdf = root.join("data/raw/eval").join(format!("{}.pdf", row.id));
    let out = root.join("data/processed/eval").join(&row.id);
    std::fs::create_dir_all(&out)?;
    // replay re-verifies the recorded response: one run, free, offline, reproducible
    let runs = if replay { 1 } else { runs.max(1) };
    let reference = register_reference(&row.planning_ref, register);

    let mut rows: Vec<RunRow> = vec![];
    let mut ok: Vec<Agreement> = vec![];
    if !pdf.exists() {
        rows.push(error_row(1, format!("corpus missing: fetch with data/reference/candidates.tsv (no {})", pdf.strip_prefix(root).unwrap_or(&pdf).display()), 0.0));
    } else {
        match document::load(&pdf, cfg, false) {
            Err(e) => rows.push(error_row(1, format!("load {}: {e:#}", pdf.display()), 0.0)),
            Ok(doc) => {
                for k in 1..=runs {
                    // run 1 records the fixture (and warms the OCR cache); the rest are live
                    let mode = if replay { Mode::Replay } else if k == 1 { Mode::Record } else { Mode::Live };
                    let (r, a) = run_once(&doc, k, model, mode, &out.join(format!("run{k}.audit.json")), client, &reference);
                    rows.push(r);
                    ok.extend(a);
                }
            }
        }
    }
    let (pages, text_source) = match ok.first() {
        Some(a) => (Some(a.provenance.page_count), Some(a.provenance.text_source.clone())),
        None => (None, None),
    };
    let refs: Vec<&Agreement> = ok.iter().collect();
    Ok(DeedSummary {
        id: row.id.clone(),
        council: row.council.clone(),
        planning_ref: row.planning_ref.clone(),
        kind: row.kind.clone(),
        register_types: reference.agreement_types,
        pages,
        text_source,
        runs: rows,
        agreement: agreement_across(&refs),
    })
}

// --------------------------------------------------------------------------- writers

fn thousands(n: u64) -> String {
    let s = n.to_string();
    let mut o = String::new();
    for (i, ch) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            o.push(',');
        }
        o.push(ch);
    }
    o
}

fn cell(s: &str, max: usize) -> String {
    s.replace(['\n', '|'], " ").chars().take(max).collect()
}

pub fn markdown(summary: &[DeedSummary], model: &str) -> String {
    let mut lines: Vec<String> = vec![
        format!("# Register eval: {model}"),
        String::new(),
        "| deed | pp | src | runs ok | gate (pass/review/fail/skip, parser err) | sums matched | register found | AH units (ours / register) | runs agree | cost |".into(),
        "|---|---|---|---|---|---|---|---|---|---|".into(),
    ];
    let mut total = 0.0;
    if summary.iter().any(|d| d.runs.iter().any(|r| r.error.as_deref().is_some_and(|e| e.starts_with("corpus missing")))) {
        lines.insert(2, "Some deeds are missing from data/raw/eval: corpus missing: fetch with data/reference/candidates.tsv".into());
        lines.insert(3, String::new());
    }
    for d in summary {
        let ok: Vec<&RunRow> = d.runs.iter().filter(|r| r.gate.is_some()).collect();
        let c: f64 = ok.iter().map(|r| r.cost_usd).sum();
        total += c;
        let Some(first) = ok.first() else {
            lines.push(format!("| {} | | | 0/{} | {} | | | | | |", d.id, d.runs.len(), cell(d.runs.first().and_then(|r| r.error.as_deref()).unwrap_or(""), 60)));
            continue;
        };
        let (g, reg) = (first.gate.as_ref().unwrap(), first.register.as_ref().unwrap());
        let ag = &d.agreement;
        let agree = if !ag.compared() { "n/a" } else if ag.all_agree() { "yes" } else { "no" };
        let units = format!("{} / {}", reg.affordable_units.map_or("None".into(), |u| u.to_string()),
            if reg.register_affordable_units.is_empty() { "-".into() } else { format!("{:?}", reg.register_affordable_units) });
        lines.push(format!("| {} | {} | {} | {}/{} | {}/{}/{}/{}, {} | {}/{} | {}/{} | {units} | {agree} | ${c:.3} |",
            d.id, d.pages.map_or(String::new(), |p| p.to_string()), d.text_source.as_deref().unwrap_or(""), ok.len(), d.runs.len(),
            g.pass, g.review, g.fail, g.skip, g.parser_errors,
            reg.extracted_sums_matched, reg.extracted_sums, reg.register_amounts_found, reg.register_amounts));
    }
    lines.push(String::new());
    lines.push(format!("Total cost: ${total:.2}"));
    lines.push(String::new());
    lines.push("Gate and register columns are for run 1; *runs agree* compares all runs (housing, sums, contribution set, flags, trigger labels).".into());
    lines.push(String::new());
    for d in summary {
        lines.push(format!("## {} ({} {}, {})", d.id, d.council, d.planning_ref, d.kind));
        for r in &d.runs {
            let (Some(g), Some(reg)) = (&r.gate, &r.register) else {
                lines.push(format!("- run {}: **error** {}", r.run, r.error.as_deref().unwrap_or("")));
                continue;
            };
            lines.push(format!("- run {}: gate {}/{}/{}/{}; {} contributions; {} in / {} out; {}s", r.run, g.pass, g.review, g.fail, g.skip,
                r.contributions.unwrap_or(0), thousands(r.tokens_in), thousands(r.tokens_out), r.seconds));
            lines.extend(g.failures.iter().map(|f| format!("  - FAIL {f}")));
            lines.extend(g.reviews.iter().map(|f| format!("  - REVIEW {f}")));
            if !reg.unmatched_extracted.is_empty() {
                lines.push(format!("  - sums not in register: {}", reg.unmatched_extracted.join(", ")));
            }
            if !reg.missed_register.is_empty() {
                lines.push(format!("  - register amounts not extracted: {}", reg.missed_register.join(", ")));
            }
        }
        let ag = &d.agreement;
        if ag.compared() {
            if !ag.housing_diffs.is_empty() {
                lines.push(format!("- housing differs across runs: {}", ag.housing_diffs.join(", ")));
            }
            if !ag.sums_identical {
                lines.push("- extracted sums differ across runs".into());
            }
            if !ag.contribution_names.only_some_runs.is_empty() {
                lines.push(format!("- contributions in only some runs: {}", ag.contribution_names.only_some_runs.join(", ")));
            }
            lines.extend(ag.field_diffs.iter().map(|f| format!("- field differs: {f}")));
        }
        lines.push(String::new());
    }
    lines.join("\n")
}

// --------------------------------------------------------------------------- main

fn read_candidates(path: &Path) -> Result<Vec<Candidate>> {
    let mut rdr = csv::ReaderBuilder::new().delimiter(b'\t').flexible(true).from_path(path).with_context(|| format!("open {}", path.display()))?;
    rdr.deserialize::<Candidate>().collect::<Result<_, _>>().with_context(|| format!("read {}", path.display()))
}

pub fn run(args: &EvalArgs, repo_root: &Path) -> Result<()> {
    let reference_dir = repo_root.join("data/reference");
    let out = repo_root.join("data/processed/eval");
    let mut rows = read_candidates(&reference_dir.join("candidates.tsv"))?;
    if !args.only.is_empty() {
        rows.retain(|r| args.only.contains(&r.id));
    }
    let register = load_register(&reference_dir)?;
    let client = if args.replay { None } else { Some(llm::client()?) };
    let cfg = Config::from_env(repo_root);
    std::fs::create_dir_all(&out)?;
    let mut summary: Vec<DeedSummary> = vec![];
    for row in &rows {
        eprintln!("== {} ({} {})", row.id, row.council, row.planning_ref);
        summary.push(evaluate(row, args.runs, &args.model, &register, client.as_ref(), args.replay, repo_root, &cfg)?);
        std::fs::write(out.join("summary.json"), serde_json::to_string_pretty(&summary)?)?;
    }
    let md = markdown(&summary, &args.model);
    std::fs::write(out.join("summary.md"), &md)?;
    println!("{md}");
    Ok(())
}
