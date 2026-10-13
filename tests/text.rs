//! Number and text helpers: OCR digit repair, number words, `value_in_snippet`.
//!
//! Ports of `OcrDigitsTest` (former Python test_triggers, see git history) and `ValueInSnippetTest` (former Python test_grounding).

use rust_decimal::Decimal;
use s106::text::{
    Value, numbers_in, ocr_digits, squash, value_in_snippet, words_to_amount, words_to_int,
};

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
    assert!(value_in_snippet(
        &50u32.into(),
        "Prior to the Occupation of 5o%of theDevelopment"
    )); // OCR o for 0
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
        words_to_int("sixteen thousand nine hundred and thirteen"),
        Some(16913)
    );
    assert_eq!(words_to_int("fiftyeight"), Some(58)); // OCR run-togethers
    assert_eq!(words_to_int("two million"), Some(2_000_000));
    assert_eq!(words_to_int("hello"), None);
    assert_eq!(words_to_int(""), None);
    assert_eq!(words_to_int("and"), None);
}

#[test]
fn words_to_amount_reads_pounds_and_pence() {
    assert_eq!(
        words_to_amount("ten thousand and fifty eight pounds and sixty pence"),
        Some(dec("10058.60"))
    );
    assert_eq!(
        words_to_amount("twelve thousand pounds"),
        Some(dec("12000"))
    );
    assert_eq!(
        words_to_amount("seventy three thousand one hundred pounds and"),
        Some(dec("73100"))
    );
    assert_eq!(words_to_amount("sixty pence"), None);
    assert_eq!(words_to_amount("pounds"), None);
}

#[test]
fn numbers_in_reads_digits_and_words() {
    let n = numbers_in("£10,058.60 (ten thousand and fifty eight pounds) and 5o%");
    assert!(n.contains(&dec("10058.6")));
    assert!(n.contains(&Decimal::from(10058)));
    assert!(n.contains(&Decimal::from(50)));
    // a thousands separator is not a decimal point, and a bare "1234,567" splits
    assert!(numbers_in("1,234,567").contains(&Decimal::from(1_234_567)));
    assert!(numbers_in("one hundred and").contains(&Decimal::from(100)));
    assert!(numbers_in("twenty-five days").contains(&Decimal::from(25)));
}
