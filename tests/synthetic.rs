//! Tier 2: the gate end to end on invented deeds (`tests/synthetic/`), with no PDF, recorded
//! model response or network. Each deed is written to produce particular outcomes; the tests
//! below name them, and `<id>/checks.json` pins every check so any change is a reviewable diff
//! (after an intended change: `S106_BLESS=1 cargo test --test synthetic`).

mod common;

use common::{bless, synthetic};
use s106::verify::{CheckResult, ErrorClass, Status};
use serde_json::{Value, json};

fn outcomes(results: &[CheckResult]) -> String {
    let body: Vec<String> = results
        .iter()
        .map(|r| {
            let v: Value = json!({
                "check": r.check_id,
                "description": r.description,
                "status": r.status,
                "error_class": r.error_class,
            });
            format!("  {v}")
        })
        .collect();
    format!("[\n{}\n]\n", body.join(",\n"))
}

#[test]
fn every_deed_reproduces_its_pinned_outcomes() {
    let mut changed = vec![];
    for id in synthetic::IDS {
        let (_, results) = synthetic::verify(id);
        let got = outcomes(&results);
        let path = synthetic::dir(id).join("checks.json");
        if bless() {
            std::fs::write(&path, &got).unwrap();
            continue;
        }
        if std::fs::read_to_string(&path).unwrap_or_default() != got {
            changed.push(format!("{id}:\n{got}"));
        }
    }
    assert!(
        changed.is_empty(),
        "outcomes changed (if intended: S106_BLESS=1 cargo test --test synthetic):\n{}",
        changed.join("\n")
    );
}

fn check<'a>(results: &'a [CheckResult], id: &str, description: &str) -> &'a CheckResult {
    results
        .iter()
        .find(|r| r.check_id == id && r.description.contains(description))
        .unwrap_or_else(|| panic!("no check {id} {description}"))
}

fn is(r: &CheckResult, status: Status, class: Option<ErrorClass>) {
    assert_eq!(
        (r.status, r.error_class),
        (status, class),
        "{}: {}",
        r.check_id,
        r.description
    );
}

/// Every input grounded, including OCR damage the matcher is built for: a pound sign read as
/// "f", dropped spaces, and a two-column definition whose term splits across rows.
#[test]
fn a_clean_agreement_passes_every_check_that_applies() {
    let (_, results) = synthetic::verify("agreement");
    for r in &results {
        if r.check_id == "V1" {
            is(r, Status::Skip, None);
        } else {
            is(r, Status::Pass, None);
        }
    }
}

#[test]
fn each_failure_mode_is_reported_as_built() {
    let (_, r) = synthetic::verify("discrepancies");
    // 70 market + 25 affordable against 100 homes, every figure stated where cited
    is(
        check(&r, "H1", ""),
        Status::Fail,
        Some(ErrorClass::DocumentDiscrepancy),
    );
    // First Homes are defined without a count
    is(
        check(&r, "H2", ""),
        Status::Fail,
        Some(ErrorClass::ParserError),
    );
    // 25% holds, but the cited sentence is on page 3, not page 2
    is(check(&r, "H4", ""), Status::Review, None);
    // £10,500 stated as "ten thousand pounds"
    is(
        check(&r, "F1.words", ""),
        Status::Fail,
        Some(ErrorClass::DocumentDiscrepancy),
    );
    // the page reads "£5o,000": found only by repairing an OCR letter, so unproven
    is(check(&r, "F2.amt", ""), Status::Review, None);
    // a trigger phrase the deed does not contain
    is(
        check(&r, "F2.trig", ""),
        Status::Fail,
        Some(ErrorClass::ParserError),
    );
    // £25,000 extracted, £20,000 in the cited snippet
    is(check(&r, "F3.amt", ""), Status::Review, None);
    // a formula, not a sum
    is(check(&r, "F4.amt", ""), Status::Skip, None);
}

#[test]
fn a_variation_skips_whole_scheme_checks_and_grounds_each_change() {
    let (_, r) = synthetic::verify("variation");
    for id in ["H1", "H2", "H3", "H4"] {
        is(check(&r, id, ""), Status::Skip, None);
    }
    is(check(&r, "V1", ""), Status::Pass, None);
    for id in ["V2.1", "V2.2", "V2.3", "V2.4"] {
        is(check(&r, id, ""), Status::Pass, None);
    }
    // a deletion of a paragraph the deed never mentions
    is(check(&r, "V2.5", ""), Status::Review, None);
}
