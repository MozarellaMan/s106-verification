//! Typed audit findings: one value per numeric problem, built in one place.
//!
//! A check that reads a field reports that field's own failure. The finding carries what the
//! extractor wrote, where in it the parse failed, and whether the cited evidence still
//! re-locates, so the human report and the JSON say the same thing.

use std::fmt::Write;
use std::ops::Range;
use std::sync::LazyLock;

use serde::ser::{Serialize, SerializeStruct, Serializer};
use sha2::{Digest, Sha256};

use crate::evidence::{Located, Pages, Repair, locate};
use crate::quantity::{self, Category, Cause, ParseError};
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
    /// SHA-256 of the extraction output this finding's field came from; set by `verify`.
    pub extraction_sha256: Option<String>,
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
            extraction_sha256: None,
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

// ------------------------------------------------------------------------------- provenance

/// Names the number-reading policy; bump it when the contract (not just the code) changes.
pub const PARSER_POLICY: &str = "s106-quantity/1";

/// Which extraction produced which parts of an audit, and what the parser accepted at the time.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RunIdentity {
    pub parser_policy: String,
    /// Behavioural: a hash of what the parser does on the probe corpus, not of its source.
    pub parser_fingerprint: String,
    pub crate_version: String,
    pub cargo_lock_sha256: String,
    /// Set by whoever builds (`S106_SOFTWARE_REVISION`); unknown otherwise, never inferred.
    pub software_revision: Option<String>,
    /// The fixture that supplied (or recorded) the extraction; none for an unrecorded live response.
    pub fixture_reference: Option<String>,
    pub fixture_sha256: Option<String>,
    pub extraction_sha256: String,
    pub page_text_sha256: String,
    #[serde(default)]
    pub replacements: Vec<ReplacementIdentity>,
}

/// A field group adopted from an escalation extraction.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ReplacementIdentity {
    /// "housing", "instrument" or a contribution name.
    pub field_group: String,
    pub extraction_sha256: String,
    pub fixture_reference: Option<String>,
    pub fixture_sha256: Option<String>,
}

impl RunIdentity {
    pub fn new(
        output: &serde_json::Value,
        pages: &Pages,
        fixture: Option<&std::path::Path>,
    ) -> RunIdentity {
        RunIdentity {
            parser_policy: PARSER_POLICY.into(),
            parser_fingerprint: parser_fingerprint().to_string(),
            crate_version: env!("CARGO_PKG_VERSION").into(),
            cargo_lock_sha256: sha256_hex(include_str!("../Cargo.lock")),
            software_revision: option_env!("S106_SOFTWARE_REVISION").map(str::to_string),
            fixture_reference: fixture.map(|p| p.display().to_string()),
            fixture_sha256: fixture
                .and_then(|p| std::fs::read_to_string(p).ok())
                .map(|s| sha256_hex(&s)),
            extraction_sha256: sha256_hex(&serde_json::to_string(output).expect("serializable")),
            page_text_sha256: sha256_hex(&serde_json::to_string(pages).expect("serializable")),
            replacements: vec![],
        }
    }

    /// The extraction a field group's values came from: its latest replacement, else the base.
    pub fn extraction_for(&self, field_group: Option<&str>) -> &str {
        field_group
            .and_then(|g| self.replacements.iter().rev().find(|r| r.field_group == g))
            .map_or(&self.extraction_sha256, |r| &r.extraction_sha256)
    }
}

const PROBES: &str = include_str!("../fixtures/numeric-probes.txt");

/// Probes are one per line with backslash, newline and carriage return escaped.
pub fn unescape_probe(line: &str) -> String {
    let mut out = String::new();
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some(o) => out.push(o),
            None => out.push('\\'),
        }
    }
    out
}

pub fn escape_probe(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
}

/// What the number parser does on every probe, one canonical block per probe: each quantity with
/// unit and span, each error with cause and spans, then the whole-input readers. This is what the
/// fingerprint hashes and what `tests/numeric_snapshot.rs` snapshots.
pub fn probe_rendering() -> String {
    let mut out = String::new();
    for line in PROBES.lines().filter(|l| !l.is_empty()) {
        let probe = unescape_probe(line);
        let _ = writeln!(out, "probe {line}");
        let scan = quantity::scan(&probe);
        for q in &scan.quantities {
            let _ = writeln!(out, "  quantity {} {:?} {:?}", q.value, q.unit, q.span);
        }
        for e in &scan.errors {
            let _ = writeln!(out, "  error {}", render_error(e));
        }
        match quantity::decimal(&probe) {
            Ok(v) => {
                let _ = writeln!(out, "  decimal ok {v}");
            }
            Err(e) => {
                let _ = writeln!(out, "  decimal err {}", render_error(&e));
            }
        }
        match quantity::amount(&probe) {
            Ok(v) => {
                let _ = writeln!(out, "  amount ok {v}");
            }
            Err(e) => {
                let _ = writeln!(out, "  amount err {}", render_error(&e));
            }
        }
    }
    out
}

fn render_error(e: &ParseError) -> String {
    format!(
        "{} {:?} candidate {:?} span {:?} expected {:?}",
        e.category().code(),
        e.cause,
        e.candidate,
        e.span,
        e.expected
    )
}

/// SHA-256 of `probe_rendering()`. A comment edit leaves it unchanged; any change in what the
/// parser accepts, rejects or measures on the probe corpus changes it.
pub fn parser_fingerprint() -> &'static str {
    static FP: LazyLock<String> = LazyLock::new(|| sha256_hex(&probe_rendering()));
    &FP
}
