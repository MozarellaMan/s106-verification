//! Typed findings: constructed extraction/source controls through the production conversion,
//! gate and renderers. The expected answers are written here, not derived from the parser.

use std::path::PathBuf;

use s106::audit::{Coded, CoordinateSpace};
use s106::document::{Document, Line, PageText, TextSource};
use s106::extract::{self, Info, field_of};
use s106::quantity::Category;
use s106::report;
use s106::schema::Agreement;
use s106::verify::{self, CheckResult, Scope, Status};
use s106::wire::PageMap;
use serde_json::{Value, json};

const SOURCE: &str = "The sum of £10 (ten pounds)";

fn audit_with(
    figure: &str,
    words: &str,
    edit: impl FnOnce(&mut Value),
) -> (Agreement, Vec<CheckResult>) {
    // An invented deed's extraction, for its shape only: every expectation and source here is constructed.
    let mut output: Value =
        serde_json::from_str(include_str!("synthetic/agreement/extraction.json")).unwrap();
    let mut c = output["contributions"][0].take();
    c["name"] = json!("Test Contribution");
    c["amount_gbp"] = json!(figure);
    c["amount_in_words"] = json!(words);
    c["amount_structure"] = json!("fixed");
    c["amount_evidence"] = json!({"page": 1, "snippet": SOURCE});
    c["instalments"] = json!([]);
    output["contributions"] = json!([c]);
    edit(&mut output);
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
                text: SOURCE.into(),
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
    let checks = verify::verify(&agreement, &doc.pages());
    (agreement, checks)
}

fn audit(figure: &str, words: &str) -> (Agreement, Vec<CheckResult>) {
    audit_with(figure, words, |_| {})
}

fn check<'a>(checks: &'a [CheckResult], id: &str) -> &'a CheckResult {
    checks.iter().find(|c| c.check_id == id).unwrap()
}

#[test]
fn invalid_figure_is_reported_by_its_own_check_and_the_corrected_twin_passes() {
    let (bad, checks) = audit("1,0", "ten pounds");
    let failure = check(&checks, "F1.amt");
    assert_eq!(failure.status, Status::Fail);
    let finding = &failure.findings[0];
    assert_eq!(finding.category, Coded(Category::Invalid));
    assert_eq!(finding.category.0.code(), "NUM002");
    assert_eq!(finding.field, "agreement/contributions/0/amount/value_gbp");
    let observed = finding.observed.as_ref().unwrap();
    assert_eq!(observed.input, "1,0");
    assert_eq!(observed.coordinates, CoordinateSpace::ObservedInput);
    let span = observed.span.clone().unwrap();
    assert!(span.end <= "1,0".len());
    let citation = finding.citation.as_ref().unwrap();
    assert_eq!((citation.page, citation.relocated), (1, true));

    let human = report::render(&bad, &checks);
    assert!(human.contains("NUM002"));
    assert!(human.contains("\"1,0\""));
    assert!(human.contains("observed_input"));
    let json = serde_json::to_value(report::AuditJson {
        agreement: &bad,
        verification: &checks,
    })
    .unwrap();
    let f = json["verification"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["check_id"] == "F1.amt")
        .unwrap()["findings"][0]
        .clone();
    assert_eq!(f["code"], "NUM002");
    assert_eq!(f["category"], "invalid");
    assert_eq!(f["observed"]["input"], "1,0");
    assert_eq!(f["observed"]["coordinates"], "observed_input");
    // passing checks carry no findings key
    assert!(
        json["verification"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r.get("findings").is_none())
    );

    let (_, checks) = audit("10", "ten pounds");
    for id in ["F1.amt", "F1.words"] {
        let c = check(&checks, id);
        assert_eq!(c.status, Status::Pass, "{id}");
        assert!(c.findings.is_empty());
    }
}

#[test]
fn unparseable_words_fail_the_words_check_with_a_finding() {
    for words in [
        "ten pounds and",
        "ten pounds and unreadable pence",
        "ten pounds9",
        "ten pounds!",
    ] {
        let (_, checks) = audit("10", words);
        let failure = check(&checks, "F1.words");
        assert_eq!(failure.status, Status::Fail, "{words}");
        let finding = &failure.findings[0];
        assert_eq!(finding.field, "agreement/contributions/0/amount/words");
        assert_eq!(finding.observed.as_ref().unwrap().input, words);
        assert_eq!(check(&checks, "F1.amt").status, Status::Pass);
    }
}

#[test]
fn blank_figure_is_an_absent_finding_without_an_invented_span() {
    let (_, checks) = audit("", "");
    let failure = check(&checks, "F1.amt");
    assert_eq!(failure.status, Status::Fail);
    let finding = &failure.findings[0];
    assert_eq!(finding.category, Coded(Category::Absent));
    assert_eq!(finding.category.0.code(), "NUM001");
    assert!(finding.observed.is_none());
}

#[test]
fn raw_text_is_kept_verbatim_beside_the_parsed_value() {
    let (a, _) = audit(" 1,0 ", "  ten pounds and  ");
    let amount = &a.contributions[0].amount;
    let value = amount.value_gbp.as_ref().unwrap();
    assert_eq!(value.raw, " 1,0 ");
    assert!(value.parsed.is_err());
    let words = amount.words.as_ref().unwrap();
    assert_eq!(words.raw, "  ten pounds and  ");
    assert!(words.parsed.is_err());
}

#[test]
fn invalid_share_and_invalid_claimed_percentage_are_reported_by_their_checks() {
    let (_, checks) = audit_with("10", "ten pounds", |o| {
        o["contributions"][0]["instalments"] = json!([
            {"share_pct": "5,0", "trigger_phrase": {"page": 1, "snippet": SOURCE}},
            {"share_pct": "50", "trigger_phrase": {"page": 1, "snippet": SOURCE}},
        ]);
        o["housing"]["claimed_affordable_pct"] = json!("3,5");
    });
    let shares = check(&checks, "F1.inst");
    assert_eq!(shares.status, Status::Fail);
    assert_eq!(shares.findings.len(), 1);
    assert_eq!(
        shares.findings[0].field,
        "agreement/contributions/0/payment/instalments/0/share_pct"
    );
    assert_eq!(shares.findings[0].observed.as_ref().unwrap().input, "5,0");
    let claimed = check(&checks, "H4");
    assert_eq!(claimed.status, Status::Fail);
    assert_eq!(
        claimed.findings[0].field,
        "agreement/housing/claimed_affordable_pct"
    );
    assert_eq!(claimed.findings[0].category.0.code(), "NUM002");
}

#[test]
fn scope_is_typed_and_field_of_follows_it() {
    let (a, checks) = audit("10", "ten pounds");
    assert_eq!(check(&checks, "F1.amt").scope, Scope::Contribution(0));
    assert_eq!(check(&checks, "H1").scope, Scope::Housing);
    assert_eq!(check(&checks, "V1").scope, Scope::Instrument);
    assert_eq!(field_of(&a, check(&checks, "F1.amt")), "Test Contribution");
    assert_eq!(field_of(&a, check(&checks, "H1")), "housing");
    assert_eq!(field_of(&a, check(&checks, "V1")), "instrument");
    let mut variation = check(&checks, "V1").clone();
    variation.check_id = "anything".into();
    variation.scope = Scope::Variation(2);
    assert_eq!(field_of(&a, &variation), "instrument");
}
