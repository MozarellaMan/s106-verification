//! Constructed extraction/source controls through production conversion, gate and renderers.
use s106::{
    document::{Document, Line, PageText, TextSource},
    extract::{self, Info},
    numeric::Category,
    report,
    verify::{self, Status},
    wire::PageMap,
};
use serde_json::{Value, json};
use std::path::PathBuf;
fn audit(
    figure: &str,
    words: &str,
    snippet: &str,
    page_source: &str,
) -> (s106::schema::Agreement, Vec<verify::CheckResult>) {
    // Recorded shape only: every numeric expectation and source here is constructed.
    let mut output: Value = serde_json::from_str(include_str!(
        "../fixtures/llm/southwark_s106_sample/claude-haiku-5-5.extract.v2.json"
    ))
    .unwrap();
    output = output["output"].take();
    let mut c = output["contributions"][0].take();
    c["name"] = json!("Test Contribution");
    c["amount_gbp"] = json!(figure);
    c["amount_in_words"] = json!(words);
    c["amount_structure"] = json!("fixed");
    c["amount_evidence"] = json!({"page": 1, "snippet": snippet});
    c["instalments"] = json!([]);
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
                text: page_source.into(),
                bbox: [0.0; 4],
            }],
        }],
    };
    let info = Info {
        model: "recorded-shape-only".into(),
        served_by: "constructed".into(),
        source: "replay".into(),
        usage: None,
        fixture: PathBuf::from("constructed-fixture.json"),
    };
    let agreement = extract::to_agreement(&doc, &output, &PageMap::identity(1), &info).unwrap();
    let checks = verify::verify(&agreement, &doc.pages());
    (agreement, checks)
}
#[test]
fn correction_and_corrupted_twins_reach_reporting() {
    let source = "The sum of £10 (ten pounds)";
    let (bad, checks) = audit("1,0", "ten pounds", source, source);
    let failure = checks.iter().find(|c| c.check_id == "F1.amt").unwrap();
    assert_eq!(failure.status, Status::Fail);
    let diagnostic = &failure.diagnostics[0];
    assert_eq!(diagnostic.code, "NUM002");
    assert_eq!(diagnostic.observed_input.as_deref(), Some("1,0"));
    assert_eq!(diagnostic.claimed_page, Some(1));
    assert_eq!(diagnostic.citation_relocated, Some(true));
    assert_eq!(
        diagnostic.span.as_ref().unwrap().coordinate_space,
        "original_observed_input"
    );
    let human = report::render(&bad, &checks);
    assert!(human.contains("NUM002"));
    assert!(human.contains("1,0"));
    let json = serde_json::to_value(report::AuditJson {
        agreement: &bad,
        verification: &checks,
    })
    .unwrap();
    assert!(
        json["verification"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["diagnostics"][0]["observed_input"] == "1,0")
    );
    let old_run = bad.parser_run.as_ref().unwrap();
    let (good, checks) = audit("10", "ten pounds", source, source);
    for id in ["F1.amt", "F1.words"] {
        assert_eq!(
            checks.iter().find(|c| c.check_id == id).unwrap().status,
            Status::Pass
        );
    }
    assert_eq!(
        old_run.page_text_sha256,
        good.parser_run.as_ref().unwrap().page_text_sha256
    );
    assert_ne!(
        old_run.extraction_sha256,
        good.parser_run.as_ref().unwrap().extraction_sha256
    );
    for words in [
        "ten pounds and",
        "ten pounds and unreadable pence",
        "ten pounds9",
        "ten pounds!",
    ] {
        let (_, checks) = audit("10", words, source, source);
        let failure = checks.iter().find(|c| c.check_id == "F1.words").unwrap();
        assert_eq!(failure.status, Status::Fail, "{words}");
        assert!(!failure.diagnostics.is_empty());
    }
    if let Ok(dir) = std::env::var("S106_DIAGNOSTIC_DEMO_DIR") {
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(format!("{dir}/failure.txt"), human).unwrap();
        std::fs::write(
            format!("{dir}/corrected.audit.json"),
            serde_json::to_string_pretty(&report::AuditJson {
                agreement: &good,
                verification: &checks,
            })
            .unwrap(),
        )
        .unwrap();
        std::fs::write(
            format!("{dir}/failure.audit.json"),
            serde_json::to_string_pretty(&json).unwrap(),
        )
        .unwrap();
    }
}
#[test]
fn parsed_changed_numbers_still_require_grounding_and_relocation() {
    let source = "The sum of £10 (ten pounds)";
    let (_, checks) = audit("11", "eleven pounds", source, source);
    for id in ["F1.amt", "F1.words"] {
        assert_ne!(
            checks.iter().find(|c| c.check_id == id).unwrap().status,
            Status::Pass
        );
    }
    let (_, checks) = audit("10", "ten pounds", source, "Something else");
    assert_eq!(
        checks
            .iter()
            .find(|c| c.check_id == "F1.amt")
            .unwrap()
            .status,
        Status::Review
    );
}
#[test]
fn ambiguous_source_is_review_with_structured_uncertainty() {
    let source = "The sum of £5o";
    let (agreement, checks) = audit("50", "", source, source);
    let check = checks.iter().find(|c| c.check_id == "F1.amt").unwrap();
    assert_eq!(check.status, Status::Review);
    let error = check
        .diagnostics
        .iter()
        .find(|d| d.category == Category::AmbiguousSource)
        .unwrap();
    assert_eq!(error.code, "NUM005");
    assert_eq!(error.citation_relocated, Some(false)); // Existing page repair means this raw OCR citation does not relocate.
    assert!(error.normalised_span.is_none());
    if let Ok(dir) = std::env::var("S106_DIAGNOSTIC_DEMO_DIR") {
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            format!("{dir}/ocr-review.audit.json"),
            serde_json::to_string_pretty(&report::AuditJson {
                agreement: &agreement,
                verification: &checks,
            })
            .unwrap(),
        )
        .unwrap();
    }
}
#[test]
fn absent_conversion_is_distinct_from_invalid_nonempty() {
    let (empty, _) = audit("", "", "", "");
    assert!(empty.numeric_diagnostics.is_empty());
    let (bad, _) = audit("banana", "", "", "");
    assert_eq!(bad.numeric_diagnostics.len(), 1);
    assert_eq!(
        bad.numeric_diagnostics[0].observed_input.as_deref(),
        Some("banana")
    );
}

#[test]
fn repaired_page_relocation_never_verifies_numeric_value() {
    let (_, checks) = audit("50", "", "The sum of £50", "The sum of £5o");
    let check = checks.iter().find(|c| c.check_id == "F1.amt").unwrap();
    assert_eq!(check.status, Status::Review);
    assert!(check.diagnostics.iter().any(|d| matches!(
        d.cause,
        s106::numeric::Cause::RelocationRequiresDigitSubstitution
    )));
}

#[test]
fn original_extraction_whitespace_is_retained() {
    let (agreement, checks) = audit(
        " 1,0 ",
        "  ten pounds and  ",
        "£10 (ten pounds)",
        "£10 (ten pounds)",
    );
    assert_eq!(
        agreement.numeric_diagnostics[0].observed_input.as_deref(),
        Some(" 1,0 ")
    );
    assert_eq!(
        agreement.contributions[0].amount.words.as_deref(),
        Some("  ten pounds and  ")
    );
    assert!(checks.iter().any(|c| !c.diagnostics.is_empty()));
}

#[test]
fn missing_required_figure_has_absent_facts_without_fabricated_span() {
    let (_, checks) = audit("", "", "A fixed contribution", "A fixed contribution");
    let check = checks.iter().find(|c| c.check_id == "F1.amt").unwrap();
    assert_eq!(check.diagnostics[0].category, Category::Absent);
    assert!(check.diagnostics[0].observed_input.is_none());
    assert!(check.diagnostics[0].span.is_none());
    assert!(check.diagnostics[0].normalised_span.is_none());
}

#[test]
fn comma_scale_locator_acceptance_is_an_explicit_expansion() {
    // Literal construction: 1,000,000 + 602. Merged-main's prose locator split the comma,
    // so an unchanged words-only citation yielded REVIEW. Record this grammar expansion
    // for consumers to flag possible weakening before deciding whether to adopt it.
    let source = "one million, six hundred and two pounds";
    let pages = s106::evidence::Pages::from([(1, source.to_owned())]);
    let input = verify::Input {
        label: "amount".into(),
        value: Some(s106::text::Value::Num("1000602".parse().unwrap())),
        evidence: Some(s106::schema::Evidence {
            page: 1,
            snippet: source.into(),
        }),
    };
    let check = verify::resolve(
        "CONTROL.amt",
        "word-only comma scale control",
        true,
        &[input],
        &pages,
        None,
        None,
        &[],
    );
    assert_eq!(check.status, Status::Pass);
    assert!(
        !s106::text::numbers_in("one million,, six hundred and two pounds")
            .contains(&"1000602".parse().unwrap())
    );
    if let Ok(dir) = std::env::var("S106_DIAGNOSTIC_DEMO_DIR") {
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            format!("{dir}/acceptance-after.json"),
            serde_json::to_string_pretty(&check).unwrap(),
        )
        .unwrap();
    }
}

#[test]
fn exact_mismatch_remains_visible_in_both_reporting_paths() {
    let source = "The sum of £10.001 (ten pounds)";
    let (agreement, checks) = audit("10.001", "ten pounds", source, source);
    let check = checks.iter().find(|c| c.check_id == "F1.words").unwrap();
    assert_eq!(check.status, Status::Fail);
    assert_eq!(
        check.error_class,
        Some(verify::ErrorClass::DocumentDiscrepancy)
    );
    assert_eq!(check.expected.as_deref(), Some("£10.001"));
    assert_eq!(check.actual.as_deref(), Some("£10.00"));
    assert!(report::render(&agreement, &checks).contains("£10.001"));
    let json = serde_json::to_value(report::AuditJson {
        agreement: &agreement,
        verification: &checks,
    })
    .unwrap();
    assert!(
        json["verification"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["expected"] == "£10.001")
    );
}
