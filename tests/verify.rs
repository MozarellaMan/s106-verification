//! Verifier behaviour: grounding outcomes, basis handling, mix-table grounding, trigger gate
//! (ports of the former Python grounding and trigger tests) plus the schema v2 checks.
//!
//! Pages are short verbatim RapidOCR excerpts from the Southwark sample (19/AP/1773).

use s106::evidence::Pages;
use s106::quantity;
use s106::schema::*;
use s106::text::Value;
use s106::verify::*;
use s106::wire::observe;

fn ev(page: usize, snippet: &str) -> Evidence {
    Evidence {
        page,
        snippet: snippet.to_string(),
    }
}

fn pages(items: &[(usize, &str)]) -> Pages {
    items.iter().map(|(p, t)| (*p, t.to_string())).collect()
}

/// Verbatim OCR lines (spaces dropped as RapidOCR drops them).
fn southwark_pages() -> Pages {
    pages(&[
        (
            5,
            r#""AffordableHousingUnits" means the 76 Residential Units (271 Habitable Rooms) tobe"#,
        ),
        (
            14,
            "445 sqm ancillary plant and equipment; and 253 residential\napartments(C3),35.75%affordablebyhabitable room,andother",
        ),
        (
            22,
            r#"3 "Market Housing Units" meansthe177ResidentialUnitsofMarketHousingcomprised"#,
        ),
        (
            91,
            "UnitType Social Rented Housing Intermediate Housing\n1 bed 10 7\n2bed 24 5\n3bed 24 6\n\
             Sub-Total 58 18\nTotal 76 (271 habitable rooms)",
        ),
    ])
}

fn total() -> Evidence {
    ev(14, "253 residential apartments")
}
fn market() -> Evidence {
    ev(22, "the 177 Residential Units of Market Housing")
}
fn affordable() -> Evidence {
    ev(5, "the 76 Residential Units (271 Habitable Rooms)")
}

fn agreement_with(kind: InstrumentKind, housing: Housing) -> Agreement {
    Agreement {
        schema_version: SCHEMA_VERSION.to_string(),
        provenance: Provenance::default(),
        instrument: Instrument {
            kind,
            ..Instrument::default()
        },
        metadata: Metadata::default(),
        housing,
        contributions: vec![],
        variations: vec![],
        warnings: vec![],
    }
}

fn agreement(housing: Housing) -> Agreement {
    agreement_with(InstrumentKind::Agreement, housing)
}

fn inp(label: &str, value: Option<Value>, evidence: Option<Evidence>) -> Input {
    Input {
        label: label.to_string(),
        value,
        evidence,
    }
}

fn good() -> Vec<Input> {
    vec![inp("total units", Some(253u32.into()), Some(total()))]
}

// --------------------------------------------------------------------------- ResolveTest

#[test]
fn resolve_holds_and_grounded_passes() {
    let r = resolve("X", "d", true, &good(), &southwark_pages(), None, None, &[]);
    assert_eq!(r.status, Status::Pass);
}

#[test]
fn resolve_holds_but_ungrounded_needs_review() {
    let i = [inp("total units", Some(250u32.into()), Some(total()))];
    let r = resolve("X", "d", true, &i, &southwark_pages(), None, None, &[]);
    assert_eq!(r.status, Status::Review);
    assert_eq!(r.error_class, None);
    assert!(
        r.detail.contains("250 not stated in its cited snippet"),
        "{}",
        r.detail
    );
}

#[test]
fn resolve_fails_and_grounded_is_discrepancy_with_caveat() {
    let r = resolve(
        "X",
        "d",
        false,
        &good(),
        &southwark_pages(),
        None,
        None,
        &[],
    );
    assert_eq!(r.status, Status::Fail);
    assert_eq!(r.error_class, Some(ErrorClass::DocumentDiscrepancy));
    assert!(r.detail.contains("confirm against the original deed"));
}

#[test]
fn resolve_snippet_found_but_value_not_in_it_is_parser_error() {
    // Review repro (B): the quotation exists in the deed, but the extracted value isn't in it.
    let i = [inp("total units", Some(250u32.into()), Some(total()))];
    let r = resolve("X", "d", false, &i, &southwark_pages(), None, None, &[]);
    assert_eq!(r.error_class, Some(ErrorClass::ParserError));
}

#[test]
fn resolve_missing_value_is_parser_error() {
    let i = [inp("total units", None, Some(total()))];
    let r = resolve("X", "d", false, &i, &southwark_pages(), None, None, &[]);
    assert_eq!(r.error_class, Some(ErrorClass::ParserError));
    assert!(r.detail.contains("not extracted"));
}

#[test]
fn resolve_relocation_still_required() {
    let i = [inp(
        "total units",
        Some(253u32.into()),
        Some(ev(22, "253 residential apartments")),
    )];
    let r = resolve("X", "d", true, &i, &southwark_pages(), None, None, &[]);
    assert_eq!(r.status, Status::Review);
    assert!(r.detail.contains("not re-locatable on p22"), "{}", r.detail);
}

#[test]
fn grounding_problem_messages() {
    let pg = southwark_pages();
    assert_eq!(grounding_problem(&good()[0], &pg), None);
    let no_ev = inp("total units", Some(253u32.into()), None);
    assert_eq!(
        grounding_problem(&no_ev, &pg).as_deref(),
        Some("total units: no evidence cited")
    );
}

// --------------------------------------------------------------------------- ConsistentButInventedTest

fn unit_housing(t: u32, m: u32, a: u32) -> Housing {
    Housing {
        total_units: Some(t),
        total_units_evidence: Some(total()),
        market_units: Some(m),
        market_units_evidence: Some(market()),
        affordable_units: Some(a),
        affordable_units_evidence: Some(affordable()),
        ..Housing::default()
    }
}

#[test]
fn arithmetic_alone_no_longer_passes() {
    // Review repro (B): 250 = 174 + 76 adds up, but 250 and 174 aren't what the cited text says.
    let a = agreement(unit_housing(250, 174, 76));
    let r = check_unit_consistency(&a, &southwark_pages());
    assert_eq!(r.status, Status::Review);
    assert!(r.detail.contains("total units: 250"), "{}", r.detail);
    assert!(r.detail.contains("market units: 174"), "{}", r.detail);
}

#[test]
fn true_values_pass() {
    let a = agreement(unit_housing(253, 177, 76));
    let r = check_unit_consistency(&a, &southwark_pages());
    assert_eq!(r.status, Status::Pass, "{}", r.detail);
    assert_eq!(r.actual.as_deref(), Some("177 + 76 = 253"));
    assert_eq!(r.expected.as_deref(), Some("253"));
}

// --------------------------------------------------------------------------- PercentageBasisTest

fn pct_base(basis: Option<AffordableBasis>) -> Agreement {
    agreement(Housing {
        total_units: Some(253),
        total_units_evidence: Some(total()),
        affordable_units: Some(76),
        affordable_units_evidence: Some(affordable()),
        claimed_affordable_pct: observe("30.04", quantity::decimal),
        claimed_affordable_basis: basis,
        claimed_affordable_evidence: Some(ev(14, "35.75%affordable")),
        ..Housing::default()
    })
}

#[test]
fn floorspace_basis_skips() {
    // 76/253 = 30.04%, so a silent units fallback would wrongly PASS this.
    let r = check_claimed_pct(
        &pct_base(Some(AffordableBasis::Floorspace)),
        &southwark_pages(),
    );
    assert_eq!(r.status, Status::Skip);
    assert!(r.detail.contains("floorspace"));
    assert_eq!(r.actual, None);
}

#[test]
fn unstated_basis_skips() {
    let r = check_claimed_pct(&pct_base(None), &southwark_pages());
    assert_eq!(r.status, Status::Skip);
    assert!(r.detail.contains("basis"));
}

#[test]
fn units_basis_still_computes() {
    let r = check_claimed_pct(&pct_base(Some(AffordableBasis::Units)), &southwark_pages());
    assert_eq!(r.actual.as_deref(), Some("76/253 = 30.04%"));
    // The cited snippet says 35.75, not 30.04, so the match is unproven rather than passed.
    assert_eq!(r.status, Status::Review);
}

#[test]
fn habitable_rooms_without_total_skips_with_implied_denominator() {
    let mut a = pct_base(Some(AffordableBasis::HabitableRooms));
    a.housing.affordable_habitable_rooms = Some(271);
    a.housing.claimed_affordable_pct = observe("35.75", quantity::decimal);
    let r = check_claimed_pct(&a, &southwark_pages());
    assert_eq!(r.status, Status::Skip);
    assert!(
        r.detail
            .contains("implied total = 271/35.75% = 758.0 rooms"),
        "{}",
        r.detail
    );
}

// --------------------------------------------------------------------------- MixTableGroundingTest

fn mix_agreement(rows: &[(&str, u32, u32)]) -> Agreement {
    use Tenure::{Intermediate, SocialRent};
    agreement(Housing {
        affordable_units: Some(76),
        affordable_units_evidence: Some(affordable()),
        affordable_habitable_rooms: Some(271),
        tenures: vec![
            TenureAllocation {
                tenure: SocialRent,
                label: None,
                units: Some(58),
                habitable_rooms: None,
                evidence: Some(ev(5, "the 76")),
            },
            TenureAllocation {
                tenure: Intermediate,
                label: None,
                units: Some(18),
                habitable_rooms: None,
                evidence: Some(ev(5, "the 76")),
            },
        ],
        mix_table: Some(MixTable {
            rows: rows
                .iter()
                .map(|(u, s, i)| MixRow {
                    unit_type: u.to_string(),
                    units_by_tenure: vec![(SocialRent, *s), (Intermediate, *i)],
                })
                .collect(),
            subtotals: vec![(SocialRent, 58), (Intermediate, 18)],
            total_units: Some(76),
            total_habitable_rooms: Some(271),
            evidence: Some(ev(91, "Total 76 (271 habitable rooms)")),
        }),
        ..Housing::default()
    })
}

fn h5(results: &[CheckResult]) -> Vec<&CheckResult> {
    results.iter().filter(|r| r.check_id == "H5").collect()
}

#[test]
fn mix_rows_on_page_pass() {
    let a = mix_agreement(&[("1 bed", 10, 7), ("2 bed", 24, 5), ("3 bed", 24, 6)]);
    let res = check_mix_table(&a, &southwark_pages());
    let h5 = h5(&res);
    assert_eq!(h5.len(), 3);
    assert!(
        h5.iter().all(|r| r.status == Status::Pass),
        "{:?}",
        h5.iter().map(|r| &r.detail).collect::<Vec<_>>()
    );
    assert_eq!(h5[0].actual.as_deref(), Some("10 + 24 + 24 = 58"));
}

#[test]
fn mix_invented_row_that_still_sums_needs_review() {
    // 10+24+24 and 7+5+6 both still sum correctly with the cells moved between rows
    let a = mix_agreement(&[("1 bed", 24, 7), ("2 bed", 10, 5), ("3 bed", 24, 6)]);
    let res = check_mix_table(&a, &southwark_pages());
    let r = res
        .iter()
        .find(|r| r.description.starts_with("Mix table: Σ Social"))
        .unwrap();
    assert_eq!(r.status, Status::Review);
    assert!(r.detail.contains("row '1 bed' not found"), "{}", r.detail);
}

#[test]
fn mix_repeated_counts_need_repeats_on_the_line() {
    let mut pg = southwark_pages();
    let t = pg[&91].replace("3bed 24 6", "3bed 24 24");
    pg.insert(91, t);
    let a = mix_agreement(&[("1 bed", 10, 7), ("2 bed", 24, 5), ("3 bed", 24, 24)]);
    let ok: String = check_mix_table(&a, &pg)
        .iter()
        .map(|r| r.detail.clone())
        .collect::<Vec<_>>()
        .join(" ");
    assert!(!ok.contains("row '3 bed'"), "{ok}");
    let bad: String = check_mix_table(&a, &southwark_pages())
        .iter()
        .map(|r| r.detail.clone())
        .collect::<Vec<_>>()
        .join(" ");
    assert!(bad.contains("row '3 bed' not found"), "{bad}");
}

#[test]
fn no_mix_table_skips() {
    let r = check_mix_table(&agreement(Housing::default()), &southwark_pages());
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].status, Status::Skip);
}

// --------------------------------------------------------------------------- CheckTriggerTest

use TriggerEvent as E;

/// Verbatim RapidOCR output for Schedule 11 para 3 (p89 of the sample): "50%" reads as "5o%".
const PAGE_89: &str = "3. CarbonGreenFund Contribution
3.1 Prior to the Occupation of 5o%of theDevelopment(or upon such other dateas may be
agreed between the Council and the Owner) the Council and the Owner shall compare the
environmental performance of the Development against the Agreed Carbon Targets and
3.5 The Carbon GreenFundContribution(as agreed between the Council and the Owner or
determined in accordance withclause 19of this Agreement) shall be payable within 20
WorkingDaysofsuchagreementordetermination.";

const CLAUSE_3_5: &str = "The Carbon Green Fund Contribution (as agreed between the Council and the Owner or determined in \
accordance with clause 19 of this Agreement) shall be payable within 20 Working Days of such \
agreement or determination.";
const CLAUSE_3_1: &str = "Prior to the Occupation of 50% of the Development (or upon such other date as may be agreed between \
the Council and the Owner) the Council and the Owner shall compare the environmental performance of \
the Development against the Agreed Carbon Targets";

fn amount(
    structure: AmountStructure,
    value: Option<&str>,
    words: Option<&str>,
    evidence: Option<Evidence>,
) -> Amount {
    Amount {
        structure,
        value_gbp: value.and_then(|v| observe(v, quantity::decimal)),
        words: words.and_then(|w| observe(w, quantity::amount)),
        index_linked: false,
        evidence,
    }
}

fn contribution(phrase: &str, clause: &str, event: E, model: Option<E>) -> Contribution {
    Contribution {
        name: "Carbon Green Fund Contribution".to_string(),
        amount: amount(
            AmountStructure::Fixed,
            Some("475991.00"),
            None,
            Some(ev(8, "£475,991.00")),
        ),
        conditionality: Conditionality {
            kind: ConditionalityKind::Unconditional,
            condition: None,
            evidence: None,
        },
        payment: Payment {
            covenant: Some(Quoted {
                text: clause.to_string(),
                evidence: None,
            }),
            trigger_event: Some(event),
            trigger_event_model: model,
            trigger_phrase: Some(ev(89, phrase)),
            instalments: vec![],
        },
    }
}

fn run_check(c: &Contribution, pg: &Pages) -> std::collections::BTreeMap<String, CheckResult> {
    check_trigger(c, pg, "F5", &[8])
        .into_iter()
        .map(|r| (r.check_id.split('.').nth(1).unwrap().to_string(), r))
        .collect()
}

fn p89() -> Pages {
    pages(&[(89, PAGE_89)])
}

#[test]
fn payment_clause_passes_and_records_model_label() {
    let c = contribution(
        "payable within 20 Working Days of",
        CLAUSE_3_5,
        E::OnDemand,
        Some(E::Other),
    );
    let r = run_check(&c, &p89());
    assert_eq!(r["trig"].status, Status::Pass, "{}", r["trig"].detail);
    assert_eq!(r["tlabel"].status, Status::Pass);
    assert!(r["tlabel"].detail.contains("model proposed 'other'"));
}

#[test]
fn assessment_clause_is_not_a_trigger() {
    // Live run 1 cited the assessment timing (3.1), which makes nothing payable.
    let c = contribution(
        "Prior to the Occupation of 50% of the Development",
        CLAUSE_3_1,
        E::PreOccupation,
        None,
    );
    let r = run_check(&c, &p89());
    assert_eq!(r["trig"].status, Status::Fail);
    assert!(
        r["trig"].detail.contains("does not impose a payment"),
        "{}",
        r["trig"].detail
    );
}

#[test]
fn invented_payment_clause_fails() {
    // Review repro: a real timing phrase wrapped in a payment covenant the deed doesn't contain.
    let c = contribution(
        "Prior to the Occupation of 50% of the Development",
        "Prior to the Occupation of 50% of the Development the Owner shall pay the Carbon Green Fund Contribution to the Council",
        E::PreOccupation,
        None,
    );
    let r = run_check(&c, &p89());
    assert_eq!(r["trig"].status, Status::Fail);
    assert!(
        r["trig"].detail.contains("payment clause not re-locatable"),
        "{}",
        r["trig"].detail
    );
}

#[test]
fn question_clause_present_in_source_fails() {
    // Re-review repros: the clause is verbatim in the deed, but it asks whether a sum is payable.
    for clause in [
        "Prior to Occupation the Council shall assess whether the Carbon Green Fund Contribution is payable.",
        "Prior to Occupation the Council shall assess if the Contribution is payable",
        "PriortoOccupationtheCouncilshallassessiftheContributionispayable",
    ] {
        let c = contribution("Prior to Occupation", clause, E::PreOccupation, None);
        let pg = pages(&[(89, &format!("{PAGE_89}\n{clause}"))]);
        let r: Vec<CheckResult> = check_trigger(&c, &pg, "F1", &[8]);
        assert_eq!(r[0].check_id, "F1.trig");
        assert_eq!(r[0].status, Status::Fail, "{clause}");
        assert!(
            r[0].detail.contains("does not impose a payment"),
            "{clause}: {}",
            r[0].detail
        );
        assert!(
            r.iter().all(|x| !x.check_id.ends_with("tlabel")),
            "{clause}"
        );
    }
}

#[test]
fn phrase_must_sit_in_the_clause() {
    let c = contribution(
        "Prior to the Occupation of 50% of the Development",
        CLAUSE_3_5,
        E::PreOccupation,
        None,
    );
    let r = run_check(&c, &p89());
    assert_eq!(r["trig"].status, Status::Fail);
    assert!(
        r["trig"].detail.contains("not part of the cited clause"),
        "{}",
        r["trig"].detail
    );
}

#[test]
fn misquoted_phrase_fails() {
    let c = contribution(
        "payable within 10 Working Days of",
        &CLAUSE_3_5.replace("20", "10"),
        E::OnDemand,
        None,
    );
    assert_eq!(run_check(&c, &p89())["trig"].status, Status::Fail);
}

#[test]
fn label_must_follow_phrase() {
    let c = contribution(
        "payable within 20 Working Days of",
        CLAUSE_3_5,
        E::PreOccupation,
        None,
    );
    let r = run_check(&c, &p89());
    assert_eq!(r["trig"].status, Status::Pass);
    assert_eq!(r["tlabel"].status, Status::Fail);
    assert_eq!(r["tlabel"].error_class, Some(ErrorClass::ParserError));
}

#[test]
fn missing_trigger_is_parser_error() {
    let mut c = contribution("x", "y", E::OnDemand, None);
    c.payment = Payment::default();
    let r = check_trigger(&c, &p89(), "F1", &[]);
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].error_class, Some(ErrorClass::ParserError));
    assert!(r[0].detail.contains("no trigger clause matched"));
}

// --------------------------------------------------------------------------- amounts

#[test]
fn amount_and_words_checks() {
    let pg = pages(&[(
        4,
        "the sum of £16,913.26 (sixteen thousand nine hundred and thirteen pounds and twenty six pence)",
    )]);
    let mut c = contribution("x", "y", E::OnDemand, None);
    c.name = "Administration Cost".to_string();
    c.amount = amount(
        AmountStructure::Fixed,
        Some("16913.26"),
        Some("sixteen thousand nine hundred and thirteen pounds and twenty six pence"),
        Some(ev(
            4,
            "the sum of £16,913.26 (sixteen thousand nine hundred and thirteen pounds and twenty six pence)",
        )),
    );
    c.payment = Payment::default();
    let r = check_contribution(&c, &pg, 3);
    assert_eq!(r[0].check_id, "F3.amt");
    assert_eq!(
        r[0].description,
        "Administration Cost: amount stated in cited evidence"
    );
    assert_eq!(r[0].status, Status::Pass, "{}", r[0].detail);
    assert_eq!(r[0].actual.as_deref(), Some("£16,913.26"));
    assert_eq!(r[1].check_id, "F3.words");
    assert_eq!(r[1].status, Status::Pass, "{}", r[1].detail);
}

#[test]
fn formula_without_figure_skips_and_fixed_without_figure_fails() {
    let pg = pages(&[]);
    let mut c = contribution("x", "y", E::OnDemand, None);
    c.payment = Payment::default();
    c.amount = amount(AmountStructure::Formula, None, None, Some(ev(6, "formula")));
    let r = check_contribution(&c, &pg, 1);
    assert_eq!(r[0].status, Status::Skip);
    assert_eq!(
        r[0].detail,
        "contingent / formula-based: no fixed sum in the deed"
    );
    c.amount = amount(AmountStructure::Fixed, None, None, Some(ev(6, "formula")));
    let r = check_contribution(&c, &pg, 1);
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].status, Status::Fail);
    assert_eq!(r[0].error_class, Some(ErrorClass::ParserError));
}

// --------------------------------------------------------------------------- v2: variations

#[test]
fn variation_skips_whole_scheme_checks() {
    let a = agreement_with(InstrumentKind::DeedOfVariation, unit_housing(250, 174, 76));
    let pg = southwark_pages();
    for r in [
        check_unit_consistency(&a, &pg),
        check_tenure_units(&a, &pg),
        check_tenure_rooms(&a, &pg),
        check_claimed_pct(&a, &pg),
    ] {
        assert_eq!(r.status, Status::Skip, "{}", r.check_id);
        assert_eq!(
            r.detail,
            "variation/supplemental deed: restates only what it changes"
        );
    }
    let all = verify(&a, &pg);
    assert_eq!(
        all.iter().find(|r| r.check_id == "H1").unwrap().status,
        Status::Skip
    );
}

#[test]
fn v1_missing_parent_fails() {
    let a = agreement_with(InstrumentKind::DeedOfVariation, Housing::default());
    let r = check_instrument(&a, &southwark_pages());
    assert_eq!(r.check_id, "V1");
    assert_eq!(r.status, Status::Fail);
    assert_eq!(r.error_class, Some(ErrorClass::ParserError));
}

#[test]
fn v1_parent_grounded_passes_ungrounded_reviews_and_other_kinds_skip() {
    let pg = pages(&[(
        1,
        "THIS DEED OF VARIATION is made to the Agreement dated 14 September 2020",
    )]);
    let mut a = agreement_with(InstrumentKind::Supplemental, Housing::default());
    a.instrument.parent = Some(ParentRef {
        description: "Agreement dated 14 September 2020".to_string(),
        date: Some("2020-09-14".to_string()),
        evidence: Some(ev(1, "the Agreement dated 14 September 2020")),
    });
    assert_eq!(check_instrument(&a, &pg).status, Status::Pass);
    a.instrument.parent.as_mut().unwrap().evidence =
        Some(ev(1, "an agreement dated 1 January 1999"));
    assert_eq!(check_instrument(&a, &pg).status, Status::Review);
    a.instrument.parent.as_mut().unwrap().evidence = None;
    assert_eq!(check_instrument(&a, &pg).status, Status::Review);
    assert_eq!(
        check_instrument(&agreement(Housing::default()), &pg).status,
        Status::Skip
    );
}

#[test]
fn v2_variation_grounding() {
    let pg = pages(&[(
        3,
        "by deleting the definition of \"Qualifying Person\" and replacing it with the sum of £115,200",
    )]);
    let mut v = Variation {
        op: VariationOp::Delete,
        target: "definition \"Qualifying Person\"".to_string(),
        new_text: None,
        evidence: Some(ev(3, "deleting the definition of \"Qualifying Person\"")),
    };
    let r = check_variation(&v, 1, &pg);
    assert_eq!(r.check_id, "V2.1");
    assert_eq!(r.status, Status::Pass, "{}", r.detail);
    v.evidence = Some(ev(
        3,
        "deleting the definition of \"Restricted-Age Dwelling\"",
    ));
    assert_eq!(check_variation(&v, 1, &pg).status, Status::Review);
    v.evidence = None;
    assert_eq!(check_variation(&v, 1, &pg).status, Status::Review);
}

// --------------------------------------------------------------------------- v2: conditionality

fn cond_contribution(
    kind: ConditionalityKind,
    condition: Option<&str>,
    evidence: Option<Evidence>,
) -> Contribution {
    let mut c = contribution("x", "y", E::OnDemand, None);
    c.name = "Default Employment Contribution".to_string();
    c.amount = amount(AmountStructure::None, None, None, None);
    c.conditionality = Conditionality {
        kind,
        condition: condition.map(str::to_string),
        evidence,
    };
    c.payment = Payment::default();
    c
}

#[test]
fn cond_conditional_with_grounded_condition_passes() {
    let pg = pages(&[(
        12,
        "the sum of £73,100 which may be payable in accordance with paragraph 3.4 of Part 1 of Schedule 9",
    )]);
    let cond = "which may be payable in accordance with paragraph 3.4 of Part 1 of Schedule 9";
    let c = cond_contribution(
        ConditionalityKind::Conditional,
        Some(cond),
        Some(ev(12, cond)),
    );
    let r = check_conditionality(&c, &pg, "F8").unwrap();
    assert_eq!(r.check_id, "F8.cond");
    assert_eq!(r.status, Status::Pass, "{}", r.detail);
    // not in the cited snippet -> REVIEW
    let c = cond_contribution(
        ConditionalityKind::Conditional,
        Some("if jobs targets are missed"),
        Some(ev(12, cond)),
    );
    assert_eq!(
        check_conditionality(&c, &pg, "F8").unwrap().status,
        Status::Review
    );
    // snippet that is not in the deed -> REVIEW
    let c = cond_contribution(
        ConditionalityKind::Conditional,
        Some(cond),
        Some(ev(13, cond)),
    );
    assert_eq!(
        check_conditionality(&c, &pg, "F8").unwrap().status,
        Status::Review
    );
}

#[test]
fn cond_unknown_skips_and_unconditional_has_no_check() {
    let pg = pages(&[]);
    let c = cond_contribution(ConditionalityKind::Unknown, None, None);
    let r = check_conditionality(&c, &pg, "F1").unwrap();
    assert_eq!(r.status, Status::Skip);
    assert_eq!(r.detail, "conditionality not established");
    assert!(
        check_conditionality(
            &cond_contribution(ConditionalityKind::Unconditional, None, None),
            &pg,
            "F1"
        )
        .is_none()
    );
}

#[test]
fn figureless_conditional_contribution_skips_amount() {
    let c = cond_contribution(ConditionalityKind::Conditional, Some("c"), None);
    let r = check_contribution(&c, &pages(&[]), 1);
    assert_eq!(r[0].check_id, "F1.amt");
    assert_eq!(r[0].status, Status::Skip);
}

// --------------------------------------------------------------------------- v2: instalments

const PAGE_30: &str =
    "The Owner covenants with the Council to pay the Contribution to the Council as follows
(a) 50% prior to Occupation of the 1st Dwelling; and
(b) 50% prior to Occupation of the 50th Dwelling.";

fn instalment_contribution(shares: [&str; 2], events: [E; 2]) -> Contribution {
    let mut c = contribution("x", "y", E::OnDemand, None);
    c.name = "Highways Contribution".to_string();
    c.amount = amount(
        AmountStructure::Fixed,
        Some("12650.00"),
        None,
        Some(ev(30, "£12,650.00")),
    );
    c.payment = Payment {
        covenant: Some(Quoted {
            text: "The Owner covenants with the Council to pay the Contribution to the Council as follows".to_string(),
            evidence: Some(ev(30, "The Owner covenants with the Council to pay the Contribution")),
        }),
        trigger_event: None,
        trigger_event_model: None,
        trigger_phrase: None,
        instalments: vec![
            Instalment {
                share_pct: observe(shares[0], quantity::decimal),
                trigger_event: Some(events[0]),
                trigger_phrase: Some(ev(30, "50% prior to Occupation of the 1st Dwelling")),
            },
            Instalment {
                share_pct: observe(shares[1], quantity::decimal),
                trigger_event: Some(events[1]),
                trigger_phrase: Some(ev(30, "50% prior to Occupation of the 50th Dwelling")),
            },
        ],
    };
    c
}

fn by_id(rs: &[CheckResult]) -> std::collections::BTreeMap<&str, &CheckResult> {
    rs.iter().map(|r| (r.check_id.as_str(), r)).collect()
}

#[test]
fn inst_shares_summing_to_100_pass() {
    let c = instalment_contribution(["50", "50"], [E::PreOccupation, E::PreOccupation]);
    let rs = check_contribution(&c, &pages(&[(30, PAGE_30)]), 3);
    let m = by_id(&rs);
    assert_eq!(m["F3.inst"].status, Status::Pass, "{}", m["F3.inst"].detail);
    assert_eq!(
        m["F3.instcov"].status,
        Status::Pass,
        "{}",
        m["F3.instcov"].detail
    );
    assert_eq!(
        m["F3.inst1"].status,
        Status::Pass,
        "{}",
        m["F3.inst1"].detail
    );
    assert_eq!(
        m["F3.inst2"].status,
        Status::Pass,
        "{}",
        m["F3.inst2"].detail
    );
    // the single-trigger checks are replaced
    assert!(!m.contains_key("F3.trig") && !m.contains_key("F3.tlabel"));
}

#[test]
fn inst_shares_not_summing_to_100_with_grounded_values_is_discrepancy() {
    let mut c = instalment_contribution(["50", "40"], [E::PreOccupation, E::PreOccupation]);
    c.payment.instalments[1]
        .trigger_phrase
        .as_mut()
        .unwrap()
        .snippet = "40% prior to Occupation of the 50th Dwelling".into();
    let source = PAGE_30.replace("(b) 50%", "(b) 40%");
    let rs = check_contribution(&c, &pages(&[(30, &source)]), 3);
    let r = by_id(&rs)["F3.inst"].clone();
    assert_eq!(r.status, Status::Fail);
    assert_eq!(r.error_class, Some(ErrorClass::DocumentDiscrepancy));
    assert_eq!(r.actual.as_deref(), Some("50 + 40 = 90%"));
}

#[test]
fn inst_shares_not_summing_with_ungrounded_phrases_is_parser_error() {
    let c = instalment_contribution(["50", "40"], [E::PreOccupation, E::PreOccupation]);
    let rs = check_contribution(&c, &pages(&[(30, "nothing relevant here")]), 3);
    let r = by_id(&rs)["F3.inst"].clone();
    assert_eq!(r.status, Status::Fail);
    assert_eq!(r.error_class, Some(ErrorClass::ParserError));
}

#[test]
fn inst_label_mismatch_fails() {
    let c = instalment_contribution(["50", "50"], [E::PreOccupation, E::PreCommencement]);
    let rs = check_contribution(&c, &pages(&[(30, PAGE_30)]), 3);
    let m = by_id(&rs);
    assert_eq!(m["F3.inst1"].status, Status::Pass);
    assert_eq!(m["F3.inst2"].status, Status::Fail);
    assert_eq!(m["F3.inst2"].error_class, Some(ErrorClass::ParserError));
    assert_eq!(
        m["F3.inst2"].expected.as_deref(),
        Some("prior to Occupation")
    );
}

#[test]
fn inst_unlocatable_phrase_needs_review_and_non_payment_lead_in_fails() {
    let mut c = instalment_contribution(["50", "50"], [E::PreOccupation, E::PreOccupation]);
    c.payment.instalments[1].trigger_phrase =
        Some(ev(30, "prior to Occupation of the 99th Dwelling"));
    let rs = check_contribution(&c, &pages(&[(30, PAGE_30)]), 3);
    assert_eq!(by_id(&rs)["F3.inst2"].status, Status::Review);

    let mut c = instalment_contribution(["50", "50"], [E::PreOccupation, E::PreOccupation]);
    c.payment.covenant = Some(Quoted {
        text: "The Council shall assess whether the Contribution is payable".to_string(),
        evidence: Some(ev(
            30,
            "The Council shall assess whether the Contribution is payable",
        )),
    });
    let pg = pages(&[(
        30,
        &format!("{PAGE_30}\nThe Council shall assess whether the Contribution is payable"),
    )]);
    let rs = check_contribution(&c, &pg, 3);
    let r = by_id(&rs)["F3.instcov"].clone();
    assert_eq!(r.status, Status::Fail);
    assert!(r.detail.contains("does not impose a payment"));
}

#[test]
fn invented_instalment_shares_cannot_pass_or_blame_the_deed() {
    for (shares, expected_status, expected_class) in [
        (["60", "40"], Status::Review, None),
        (["50", "40"], Status::Fail, Some(ErrorClass::ParserError)),
    ] {
        let c = instalment_contribution(shares, [E::PreOccupation, E::PreOccupation]);
        let rs = check_contribution(&c, &pages(&[(30, PAGE_30)]), 3);
        let r = by_id(&rs)["F3.inst"];
        assert_eq!(r.status, expected_status, "{}", r.detail);
        assert_eq!(r.error_class, expected_class, "{}", r.detail);
    }
}

#[test]
fn timing_without_share_evidence_is_review() {
    let mut c = instalment_contribution(["50", "50"], [E::PreOccupation, E::PreOccupation]);
    for inst in &mut c.payment.instalments {
        let ev = inst.trigger_phrase.as_mut().unwrap();
        ev.snippet = ev.snippet.trim_start_matches("50% ").to_string();
    }
    let rs = check_contribution(&c, &pages(&[(30, PAGE_30)]), 3);
    assert_eq!(by_id(&rs)["F3.inst"].status, Status::Review);
}

#[test]
fn unreadable_pence_is_a_parser_error_even_when_the_quote_relocates() {
    let words = "ten pounds and illegible pence";
    let snippet = format!("£10 ({words})");
    let mut c = contribution("x", "y", E::OnDemand, None);
    c.amount = amount(
        AmountStructure::Fixed,
        Some("10"),
        Some(words),
        Some(ev(1, &snippet)),
    );
    let rs = check_contribution(&c, &pages(&[(1, &snippet)]), 1);
    let r = by_id(&rs)["F1.words"];
    assert_eq!(r.status, Status::Fail);
    assert_eq!(r.error_class, Some(ErrorClass::ParserError));
}

#[test]
fn dwelling_numbers_do_not_ground_instalment_percentages() {
    let mut c = instalment_contribution(["50", "50"], [E::PreOccupation, E::PreOccupation]);
    for inst in &mut c.payment.instalments {
        inst.trigger_phrase = Some(ev(30, "prior to Occupation of the 50th Dwelling"));
    }
    let rs = check_contribution(&c, &pages(&[(30, PAGE_30)]), 3);
    assert_eq!(by_id(&rs)["F3.inst"].status, Status::Review);
}

#[test]
fn unparseable_amount_does_not_suppress_instalment_checks() {
    let mut c = instalment_contribution(["40", "40"], [E::PreOccupation, E::PreOccupation]);
    c.amount.structure = AmountStructure::Rate;
    c.amount.value_gbp = observe("banana", quantity::decimal);
    c.payment.instalments[0]
        .trigger_phrase
        .as_mut()
        .unwrap()
        .snippet = "40% prior to Occupation of the 1st Dwelling".into();
    c.payment.instalments[1]
        .trigger_phrase
        .as_mut()
        .unwrap()
        .snippet = "40% prior to Occupation of the 50th Dwelling".into();
    let source = PAGE_30.replace("50%", "40%");
    let rs = check_contribution(&c, &pages(&[(30, &source)]), 3);
    let m = by_id(&rs);
    assert_eq!(m["F3.amt"].status, Status::Fail);
    assert!(!m.contains_key("F3.words"));
    let r = m["F3.inst"];
    assert_eq!(r.status, Status::Fail, "{}", r.detail);
    assert_eq!(r.error_class, Some(ErrorClass::DocumentDiscrepancy));
}
