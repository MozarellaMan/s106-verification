//! Structured-output types the model fills in ("wire" types), and their conversion to schema v2.
//!
//! The wire types are deliberately narrower than the schema:
//! * Evidence is never optional: an empty snippet means "the deed doesn't state this". Optional
//!   evidence objects inflate the compiled output grammar past the API's limit.
//! * Lists instead of maps, and plain strings for money ("10058.60"), so the JSON schema stays strict.
//! * Pages are positions within the attached PDF; `PageMap` turns them into PDF page indices.
//!
//! Trigger labels are not trusted from the model: `triggers::classify` decides them from the cited
//! phrase (the model's label is kept only when it differs), so repeat runs agree.

use rust_decimal::Decimal;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::quantity::{self, ParseError};
use crate::schema::*;
use crate::triggers::classify_cited;

/// v1 fixtures have `null` where v2 uses an empty string; read both.
fn null_as_empty<'de, D: serde::Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    Ok(Option::<String>::deserialize(d)?.unwrap_or_default())
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Cite {
    /// 1-based page position within the attached PDF (not the printed page number); 0 when not stated
    pub page: u32,
    /// Under 30 words copied character for character from that page, containing the value; empty when the deed does not state it
    pub snippet: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WireCount {
    pub tenure: Tenure,
    pub units: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WireMixRow {
    /// e.g. "1 bed"
    pub unit_type: String,
    pub counts: Vec<WireCount>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WireMixTable {
    pub rows: Vec<WireMixRow>,
    pub subtotals: Vec<WireCount>,
    pub total_units: Option<u32>,
    pub total_habitable_rooms: Option<u32>,
    pub evidence: Cite,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WireTenure {
    pub tenure: Tenure,
    /// The deed's own name for this tenure, e.g. "Social Rented Housing Units"
    #[serde(default, deserialize_with = "null_as_empty")]
    pub label: String,
    pub units: Option<u32>,
    pub habitable_rooms: Option<u32>,
    pub evidence: Cite,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WireHousing {
    pub total_units: Option<u32>,
    pub total_units_evidence: Cite,
    pub market_units: Option<u32>,
    pub market_units_evidence: Cite,
    pub affordable_units: Option<u32>,
    pub affordable_units_evidence: Cite,
    pub affordable_habitable_rooms: Option<u32>,
    pub total_habitable_rooms: Option<u32>,
    pub tenures: Vec<WireTenure>,
    /// Empty rows when the deed has no mix table
    pub mix_table: WireMixTable,
    /// Percentage exactly as printed, digits only, e.g. "35.75"; empty if none
    #[serde(deserialize_with = "null_as_empty")]
    pub claimed_affordable_pct: String,
    pub claimed_affordable_basis: Option<AffordableBasis>,
    pub claimed_affordable_evidence: Cite,
}

// ------------------------------------------------------------------------------- v2 wire

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WireInstrument {
    pub kind: InstrumentKind,
    /// Where the deed says what kind of instrument it is (title or recital)
    pub evidence: Cite,
    /// For a variation or supplemental deed: the instrument it varies, as the deed describes it; empty otherwise
    pub parent_description: String,
    /// The parent's date as written, e.g. "6 February 2020"; empty if not stated
    pub parent_date: String,
    pub parent_evidence: Cite,
    /// Partial scope as written, e.g. "in so far as it relates to the Affordable Housing Land"; empty if the whole site
    pub scope: String,
    pub scope_evidence: Cite,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WireMetadata {
    /// The local planning authority, e.g. "London Borough of Southwark"
    pub council: String,
    pub planning_references: Vec<String>,
    /// The council's legal/file reference, if printed; empty otherwise
    pub legal_reference: String,
    /// Date of the deed as written; empty if blank in the deed
    pub agreement_date: String,
    pub site_address: String,
    pub parties: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WireInstalment {
    /// Share as printed, digits only, e.g. "50"; empty if the instalment isn't a percentage
    pub share_pct: String,
    /// The phrase saying when this instalment is due, e.g. "prior to Occupation of the 1st Dwelling"
    pub trigger_phrase: Cite,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WireContribution {
    /// Defined term exactly as written, without quotes
    pub name: String,
    pub amount_structure: AmountStructure,
    /// Figure as printed, digits and decimal point only, e.g. "12650" or "10058.60"; empty if none
    pub amount_gbp: String,
    /// Words form inside the brackets, exactly as written; empty if none
    pub amount_in_words: String,
    pub index_linked: bool,
    /// The definition stating the amount (or saying there is none)
    pub amount_evidence: Cite,
    pub conditionality: ConditionalityKind,
    /// For a conditional sum: the words of the deed that state the condition, copied exactly (this is the condition)
    pub condition_evidence: Cite,
    /// The covenant that makes it payable (or the lead-in of an instalment list), verbatim; empty if not found
    pub covenant: String,
    pub covenant_evidence: Cite,
    /// The trigger phrase within the covenant, e.g. "on or before Implementation"; empty when instalments are given
    pub trigger_phrase: Cite,
    /// One entry per instalment when the sum is paid in parts; empty otherwise
    pub instalments: Vec<WireInstalment>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WireVariation {
    pub op: VariationOp,
    /// What is varied, as the deed names it, e.g. definition "Affordable Housing Units", or Schedule 1 paragraph 4.3
    pub target: String,
    /// The new wording or value, copied from the deed; empty for a deletion
    pub new_text: String,
    pub evidence: Cite,
}

// One schema for everything compiles to an output grammar the API rejects as too large
// (measured with examples/schema_probe.rs), so extraction is three calls over the same PDF.

/// First call: what the instrument is, and what it varies.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WireInstrumentPart {
    pub instrument: WireInstrument,
    pub metadata: WireMetadata,
    /// For a deed of variation or supplemental deed: every change it makes; empty otherwise
    pub variations: Vec<WireVariation>,
}

/// Second call: housing.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WireHousingPart {
    pub housing: WireHousing,
}

/// Third call: the financial contributions.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WireContributionsPart {
    pub contributions: Vec<WireContribution>,
}

/// The three calls' answers combined; this is what fixtures store.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WireExtraction {
    pub instrument: WireInstrument,
    pub metadata: WireMetadata,
    pub housing: WireHousing,
    pub contributions: Vec<WireContribution>,
    /// For a deed of variation or supplemental deed: every change it makes; empty otherwise
    pub variations: Vec<WireVariation>,
}

// ------------------------------------------------------------------------------- v1 wire (replay only)

/// The v1 contribution shape recorded by the former Python harness. Kept so v1 fixtures replay.
#[derive(Debug, Clone, Deserialize)]
pub struct WireContributionV1 {
    pub name: String,
    pub amount_gbp: Option<String>,
    pub amount_in_words: Option<String>,
    pub index_linked: bool,
    pub is_cap: bool,
    pub contingent: bool,
    pub trigger_event: Option<TriggerEvent>,
    pub trigger_clause: Option<String>,
    pub trigger_evidence: Cite,
    pub evidence: Cite,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WireExtractionV1 {
    pub housing: WireHousing,
    pub contributions: Vec<WireContributionV1>,
}

// ------------------------------------------------------------------------------- conversion

/// Attached-PDF position (1-based) -> PDF page index.
pub struct PageMap(pub Vec<usize>);

impl PageMap {
    pub fn identity(pages: usize) -> PageMap {
        PageMap((1..=pages).collect())
    }

    /// An empty snippet is the model's "not stated"; a page outside the attachment maps to 0,
    /// which the verifier can never re-locate.
    pub fn ev(&self, c: &Cite) -> Option<Evidence> {
        if c.snippet.trim().is_empty() {
            return None;
        }
        let page = if c.page >= 1 && (c.page as usize) <= self.0.len() {
            self.0[c.page as usize - 1]
        } else {
            0
        };
        Some(Evidence {
            page,
            snippet: c.snippet.clone(),
        })
    }
}

/// Keep what the extractor wrote (verbatim, untrimmed) with its parse outcome. `None` is a blank field.
pub fn observe(
    raw: &str,
    parse: fn(&str) -> Result<Decimal, ParseError>,
) -> Option<Observed<Decimal>> {
    if raw.trim().is_empty() {
        return None;
    }
    Some(Observed {
        raw: raw.to_string(),
        parsed: parse(raw),
    })
}

fn nonempty(s: &str) -> Option<String> {
    let t = s.trim();
    if t.is_empty() {
        None
    } else {
        Some(t.to_string())
    }
}

/// Plural defined terms ("Site and Development Contributions") only group other contributions
/// for a shared trigger; some runs list them as obligations of their own.
pub fn is_group_term(name: &str) -> bool {
    name.trim_end().to_lowercase().ends_with("contributions")
}

pub fn housing(w: &WireHousing, pm: &PageMap) -> Housing {
    let t = &w.mix_table;
    let mix = (!t.rows.is_empty() || !t.subtotals.is_empty()).then(|| MixTable {
        rows: t
            .rows
            .iter()
            .map(|r| MixRow {
                unit_type: r.unit_type.clone(),
                units_by_tenure: r.counts.iter().map(|c| (c.tenure, c.units)).collect(),
            })
            .collect(),
        subtotals: t.subtotals.iter().map(|c| (c.tenure, c.units)).collect(),
        total_units: t.total_units,
        total_habitable_rooms: t.total_habitable_rooms,
        evidence: pm.ev(&t.evidence),
    });
    Housing {
        total_units: w.total_units,
        total_units_evidence: pm.ev(&w.total_units_evidence),
        market_units: w.market_units,
        market_units_evidence: pm.ev(&w.market_units_evidence),
        affordable_units: w.affordable_units,
        affordable_units_evidence: pm.ev(&w.affordable_units_evidence),
        affordable_habitable_rooms: w.affordable_habitable_rooms,
        total_habitable_rooms: w.total_habitable_rooms,
        tenures: w
            .tenures
            .iter()
            .map(|t| TenureAllocation {
                tenure: t.tenure,
                label: nonempty(&t.label),
                units: t.units,
                habitable_rooms: t.habitable_rooms,
                evidence: pm.ev(&t.evidence),
            })
            .collect(),
        mix_table: mix,
        claimed_affordable_pct: observe(&w.claimed_affordable_pct, quantity::decimal),
        claimed_affordable_basis: w.claimed_affordable_basis,
        claimed_affordable_evidence: pm.ev(&w.claimed_affordable_evidence),
    }
}

pub fn contribution(w: &WireContribution, pm: &PageMap) -> Contribution {
    let value = observe(&w.amount_gbp, quantity::decimal);
    let phrase = pm.ev(&w.trigger_phrase);
    let covenant = nonempty(&w.covenant).map(|text| Quoted {
        text,
        evidence: pm.ev(&w.covenant_evidence),
    });
    // the label is decided by the cited phrase only (the model isn't asked for one)
    let event = classify_cited(
        phrase.as_ref().map(|e| e.snippet.as_str()),
        covenant.as_ref().map(|q| q.text.as_str()),
    );
    let instalments: Vec<Instalment> = w
        .instalments
        .iter()
        .map(|i| {
            let p = pm.ev(&i.trigger_phrase);
            Instalment {
                share_pct: observe(&i.share_pct, quantity::decimal),
                trigger_event: crate::triggers::classify(p.as_ref().map(|e| e.snippet.as_str())),
                trigger_phrase: p,
            }
        })
        .collect();
    Contribution {
        name: w.name.clone(),
        amount: Amount {
            // "none" with a figure is a contradiction in the model's answer: keep the figure, call it fixed
            structure: if value.is_some() && w.amount_structure == AmountStructure::None {
                AmountStructure::Fixed
            } else {
                w.amount_structure
            },
            value_gbp: value,
            words: observe(&w.amount_in_words, quantity::amount),
            index_linked: w.index_linked,
            evidence: pm.ev(&w.amount_evidence),
        },
        conditionality: Conditionality {
            kind: w.conditionality,
            // the condition is the deed's own wording, so it is grounded by construction once its snippet re-locates
            condition: pm.ev(&w.condition_evidence).map(|e| e.snippet),
            evidence: pm.ev(&w.condition_evidence),
        },
        payment: Payment {
            covenant,
            trigger_event: if instalments.is_empty() { event } else { None },
            trigger_event_model: None,
            trigger_phrase: if instalments.is_empty() { phrase } else { None },
            instalments,
        },
    }
}

/// v1 (Python harness) contribution -> v2. `contingent` meant "no fixed figure", so it maps to
/// structure `none` with unknown conditionality; `is_cap` to structure `cap`.
pub fn contribution_v1(w: &WireContributionV1, pm: &PageMap) -> Contribution {
    let value = w
        .amount_gbp
        .as_deref()
        .and_then(|s| observe(s, quantity::decimal));
    let phrase = pm.ev(&w.trigger_evidence);
    let covenant = w
        .trigger_clause
        .as_deref()
        .and_then(nonempty)
        .map(|text| Quoted {
            text,
            evidence: phrase.clone(),
        });
    let event = classify_cited(
        phrase.as_ref().map(|e| e.snippet.as_str()),
        covenant.as_ref().map(|q| q.text.as_str()),
    )
    .or(w.trigger_event);
    let structure = match (value.is_some(), w.is_cap) {
        (true, true) => AmountStructure::Cap,
        (true, false) => AmountStructure::Fixed,
        (false, _) => AmountStructure::None,
    };
    Contribution {
        name: w.name.clone(),
        amount: Amount {
            structure,
            value_gbp: value,
            words: w
                .amount_in_words
                .as_deref()
                .and_then(|s| observe(s, quantity::amount)),
            index_linked: w.index_linked,
            evidence: pm.ev(&w.evidence),
        },
        conditionality: Conditionality {
            // v1 "contingent" meant "no fixed figure", not "conditional" (docs/schema-v2.md)
            kind: ConditionalityKind::Unknown,
            condition: None,
            evidence: None,
        },
        payment: Payment {
            covenant,
            trigger_event: event,
            trigger_event_model: if w.trigger_event != event {
                w.trigger_event
            } else {
                None
            },
            trigger_phrase: phrase,
            instalments: vec![],
        },
    }
}

pub fn variation(w: &WireVariation, pm: &PageMap) -> Variation {
    Variation {
        op: w.op,
        target: w.target.clone(),
        new_text: nonempty(&w.new_text),
        evidence: pm.ev(&w.evidence),
    }
}

pub fn instrument(w: &WireInstrument, pm: &PageMap) -> Instrument {
    Instrument {
        kind: w.kind,
        evidence: pm.ev(&w.evidence),
        parent: nonempty(&w.parent_description).map(|description| ParentRef {
            description,
            date: nonempty(&w.parent_date),
            evidence: pm.ev(&w.parent_evidence),
        }),
        scope: nonempty(&w.scope).map(|text| Quoted {
            text,
            evidence: pm.ev(&w.scope_evidence),
        }),
    }
}

pub fn metadata(w: &WireMetadata) -> Metadata {
    Metadata {
        council: nonempty(&w.council),
        planning_references: w
            .planning_references
            .iter()
            .filter_map(|s| nonempty(s))
            .collect(),
        legal_reference: nonempty(&w.legal_reference),
        agreement_date: nonempty(&w.agreement_date),
        site_address: nonempty(&w.site_address),
        parties: w.parties.iter().filter_map(|s| nonempty(s)).collect(),
    }
}
