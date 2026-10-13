//! Layer B, first cut (roadmap M5): what one development's deeds oblige, set against what PLD
//! says was permitted and delivered.
//!
//! * Obligations become `ObligationEvent`s, one per measure per instrument. Nothing is
//!   overwritten: a deed of variation adds `Varied` events and the current obligation is the last
//!   event for each measure, in the order the instruments are given.
//! * Delivery comes from `Delivery`, built from PLD snapshots (application and unit records).
//!   Every delivered figure cites the snapshot files it was counted from.
//! * Each row gets a status. A shortfall on a completed scheme is `DiffersReviewRequired`, never
//!   "unmet": PLD completions are borough-reported and tenure at completion isn't tenure in use.
//!
//! Every deed value is grounded the same way the verifier grounds it (value in snippet, snippet
//! on its page); an ungrounded deed value makes its row `Indeterminate`.

use std::collections::BTreeMap;

use chrono::NaiveDate;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::evidence::Pages;
use crate::pld::SnapshotRef;
use crate::schema::{AffordableBasis, Agreement, ConditionalityKind, Evidence, Tenure};
use crate::text::Value;
use crate::verify::{Input, grounding_problem};

pub const RECONCILE_VERSION: &str = "1";

// --------------------------------------------------------------------------- obligations

/// What is measured. Labels are stable: they key rows across instruments and runs.
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
    /// A stated sum (fixed, or a cap at its maximum); None when the deed states no figure.
    Contribution {
        name: String,
    },
}

impl Measure {
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
            Measure::Contribution { name } => format!("{name} (£)"),
        }
    }
}

fn basis_label(b: AffordableBasis) -> &'static str {
    match b {
        AffordableBasis::HabitableRooms => "habitable room",
        AffordableBasis::Units => "unit",
        AffordableBasis::Floorspace => "floorspace",
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    /// from an agreement or unilateral undertaking
    Original,
    /// a replacement value from a deed of variation or supplemental deed
    Varied,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "grounding", content = "detail", rename_all = "snake_case")]
pub enum Grounding {
    /// the value appears in its snippet and the snippet is on its cited page
    Grounded,
    Ungrounded(String),
    /// no page text was supplied
    NotChecked,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstrumentRef {
    pub sha256: String,
    pub kind: String,
    pub date: Option<String>,
    pub legal_reference: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObligationEvent {
    #[serde(flatten)]
    pub measure: Measure,
    pub value: Option<Decimal>,
    pub kind: EventKind,
    pub instrument: InstrumentRef,
    pub evidence: Option<Evidence>,
    pub grounding: Grounding,
    /// e.g. the condition of a conditional contribution
    pub note: Option<String>,
}

/// One event per extracted measure of one instrument. `pages` grounds each value.
pub fn obligation_events(a: &Agreement, pages: Option<&Pages>) -> Vec<ObligationEvent> {
    let kind = if a.instrument.kind.is_variation() {
        EventKind::Varied
    } else {
        EventKind::Original
    };
    let instrument = InstrumentRef {
        sha256: a.provenance.sha256.clone(),
        kind: format!("{:?}", a.instrument.kind),
        date: a.metadata.agreement_date.clone(),
        legal_reference: a.metadata.legal_reference.clone(),
    };
    let ground = |label: &str, value: Option<Decimal>, evidence: &Option<Evidence>| match pages {
        None => Grounding::NotChecked,
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
    let mut out = vec![];
    let mut push = |measure: Measure, value: Option<Decimal>, evidence: Option<Evidence>| {
        if value.is_none() && evidence.is_none() {
            return;
        }
        out.push(ObligationEvent {
            grounding: ground(&measure.label(), value, &evidence),
            measure,
            value,
            kind,
            instrument: instrument.clone(),
            evidence,
            note: None,
        });
    };

    let h = &a.housing;
    let dec = |v: Option<u32>| v.map(Decimal::from);
    // Variations restate only what changes; scheme totals from them are replacement values too.
    push(
        Measure::TotalUnits,
        dec(h.total_units),
        h.total_units_evidence.clone(),
    );
    push(
        Measure::MarketUnits,
        dec(h.market_units),
        h.market_units_evidence.clone(),
    );
    push(
        Measure::AffordableUnits,
        dec(h.affordable_units),
        h.affordable_units_evidence.clone(),
    );
    push(
        Measure::AffordableHabitableRooms,
        dec(h.affordable_habitable_rooms),
        h.affordable_units_evidence.clone(),
    );
    for t in &h.tenures {
        push(
            Measure::TenureUnits { tenure: t.tenure },
            dec(t.units),
            t.evidence.clone(),
        );
        if t.habitable_rooms.is_some() {
            push(
                Measure::TenureHabitableRooms { tenure: t.tenure },
                dec(t.habitable_rooms),
                t.evidence.clone(),
            );
        }
    }
    if let Some(basis) = h.claimed_affordable_basis {
        push(
            Measure::AffordablePct { basis },
            h.claimed_affordable_pct,
            h.claimed_affordable_evidence.clone(),
        );
    }
    for c in &a.contributions {
        let value = c.amount.stated_sum_gbp();
        let measure = Measure::Contribution {
            name: c.name.clone(),
        };
        out.push(ObligationEvent {
            grounding: if value.is_some() {
                ground(&measure.label(), value, &c.amount.evidence)
            } else {
                // nothing numeric to ground; the verifier's own checks cover the wording
                Grounding::NotChecked
            },
            measure,
            value,
            kind,
            instrument: instrument.clone(),
            evidence: c.amount.evidence.clone(),
            note: match c.conditionality.kind {
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
        });
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeliveredUnit {
    /// PLD's unit identifier
    pub id: String,
    pub tenure: UnitTenure,
    pub habitable_rooms: Option<u32>,
    pub completed_on: Option<NaiveDate>,
    /// index into `Delivery::snapshots`
    pub snapshot: usize,
}

/// Figures PLD records for the permission (proposed), not for delivery.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Permitted {
    pub total_units: Option<u32>,
    pub market_units: Option<u32>,
    pub affordable_units: Option<u32>,
    #[serde(default)]
    pub tenure_units: Vec<(Tenure, u32)>,
    /// index into `Delivery::snapshots`
    pub snapshot: Option<usize>,
}

/// PLD's view of one development: permission figures, scheme status and unit records.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Delivery {
    /// PLD record id, e.g. `Southwark-19_AP_1773`
    pub record_id: String,
    pub snapshots: Vec<SnapshotRef>,
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

impl Delivery {
    fn completed(&self) -> impl Iterator<Item = &DeliveredUnit> {
        self.units.iter().filter(|u| u.completed_on.is_some())
    }

    fn unrecognised(&self) -> usize {
        self.completed()
            .filter(|u| matches!(u.tenure, UnitTenure::Unrecognised(_)))
            .count()
    }

    fn count(&self, what: &str, f: impl Fn(&UnitTenure) -> bool, rooms: bool) -> Counted {
        let n_done = self.completed().count();
        if self.units.is_empty() {
            return Counted {
                value: None,
                basis: "no PLD unit records".into(),
            };
        }
        let unknown = self.unrecognised();
        if unknown > 0 && what != "all" {
            return Counted {
                value: None,
                basis: format!("{unknown} completed unit record(s) have an unrecognised tenure"),
            };
        }
        let matching: Vec<&DeliveredUnit> = self.completed().filter(|u| f(&u.tenure)).collect();
        if rooms {
            let missing = matching
                .iter()
                .filter(|u| u.habitable_rooms.is_none())
                .count();
            if missing > 0 {
                return Counted {
                    value: None,
                    basis: format!("{missing} matching unit record(s) lack habitable rooms"),
                };
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
                        return Counted {
                            value: None,
                            basis: "PLD unit records carry no floorspace basis".into(),
                        };
                    }
                };
                let num = self.count("affordable", affordable, rooms);
                let den = self.count("all", |_| true, rooms);
                match (num.value, den.value) {
                    (Some(n), Some(d)) if !d.is_zero() => Counted {
                        value: Some((n * Decimal::ONE_HUNDRED / d).round_dp(2)),
                        basis: format!("{} / {}", num.basis, den.basis),
                    },
                    _ => Counted {
                        value: None,
                        basis: if num.value.is_none() {
                            num.basis
                        } else {
                            den.basis
                        },
                    },
                }
            }
            Measure::Contribution { .. } => Counted {
                value: None,
                basis: "PLD records no S106 payments".into(),
            },
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

// --------------------------------------------------------------------------- reconciliation

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RowStatus {
    /// delivered equals the current obligation
    Consistent,
    /// the scheme isn't complete and delivery is still short
    NotYetDue,
    /// the obligation was varied; compare against the variation by hand
    Varied,
    /// delivery differs from the obligation: a human must check the borough's records.
    /// Never a finding that the obligation was unmet.
    DiffersReviewRequired,
    /// something needed for a comparison is missing or unproven
    Indeterminate,
}

impl RowStatus {
    pub fn label(self) -> &'static str {
        match self {
            RowStatus::Consistent => "consistent",
            RowStatus::NotYetDue => "not yet due",
            RowStatus::Varied => "varied",
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
    /// as first agreed
    pub agreed: Option<Decimal>,
    /// after every variation given (the last event)
    pub current: Option<Decimal>,
    /// all events for this measure, in instrument order
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
    pub instruments: Vec<InstrumentRef>,
    pub delivery: Option<Delivery>,
    pub rows: Vec<Row>,
}

fn decide(row: &Row, delivery: Option<&Delivery>) -> (RowStatus, String) {
    let Some(last) = row.events.last() else {
        return (RowStatus::Indeterminate, "no obligation event".into());
    };
    let Some(current) = row.current else {
        return (
            RowStatus::Indeterminate,
            match &last.note {
                Some(n) => format!("no stated figure ({n})"),
                None => "no stated figure in the deed".into(),
            },
        );
    };
    if let Grounding::Ungrounded(why) = &last.grounding {
        return (
            RowStatus::Indeterminate,
            format!("deed value unproven: {why}"),
        );
    }
    let varied = row.events.iter().any(|e| e.kind == EventKind::Varied);
    let Some(delivered) = row.delivered.value else {
        return if varied {
            (RowStatus::Varied, row.delivered.basis.clone())
        } else {
            (RowStatus::Indeterminate, row.delivered.basis.clone())
        };
    };
    if delivered == current {
        return (RowStatus::Consistent, row.delivered.basis.clone());
    }
    if varied {
        return (
            RowStatus::Varied,
            format!("delivered {delivered} vs varied obligation {current}; compare by hand"),
        );
    }
    let complete = delivery.and_then(|d| d.scheme_complete);
    match complete {
        Some(true) => (
            RowStatus::DiffersReviewRequired,
            format!(
                "PLD reports the scheme complete with {delivered} against {current}; \
                 check the borough's records before drawing any conclusion"
            ),
        ),
        _ if delivered < current && complete == Some(false) => (
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

/// `events`: from `obligation_events`, instruments in chronological order (the original first).
pub fn reconcile(
    development: &str,
    events: Vec<ObligationEvent>,
    delivery: Option<Delivery>,
) -> Reconciliation {
    let mut instruments: Vec<InstrumentRef> = vec![];
    for e in &events {
        if !instruments.iter().any(|i| i.sha256 == e.instrument.sha256) {
            instruments.push(e.instrument.clone());
        }
    }
    // Rows in first-seen order, so the table reads like the deed.
    let mut order: Vec<Measure> = vec![];
    let mut by_measure: BTreeMap<Measure, Vec<ObligationEvent>> = BTreeMap::new();
    for e in events {
        if !by_measure.contains_key(&e.measure) {
            order.push(e.measure.clone());
        }
        by_measure.entry(e.measure.clone()).or_default().push(e);
    }
    let mut rows = vec![];
    for m in order {
        let evs = by_measure.remove(&m).unwrap_or_default();
        let mut row = Row {
            label: m.label(),
            agreed: evs.first().and_then(|e| e.value),
            current: evs.last().and_then(|e| e.value),
            events: evs,
            permitted: delivery.as_ref().and_then(|d| d.permitted(&m)),
            delivered: match &delivery {
                Some(d) => d.delivered(&m),
                None => Counted {
                    value: None,
                    basis: "no PLD delivery data supplied".into(),
                },
            },
            measure: m,
            status: RowStatus::Indeterminate,
            reason: String::new(),
        };
        (row.status, row.reason) = decide(&row, delivery.as_ref());
        rows.push(row);
    }
    if let Some(d) = &delivery {
        let deed_tenures: Vec<Tenure> = rows
            .iter()
            .filter_map(|r| match r.measure {
                Measure::TenureUnits { tenure } => Some(tenure),
                _ => None,
            })
            .collect();
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
    Reconciliation {
        reconcile_version: RECONCILE_VERSION.into(),
        development: development.into(),
        instruments,
        delivery,
        rows,
    }
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
    match &e.evidence {
        Some(ev) => format!("p{}{g}", ev.page),
        None => format!("no citation{g}"),
    }
}

/// A Markdown table: one row per measure, each cell with its source.
pub fn markdown(r: &Reconciliation) -> String {
    let mut s = format!("# Reconciliation: {}\n\n", r.development);
    for i in &r.instruments {
        s += &format!(
            "- Instrument: {} dated {}, {}; sha256 `{}`\n",
            i.kind,
            i.date.as_deref().unwrap_or("?"),
            i.legal_reference.as_deref().unwrap_or("no legal reference"),
            i.sha256
        );
    }
    match &r.delivery {
        Some(d) => {
            s += &format!(
                "- PLD record `{}`: status {}, started {}, completed {}\n",
                d.record_id,
                d.status.as_deref().unwrap_or("?"),
                d.started_on.map(|x| x.to_string()).unwrap_or("?".into()),
                d.completed_on.map(|x| x.to_string()).unwrap_or("?".into()),
            );
            for (k, snap) in d.snapshots.iter().enumerate() {
                s += &format!(
                    "  - snapshot [{k}] `{}` retrieved {} (sha256 `{}`)\n",
                    snap.path, snap.retrieved_at, snap.sha256
                );
            }
        }
        None => s += "- No PLD delivery data supplied\n",
    }
    s += "\n| Measure | Agreed | Current | Deed source | PLD permitted | PLD delivered | Status | Why |\n";
    s += "|---|---|---|---|---|---|---|---|\n";
    for row in &r.rows {
        let source = row.events.iter().map(cite).collect::<Vec<_>>().join("; ");
        s += &format!(
            "| {} | {} | {} | {} | {} | {} | {} | {} |\n",
            row.label,
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
    s += "\n\"differs: review\" is never a finding that an obligation was unmet: PLD completions are \
          borough-reported and unverified on receipt, and tenure at completion is not tenure in use.\n";
    s
}
