//! Constructed controls: expected answers are written independently of the parser.
use rust_decimal::Decimal;
use s106::{
    numeric::{self, Category, Cause},
    text,
};
fn d(s: &str) -> Decimal {
    Decimal::from_str_exact(s).unwrap()
}

#[test]
fn clean_words_and_corrupted_twins() {
    for (source, expected) in [
        ("FIFTYEIGHT", "58"),
        ("twenty-five", "25"),
        (
            "five million and three hundred and thirty six thousand and five hundred",
            "5336500",
        ),
        (
            "one million, six hundred and eleven thousand and five hundred and two",
            "1611502",
        ),
        (
            "six hundred and eighty three thousand and one hundred",
            "683100",
        ),
    ] {
        assert_eq!(
            Decimal::from(numeric::integer(source).unwrap()),
            d(expected),
            "{source}"
        );
        for corrupt in [
            format!("{source} garbage"),
            format!("{source}9"),
            format!("{source}#"),
            format!("{source} and and one"),
        ] {
            assert!(numeric::integer(&corrupt).is_err(), "{corrupt}");
        }
    }
    for bad in [
        "twenty#five",
        "twenty,five",
        "fif5ty",
        "twenty/five",
        "-one",
        "one,,two",
        "one one",
        "one thousand two million",
        "and",
        "one hundred and",
        "ninety hundred hundred hundred hundred hundred hundred hundred hundred hundred hundred",
    ] {
        assert!(numeric::integer(bad).is_err(), "{bad}");
    }
    assert_eq!(numeric::integer("").unwrap_err().category, Category::Absent);
}
#[test]
fn complete_money_suffix_without_prototype_domain_policies() {
    for (source, expected) in [
        ("ten poundsandtwentypence", "10.20"),
        (
            "sixteen thousand nine hundred and thirteen pounds and twenty six pence",
            "16913.26",
        ),
        ("ten pounds and zero pence", "10"),
        ("ten pounds one penny", "10.01"),
        ("ten pounds and one hundred pence", "11"),
    ] {
        assert_eq!(numeric::amount(source).unwrap(), d(expected));
        assert!(numeric::amount(&format!("{source} banana")).is_err());
    }
    for bad in [
        "ten pounds and",
        "ten pounds and pence",
        "ten pounds and unknown pence",
        "ten pounds and twenty",
        "ten pounds 20 pence",
        "ten pounds!",
        "ten pounds and twenty pence and",
        "ten pounds and and twenty pence",
    ] {
        assert!(numeric::amount(bad).is_err(), "{bad}");
    }
}
#[test]
fn exact_decimal_and_candidate_boundaries() {
    for good in [
        "1,234,567.123",
        "£-1,234.50",
        "£ 1,234.50",
        "+50.1234",
        "79228162514264337593543950335",
        "0.0000000000000000000000000001",
    ] {
        assert!(numeric::decimal(good).is_ok(), "{good}");
    }
    for bad in [
        "1,23",
        "1234,567",
        "1,,234",
        "1.2.3",
        "--50",
        "- 50",
        ".50",
        "1 234",
        "1e3",
        "79228162514264337593543950336",
        "0.00000000000000000000000000001",
    ] {
        assert!(numeric::decimal(bad).is_err(), "{bad}");
    }
    for bad in ["50.0.1%", "--50%", "- 50%", ".50%", "1,23%", "1234,567%"] {
        assert!(!text::numbers_in(bad).contains(&d("50")), "{bad}");
        assert!(!text::percentage_in_snippet(d("50"), bad), "{bad}");
    }
    assert!(!text::numbers_in("-50%").contains(&d("50")));
    assert!(text::numbers_in("-50%").contains(&d("-50")));
    assert!(!text::numbers_in("1.2.3").contains(&d("1.2")));
    assert!(!text::numbers_in("−50%").contains(&d("50")));
    assert!(!text::numbers_in("-\u{a0}50%").contains(&d("50")));
    for corrupt in [
        "fif5ty",
        "twenty#five",
        "twenty,five",
        "one thousand5",
        "one hundred and",
        "twenty-",
    ] {
        assert!(text::numbers_in(corrupt).is_empty(), "{corrupt}");
    }
    assert!(text::numbers_in("one million, six hundred and two pounds").contains(&d("1000602")));
    assert!(!text::numbers_in("1234,567").contains(&d("567")));
}
#[test]
fn percentage_and_ordinal_controls() {
    for good in [
        "50%of the sum",
        "fiftypercent",
        "fifty per cent",
        "50 per cent",
        "50.1234%",
    ] {
        let value = if good.starts_with("50.1234") {
            d("50.1234")
        } else {
            d("50")
        };
        assert!(text::percentage_in_snippet(value, good), "{good}");
    }
    for bad in [
        "50th Dwelling",
        "50 per cento",
        "50percentbanana",
        "50%%",
        "50 per",
        "50.1.2%",
        "5o%",
    ] {
        assert!(!text::percentage_in_snippet(d("50"), bad), "{bad}");
    }
}
#[test]
fn original_utf8_spans_and_ocr_uncertainty() {
    let source = "Café — sum 1,23 payable";
    let located = text::located_numbers(source);
    assert!(located.values.is_empty());
    let error = &located.diagnostics[0];
    assert_eq!(error.observed_input.as_deref(), Some("1,23"));
    assert_eq!(
        error.span.as_ref().unwrap().coordinate_space,
        "original_excerpt"
    );
    assert_eq!(
        error.span.as_ref().unwrap().range,
        source.find("1,23").unwrap() + 2..source.find("1,23").unwrap() + 3
    );
    assert_eq!(&source[error.span.as_ref().unwrap().range.clone()], "2");
    assert_eq!(
        error.normalised_span.as_ref().unwrap().coordinate_space,
        "ascii_folded_observed_input"
    );
    let repaired = text::located_numbers("5o% of the Development");
    assert!(!repaired.values.contains(&d("50")));
    let error = repaired
        .diagnostics
        .iter()
        .find(|e| e.category == Category::AmbiguousSource)
        .unwrap();
    assert!(matches!(
        error.cause,
        Cause::OcrSubstitutionCandidate { .. }
    ));
    assert!(error.normalised_span.is_none());
}

#[test]
fn recorded_words_replay_with_negative_twins() {
    // Recorded figures are compatibility references, not independently verified legal truth.
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut checked = 0;
    for version in ["llm", "llm-v1"] {
        for deed in std::fs::read_dir(root.join("fixtures").join(version)).unwrap() {
            for fixture in std::fs::read_dir(deed.unwrap().path()).unwrap() {
                let path = fixture.unwrap().path();
                let raw: serde_json::Value =
                    serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
                for c in raw["output"]["contributions"].as_array().unwrap() {
                    let Some(words) = c["amount_in_words"].as_str().filter(|s| !s.is_empty())
                    else {
                        continue;
                    };
                    let figure =
                        Decimal::from_str_exact(c["amount_gbp"].as_str().unwrap()).unwrap();
                    assert_eq!(
                        numeric::amount(words).unwrap(),
                        figure,
                        "{}: {words}",
                        path.display()
                    );
                    assert!(
                        numeric::amount(&format!("{words} banana")).is_err(),
                        "{}: corrupted {words}",
                        path.display()
                    );
                    checked += 1;
                }
            }
        }
    }
    assert_eq!(
        checked, 70,
        "recorded corpus denominator changed; investigate explicitly"
    );
    eprintln!("{checked}/70 recorded words fields reparsed; {checked}/70 corrupted twins rejected");
}
