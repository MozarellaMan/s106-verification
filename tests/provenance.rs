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
        Some(Path::new("missing.json")),
    );
    let base = run.extraction_sha256.clone();
    let rep = |sha: &str| ReplacementIdentity {
        field_group: "housing".into(),
        extraction_sha256: sha.into(),
        fixture_reference: Some("f".into()),
        fixture_sha256: None,
    };
    run.replacements = vec![rep("a"), rep("b")];
    assert_eq!(run.extraction_for(Some("housing")), "b");
    assert_eq!(run.extraction_for(Some("Other Contribution")), base);
    assert_eq!(run.extraction_for(None), base);
    assert_eq!(run.parser_fingerprint.len(), 64);
}

#[test]
fn only_the_fixture_that_supplied_the_output_is_hashed() {
    // an older fixture sitting at the usual path must not be tied to an unrecorded live answer
    let stale = std::env::temp_dir().join(format!("s106-stale-{}.json", std::process::id()));
    std::fs::write(&stale, "{\"output\": {}}").unwrap();
    let live = RunIdentity::new(&serde_json::json!({}), &Default::default(), None);
    assert_eq!(live.fixture_reference, None);
    assert_eq!(live.fixture_sha256, None);
    let replay = RunIdentity::new(&serde_json::json!({}), &Default::default(), Some(&stale));
    assert_eq!(
        replay.fixture_reference.as_deref(),
        Some(stale.display().to_string().as_str())
    );
    assert_eq!(replay.fixture_sha256.as_ref().map(String::len), Some(64));
    std::fs::remove_file(&stale).unwrap();
}
