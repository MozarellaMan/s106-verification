//! s106: verification harness for UK Section 106 planning agreements.
//!
//! Pipeline: `document` (PDF text layer or OCR, with boxes, cached by content hash)
//! → `extract` (Claude reads the PDF into `wire` types, converted to `schema` v2)
//! → `verify` (deterministic, evidence-grounded checks) → `report` / JSON.
//! Layer B: `pld` (Planning London Datahub snapshots) and `reconcile` (obligations vs delivery).
//! `labels`: human labels, scored against the pipeline's readings; nothing else depends on it.

pub mod audit;
pub mod data;
pub mod document;
pub mod eval;
pub mod evidence;
pub mod extract;
pub mod labels;
pub mod llm;
pub mod ocrbench;
pub mod pld;
pub mod quantity;
pub mod reconcile;
pub mod report;
pub mod schema;
pub mod text;
pub mod triggers;
pub mod verify;
pub mod wire;
