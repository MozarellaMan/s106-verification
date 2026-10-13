//! Typed audit findings: one value per numeric problem, built in one place.
//!
//! A check that reads a field reports that field's own failure. The finding carries what the
//! extractor wrote, where in it the parse failed, and whether the cited evidence still
//! re-locates, so the human report and the JSON say the same thing.

use std::fmt::Write;
use std::ops::Range;

use serde::ser::{Serialize, SerializeStruct, Serializer};
use sha2::{Digest, Sha256};

use crate::evidence::{Pages, evidence_found};
use crate::quantity::{Category, Cause, ParseError};
use crate::schema::Evidence;

pub fn sha256_hex(s: &str) -> String {
    Sha256::digest(s.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// What a byte span is measured against. Always UTF-8 byte offsets, never PDF glyph positions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CoordinateSpace {
    ObservedInput,
    Excerpt,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Observation {
    pub input: String,
    pub sha256: String,
    pub span: Option<Range<usize>>,
    pub coordinates: CoordinateSpace,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct CitationRef {
    pub page: usize,
    pub excerpt: String,
    pub relocated: bool,
}

/// A category that serialises as `{"code": "NUM002", "category": "invalid"}`; the code is derived.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Coded(pub Category);

impl Serialize for Coded {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut st = s.serialize_struct("Coded", 2)?;
        st.serialize_field("code", self.0.code())?;
        st.serialize_field("category", &self.0)?;
        st.end()
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Finding {
    pub check_id: String,
    /// Path of the field in the agreement, e.g. "agreement/contributions/0/amount/value_gbp".
    pub field: String,
    #[serde(flatten)]
    pub category: Coded,
    pub cause: Cause,
    pub expected: String,
    pub observed: Option<Observation>,
    pub citation: Option<CitationRef>,
}

impl Finding {
    /// The only constructor. `observed` None means the required input was absent
    /// (`Category::Absent`, no observation, no invented span).
    pub fn new(
        check_id: &str,
        field: String,
        observed: Option<(&str, &ParseError)>,
        citation: Option<&Evidence>,
        pages: &Pages,
    ) -> Finding {
        let (category, cause, expected, observed) = match observed {
            Some((input, err)) => (
                err.category(),
                err.cause.clone(),
                err.expected.to_string(),
                Some(Observation {
                    input: input.to_string(),
                    sha256: sha256_hex(input),
                    span: Some(err.span.clone()),
                    coordinates: CoordinateSpace::ObservedInput,
                }),
            ),
            None => (
                Category::Absent,
                Cause::Empty,
                "a value stated in the deed".to_string(),
                None,
            ),
        };
        Finding {
            check_id: check_id.to_string(),
            field,
            category: Coded(category),
            cause,
            expected,
            observed,
            citation: citation.map(|ev| CitationRef {
                page: ev.page,
                excerpt: ev.snippet.clone(),
                relocated: evidence_found(Some(ev), pages),
            }),
        }
    }

    /// One or two lines for the human report.
    pub fn render(&self) -> String {
        let category = serde_json::to_value(self.category.0)
            .ok()
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_default();
        let mut o = format!(
            "{} {category}: {} ({:?}, expected {})",
            self.category.0.code(),
            self.field,
            self.cause,
            self.expected
        );
        match &self.observed {
            Some(ob) => {
                let _ = write!(o, "\n             input {:?}, ", ob.input);
                match &ob.span {
                    Some(sp) => {
                        let coords = match ob.coordinates {
                            CoordinateSpace::ObservedInput => "observed_input",
                            CoordinateSpace::Excerpt => "excerpt",
                        };
                        let _ = write!(o, "bytes {}..{} ({coords})", sp.start, sp.end);
                    }
                    None => o.push_str("no source span"),
                }
            }
            None => o.push_str("\n             no input, no source span"),
        }
        o
    }
}
