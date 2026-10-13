//! Run identity and the shared money formatter.

use std::path::Path;

use rust_decimal::Decimal;
use s106::audit::{ReplacementIdentity, RunIdentity};

fn d(s: &str) -> Decimal {
    Decimal::from_str_exact(s).unwrap()
}

#[test]
fn money_formatter_keeps_exact_value_and_sign() {
    assert_eq!(s106::report::gbp(d("10058.6")), "£10,058.60");
    assert_eq!(s106::report::gbp(d("0.125")), "£0.125");
    assert_eq!(s106::report::gbp(d("-5")), "-£5.00");
    assert_eq!(s106::report::gbp(d("1234567")), "£1,234,567.00");
}

#[test]
fn latest_replacement_names_the_extraction_for_a_group() {
    let mut run = RunIdentity::new(
        &serde_json::json!({}),
        &Default::default(),
        Path::new("missing.json"),
    );
    let base = run.extraction_sha256.clone();
    let rep = |sha: &str| ReplacementIdentity {
        field_group: "housing".into(),
        extraction_sha256: sha.into(),
        fixture_reference: "f".into(),
        fixture_sha256: None,
    };
    run.replacements = vec![rep("a"), rep("b")];
    assert_eq!(run.extraction_for(Some("housing")), "b");
    assert_eq!(run.extraction_for(Some("Other Contribution")), base);
    assert_eq!(run.extraction_for(None), base);
    assert_eq!(run.parser_fingerprint.len(), 64);
}
