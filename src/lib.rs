//! s106: research tools for UK Section 106 planning agreements.
//!
//! `document`: page text from a PDF's text layer or OCR, with boxes, cached by content hash.
//! `pld`: Planning London Datahub snapshots; `datahub` maps them into readings. `explore`: the results explorer's data.
//! `labels`: human labels, scored against the agents' proposals; only `accuracy` and `results` use them.
//! `results`: a batch's scheme ledger, decision rules and claims, each with its check level;
//! `accuracy` estimates each reader's error rate from how often readers agree.
//! `figures`: checks that figures cited in markdown still appear in the files they cite.

pub mod accuracy;
pub mod data;
pub mod datahub;
pub mod document;
pub mod explore;
pub mod figures;
pub mod labels;
pub mod ocrbench;
pub mod pld;
pub mod results;
