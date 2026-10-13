//! Programmatic gatekeeper for extracted S106 data.
//!
//! Checks combine deterministic arithmetic and evidence rules. Nothing here is a model judgement.
//! Each check's inputs are also *grounded*: a value counts as supported only when it appears in
//! its cited evidence snippet (digits, or number words) and that snippet re-locates on the cited
//! page of the page text. Outcomes:
//!
//! * PASS: the assertion holds and every input is grounded
//! * REVIEW: the assertion holds, but some input isn't grounded, so its correctness is unproven
//! * FAIL + ParserError: an input is missing or ungrounded, and the assertion fails
//! * FAIL + DocumentDiscrepancy: the assertion fails although every input is grounded. This is
//!   what the deed *appears* to say through OCR; confirm against the original before relying on it.
//! * SKIP: the deed doesn't state what the check needs

use std::collections::{BTreeMap, BTreeSet};
use std::sync::LazyLock;

use regex::Regex;
use rust_decimal::prelude::*;
use serde::Serialize;

use crate::evidence::{Pages, evidence_found};
use crate::schema::{
    AffordableBasis, Agreement, AmountStructure, ConditionalityKind, Contribution, Evidence,
    Instalment, Tenure, Variation,
};
use crate::text::{Value, ocr_digits, percentage_in_snippet, squash, value_in_snippet};
use crate::triggers::{classify, classify_cited, imposes_payment};

/// Percentage points; deeds quote 2 dp.
const PCT_TOLERANCE: Decimal = Decimal::from_parts(5, 0, 0, false, 2);

const VARIATION_SKIP: &str = "variation/supplemental deed: restates only what it changes";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Status {
    Pass,
    Review,
    Fail,
    Skip,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum ErrorClass {
    #[serde(rename = "Parser Error")]
    ParserError,
    #[serde(rename = "Document Discrepancy")]
    DocumentDiscrepancy,
}

#[derive(Debug, Clone, Serialize)]
pub struct CheckResult {
    pub check_id: String,
    pub description: String,
    pub status: Status,
    pub expected: Option<String>,
    pub actual: Option<String>,
    pub error_class: Option<ErrorClass>,
    pub detail: String,
    pub pages: Vec<usize>,
}

/// One value a check depends on, with the evidence cited for it.
#[derive(Debug, Clone)]
pub struct Input {
    pub label: String,
    pub value: Option<Value>,
    pub evidence: Option<Evidence>,
}

// --------------------------------------------------------------------------- helpers

fn input(label: impl Into<String>, value: Option<Value>, evidence: Option<&Evidence>) -> Input {
    Input {
        label: label.into(),
        value,
        evidence: evidence.cloned(),
    }
}

fn unum(v: Option<u32>) -> Option<Value> {
    v.map(Value::from)
}

fn page_list<'a>(evs: impl IntoIterator<Item = Option<&'a Evidence>>) -> Vec<usize> {
    evs.into_iter()
        .flatten()
        .map(|e| e.page)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn result(
    id: &str,
    desc: &str,
    status: Status,
    expected: Option<String>,
    actual: Option<String>,
    error_class: Option<ErrorClass>,
    detail: impl Into<String>,
    pages: Vec<usize>,
) -> CheckResult {
    CheckResult {
        check_id: id.to_string(),
        description: desc.to_string(),
        status,
        expected,
        actual,
        error_class,
        detail: detail.into(),
        pages,
    }
}

fn skip(id: &str, desc: &str, detail: &str, pages: Vec<usize>) -> CheckResult {
    result(id, desc, Status::Skip, None, None, None, detail, pages)
}

fn parser_fail(id: &str, desc: &str, detail: &str, pages: Vec<usize>) -> CheckResult {
    result(
        id,
        desc,
        Status::Fail,
        None,
        None,
        Some(ErrorClass::ParserError),
        detail,
        pages,
    )
}

/// Pounds with thousands separators and two decimal places: 10058.6 -> "£10,058.60".
fn gbp(d: Decimal) -> String {
    let mut r = d.round_dp(2);
    r.rescale(2);
    let s = r.abs().to_string();
    let (int, frac) = s.split_once('.').unwrap_or((&s, "00"));
    let mut grouped = String::new();
    for (i, ch) in int.chars().enumerate() {
        if i > 0 && (int.len() - i) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(ch);
    }
    format!(
        "{}£{grouped}.{frac}",
        if r.is_sign_negative() && !r.is_zero() {
            "-"
        } else {
            ""
        }
    )
}

fn basis_name(b: AffordableBasis) -> &'static str {
    match b {
        AffordableBasis::HabitableRooms => "habitable_rooms",
        AffordableBasis::Units => "units",
        AffordableBasis::Floorspace => "floorspace",
    }
}

fn opt_str<T: std::fmt::Display>(v: Option<T>) -> String {
    v.map_or_else(|| "None".to_string(), |v| v.to_string())
}

// --------------------------------------------------------------------------- grounding

/// Why `input` isn't supported by its evidence, or None if it is.
pub fn grounding_problem(input: &Input, pages: &Pages) -> Option<String> {
    let Some(ev) = input.evidence.as_ref() else {
        return Some(format!("{}: no evidence cited", input.label));
    };
    let Some(value) = input.value.as_ref() else {
        return Some(format!("{}: not extracted", input.label));
    };
    if !value_in_snippet(value, &ev.snippet) {
        return Some(format!(
            "{}: {} not stated in its cited snippet",
            input.label, value
        ));
    }
    if !evidence_found(Some(ev), pages) {
        return Some(format!(
            "{}: cited snippet not re-locatable on p{}",
            input.label, ev.page
        ));
    }
    None
}

/// Turn an assertion result plus its inputs' grounding into a CheckResult.
#[allow(clippy::too_many_arguments)]
pub fn resolve(
    check_id: &str,
    description: &str,
    holds: bool,
    inputs: &[Input],
    pages: &Pages,
    expected: Option<String>,
    actual: Option<String>,
    extra_problems: &[String],
) -> CheckResult {
    let pg = page_list(inputs.iter().map(|i| i.evidence.as_ref()));
    let missing: Vec<&str> = inputs
        .iter()
        .filter(|i| i.value.is_none())
        .map(|i| i.label.as_str())
        .collect();
    if !missing.is_empty() {
        return result(
            check_id,
            description,
            Status::Fail,
            expected,
            actual,
            Some(ErrorClass::ParserError),
            format!("not extracted: {}", missing.join(", ")),
            pg,
        );
    }
    let mut problems: Vec<String> = inputs
        .iter()
        .filter_map(|i| grounding_problem(i, pages))
        .collect();
    problems.extend(extra_problems.iter().cloned());
    let (status, error_class, detail) = match (holds, problems.is_empty()) {
        (true, true) => (Status::Pass, None, String::new()),
        (true, false) => (
            Status::Review,
            None,
            format!("holds, but not grounded: {}", problems.join("; ")),
        ),
        (false, false) => (
            Status::Fail,
            Some(ErrorClass::ParserError),
            format!("ungrounded: {}", problems.join("; ")),
        ),
        (false, true) => (
            Status::Fail,
            Some(ErrorClass::DocumentDiscrepancy),
            "every input is stated in its cited snippet and the snippet re-locates in the OCR text; \
             confirm against the original deed before relying on this".to_string(),
        ),
    };
    result(
        check_id,
        description,
        status,
        expected,
        actual,
        error_class,
        detail,
        pg,
    )
}

// --------------------------------------------------------------------------- housing checks

pub fn check_unit_consistency(a: &Agreement, pages: &Pages) -> CheckResult {
    let h = &a.housing;
    let (cid, desc) = (
        "H1",
        "Market units + affordable units == total residential units",
    );
    let inputs = [
        input(
            "total units",
            unum(h.total_units),
            h.total_units_evidence.as_ref(),
        ),
        input(
            "market units",
            unum(h.market_units),
            h.market_units_evidence.as_ref(),
        ),
        input(
            "affordable units",
            unum(h.affordable_units),
            h.affordable_units_evidence.as_ref(),
        ),
    ];
    if a.instrument.kind.is_variation() {
        return skip(cid, desc, VARIATION_SKIP, vec![]);
    }
    if h.total_units.is_some() && h.affordable_units.is_some() && h.market_units.is_none() {
        return skip(
            cid,
            desc,
            "deed does not state market unit count separately",
            page_list(inputs.iter().map(|i| i.evidence.as_ref())),
        );
    }
    let sum = match (h.market_units, h.affordable_units) {
        (Some(m), Some(af)) => Some(u64::from(m) + u64::from(af)),
        _ => None,
    };
    let holds = matches!((h.total_units, sum), (Some(t), Some(s)) if u64::from(t) == s)
        && h.market_units.is_some();
    let actual = sum.map(|s| {
        format!(
            "{} + {} = {s}",
            h.market_units.unwrap_or(0),
            h.affordable_units.unwrap_or(0)
        )
    });
    resolve(
        cid,
        desc,
        holds,
        &inputs,
        pages,
        h.total_units.map(|t| t.to_string()),
        actual,
        &[],
    )
}

pub fn check_tenure_units(a: &Agreement, pages: &Pages) -> CheckResult {
    let h = &a.housing;
    let (cid, desc) = ("H2", "Sum of tenure units == affordable units");
    if a.instrument.kind.is_variation() {
        return skip(cid, desc, VARIATION_SKIP, vec![]);
    }
    if h.tenures.is_empty() {
        return parser_fail(cid, desc, "no tenure breakdown extracted", vec![]);
    }
    let mut inputs = vec![input(
        "affordable units",
        unum(h.affordable_units),
        h.affordable_units_evidence.as_ref(),
    )];
    inputs.extend(h.tenures.iter().map(|t| {
        input(
            format!("{} units", t.tenure.label()),
            unum(t.units),
            t.evidence.as_ref(),
        )
    }));
    if inputs.iter().any(|i| i.value.is_none()) {
        return resolve(cid, desc, false, &inputs, pages, None, None, &[]);
    }
    let got: u64 = h
        .tenures
        .iter()
        .map(|t| u64::from(t.units.unwrap_or(0)))
        .sum();
    let breakdown = h
        .tenures
        .iter()
        .map(|t| format!("{} {}", t.units.unwrap_or(0), t.tenure.label()))
        .collect::<Vec<_>>()
        .join(" + ");
    resolve(
        cid,
        desc,
        Some(got) == h.affordable_units.map(u64::from),
        &inputs,
        pages,
        h.affordable_units.map(|u| u.to_string()),
        Some(format!("{breakdown} = {got}")),
        &[],
    )
}

pub fn check_tenure_rooms(a: &Agreement, pages: &Pages) -> CheckResult {
    let h = &a.housing;
    let (cid, desc) = (
        "H3",
        "Sum of tenure habitable rooms == affordable habitable rooms",
    );
    if a.instrument.kind.is_variation() {
        return skip(cid, desc, VARIATION_SKIP, vec![]);
    }
    if h.affordable_habitable_rooms.is_none()
        || !h.tenures.iter().any(|t| t.habitable_rooms.unwrap_or(0) > 0)
    {
        return skip(cid, desc, "habitable-room breakdown not stated", vec![]);
    }
    let mut inputs = vec![input(
        "affordable habitable rooms",
        unum(h.affordable_habitable_rooms),
        h.affordable_units_evidence.as_ref(),
    )];
    inputs.extend(h.tenures.iter().map(|t| {
        input(
            format!("{} habitable rooms", t.tenure.label()),
            unum(t.habitable_rooms),
            t.evidence.as_ref(),
        )
    }));
    if inputs.iter().any(|i| i.value.is_none()) {
        return resolve(cid, desc, false, &inputs, pages, None, None, &[]);
    }
    let got: u64 = h
        .tenures
        .iter()
        .map(|t| u64::from(t.habitable_rooms.unwrap_or(0)))
        .sum();
    let breakdown = h
        .tenures
        .iter()
        .map(|t| format!("{} {}", t.habitable_rooms.unwrap_or(0), t.tenure.label()))
        .collect::<Vec<_>>()
        .join(" + ");
    resolve(
        cid,
        desc,
        Some(got) == h.affordable_habitable_rooms.map(u64::from),
        &inputs,
        pages,
        h.affordable_habitable_rooms.map(|u| u.to_string()),
        Some(format!("{breakdown} = {got}")),
        &[],
    )
}

pub fn check_claimed_pct(a: &Agreement, pages: &Pages) -> CheckResult {
    let h = &a.housing;
    let cid = "H4";
    let basis = h.claimed_affordable_basis;
    let desc = format!(
        "Claimed affordable % vs calculated % (basis: {})",
        basis.map_or("not stated", basis_name)
    );
    if a.instrument.kind.is_variation() {
        return skip(cid, &desc, VARIATION_SKIP, vec![]);
    }
    let Some(pct) = h.claimed_affordable_pct else {
        return skip(cid, &desc, "no affordable % stated in the deed", vec![]);
    };
    let claimed = input(
        "claimed %",
        Some(Value::Num(pct)),
        h.claimed_affordable_evidence.as_ref(),
    );
    let (num, den) = match basis {
        Some(AffordableBasis::HabitableRooms) => (
            input(
                "affordable habitable rooms",
                unum(h.affordable_habitable_rooms),
                h.affordable_units_evidence.as_ref(),
            ),
            // the schema carries no evidence for a scheme-wide habitable-room total
            input("total habitable rooms", unum(h.total_habitable_rooms), None),
        ),
        Some(AffordableBasis::Units) => (
            input(
                "affordable units",
                unum(h.affordable_units),
                h.affordable_units_evidence.as_ref(),
            ),
            input(
                "total units",
                unum(h.total_units),
                h.total_units_evidence.as_ref(),
            ),
        ),
        _ => {
            // Never fall back to units: a floorspace or unstated basis compared by units would be
            // a different measure, and a match or mismatch would mean nothing.
            let why = if basis == Some(AffordableBasis::Floorspace) {
                "floorspace basis: floorspace figures are not extracted"
            } else {
                "the deed's basis for the % was not extracted"
            };
            return result(
                cid,
                &desc,
                Status::Skip,
                Some(format!("{pct}%")),
                None,
                None,
                format!("cannot compute: {why}"),
                page_list([h.claimed_affordable_evidence.as_ref()]),
            );
        }
    };
    let as_dec = |v: &Option<Value>| match v {
        Some(Value::Num(d)) => Some(*d),
        _ => None,
    };
    let (num_d, den_d) = (
        as_dec(&num.value),
        as_dec(&den.value).filter(|d| !d.is_zero()),
    );
    if basis == Some(AffordableBasis::HabitableRooms) && num_d.is_some() && den_d.is_none() {
        // Deeds commonly quote the % by habitable room without the scheme-wide room count;
        // the check can't be closed from the deed alone. Report the implied denominator instead.
        let n = num_d.unwrap_or_default();
        let implied = if pct.is_zero() {
            "implied total not computable".to_string()
        } else {
            let mut v = (n * Decimal::from(100) / pct)
                .round_dp_with_strategy(1, RoundingStrategy::MidpointNearestEven);
            v.rescale(1);
            format!("implied total = {n}/{pct}% = {v} rooms (confirm against the approved plans)")
        };
        return result(
            cid,
            &desc,
            Status::Skip,
            Some(format!("{pct}%")),
            None,
            None,
            format!("total habitable rooms not stated in the deed; {implied}"),
            page_list([claimed.evidence.as_ref(), num.evidence.as_ref()]),
        );
    }
    let (Some(n), Some(d)) = (num_d, den_d) else {
        return resolve(
            cid,
            &desc,
            false,
            &[claimed, num, den],
            pages,
            Some(format!("{pct}%")),
            None,
            &[],
        );
    };
    let mut calc = (n * Decimal::from(100) / d)
        .round_dp_with_strategy(2, RoundingStrategy::MidpointAwayFromZero);
    calc.rescale(2);
    let diff = (calc - pct).abs();
    let mut r = resolve(
        cid,
        &desc,
        diff <= PCT_TOLERANCE,
        &[claimed, num, den],
        pages,
        Some(format!("{pct}%")),
        Some(format!("{n}/{d} = {calc}%")),
        &[],
    );
    if r.status == Status::Fail {
        r.detail.push_str(&format!("; off by {diff} pp"));
    }
    r
}

// --------------------------------------------------------------------------- mix table

static NUMBER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\d{1,3}(?:,\d{3})+(?:\.\d+)?|\d+(?:\.\d+)?").expect("valid regex")
});

fn counts_of(nums: impl IntoIterator<Item = Decimal>) -> BTreeMap<Decimal, usize> {
    let mut m = BTreeMap::new();
    for n in nums {
        *m.entry(n).or_insert(0) += 1;
    }
    m
}

/// Is there a line on the page stating all of these counts (repeats included)?
fn row_on_page(counts: &[u32], page: Option<&String>) -> bool {
    let Some(page) = page else { return false };
    let want = counts_of(counts.iter().map(|&n| Decimal::from(n)));
    page.lines().any(|ln| {
        let have = counts_of(
            NUMBER
                .find_iter(&ocr_digits(ln))
                .filter_map(|m| Decimal::from_str(&m.as_str().replace(',', "")).ok()),
        );
        want.iter()
            .all(|(k, v)| have.get(k).copied().unwrap_or(0) >= *v)
    })
}

fn cell(row: &crate::schema::MixRow, ten: Tenure) -> u32 {
    row.units_by_tenure
        .iter()
        .filter(|(t, _)| *t == ten)
        .map(|(_, n)| *n)
        .sum()
}

/// Schedule mix table is an independent restatement of the tenure counts: cross-check it.
///
/// Cells carry no snippets of their own, so a row counts as grounded when a line on the
/// table's cited page states all of its counts.
pub fn check_mix_table(a: &Agreement, pages: &Pages) -> Vec<CheckResult> {
    let h = &a.housing;
    let Some(t) = h.mix_table.as_ref() else {
        return vec![skip(
            "H5",
            "Approved mix table: rows sum to subtotals",
            "no mix table found",
            vec![],
        )];
    };
    let page = t.evidence.as_ref().and_then(|e| pages.get(&e.page));
    let where_ = t
        .evidence
        .as_ref()
        .map_or("?".to_string(), |e| e.page.to_string());
    let row_problems: Vec<String> = t
        .rows
        .iter()
        .filter(|r| {
            let counts: Vec<u32> = r.units_by_tenure.iter().map(|(_, n)| *n).collect();
            !row_on_page(&counts, page)
        })
        .map(|r| format!("row '{}' not found on p{where_}", r.unit_type))
        .collect();
    let sub_counts: Vec<u32> = t.subtotals.iter().map(|(_, n)| *n).collect();
    let sub_problem: Vec<String> = if t.subtotals.is_empty() || row_on_page(&sub_counts, page) {
        vec![]
    } else {
        vec!["sub-total row not found on the cited page".to_string()]
    };
    let total = input("table total", unum(t.total_units), t.evidence.as_ref());
    let mut out = Vec::new();
    let both: Vec<String> = row_problems.iter().chain(&sub_problem).cloned().collect();
    for (ten, sub) in &t.subtotals {
        let col: u32 = t.rows.iter().map(|r| cell(r, *ten)).sum();
        let parts = t
            .rows
            .iter()
            .map(|r| cell(r, *ten).to_string())
            .collect::<Vec<_>>()
            .join(" + ");
        out.push(resolve(
            "H5",
            &format!("Mix table: Σ {} rows == sub-total", ten.label()),
            col == *sub,
            &[],
            pages,
            Some(sub.to_string()),
            Some(format!("{parts} = {col}")),
            &both,
        ));
    }
    if !t.subtotals.is_empty() {
        let got: u64 = t.subtotals.iter().map(|(_, n)| u64::from(*n)).sum();
        out.push(resolve(
            "H5",
            "Mix table: Σ sub-totals == table total",
            Some(got) == t.total_units.map(u64::from),
            &[total],
            pages,
            t.total_units.map(|n| n.to_string()),
            Some(got.to_string()),
            &sub_problem,
        ));
    }
    for alloc in &h.tenures {
        let Some((_, sub)) = t.subtotals.iter().find(|(ten, _)| *ten == alloc.tenure) else {
            continue;
        };
        out.push(resolve(
            "H6",
            &format!("Definitions vs mix table: {} units", alloc.tenure.label()),
            alloc.units == Some(*sub),
            &[input(
                format!("{} units", alloc.tenure.label()),
                unum(alloc.units),
                alloc.evidence.as_ref(),
            )],
            pages,
            Some(format!("{sub} (schedule)")),
            Some(format!("{} (definition)", opt_str(alloc.units))),
            &sub_problem,
        ));
    }
    for (label, defn, tbl) in [
        ("units", h.affordable_units, t.total_units),
        (
            "habitable rooms",
            h.affordable_habitable_rooms,
            t.total_habitable_rooms,
        ),
    ] {
        let (Some(defn), Some(tbl)) = (defn, tbl) else {
            continue;
        };
        out.push(resolve(
            "H6",
            &format!("Definitions vs mix table: affordable {label}"),
            defn == tbl,
            &[
                input(
                    format!("affordable {label} (definition)"),
                    Some(Value::from(defn)),
                    h.affordable_units_evidence.as_ref(),
                ),
                input(
                    format!("affordable {label} (table)"),
                    Some(Value::from(tbl)),
                    t.evidence.as_ref(),
                ),
            ],
            pages,
            Some(format!("{tbl} (schedule)")),
            Some(format!("{defn} (definition)")),
            &[],
        ));
    }
    out
}

// --------------------------------------------------------------------------- instrument and variations

/// V1: a variation or supplemental deed cites the instrument it varies, and that citation is in the deed.
pub fn check_instrument(a: &Agreement, pages: &Pages) -> CheckResult {
    let (cid, desc) = (
        "V1",
        "Variation/supplemental deed identifies its parent instrument",
    );
    if !a.instrument.kind.is_variation() {
        return skip(cid, desc, "not a variation or supplemental deed", vec![]);
    }
    let Some(parent) = a.instrument.parent.as_ref() else {
        return parser_fail(
            cid,
            desc,
            "no parent instrument cited",
            page_list([a.instrument.evidence.as_ref()]),
        );
    };
    let pg = page_list([parent.evidence.as_ref()]);
    if evidence_found(parent.evidence.as_ref(), pages) {
        return result(
            cid,
            desc,
            Status::Pass,
            None,
            Some(parent.description.clone()),
            None,
            "",
            pg,
        );
    }
    let why = match parent.evidence.as_ref() {
        None => "no evidence cited for the parent instrument".to_string(),
        Some(e) => format!("parent instrument citation not re-locatable on p{}", e.page),
    };
    result(
        cid,
        desc,
        Status::Review,
        None,
        Some(parent.description.clone()),
        None,
        format!("parent cited, but not grounded: {why}"),
        pg,
    )
}

/// The quoted term inside a target like `definition "Affordable Housing Units"`, if any.
fn quoted_term(target: &str) -> Option<String> {
    static QUOTED: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new("[\"\u{201c}\u{201d}\u{2018}\u{2019}']([^\"\u{201c}\u{201d}\u{2018}\u{2019}']+)[\"\u{201c}\u{201d}\u{2018}\u{2019}']").expect("valid regex")
    });
    QUOTED.captures(target).map(|c| c[1].to_string())
}

/// Words that identify a variation's target: clause or paragraph numbers ("1.1.8", "1.1.5A", "4.3")
/// and the defined term ("Chargee", quoted or after "definition of"). The model describes targets
/// ("Clause 1.1.8 and definition of Chargee") rather than quoting them, so the description itself
/// is never verbatim; these tokens are.
fn target_tokens(target: &str) -> Vec<String> {
    static NUM: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"\b\d+(?:\.\d+)+[A-Za-z]?\b").expect("valid regex"));
    static TERM: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?i)definition of (?:the )?(.+?)(?:\s+(?:in|of)\s+(?:clause|schedule|paragraph)\b.*)?$").expect("valid regex")
    });
    let mut out: Vec<String> = NUM
        .find_iter(target)
        .map(|m| m.as_str().to_string())
        .collect();
    if let Some(q) = quoted_term(target) {
        out.push(q);
    } else if let Some(c) = TERM.captures(target) {
        out.push(
            c[1].trim_matches(|ch: char| "\"'\u{201c}\u{201d}\u{2018}\u{2019} ".contains(ch))
                .to_string(),
        );
    } else if let Some(last) = NUM.find_iter(target).last() {
        // "New clause 1.1.5A Affordable Rent": the term follows the number
        out.push(target[last.end()..].trim().to_string());
    }
    out.retain(|t| !t.is_empty());
    out
}

/// V2.n: each operation's citation identifies its target. Grounding only: there is no arithmetic.
/// The new wording is reported, not grounded: it usually runs past a 30-word citation.
pub fn check_variation(v: &Variation, idx: usize, pages: &Pages) -> CheckResult {
    let cid = format!("V2.{idx}");
    let desc = format!(
        "Variation {idx}: {:?} of {} is cited in the deed",
        v.op, v.target
    );
    let snippet = v
        .evidence
        .as_ref()
        .map(|e| e.snippet.as_str())
        .unwrap_or("");
    let tokens = target_tokens(&v.target);
    // the first identifying token the citation states, else the whole description (which then fails honestly)
    let target_value = tokens
        .iter()
        .find(|t| value_in_snippet(&Value::Text((*t).clone()), snippet))
        .cloned()
        .unwrap_or_else(|| v.target.clone());
    resolve(
        &cid,
        &desc,
        true,
        &[input(
            "target",
            Some(Value::Text(target_value)),
            v.evidence.as_ref(),
        )],
        pages,
        None,
        v.new_text.as_ref().map(|t| t.chars().take(80).collect()),
        &[],
    )
}

// --------------------------------------------------------------------------- contributions

pub fn check_contribution(c: &Contribution, pages: &Pages, idx: usize) -> Vec<CheckResult> {
    let mut out = Vec::new();
    let pg = page_list([c.amount.evidence.as_ref()]);
    let cid = format!("F{idx}");
    let value = c.amount.value_gbp;
    match value {
        None => {
            let figureless_ok = matches!(
                c.amount.structure,
                AmountStructure::Formula | AmountStructure::Rate
            ) || (c.amount.structure == AmountStructure::None
                && c.conditionality.kind != ConditionalityKind::Unconditional);
            let desc = format!("{}: amount extracted", c.name);
            if figureless_ok {
                out.push(skip(
                    &format!("{cid}.amt"),
                    &desc,
                    "contingent / formula-based: no fixed sum in the deed",
                    pg.clone(),
                ));
            } else {
                out.push(parser_fail(
                    &format!("{cid}.amt"),
                    &desc,
                    "no figure parsed",
                    pg,
                ));
                return out;
            }
        }
        Some(v) => {
            let amount = input("amount", Some(Value::Num(v)), c.amount.evidence.as_ref());
            out.push(resolve(
                &format!("{cid}.amt"),
                &format!("{}: amount stated in cited evidence", c.name),
                true,
                std::slice::from_ref(&amount),
                pages,
                None,
                Some(gbp(v)),
                &[],
            ));
            // figures vs words: the deed states every sum twice, so an exact cross-check is free
            let desc = format!("{}: figures == words", c.name);
            let wid = format!("{cid}.words");
            match c.amount.words.as_deref().filter(|w| !w.is_empty()) {
                None => out.push(skip(&wid, &desc, "no words form captured", pg.clone())),
                Some(words) => match crate::quantity::amount(words).ok() {
                    None => out.push(result(
                        &wid,
                        &desc,
                        Status::Fail,
                        Some(gbp(v)),
                        Some(words.to_string()),
                        Some(ErrorClass::ParserError),
                        "words form not parseable",
                        pg.clone(),
                    )),
                    Some(w) => out.push(resolve(
                        &wid,
                        &desc,
                        w == v,
                        &[
                            amount,
                            input(
                                "words form",
                                Some(Value::Text(words.to_string())),
                                c.amount.evidence.as_ref(),
                            ),
                        ],
                        pages,
                        Some(gbp(v)),
                        Some(gbp(w)),
                        &[],
                    )),
                },
            }
        }
    }
    out.extend(check_conditionality(c, pages, &cid));
    // A contribution with no stated figure only gets payment checks if the extraction cites a payment
    // at all (the Python verifier stopped after the amount check; v2 can carry a trigger for these).
    let has_payment = c.payment.covenant.is_some()
        || c.payment.trigger_phrase.is_some()
        || c.payment.trigger_event.is_some()
        || !c.payment.instalments.is_empty();
    if value.is_some() || has_payment {
        if c.payment.instalments.is_empty() {
            out.extend(check_trigger(c, pages, &cid, &pg));
        } else {
            out.extend(check_instalments(c, pages, &cid));
        }
    }
    out
}

/// F*.cond: a conditional contribution quotes its condition, and the quote is in the deed.
pub fn check_conditionality(c: &Contribution, pages: &Pages, cid: &str) -> Option<CheckResult> {
    let id = format!("{cid}.cond");
    let desc = format!("{}: condition quoted and grounded", c.name);
    match c.conditionality.kind {
        ConditionalityKind::Unconditional => None,
        ConditionalityKind::Unknown => Some(skip(
            &id,
            &desc,
            "conditionality not established",
            page_list([c.conditionality.evidence.as_ref()]),
        )),
        ConditionalityKind::Conditional => {
            let cond = c
                .conditionality
                .condition
                .as_ref()
                .filter(|s| !s.trim().is_empty());
            let inp = input(
                "condition",
                cond.map(|s| Value::Text(s.clone())),
                c.conditionality.evidence.as_ref(),
            );
            Some(resolve(&id, &desc, true, &[inp], pages, None, None, &[]))
        }
    }
}

/// `pages` with the cited page joined to the next, so a clause running over a page break re-locates.
fn spread(page: usize, pages: &Pages) -> Pages {
    let text = pages.get(&page).cloned().unwrap_or_default();
    let next = pages.get(&(page + 1)).cloned().unwrap_or_default();
    Pages::from([(page, format!("{text}\n{next}"))])
}

/// Why the clause can't be trusted as a payment covenant on `page`: it must itself be in the deed
/// (on that page, or running onto the next) and its wording must impose a payment.
fn covenant_problem(clause: &str, page: usize, pages: &Pages) -> Option<String> {
    let ev = Evidence {
        page,
        snippet: clause.to_string(),
    };
    if !evidence_found(Some(&ev), &spread(page, pages)) {
        return Some(format!(
            "cited payment clause not re-locatable in source (p{page})"
        ));
    }
    if !imposes_payment(Some(clause)) {
        // e.g. "Prior to Occupation ... the Council shall assess the amount payable" schedules an
        // assessment; the payment covenant is elsewhere.
        return Some("cited clause does not impose a payment".to_string());
    }
    None
}

/// Why the cited clause can't be trusted as this contribution's payment covenant, if it can't.
///
/// The timing phrase alone proves little: "Prior to Occupation" also schedules assessments. So
/// the clause must itself be in the deed, the phrase must sit inside it, and its wording must
/// impose a payment.
fn trigger_clause_problem(phrase: &Evidence, clause: &str, pages: &Pages) -> Option<String> {
    if !squash(&ocr_digits(clause)).contains(&squash(&ocr_digits(&phrase.snippet))) {
        return Some("cited trigger phrase is not part of the cited clause".to_string());
    }
    covenant_problem(clause, phrase.page, pages)
}

/// Trigger located, its cited phrase is in the deed, and the phrase decides the label.
pub fn check_trigger(c: &Contribution, pages: &Pages, cid: &str, pg: &[usize]) -> Vec<CheckResult> {
    let id = format!("{cid}.trig");
    let desc = format!("{}: payment trigger located", c.name);
    let p = &c.payment;
    let (Some(event), Some(clause)) = (
        p.trigger_event,
        p.covenant
            .as_ref()
            .map(|q| q.text.as_str())
            .filter(|t| !t.is_empty()),
    ) else {
        return vec![parser_fail(
            &id,
            &desc,
            "no trigger clause matched in the schedules",
            pg.to_vec(),
        )];
    };
    let tpg = page_list([p.trigger_phrase.as_ref()]);
    let fail = |detail: String| {
        result(
            &id,
            &desc,
            Status::Fail,
            None,
            Some(event.label().to_string()),
            Some(ErrorClass::ParserError),
            detail,
            tpg.clone(),
        )
    };
    let phrase = match p.trigger_phrase.as_ref() {
        Some(ph) if evidence_found(Some(ph), pages) => ph,
        other => {
            let where_ = other.map_or("no evidence cited".to_string(), |e| format!("p{}", e.page));
            return vec![fail(format!(
                "cited trigger phrase not re-locatable in source ({where_})"
            ))];
        }
    };
    if let Some(problem) = trigger_clause_problem(phrase, clause, pages) {
        return vec![fail(problem)];
    }
    let mut out = vec![result(
        &id,
        &desc,
        Status::Pass,
        None,
        Some(event.label().to_string()),
        None,
        "",
        tpg.clone(),
    )];
    let (tid, tdesc) = (
        format!("{cid}.tlabel"),
        format!("{}: trigger label follows from cited phrase", c.name),
    );
    let derived = classify_cited(Some(&phrase.snippet), Some(clause));
    let note = p
        .trigger_event_model
        .map_or(String::new(), |m| format!("model proposed '{}'", m.label()));
    out.push(match derived {
        None => result(
            &tid,
            &tdesc,
            Status::Skip,
            None,
            Some(event.label().to_string()),
            None,
            "phrase not in the trigger vocabulary; label unverified",
            tpg,
        ),
        Some(d) if d == event => result(
            &tid,
            &tdesc,
            Status::Pass,
            Some(d.label().to_string()),
            Some(event.label().to_string()),
            None,
            note,
            tpg,
        ),
        Some(d) => result(
            &tid,
            &tdesc,
            Status::Fail,
            Some(d.label().to_string()),
            Some(event.label().to_string()),
            Some(ErrorClass::ParserError),
            "label disagrees with the cited phrase",
            tpg,
        ),
    });
    out
}

/// F*.inst: instalment shares sum to 100, the lead-in imposes payment, and each instalment's trigger
/// phrase is in the deed and decides its label. Replaces the single-trigger checks.
///
/// Ids: `F3.inst` (shares), `F3.instcov` (lead-in covenant), `F3.inst1`, `F3.inst2`, ... (triggers).
pub fn check_instalments(c: &Contribution, pages: &Pages, cid: &str) -> Vec<CheckResult> {
    let insts: &[Instalment] = &c.payment.instalments;
    let mut out = Vec::new();
    let all_pg = page_list(insts.iter().map(|i| i.trigger_phrase.as_ref()));

    // Each share must be stated in its citation, not merely consistent with the total.
    let (id, desc) = (
        format!("{cid}.inst"),
        format!("{}: instalment shares sum to 100%", c.name),
    );
    if insts.iter().all(|i| i.share_pct.is_some()) {
        let sum: Decimal = insts.iter().filter_map(|i| i.share_pct).sum();
        let parts = insts
            .iter()
            .filter_map(|i| i.share_pct)
            .map(|d| d.to_string())
            .collect::<Vec<_>>()
            .join(" + ");
        let inputs: Vec<Input> = insts
            .iter()
            .enumerate()
            .map(|(n, i)| {
                input(
                    format!("instalment {} share", n + 1),
                    i.share_pct.map(Value::Num),
                    i.trigger_phrase.as_ref(),
                )
            })
            .collect();
        let share_problems: Vec<String> = insts
            .iter()
            .enumerate()
            .filter_map(|(n, inst)| {
                let share = inst.share_pct?;
                let evidence = inst.trigger_phrase.as_ref()?;
                (!percentage_in_snippet(share, &evidence.snippet)).then(|| {
                    format!(
                        "instalment {} share: {share}% not stated as a percentage in its citation",
                        n + 1
                    )
                })
            })
            .collect();
        let mut r = resolve(
            &id,
            &desc,
            sum == Decimal::from(100),
            &inputs,
            pages,
            Some("100%".to_string()),
            Some(format!("{parts} = {sum}%")),
            &share_problems,
        );
        r.pages = all_pg.clone();
        out.push(r);
    } else {
        out.push(skip(
            &id,
            &desc,
            "instalment shares not all stated as percentages",
            all_pg.clone(),
        ));
    }

    // The lead-in must itself impose payment and occur in the deed.
    let (id, desc) = (
        format!("{cid}.instcov"),
        format!("{}: instalment lead-in imposes a payment", c.name),
    );
    match c.payment.covenant.as_ref().filter(|q| !q.text.is_empty()) {
        None => out.push(parser_fail(
            &id,
            &desc,
            "no covenant or lead-in cited for the instalments",
            all_pg.clone(),
        )),
        Some(q) => {
            let page = q.evidence.as_ref().map(|e| e.page).or_else(|| {
                insts
                    .iter()
                    .find_map(|i| i.trigger_phrase.as_ref().map(|e| e.page))
            });
            let pg = page_list([q.evidence.as_ref()]);
            match page.map(|p| covenant_problem(&q.text, p, pages)) {
                None => out.push(parser_fail(
                    &id,
                    &desc,
                    "no page cited for the covenant",
                    vec![],
                )),
                Some(Some(problem)) => out.push(result(
                    &id,
                    &desc,
                    Status::Fail,
                    None,
                    None,
                    Some(ErrorClass::ParserError),
                    problem,
                    pg,
                )),
                Some(None) => out.push(result(&id, &desc, Status::Pass, None, None, None, "", pg)),
            }
        }
    }

    // Each trigger phrase must occur in the deed and determine its label.
    for (n, inst) in insts.iter().enumerate() {
        let n = n + 1;
        let (id, desc) = (
            format!("{cid}.inst{n}"),
            format!("{}: instalment {n} trigger located and labelled", c.name),
        );
        let Some(phrase) = inst.trigger_phrase.as_ref() else {
            out.push(parser_fail(&id, &desc, "no trigger phrase cited", vec![]));
            continue;
        };
        let pg = vec![phrase.page];
        let actual = inst.trigger_event.map(|t| t.label().to_string());
        let derived = classify(Some(&phrase.snippet));
        match derived {
            Some(d) if Some(d) != inst.trigger_event => out.push(result(
                &id,
                &desc,
                Status::Fail,
                Some(d.label().to_string()),
                actual,
                Some(ErrorClass::ParserError),
                "label disagrees with the cited phrase",
                pg,
            )),
            None if evidence_found(Some(phrase), pages) => out.push(result(
                &id,
                &desc,
                Status::Skip,
                None,
                actual,
                None,
                "phrase not in the trigger vocabulary; label unverified",
                pg,
            )),
            _ => {
                let inp = input(
                    format!("instalment {n} trigger phrase"),
                    Some(Value::Text(phrase.snippet.clone())),
                    Some(phrase),
                );
                out.push(resolve(
                    &id,
                    &desc,
                    true,
                    &[inp],
                    pages,
                    derived.map(|d| d.label().to_string()),
                    actual,
                    &[],
                ));
            }
        }
    }
    out
}

// --------------------------------------------------------------------------- entry point

/// Run every check on an extracted agreement against its page text.
pub fn verify(agreement: &Agreement, pages: &Pages) -> Vec<CheckResult> {
    let mut results = vec![
        check_unit_consistency(agreement, pages),
        check_tenure_units(agreement, pages),
        check_tenure_rooms(agreement, pages),
        check_claimed_pct(agreement, pages),
    ];
    results.extend(check_mix_table(agreement, pages));
    results.push(check_instrument(agreement, pages));
    for (i, v) in agreement.variations.iter().enumerate() {
        results.push(check_variation(v, i + 1, pages));
    }
    for (i, c) in agreement.contributions.iter().enumerate() {
        results.extend(check_contribution(c, pages, i + 1));
    }
    results
}
