//! Trigger labels are decided by the cited phrase.
//!
//! Phrases are the ones Claude Haiku 5.5 cited across live runs on the Southwark sample,
//! including the runs whose labels drifted.

use s106::schema::TriggerEvent as E;
use s106::triggers::{classify, imposes_payment};

const CLAUSE_3_5: &str = "The Carbon Green Fund Contribution (as agreed between the Council and the Owner or determined in \
accordance with clause 19 of this Agreement) shall be payable within 20 Working Days of such \
agreement or determination.";
const CLAUSE_3_1: &str = "Prior to the Occupation of 50% of the Development (or upon such other date as may be agreed between \
the Council and the Owner) the Council and the Owner shall compare the environmental performance of \
the Development against the Agreed Carbon Targets";

// ---- ClassifyTest

#[test]
fn classify_cited_phrases() {
    let cases = [
        ("within 28 days of receiving written request", E::OnDemand),
        (
            "the Owner will pay the Default Employment in the End Use Contribution within 28 days",
            E::OnDemand,
        ),
        (
            "shall be payable within 20 Working Days of such agreement or determination.",
            E::OnDemand,
        ),
        (
            "To pay the Site and Development Contributions on or before Implementation.",
            E::PreImplementation,
        ),
        (
            "not to Implement the Development until the Green Run-off Rate Off-Set",
            E::PreImplementation,
        ),
        (
            "to pay the Bus Contribution on or before first Occupation of any part of the Development",
            E::PreOccupation,
        ),
        (
            "The Owner shall not Occupy or permit Occupation of more than 50 per cent of the Market",
            E::PreOccupation,
        ),
        (
            "To pay the Carbon Green Fund Contribution in accordance with the procedure outlined in paragraph 3",
            E::Other,
        ),
        (
            "prior to Commencement of the Development",
            E::PreCommencement,
        ),
        ("on completion of this Agreement", E::OnCompletion),
        ("on the date hereof", E::OnCompletion),
    ];
    for (phrase, event) in cases {
        assert_eq!(classify(Some(phrase)), Some(event), "{phrase}");
    }
}

#[test]
fn classify_ocr_spacing_does_not_change_the_label() {
    assert_eq!(
        classify(Some(
            "TopaytheCycleHireDockingStationContributiononorbeforeOccupation."
        )),
        Some(E::PreOccupation)
    );
    assert_eq!(
        classify(Some("within twenty eight days of demand")),
        Some(E::OnDemand)
    );
}

#[test]
fn classify_earliest_phrase_wins() {
    assert_eq!(
        classify(Some(
            "Not to Occupy until paid, and to pay within 28 days of demand"
        )),
        Some(E::PreOccupation)
    );
}

#[test]
fn classify_unrecognised() {
    assert_eq!(classify(Some("as the Council may direct")), None);
    // Review repro: construction completion is not completion of the agreement.
    assert_eq!(classify(Some("on completion of construction")), None);
    assert_eq!(classify(Some("upon completion of the Development")), None);
    assert_eq!(classify(None), None);
    assert_eq!(classify(Some("")), None);
}

// ---- ImposesPaymentTest

#[test]
fn imposes_payment_obligations() {
    let clauses = [
        "To pay the Site and Development Contributions on or before Implementation.",
        "1.3 TopaytheCycleHireDockingStationContributiononorbeforeOccupation.",
        "Unless a lesser amount is specified in writing, to pay the Bus Contribution on or before first Occupation",
        "The Owner covenants with the Council to pay the Administration Cost within 28 days",
        "the Owner will pay the Default Employment in the End Use Contribution within 28 days",
        CLAUSE_3_5,
        "Not to Implement the Development until the Site and Development Contributions have been paid to the \
         Council in full.",
        "The Council shall assess the Contribution. The Contribution is payable within 28 days of demand.",
        "The sum (if any) is payable within 28 days of the Assessment.",
        // "if" inside words is not a condition
        "The specified sum is payable within 28 days of the date notified to the Owner",
        "Lift Contribution is payable on or before Occupation",
    ];
    for clause in clauses {
        assert!(imposes_payment(Some(clause)), "{clause}");
    }
}

#[test]
fn imposes_payment_mentions_are_not_obligations() {
    // Review repro: mentioning a payable amount is not imposing payment.
    let clauses = [
        "Prior to Occupation the Council shall assess the amount payable",
        // Re-review repro: "is payable" inside a question is not an obligation.
        "Prior to Occupation the Council shall assess whether the Carbon Green Fund Contribution is payable.",
        "PriortoOccupationtheCouncilshalldeterminewhethertheContributionispayable",
        // Re-review repro: the same "assess if" question, with and without OCR spacing.
        "Prior to Occupation the Council shall assess if the Contribution is payable",
        "PriortoOccupationtheCouncilshallassessiftheContributionispayable",
        "the Council will check if the Contribution shall be payable",
        "3.2 IftheViabilityReviewshowsasurplustheContributionispayable",
        "(a) If the Viability Review shows a surplus the Contribution is payable",
        "If the Viability Review shows a surplus the Affordable Housing Contribution is payable",
        "the Council shall calculate the sum to be paid by the Owner",
        CLAUSE_3_1,
    ];
    for clause in clauses {
        assert!(!imposes_payment(Some(clause)), "{clause}");
    }
}

#[test]
fn imposes_payment_none_or_empty_is_false() {
    assert!(!imposes_payment(None));
    assert!(!imposes_payment(Some("")));
}
