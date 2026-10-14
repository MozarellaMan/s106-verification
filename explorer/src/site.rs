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
}

/// One cell of a results ledger (`results/<batch>/ledger.csv`): what one source says about one
/// variable, and how far it has been checked.
#[derive(Clone, Serialize, Deserialize)]
pub struct Reading {
    pub stage: Stage,
    /// e.g. "V1", and what it is ("Total homes; affordable homes by tenure")
    pub variable: String,
    pub name: String,
    /// committee_report, deed, datahub, epc, gla, ...
    pub source: String,
    /// found, not_stated, source_missing, not_applicable, ambiguous, inconsistent; blank if unread
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub basis: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pages: Option<String>,
    /// independent, anchored, proposed, unsettled or unread
    pub check: String,
}

/// The lineage stages a ledger uses.
#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Stage {
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
