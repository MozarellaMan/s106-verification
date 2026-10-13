//! Layer B, first cut (roadmap M5): what one development's deeds oblige, set against what PLD
//! says was permitted and delivered.
//!
//! * Each deed becomes an `InstrumentObligations`: one `Statement` per obligation it states, plus
//!   the deletions it makes. Duplicates within one deed are merged, or flagged when their figures
//!   differ.
//! * `reconcile` collects every deed's statements and deletions per obligation (`events`), in
//!   the order given, without applying them. The first deed sets the figure. Anything a later deed
//!   changes (a different figure, a deletion, a partial scope, no planning reference, differing
//!   figures) adds a `ReviewReason` and the row goes to a person (`facts`). A restatement of the
//!   same figure stays automatic.
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

use crate::evidence::Pages;
use crate::pld::{Snapshot, SnapshotRef};
use crate::schema::{
    AffordableBasis, Agreement, ConditionalityKind, Evidence, Tenure, VariationOp,
};
use crate::text::Value;
use crate::verify::{Input, PCT_TOLERANCE, grounding_problem};

pub const RECONCILE_VERSION: &str = "3";
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
}

/// What one deed says, before it is set against the deeds before it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstrumentObligations {
    pub instrument: InstrumentRef,
    /// a deed of variation or supplemental deed
    pub variation: bool,
    pub planning_references: Vec<String>,
    /// a partial scope the deed states, e.g. "in so far as it relates to Phase 2"; anything a
    /// scoped deed says about an obligation goes to review
    #[serde(default)]
    pub scope: Option<String>,
    pub statements: Vec<Statement>,
    pub deletions: Vec<Deletion>,
}

/// What a deed does with an obligation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    /// states it, with a figure or without one
    States,
    /// deletes it (a variation's deletion whose target names it)
    Deletes,
}

/// One deed's statement or deletion of one obligation, recorded as the deed gives it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObligationEvent {
    #[serde(flatten)]
    pub measure: Measure,
    pub label: String,
    pub action: Action,
    pub value: Option<Decimal>,
    /// the instrument's position in the order given, from 1
    pub instrument_no: usize,
    pub instrument: InstrumentRef,
    pub evidence: Option<Evidence>,
    pub grounding: Grounding,
    pub note: Option<String>,
    pub conflict: Option<String>,
    /// false when the instrument states no planning reference, so its link to the development
    /// couldn't be checked
    #[serde(default = "yes")]
    pub verified: bool,
    /// the instrument's partial scope, when it has one
    #[serde(default)]
    pub scope: Option<String>,
    /// for a deletion, its target as the deed names it
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
}

fn yes() -> bool {
    true
}

/// One statement per extracted measure of one deed. `pages` grounds each value.
pub fn instrument_obligations(a: &Agreement, pages: Option<&Pages>) -> InstrumentObligations {
    let ground = |m: &Measure, label: &str, value: Option<Decimal>, evidence: &Option<Evidence>| {
        let Some((p, v)) = pages.zip(value) else {
            return Grounding::NotChecked;
        };
        {
            let input = Input {
                label: label.to_string(),
                field: None,
                value: Some(match m {
                    Measure::Contribution { .. } => Value::Gbp(v),
                    Measure::AffordablePct { .. } => Value::Percent(v),
                    _ => Value::Count(v),
                }),
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
            grounding: ground(&measure, &label, value, &evidence),
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
            h.claimed_affordable_pct
                .as_ref()
                .and_then(crate::schema::Observed::ok)
                .copied(),
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
        scope: a.instrument.scope.as_ref().map(|q| q.text.clone()),
        statements: merge_duplicates(raw),
        deletions: a
            .variations
            .iter()
            .filter(|v| v.op == VariationOp::Delete)
            .map(|v| Deletion {
                target: v.target.clone(),
                evidence: v.evidence.clone(),
            })
            .collect(),
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
    /// why there is no value
    #[serde(default)]
    pub cause: Option<Cause>,
}

impl Counted {
    pub fn some(value: Decimal, basis: impl Into<String>) -> Counted {
        Counted {
            value: Some(value),
            basis: basis.into(),
            cause: None,
        }
    }

    pub fn none(cause: Cause, basis: impl Into<String>) -> Counted {
        Counted {
            value: None,
            basis: basis.into(),
            cause: Some(cause),
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
            return Counted::none(Cause::NoUnitRecords, "no PLD unit records");
        }
        if what != "all" {
            let missing = self
                .completed()
                .filter(|u| u.tenure == UnitTenure::Missing)
                .count();
            if missing > 0 {
                return Counted::none(
                    Cause::UnknownTenure,
                    format!("{missing} completed unit record(s) give no tenure"),
                );
            }
            let unknown = self
                .completed()
                .filter(|u| matches!(u.tenure, UnitTenure::Unrecognised(_)))
                .count();
            if unknown > 0 {
                return Counted::none(
                    Cause::UnknownTenure,
                    format!("{unknown} completed unit record(s) have an unrecognised tenure"),
                );
            }
        }
        let matching: Vec<&DeliveredUnit> = self.completed().filter(|u| f(&u.tenure)).collect();
        if rooms {
            let missing = matching
                .iter()
                .filter(|u| u.habitable_rooms.is_none())
                .count();
            if missing > 0 {
                return Counted::none(
                    Cause::NoRooms,
                    format!("{missing} matching unit record(s) lack habitable rooms"),
                );
            }
            let sum: u32 = matching.iter().filter_map(|u| u.habitable_rooms).sum();
            return Counted::some(
                Decimal::from(sum),
                format!(
                    "habitable rooms of {} {what} unit record(s) with a completion date",
                    matching.len()
                ),
            );
        }
        Counted::some(
            Decimal::from(matching.len() as u64),
            format!(
                "{} {what} of {n_done} unit record(s) with a completion date ({} records in all)",
                matching.len(),
                self.units.len()
            ),
        )
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
                        return Counted::none(
                            Cause::NoFloorspaceBasis,
                            "PLD unit records carry no floorspace basis",
                        );
                    }
                };
                let num = self.count("affordable", affordable, rooms);
                let den = self.count("all", |_| true, rooms);
                match (num.value, den.value) {
                    (Some(n), Some(d)) if !d.is_zero() => Counted::some(
                        (n * Decimal::ONE_HUNDRED / d).round_dp(2),
                        format!("{} / {}", num.basis, den.basis),
                    ),
                    (None, _) => num,
                    (_, None) => den,
                    _ => Counted::none(Cause::NoUnitRecords, "no completed units to divide by"),
                }
            }
            Measure::Contribution { .. } => {
                Counted::none(Cause::NoPayments, "PLD records no S106 payments")
            }
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

/// The part of a PLD application record the mapping reads. Every field here must be present. The
/// `Option`s may be null, and they are the ones PLD leaves null (dates and status before a scheme
/// starts, a unit's number or tenure), so a renamed or retyped field fails the read instead of
/// turning every row indeterminate.
#[derive(Debug, Deserialize)]
struct PldApplication {
    id: String,
    #[serde(deserialize_with = "nullable")]
    lpa_app_no: Option<String>,
    #[serde(deserialize_with = "nullable")]
    status: Option<String>,
    #[serde(deserialize_with = "nullable")]
    actual_commencement_date: Option<String>,
    #[serde(deserialize_with = "nullable")]
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
    #[serde(deserialize_with = "nullable")]
    tenure: Option<String>,
    #[serde(deserialize_with = "nullable")]
    actual_completion_date: Option<String>,
    #[serde(deserialize_with = "nullable")]
    unit_no: Option<serde_json::Value>,
}

/// A field that must be present but may be null. (Serde treats a missing `Option` field as
/// None; with `deserialize_with` and no `default`, a missing field is an error instead.)
fn nullable<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
    d: D,
) -> std::result::Result<Option<T>, D::Error> {
    Option::<T>::deserialize(d)
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RowStatus {
    /// PLD's completed-unit count equals the deed's figure. Counts only: says nothing about
    /// tenure in use, rents or quality.
    CountsMatch,
    /// a later deed changed the figure, and delivery equals the latest figure from a grounded,
    /// verified, whole-scheme deed: a match either way, so no judgement is needed
    CountsMatchVariedFigure,
    /// the scheme isn't complete and delivery is still short
    NotYetDue,
    /// a later deed did something that needs a person (see the row's `review` reasons)
    Varied,
    /// delivery differs from the obligation: a human must check the borough's records.
    /// Never a finding that the obligation was unmet.
    DiffersReviewRequired,
    /// something needed for a comparison is missing or unproven (see the row's `cause`)
    Indeterminate,
}

impl RowStatus {
    pub fn label(self) -> &'static str {
        match self {
            RowStatus::CountsMatch => "counts match",
            RowStatus::CountsMatchVariedFigure => "counts match (varied figure)",
            RowStatus::NotYetDue => "not yet due",
            RowStatus::Varied => "varied: compare by hand",
            RowStatus::DiffersReviewRequired => "differs: review",
            RowStatus::Indeterminate => "indeterminate",
        }
    }

    /// decided without a person's judgement on the deeds (a "differs" row still goes to a
    /// person, but on delivery, which is what the reconciliation is for)
    pub fn decided(self) -> bool {
        !matches!(self, RowStatus::Varied | RowStatus::Indeterminate)
    }
}

/// Why a row is indeterminate, as a code a research loop can count.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Cause {
    /// the deed states no figure (a formula, a rate, or none)
    NoFigure,
    /// the first deed states differing figures for it
    Conflict,
    /// the deed's figure isn't grounded on its page
    Ungrounded,
    /// no PLD record was supplied
    NoDeliveryData,
    NoUnitRecords,
    /// PLD records no S106 payments
    NoPayments,
    /// unit records carry no habitable rooms
    NoRooms,
    /// a completed unit's tenure is missing or unrecognised
    UnknownTenure,
    /// PLD has no floorspace basis
    NoFloorspaceBasis,
    SchemeStatusUnknown,
}

/// Why a row needs a person: something a later deed did. Instruments are numbered from 1 in the
/// order given.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "reason", rename_all = "snake_case")]
pub enum ReviewReason {
    /// states a different figure from the one before it
    Replaced {
        instrument: usize,
        from: Option<Decimal>,
        to: Decimal,
    },
    /// a deletion whose target names this obligation
    Deleted { instrument: usize, target: String },
    /// from a deed that applies to only part of the development
    Scoped { instrument: usize, scope: String },
    /// from a deed that states no planning reference
    Unverified { instrument: usize },
    /// the deed states differing figures for it
    Conflict { instrument: usize, detail: String },
    /// restates a figure that isn't grounded on its page
    Ungrounded { instrument: usize, why: String },
    /// a later deed deletes something that matches no obligation; it could be this one
    UnmatchedDeletion { instrument: usize, target: String },
}

impl ReviewReason {
    pub fn code(&self) -> &'static str {
        match self {
            ReviewReason::Replaced { .. } => "replaced",
            ReviewReason::Deleted { .. } => "deleted",
            ReviewReason::Scoped { .. } => "scoped",
            ReviewReason::Unverified { .. } => "unverified",
            ReviewReason::Conflict { .. } => "conflict",
            ReviewReason::Ungrounded { .. } => "ungrounded",
            ReviewReason::UnmatchedDeletion { .. } => "unmatched_deletion",
        }
    }

    fn describe(&self) -> String {
        match self {
            ReviewReason::Replaced {
                instrument,
                from,
                to,
            } => format!("instrument {instrument} states {to} (was {})", num(*from)),
            ReviewReason::Deleted { instrument, target } => {
                format!("instrument {instrument} deletes \"{target}\"")
            }
            ReviewReason::Scoped { instrument, scope } => {
                format!("instrument {instrument} applies only \"{scope}\"")
            }
            ReviewReason::Unverified { instrument } => {
                format!("instrument {instrument} states no planning reference")
            }
            ReviewReason::Conflict { instrument, detail } => {
                format!("instrument {instrument}: {detail}")
            }
            ReviewReason::Ungrounded { instrument, why } => {
                format!("instrument {instrument}'s figure is unproven: {why}")
            }
            ReviewReason::UnmatchedDeletion { instrument, target } => {
                format!("instrument {instrument} deletes \"{target}\", which matches no obligation")
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Row {
    #[serde(flatten)]
    pub measure: Measure,
    pub label: String,
    /// as the first deed states it
    pub agreed: Option<Decimal>,
    /// the latest figure a verified, whole-scheme deed states (shown, not applied)
    pub current: Option<Decimal>,
    /// every deed's statement or deletion of this obligation, in instrument order
    pub events: Vec<ObligationEvent>,
    /// PLD's permission figure
    pub permitted: Option<Decimal>,
    pub delivered: Counted,
    pub status: RowStatus,
    pub reason: String,
    /// set when the status is indeterminate
    pub cause: Option<Cause>,
    /// set when a later deed did something that needs a person
    #[serde(default)]
    pub review: Vec<ReviewReason>,
}

/// Counts for a research loop: how much was decided without a person, and what the rest waits on.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Summary {
    pub rows: usize,
    /// rows whose status is decided (`RowStatus::decided`)
    pub decided: usize,
    pub by_status: BTreeMap<RowStatus, usize>,
    pub indeterminate_by_cause: BTreeMap<Cause, usize>,
    /// review rows by reason code; a row with two reasons counts under each
    pub review_by_reason: BTreeMap<String, usize>,
}

impl Summary {
    pub fn of(rows: &[Row]) -> Summary {
        let mut s = Summary {
            rows: rows.len(),
            ..Default::default()
        };
        for r in rows {
            *s.by_status.entry(r.status).or_default() += 1;
            s.decided += usize::from(r.status.decided());
            if let Some(c) = r.cause {
                *s.indeterminate_by_cause.entry(c).or_default() += 1;
            }
            let mut codes: Vec<&str> = r.review.iter().map(ReviewReason::code).collect();
            codes.sort_unstable();
            codes.dedup();
            for c in codes {
                *s.review_by_reason.entry(c.into()).or_default() += 1;
            }
        }
        s
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Reconciliation {
    pub reconcile_version: String,
    pub development: String,
    /// in the order given
    pub instruments: Vec<InstrumentRef>,
    pub delivery: Option<Delivery>,
    pub rows: Vec<Row>,
    pub summary: Summary,
    /// things a reader should know that don't belong to one row
    #[serde(default)]
    pub warnings: Vec<String>,
}

/// The first deed's figure for an obligation.
#[derive(Debug, Clone, PartialEq)]
pub enum Current {
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
    /// the grounding of the first deed's figure
    pub grounding: Grounding,
    /// what later deeds did that needs a person
    pub review: Vec<ReviewReason>,
    /// the latest figure from a later deed that is grounded, verified and whole-scheme, when
    /// the only review reasons are replacements
    pub latest: Option<Decimal>,
    pub delivered: Counted,
    /// compare within the verifier's percentage tolerance rather than exactly
    pub percentage: bool,
    pub scheme_complete: Option<bool>,
}

/// The status rules, in order (see docs/reconciliation.md): a status, the reason in words, and
/// the cause code when indeterminate.
pub fn decide(f: &Facts) -> (RowStatus, String, Option<Cause>) {
    let same = |a: Decimal, b: Decimal| {
        if f.percentage {
            (a - b).abs() <= PCT_TOLERANCE
        } else {
            a == b
        }
    };
    if !f.review.is_empty() {
        if let (Some(latest), Some(d)) = (f.latest, f.delivered.value)
            && same(d, latest)
        {
            return (
                RowStatus::CountsMatchVariedFigure,
                format!("{}; {}", f.delivered.basis, join_reasons(&f.review)),
                None,
            );
        }
        return (RowStatus::Varied, join_reasons(&f.review), None);
    }
    let current = match &f.current {
        Current::Conflict(why) => {
            return (RowStatus::Indeterminate, why.clone(), Some(Cause::Conflict));
        }
        Current::NoFigure(note) => {
            return (
                RowStatus::Indeterminate,
                match note {
                    Some(n) => format!("no stated figure ({n})"),
                    None => "no stated figure in the deed".into(),
                },
                Some(Cause::NoFigure),
            );
        }
        Current::Value(v) => *v,
    };
    if let Grounding::Ungrounded(why) = &f.grounding {
        return (
            RowStatus::Indeterminate,
            format!("deed value unproven: {why}"),
            Some(Cause::Ungrounded),
        );
    }
    let Some(delivered) = f.delivered.value else {
        return (
            RowStatus::Indeterminate,
            f.delivered.basis.clone(),
            Some(f.delivered.cause.unwrap_or(Cause::NoDeliveryData)),
        );
    };
    if same(delivered, current) {
        return (RowStatus::CountsMatch, f.delivered.basis.clone(), None);
    }
    match f.scheme_complete {
        Some(true) => (
            RowStatus::DiffersReviewRequired,
            format!(
                "PLD reports the scheme complete with {delivered} against {current}; \
                 check the borough's records before drawing any conclusion"
            ),
            None,
        ),
        Some(false) if delivered < current => (
            RowStatus::NotYetDue,
            format!("{delivered} of {current} so far; PLD does not report the scheme complete"),
            None,
        ),
        Some(false) => (
            RowStatus::DiffersReviewRequired,
            format!("{delivered} delivered so far against {current}, scheme not complete"),
            None,
        ),
        None => (
            RowStatus::Indeterminate,
            format!("{delivered} against {current}; PLD scheme status not established"),
            Some(Cause::SchemeStatusUnknown),
        ),
    }
}

fn join_reasons(r: &[ReviewReason]) -> String {
    format!(
        "{}; compare by hand",
        r.iter()
            .map(ReviewReason::describe)
            .collect::<Vec<_>>()
            .join("; ")
    )
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

/// Does a deletion's target name this obligation? Only named contributions can be matched.
fn deletion_matches(target: &str, m: &Measure) -> bool {
    let t = contribution_key(target);
    match m {
        Measure::Contribution { key } => {
            !key.is_empty() && format!(" {t} ").contains(&format!(" {key} "))
        }
        _ => false,
    }
}

/// Obligations in first-stated order, their events, and deletions that matched nothing (with
/// their instrument number).
pub type Events = (
    Vec<Measure>,
    BTreeMap<Measure, Vec<ObligationEvent>>,
    Vec<(usize, String)>,
);

/// Collect every deed's statements and matched deletions per obligation, in instrument order.
/// No interpretation and no running state: that is `facts`. Also returns each deletion that
/// matched no obligation stated before it, with its instrument number.
pub fn events(instruments: &[InstrumentObligations], verified: &[bool]) -> Events {
    let mut order: Vec<Measure> = vec![];
    let mut by: BTreeMap<Measure, Vec<ObligationEvent>> = BTreeMap::new();
    let mut unmatched = vec![];
    for (k, inst) in instruments.iter().enumerate() {
        let event = |measure: &Measure,
                     label: &str,
                     action: Action,
                     value: Option<Decimal>,
                     evidence: Option<Evidence>| ObligationEvent {
            measure: measure.clone(),
            label: label.into(),
            action,
            value,
            instrument_no: k + 1,
            instrument: inst.instrument.clone(),
            evidence,
            grounding: Grounding::NotChecked,
            note: None,
            conflict: None,
            verified: verified[k],
            scope: inst.scope.clone(),
            target: None,
        };
        for s in &inst.statements {
            if !by.contains_key(&s.measure) {
                order.push(s.measure.clone());
            }
            by.entry(s.measure.clone())
                .or_default()
                .push(ObligationEvent {
                    grounding: s.grounding.clone(),
                    note: s.note.clone(),
                    conflict: s.conflict.clone(),
                    ..event(
                        &s.measure,
                        &s.label,
                        Action::States,
                        s.value,
                        s.evidence.clone(),
                    )
                });
        }
        for del in &inst.deletions {
            let hits: Vec<Measure> = order
                .iter()
                .filter(|m| deletion_matches(&del.target, m))
                .cloned()
                .collect();
            if hits.is_empty() {
                unmatched.push((k + 1, del.target.clone()));
            }
            for m in hits {
                let label = by[&m][0].label.clone();
                by.entry(m.clone()).or_default().push(ObligationEvent {
                    note: Some(format!("deletes \"{}\"", del.target)),
                    target: Some(del.target.clone()),
                    ..event(&m, &label, Action::Deletes, None, del.evidence.clone())
                });
            }
        }
    }
    (order, by, unmatched)
}

/// Read one obligation's events. The first deed sets the figure. A later event leaves the row
/// automatic only when it is a grounded restatement of the same figure, or a bare citation, from
/// a verified, whole-scheme deed; anything else adds a review reason. `unmatched` deletions from
/// deeds after the first event reach this row too.
pub fn facts(
    evs: &[ObligationEvent],
    unmatched: &[(usize, String)],
    delivery: Option<&Delivery>,
) -> Facts {
    let first = &evs[0];
    let mut review = vec![];
    // An obligation the original deed states is checked from its second event. One a later deed
    // introduces is checked from its first, as a replacement of nothing.
    let original = first.instrument_no == 1;
    let mut prev = if original { first.value } else { None };
    let mut latest = None;
    for e in &evs[usize::from(original)..] {
        let n = e.instrument_no;
        if !e.verified {
            review.push(ReviewReason::Unverified { instrument: n });
        }
        if let Some(scope) = &e.scope {
            review.push(ReviewReason::Scoped {
                instrument: n,
                scope: scope.clone(),
            });
        }
        if e.action == Action::Deletes {
            review.push(ReviewReason::Deleted {
                instrument: n,
                target: e.target.clone().unwrap_or_default(),
            });
            continue;
        }
        if let Some(c) = &e.conflict {
            review.push(ReviewReason::Conflict {
                instrument: n,
                detail: c.clone(),
            });
            continue;
        }
        let Some(v) = e.value else { continue };
        if let Grounding::Ungrounded(why) = &e.grounding {
            review.push(ReviewReason::Ungrounded {
                instrument: n,
                why: why.clone(),
            });
        }
        if Some(v) != prev {
            review.push(ReviewReason::Replaced {
                instrument: n,
                from: prev,
                to: v,
            });
        }
        prev = Some(v);
        let trusted =
            e.verified && e.scope.is_none() && !matches!(e.grounding, Grounding::Ungrounded(_));
        latest = trusted.then_some(v);
    }
    for (n, target) in unmatched {
        if *n > first.instrument_no {
            review.push(ReviewReason::UnmatchedDeletion {
                instrument: *n,
                target: target.clone(),
            });
        }
    }
    let only_replaced = review
        .iter()
        .all(|r| matches!(r, ReviewReason::Replaced { .. }));
    let m = &first.measure;
    Facts {
        current: match (&first.conflict, first.value) {
            (Some(c), _) => Current::Conflict(c.clone()),
            (None, Some(v)) => Current::Value(v),
            (None, None) => Current::NoFigure(first.note.clone()),
        },
        grounding: first.grounding.clone(),
        latest: if only_replaced { latest } else { None },
        review,
        delivered: match delivery {
            Some(d) => d.delivered(m),
            None => Counted::none(Cause::NoDeliveryData, "no PLD delivery data supplied"),
        },
        percentage: matches!(m, Measure::AffordablePct { .. }),
        scheme_complete: delivery.and_then(|d| d.scheme_complete),
    }
}

/// Set a development's deeds against PLD.
///
/// `instruments` are read in the order given, original first; a warning is raised when that
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
    // later deed citing none is recorded, and every obligation it mentions goes to review.
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
                    "instrument {} states no planning reference; what it says goes to review",
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
                "instrument {} is dated {b}, before instrument {} ({a}); instruments are read \
                 in the order given",
                k + 2,
                k + 1
            ));
        }
    }

    let (order, mut by, unmatched) = events(&instruments, &verified);
    for (n, target) in &unmatched {
        warnings.push(format!(
            "instrument {n} deletes \"{target}\", which isn't matched to any obligation"
        ));
    }
    let mut rows = vec![];
    for m in order {
        let evs = by.remove(&m).unwrap_or_default();
        let f = facts(&evs, &unmatched, delivery.as_ref());
        let (status, reason, cause) = decide(&f);
        let first = &evs[0];
        rows.push(Row {
            label: first.label.clone(),
            agreed: first.value,
            current: evs
                .iter()
                .rev()
                .filter(|e| {
                    e.action == Action::States
                        && e.conflict.is_none()
                        && e.verified
                        && e.scope.is_none()
                })
                .find_map(|e| e.value)
                .or(first.value),
            permitted: delivery.as_ref().and_then(|d| d.permitted(&m)),
            delivered: f.delivered,
            events: evs,
            measure: m,
            status,
            reason,
            cause,
            review: f.review,
        });
    }
    if let Some(d) = &delivery {
        let deed_tenures: Vec<Tenure> = rows
            .iter()
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
                    delivered: Counted::some(
                        Decimal::from(n as u64),
                        format!("{n} completed unit record(s) with a tenure no deed tenure covers"),
                    ),
                    status: RowStatus::DiffersReviewRequired,
                    reason: "delivered in a tenure the deed does not provide for".into(),
                    cause: None,
                    review: vec![],
                });
            }
        }
    }
    Ok(Reconciliation {
        reconcile_version: RECONCILE_VERSION.into(),
        development: development.into(),
        instruments: instruments.into_iter().map(|i| i.instrument).collect(),
        delivery,
        summary: Summary::of(&rows),
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
    let op = match (e.action, e.instrument_no) {
        (Action::Deletes, _) => format!(" deleted by {}", e.instrument_no),
        (_, 1) => String::new(),
        _ => format!(" in {}", e.instrument_no),
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
    let sm = &r.summary;
    s += &format!(
        "- Summary: {} of {} rows decided without review of the deeds",
        sm.decided, sm.rows
    );
    let causes: Vec<String> = sm
        .indeterminate_by_cause
        .iter()
        .map(|(c, n)| {
            format!(
                "{} {n}",
                serde_json::to_value(c).unwrap().as_str().unwrap_or("?")
            )
        })
        .collect();
    if !causes.is_empty() {
        s += &format!("; indeterminate by cause: {}", causes.join(", "));
    }
    let reasons: Vec<String> = sm
        .review_by_reason
        .iter()
        .map(|(c, n)| format!("{c} {n}"))
        .collect();
    if !reasons.is_empty() {
        s += &format!("; review by reason: {}", reasons.join(", "));
    }
    s += "\n";
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
