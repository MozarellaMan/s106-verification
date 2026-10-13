//! Typed audit findings: one value per numeric problem, built in one place.
//!
//! A check that reads a field reports that field's own failure. The finding carries what the
//! extractor wrote, where in it the parse failed, and whether the cited evidence still
//! re-locates, so the human report and the JSON say the same thing.

use std::fmt::Write;
use std::ops::Range;

use serde::ser::{Serialize, SerializeStruct, Serializer};
use sha2::{Digest, Sha256};

use crate::evidence::{Located, Pages, Repair, locate};
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
    /// Where the excerpt matched in the page text (byte offsets), when it relocated.
    pub range: Option<Range<usize>>,
    /// OCR digit substitutions the match relied on; non-empty means the page as printed differs.
    pub repairs: Vec<Repair>,
}

impl CitationRef {
    fn new(ev: &Evidence, located: Option<&Located>) -> CitationRef {
        CitationRef {
            page: ev.page,
            excerpt: ev.snippet.clone(),
            relocated: located.is_some(),
            range: located.map(|l| l.range.clone()),
            repairs: located.map(|l| l.repairs.clone()).unwrap_or_default(),
        }
    }
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
    /// A numeric field that could not be read. `observed` None means the required input was
    /// absent (`Category::Absent`, no observation, no invented span). The span is in the
    /// coordinates of the observed input.
    pub fn new(
        check_id: &str,
        field: String,
        observed: Option<(&str, &ParseError)>,
        citation: Option<&Evidence>,
        pages: &Pages,
    ) -> Finding {
        let located = citation.and_then(|ev| locate(ev, pages));
        let (cause, expected, observation) = match observed {
            Some((input, err)) => (
                err.cause.clone(),
                err.expected.to_string(),
                Some(Observation {
                    input: input.to_string(),
                    sha256: sha256_hex(input),
                    span: Some(err.span.clone()),
                    coordinates: CoordinateSpace::ObservedInput,
                }),
            ),
            None => (Cause::Empty, "a value stated in the deed".to_string(), None),
        };
        Finding::assemble(
            check_id,
            field,
            cause,
            expected,
            observation,
            citation.map(|ev| CitationRef::new(ev, located.as_ref())),
        )
    }

    /// A grounding problem: the cited excerpt does not support the value. `error` is a reason the
    /// excerpt's numbers were unreadable, with its span in excerpt coordinates; without one the
    /// finding is about the excerpt as a whole (`cause`, `expected`, no span). `located` is the
    /// citation's single relocation, computed by the caller.
    pub fn in_excerpt(
        check_id: &str,
        field: &str,
        reason: Result<(Cause, String), &ParseError>,
        citation: &Evidence,
        located: Option<&Located>,
    ) -> Finding {
        let (cause, expected, span) = match reason {
            Ok((cause, expected)) => (cause, expected, None),
            Err(err) => (
                err.cause.clone(),
                err.expected.to_string(),
                Some(err.span.clone()),
            ),
        };
        Finding::assemble(
            check_id,
            field.to_string(),
            cause,
            expected,
            Some(Observation {
                input: citation.snippet.clone(),
                sha256: sha256_hex(&citation.snippet),
                span,
                coordinates: CoordinateSpace::Excerpt,
            }),
            Some(CitationRef::new(citation, located)),
        )
    }

    fn assemble(
        check_id: &str,
        field: String,
        cause: Cause,
        expected: String,
        observed: Option<Observation>,
        citation: Option<CitationRef>,
    ) -> Finding {
        Finding {
            check_id: check_id.to_string(),
            field,
            category: Coded(cause.category()),
            cause,
            expected,
            observed,
            citation,
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
