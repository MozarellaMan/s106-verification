//! Layer B, first cut (roadmap M5): what one development's deeds oblige, set against what PLD
//! says was permitted and delivered.
//!
//! * Each deed becomes an `InstrumentObligations`: one `Statement` per obligation it states, plus
//!   the deletions it makes. Duplicates within one deed are merged, or flagged when their figures
//!   differ.
//! * `reconcile` replays the instruments in date order as operations on each obligation
//!   (`Op::Introduce`, `Restate`, `Replace`, `Unchanged`, `Delete`). Nothing is overwritten; the
//!   current obligation is the fold of its operations, so a restatement is not a change and a
//!   deletion removes the obligation.
//! * Delivery comes from `Delivery`, read from one saved PLD application snapshot through a typed
//!   view that fails when a field the mapping needs is missing. Every delivered figure cites that
//!   snapshot.
//! * Each row gets a status from `decide`. A shortfall on a completed scheme is
//!   `DiffersReviewRequired`, never "unmet": PLD completions are borough-reported and tenure at
//!   completion isn't tenure in use.
//!
//! Every deed value is grounded the same way the verifier grounds it (value in snippet, snippet
//! on its page); an ungrounded deed value makes its row `Indeterminate`.

use std::collections::BTreeMap;

use anyhow::{Context, Result, bail};
use chrono::NaiveDate;
use rust_decimal::Decimal;
use serde::{Deserialize, Deserializer, Serialize};

use crate::evidence::{Pages, evidence_found};
use crate::pld::{Snapshot, SnapshotRef};
use crate::schema::{
    AffordableBasis, Agreement, ConditionalityKind, Evidence, Tenure, VariationOp,
};
use crate::text::Value;
use crate::verify::{Input, PCT_TOLERANCE, grounding_problem};

pub const RECONCILE_VERSION: &str = "2";
/// Version of `delivery_from_pld`'s reading of PLD application records.
pub const PLD_MAPPING_VERSION: &str = "1";

// --------------------------------------------------------------------------- obligations

/// What is measured. Keys are stable: they identify an obligation across instruments and runs.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "measure", rename_all = "snake_case")]
pub enum Measure {
    TotalUnits,
    MarketUnits,
    AffordableUnits,
    AffordableHabitableRooms,
    TenureUnits {
        tenure: Tenure,
    },
    TenureHabitableRooms {
        tenure: Tenure,
    },
    AffordablePct {
        basis: AffordableBasis,
    },
    /// A named contribution. `key` is the name normalised by `contribution_key`, so "the Carbon
    /// Offset Contribution" and "Carbon Offset Contribution" are one obligation.
    Contribution {
        key: String,
    },
}

impl Measure {
    pub fn contribution(name: &str) -> Measure {
        Measure::Contribution {
            key: contribution_key(name),
        }
    }

    pub fn label(&self) -> String {
        match self {
            Measure::TotalUnits => "Total units".into(),
            Measure::MarketUnits => "Market units".into(),
            Measure::AffordableUnits => "Affordable units".into(),
            Measure::AffordableHabitableRooms => "Affordable habitable rooms".into(),
            Measure::TenureUnits { tenure } => format!("{} units", tenure.label()),
            Measure::TenureHabitableRooms { tenure } => {
                format!("{} habitable rooms", tenure.label())
            }
            Measure::AffordablePct { basis } => format!("Affordable % by {}", basis_label(*basis)),
            Measure::Contribution { key } => format!("{key} (£)"),
        }
    }
}

/// Lower case, punctuation to spaces, whitespace collapsed, a leading "the" dropped.
pub fn contribution_key(name: &str) -> String {
    let words: Vec<String> = name
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(String::from)
        .collect();
    let start = usize::from(words.first().is_some_and(|w| w == "the"));
    words[start..].join(" ")
}

fn basis_label(b: AffordableBasis) -> &'static str {
    match b {
        AffordableBasis::HabitableRooms => "habitable room",
        AffordableBasis::Units => "unit",
        AffordableBasis::Floorspace => "floorspace",
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "grounding", content = "detail", rename_all = "snake_case")]
pub enum Grounding {
    /// the value appears in its snippet and the snippet is on its cited page
    Grounded,
    Ungrounded(String),
    /// no page text was supplied, or there is no figure to ground
    NotChecked,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstrumentRef {
    pub sha256: String,
    pub kind: String,
    pub date: Option<String>,
    pub legal_reference: Option<String>,
}

/// One obligation as one deed states it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Statement {
    #[serde(flatten)]
    pub measure: Measure,
    /// the measure as this deed words it (a contribution's own name)
    pub label: String,
    /// the deed's figure; None for a formula, a rate or no figure
    pub value: Option<Decimal>,
    pub evidence: Option<Evidence>,
    pub grounding: Grounding,
    /// e.g. the condition of a conditional contribution
    pub note: Option<String>,
    /// set when this deed states the obligation more than once with different figures
    pub conflict: Option<String>,
}

/// A deletion a variation makes, as the deed names its target.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Deletion {
    pub target: String,
    pub evidence: Option<Evidence>,
    /// whether the cited snippet is on its page; only a grounded deletion is applied
    pub grounding: Grounding,
}

/// What one deed says, before it is set against the deeds before it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstrumentObligations {
    pub instrument: InstrumentRef,
    /// a deed of variation or supplemental deed
    pub variation: bool,
    pub planning_references: Vec<String>,
    pub statements: Vec<Statement>,
    pub deletions: Vec<Deletion>,
}

/// What an instrument does to an obligation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Op {
    /// first stated; None when it states no figure
    Introduce { value: Option<Decimal> },
    /// stated again with the same figure
    Restate { value: Decimal },
    /// stated with a different figure
    Replace { from: Option<Decimal>, to: Decimal },
    /// cited again without a figure: taken as unchanged
    Unchanged,
    /// removed by a variation
    Delete,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObligationEvent {
    #[serde(flatten)]
    pub measure: Measure,
    pub label: String,
    #[serde(flatten)]
    pub op: Op,
    pub instrument: InstrumentRef,
    pub evidence: Option<Evidence>,
    pub grounding: Grounding,
    pub note: Option<String>,
    pub conflict: Option<String>,
    /// false when the instrument's link to the development couldn't be checked (it states no
    /// planning reference); what it changes goes to review
    #[serde(default = "yes")]
    pub verified: bool,
}

fn yes() -> bool {
    true
}

/// One statement per extracted measure of one deed. `pages` grounds each value.
pub fn instrument_obligations(a: &Agreement, pages: Option<&Pages>) -> InstrumentObligations {
    let ground = |label: &str, value: Option<Decimal>, evidence: &Option<Evidence>| match pages {
        None => Grounding::NotChecked,
        Some(_) if value.is_none() => Grounding::NotChecked,
        Some(p) => {
            let input = Input {
                label: label.to_string(),
                value: value.map(Value::Num),
                evidence: evidence.clone(),
            };
            match grounding_problem(&input, p) {
                None => Grounding::Grounded,
                Some(why) => Grounding::Ungrounded(why),
            }
        }
    };
    let mut raw: Vec<Statement> = vec![];
    let mut push = |measure: Measure,
                    label: String,
                    value: Option<Decimal>,
                    evidence: Option<Evidence>,
                    note: Option<String>| {
        if value.is_none() && evidence.is_none() {
            return;
        }
        raw.push(Statement {
            grounding: ground(&label, value, &evidence),
            measure,
            label,
            value,
            evidence,
            note,
            conflict: None,
        });
    };
    let mut housing = |m: Measure, value: Option<u32>, evidence: Option<Evidence>| {
        let label = m.label();
        push(m, label, value.map(Decimal::from), evidence, None)
    };

    let h = &a.housing;
    housing(
        Measure::TotalUnits,
        h.total_units,
        h.total_units_evidence.clone(),
    );
    housing(
        Measure::MarketUnits,
        h.market_units,
        h.market_units_evidence.clone(),
    );
    housing(
        Measure::AffordableUnits,
        h.affordable_units,
        h.affordable_units_evidence.clone(),
    );
    housing(
        Measure::AffordableHabitableRooms,
        h.affordable_habitable_rooms,
        h.affordable_units_evidence.clone(),
    );
    for t in &h.tenures {
        housing(
            Measure::TenureUnits { tenure: t.tenure },
            t.units,
            t.evidence.clone(),
        );
        if t.habitable_rooms.is_some() {
            housing(
                Measure::TenureHabitableRooms { tenure: t.tenure },
                t.habitable_rooms,
                t.evidence.clone(),
            );
        }
    }
    if let Some(basis) = h.claimed_affordable_basis {
        let m = Measure::AffordablePct { basis };
        let label = m.label();
        push(
            m,
            label,
            h.claimed_affordable_pct,
            h.claimed_affordable_evidence.clone(),
            None,
        );
    }
    for c in &a.contributions {
        push(
            Measure::contribution(&c.name),
            format!("{} (£)", c.name),
            c.amount.stated_sum_gbp(),
            c.amount.evidence.clone(),
            match c.conditionality.kind {
                ConditionalityKind::Conditional => Some(format!(
                    "conditional: {}",
                    c.conditionality
                        .condition
                        .as_deref()
                        .unwrap_or("condition not quoted")
                )),
                ConditionalityKind::Unknown => Some("conditionality unknown".into()),
                ConditionalityKind::Unconditional => None,
            },
        );
    }

    InstrumentObligations {
        instrument: InstrumentRef {
            sha256: a.provenance.sha256.clone(),
            kind: format!("{:?}", a.instrument.kind),
            date: a.metadata.agreement_date.clone(),
            legal_reference: a.metadata.legal_reference.clone(),
        },
        variation: a.instrument.kind.is_variation(),
        planning_references: a.metadata.planning_references.clone(),
        statements: merge_duplicates(raw),
        deletions: a
            .variations
            .iter()
            .filter(|v| v.op == VariationOp::Delete)
            .map(|v| Deletion {
                target: v.target.clone(),
                evidence: v.evidence.clone(),
                grounding: match (pages, &v.evidence) {
                    (None, _) => Grounding::NotChecked,
                    (Some(_), None) => Grounding::Ungrounded("no evidence cited".into()),
                    (Some(_), Some(ev)) if !says_deleted(&ev.snippet) => Grounding::Ungrounded(
                        "cited snippet doesn't say anything is deleted".into(),
                    ),
                    (Some(p), Some(ev)) if evidence_found(Some(ev), p) => Grounding::Grounded,
                    (Some(_), Some(ev)) => Grounding::Ungrounded(format!(
                        "cited snippet not re-locatable on p{}",
                        ev.page
                    )),
                },
            })
            .collect(),
    }
}

/// Does a snippet say something is deleted (or omitted, revoked, removed)?
fn says_deleted(snippet: &str) -> bool {
    let s = snippet.to_lowercase();
    ["delet", "omit", "revoke", "remove", "struck out"]
        .iter()
        .any(|w| s.contains(w))
}

/// Does a deletion's cited snippet itself name the obligation it is applied to? The target
/// matching an obligation isn't enough: the snippet is the evidence.
fn snippet_names(snippet: &str, m: &Measure) -> bool {
    match m {
        Measure::Contribution { key } => {
            !key.is_empty()
                && format!(" {} ", contribution_key(snippet)).contains(&format!(" {key} "))
        }
        _ => false,
    }
}

/// One statement per measure. Repeats with the same figure (a sum defined and then restated in a
/// schedule) merge into the first; repeats with different figures (phased sums, or a misread)
/// keep no figure and carry a conflict, so their row goes to a person.
fn merge_duplicates(raw: Vec<Statement>) -> Vec<Statement> {
    let mut out: Vec<Statement> = vec![];
    let mut seen: BTreeMap<Measure, Vec<Option<Decimal>>> = BTreeMap::new();
    for s in raw {
        seen.entry(s.measure.clone()).or_default().push(s.value);
        if !out.iter().any(|o| o.measure == s.measure) {
            out.push(s);
        }
    }
    for s in &mut out {
        let values = &seen[&s.measure];
        if values.len() < 2 {
            continue;
        }
        if values.iter().all(|v| *v == values[0]) {
            let extra = format!("stated {} times in this deed", values.len());
            s.note = Some(match s.note.take() {
                Some(n) => format!("{n}; {extra}"),
                None => extra,
            });
        } else {
            let figures: Vec<String> = values.iter().map(|v| num(*v)).collect();
            s.conflict = Some(format!(
                "stated {} times in one deed with different figures: {}",
                values.len(),
                figures.join(", ")
            ));
            s.value = None;
            s.grounding = Grounding::NotChecked;
        }
    }
    out
}

// --------------------------------------------------------------------------- delivery

/// A unit record's tenure, as PLD reports it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "tenure", rename_all = "snake_case")]
pub enum UnitTenure {
    Market,
    Affordable(Tenure),
    /// a tenure string the PLD mapping does not recognise; kept verbatim
    Unrecognised(String),
    /// the unit record gives no tenure
    Missing,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeliveredUnit {
    /// PLD's unit identifier, or the record's position when it has none
    pub id: String,
    pub tenure: UnitTenure,
    pub habitable_rooms: Option<u32>,
    pub completed_on: Option<NaiveDate>,
}

/// Figures PLD records for the permission (proposed), not for delivery.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Permitted {
    pub total_units: Option<u32>,
    pub market_units: Option<u32>,
    pub affordable_units: Option<u32>,
    #[serde(default)]
    pub tenure_units: Vec<(Tenure, u32)>,
}

/// PLD's view of one development, read from one saved application snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Delivery {
    /// PLD record id, e.g. `Southwark-19_AP_1773`
    pub record_id: String,
    /// the borough's planning reference, e.g. `19/AP/1773`
    pub lpa_app_no: Option<String>,
    /// the snapshot every figure here was read from
    pub snapshot: SnapshotRef,
    /// what PLD said on this date (the snapshot's retrieval date); a later snapshot is a new
    /// data point, not a correction
    pub as_of: String,
    pub mapping_version: String,
    pub status: Option<String>,
    pub started_on: Option<NaiveDate>,
    pub completed_on: Option<NaiveDate>,
    /// Some(true) only when PLD reports the whole scheme complete.
    pub scheme_complete: Option<bool>,
    pub permitted: Permitted,
    #[serde(default)]
    pub units: Vec<DeliveredUnit>,
}

/// Does a deed tenure cover a delivered tenure? Exact, or the deed uses an umbrella term:
/// "Intermediate" covers the London Plan intermediate products, "Shared Ownership" covers the
/// London brand. Social Rent never covers London Affordable Rent or Affordable Rent: their rents
/// differ, so a swap goes to review. Discount Market Sale and First Homes are not treated as
/// intermediate without a deed saying so.
pub fn covers(deed: Tenure, delivered: Tenure) -> bool {
    use Tenure::*;
    deed == delivered
        || match deed {
            Intermediate => matches!(
                delivered,
                SharedOwnership | LondonSharedOwnership | LondonLivingRent | DiscountMarketRent
            ),
            SharedOwnership => delivered == LondonSharedOwnership,
            _ => false,
        }
}

/// A figure counted from PLD, with what it was counted from.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Counted {
    pub value: Option<Decimal>,
    /// how it was counted, or why it couldn't be
    pub basis: String,
}

impl Counted {
    pub fn none(basis: impl Into<String>) -> Counted {
        Counted {
            value: None,
            basis: basis.into(),
        }
    }
}

impl Delivery {
    fn completed(&self) -> impl Iterator<Item = &DeliveredUnit> {
        self.units.iter().filter(|u| u.completed_on.is_some())
    }

    fn count(&self, what: &str, f: impl Fn(&UnitTenure) -> bool, rooms: bool) -> Counted {
        let n_done = self.completed().count();
        if self.units.is_empty() {
            return Counted::none("no PLD unit records");
        }
        if what != "all" {
            let missing = self
                .completed()
                .filter(|u| u.tenure == UnitTenure::Missing)
                .count();
            if missing > 0 {
                return Counted::none(format!("{missing} completed unit record(s) give no tenure"));
            }
            let unknown = self
                .completed()
                .filter(|u| matches!(u.tenure, UnitTenure::Unrecognised(_)))
                .count();
            if unknown > 0 {
                return Counted::none(format!(
                    "{unknown} completed unit record(s) have an unrecognised tenure"
                ));
            }
        }
        let matching: Vec<&DeliveredUnit> = self.completed().filter(|u| f(&u.tenure)).collect();
        if rooms {
            let missing = matching
                .iter()
                .filter(|u| u.habitable_rooms.is_none())
                .count();
            if missing > 0 {
                return Counted::none(format!(
                    "{missing} matching unit record(s) lack habitable rooms"
                ));
            }
            let sum: u32 = matching.iter().filter_map(|u| u.habitable_rooms).sum();
            return Counted {
                value: Some(Decimal::from(sum)),
                basis: format!(
                    "habitable rooms of {} {what} unit record(s) with a completion date",
                    matching.len()
                ),
            };
        }
        Counted {
            value: Some(Decimal::from(matching.len() as u64)),
            basis: format!(
                "{} {what} of {n_done} unit record(s) with a completion date ({} records in all)",
                matching.len(),
                self.units.len()
            ),
        }
    }

    pub fn delivered(&self, m: &Measure) -> Counted {
        let affordable = |t: &UnitTenure| matches!(t, UnitTenure::Affordable(_));
        match m {
            Measure::TotalUnits => self.count("all", |_| true, false),
            Measure::MarketUnits => {
                self.count("market", |t| matches!(t, UnitTenure::Market), false)
            }
            Measure::AffordableUnits => self.count("affordable", affordable, false),
            Measure::AffordableHabitableRooms => self.count("affordable", affordable, true),
            Measure::TenureUnits { tenure } => self.count(
                tenure.label(),
                |t| matches!(t, UnitTenure::Affordable(d) if covers(*tenure, *d)),
                false,
            ),
            Measure::TenureHabitableRooms { tenure } => self.count(
                tenure.label(),
                |t| matches!(t, UnitTenure::Affordable(d) if covers(*tenure, *d)),
                true,
            ),
            Measure::AffordablePct { basis } => {
                let rooms = match basis {
                    AffordableBasis::Units => false,
                    AffordableBasis::HabitableRooms => true,
                    AffordableBasis::Floorspace => {
                        return Counted::none("PLD unit records carry no floorspace basis");
                    }
                };
                let num = self.count("affordable", affordable, rooms);
                let den = self.count("all", |_| true, rooms);
                match (num.value, den.value) {
                    (Some(n), Some(d)) if !d.is_zero() => Counted {
                        value: Some((n * Decimal::ONE_HUNDRED / d).round_dp(2)),
                        basis: format!("{} / {}", num.basis, den.basis),
                    },
                    _ => Counted::none(if num.value.is_none() {
                        num.basis
                    } else {
                        den.basis
                    }),
                }
            }
            Measure::Contribution { .. } => Counted::none("PLD records no S106 payments"),
        }
    }

    pub fn permitted(&self, m: &Measure) -> Option<Decimal> {
        let p = &self.permitted;
        let v = match m {
            Measure::TotalUnits => p.total_units,
            Measure::MarketUnits => p.market_units,
            Measure::AffordableUnits => p.affordable_units,
            Measure::TenureUnits { tenure } => {
                let matching: Vec<u32> = p
                    .tenure_units
                    .iter()
                    .filter(|(t, _)| covers(*tenure, *t))
                    .map(|(_, n)| *n)
                    .collect();
                (!matching.is_empty()).then(|| matching.iter().sum())
            }
            _ => None,
        };
        v.map(Decimal::from)
    }

    /// Completed affordable units whose tenure no deed tenure covers.
    fn uncovered(&self, deed_tenures: &[Tenure]) -> BTreeMap<Tenure, usize> {
        let mut out = BTreeMap::new();
        for u in self.completed() {
            if let UnitTenure::Affordable(d) = u.tenure
                && !deed_tenures.iter().any(|t| covers(*t, d))
            {
                *out.entry(d).or_default() += 1;
            }
        }
        out
    }
}

// --------------------------------------------------------------------------- PLD mapping

/// The part of a PLD application record the mapping reads. Every field here is required unless
/// it is an `Option`, and those are the ones PLD leaves null (dates and status before a scheme
/// starts, a unit's number or tenure), so a renamed or retyped field fails the read instead of
/// turning every row indeterminate.
#[derive(Debug, Deserialize)]
struct PldApplication {
    id: String,
    lpa_app_no: Option<String>,
    status: Option<String>,
    actual_commencement_date: Option<String>,
    actual_completion_date: Option<String>,
    application_details: PldDetails,
}

#[derive(Debug, Deserialize)]
struct PldDetails {
    residential_details: PldResidential,
}

#[derive(Debug, Deserialize)]
struct PldResidential {
    #[serde(deserialize_with = "pld_count")]
    total_no_proposed_residential_units: u32,
    #[serde(deserialize_with = "pld_count")]
    total_no_proposed_residential_affordable_units: u32,
    #[serde(deserialize_with = "pld_count")]
    total_no_proposed_residential_units_market_for_sale: u32,
    #[serde(deserialize_with = "pld_count")]
    total_no_proposed_residential_units_market_for_rent: u32,
    #[serde(deserialize_with = "pld_count")]
    total_no_proposed_residential_units_social_rent: u32,
    #[serde(deserialize_with = "pld_count")]
    total_no_proposed_residential_units_london_affordable_rent: u32,
    #[serde(deserialize_with = "pld_count")]
    total_no_proposed_residential_units_affordable_rent: u32,
    #[serde(deserialize_with = "pld_count")]
    total_no_proposed_residential_units_london_living_rent: u32,
    #[serde(deserialize_with = "pld_count")]
    total_no_proposed_residential_units_london_shared_ownership: u32,
    #[serde(deserialize_with = "pld_count")]
    total_no_proposed_residential_units_intermediate: u32,
    #[serde(deserialize_with = "pld_count")]
    total_no_proposed_residential_units_discount_market_rent: u32,
    #[serde(deserialize_with = "pld_count")]
    total_no_proposed_residential_units_discount_market_sale: u32,
    residential_units: Vec<PldUnit>,
}

#[derive(Debug, Deserialize)]
struct PldUnit {
    change_type: String,
    tenure: Option<String>,
    actual_completion_date: Option<String>,
    unit_no: Option<serde_json::Value>,
}

/// A whole, non-negative count, given as a number or a numeric string.
fn pld_count<'de, D: Deserializer<'de>>(d: D) -> std::result::Result<u32, D::Error> {
    use serde::de::Error;
    let v = serde_json::Value::deserialize(d)?;
    let n = match &v {
        serde_json::Value::Number(n) => n.as_f64(),
        serde_json::Value::String(s) => s.trim().parse::<f64>().ok(),
        _ => None,
    };
    match n {
        Some(n) if n >= 0.0 && n.fract() == 0.0 && n <= f64::from(u32::MAX) => Ok(n as u32),
        _ => Err(D::Error::custom(format!("expected a whole count, got {v}"))),
    }
}

/// A PLD unit record's `tenure` string. Market tenures are market; an affordable tenure is
/// recognised only when it is spelled exactly as a deed tenure; anything else is kept verbatim.
pub fn pld_unit_tenure(raw: Option<&str>) -> UnitTenure {
    use Tenure::*;
    let Some(raw) = raw.map(str::trim).filter(|s| !s.is_empty()) else {
        return UnitTenure::Missing;
    };
    if raw.eq_ignore_ascii_case("Market for sale") || raw.eq_ignore_ascii_case("Market for rent") {
        return UnitTenure::Market;
    }
    [
        SocialRent,
        LondonAffordableRent,
        AffordableRent,
        LondonLivingRent,
        SharedOwnership,
        LondonSharedOwnership,
        Intermediate,
        DiscountMarketRent,
        DiscountMarketSale,
        FirstHomes,
    ]
    .into_iter()
    .find(|t| t.label().eq_ignore_ascii_case(raw))
    .map_or_else(
        || UnitTenure::Unrecognised(raw.into()),
        UnitTenure::Affordable,
    )
}

/// PLD dates are dd/mm/yyyy. Null is no date; anything else that doesn't parse is an error.
fn pld_date(field: &str, v: &Option<String>) -> Result<Option<NaiveDate>> {
    match v.as_deref().map(str::trim) {
        None | Some("") => Ok(None),
        Some(s) => NaiveDate::parse_from_str(s, "%d/%m/%Y")
            .map(Some)
            .with_context(|| format!("PLD {field} {s:?} is not dd/mm/yyyy")),
    }
}

/// Build `Delivery` from one saved `applications/_source/<id>` snapshot.
///
/// * Scheme complete only when `status` is "Completed"; "Started" / "Not started" mean not yet;
///   any other status leaves it unknown. Case is ignored.
/// * Permission figures are the `total_no_proposed_*` fields. Market = for sale + for rent.
/// * Units are `residential_units` with `change_type` "Gain" (losses of existing homes are not
///   delivery). PLD unit records carry bedrooms, not habitable rooms, so `habitable_rooms` is
///   left empty and room-based rows stay indeterminate.
pub fn delivery_from_pld(snap: &Snapshot, at: SnapshotRef) -> Result<Delivery> {
    let doc = snap.response.get("_source").unwrap_or(&snap.response);
    let app: PldApplication = serde_json::from_value(doc.clone()).with_context(|| {
        format!(
            "{} does not have the fields the PLD mapping (v{PLD_MAPPING_VERSION}) reads",
            at.path
        )
    })?;
    let status = app.status.clone();
    let scheme_complete = match status.as_deref().map(str::trim) {
        Some(s) if s.eq_ignore_ascii_case("completed") => Some(true),
        Some(s) if s.eq_ignore_ascii_case("started") || s.eq_ignore_ascii_case("not started") => {
            Some(false)
        }
        _ => None,
    };
    let rd = &app.application_details.residential_details;
    let permitted = Permitted {
        total_units: Some(rd.total_no_proposed_residential_units),
        market_units: Some(
            rd.total_no_proposed_residential_units_market_for_sale
                + rd.total_no_proposed_residential_units_market_for_rent,
        ),
        affordable_units: Some(rd.total_no_proposed_residential_affordable_units),
        // Shared equity, starter homes and self-build have no deed tenure here and are left out.
        tenure_units: vec![
            (
                Tenure::SocialRent,
                rd.total_no_proposed_residential_units_social_rent,
            ),
            (
                Tenure::LondonAffordableRent,
                rd.total_no_proposed_residential_units_london_affordable_rent,
            ),
            (
                Tenure::AffordableRent,
                rd.total_no_proposed_residential_units_affordable_rent,
            ),
            (
                Tenure::LondonLivingRent,
                rd.total_no_proposed_residential_units_london_living_rent,
            ),
            (
                Tenure::LondonSharedOwnership,
                rd.total_no_proposed_residential_units_london_shared_ownership,
            ),
            (
                Tenure::Intermediate,
                rd.total_no_proposed_residential_units_intermediate,
            ),
            (
                Tenure::DiscountMarketRent,
                rd.total_no_proposed_residential_units_discount_market_rent,
            ),
            (
                Tenure::DiscountMarketSale,
                rd.total_no_proposed_residential_units_discount_market_sale,
            ),
        ],
    };
    let mut units = vec![];
    for (i, u) in rd.residential_units.iter().enumerate() {
        if !u.change_type.trim().eq_ignore_ascii_case("gain") {
            continue;
        }
        units.push(DeliveredUnit {
            id: match &u.unit_no {
                None | Some(serde_json::Value::Null) => format!("residential_units[{i}]"),
                Some(serde_json::Value::String(s)) => s.clone(),
                Some(v) => v.to_string(),
            },
            tenure: pld_unit_tenure(u.tenure.as_deref()),
            habitable_rooms: None,
            completed_on: pld_date(
                &format!("residential_units[{i}].actual_completion_date"),
                &u.actual_completion_date,
            )?,
        });
    }
    Ok(Delivery {
        record_id: app.id,
        lpa_app_no: app.lpa_app_no,
        as_of: at.retrieved_at.clone(),
        snapshot: at,
        mapping_version: PLD_MAPPING_VERSION.into(),
        status,
        started_on: pld_date("actual_commencement_date", &app.actual_commencement_date)?,
        completed_on: pld_date("actual_completion_date", &app.actual_completion_date)?,
        scheme_complete,
        permitted,
        units,
    })
}

// --------------------------------------------------------------------------- reconciliation

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RowStatus {
    /// PLD's completed-unit count equals the current obligation. Counts only: says nothing about
    /// tenure in use, rents or quality.
    CountsMatch,
    /// the scheme isn't complete and delivery is still short
    NotYetDue,
    /// the obligation was replaced by a later deed, or a later deed deletes something that could
    /// not be matched; compare by hand
    Varied,
    /// a variation removed the obligation
    Removed,
    /// delivery differs from the obligation: a human must check the borough's records.
    /// Never a finding that the obligation was unmet.
    DiffersReviewRequired,
    /// something needed for a comparison is missing or unproven
    Indeterminate,
}

impl RowStatus {
    pub fn label(self) -> &'static str {
        match self {
            RowStatus::CountsMatch => "counts match",
            RowStatus::NotYetDue => "not yet due",
            RowStatus::Varied => "varied",
            RowStatus::Removed => "removed by variation",
            RowStatus::DiffersReviewRequired => "differs: review",
            RowStatus::Indeterminate => "indeterminate",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Row {
    #[serde(flatten)]
    pub measure: Measure,
    pub label: String,
    /// as first stated
    pub agreed: Option<Decimal>,
    /// after every later instrument (the fold of the events); None when removed or unstated
    pub current: Option<Decimal>,
    /// all events for this obligation, in instrument order
    pub events: Vec<ObligationEvent>,
    /// PLD's permission figure
    pub permitted: Option<Decimal>,
    pub delivered: Counted,
    pub status: RowStatus,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Reconciliation {
    pub reconcile_version: String,
    pub development: String,
    /// in the order they were applied
    pub instruments: Vec<InstrumentRef>,
    pub delivery: Option<Delivery>,
    pub rows: Vec<Row>,
    /// things a reader should know that don't belong to one row
    #[serde(default)]
    pub warnings: Vec<String>,
}

/// The current state of an obligation after folding its events.
#[derive(Debug, Clone, PartialEq)]
pub enum Current {
    Removed,
    /// the deed states it more than once with different figures
    Conflict(String),
    /// no figure (formula, rate, or none stated); the note says why when known
    NoFigure(Option<String>),
    Value(Decimal),
}

/// Everything `decide` looks at. It is a pure function of these.
#[derive(Debug, Clone)]
pub struct Facts {
    pub current: Current,
    /// the grounding of the event that set the current figure
    pub grounding: Grounding,
    /// some later deed replaced the figure
    pub replaced: bool,
    /// a later deed deletes something not matched to any obligation
    pub unmatched_deletion: Option<String>,
    /// a later deed deletes this obligation, but the deletion couldn't be applied (why)
    pub unproven_deletion: Option<String>,
    /// a deed whose link to the development is unchecked changed this obligation (which)
    pub unverified_change: Option<String>,
    pub delivered: Counted,
    /// compare within the verifier's percentage tolerance rather than exactly
    pub percentage: bool,
    pub scheme_complete: Option<bool>,
}

/// The status rules, in order (see docs/reconciliation.md).
pub fn decide(f: &Facts) -> (RowStatus, String) {
    let current = match &f.current {
        Current::Removed => {
            return (
                RowStatus::Removed,
                "a variation removes this obligation".into(),
            );
        }
        Current::Conflict(why) => return (RowStatus::Indeterminate, why.clone()),
        Current::NoFigure(note) => {
            return (
                RowStatus::Indeterminate,
                match note {
                    Some(n) => format!("no stated figure ({n})"),
                    None => "no stated figure in the deed".into(),
                },
            );
        }
        Current::Value(v) => *v,
    };
    if let Grounding::Ungrounded(why) = &f.grounding {
        return (
            RowStatus::Indeterminate,
            format!("deed value unproven: {why}"),
        );
    }
    if let Some(why) = &f.unproven_deletion {
        return (
            RowStatus::Varied,
            format!(
                "a later deed deletes this, but the deletion isn't proven ({why}); compare by hand"
            ),
        );
    }
    if let Some(which) = &f.unverified_change {
        return (
            RowStatus::Varied,
            format!("changed by {which}, which states no planning reference; compare by hand"),
        );
    }
    if let Some(target) = &f.unmatched_deletion {
        return (
            RowStatus::Varied,
            format!(
                "a later deed deletes \"{target}\", which isn't matched to any obligation; \
                 compare by hand"
            ),
        );
    }
    let Some(delivered) = f.delivered.value else {
        return (RowStatus::Indeterminate, f.delivered.basis.clone());
    };
    let same = if f.percentage {
        (delivered - current).abs() <= PCT_TOLERANCE
    } else {
        delivered == current
    };
    if same {
        return (RowStatus::CountsMatch, f.delivered.basis.clone());
    }
    if f.replaced {
        return (
            RowStatus::Varied,
            format!("delivered {delivered} vs varied obligation {current}; compare by hand"),
        );
    }
    match f.scheme_complete {
        Some(true) => (
            RowStatus::DiffersReviewRequired,
            format!(
                "PLD reports the scheme complete with {delivered} against {current}; \
                 check the borough's records before drawing any conclusion"
            ),
        ),
        Some(false) if delivered < current => (
            RowStatus::NotYetDue,
            format!("{delivered} of {current} so far; PLD does not report the scheme complete"),
        ),
        Some(false) => (
            RowStatus::DiffersReviewRequired,
            format!("{delivered} delivered so far against {current}, scheme not complete"),
        ),
        None => (
            RowStatus::Indeterminate,
            format!("{delivered} against {current}; PLD scheme status not established"),
        ),
    }
}

/// Planning references compared without case, spaces or separators: "19/AP/1773" = "19-ap-1773".
fn same_reference(a: &str, b: &str) -> bool {
    let norm = |s: &str| {
        s.chars()
            .filter(char::is_ascii_alphanumeric)
            .map(|c| c.to_ascii_uppercase())
            .collect::<String>()
    };
    !norm(a).is_empty() && norm(a) == norm(b)
}

fn parse_deed_date(s: &str) -> Option<NaiveDate> {
    let s = s.trim();
    ["%d %B %Y", "%e %B %Y", "%Y-%m-%d", "%d/%m/%Y", "%d %b %Y"]
        .iter()
        .find_map(|f| NaiveDate::parse_from_str(s, f).ok())
        .or_else(|| {
            // "28th January 2021"
            let cleaned: String = s
                .split_whitespace()
                .map(|w| {
                    w.trim_end_matches(|c: char| c.is_ascii_alphabetic())
                        .parse::<u32>()
                        .map_or(w.to_string(), |n| n.to_string())
                })
                .collect::<Vec<_>>()
                .join(" ");
            NaiveDate::parse_from_str(&cleaned, "%d %B %Y").ok()
        })
}

/// A key for matching a deletion target against an obligation: the contribution key, or the
/// measure's label normalised the same way.
fn deletion_matches(target: &str, m: &Measure) -> bool {
    let t = contribution_key(target);
    let k = match m {
        Measure::Contribution { key } => key.clone(),
        _ => return false,
    };
    !k.is_empty() && format!(" {t} ").contains(&format!(" {k} "))
}

/// Set a development's deeds against PLD.
///
/// `instruments` are applied in the order given, original first; a warning is raised when that
/// order disagrees with the deeds' dates. Fails when the PLD record is not the development named,
/// or when the deeds' planning references and PLD's disagree.
pub fn reconcile(
    development: &str,
    instruments: Vec<InstrumentObligations>,
    delivery: Option<Delivery>,
) -> Result<Reconciliation> {
    let mut warnings = vec![];
    if let Some(d) = &delivery {
        if d.record_id != development {
            bail!(
                "the PLD snapshot is record {}, not {development}",
                d.record_id
            );
        }
        if d.lpa_app_no.is_none() {
            warnings
                .push("the PLD record has no planning reference to check the deeds against".into());
        }
    }
    // Every instrument must be a deed for this development. The reference is PLD's planning
    // reference, or failing that the first deed's. A deed citing another reference is refused; a
    // later deed citing none is applied, but what it changes goes to review.
    let reference: Vec<String> = match delivery.as_ref().and_then(|d| d.lpa_app_no.clone()) {
        Some(r) => vec![r],
        None => instruments
            .first()
            .map(|i| i.planning_references.clone())
            .unwrap_or_default(),
    };
    let mut verified = vec![true; instruments.len()];
    for (k, inst) in instruments.iter().enumerate() {
        let refs = &inst.planning_references;
        if refs.is_empty() {
            if k == 0 {
                if !reference.is_empty() {
                    warnings.push(format!(
                        "instrument 1 states no planning reference, so it could not be checked \
                         against {}",
                        reference.join(", ")
                    ));
                }
            } else {
                verified[k] = false;
                warnings.push(format!(
                    "instrument {} states no planning reference; what it changes goes to review",
                    k + 1
                ));
            }
        } else if !reference.is_empty()
            && !refs
                .iter()
                .any(|r| reference.iter().any(|x| same_reference(r, x)))
        {
            bail!(
                "instrument {} (sha256 {}) cites planning reference(s) {}, not {}: it is not a deed \
                 for this development",
                k + 1,
                inst.instrument.sha256,
                refs.join(", "),
                reference.join(", ")
            );
        }
    }
    let dates: Vec<Option<NaiveDate>> = instruments
        .iter()
        .map(|i| i.instrument.date.as_deref().and_then(parse_deed_date))
        .collect();
    for (k, pair) in dates.windows(2).enumerate() {
        if let [Some(a), Some(b)] = pair
            && b < a
        {
            warnings.push(format!(
                "instrument {} is dated {b}, before instrument {} ({a}); instruments are applied \
                 in the order given",
                k + 2,
                k + 1
            ));
        }
    }

    // Replay the instruments as operations on each obligation.
    let mut order: Vec<Measure> = vec![];
    let mut events: BTreeMap<Measure, Vec<ObligationEvent>> = BTreeMap::new();
    let mut current: BTreeMap<Measure, Option<Decimal>> = BTreeMap::new();
    let mut unmatched: Vec<(usize, String)> = vec![];
    for (k, inst) in instruments.iter().enumerate() {
        for s in &inst.statements {
            let op = match current.get(&s.measure) {
                None => Op::Introduce { value: s.value },
                Some(prev) => match s.value {
                    None => Op::Unchanged,
                    Some(v) if *prev == Some(v) => Op::Restate { value: v },
                    Some(v) => Op::Replace { from: *prev, to: v },
                },
            };
            match &op {
                Op::Introduce { value } => {
                    current.insert(s.measure.clone(), *value);
                }
                Op::Restate { value } | Op::Replace { to: value, .. } => {
                    current.insert(s.measure.clone(), Some(*value));
                }
                Op::Unchanged | Op::Delete => {}
            }
            if s.conflict.is_some() && op == Op::Unchanged {
                // a conflicting restatement: the figure is no longer known
                current.insert(s.measure.clone(), None);
            }
            if !events.contains_key(&s.measure) {
                order.push(s.measure.clone());
            }
            events
                .entry(s.measure.clone())
                .or_default()
                .push(ObligationEvent {
                    measure: s.measure.clone(),
                    label: s.label.clone(),
                    op,
                    instrument: inst.instrument.clone(),
                    evidence: s.evidence.clone(),
                    grounding: s.grounding.clone(),
                    note: s.note.clone(),
                    conflict: s.conflict.clone(),
                    verified: verified[k],
                });
        }
        for del in &inst.deletions {
            let hits: Vec<Measure> = order
                .iter()
                .filter(|m| deletion_matches(&del.target, m))
                .cloned()
                .collect();
            if hits.is_empty() {
                unmatched.push((k, del.target.clone()));
            }
            for m in hits {
                // Grounded on its page, and the snippet names this obligation.
                let grounding = match (&del.grounding, &del.evidence) {
                    (Grounding::Grounded, Some(ev)) if !snippet_names(&ev.snippet, &m) => {
                        Grounding::Ungrounded(format!(
                            "its cited snippet on p{} doesn't name the {}",
                            ev.page,
                            events[&m][0].label.trim_end_matches(" (£)")
                        ))
                    }
                    (g, _) => g.clone(),
                };
                if grounding == Grounding::Grounded && verified[k] {
                    current.insert(m.clone(), None);
                }
                let label = events[&m][0].label.clone();
                events.entry(m.clone()).or_default().push(ObligationEvent {
                    measure: m,
                    label,
                    op: Op::Delete,
                    instrument: inst.instrument.clone(),
                    evidence: del.evidence.clone(),
                    grounding,
                    note: Some(format!("deletes \"{}\"", del.target)),
                    conflict: None,
                    verified: verified[k],
                });
            }
        }
    }
    for (k, target) in &unmatched {
        warnings.push(format!(
            "instrument {} deletes \"{target}\", which isn't matched to any obligation",
            k + 1
        ));
    }

    let mut rows = vec![];
    for m in order {
        let evs = events.remove(&m).unwrap_or_default();
        // Fold the events. A delete applies only when its citation is grounded and its deed is
        // verified; a later figure reinstates; an unchanged citation keeps the state as it was.
        // A conflict lasts until a later deed states a figure without one.
        let mut removed = false;
        let mut value: Option<Decimal> = None;
        let mut conflict: Option<String> = None;
        let mut setter: Option<&ObligationEvent> = None;
        let mut unproven_deletion: Option<String> = None;
        let mut unverified_change: Option<String> = None;
        for e in &evs {
            let sets = match e.op {
                Op::Introduce { value: v } => Some(v),
                Op::Restate { value: v } | Op::Replace { to: v, .. } => Some(Some(v)),
                Op::Unchanged | Op::Delete => None,
            };
            if let Some(v) = sets {
                (removed, value, setter, unproven_deletion) = (false, v, Some(e), None);
            }
            if e.op == Op::Delete {
                if e.grounding == Grounding::Grounded && e.verified {
                    (removed, value, conflict, unproven_deletion) = (true, None, None, None);
                } else {
                    unproven_deletion = Some(match &e.grounding {
                        _ if !e.verified => "its deed states no planning reference".into(),
                        Grounding::Ungrounded(why) => why.clone(),
                        _ => "its citation was not checked against the deed's pages".into(),
                    });
                }
            }
            if e.conflict.is_some() {
                conflict = e.conflict.clone();
                value = None;
            } else if sets.is_some() {
                conflict = None;
            }
            // Whatever an unverified deed introduces, replaces or deletes needs review, until a
            // verified deed states the figure again (the events keep the history).
            let changes = matches!(e.op, Op::Introduce { .. } | Op::Replace { .. } | Op::Delete);
            if e.verified && (sets.is_some() || (e.op == Op::Delete && removed)) {
                unverified_change = None;
            }
            if !e.verified && changes {
                unverified_change = Some(format!(
                    "the {} dated {}",
                    e.instrument.kind,
                    e.instrument.date.as_deref().unwrap_or("?")
                ));
            }
        }
        let first_instrument = instruments
            .iter()
            .position(|i| {
                evs.first()
                    .is_some_and(|e| e.instrument.sha256 == i.instrument.sha256)
            })
            .unwrap_or(0);
        let unmatched_deletion = unmatched
            .iter()
            .find(|(k, _)| *k > first_instrument)
            .map(|(_, t)| t.clone());

        let facts = Facts {
            current: if removed {
                Current::Removed
            } else if let Some(c) = conflict {
                Current::Conflict(c)
            } else {
                match value {
                    Some(v) => Current::Value(v),
                    None => Current::NoFigure(setter.and_then(|e| e.note.clone())),
                }
            },
            grounding: setter
                .map(|e| e.grounding.clone())
                .unwrap_or(Grounding::NotChecked),
            replaced: evs.iter().any(|e| matches!(e.op, Op::Replace { .. })),
            unmatched_deletion,
            unproven_deletion,
            unverified_change,
            delivered: match &delivery {
                Some(d) => d.delivered(&m),
                None => Counted::none("no PLD delivery data supplied"),
            },
            percentage: matches!(m, Measure::AffordablePct { .. }),
            scheme_complete: delivery.as_ref().and_then(|d| d.scheme_complete),
        };
        let (status, reason) = decide(&facts);
        rows.push(Row {
            label: evs
                .first()
                .map(|e| e.label.clone())
                .unwrap_or_else(|| m.label()),
            agreed: match evs.first().map(|e| &e.op) {
                Some(Op::Introduce { value }) => *value,
                _ => None,
            },
            current: value,
            permitted: delivery.as_ref().and_then(|d| d.permitted(&m)),
            delivered: facts.delivered,
            events: evs,
            measure: m,
            status,
            reason,
        });
    }
    if let Some(d) = &delivery {
        let deed_tenures: Vec<Tenure> = rows
            .iter()
            .filter(|r| r.status != RowStatus::Removed)
            .filter_map(|r| match r.measure {
                Measure::TenureUnits { tenure } => Some(tenure),
                _ => None,
            })
            .collect();
        // Only a deed that splits tenures can be said not to provide for one.
        if !deed_tenures.is_empty() {
            for (tenure, n) in d.uncovered(&deed_tenures) {
                rows.push(Row {
                    measure: Measure::TenureUnits { tenure },
                    label: format!("{} units (not in deed)", tenure.label()),
                    agreed: None,
                    current: None,
                    events: vec![],
                    permitted: d.permitted(&Measure::TenureUnits { tenure }),
                    delivered: Counted {
                        value: Some(Decimal::from(n as u64)),
                        basis: format!(
                            "{n} completed unit record(s) with a tenure no deed tenure covers"
                        ),
                    },
                    status: RowStatus::DiffersReviewRequired,
                    reason: "delivered in a tenure the deed does not provide for".into(),
                });
            }
        }
    }
    Ok(Reconciliation {
        reconcile_version: RECONCILE_VERSION.into(),
        development: development.into(),
        instruments: instruments.into_iter().map(|i| i.instrument).collect(),
        delivery,
        rows,
        warnings,
    })
}

// --------------------------------------------------------------------------- report

fn num(v: Option<Decimal>) -> String {
    v.map(|d| d.normalize().to_string())
        .unwrap_or_else(|| "–".into())
}

fn cite(e: &ObligationEvent) -> String {
    let g = match &e.grounding {
        Grounding::Grounded => "",
        Grounding::Ungrounded(_) => " (unproven)",
        Grounding::NotChecked => " (not checked)",
    };
    let op = match e.op {
        Op::Introduce { .. } => "",
        Op::Restate { .. } => " restated",
        Op::Replace { .. } => " replaced",
        Op::Unchanged => " cited",
        Op::Delete => " deleted",
    };
    match &e.evidence {
        Some(ev) => format!("p{}{op}{g}", ev.page),
        None => format!("no citation{op}{g}"),
    }
}

/// A Markdown table: one row per obligation, each cell with its source.
pub fn markdown(r: &Reconciliation) -> String {
    let mut s = format!("# Reconciliation: {}\n\n", r.development);
    for (k, i) in r.instruments.iter().enumerate() {
        s += &format!(
            "- Instrument {}: {} dated {}, {}; sha256 `{}`\n",
            k + 1,
            i.kind,
            i.date.as_deref().unwrap_or("?"),
            i.legal_reference.as_deref().unwrap_or("no legal reference"),
            i.sha256
        );
    }
    match &r.delivery {
        Some(d) => {
            s += &format!(
                "- PLD record `{}` ({}) as of {}: status {}, started {}, completed {}\n",
                d.record_id,
                d.lpa_app_no.as_deref().unwrap_or("no planning reference"),
                d.as_of,
                d.status.as_deref().unwrap_or("?"),
                d.started_on.map(|x| x.to_string()).unwrap_or("?".into()),
                d.completed_on.map(|x| x.to_string()).unwrap_or("?".into()),
            );
            s += &format!(
                "  - snapshot `{}` retrieved {} (sha256 `{}`), PLD mapping v{}\n",
                d.snapshot.path, d.snapshot.retrieved_at, d.snapshot.sha256, d.mapping_version
            );
        }
        None => s += "- No PLD delivery data supplied\n",
    }
    for w in &r.warnings {
        s += &format!("- Warning: {w}\n");
    }
    s += "\n| Obligation | Agreed | Current | Deed source | PLD permitted | PLD delivered | Status | Why |\n";
    s += "|---|---|---|---|---|---|---|---|\n";
    for row in &r.rows {
        let source = row.events.iter().map(cite).collect::<Vec<_>>().join("; ");
        s += &format!(
            "| {} | {} | {} | {} | {} | {} | {} | {} |\n",
            row.label.replace('|', "\\|"),
            num(row.agreed),
            num(row.current),
            if source.is_empty() {
                "–".into()
            } else {
                source
            },
            num(row.permitted),
            num(row.delivered.value),
            row.status.label(),
            row.reason.replace('|', "\\|"),
        );
    }
    s += "\n\"Counts match\" compares completed-unit counts only; it says nothing about tenure in \
          use, rents or quality. \"Differs: review\" is never a finding that an obligation was \
          unmet: PLD completions are borough-reported and unverified on receipt, and tenure at \
          completion is not tenure in use.\n";
    s
}
