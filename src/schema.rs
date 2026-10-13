//! Schema v2: what an S106 instrument says, with evidence for every extracted value.
//!
//! Design notes and the real deeds behind each choice: docs/schema-v2.md.
//! Every number carries an `Evidence` (page + verbatim snippet) so the verifier can re-locate it
//! in the page text and decide whether a failed check is the extractor's fault or the deed's.

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

pub const SCHEMA_VERSION: &str = "2";

/// A verbatim quotation from a page. `page` is the 1-based PDF page index, not the printed number.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Evidence {
    pub page: usize,
    pub snippet: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Agreement {
    pub schema_version: String,
    pub provenance: Provenance,
    pub instrument: Instrument,
    pub metadata: Metadata,
    pub housing: Housing,
    #[serde(default)]
    pub contributions: Vec<Contribution>,
    #[serde(default)]
    pub variations: Vec<Variation>,
    #[serde(default)]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Provenance {
    pub source_path: String,
    pub sha256: String,
    pub page_count: usize,
    /// "text-layer", "ocr" or "mixed"
    pub text_source: String,
    pub ocr_version: Option<String>,
    /// e.g. "claude-haiku-5-5 (replay of fixtures/…)"
    pub extracted_by: String,
}

// ------------------------------------------------------------------------------- instrument

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum InstrumentKind {
    Agreement,
    UnilateralUndertaking,
    DeedOfVariation,
    Supplemental,
    Unknown,
}

impl InstrumentKind {
    /// Variations and supplemental deeds restate only what changes, so whole-scheme checks don't apply.
    pub fn is_variation(self) -> bool {
        matches!(
            self,
            InstrumentKind::DeedOfVariation | InstrumentKind::Supplemental
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Instrument {
    pub kind: InstrumentKind,
    pub evidence: Option<Evidence>,
    /// The instrument this one varies or supplements, as the deed identifies it.
    pub parent: Option<ParentRef>,
    /// Partial scope, e.g. "in so far as it relates to the Affordable Housing Land".
    pub scope: Option<Quoted>,
}

impl Default for Instrument {
    fn default() -> Self {
        Instrument {
            kind: InstrumentKind::Unknown,
            evidence: None,
            parent: None,
            scope: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParentRef {
    pub description: String,
    pub date: Option<String>,
    pub evidence: Option<Evidence>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Quoted {
    pub text: String,
    pub evidence: Option<Evidence>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Metadata {
    pub council: Option<String>,
    #[serde(default)]
    pub planning_references: Vec<String>,
    pub legal_reference: Option<String>,
    pub agreement_date: Option<String>,
    pub site_address: Option<String>,
    #[serde(default)]
    pub parties: Vec<String>,
}

// ------------------------------------------------------------------------------- housing

#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    PartialOrd,
    Ord,
    Serialize,
    Deserialize,
    schemars::JsonSchema,
)]
pub enum Tenure {
    #[serde(rename = "Social Rent")]
    SocialRent,
    #[serde(rename = "London Affordable Rent")]
    LondonAffordableRent,
    #[serde(rename = "Affordable Rent")]
    AffordableRent,
    #[serde(rename = "London Living Rent")]
    LondonLivingRent,
    #[serde(rename = "Shared Ownership")]
    SharedOwnership,
    #[serde(rename = "London Shared Ownership")]
    LondonSharedOwnership,
    #[serde(rename = "Intermediate")]
    Intermediate,
    #[serde(rename = "Discount Market Rent")]
    DiscountMarketRent,
    #[serde(rename = "Discount Market Sale")]
    DiscountMarketSale,
    #[serde(rename = "First Homes")]
    FirstHomes,
    #[serde(rename = "Other")]
    Other,
}

impl Tenure {
    pub fn label(self) -> &'static str {
        match self {
            Tenure::SocialRent => "Social Rent",
            Tenure::LondonAffordableRent => "London Affordable Rent",
            Tenure::AffordableRent => "Affordable Rent",
            Tenure::LondonLivingRent => "London Living Rent",
            Tenure::SharedOwnership => "Shared Ownership",
            Tenure::LondonSharedOwnership => "London Shared Ownership",
            Tenure::Intermediate => "Intermediate",
            Tenure::DiscountMarketRent => "Discount Market Rent",
            Tenure::DiscountMarketSale => "Discount Market Sale",
            Tenure::FirstHomes => "First Homes",
            Tenure::Other => "Other",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AffordableBasis {
    HabitableRooms,
    Units,
    Floorspace,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TenureAllocation {
    pub tenure: Tenure,
    /// The deed's own name for it, e.g. "Social Rented Housing Units".
    pub label: Option<String>,
    pub units: Option<u32>,
    pub habitable_rooms: Option<u32>,
    pub evidence: Option<Evidence>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MixRow {
    pub unit_type: String,
    pub units_by_tenure: Vec<(Tenure, u32)>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MixTable {
    #[serde(default)]
    pub rows: Vec<MixRow>,
    #[serde(default)]
    pub subtotals: Vec<(Tenure, u32)>,
    pub total_units: Option<u32>,
    pub total_habitable_rooms: Option<u32>,
    pub evidence: Option<Evidence>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Housing {
    pub total_units: Option<u32>,
    pub total_units_evidence: Option<Evidence>,
    pub market_units: Option<u32>,
    pub market_units_evidence: Option<Evidence>,
    pub affordable_units: Option<u32>,
    pub affordable_units_evidence: Option<Evidence>,
    pub affordable_habitable_rooms: Option<u32>,
    pub total_habitable_rooms: Option<u32>,
    #[serde(default)]
    pub tenures: Vec<TenureAllocation>,
    pub mix_table: Option<MixTable>,
    pub claimed_affordable_pct: Option<Decimal>,
    pub claimed_affordable_basis: Option<AffordableBasis>,
    pub claimed_affordable_evidence: Option<Evidence>,
}

// ------------------------------------------------------------------------------- contributions

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AmountStructure {
    /// a stated sum
    Fixed,
    /// a stated maximum ("up to a maximum of", "or such lesser amount")
    Cap,
    /// computed by a formula in the deed
    Formula,
    /// a rate, e.g. "£366 per cubic metre"
    Rate,
    /// no figure stated
    None,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Amount {
    pub structure: AmountStructure,
    pub value_gbp: Option<Decimal>,
    /// The deed's words form, e.g. "ten thousand and fifty eight pounds and sixty pence".
    pub words: Option<String>,
    pub index_linked: bool,
    pub evidence: Option<Evidence>,
}

impl Amount {
    /// A contribution total, excluding rates and formula parameters.
    pub fn stated_sum_gbp(&self) -> Option<Decimal> {
        match self.structure {
            AmountStructure::Fixed | AmountStructure::Cap => self.value_gbp,
            AmountStructure::Formula | AmountStructure::Rate | AmountStructure::None => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ConditionalityKind {
    Unconditional,
    /// Payable only if something happens (a review finds a surplus, targets are missed, ...).
    Conditional,
    /// v1 data, or the deed doesn't make it clear.
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Conditionality {
    pub kind: ConditionalityKind,
    /// The condition as quoted from the deed.
    pub condition: Option<String>,
    pub evidence: Option<Evidence>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub enum TriggerEvent {
    #[serde(rename = "prior to Implementation")]
    PreImplementation,
    #[serde(rename = "prior to Commencement")]
    PreCommencement,
    #[serde(rename = "prior to Occupation")]
    PreOccupation,
    #[serde(rename = "on completion of agreement")]
    OnCompletion,
    #[serde(rename = "on demand / review")]
    OnDemand,
    #[serde(rename = "other")]
    Other,
}

impl TriggerEvent {
    pub fn label(self) -> &'static str {
        match self {
            TriggerEvent::PreImplementation => "prior to Implementation",
            TriggerEvent::PreCommencement => "prior to Commencement",
            TriggerEvent::PreOccupation => "prior to Occupation",
            TriggerEvent::OnCompletion => "on completion of agreement",
            TriggerEvent::OnDemand => "on demand / review",
            TriggerEvent::Other => "other",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Instalment {
    pub share_pct: Option<Decimal>,
    /// Decided by `triggers::classify` from `trigger_phrase`, not by the model.
    pub trigger_event: Option<TriggerEvent>,
    pub trigger_phrase: Option<Evidence>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Payment {
    /// The covenant that imposes payment (or the lead-in of an instalment list), verbatim.
    pub covenant: Option<Quoted>,
    /// Single trigger when there are no instalments. Decided from `trigger_phrase`.
    pub trigger_event: Option<TriggerEvent>,
    /// The model's own label, kept only when the cited phrase decides differently.
    pub trigger_event_model: Option<TriggerEvent>,
    pub trigger_phrase: Option<Evidence>,
    #[serde(default)]
    pub instalments: Vec<Instalment>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Contribution {
    pub name: String,
    pub amount: Amount,
    pub conditionality: Conditionality,
    pub payment: Payment,
}

// ------------------------------------------------------------------------------- variations

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum VariationOp {
    Amend,
    Replace,
    Insert,
    Delete,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Variation {
    pub op: VariationOp,
    /// What is varied, as the deed names it: `definition "Affordable Housing Units"`, `Schedule 1 paragraph 4.3`.
    pub target: String,
    pub new_text: Option<String>,
    pub evidence: Option<Evidence>,
}

impl Agreement {
    /// Sum of fixed or capped amounts, unindexed (a cap counts at its maximum).
    pub fn total_stated_gbp(&self) -> Decimal {
        self.contributions
            .iter()
            .filter_map(|c| c.amount.stated_sum_gbp())
            .sum()
    }
}
