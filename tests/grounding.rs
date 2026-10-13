//! Unit-aware grounding and relocation findings: constructed controls through the production
//! conversion and gate. Expected answers are written here, not derived from the verifier.

use std::path::PathBuf;

use rust_decimal::Decimal;
use s106::audit::CoordinateSpace;
use s106::document::{Document, Line, PageText, TextSource};
use s106::evidence::Pages;
use s106::extract::{self, Info};
use s106::quantity::{Category, Cause};
use s106::schema::Evidence;
use s106::text::{Value, value_in_snippet};
use s106::verify::{self, CheckResult, Input, Status};
use s106::wire::PageMap;
use serde_json::{Value as Json, json};

fn dec(s: &str) -> Decimal {
    s.parse().unwrap()
}

fn audit(figure: &str, snippet: &str, page: &str) -> Vec<CheckResult> {
    audit_with(figure, snippet, page, json!([]))
}

fn audit_with(figure: &str, snippet: &str, page: &str, instalments: Json) -> Vec<CheckResult> {
    // An invented deed's extraction, for its shape only: every expectation and source here is constructed.
    let mut output: Json =
        serde_json::from_str(include_str!("synthetic/agreement/extraction.json")).unwrap();
    let mut c = output["contributions"][0].take();
    c["name"] = json!("Test Contribution");
    c["amount_gbp"] = json!(figure);
    c["amount_in_words"] = json!("");
    c["amount_structure"] = json!("fixed");
    c["amount_evidence"] = json!({"page": 1, "snippet": snippet});
    c["instalments"] = instalments;
    output["contributions"] = json!([c]);
    let doc = Document {
        path: PathBuf::from("constructed.pdf"),
        sha256: "constructed-source".into(),
        ocr_version: "constructed/1".into(),
        pages: vec![PageText {
            page: 1,
            source: TextSource::Ocr,
            width: 1.0,
            height: 1.0,
            lines: vec![Line {
                text: page.into(),
                bbox: [0.0; 4],
            }],
        }],
    };
    let info = Info {
        model: "recorded-shape-only".into(),
        served_by: "constructed".into(),
        source: "replay".into(),
        usage: None,
        fixture: Some(PathBuf::from("constructed-fixture.json")),
    };
    let agreement = extract::to_agreement(&doc, &output, &PageMap::identity(1), &info).unwrap();
    verify::verify(&agreement, &doc.pages())
}

fn amt(checks: &[CheckResult]) -> &CheckResult {
    checks.iter().find(|c| c.check_id == "F1.amt").unwrap()
}

fn control(value: Value, snippet: &str, page: &str) -> CheckResult {
    let pages = Pages::from([(1, page.to_string())]);
    let input = Input {
        label: "amount".into(),
        field: None,
        value: Some(value),
        evidence: Some(Evidence {
            page: 1,
            snippet: snippet.into(),
        }),
    };
    verify::resolve("C.amt", "control", true, &[input], &pages, None, None, &[])
}

#[test]
fn a_percentage_does_not_ground_an_amount() {
    assert!(!value_in_snippet(&Value::Gbp(dec("50")), "50% of the sum"));
    assert!(value_in_snippet(
        &Value::Percent(dec("50")),
        "50% of the sum"
    ));
    assert!(!value_in_snippet(
        &Value::Count(dec("50")),
        "50% of the sum"
    ));
}

#[test]
fn an_amount_does_not_ground_a_count() {
    assert!(value_in_snippet(
        &Value::Gbp(dec("10000")),
        "the sum of £10,000"
    ));
    assert!(!value_in_snippet(
        &Value::Count(dec("10000")),
        "the sum of £10,000"
    ));
    // an amount is often printed without the pound sign, or as words
    assert!(value_in_snippet(
        &Value::Gbp(dec("10000")),
        "the sum of 10,000"
    ));
    assert!(value_in_snippet(
        &Value::Gbp(dec("10000")),
        "ten thousand pounds"
    ));
}

#[test]
fn a_count_is_grounded_by_a_count_in_words_and_digits() {
    assert!(value_in_snippet(
        &Value::Count(dec("50")),
        "fifty (50) Affordable Housing Units"
    ));
}

#[test]
fn control_with_wrong_unit_is_review_with_value_not_stated() {
    let source = "50% of the Affordable Housing";
    let r = control(Value::Gbp(dec("50")), source, source);
    assert_eq!(r.status, Status::Review);
    assert!(r.findings.iter().any(|f| matches!(
        &f.cause,
        Cause::ValueNotStated { expected_value } if expected_value == "50"
    )));
    assert_eq!(
        control(Value::Percent(dec("50")), source, source).status,
        Status::Pass
    );
}

#[test]
fn repaired_only_relocation_is_review_with_the_repair() {
    let checks = audit("50", "The sum of £50", "The sum of £5o");
    let check = amt(&checks);
    assert_eq!(check.status, Status::Review);
    assert!(
        check.detail.contains("page-digit substitution"),
        "{}",
        check.detail
    );
    let finding = check
        .findings
        .iter()
        .find(|f| f.cause == Cause::RelocationRequiresDigitSubstitution)
        .expect("relocation finding");
    assert_eq!(finding.category.0, Category::AmbiguousSource);
    assert_eq!(finding.field, "agreement/contributions/0/amount/value_gbp");
    let citation = finding.citation.as_ref().unwrap();
    assert!(citation.relocated);
    assert_eq!(citation.repairs.len(), 1);
    assert_eq!(
        (citation.repairs[0].from, citation.repairs[0].to),
        ('o', '0')
    );
    let at = "The sum of £5o".rfind('o').unwrap();
    assert_eq!(citation.repairs[0].at, at);
    assert!(citation.range.as_ref().unwrap().contains(&at));
    let observed = finding.observed.as_ref().unwrap();
    assert_eq!(observed.coordinates, CoordinateSpace::Excerpt);
    assert_eq!(observed.input, "The sum of £50");
}

#[test]
fn unreadable_ocr_in_the_snippet_is_an_ambiguous_source_finding() {
    let source = "The sum of £5o";
    let checks = audit("50", source, source);
    let check = amt(&checks);
    assert_eq!(check.status, Status::Review);
    let finding = check
        .findings
        .iter()
        .find(|f| matches!(f.cause, Cause::OcrSubstitutionCandidate { .. }))
        .expect("OCR candidate finding");
    assert_eq!(finding.category.0, Category::AmbiguousSource);
    let observed = finding.observed.as_ref().unwrap();
    assert_eq!(observed.coordinates, CoordinateSpace::Excerpt);
    assert_eq!(observed.input, source);
    let span = observed.span.clone().unwrap();
    assert!(span.end <= source.len() && span.start <= span.end);
    assert!(source[..span.end].ends_with('o') || source[span.start..].contains('o'));
}

#[test]
fn ungrounded_value_reports_value_not_stated() {
    let source = "The sum of £10";
    let checks = audit("11", source, source);
    let check = amt(&checks);
    assert_ne!(check.status, Status::Pass);
    let finding = &check.findings[0];
    assert_eq!(
        finding.cause,
        Cause::ValueNotStated {
            expected_value: "11".into()
        }
    );
    assert_eq!(finding.category.0, Category::Ungrounded);
    assert!(finding.citation.as_ref().unwrap().relocated);
}

#[test]
fn unrelocatable_citation_reports_citation_not_relocated() {
    let checks = audit("10", "The sum of £10 (ten pounds)", "Something else");
    let check = amt(&checks);
    assert_eq!(check.status, Status::Review);
    let finding = &check.findings[0];
    assert_eq!(finding.cause, Cause::CitationNotRelocated);
    let citation = finding.citation.as_ref().unwrap();
    assert!(!citation.relocated && citation.range.is_none() && citation.repairs.is_empty());
}

#[test]
fn grounded_value_has_no_findings() {
    let source = "The sum of £10 (ten pounds)";
    let checks = audit("10", source, source);
    assert_eq!(amt(&checks).status, Status::Pass);
    assert!(amt(&checks).findings.is_empty());
}

#[test]
fn word_only_comma_scale_control_passes() {
    let source = "one million, six hundred and two pounds";
    let r = control(Value::Gbp(dec("1000602")), source, source);
    assert_eq!(r.status, Status::Pass);
}

#[test]
fn grounding_findings_carry_canonical_field_paths() {
    // an unreadable amount, and a share written without percentage wording
    let page = "The sum of £5o payable 50 on commencement";
    let checks = audit_with(
        "50",
        "The sum of £5o",
        page,
        json!([
            {"share_pct": "50", "trigger_phrase": {"page": 1, "snippet": "50 on commencement"}},
            {"share_pct": "50", "trigger_phrase": {"page": 1, "snippet": "50 on commencement"}},
        ]),
    );
    let amount = amt(&checks);
    assert!(!amount.findings.is_empty());
    assert!(
        amount
            .findings
            .iter()
            .all(|f| f.field == "agreement/contributions/0/amount/value_gbp"),
        "{:?}",
        amount.findings.iter().map(|f| &f.field).collect::<Vec<_>>()
    );
    let shares = checks.iter().find(|c| c.check_id == "F1.inst").unwrap();
    let fields: Vec<&str> = shares.findings.iter().map(|f| f.field.as_str()).collect();
    assert!(!fields.is_empty());
    assert!(
        fields.iter().all(
            |f| f.starts_with("agreement/contributions/0/payment/instalments/")
                && f.ends_with("/share_pct")
        ),
        "{fields:?}"
    );
    assert!(shares.findings.iter().any(|f| {
        matches!(f.cause, Cause::MissingPercentageWording { .. })
            && f.field == "agreement/contributions/0/payment/instalments/0/share_pct"
    }));
}
