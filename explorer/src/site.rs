//! The explorer's data file, `sites.json`: written by `s106 explore` and read by the site. Both
//! crates include this one file, so the two sides can't drift apart. Serde only, no other imports.

use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
pub struct Bundle {
    pub sources: Sources,
    pub sites: Vec<Site>,
}

/// Where the figures came from and what was left out, shown in the site's footer.
#[derive(Clone, Serialize, Deserialize)]
pub struct Sources {
    /// paths in the data repository
    pub applications: String,
    pub snapshot: String,
    pub boundaries: String,
    /// the snapshot's date in words, e.g. "14 October 2026"
    pub snapshot_date: String,
    /// records in the table, and how each one left out was left out
    pub records: u64,
    pub not_permitted: u64,
    pub no_decision_year: u64,
    pub wholly_superseded: u64,
    pub without_centroid: u64,
    /// Private builds only: the results folder whose ledger readings were added, and its schemes
    /// that matched no site on the map. A public build never carries these.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub results: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unmatched: Vec<String>,
    /// Public builds with deed figures: the results folder whose deed cells were added, each with
    /// its check level. Only deed cells for the obligation, variation and completion stages go in;
    /// no other source, no retrieval.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deed_cells: Option<String>,
    /// the private data repository's commit a private build was made from; never in a public build
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_commit: Option<String>,
}

/// One permission, as on its current Datahub record. Every count is of homes no later permission
/// replaced.
#[derive(Clone, Serialize, Deserialize)]
pub struct Site {
    pub borough: String,
    #[serde(rename = "ref")]
    pub reference: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub site: Option<String>,
    pub status: Status,
    /// year of the decision (or of the appeal decision, for refusals allowed on appeal)
    pub year: i32,
    pub homes: u32,
    /// of which affordable (low-cost rent plus intermediate)
    pub affordable: u32,
    /// homes with a recorded tenure; affordable shares are of these
    pub known: u32,
    /// homes with an actual completion date
    pub completed: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completion_date: Option<String>,
    /// when the borough last changed the record (YYYY-MM-DD)
    pub updated: String,
    pub lat: f64,
    pub lon: f64,
    /// Private builds only: this scheme's cells from a results ledger.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub readings: Vec<Reading>,
    /// Private builds only: the documents of each kind listed for this scheme, and those not in hand.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub retrieval: Vec<Retrieval>,
}

/// One row of a results folder's `retrieval.csv`.
#[derive(Clone, Serialize, Deserialize)]
pub struct Retrieval {
    /// committee_report, deed, viability, ...
    pub source: String,
    pub documents: u32,
    pub in_hand: u32,
    /// e.g. "listed_not_fetched x13"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub not_in_hand: Option<String>,
}

/// One cell of a results ledger (`results/<batch>/ledger.csv`): what one source says about one
/// variable, and how far it has been checked.
#[derive(Clone, Serialize, Deserialize)]
pub struct Reading {
    pub stage: Lineage,
    /// e.g. "V1", and what it is ("Total homes; affordable homes by tenure")
    pub variable: String,
    pub name: String,
    /// committee_report, deed, datahub, epc, gla, ...
    pub source: String,
    pub status: Finding,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub basis: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pages: Option<String>,
    pub check: Check,
}

/// What a source says about a variable (a ledger's `status`; blank there means unread).
#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Finding {
    Found,
    Ambiguous,
    Inconsistent,
    NotStated,
    SourceMissing,
    NotApplicable,
    Unread,
}

impl Finding {
    pub fn from_ledger(s: &str) -> Option<Finding> {
        Some(match s {
            "found" => Finding::Found,
            "ambiguous" => Finding::Ambiguous,
            "inconsistent" => Finding::Inconsistent,
            "not_stated" => Finding::NotStated,
            "source_missing" => Finding::SourceMissing,
            "not_applicable" => Finding::NotApplicable,
            "" => Finding::Unread,
            _ => return None,
        })
    }
}

/// How far a cell has been checked, weakest first: only `Anchored` and `Independent` are checked
/// by a person.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Check {
    /// nobody has read it
    Unread,
    /// an agent's reading only, which is not a result
    Proposed,
    /// labellers disagree
    Unsettled,
    /// a person confirmed or corrected an agent's reading
    Anchored,
    /// a person read the source
    Independent,
}

/// The lineage stages a ledger uses.
#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Lineage {
    Offer,
    Decision,
    Obligation,
    Variation,
    Completion,
}

/// The Datahub's status, folded case-insensitively into the few the map tells apart.
#[derive(Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Completed,
    Commenced,
    /// approved or allowed on appeal, with no start recorded
    Approved,
    Lapsed,
    Superseded,
    /// blank, appeal in progress, unknown and the rest
    Other,
}

impl Status {
    pub fn from_datahub(s: &str) -> Status {
        match s.trim().to_lowercase().as_str() {
            "completed" => Status::Completed,
            "commenced" => Status::Commenced,
            "approved" | "allowed" => Status::Approved,
            "lapsed" => Status::Lapsed,
            "superseded" => Status::Superseded,
            _ => Status::Other,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Status::Completed => "Completed",
            Status::Commenced => "Commenced",
            Status::Approved => "Approved, no start recorded",
            Status::Lapsed => "Lapsed",
            Status::Superseded => "Superseded",
            Status::Other => "Other or blank",
        }
    }
}
