//! Results: what a batch's sources say, scheme by scheme, and how far a person has checked it.
//!
//! `s106 results` reads a batch's blank labelling sheet (which cells exist), the human labels in
//! `labels/` and the agents' proposals in `proposals/`, and writes one folder:
//!
//! - `ledger.csv`: one row per scheme × variable × source, set out along the scheme's lineage
//!   (offer, decision, obligation, variation, completion), with the best reading of each cell and
//!   its check level: `independent` (a person read the source), `anchored` (a person confirmed or
//!   corrected an agent's proposal), `proposed` (an agent only), `unsettled` (labellers disagree)
//!   or `unread`;
//! - `rules.csv`: the batch's pre-registered decision rules worked out on the ledger, scheme by
//!   scheme, each outcome carrying the lowest check level it rests on;
//! - `retrieval.csv`: per scheme and source, which documents are in hand and which are not;
//! - `README.md`: the note, including the written claims from the folder's hand-kept `claims.csv`,
//!   grouped by the check level of the cells each one cites.
//!
//! Like `s106 labels`, nothing waits on this: with no labels every cell is `proposed` or
//! `unread`, and the note says so. A rule outcome or a claim at the `proposed` level is not a
//! result. A difference between two sources is a cell for a person to check, never a breach.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::labels::{self, Reading};

/// Who stands behind a cell, weakest first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Check {
    Unread,
    Unsettled,
    Proposed,
    Anchored,
    Independent,
}

impl Check {
    pub fn as_str(self) -> &'static str {
        match self {
            Check::Unread => "unread",
            Check::Unsettled => "unsettled",
            Check::Proposed => "proposed",
            Check::Anchored => "anchored",
            Check::Independent => "independent",
        }
    }
}

/// The lineage stages, in order. A cell's stage follows from its variable and source.
pub const STAGES: [&str; 5] = ["offer", "decision", "obligation", "variation", "completion"];

pub fn stage(variable_id: &str, source: &str) -> &'static str {
    match (variable_id, source) {
        ("V3", _) => "offer",
        ("V7", _) => "variation",
        ("V8", _) | (_, "epc") => "completion",
        (_, "deed") => "obligation",
        // committee and GLA reports, and the Datahub's permitted figures
        _ => "decision",
    }
}

/// One cell of the ledger.
#[derive(Debug, Clone, Serialize)]
pub struct Entry {
    pub scheme: String,
    pub stage: &'static str,
    pub variable_id: String,
    pub variable: String,
    pub source: String,
    pub status: String,
    pub value: String,
    pub basis: String,
    pub doc_ids: String,
    pub pages: String,
    pub check: Check,
    /// the labeller or producer of the reading shown
    pub reader: String,
    /// `<file>:<line>` of the reading shown
    pub origin: String,
}

impl Entry {
    fn fields(&self) -> BTreeMap<String, String> {
        Reading {
            scheme: self.scheme.clone(),
            variable_id: self.variable_id.clone(),
            source: self.source.clone(),
            value: self.value.clone(),
            basis: self.basis.clone(),
            status: self.status.clone(),
            labeller: String::new(),
            labelled_on: String::new(),
            minutes: None,
            doc_ids: String::new(),
            pages: String::new(),
            method: String::new(),
            origin: String::new(),
        }
        .fields()
    }
}

type Key = (String, String, String);

fn key(r: &Reading) -> Key {
    (r.scheme.clone(), r.variable_id.clone(), r.source.clone())
}

/// The cells a batch's blank sheet lists: (scheme, variable_id, source) -> variable name.
fn sheet_cells(path: &Path) -> Result<BTreeMap<Key, String>> {
    let mut r =
        csv::Reader::from_path(path).with_context(|| format!("opening {}", path.display()))?;
    let mut out = BTreeMap::new();
    for row in r.deserialize::<BTreeMap<String, String>>() {
        let row = row?;
        let get = |c: &str| row.get(c).map(|v| v.trim().to_string()).unwrap_or_default();
        out.insert(
            (
                get("scheme").to_uppercase(),
                get("variable_id").to_uppercase(),
                get("source").to_lowercase(),
            ),
            get("variable"),
        );
    }
    Ok(out)
}

/// The best reading of every cell. The sheet's cells all appear; a cell that only labels or
/// proposals name (a source carrying a variable nobody expected) is added for the sheet's schemes.
/// Labels outrank proposals; among proposals, the one in the later file (by path) wins, which is
/// how a newer producer's file supersedes an older one.
pub fn ledger(
    sheet: &BTreeMap<Key, String>,
    submitted: &[Reading],
    proposed: &[Reading],
) -> Vec<Entry> {
    let schemes: BTreeSet<&String> = sheet.keys().map(|k| &k.0).collect();
    let names: BTreeMap<&str, &str> = sheet
        .iter()
        .map(|(k, v)| (k.1.as_str(), v.as_str()))
        .collect();
    let mut labels: BTreeMap<Key, Vec<Reading>> = BTreeMap::new();
    for r in labels::latest(submitted) {
        labels.entry(key(&r)).or_default().push(r);
    }
    let mut proposals: BTreeMap<Key, &Reading> = BTreeMap::new();
    for r in proposed {
        proposals.insert(key(r), r);
    }
    let mut keys: BTreeSet<Key> = sheet.keys().cloned().collect();
    keys.extend(
        labels
            .keys()
            .chain(proposals.keys())
            .filter(|k| schemes.contains(&k.0))
            .cloned(),
    );
    keys.into_iter()
        .map(|k| {
            let blank = |check: Check| Entry {
                scheme: k.0.clone(),
                stage: stage(&k.1, &k.2),
                variable_id: k.1.clone(),
                variable: names.get(k.1.as_str()).unwrap_or(&"").to_string(),
                source: k.2.clone(),
                status: String::new(),
                value: String::new(),
                basis: String::new(),
                doc_ids: String::new(),
                pages: String::new(),
                check,
                reader: String::new(),
                origin: String::new(),
            };
            let shown = |r: &Reading, check: Check| Entry {
                status: r.status.clone(),
                value: r.value.clone(),
                basis: r.basis.clone(),
                doc_ids: r.doc_ids.clone(),
                pages: r.pages.clone(),
                reader: r.labeller.clone(),
                origin: r.origin.clone(),
                ..blank(check)
            };
            match (labels.get(&k), proposals.get(&k)) {
                (Some(ls), _) => {
                    let agree = ls
                        .iter()
                        .all(|r| r.status == ls[0].status && r.fields() == ls[0].fields());
                    if !agree {
                        let mut e = blank(Check::Unsettled);
                        e.value = ls
                            .iter()
                            .map(|r| format!("{}: {} `{}`", r.labeller, r.status, r.value))
                            .collect::<Vec<_>>()
                            .join(" / ");
                        return e;
                    }
                    // An independent reading, where there is one, is the one shown.
                    let best = ls
                        .iter()
                        .find(|r| labels::label_kind(&r.method) == "independent")
                        .unwrap_or(&ls[0]);
                    let check = match labels::label_kind(&best.method) {
                        "independent" => Check::Independent,
                        _ => Check::Anchored,
                    };
                    shown(best, check)
                }
                (None, Some(p)) => shown(p, Check::Proposed),
                (None, None) => blank(Check::Unread),
            }
        })
        .collect()
}

// -------------------------------------------------------------------------------------- rules

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    /// the scheme meets the rule's condition
    Counts,
    /// it doesn't
    DoesNotCount,
    /// the cells can't decide it (a reading is ambiguous, or a source states nothing usable);
    /// counted against the rule, since the rule needs a positive finding
    Indeterminate,
    /// outside the rule's count, as the rule itself says (a report not retrieved, a scheme not
    /// recorded as completed)
    LeftOut,
}

#[derive(Debug, Clone, Serialize)]
pub struct SchemeOutcome {
    pub rule: &'static str,
    pub scheme: String,
    pub outcome: Outcome,
    pub reason: String,
    /// the weakest check level among the cells that decided this outcome
    pub check: Check,
}

#[derive(Debug, Serialize)]
pub struct Rule {
    pub id: &'static str,
    pub title: &'static str,
    /// the threshold as the batch's README fixes it
    pub threshold: &'static str,
    pub counted: usize,
    pub counts: usize,
    pub needed: usize,
    pub met: bool,
    /// the weakest check level among all outcomes: the rule's verdict is a result only when this
    /// is `anchored` or `independent`
    pub check: Check,
    pub schemes: Vec<SchemeOutcome>,
}

struct Cells<'a>(BTreeMap<Key, &'a Entry>);

impl<'a> Cells<'a> {
    fn get(&self, scheme: &str, variable_id: &str, source: &str) -> Option<&'a Entry> {
        self.0
            .get(&(scheme.into(), variable_id.into(), source.into()))
            .copied()
    }
}

fn weakest<'a>(cells: impl IntoIterator<Item = Option<&'a Entry>>) -> Check {
    cells
        .into_iter()
        .map(|c| c.map_or(Check::Unread, |c| c.check))
        .min()
        .unwrap_or(Check::Unread)
}

fn status(c: Option<&Entry>) -> &str {
    match c {
        Some(c) if !c.status.is_empty() => &c.status,
        _ => "unread",
    }
}

/// Retrieval gaps that a rule leaves out of its count rather than holding against a scheme.
fn not_in_hand(c: Option<&Entry>) -> bool {
    matches!(status(c), "source_missing" | "no_source" | "unread")
}

const NOT_AFFORDABLE: [&str; 3] = ["total", "market", "basis"];

/// The affordable tenures a V1 reading states, or `None` when it gives no affordable quantity at
/// all (no tenure, and no market figure equal to the total to say there are none).
fn affordable(fields: &BTreeMap<String, String>) -> Option<BTreeMap<String, String>> {
    let tenures: BTreeMap<String, String> = fields
        .iter()
        .filter(|(k, _)| !NOT_AFFORDABLE.contains(&k.as_str()))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    let all_market = fields.contains_key("total") && fields.get("market") == fields.get("total");
    (!tenures.is_empty() || all_market).then_some(tenures)
}

fn shown(m: &BTreeMap<String, String>) -> String {
    if m.is_empty() {
        return "none".into();
    }
    m.iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("; ")
}

/// Rule 1: the committee report states the affordable quantity, its tenure split and its
/// measurement basis, and all three equal the original executed deed. Quantity and split are
/// read from V1's affordable tenures, the basis from V1's basis; where both V2 readings state a
/// split or a basis, those must agree too. A report not in hand is left out.
fn rule1(cells: &Cells, schemes: &[String]) -> Vec<SchemeOutcome> {
    schemes
        .iter()
        .map(|s| {
            let (r1, d1) = (
                cells.get(s, "V1", "committee_report"),
                cells.get(s, "V1", "deed"),
            );
            let (r2, d2) = (
                cells.get(s, "V2", "committee_report"),
                cells.get(s, "V2", "deed"),
            );
            let out = |outcome, reason: String, used: Vec<Option<&Entry>>| SchemeOutcome {
                rule: "1",
                scheme: s.clone(),
                outcome,
                reason,
                check: weakest(used),
            };
            if not_in_hand(r1) {
                return out(
                    Outcome::LeftOut,
                    format!("committee report V1 {}", status(r1)),
                    vec![r1],
                );
            }
            // V2 counts towards the level only where both readings are compared (below)
            let mut used = vec![r1, d1];
            if status(r1) != "found" {
                return out(
                    Outcome::DoesNotCount,
                    format!("committee report V1 {}", status(r1)),
                    vec![r1],
                );
            }
            if status(d1) != "found" {
                return out(
                    Outcome::Indeterminate,
                    format!("deed V1 {}", status(d1)),
                    used,
                );
            }
            let (rf, df) = (r1.unwrap().fields(), d1.unwrap().fields());
            let Some(ra) = affordable(&rf) else {
                return out(
                    Outcome::DoesNotCount,
                    "the committee report gives no affordable quantity".into(),
                    used,
                );
            };
            let Some(da) = affordable(&df) else {
                return out(
                    Outcome::Indeterminate,
                    "the deed's V1 gives no affordable quantity".into(),
                    used,
                );
            };
            let mut differ = vec![];
            if ra != da {
                differ.push(format!(
                    "tenures: report {}, deed {}",
                    shown(&ra),
                    shown(&da)
                ));
            }
            if rf.get("basis") != df.get("basis") {
                differ.push(format!(
                    "V1 basis: report {}, deed {}",
                    rf.get("basis").map_or("none", |b| b),
                    df.get("basis").map_or("none", |b| b)
                ));
            }
            if let (Some(r2), Some(d2)) = (r2, d2)
                && r2.status == "found"
                && d2.status == "found"
            {
                used.extend([Some(r2), Some(d2)]);
                let (rf2, df2) = (r2.fields(), d2.fields());
                for k in ["split", "basis"] {
                    if let (Some(a), Some(b)) = (rf2.get(k), df2.get(k))
                        && a != b
                    {
                        differ.push(format!("V2 {k}: report {a}, deed {b}"));
                    }
                }
            }
            if differ.is_empty() {
                out(
                    Outcome::Counts,
                    format!("report and deed agree: {}", shown(&ra)),
                    used,
                )
            } else {
                out(Outcome::DoesNotCount, differ.join("; "), used)
            }
        })
        .collect()
}

fn number(v: &str) -> Option<f64> {
    v.parse().ok()
}

/// Rule 2: for each scheme the Datahub records as completed, the counts agree when
/// |Datahub completed homes − EPC new-dwelling certificates| ≤ 5% of the Datahub figure.
fn rule2(cells: &Cells, schemes: &[String]) -> Vec<SchemeOutcome> {
    schemes
        .iter()
        .map(|s| {
            let (dh, epc) = (cells.get(s, "V8", "datahub"), cells.get(s, "V8", "epc"));
            let out = |outcome, reason: String, used: Vec<Option<&Entry>>| SchemeOutcome {
                rule: "2",
                scheme: s.clone(),
                outcome,
                reason,
                check: weakest(used),
            };
            let homes = dh
                .filter(|d| d.status == "found")
                .and_then(|d| d.fields().get("completed_homes").and_then(|v| number(v)));
            let Some(homes) = homes else {
                return out(
                    Outcome::LeftOut,
                    format!("Datahub records no completed homes (V8 {})", status(dh)),
                    vec![dh],
                );
            };
            let certs = epc
                .filter(|e| e.status == "found")
                .and_then(|e| e.fields().get("new_dwelling_epcs").and_then(|v| number(v)));
            let Some(certs) = certs else {
                return out(
                    Outcome::Indeterminate,
                    format!(
                        "EPC V8 {}: {}",
                        status(epc),
                        epc.map_or("", |e| e.value.as_str())
                    ),
                    vec![dh, epc],
                );
            };
            let gap = (homes - certs).abs() / homes;
            let reason = format!(
                "Datahub {homes} completed, EPC {certs} new-dwelling certificates ({:.1}% apart)",
                gap * 100.0
            );
            let outcome = if gap <= 0.05 {
                Outcome::Counts
            } else {
                Outcome::DoesNotCount
            };
            out(outcome, reason, vec![dh, epc])
        })
        .collect()
}

/// Whether a V4 reading describes a review mechanism: `None` when it can't say.
fn describes_review(c: &Entry) -> Option<bool> {
    match c.status.as_str() {
        "found" => Some(
            c.fields()
                .get("reviews")
                .is_some_and(|r| r != "none" && r != "not_stated"),
        ),
        "not_stated" | "not_applicable" => Some(false),
        _ => None,
    }
}

/// Rule 3: the deed's V4 records at least one review mechanism and neither the committee
/// report nor the GLA report (where retrieved) describes one.
fn rule3(cells: &Cells, schemes: &[String]) -> Vec<SchemeOutcome> {
    schemes
        .iter()
        .map(|s| {
            let deed = cells.get(s, "V4", "deed");
            // the reports in hand, which are the only ones the rule consults
            let reports: Vec<(&str, Option<&Entry>)> = ["committee_report", "gla"]
                .into_iter()
                .map(|src| (src, cells.get(s, "V4", src)))
                .filter(|(_, c)| !not_in_hand(*c))
                .collect();
            let (outcome, reason) = match deed.and_then(describes_review) {
                None => (Outcome::Indeterminate, format!("deed V4 {}", status(deed))),
                Some(false) => (Outcome::DoesNotCount, "the deed records no review".into()),
                Some(true) => reports
                    .iter()
                    .find_map(|(src, c)| match c.and_then(describes_review) {
                        None => Some((Outcome::Indeterminate, format!("{src} V4 {}", status(*c)))),
                        Some(true) => Some((
                            Outcome::DoesNotCount,
                            format!("the {src} describes a review too"),
                        )),
                        Some(false) => None,
                    })
                    .unwrap_or_else(|| {
                        (
                            Outcome::Counts,
                            format!(
                                "deed: {}; no report in hand describes a review",
                                deed.map_or("", |d| d.value.as_str())
                            ),
                        )
                    }),
            };
            SchemeOutcome {
                rule: "3",
                scheme: s.clone(),
                outcome,
                reason,
                check: weakest(std::iter::once(deed).chain(reports.iter().map(|(_, c)| *c))),
            }
        })
        .collect()
}

fn rule(
    id: &'static str,
    title: &'static str,
    threshold: &'static str,
    schemes: Vec<SchemeOutcome>,
    needed: impl Fn(usize) -> usize,
) -> Rule {
    let counted = schemes
        .iter()
        .filter(|o| o.outcome != Outcome::LeftOut)
        .count();
    let counts = schemes
        .iter()
        .filter(|o| o.outcome == Outcome::Counts)
        .count();
    let needed = needed(counted);
    Rule {
        id,
        title,
        threshold,
        counted,
        counts,
        needed,
        met: counted > 0 && counts >= needed,
        check: schemes
            .iter()
            .map(|o| o.check)
            .min()
            .unwrap_or(Check::Unread),
        schemes,
    }
}

/// The source-audit batch's three decision rules, as fixed in its README on 14 Oct 2026 before
/// any value was read.
pub fn rules(ledger: &[Entry]) -> Vec<Rule> {
    let cells = Cells(
        ledger
            .iter()
            .map(|e| {
                (
                    (e.scheme.clone(), e.variable_id.clone(), e.source.clone()),
                    e,
                )
            })
            .collect(),
    );
    let schemes: Vec<String> = ledger
        .iter()
        .map(|e| e.scheme.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    vec![
        rule(
            "1",
            "Offer record from reports: the committee report's affordable quantity, split and basis equal the deed's",
            "all but one of the schemes counted (7 of 8; a report not in hand is left out)",
            rule1(&cells, &schemes),
            |n| n.saturating_sub(1),
        ),
        rule(
            "2",
            "Datahub delivery against EPC: completed homes within 5% of new-dwelling certificates",
            "at least 75% of the schemes the Datahub records as completed (3 of 4 in batch 1)",
            rule2(&cells, &schemes),
            |n| (n * 3).div_ceil(4),
        ),
        rule(
            "3",
            "Reviews only in the deed: the deed has a review mechanism the reports don't describe",
            "at least 5 of the 8 schemes",
            rule3(&cells, &schemes),
            |_| 5,
        ),
    ]
}

// ---------------------------------------------------------------------------- retrieval, claims

#[derive(Debug, Serialize)]
pub struct Retrieval {
    pub scheme: String,
    pub source: String,
    pub documents: usize,
    pub in_hand: usize,
    /// status of each document not in hand, e.g. `listed_not_fetched x3`
    pub not_in_hand: String,
}

/// Per scheme and source, from the batch's `sources.csv` (next to the sheet). Missing file: none.
fn retrieval(path: &Path) -> Result<Vec<Retrieval>> {
    if !path.exists() {
        return Ok(vec![]);
    }
    let mut r = csv::Reader::from_path(path)?;
    let mut by: BTreeMap<(String, String), Vec<String>> = BTreeMap::new();
    for row in r.deserialize::<BTreeMap<String, String>>() {
        let row = row?;
        let get = |c: &str| row.get(c).cloned().unwrap_or_default();
        by.entry((get("scheme"), get("source")))
            .or_default()
            .push(get("status"));
    }
    Ok(by
        .into_iter()
        .map(|((scheme, source), statuses)| {
            let in_hand = |s: &str| matches!(s, "downloaded" | "snapshot_saved");
            let mut missing: BTreeMap<&str, usize> = BTreeMap::new();
            for s in statuses.iter().filter(|s| !in_hand(s)) {
                *missing.entry(s).or_default() += 1;
            }
            Retrieval {
                documents: statuses.len(),
                in_hand: statuses.iter().filter(|s| in_hand(s)).count(),
                not_in_hand: missing
                    .iter()
                    .map(|(s, n)| format!("{s} x{n}"))
                    .collect::<Vec<_>>()
                    .join(", "),
                scheme,
                source,
            }
        })
        .collect())
}

/// A written claim from `claims.csv`, with what the ledger says about the cells it cites.
#[derive(Debug, Serialize)]
pub struct Claim {
    pub id: String,
    pub claim: String,
    /// `<scheme>/<variable_id>/<source>` ids, as the proposals and audit app use
    pub cells: Vec<String>,
    /// it is about an identifiable scheme, developer or council
    pub specific: bool,
    /// who signed it off for use outside the private repo, if anyone
    pub signed_off: String,
    pub check: Check,
    /// cell ids the ledger doesn't hold
    pub unknown: Vec<String>,
}

fn claims(path: &Path, ledger: &[Entry]) -> Result<Vec<Claim>> {
    if !path.exists() {
        return Ok(vec![]);
    }
    let by_id: BTreeMap<String, &Entry> = ledger
        .iter()
        .map(|e| (format!("{}/{}/{}", e.scheme, e.variable_id, e.source), e))
        .collect();
    let mut r = csv::Reader::from_path(path)?;
    let mut out = vec![];
    for row in r.deserialize::<BTreeMap<String, String>>() {
        let row = row?;
        let get = |c: &str| row.get(c).map(|v| v.trim().to_string()).unwrap_or_default();
        let cells: Vec<String> = get("cells").split_whitespace().map(String::from).collect();
        let unknown: Vec<String> = cells
            .iter()
            .filter(|c| !by_id.contains_key(*c))
            .cloned()
            .collect();
        let check = if cells.is_empty() || !unknown.is_empty() {
            Check::Unread
        } else {
            cells
                .iter()
                .map(|c| by_id[c].check)
                .min()
                .unwrap_or(Check::Unread)
        };
        out.push(Claim {
            id: get("id"),
            claim: get("claim"),
            specific: get("specific").eq_ignore_ascii_case("yes"),
            signed_off: get("signed_off"),
            cells,
            check,
            unknown,
        });
    }
    Ok(out)
}

// ------------------------------------------------------------------------------------- output

#[derive(Debug, Serialize)]
pub struct Results {
    pub name: String,
    pub ledger: Vec<Entry>,
    pub rules: Vec<Rule>,
    pub retrieval: Vec<Retrieval>,
    pub claims: Vec<Claim>,
    /// every file read, relative to the data root, with its SHA-256
    pub inputs: Vec<(String, String)>,
    pub rejected: Vec<labels::Rejected>,
}

const CHECKS: [Check; 5] = [
    Check::Independent,
    Check::Anchored,
    Check::Proposed,
    Check::Unsettled,
    Check::Unread,
];

fn cell(s: &str) -> String {
    s.replace('|', "\\|").replace('\n', " ")
}

pub fn markdown(r: &Results) -> String {
    let mut s = format!(
        "# Results: {}\n\n\
         Generated by `s106 results`. Do not edit; `claims.csv` is the only hand-kept file here. \
         Rerun to rebuild.\n\n\
         Every cell below carries its check level. **independent**: a person read the source. \
         **anchored**: a person confirmed or corrected an agent's proposal. **proposed**: an agent \
         only, which is not a result. **unsettled**: labellers disagree. **unread**: nobody has \
         read it. A difference between two sources is a cell for a person to check, never a breach; \
         a deed figure is the original obligation, not what is due today.\n\n",
        r.name
    );

    s += "## Check levels\n\n| level | cells | of which found |\n|---|---|---|\n";
    for c in CHECKS {
        let all: Vec<&Entry> = r.ledger.iter().filter(|e| e.check == c).collect();
        let found = all.iter().filter(|e| e.status == "found").count();
        s += &format!("| {} | {} | {found} |\n", c.as_str(), all.len());
    }

    s += "\n## Decision rules\n\nFixed before any value was read (the batch README). A verdict \
          is a result only when its level is anchored or independent.\n";
    for rule in &r.rules {
        s += &format!(
            "\n### Rule {}: {}\n\n**{}** at the **{}** level: {} of {} schemes counted meet it, {} \
             needed ({}); {} left out.\n\n| scheme | outcome | why | level |\n|---|---|---|---|\n",
            rule.id,
            rule.title,
            if rule.met { "Met" } else { "Not met" },
            rule.check.as_str(),
            rule.counts,
            rule.counted,
            rule.needed,
            rule.threshold,
            rule.schemes.len() - rule.counted,
        );
        for o in &rule.schemes {
            s += &format!(
                "| {} | {} | {} | {} |\n",
                o.scheme,
                serde_json::to_value(o.outcome)
                    .unwrap()
                    .as_str()
                    .unwrap_or(""),
                cell(&o.reason),
                o.check.as_str()
            );
        }
    }

    s += "\n## Claims\n\nFrom `claims.csv`. A claim stands at the weakest level of the cells it \
          cites. One about an identifiable scheme, developer or council needs a sign-off before it \
          leaves this repository.\n";
    if r.claims.is_empty() {
        s += "\nNo claims written yet.\n";
    }
    for c in CHECKS {
        let group: Vec<&Claim> = r.claims.iter().filter(|cl| cl.check == c).collect();
        if group.is_empty() {
            continue;
        }
        s += &format!("\n### {}\n\n", c.as_str());
        for cl in group {
            s += &format!("- **{}** {}", cl.id, cl.claim);
            s += &format!(" (cells: {})", cl.cells.join(", "));
            if cl.specific {
                s += &match cl.signed_off.as_str() {
                    "" => " *Specific: not signed off.*".to_string(),
                    who => format!(" *Signed off: {who}.*"),
                };
            }
            if !cl.unknown.is_empty() {
                s += &format!(" **Unknown cells: {}.**", cl.unknown.join(", "));
            }
            s += "\n";
        }
    }

    s += "\n## Scheme lineages\n\nEach scheme's cells in lineage order. `ledger.csv` has the \
          documents and the row each reading came from.\n";
    let schemes: BTreeSet<&String> = r.ledger.iter().map(|e| &e.scheme).collect();
    for scheme in schemes {
        s += &format!(
            "\n### {scheme}\n\n| stage | variable | source | status | value | basis | pages | level |\n\
             |---|---|---|---|---|---|---|---|\n"
        );
        let mut rows: Vec<&Entry> = r.ledger.iter().filter(|e| &e.scheme == scheme).collect();
        rows.sort_by_key(|e| {
            (
                STAGES.iter().position(|st| *st == e.stage),
                e.variable_id.clone(),
                e.source.clone(),
            )
        });
        for e in rows {
            s += &format!(
                "| {} | {} | {} | {} | {} | {} | {} | {} |\n",
                e.stage,
                e.variable_id,
                e.source,
                if e.status.is_empty() { "-" } else { &e.status },
                cell(&e.value),
                e.basis,
                cell(&e.pages),
                e.check.as_str()
            );
        }
    }

    s += "\n## Retrieval\n\nEvery document listed for each scheme and source, and which are in \
          hand. A gap here is recorded, not dropped, so the sample's selection can be checked.\n\n\
          | scheme | source | documents | in hand | not in hand |\n|---|---|---|---|---|\n";
    for x in &r.retrieval {
        s += &format!(
            "| {} | {} | {} | {} | {} |\n",
            x.scheme, x.source, x.documents, x.in_hand, x.not_in_hand
        );
    }

    if !r.rejected.is_empty() {
        s += "\n## Rows not used\n\n";
        for x in &r.rejected {
            s += &format!("- {}: {}\n", x.origin, x.reason);
        }
    }

    s += "\n## Inputs\n\n| file | SHA-256 |\n|---|---|\n";
    for (f, h) in &r.inputs {
        s += &format!("| `{f}` | `{}` |\n", &h[..16]);
    }
    s
}

fn sha256(path: &Path) -> Result<String> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    Ok(Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

fn csv_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            csv_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "csv") {
            out.push(p);
        }
    }
}

/// Builds the results for `sheet` (a batch's blank labelling sheet, relative to `root`) into
/// `out` (relative to `root`), reading `<out>/claims.csv` if there is one.
pub fn run(root: &Path, sheet: &Path, out: &Path) -> Result<Results> {
    let sheet_path = root.join(sheet);
    let out_dir = root.join(out);
    let cells = sheet_cells(&sheet_path)?;
    let (submitted, mut rejected) = labels::load(&root.join("labels"))?;
    let (proposed, bad) = labels::proposals(root)?;
    rejected.extend(bad);

    let ledger = ledger(&cells, &submitted, &proposed);
    let rules = rules(&ledger);
    let sources = sheet_path.with_file_name("sources.csv");
    let retrieval = retrieval(&sources)?;
    let claims_path = out_dir.join("claims.csv");
    let claims = claims(&claims_path, &ledger)?;

    let mut read = vec![sheet_path.clone(), sources, claims_path];
    csv_files(&root.join("labels"), &mut read);
    csv_files(&root.join("proposals"), &mut read);
    read.retain(|p| p.exists());
    read.sort();
    let inputs = read
        .iter()
        .map(|p| {
            let rel = p.strip_prefix(root).unwrap_or(p).display().to_string();
            Ok((rel, sha256(p)?))
        })
        .collect::<Result<_>>()?;

    let r = Results {
        name: out
            .file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into()),
        ledger,
        rules,
        retrieval,
        claims,
        inputs,
        rejected,
    };
    std::fs::create_dir_all(&out_dir)?;
    let mut w = csv::Writer::from_path(out_dir.join("ledger.csv"))?;
    for e in &r.ledger {
        w.serialize(e)?;
    }
    w.flush()?;
    let mut w = csv::Writer::from_path(out_dir.join("rules.csv"))?;
    for o in r.rules.iter().flat_map(|x| &x.schemes) {
        w.serialize(o)?;
    }
    w.flush()?;
    let mut w = csv::Writer::from_path(out_dir.join("retrieval.csv"))?;
    for x in &r.retrieval {
        w.serialize(x)?;
    }
    w.flush()?;
    std::fs::write(out_dir.join("README.md"), markdown(&r))?;
    Ok(r)
}
