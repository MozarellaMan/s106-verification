//! s106: verification harness for UK Section 106 planning agreements.
//!
//! Pipeline: `document` (PDF text layer or OCR, with boxes, cached by content hash)
//! → `extract` (Claude reads the PDF into `wire` types, converted to `schema` v2)
//! → `verify` (deterministic, evidence-grounded checks) → `report` / JSON.
//! Layer B: `pld` (Planning London Datahub snapshots) and `reconcile` (obligations vs delivery).
//! `labels`: human labels, scored against the pipeline's readings; nothing else depends on it.
//! `results`: a batch's scheme ledger, decision rules and claims, each with its check level.
//! `figures`: checks that figures cited in markdown still appear in the files they cite.

pub mod audit;
pub mod data;
pub mod document;
pub mod eval;
pub mod evidence;
pub mod explore;
pub mod extract;
pub mod figures;
pub mod labels;
pub mod llm;
pub mod ocrbench;
pub mod pld;
pub mod quantity;
pub mod reconcile;
pub mod report;
pub mod results;
pub mod schema;
pub mod text;
pub mod triggers;
pub mod verify;
pub mod wire;
