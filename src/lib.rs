//! s106: verification harness for UK Section 106 planning agreements.
//!
//! Pipeline: `document` (PDF text layer or OCR, with boxes, cached by content hash)
//! → `extract` (Claude reads the PDF into `wire` types, converted to `schema` v2)
//! → `verify` (deterministic, evidence-grounded checks) → `report` / JSON.

pub mod document;
pub mod eval;
pub mod evidence;
pub mod extract;
pub mod llm;
pub mod ocrbench;
pub mod report;
pub mod schema;
pub mod text;
pub mod triggers;
pub mod verify;
pub mod wire;
