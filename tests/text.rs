//! Number and text helpers: OCR digit repair, number words, `value_in_snippet`.
//!
//! Ports of `OcrDigitsTest` (former Python test_triggers, see git history) and `ValueInSnippetTest` (former Python test_grounding).

use rust_decimal::Decimal;
use s106::quantity;
use s106::text::{Value, numbers_in, ocr_digits, squash, value_in_snippet};

fn dec(s: &str) -> Decimal {
    s.parse().unwrap()
}

fn num(s: &str) -> Value {
    Value::Num(dec(s))
}

// ---- OcrDigitsTest

#[test]
fn ocr_digits_letters_inside_numbers() {
    assert_eq!(ocr_digits("of 5o%of the"), "of 50%of the");
    assert_eq!(ocr_digits("sum of f1o,058"), "sum of f10,058");
    assert_eq!(ocr_digits("l0 days"), "10 days");
}

#[test]
fn ocr_digits_letters_next_to_words_untouched() {
    assert_eq!(
        ocr_digits("withparagraph1of Schedule"),
        "withparagraph1of Schedule"
    );
    assert_eq!(ocr_digits("Block1 and Lot2"), "Block1 and Lot2");
}

// ---- ValueInSnippetTest

#[test]
fn value_in_snippet_digit_forms() {
    assert!(value_in_snippet(
        &num("10058.60"),
        "the sum of £10,058.60 (ten thousand"
    ));
    assert!(value_in_snippet(&num("10058.6"), "the sum of f10,058.60")); // OCR reads £ as f
    assert!(value_in_snippet(
        &num("35.75"),
        "35.75%affordablebyhabitable room"
    ));
    // OCR "o" for "0" is reported as ambiguous (see tests/numeric.rs), never silently repaired
    assert!(!value_in_snippet(
        &50u32.into(),
        "Prior to the Occupation of 5o%of theDevelopment"
    ));
    assert!(value_in_snippet(
        &212u32.into(),
        "three bedroom units (212) Habitable Rooms"
    ));
}

#[test]
fn value_in_snippet_word_forms() {
    assert!(value_in_snippet(
        &76u32.into(),
        "seventy six affordable housing units"
    ));
    assert!(
        numbers_in("six hundred and eighty three thousand and one hundred pounds")
            .contains(&Decimal::from(683100))
    );
}

#[test]
fn value_in_snippet_absent_or_wrong_value() {
    assert!(!value_in_snippet(
        &75u32.into(),
        "the 76 Residential Units (271 Habitable Rooms)"
    ));
    assert!(!value_in_snippet(&num("10058.00"), "the sum of £10,058.60"));
    assert!(!value_in_snippet(&7u32.into(), "the 76 Residential Units")); // digits inside a number don't count
}

#[test]
fn value_in_snippet_text_values() {
    assert!(value_in_snippet(
        &"ten thousand and fifty eight pounds".into(),
        "(tenthousandandfiftyeightpounds"
    ));
    assert!(!value_in_snippet(&"".into(), "anything"));
    assert!(!value_in_snippet(&"£ !".into(), "anything"));
}

// ---- helpers the above rest on

#[test]
fn squash_keeps_lowercase_ascii_alphanumerics() {
    assert_eq!(
        squash("On or Before, Occupation (1.3)!"),
        "onorbeforeoccupation13"
    );
    assert_eq!(squash("Café £5"), "caf5");
}

#[test]
fn words_to_int_reads_number_words() {
    assert_eq!(
        quantity::integer("sixteen thousand nine hundred and thirteen").ok(),
        Some(16913)
    );
    assert_eq!(quantity::integer("fiftyeight").ok(), Some(58)); // OCR run-togethers
    assert_eq!(quantity::integer("two million").ok(), Some(2_000_000));
    assert_eq!(quantity::integer("hello").ok(), None);
    assert_eq!(quantity::integer("").ok(), None);
    assert_eq!(quantity::integer("and").ok(), None);
}

#[test]
fn words_to_amount_reads_pounds_and_pence() {
    assert_eq!(
        quantity::amount("ten thousand and fifty eight pounds and sixty pence").ok(),
        Some(dec("10058.60"))
    );
    assert_eq!(
        quantity::amount("twelve thousand pounds").ok(),
        Some(dec("12000"))
    );
    // a dangling "and" is an incomplete expression, not a whole number of pounds
    assert_eq!(
        quantity::amount("seventy three thousand one hundred pounds and").ok(),
        None
    );
    assert_eq!(quantity::amount("sixty pence").ok(), None);
    assert_eq!(quantity::amount("pounds").ok(), None);
}

#[test]
fn numbers_in_reads_digits_and_words() {
    let n = numbers_in("£10,058.60 (ten thousand and fifty eight pounds) and 5o%");
    assert!(n.contains(&dec("10058.6")));
    assert!(n.contains(&Decimal::from(10058)));
    assert!(!n.contains(&Decimal::from(50))); // "5o" is reported, not repaired
    // a thousands separator is not a decimal point, and a bare "1234,567" splits
    assert!(numbers_in("1,234,567").contains(&Decimal::from(1_234_567)));
    assert!(numbers_in("one hundred and").is_empty()); // dangling "and": incomplete, no value
    assert!(numbers_in("twenty-five days").contains(&Decimal::from(25)));
}

#[test]
fn unknown_pence_is_not_zero() {
    for words in [
        "ten pounds and illegible pence",
        "ten pounds and pence",
        "ten pounds banana",
    ] {
        assert_eq!(quantity::amount(words).ok(), None, "{words}");
    }
    assert_eq!(
        quantity::amount("ten pounds and zero pence").ok(),
        Some(dec("10"))
    );
    assert_eq!(
        quantity::amount("ten pounds and one penny").ok(),
        Some(dec("10.01"))
    );
    assert_eq!(
        quantity::amount("ten poundsandtwentypence").ok(),
        Some(dec("10.20"))
    );
}

#[test]
fn overflowing_number_words_are_not_clamped() {
    assert_eq!(
        quantity::integer(
            "ninety hundred hundred hundred hundred hundred hundred hundred hundred hundred hundred"
        )
        .ok(),
        None
    );
}

#[test]
fn percentages_require_a_percentage_suffix() {
    use s106::text::percentage_in_snippet;
    for text in [
        "50% prior to Occupation",
        "50 per cent",
        "fifty per cent",
        "fiftypercent",
    ] {
        assert!(percentage_in_snippet(dec("50"), text), "{text}");
    }
    for text in [
        "50th Dwelling",
        "150%",
        "50 pounds",
        "5.0%",
        "5o%of the sum", // OCR o for 0 is reported, not repaired
        "fifteen per cent",
    ] {
        assert!(!percentage_in_snippet(dec("50"), text), "{text}");
    }
}
