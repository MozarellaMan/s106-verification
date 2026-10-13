//! Constructed controls: expected answers are written independently of the parser.
use rust_decimal::Decimal;
use s106::quantity::{self, Category, Cause, Unit};
use s106::text;

fn d(s: &str) -> Decimal {
    Decimal::from_str_exact(s).unwrap()
}

/// (value, unit) of every quantity found in `text`.
fn found(text: &str) -> Vec<(Decimal, Unit)> {
    quantity::scan(text)
        .quantities
        .into_iter()
        .map(|q| (q.value, q.unit))
        .collect()
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
            Decimal::from(quantity::integer(source).unwrap()),
            d(expected),
            "{source}"
        );
        for corrupt in [
            format!("{source} garbage"),
            format!("{source}9"),
            format!("{source}#"),
            format!("{source} and and one"),
        ] {
            assert!(quantity::integer(&corrupt).is_err(), "{corrupt}");
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
        assert!(quantity::integer(bad).is_err(), "{bad}");
    }
    assert_eq!(
        quantity::integer("").unwrap_err().category(),
        Category::Absent
    );
    assert_eq!(
        quantity::integer("hello").unwrap_err().cause,
        Cause::UnsupportedWord
    );
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
    ] {
        assert_eq!(quantity::amount(source).unwrap(), d(expected));
        assert!(quantity::amount(&format!("{source} banana")).is_err());
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
        // pence are below one hundred; this used to read as 11 pounds
        "ten pounds and one hundred pence",
    ] {
        assert!(quantity::amount(bad).is_err(), "{bad}");
    }
    assert_eq!(
        quantity::amount("ten pounds and").unwrap_err().cause,
        Cause::IncompleteExpression
    );
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
        assert!(quantity::decimal(good).is_ok(), "{good}");
    }
    assert_eq!(quantity::decimal("£-1,234.50").unwrap(), d("-1234.50"));
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
        "50%",
        "banana",
        "1,0",
    ] {
        assert!(quantity::decimal(bad).is_err(), "{bad}");
    }
    assert_eq!(
        quantity::decimal("  ").unwrap_err().category(),
        Category::Absent
    );
    assert_eq!(
        quantity::decimal("79228162514264337593543950336")
            .unwrap_err()
            .category(),
        Category::NumericRange
    );
    for bad in ["50.0.1%", "--50%", ".50%", "1,23%", "1234,567%"] {
        assert!(!text::numbers_in(bad).contains(&d("50")), "{bad}");
        assert!(!text::percentage_in_snippet(d("50"), bad), "{bad}");
    }
    // a hyphen followed by a space (or NBSP) is a bullet, not a sign
    for bullet in ["- 50%", "-\u{a0}50%"] {
        assert!(text::numbers_in(bullet).contains(&d("50")), "{bullet:?}");
        assert!(text::percentage_in_snippet(d("50"), bullet), "{bullet:?}");
    }
    assert!(!text::numbers_in("-50%").contains(&d("50")));
    assert!(text::numbers_in("-50%").contains(&d("-50")));
    assert!(!text::numbers_in("1.2.3").contains(&d("1.2")));
    assert!(!text::numbers_in("−50%").contains(&d("50")));
    assert!(text::numbers_in("−50%").contains(&d("-50")));
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
fn scan_controls() {
    let count = |n: &str| (d(n), Unit::Count);
    assert_eq!(found("3-5 dwellings"), [count("3"), count("5")]);
    assert_eq!(found("3–5 dwellings"), [count("3"), count("5")]);
    assert_eq!(found("50%."), [(d("50"), Unit::Percent)]);
    let per_annum = quantity::scan("50 per annum");
    assert_eq!(
        per_annum
            .quantities
            .iter()
            .map(|q| (q.value, q.unit))
            .collect::<Vec<_>>(),
        [count("50")]
    );
    assert!(per_annum.errors.is_empty());
    assert_eq!(found("fifty thousand, payable"), [count("50000")]);
    assert_eq!(found("£10,058.60"), [(d("10058.60"), Unit::Gbp)]);
    assert_eq!(
        found("ten thousand and fifty eight pounds and sixty pence"),
        [(d("10058.60"), Unit::Gbp)]
    );
    assert_eq!(found("fifty per cent"), [(d("50"), Unit::Percent)]);
    assert_eq!(found("the 50th Dwelling"), [count("50")]);
    assert_eq!(found("f10,058.60"), [(d("10058.60"), Unit::Gbp)]);
    assert_eq!(found("1st"), [count("1")]);
}

#[test]
fn original_utf8_spans_and_ocr_uncertainty() {
    let source = "Café — sum 1,23 payable";
    let scan = quantity::scan(source);
    assert!(scan.quantities.is_empty());
    let error = &scan.errors[0];
    let at = source.find("1,23").unwrap();
    assert_eq!(&source[error.candidate.clone()], "1,23");
    assert_eq!(error.candidate, at..at + 4);
    assert_eq!(error.span, at + 2..at + 3);
    assert_eq!(&source[error.span.clone()], "2");
    assert_eq!(error.cause, Cause::UnexpectedToken);

    let repaired = quantity::scan("5o% of the Development");
    assert!(repaired.quantities.is_empty());
    let [error] = repaired.errors.as_slice() else {
        panic!("{:?}", repaired.errors)
    };
    assert_eq!(error.category(), Category::AmbiguousSource);
    assert_eq!(error.candidate, 0..2); // per token, not the whole snippet
    assert_eq!(
        error.cause,
        Cause::OcrSubstitutionCandidate {
            normalised: "50".into()
        }
    );
    let mixed = quantity::scan("see fif5ty and paragraph1of");
    assert_eq!(mixed.errors.len(), 2);
    assert!(
        mixed
            .errors
            .iter()
            .all(|e| e.cause == Cause::MixedLettersAndDigits)
    );
}

// ---- exhaustive round trip against an independent generator

fn ones(n: u64) -> &'static str {
    [
        "zero",
        "one",
        "two",
        "three",
        "four",
        "five",
        "six",
        "seven",
        "eight",
        "nine",
        "ten",
        "eleven",
        "twelve",
        "thirteen",
        "fourteen",
        "fifteen",
        "sixteen",
        "seventeen",
        "eighteen",
        "nineteen",
    ][n as usize]
}

/// 1..=99. `hyphen` joins tens and units ("twenty-five").
fn tens_units(n: u64, hyphen: bool) -> String {
    if n < 20 {
        return ones(n).into();
    }
    let tens = [
        "twenty", "thirty", "forty", "fifty", "sixty", "seventy", "eighty", "ninety",
    ][(n / 10 - 2) as usize];
    match (n % 10, hyphen) {
        (0, _) => tens.into(),
        (u, true) => format!("{tens}-{}", ones(u)),
        (u, false) => format!("{tens} {}", ones(u)),
    }
}

/// 1..=999. `and` is the British "one hundred and five".
fn hundreds(n: u64, and: bool, hyphen: bool) -> String {
    match (n / 100, n % 100) {
        (0, r) => tens_units(r, hyphen),
        (h, 0) => format!("{} hundred", ones(h)),
        (h, r) => format!(
            "{} hundred {}{}",
            ones(h),
            if and { "and " } else { "" },
            tens_units(r, hyphen)
        ),
    }
}

/// British style: "two thousand and one", "one million, two hundred thousand".
fn words_with(n: u64, and: bool, hyphen: bool) -> String {
    if n == 0 {
        return "zero".into();
    }
    let (m, t, r) = (n / 1_000_000, n / 1_000 % 1_000, n % 1_000);
    let mut out = String::new();
    if m > 0 {
        out += &format!("{} million", hundreds(m, and, hyphen));
    }
    if t > 0 {
        if m > 0 {
            out += ", ";
        }
        out += &format!("{} thousand", hundreds(t, and, hyphen));
    }
    if r > 0 {
        if m + t > 0 {
            // "and" only before a final group of fewer than a hundred
            out += if r < 100 && and { " and " } else { ", " };
        }
        out += &hundreds(r, and, hyphen);
    }
    out
}

fn words(n: u64) -> String {
    words_with(n, true, false)
}

#[test]
fn every_cardinal_up_to_a_million_round_trips() {
    // spread over the cores to keep the debug-build runtime down
    let threads = 8u64;
    std::thread::scope(|scope| {
        for t in 0..threads {
            scope.spawn(move || {
                for n in (t..=1_000_000).step_by(threads as usize) {
                    let w = words(n);
                    assert_eq!(quantity::integer(&w), Ok(n), "{w}");
                }
            });
        }
    });
}

#[test]
fn strided_larger_cardinals_and_variants_round_trip() {
    let mut checked = 0;
    for n in (0..1_000_000_000u64).step_by(7_919) {
        let w = words(n);
        assert_eq!(quantity::integer(&w), Ok(n), "{w}");
        checked += 1;
    }
    // variants on a sparser stride: no "and", hyphenated tens, and OCR run-togethers
    for n in (0..1_000_000_000u64).step_by(7_919 * 16) {
        for (and, hyphen) in [(false, false), (true, true), (false, true)] {
            let w = words_with(n, and, hyphen);
            assert_eq!(quantity::integer(&w), Ok(n), "{w}");
        }
        let joined: String = words(n).chars().filter(|c| c.is_alphabetic()).collect();
        assert_eq!(quantity::integer(&joined), Ok(n), "{joined}");
    }
    assert!(checked > 100_000);
    // the amount path shares the cardinal
    for n in (0..1_000_000u64).step_by(997) {
        let pence = n % 99 + 1;
        let w = format!("{} pounds and {} pence", words(n), tens_units(pence, false));
        assert_eq!(
            quantity::amount(&w),
            Ok(Decimal::from(n) + Decimal::new(pence as i64, 2)),
            "{w}"
        );
    }
}

#[test]
fn mutations_of_sampled_cardinals_fail_or_change_the_value() {
    let mut mutated = 0;
    let sample = (0..1_000_000u64)
        .step_by(10_007)
        .chain((1_000_000..1_000_000_000).step_by(7_919_011));
    for n in sample {
        let w = words(n);
        let tokens: Vec<&str> = w.split(' ').collect();
        let differs = |m: String| {
            assert_ne!(quantity::integer(&m), Ok(n), "{w:?} -> {m:?}");
        };
        differs(format!("{w} and"));
        differs(format!("{w} 7"));
        differs(format!("{w}7"));
        differs(format!("7 {w}"));
        for at in 0..tokens.len() {
            // "and" is optional filler ("two thousand seven" is read as 2007), so dropping it
            // may legitimately keep the value
            if tokens[at] != "and" {
                let mut dropped = tokens.clone();
                dropped.remove(at);
                differs(dropped.join(" "));
            }
            let mut repeated = tokens.clone();
            repeated.insert(at, tokens[at]);
            differs(repeated.join(" "));
            let mut digit = tokens.clone();
            digit.insert(at, "7");
            differs(digit.join(" "));
            mutated += 3;
        }
    }
    assert!(mutated > 1_000);
}

#[test]
fn review_regressions_from_pr13() {
    // exact fields carry no prose suffix
    for bad in ["50th", "£50%", "50%", "50 per cent", "50percent"] {
        assert!(quantity::decimal(bad).is_err(), "{bad}");
    }
    assert!(found("£50% of the sum").is_empty());
    // a conjunction after a completed quantity is prose
    assert_eq!(
        found("fifty per cent and payable on commencement"),
        vec![(d("50"), Unit::Percent)]
    );
    assert_eq!(
        found("the sum of ten pounds and payable"),
        vec![(d("10"), Unit::Gbp)]
    );
    assert!(found("one hundred and").is_empty());
    assert!(quantity::amount("ten pounds and").is_err());
    assert!(quantity::integer("one hundred and").is_err());
    // percentage suffixes joined to digits are readable; corrupt runs are not
    for good in ["50percent", "50per cent", "50percent of"] {
        assert_eq!(found(good), vec![(d("50"), Unit::Percent)], "{good}");
    }
    assert!(found("50percentbanana").is_empty());
    // a percent sign after number words
    for good in ["fifty%", "fifty %", "fifty% of the sum"] {
        assert_eq!(found(good), vec![(d("50"), Unit::Percent)], "{good}");
    }
    assert!(found("fifty%%").is_empty());
}

#[test]
fn second_review_regressions_from_pr13() {
    // an attempted pence clause keeps the amount uncertain
    assert!(found("ten pounds and unknown pence").is_empty());
    assert!(
        !quantity::scan("ten pounds and unknown pence")
            .errors
            .is_empty()
    );
    assert!(found("ten pounds and fifty").is_empty());
    // OCR-joined conjunctions remove only the "and"
    assert_eq!(
        found("fiftypercentand payable"),
        vec![(d("50"), Unit::Percent)]
    );
    assert_eq!(
        found("one hundredand payable"),
        vec![(d("100"), Unit::Count)]
    );
    assert_eq!(
        found("one hundred and the Owner"),
        vec![(d("100"), Unit::Count)]
    );
    // a conjunction between completed quantities splits them
    assert_eq!(
        found("fifty per cent and sixty per cent respectively"),
        vec![(d("50"), Unit::Percent), (d("60"), Unit::Percent)]
    );
    // pence alone are not an amount in pounds, so only the first quantity reads
    assert_eq!(
        found("ten pounds and twenty pence and five pence"),
        vec![(d("10.20"), Unit::Gbp)]
    );
    // a sign before the currency symbol, and £ ranges
    assert_eq!(quantity::decimal("-£1,234.50").unwrap(), d("-1234.50"));
    assert_eq!(quantity::decimal("+£5").unwrap(), d("5"));
    assert!(quantity::decimal("--£5").is_err());
    assert_eq!(
        found("£10,000-£20,000"),
        vec![(d("10000"), Unit::Gbp), (d("20000"), Unit::Gbp)]
    );
    assert_eq!(
        found("£10,000–£20,000"),
        vec![(d("10000"), Unit::Gbp), (d("20000"), Unit::Gbp)]
    );
}

#[test]
fn third_review_regressions_from_pr13() {
    // scanning resumes at a quantity OCR-joined to the conjunction
    assert_eq!(
        found("fifty per cent andsixty per cent respectively"),
        vec![(d("50"), Unit::Percent), (d("60"), Unit::Percent)]
    );
    // a later, unrelated pence clause does not hold back a quantity that cannot take pence
    assert_eq!(
        found("one hundred and the sum of ten pounds and twenty pence"),
        vec![(d("100"), Unit::Count), (d("10.20"), Unit::Gbp)]
    );
    // separately stated whole-pound amounts split; incomplete pence stay rejected
    assert_eq!(
        found("ten pounds and twenty pounds respectively"),
        vec![(d("10"), Unit::Gbp), (d("20"), Unit::Gbp)]
    );
    assert!(found("ten pounds and unknown pence").is_empty());
    assert!(found("ten pounds and fifty").is_empty());
}

#[test]
fn misspelt_pence_clauses_are_not_cut_at_and() {
    for bad in [
        "the sum of two hundred and fourty pence",
        "one hundred and ninty pence",
        "fifty and illegible pence",
    ] {
        assert!(found(bad).is_empty(), "{bad}");
        assert!(!quantity::scan(bad).errors.is_empty(), "{bad}");
    }
    assert_eq!(
        found("ten pounds and twenty pounds and thirty pounds"),
        vec![
            (d("10"), Unit::Gbp),
            (d("20"), Unit::Gbp),
            (d("30"), Unit::Gbp)
        ]
    );
}

#[test]
fn pound_amount_then_prose_then_pounds_and_pence() {
    assert_eq!(
        found("ten pounds and the sum of five pounds and twenty pence"),
        vec![(d("10"), Unit::Gbp), (d("5.20"), Unit::Gbp)]
    );
}
