//! evidence_found(): two-column OCR interleaving re-locates, changed or misplaced evidence does not.
//!
//! The pages are verbatim RapidOCR output from the Southwark sample, so no data download is needed.

use std::sync::LazyLock;

use s106::evidence::{Pages, evidence_found};
use s106::schema::Evidence;

const PAGE_4: &str = r#""Affordable Housing has the meaning given inparagraph 1of Schedule 4;
Pnm MaximumProvision"
"Affordable Housing means the sumof f10,058.60(ten thousand and fifty eightpounds
MonitoringContribution" andsixtypence)Index Linkedtobepaidby theOwner tothe
Council in accordance withparagraph1of Schedule 6and applied
bytheCounciltowardsthecostofmonitoring theprovisionof the
Affordable Housing Units;
"#;

const PAGE_12: &str = r#""Cycle Hire Docking Station means the sumof f12,650(twelve thousand and sixhundred and
Contribution" ftypounds) Index Linked to be paid by the Owner to the Council in
accordancewithparagraph1ofSchedule6andtobegivenby the
Council to Transport for London to enableTransport for London to
provide a Cycle Hire Docking Station within the vicinity of the
Development;
"Default Employment in the means a financial contribution of f73,100 (seventy three thousand
EndUseContribution" one hundred pounds) Index Linked which may be payable by the
"#;

const PAGE_17: &str = r#""Environmental Health means the sumoff10,120(ten thousandandone hundred and
Officer Contribution" twentypounds)Index Linked tobepaid by theOwner totheCouncil
inaccordancewithparagraph1ofSchedule6andappliedby thie
Council towardsthecostofmonitoring theDemolitionEnvironmental
"#;

static PAGES: LazyLock<Pages> = LazyLock::new(|| {
    Pages::from([
        (4, PAGE_4.to_string()),
        (12, PAGE_12.to_string()),
        (17, PAGE_17.to_string()),
    ])
});

fn ev(page: usize, snippet: &str) -> Evidence {
    Evidence {
        page,
        snippet: snippet.to_string(),
    }
}

fn found(page: usize, snippet: &str) -> bool {
    evidence_found(Some(&ev(page, snippet)), &PAGES)
}

fn found_in(page_text: &str, snippet: &str) -> bool {
    evidence_found(
        Some(&ev(1, snippet)),
        &Pages::from([(1, page_text.to_string())]),
    )
}

// ---- InterleavingTest

#[test]
fn left_column_term_spliced_at_line_wrap() {
    assert!(found(
        4,
        "£10,058.60 (ten thousand and fifty eight pounds and sixty pence) Index Linked"
    ));
    assert!(found(
        12,
        "£73,100 (seventy three thousand one hundred pounds) Index Linked"
    ));
    assert!(found(
        17,
        "£10,120 (ten thousand and one hundred and twenty pounds) Index Linked"
    ));
}

#[test]
fn pound_sign_read_as_f() {
    assert!(found(
        4,
        "means the sum of £10,058.60 (ten thousand and fifty eight pounds"
    ));
    assert!(found(
        12,
        "means the sum of £12,650 (twelve thousand and six hundred and"
    ));
}

#[test]
fn one_garbled_plain_word() {
    // OCR reads "the" as "thie"; 16 plain words allow one skip
    assert!(found(
        17,
        "in accordance with paragraph 1 of Schedule 6 and applied by the Council towards the cost of monitoring"
    ));
}

#[test]
fn exact_substring_still_matches() {
    assert!(found(12, "Index Linked to be paid by the Owner"));
}

// ---- StrictnessTest

#[test]
fn changed_figure_fails() {
    assert!(!found(
        12,
        "means the sum of £12,560 (twelve thousand and six hundred and"
    ));
    assert!(!found(
        4,
        "£10,058.50 (ten thousand and fifty eight pounds and sixty pence) Index Linked"
    ));
}

#[test]
fn figure_must_be_digit_bounded() {
    assert!(!found(
        12,
        "means the sum of £2,650 (twelve thousand and six hundred and"
    ));
    assert!(!found(
        12,
        "means the sum of £12,65 (twelve thousand and six hundred and"
    ));
}

#[test]
fn changed_number_word_fails() {
    assert!(!found(
        4,
        "£10,058.60 (ten thousand and fifty eight pounds and seventy pence) Index Linked"
    ));
    assert!(!found(
        17,
        "£10,120 (ten thousand and two hundred and twenty pounds) Index Linked"
    ));
}

#[test]
fn dropped_words_are_not_absorbed_by_a_gap() {
    // the page reads "six hundred and [fifty] pounds"; the snippet leaves out "and fifty"
    assert!(!found(
        12,
        "£12,650 (twelve thousand and six hundred pounds)"
    ));
    assert!(!found(
        17,
        "£10,120 (ten thousand and one hundred pounds) Index Linked"
    ));
}

#[test]
fn joined_number_words_at_line_wrap_are_not_absorbed() {
    // review repro: the snippet leaves out "and twenty", which OCR joined at the line start
    let page =
        "means the sumoff10,120(ten thousand and one hundred\nandtwenty pounds)Index Linked\n";
    assert!(!found_in(
        page,
        "£10,120 (ten thousand and one hundred pounds) Index Linked"
    ));
    let page = "means the sumoff10,010(ten thousand and\nandten pounds)Index Linked\n";
    assert!(!found_in(
        page,
        "£10,010 (ten thousand and pounds) Index Linked"
    ));
}

#[test]
fn short_number_words_joined_to_column_fragment_are_not_absorbed() {
    // review repro: OCR joins the left-column term to "and ten"; the snippet leaves it out
    let snippet = "£10,010 (ten thousand pounds) Index Linked";
    let page = "means the sumoff10,010(ten thousand\nContributionandten pounds)Index Linked\n";
    assert!(!found_in(page, snippet));
    let page = "means the sumoff10,010(ten thousand\nContributionten pounds)Index Linked\n";
    assert!(!found_in(page, snippet));
    // the same split with no amount words re-locates, as do terms with "ten" mid-word
    let snippet = "£10,000 (ten thousand pounds) Index Linked";
    let page =
        "means the sumoff10,000(ten thousand\nMaintenanceContribution\" pounds)Index Linked\n";
    assert!(found_in(page, snippet));
}

#[test]
fn digits_at_line_wrap_are_not_absorbed() {
    let snippet = "means the sum of £12,650 Index Linked to be paid";
    let page = "means the sum of f12,650\nClause 4.2 Index Linked to be paid by the Owner\n";
    assert!(!found_in(page, snippet));
    // the same layout without the figure re-locates
    let page = "means the sum of f12,650\nContribution\" Index Linked to be paid by the Owner\n";
    assert!(found_in(page, snippet));
}

#[test]
fn snippet_from_another_page_fails() {
    assert!(found(
        4,
        "£10,058.60 (ten thousand and fifty eight pounds and sixty pence) Index Linked"
    ));
    assert!(!found(
        12,
        "£10,058.60 (ten thousand and fifty eight pounds and sixty pence) Index Linked"
    ));
    assert!(!found(
        17,
        "£73,100 (seventy three thousand one hundred pounds) Index Linked"
    ));
}

#[test]
fn missing_page_or_evidence() {
    assert!(!found(99, "means the sum of £12,650"));
    assert!(!evidence_found(None, &PAGES));
}

#[test]
fn contiguous_matches_preserve_complete_numbers() {
    for (page, snippet) in [
        ("150", "50"),
        ("500", "50"),
        ("£100", "£1.00"),
        ("£1.00", "£100"),
        ("£10.50 payable", "£1050 payable"),
        ("£10,500 payable", "£10.500 payable"),
    ] {
        assert!(!found_in(page, snippet), "{snippet:?} matched {page:?}");
    }
    assert!(found_in("£1,000.00 payable", "£1000.00 payable"));
    assert!(found_in(
        "50% prior to Occupation",
        "50% prior to Occupation"
    ));
}

#[test]
fn layout_matches_preserve_decimal_positions() {
    let page = "means the sum of f100\nContribution pounds payable to the Council";
    assert!(!found_in(
        page,
        "means the sum of £1.00 pounds payable to the Council"
    ));
    assert!(found_in(
        page,
        "means the sum of £100 pounds payable to the Council"
    ));
}

#[test]
fn numbered_clauses_do_not_absorb_a_following_percentage() {
    for page in [
        "1.1.1. 50% prior to Occupation",
        "1.1.1  50% prior to Occupation",
        "5 3 1 50% prior to Occupation",
        "1.1.1\n50% prior to Occupation",
    ] {
        assert!(found_in(page, "50% prior to Occupation"), "{page}");
    }
    assert!(!found_in(
        "150% prior to Occupation",
        "50% prior to Occupation"
    ));
    assert!(!found_in(
        "£1.50 prior to Occupation",
        "50 prior to Occupation"
    ));
    // With no separator, a clause suffix is ambiguous with a decimal amount.
    assert!(!found_in(
        "9.1.1.50% prior to Occupation",
        "50% prior to Occupation"
    ));
}

#[test]
fn quoted_number_at_line_end_is_set_apart_from_the_next_clause_number() {
    // Punctuation between the two numbers used to drop the line break, joining "18" and "2." into
    // one number, so the digit-boundary check rejected a citation the page states verbatim.
    let page = "replacing it with the number \"18\"\n2. Paragraph 3.2 shall be deleted";
    assert!(found_in(page, "replacing it with the number \"18\""));
    assert!(found_in(
        "as set out in (271)\n3. The Owner",
        "as set out in (271)"
    ));
    // still a different number when the next line's digits are part of it
    assert!(!found_in("the number 182. Paragraph", "the number 18"));
}
