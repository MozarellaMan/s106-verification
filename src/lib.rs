//! s106: research tools for UK Section 106 planning agreements.
//!
//! `document`: page text from a PDF's text layer or OCR, with boxes, cached by content hash.
//! `pld`: Planning London Datahub snapshots. `explore`: the results explorer's data.
//! `labels`: human labels, scored against the agents' proposals; nothing else depends on it.
//! `results`: a batch's scheme ledger, decision rules and claims, each with its check level.
//! `figures`: checks that figures cited in markdown still appear in the files they cite.

pub mod data;
pub mod document;
pub mod explore;
pub mod figures;
pub mod labels;
pub mod ocrbench;
pub mod pld;
pub mod results;
