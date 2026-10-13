//! Synthetic controls fixed in validation/southwark-19-AP-1773/protocol.md.
//! These characterise the existing verifier; passing limitation tests do NOT mean the
//! mutation was detected. No model output or OCR benchmark supplies the expected truth.
use s106::{
    evidence::Pages,
    schema::*,
    verify::{self, Status},
};

fn ev(page: usize, snippet: &str) -> Evidence {
    Evidence {
        page,
        snippet: snippet.into(),
    }
}
const CLAUSE: &str = "If the jobs target is missed the Owner shall pay the Training Contribution within 28 days of receiving written notice from the Council.";
fn plant() -> (Agreement, Pages) {
    let pg = Pages::from([
        (1, "Training Contribution means a sum of up to a maximum £100 (one hundred pounds).".into()),
        (2, CLAUSE.into()),
        (3, "Development means 100 residential units. Market Housing Units means 75 units. Affordable Housing Units means 25 units (50 Habitable Rooms). Social Rent means 25 units (50 Habitable Rooms). The Development is 25% affordable by habitable room with 200 Habitable Rooms in total.".into()),
    ]);
    let c = Contribution {
        name: "Training Contribution".into(),
        amount: Amount {
            structure: AmountStructure::Cap,
            value_gbp: Some("100".parse().unwrap()),
            words: Some("one hundred pounds".into()),
            index_linked: false,
            evidence: Some(ev(1, "a sum of up to a maximum £100 (one hundred pounds)")),
        },
        conditionality: Conditionality {
            kind: ConditionalityKind::Conditional,
            condition: Some("If the jobs target is missed".into()),
            evidence: Some(ev(2, "If the jobs target is missed")),
        },
        payment: Payment {
            covenant: Some(Quoted {
                text: CLAUSE.into(),
                evidence: Some(ev(2, CLAUSE)),
            }),
            trigger_event: Some(TriggerEvent::OnDemand),
            trigger_phrase: Some(ev(
                2,
                "within 28 days of receiving written notice from the Council",
            )),
            ..Payment::default()
        },
    };
    let a = Agreement {
        schema_version: SCHEMA_VERSION.into(),
        provenance: Provenance::default(),
        instrument: Instrument {
            kind: InstrumentKind::Agreement,
            ..Instrument::default()
        },
        metadata: Metadata::default(),
        housing: Housing {
            total_units: Some(100),
            total_units_evidence: Some(ev(3, "Development means 100 residential units")),
            market_units: Some(75),
            market_units_evidence: Some(ev(3, "Market Housing Units means 75 units")),
            affordable_units: Some(25),
            affordable_units_evidence: Some(ev(
                3,
                "Affordable Housing Units means 25 units (50 Habitable Rooms)",
            )),
            affordable_habitable_rooms: Some(50),
            tenures: vec![TenureAllocation {
                tenure: Tenure::SocialRent,
                label: None,
                units: Some(25),
                habitable_rooms: Some(50),
                evidence: Some(ev(3, "Social Rent means 25 units (50 Habitable Rooms)")),
            }],
            claimed_affordable_pct: Some("25".parse().unwrap()),
            claimed_affordable_basis: Some(AffordableBasis::HabitableRooms),
            claimed_affordable_evidence: Some(ev(
                3,
                "The Development is 25% affordable by habitable room",
            )),
            ..Housing::default()
        },
        contributions: vec![c],
        variations: vec![],
        warnings: vec![],
    };
    (a, pg)
}
fn checks(a: &Agreement, pg: &Pages) -> Vec<verify::CheckResult> {
    verify::check_contribution(&a.contributions[0], pg, 1)
}

#[test]
fn synthetic_clean_twin_passes_financial_checks() {
    let (a, pg) = plant();
    let r = checks(&a, &pg);
    assert_eq!(r.len(), 5);
    assert!(r.iter().all(|r| r.status == Status::Pass), "{r:?}");
    assert_eq!(verify::check_unit_consistency(&a, &pg).status, Status::Pass);
    assert_eq!(verify::check_claimed_pct(&a, &pg).status, Status::Skip);
}
#[test]
fn incorrect_amount_is_detected() {
    let (mut a, pg) = plant();
    a.contributions[0].amount.value_gbp = Some("101".parse().unwrap());
    let r = checks(&a, &pg);
    assert_eq!(r[0].status, Status::Review);
    assert_eq!(r[1].status, Status::Fail);
}
#[test]
fn incorrect_amount_and_condition_pages_are_detected() {
    let (mut a, pg) = plant();
    a.contributions[0].amount.evidence.as_mut().unwrap().page = 3;
    assert_eq!(checks(&a, &pg)[0].status, Status::Review);
    a.contributions[0]
        .conditionality
        .evidence
        .as_mut()
        .unwrap()
        .page = 3;
    assert_eq!(checks(&a, &pg)[2].status, Status::Review);
}
#[test]
fn incorrect_trigger_page_without_matching_next_page_is_detected() {
    let (mut a, pg) = plant();
    a.contributions[0]
        .payment
        .trigger_phrase
        .as_mut()
        .unwrap()
        .page = 3;
    assert_eq!(checks(&a, &pg)[3].status, Status::Fail);
}
#[test]
fn ordinary_basis_mutation_with_wrong_arithmetic_is_detected() {
    let (mut a, pg) = plant();
    a.housing.total_units = Some(200);
    a.housing.claimed_affordable_basis = Some(AffordableBasis::Units);
    assert_eq!(verify::check_claimed_pct(&a, &pg).status, Status::Fail);
}
#[test]
fn limitation_basis_enum_is_not_grounded_when_both_ratios_coincide() {
    let (mut a, pg) = plant();
    // Source says habitable room. 25/100 and 50/200 both yield 25%, so arithmetic
    // cannot detect the wrong basis label. The synthetic answer is known by construction.
    a.housing.claimed_affordable_basis = Some(AffordableBasis::Units);
    assert_eq!(verify::check_claimed_pct(&a, &pg).status, Status::Pass);
}
#[test]
fn limitation_conditional_to_unconditional_removes_condition_check() {
    let (mut a, pg) = plant();
    a.contributions[0].conditionality = Conditionality {
        kind: ConditionalityKind::Unconditional,
        condition: None,
        evidence: None,
    };
    let r = checks(&a, &pg);
    assert_eq!(r.len(), 4);
    assert!(r.iter().all(|r| r.status == Status::Pass));
    assert!(r.iter().all(|r| !r.check_id.ends_with(".cond")));
}
#[test]
fn limitation_cap_to_fixed_is_not_detected() {
    let (mut a, pg) = plant();
    a.contributions[0].amount.structure = AmountStructure::Fixed;
    assert!(checks(&a, &pg).iter().all(|r| r.status == Status::Pass));
}
#[test]
fn limitation_omission_produces_no_coverage_alarm() {
    let (mut a, pg) = plant();
    a.contributions.clear();
    let r = verify::verify(&a, &pg);
    assert!(!r.is_empty());
    assert!(r.iter().all(|r| !r.check_id.starts_with('F')));
    assert!(
        r.iter()
            .all(|r| matches!(r.status, Status::Pass | Status::Skip))
    );
}
#[test]
fn incorrect_trigger_page_immediately_preceding_is_detected() {
    let (mut a, pg) = plant();
    // Although covenant matching uses page+1 for wrapped clauses, the trigger phrase
    // itself must still occur on the cited page.
    a.contributions[0]
        .payment
        .trigger_phrase
        .as_mut()
        .unwrap()
        .page = 1;
    assert_eq!(checks(&a, &pg)[3].status, Status::Fail);
}
