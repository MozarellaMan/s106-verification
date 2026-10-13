//! Pure scoring logic of the register eval: no network, no PDFs.

use rust_decimal::Decimal;
use s106::eval::{
    Reference, Register, agreement_across, gate_summary, register_reference, score_against_register,
};
use s106::schema::*;
use s106::verify::{CheckResult, ErrorClass, Status};

fn d(s: &str) -> Decimal {
    s.parse().unwrap()
}

fn contribution(name: &str, value: Option<&str>, trigger: Option<TriggerEvent>) -> Contribution {
    Contribution {
        name: name.into(),
        amount: Amount {
            structure: AmountStructure::Fixed,
            value_gbp: value.map(d),
            words: None,
            index_linked: false,
            evidence: None,
        },
        conditionality: Conditionality {
            kind: ConditionalityKind::Unconditional,
            condition: None,
            evidence: None,
        },
        payment: Payment {
            trigger_event: trigger,
            ..Default::default()
        },
    }
}

fn agreement(contributions: Vec<Contribution>, affordable_units: Option<u32>) -> Agreement {
    Agreement {
        schema_version: SCHEMA_VERSION.into(),
        provenance: Provenance::default(),
        instrument: Instrument::default(),
        metadata: Metadata::default(),
        housing: Housing {
            affordable_units,
            ..Default::default()
        },
        contributions,
        variations: vec![],
        warnings: vec![],
    }
}

fn reference(amounts: &[(&str, &str)], units: &[u32]) -> Reference {
    let amounts: Vec<(String, Decimal)> =
        amounts.iter().map(|(p, a)| (p.to_string(), d(a))).collect();
    let mut purpose_totals = std::collections::BTreeMap::new();
    for (p, a) in &amounts {
        *purpose_totals.entry(p.clone()).or_insert(Decimal::ZERO) += *a;
    }
    Reference {
        amounts,
        purpose_totals,
        affordable_units: units.to_vec(),
        agreement_types: vec![],
    }
}

// ---- ScoreAgainstRegister

#[test]
fn exact_match_within_a_pound() {
    let r = reference(&[("education", "10000.00"), ("highways", "2500.00")], &[]);
    let a = agreement(
        vec![
            contribution("Education", Some("10000.50"), None),
            contribution("Highways", Some("2499.00"), None),
            contribution("Other", Some("77"), None),
        ],
        None,
    );
    let s = score_against_register(&a, &r);
    assert_eq!((s.extracted_sums, s.extracted_sums_matched), (3, 2));
    assert_eq!((s.register_amounts, s.register_amounts_found), (2, 2));
    assert_eq!(s.unmatched_extracted, vec!["77"]);
    assert!(s.missed_register.is_empty());
    assert_eq!(s.affordable_units_match, None);
}

#[test]
fn just_outside_the_tolerance_misses() {
    let r = reference(&[("education", "10000.00")], &[]);
    let a = agreement(
        vec![contribution("Education", Some("10001.01"), None)],
        None,
    );
    let s = score_against_register(&a, &r);
    assert_eq!(s.extracted_sums_matched, 0);
    assert_eq!(s.missed_register, vec!["education=10000.00"]);
}

#[test]
fn purpose_total_matches_an_instalment_split() {
    // the register splits one £30,000 sum into two instalments; the deed states the total
    let r = reference(&[("education", "10000"), ("education", "20000")], &[]);
    let a = agreement(vec![contribution("Education", Some("30000"), None)], None);
    let s = score_against_register(&a, &r);
    assert_eq!(s.extracted_sums_matched, 1);
    assert_eq!(
        s.register_amounts_found, 2,
        "both instalments are accounted for by the purpose total"
    );
    assert!(s.missed_register.is_empty());
}

#[test]
fn contributions_without_a_sum_are_not_scored() {
    let r = reference(&[("education", "10000")], &[]);
    let a = agreement(vec![contribution("Travel plan", None, None)], None);
    let s = score_against_register(&a, &r);
    assert_eq!(
        (
            s.extracted_sums,
            s.extracted_sums_matched,
            s.register_amounts_found
        ),
        (0, 0, 0)
    );
    assert_eq!(s.missed_register, vec!["education=10000"]);
}

#[test]
fn cil_agreements_are_excluded_from_the_reference() {
    let reg = Register::from_rows(
        &[
            ("20/1/FUL", "A-1", "section-106"),
            ("20/1/FUL", "A-2", "community-infrastructure-levy"),
            ("other", "B-1", "section-106"),
        ],
        &[
            ("A-1", "education", Some(d("5000")), None),
            ("A-1", "affordable-housing", None, Some(d("3"))),
            ("A-2", "education", Some(d("99999")), None),
            ("A-2", "affordable-housing", None, Some(d("40"))),
            ("B-1", "highways", Some(d("1")), None),
        ],
    );
    let r = register_reference("20/1/FUL", &reg);
    assert_eq!(r.amounts, vec![("education".to_string(), d("5000"))]);
    assert_eq!(r.affordable_units, vec![3]);
    assert_eq!(r.agreement_types, vec!["section-106"]);
    let none = register_reference("unknown", &reg);
    assert!(none.amounts.is_empty() && none.affordable_units.is_empty());
}

#[test]
fn zero_and_missing_register_amounts_are_ignored() {
    let reg = Register::from_rows(
        &[("p", "A", "section-106")],
        &[
            ("A", "education", Some(d("0")), None),
            ("A", "highways", None, None),
            ("A", "health", Some(d("12.50")), None),
        ],
    );
    let r = register_reference("p", &reg);
    assert_eq!(r.amounts, vec![("health".to_string(), d("12.50"))]);
}

#[test]
fn affordable_units_match_a_single_value() {
    let r = reference(&[], &[3, 5]);
    assert_eq!(
        score_against_register(&agreement(vec![], Some(5)), &r).affordable_units_match,
        Some(true)
    );
}

#[test]
fn affordable_units_match_the_sum_of_register_values() {
    let r = reference(&[], &[3, 5]);
    assert_eq!(
        score_against_register(&agreement(vec![], Some(8)), &r).affordable_units_match,
        Some(true)
    );
}

#[test]
fn affordable_units_mismatch_or_missing() {
    let r = reference(&[], &[3, 5]);
    assert_eq!(
        score_against_register(&agreement(vec![], Some(4)), &r).affordable_units_match,
        Some(false)
    );
    assert_eq!(
        score_against_register(&agreement(vec![], None), &r).affordable_units_match,
        Some(false)
    );
    assert_eq!(
        score_against_register(&agreement(vec![], Some(4)), &reference(&[], &[]))
            .affordable_units_match,
        None
    );
}

// ---- AgreementAcross

fn two_contribution_agreement(trigger: TriggerEvent) -> Agreement {
    agreement(
        vec![
            contribution("Bus Contribution", Some("1000"), Some(trigger)),
            contribution(
                "Travel  Plan",
                Some("250.00"),
                Some(TriggerEvent::PreCommencement),
            ),
        ],
        Some(4),
    )
}

#[test]
fn identical_runs_agree() {
    let (a, b) = (
        two_contribution_agreement(TriggerEvent::PreOccupation),
        two_contribution_agreement(TriggerEvent::PreOccupation),
    );
    let ag = agreement_across(&[&a, &b]);
    assert_eq!(ag.runs_compared, 2);
    assert!(ag.all_agree());
    assert!(ag.housing_diffs.is_empty() && ag.field_diffs.is_empty());
    assert_eq!(
        (
            ag.contribution_names.union,
            ag.contribution_names.in_every_run
        ),
        (2, 2)
    );
}

#[test]
fn a_changed_trigger_label_is_reported() {
    let a = two_contribution_agreement(TriggerEvent::PreOccupation);
    let b = two_contribution_agreement(TriggerEvent::PreCommencement);
    let ag = agreement_across(&[&a, &b]);
    assert!(!ag.all_agree());
    assert!(ag.sums_identical && ag.housing_identical);
    assert_eq!(ag.field_diffs.len(), 1, "{:?}", ag.field_diffs);
    assert!(
        ag.field_diffs[0].starts_with("bus contribution: trigger"),
        "{:?}",
        ag.field_diffs
    );
}

#[test]
fn changed_amount_housing_and_missing_contribution_are_reported() {
    let a = two_contribution_agreement(TriggerEvent::PreOccupation);
    let mut b = two_contribution_agreement(TriggerEvent::PreOccupation);
    b.contributions[0].amount.value_gbp = Some(d("1200"));
    b.contributions[0].amount.index_linked = true;
    b.housing.affordable_units = Some(5);
    b.contributions.pop();
    let ag = agreement_across(&[&a, &b]);
    assert_eq!(ag.housing_diffs, vec!["affordable_units"]);
    assert!(!ag.sums_identical);
    assert_eq!(ag.contribution_names.only_some_runs, vec!["travel plan"]);
    assert!(
        ag.field_diffs.iter().any(|f| f.contains("amount"))
            && ag.field_diffs.iter().any(|f| f.contains("index_linked"))
    );
}

#[test]
fn instalment_trigger_labels_are_compared() {
    let inst = |t| Instalment {
        share_pct: Some(d("50")),
        trigger_event: Some(t),
        trigger_phrase: None,
    };
    let with = |second| {
        let mut c = contribution("Education", Some("1000"), None);
        c.payment.instalments = vec![inst(TriggerEvent::PreCommencement), inst(second)];
        agreement(vec![c], None)
    };
    let (a, b, c) = (
        with(TriggerEvent::PreOccupation),
        with(TriggerEvent::PreOccupation),
        with(TriggerEvent::OnDemand),
    );
    assert!(agreement_across(&[&a, &b]).all_agree());
    assert!(!agreement_across(&[&a, &c]).all_agree());
}

#[test]
fn fewer_than_two_runs_cannot_be_compared() {
    let a = two_contribution_agreement(TriggerEvent::PreOccupation);
    assert!(!agreement_across(&[&a]).compared());
    assert!(!agreement_across(&[]).compared());
}

// ---- GateSummary

fn check(id: &str, status: Status, class: Option<ErrorClass>) -> CheckResult {
    CheckResult {
        check_id: id.into(),
        description: "desc".into(),
        status,
        expected: None,
        actual: None,
        error_class: class,
        detail: "why".into(),
        pages: vec![],
    }
}

#[test]
fn gate_summary_counts_statuses_and_classes() {
    let g = gate_summary(&[
        check("H1", Status::Pass, None),
        check("H2", Status::Review, None),
        check("F1.a", Status::Fail, Some(ErrorClass::ParserError)),
        check("F2.a", Status::Fail, Some(ErrorClass::DocumentDiscrepancy)),
        check("F3.a", Status::Skip, None),
    ]);
    assert_eq!((g.pass, g.review, g.fail, g.skip), (1, 1, 2, 1));
    assert_eq!((g.parser_errors, g.document_discrepancies), (1, 1));
    assert_eq!(g.failures, vec!["F1.a desc: why", "F2.a desc: why"]);
    assert_eq!(g.reviews, vec!["H2 desc: why"]);
}

#[test]
fn only_fixed_sums_and_caps_are_totalled_and_scored() {
    let mut a = agreement(vec![], None);
    for (structure, value) in [
        (AmountStructure::Fixed, "100"),
        (AmountStructure::Cap, "200"),
        (AmountStructure::Rate, "10"),
        (AmountStructure::Formula, "20"),
        (AmountStructure::None, "30"),
    ] {
        let mut c = contribution("test", Some(value), None);
        c.amount.structure = structure;
        a.contributions.push(c);
    }
    assert_eq!(a.total_stated_gbp(), d("300"));
    let score = score_against_register(
        &a,
        &reference(&[("education", "100"), ("highways", "200")], &[]),
    );
    assert_eq!(score.extracted_sums, 2);
    assert_eq!(score.extracted_sums_matched, 2);
    assert!(score.unmatched_extracted.is_empty());
}
