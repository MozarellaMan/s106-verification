//! Reconciliation: obligation events from the recorded 19/AP/1773 extraction, and status rules
//! against delivery records.
//!
//! Most delivery records here are synthetic, built to exercise each rule. The tests named `pld_*`
//! and `southwark_against_pld_snapshot` read the real saved PLD snapshot.

use std::path::Path;

use chrono::NaiveDate;
use rust_decimal::Decimal;
use s106::evidence::Pages;
use s106::pld::{self, Request, Snapshot, SnapshotRef};
use s106::reconcile::*;
use s106::schema::*;
use s106::wire::{self, PageMap, WireExtraction};

fn dec(s: &str) -> Decimal {
    s.parse().unwrap()
}

/// The Southwark sample's recorded v2 extraction, converted without the PDF (so ungrounded).
fn southwark() -> Agreement {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures/llm/southwark_s106_sample/claude-haiku-5-5.extract.v2.json");
    let fixture: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let pm = PageMap(serde_json::from_value(fixture["pages"].clone()).unwrap());
    let w: WireExtraction = serde_json::from_value(fixture["output"].clone()).unwrap();
    // `..Default::default()` keeps this compiling as `Agreement` gains fields.
    Agreement {
        schema_version: SCHEMA_VERSION.into(),
        provenance: Provenance {
            sha256: fixture["pdf_sha256"].as_str().unwrap().into(),
            ..Default::default()
        },
        instrument: wire::instrument(&w.instrument, &pm),
        metadata: wire::metadata(&w.metadata),
        housing: wire::housing(&w.housing, &pm),
        contributions: w
            .contributions
            .iter()
            .map(|c| wire::contribution(c, &pm))
            .collect(),
        ..Default::default()
    }
}

/// Page text standing in for OCR: the deed wording the fixture quotes for p5 and p22, with
/// spaces dropped as RapidOCR drops them. p44 is absent, so citations there can't re-locate.
fn southwark_pages() -> Pages {
    [
        (
            5,
            "\"AffordableHousingUnits\" means the 76 Residential Units (271 Habitable Rooms) tobe\n\
             constructed upon the Site in accordance with the Approved Affordable Housing Mix",
        ),
        (
            22,
            "3 \"Market Housing Units\" meansthe177ResidentialUnitsofMarketHousingcomprised\n\
             within the Development;",
        ),
    ]
    .into_iter()
    .map(|(p, t)| (p, t.to_string()))
    .collect()
}

const DEV: &str = "Southwark-19_AP_1773";
const SR: Tenure = Tenure::SocialRent;
const INT: Tenure = Tenure::Intermediate;
const LSO: Tenure = Tenure::LondonSharedOwnership;

fn row<'a>(r: &'a Reconciliation, m: &Measure) -> &'a Row {
    r.rows
        .iter()
        .find(|x| &x.measure == m && !x.label.ends_with("(not in deed)"))
        .unwrap_or_else(|| panic!("no row for {m:?}"))
}

fn unit(id: usize, tenure: UnitTenure, rooms: u32, done: bool) -> DeliveredUnit {
    DeliveredUnit {
        id: format!("u{id}"),
        tenure,
        habitable_rooms: Some(rooms),
        completed_on: done.then(|| NaiveDate::from_ymd_opt(2025, 7, 19).unwrap()),
    }
}

/// Synthetic units: `market` market units, then affordable units by tenure.
fn delivery(market: usize, affordable: &[(Tenure, usize)], complete: bool) -> Delivery {
    let mut units = vec![];
    for _ in 0..market {
        units.push(unit(units.len(), UnitTenure::Market, 2, complete));
    }
    for (t, n) in affordable {
        for _ in 0..*n {
            units.push(unit(units.len(), UnitTenure::Affordable(*t), 3, complete));
        }
    }
    Delivery {
        record_id: DEV.into(),
        lpa_app_no: Some("19/AP/1773".into()),
        snapshot: SnapshotRef {
            path: "synthetic".into(),
            sha256: "synthetic".into(),
            retrieved_at: "2026-10-13T00:00:00Z".into(),
        },
        as_of: "2026-10-13T00:00:00Z".into(),
        mapping_version: PLD_MAPPING_VERSION.into(),
        status: Some(if complete { "Completed" } else { "Started" }.into()),
        started_on: None,
        completed_on: None,
        scheme_complete: Some(complete),
        permitted: Permitted::default(),
        units,
    }
}

fn run(deeds: &[Agreement], d: Delivery) -> Reconciliation {
    let instruments = deeds
        .iter()
        .map(|a| instrument_obligations(a, None))
        .collect();
    reconcile(DEV, instruments, Some(d)).unwrap()
}

fn full() -> Delivery {
    delivery(177, &[(SR, 58), (LSO, 18)], true)
}

/// A deed of variation that states only what the test gives it.
fn dov() -> Agreement {
    let mut d = southwark();
    d.provenance.sha256 = "dov".into();
    d.instrument.kind = InstrumentKind::DeedOfVariation;
    d.metadata.agreement_date = Some("1 March 2023".into());
    d.housing = Housing::default();
    d.contributions.clear();
    d
}

/// The variation's page text: p2 carries the deletion clause.
fn dov_pages() -> Pages {
    [(
        2,
        "The definition of Bus Contribution is deleted. The definition of Carbon Green Fund \
         Contribution is deleted."
            .to_string(),
    )]
    .into_iter()
    .collect()
}

/// `before`, unchecked, then `v` grounded against `dov_pages`.
fn with_pages(before: &[Agreement], v: &Agreement, d: Delivery) -> Reconciliation {
    let mut instruments: Vec<_> = before
        .iter()
        .map(|a| instrument_obligations(a, None))
        .collect();
    instruments.push(instrument_obligations(v, Some(&dov_pages())));
    reconcile(DEV, instruments, Some(d)).unwrap()
}

fn delete_bus(page: usize) -> Variation {
    Variation {
        op: VariationOp::Delete,
        target: "the definition of \"Bus Contribution\"".into(),
        new_text: None,
        evidence: Some(Evidence {
            page,
            snippet: "The definition of Bus Contribution is deleted".into(),
        }),
    }
}

fn sr_tenure(units: Option<u32>) -> TenureAllocation {
    TenureAllocation {
        tenure: SR,
        label: None,
        units,
        habitable_rooms: None,
        evidence: Some(Evidence {
            page: 3,
            snippet: "Social Rented Housing Units".into(),
        }),
    }
}

fn bus() -> Measure {
    Measure::contribution("Bus Contribution")
}

#[test]
fn statements_carry_the_deed_values_and_citations() {
    let inst = instrument_obligations(&southwark(), None);
    let get = |m: Measure| {
        inst.statements
            .iter()
            .find(|s| s.measure == m)
            .unwrap_or_else(|| panic!("{m:?}"))
    };
    assert_eq!(get(Measure::TotalUnits).value, Some(dec("253")));
    assert_eq!(get(Measure::MarketUnits).value, Some(dec("177")));
    assert_eq!(get(Measure::AffordableUnits).value, Some(dec("76")));
    assert_eq!(
        get(Measure::AffordableHabitableRooms).value,
        Some(dec("271"))
    );
    assert_eq!(
        get(Measure::TenureUnits { tenure: SR }).value,
        Some(dec("58"))
    );
    assert_eq!(
        get(Measure::TenureUnits { tenure: INT }).value,
        Some(dec("18"))
    );
    assert_eq!(
        get(Measure::TenureHabitableRooms { tenure: SR }).value,
        Some(dec("212"))
    );
    let pct = get(Measure::AffordablePct {
        basis: AffordableBasis::HabitableRooms,
    });
    assert_eq!(pct.value, Some(dec("35.75")));
    assert_eq!(pct.evidence.as_ref().unwrap().page, 5);

    let de = get(Measure::contribution(
        "Default Employment in the End Use Contribution",
    ));
    assert_eq!(de.value, Some(dec("73100")));
    assert_eq!(
        de.label,
        "Default Employment in the End Use Contribution (£)"
    );
    assert!(de.note.as_deref().unwrap().starts_with("conditional"));
    // A formula contribution has no stated sum.
    let ciet = get(Measure::contribution(
        "Construction Industry Employment and Training Contribution",
    ));
    assert_eq!(ciet.value, None);
    assert!(!inst.variation);
    assert_eq!(inst.planning_references, ["19/AP/1773"]);
    assert!(
        inst.statements
            .iter()
            .all(|s| s.grounding == Grounding::NotChecked)
    );
}

#[test]
fn grounding_uses_the_verifier_rules() {
    let inst = instrument_obligations(&southwark(), Some(&southwark_pages()));
    let g = |m: Measure| {
        inst.statements
            .iter()
            .find(|s| s.measure == m)
            .unwrap()
            .grounding
            .clone()
    };
    assert_eq!(g(Measure::AffordableUnits), Grounding::Grounded);
    assert_eq!(g(Measure::MarketUnits), Grounding::Grounded);
    // The recorded citation for 253 is a p44 phrase that doesn't state 253.
    assert!(matches!(g(Measure::TotalUnits), Grounding::Ungrounded(w) if w.contains("not stated")));
}

#[test]
fn ungrounded_deed_value_is_indeterminate_even_when_delivery_matches() {
    let inst = instrument_obligations(&southwark(), Some(&southwark_pages()));
    let r = reconcile(DEV, vec![inst], Some(full())).unwrap();
    let total = row(&r, &Measure::TotalUnits);
    assert_eq!(total.delivered.value, Some(dec("253")));
    assert_eq!(total.status, RowStatus::Indeterminate);
    assert!(total.reason.starts_with("deed value unproven"));
    assert_eq!(
        row(&r, &Measure::AffordableUnits).status,
        RowStatus::CountsMatch
    );
}

#[test]
fn matching_delivery_counts_match_and_intermediate_covers_london_shared_ownership() {
    let r = run(&[southwark()], full());
    for m in [
        Measure::TotalUnits,
        Measure::MarketUnits,
        Measure::AffordableUnits,
        Measure::TenureUnits { tenure: SR },
        Measure::TenureUnits { tenure: INT },
    ] {
        assert_eq!(row(&r, &m).status, RowStatus::CountsMatch, "{m:?}");
    }
    assert!(!r.rows.iter().any(|x| x.label.ends_with("(not in deed)")));
    // Synthetic rooms (3 per affordable unit) don't match the deed's 271: a difference to review.
    let rooms = row(&r, &Measure::AffordableHabitableRooms);
    assert_eq!(rooms.delivered.value, Some(dec("228")));
    assert_eq!(rooms.status, RowStatus::DiffersReviewRequired);
    // Contributions have no PLD delivery source.
    let b = row(&r, &bus());
    assert_eq!(b.status, RowStatus::Indeterminate);
    assert_eq!(b.delivered.basis, "PLD records no S106 payments");
    assert!(r.warnings.is_empty(), "{:?}", r.warnings);
}

#[test]
fn shortfall_on_a_complete_scheme_goes_to_review_never_unmet() {
    let r = run(&[southwark()], delivery(187, &[(SR, 48), (INT, 18)], true));
    let sr = row(&r, &Measure::TenureUnits { tenure: SR });
    assert_eq!(sr.delivered.value, Some(dec("48")));
    assert_eq!(sr.status, RowStatus::DiffersReviewRequired);
    assert!(sr.reason.contains("check the borough's records"));
    assert!(markdown(&r).contains("never a finding that an obligation was unmet"));
}

#[test]
fn shortfall_before_completion_is_not_yet_due() {
    let mut d = delivery(177, &[(SR, 58), (INT, 18)], false);
    for u in d.units.iter_mut().take(100) {
        u.completed_on = Some(NaiveDate::from_ymd_opt(2024, 1, 1).unwrap());
    }
    let r = run(&[southwark()], d);
    let market = row(&r, &Measure::MarketUnits);
    assert_eq!(market.delivered.value, Some(dec("100")));
    assert_eq!(market.status, RowStatus::NotYetDue);
}

#[test]
fn social_rent_does_not_cover_london_affordable_rent() {
    let r = run(
        &[southwark()],
        delivery(177, &[(Tenure::LondonAffordableRent, 58), (INT, 18)], true),
    );
    let sr = row(&r, &Measure::TenureUnits { tenure: SR });
    assert_eq!(sr.delivered.value, Some(dec("0")));
    assert_eq!(sr.status, RowStatus::DiffersReviewRequired);
    let extra = r
        .rows
        .iter()
        .find(|x| x.label == "London Affordable Rent units (not in deed)")
        .unwrap();
    assert_eq!(extra.delivered.value, Some(dec("58")));
    assert_eq!(extra.status, RowStatus::DiffersReviewRequired);
}

#[test]
fn a_deed_without_a_tenure_split_gets_no_not_in_deed_rows() {
    let mut a = southwark();
    a.housing.tenures.clear();
    let r = run(&[a], full());
    assert!(!r.rows.iter().any(|x| x.label.ends_with("(not in deed)")));
    assert_eq!(
        row(&r, &Measure::AffordableUnits).status,
        RowStatus::CountsMatch
    );
}

#[test]
fn unrecognised_or_missing_tenure_blocks_tenure_counts_but_not_the_total() {
    let mut d = delivery(177, &[(SR, 57), (INT, 18)], true);
    d.units.push(unit(
        999,
        UnitTenure::Unrecognised("Key worker".into()),
        2,
        true,
    ));
    let r = run(&[southwark()], d.clone());
    assert_eq!(row(&r, &Measure::TotalUnits).status, RowStatus::CountsMatch);
    let sr = row(&r, &Measure::TenureUnits { tenure: SR });
    assert_eq!(sr.status, RowStatus::Indeterminate);
    assert!(sr.delivered.basis.contains("unrecognised tenure"));

    d.units.last_mut().unwrap().tenure = UnitTenure::Missing;
    let r = run(&[southwark()], d);
    let sr = row(&r, &Measure::TenureUnits { tenure: SR });
    assert_eq!(sr.status, RowStatus::Indeterminate);
    assert!(sr.delivered.basis.contains("give no tenure"));
}

#[test]
fn a_replaced_figure_becomes_the_current_obligation() {
    let mut v = dov();
    v.housing.tenures = vec![sr_tenure(Some(50))];
    let r = run(
        &[southwark(), v.clone()],
        delivery(177, &[(SR, 50), (INT, 18)], true),
    );
    assert_eq!(r.instruments.len(), 2);
    let sr = row(&r, &Measure::TenureUnits { tenure: SR });
    assert_eq!((sr.agreed, sr.current), (Some(dec("58")), Some(dec("50"))));
    assert_eq!(
        sr.events[1].op,
        Op::Replace {
            from: Some(dec("58")),
            to: dec("50")
        }
    );
    assert_eq!(sr.status, RowStatus::CountsMatch);
    // Other obligations the variation doesn't mention are unchanged.
    assert_eq!(row(&r, &Measure::MarketUnits).events.len(), 1);

    let r = run(
        &[southwark(), v],
        delivery(177, &[(SR, 45), (INT, 18)], true),
    );
    assert_eq!(
        row(&r, &Measure::TenureUnits { tenure: SR }).status,
        RowStatus::Varied
    );
}

#[test]
fn a_restated_figure_is_not_a_variation() {
    let mut v = dov();
    v.housing.tenures = vec![sr_tenure(Some(58))];
    let r = run(
        &[southwark(), v.clone()],
        delivery(177, &[(SR, 48), (INT, 18)], true),
    );
    let sr = row(&r, &Measure::TenureUnits { tenure: SR });
    assert_eq!(sr.events[1].op, Op::Restate { value: dec("58") });
    // A restatement doesn't hide a shortfall behind "compare by hand".
    assert_eq!(sr.status, RowStatus::DiffersReviewRequired);

    // Missing delivery data is indeterminate, restated or not.
    let mut d = full();
    d.units.clear();
    let r = run(&[southwark(), v], d);
    assert_eq!(
        row(&r, &Measure::TenureUnits { tenure: SR }).status,
        RowStatus::Indeterminate
    );
}

#[test]
fn a_variation_citing_an_obligation_without_a_figure_leaves_it_unchanged() {
    let mut v = dov();
    v.housing.tenures = vec![sr_tenure(None)];
    let r = run(&[southwark(), v], full());
    let sr = row(&r, &Measure::TenureUnits { tenure: SR });
    assert_eq!(sr.events[1].op, Op::Unchanged);
    assert_eq!(sr.current, Some(dec("58")));
    assert_eq!(sr.status, RowStatus::CountsMatch);
}

#[test]
fn a_deletion_removes_the_obligation() {
    let mut v = dov();
    let mut del = delete_bus(2);
    del.target = "the definition of \"Bus Contribution\" and paragraph 2 of Schedule 4".into();
    v.variations = vec![del];
    let r = with_pages(&[southwark()], &v, full());
    let b = row(&r, &bus());
    assert_eq!(b.status, RowStatus::Removed);
    assert_eq!(b.current, None);
    assert_eq!(b.events.last().unwrap().op, Op::Delete);
    // Nothing else is touched.
    assert_eq!(
        row(&r, &Measure::MarketUnits).status,
        RowStatus::CountsMatch
    );
    assert!(r.warnings.is_empty(), "{:?}", r.warnings);
}

#[test]
fn an_unmatched_deletion_sends_earlier_obligations_to_review() {
    let mut v = dov();
    v.variations = vec![Variation {
        op: VariationOp::Delete,
        target: "paragraph 4.3 of Schedule 2".into(),
        new_text: None,
        evidence: None,
    }];
    let r = run(&[southwark(), v], full());
    let market = row(&r, &Measure::MarketUnits);
    assert_eq!(market.status, RowStatus::Varied);
    assert!(market.reason.contains("paragraph 4.3 of Schedule 2"));
    assert!(r.warnings.iter().any(|w| w.contains("isn't matched")));
}

#[test]
fn duplicates_in_one_deed_merge_or_conflict() {
    let mut a = southwark();
    let mut again = a
        .contributions
        .iter()
        .find(|c| c.name == "Bus Contribution")
        .unwrap()
        .clone();
    again.name = "the Bus Contribution".into();
    a.contributions.push(again.clone());
    let inst = instrument_obligations(&a, None);
    let b: Vec<&Statement> = inst
        .statements
        .iter()
        .filter(|s| s.measure == bus())
        .collect();
    assert_eq!(b.len(), 1);
    assert_eq!(b[0].value, Some(dec("683100")));
    assert!(b[0].note.as_deref().unwrap().contains("stated 2 times"));

    again.amount.value_gbp = Some(dec("100000"));
    a.contributions.push(again);
    let r = run(&[a], full());
    let b = row(&r, &bus());
    assert_eq!(b.status, RowStatus::Indeterminate);
    assert!(
        b.reason
            .contains("different figures: 683100, 683100, 100000")
    );
    assert_eq!(b.agreed, None);
}

#[test]
fn contribution_names_match_across_instruments_after_normalising() {
    let mut v = dov();
    let mut c = southwark()
        .contributions
        .into_iter()
        .find(|c| c.name == "Bus Contribution")
        .unwrap();
    c.name = "the BUS contribution".into();
    c.amount.value_gbp = Some(dec("700000"));
    v.contributions = vec![c];
    let r = run(&[southwark(), v], full());
    let b = row(&r, &bus());
    assert_eq!(
        (b.agreed, b.current),
        (Some(dec("683100")), Some(dec("700000")))
    );
    assert_eq!(r.rows.iter().filter(|x| x.measure == bus()).count(), 1);
}

#[test]
fn a_pld_record_for_another_scheme_is_refused() {
    let one = || vec![instrument_obligations(&southwark(), None)];
    let mut d = full();
    d.record_id = "Southwark-20_AP_0001".into();
    let e = reconcile(DEV, one(), Some(d)).unwrap_err().to_string();
    assert!(e.contains("not Southwark-19_AP_1773"), "{e}");

    let mut d = full();
    d.lpa_app_no = Some("20/AP/0001".into());
    let e = reconcile(DEV, one(), Some(d)).unwrap_err().to_string();
    assert!(e.contains("20/AP/0001"), "{e}");

    // Separators and case don't matter.
    let mut d = full();
    d.lpa_app_no = Some("19-ap-1773".into());
    assert!(reconcile(DEV, one(), Some(d)).is_ok());
}

#[test]
fn out_of_order_instruments_are_flagged() {
    let mut v = dov();
    v.metadata.agreement_date = Some("1 March 2019".into());
    let r = run(&[southwark(), v], full());
    assert!(
        r.warnings.iter().any(|w| w.contains("before instrument 1")),
        "{:?}",
        r.warnings
    );
}

#[test]
fn reconciliation_round_trips_as_json() {
    let r = run(&[southwark()], full());
    let json = serde_json::to_string(&r).unwrap();
    let back: Reconciliation = serde_json::from_str(&json).unwrap();
    assert_eq!(back.rows.len(), r.rows.len());
    assert_eq!(back.rows[0].measure, r.rows[0].measure);
}

/// Every combination of the rules for a stated, grounded figure: replaced or not, delivered
/// missing / short / equal / over, scheme complete / not / unknown.
#[test]
fn decision_table() {
    use RowStatus::*;
    let current = dec("10");
    let delivered = |d: &str| match d {
        "none" => Counted::none("no PLD unit records"),
        "less" => Counted {
            value: Some(dec("8")),
            basis: "x".into(),
        },
        "equal" => Counted {
            value: Some(dec("10")),
            basis: "x".into(),
        },
        "more" => Counted {
            value: Some(dec("12")),
            basis: "x".into(),
        },
        _ => unreachable!(),
    };
    let complete = |c: &str| match c {
        "yes" => Some(true),
        "no" => Some(false),
        _ => None,
    };
    #[rustfmt::skip]
    let table: &[(bool, &str, &str, RowStatus)] = &[
        // replaced, delivered, complete, expected
        (false, "none",  "yes", Indeterminate),
        (false, "none",  "no",  Indeterminate),
        (false, "none",  "?",   Indeterminate),
        (false, "less",  "yes", DiffersReviewRequired),
        (false, "less",  "no",  NotYetDue),
        (false, "less",  "?",   Indeterminate),
        (false, "equal", "yes", CountsMatch),
        (false, "equal", "no",  CountsMatch),
        (false, "equal", "?",   CountsMatch),
        (false, "more",  "yes", DiffersReviewRequired),
        (false, "more",  "no",  DiffersReviewRequired),
        (false, "more",  "?",   Indeterminate),
        (true,  "none",  "yes", Indeterminate),
        (true,  "none",  "no",  Indeterminate),
        (true,  "none",  "?",   Indeterminate),
        (true,  "less",  "yes", Varied),
        (true,  "less",  "no",  Varied),
        (true,  "less",  "?",   Varied),
        (true,  "equal", "yes", CountsMatch),
        (true,  "equal", "no",  CountsMatch),
        (true,  "equal", "?",   CountsMatch),
        (true,  "more",  "yes", Varied),
        (true,  "more",  "no",  Varied),
        (true,  "more",  "?",   Varied),
    ];
    for &(replaced, d, c, want) in table {
        let mut f = Facts {
            current: Current::Value(current),
            grounding: Grounding::Grounded,
            replaced,
            unmatched_deletion: None,
            unproven_deletion: None,
            unverified_change: None,
            delivered: delivered(d),
            percentage: false,
            scheme_complete: complete(c),
        };
        assert_eq!(
            decide(&f).0,
            want,
            "replaced={replaced} delivered={d} complete={c}"
        );
        // Not checked against pages: the same as grounded.
        f.grounding = Grounding::NotChecked;
        assert_eq!(decide(&f).0, want);
        // Every other state of the obligation wins over delivery.
        f.grounding = Grounding::Ungrounded("why".into());
        assert_eq!(decide(&f).0, Indeterminate);
        f.grounding = Grounding::Grounded;
        f.unmatched_deletion = Some("clause 9".into());
        assert_eq!(decide(&f).0, Varied);
        f.unmatched_deletion = None;
        f.unproven_deletion = Some("not on p999".into());
        assert_eq!(decide(&f).0, Varied);
        f.unproven_deletion = None;
        f.unverified_change = Some("a deed".into());
        assert_eq!(decide(&f).0, Varied);
        f.unverified_change = None;
        for (state, status) in [
            (Current::Removed, Removed),
            (Current::Conflict("two figures".into()), Indeterminate),
            (Current::NoFigure(None), Indeterminate),
        ] {
            f.current = state;
            assert_eq!(decide(&f).0, status);
        }
    }
}

#[test]
fn percentages_compare_within_the_verifier_tolerance() {
    let f = |deed: &str, delivered: &str| Facts {
        current: Current::Value(dec(deed)),
        grounding: Grounding::Grounded,
        replaced: false,
        unmatched_deletion: None,
        unproven_deletion: None,
        unverified_change: None,
        delivered: Counted {
            value: Some(dec(delivered)),
            basis: "x".into(),
        },
        percentage: true,
        scheme_complete: Some(true),
    };
    assert_eq!(decide(&f("35.7", "35.75")).0, RowStatus::CountsMatch);
    assert_eq!(
        decide(&f("36", "35.75")).0,
        RowStatus::DiffersReviewRequired
    );
    let mut exact = f("35.7", "35.75");
    exact.percentage = false;
    assert_eq!(decide(&exact).0, RowStatus::DiffersReviewRequired);
}

#[test]
fn snapshots_are_saved_verbatim_and_never_overwritten() {
    let dir = tempfile::tempdir().unwrap();
    let snap = Snapshot {
        request: Request {
            method: "GET".into(),
            url: format!("{}/applications/_source/{DEV}", pld::BASE_URL),
            body: None,
        },
        retrieved_at: "2026-10-13T20:00:00Z".into(),
        response: serde_json::json!({"id": DEV}),
    };
    let r = pld::save(dir.path(), "applications", DEV, &snap).unwrap();
    assert_eq!(
        r.path,
        "snapshots/pld/applications/Southwark-19_AP_1773/2026-10-13T20_00_00Z.json"
    );
    let (back, r2) = pld::load(dir.path(), Path::new(&r.path)).unwrap();
    assert_eq!(r, r2);
    assert_eq!(back.response, snap.response);
    assert!(pld::save(dir.path(), "applications", DEV, &snap).is_err());
}

fn rd(r: &mut serde_json::Value) -> &mut serde_json::Value {
    &mut r["application_details"]["residential_details"]
}

const SNAPSHOT: &str = "snapshots/pld/applications/Southwark-19_AP_1773/2026-10-13T20_51_14Z.json";

/// The saved PLD application snapshot for 19/AP/1773 (real data, retrieved 13 Oct 2026).
fn pld_snapshot() -> (Snapshot, SnapshotRef) {
    pld::load(Path::new(env!("CARGO_MANIFEST_DIR")), Path::new(SNAPSHOT)).unwrap()
}

fn pld_southwark() -> Delivery {
    let (snap, at) = pld_snapshot();
    delivery_from_pld(&snap, at).unwrap()
}

#[test]
fn pld_snapshot_maps_to_delivery() {
    let d = pld_southwark();
    assert_eq!(d.record_id, DEV);
    assert_eq!(d.lpa_app_no.as_deref(), Some("19/AP/1773"));
    assert_eq!(d.as_of, "2026-10-13T20:51:14Z");
    assert_eq!(d.snapshot.path, SNAPSHOT);
    assert_eq!(d.status.as_deref(), Some("Completed"));
    assert_eq!(d.scheme_complete, Some(true));
    assert_eq!(d.started_on, NaiveDate::from_ymd_opt(2022, 5, 23));
    assert_eq!(d.completed_on, NaiveDate::from_ymd_opt(2025, 7, 19));
    assert_eq!(d.permitted.total_units, Some(253));
    assert_eq!(d.permitted.market_units, Some(177));
    assert_eq!(d.permitted.affordable_units, Some(76));
    assert!(d.permitted.tenure_units.contains(&(SR, 58)));
    assert!(d.permitted.tenure_units.contains(&(LSO, 18)));
    assert_eq!(d.units.len(), 253);
    let n = |t: &UnitTenure| d.units.iter().filter(|u| &u.tenure == t).count();
    assert_eq!(n(&UnitTenure::Market), 177);
    assert_eq!(n(&UnitTenure::Affordable(SR)), 58);
    assert_eq!(n(&UnitTenure::Affordable(LSO)), 18);
    assert!(
        d.units
            .iter()
            .all(|u| u.completed_on.is_some() && u.habitable_rooms.is_none())
    );
}

#[test]
fn pld_tenures_and_values_are_read_strictly() {
    assert_eq!(
        pld_unit_tenure(Some("Shared Equity")),
        UnitTenure::Unrecognised("Shared Equity".into())
    );
    assert_eq!(
        pld_unit_tenure(Some("social rent")),
        UnitTenure::Affordable(SR)
    );
    assert_eq!(pld_unit_tenure(None), UnitTenure::Missing);
    assert_eq!(pld_unit_tenure(Some(" ")), UnitTenure::Missing);

    let (snap, at) = pld_snapshot();

    // Numeric strings and status case are accepted.
    let mut s = snap.clone();
    rd(&mut s.response)["total_no_proposed_residential_units"] = "253".into();
    s.response["status"] = "COMPLETED".into();
    let d = delivery_from_pld(&s, at.clone()).unwrap();
    assert_eq!(d.permitted.total_units, Some(253));
    assert_eq!(d.scheme_complete, Some(true));
    // A field the mapping needs going missing is an error, not a quiet "indeterminate".
    let mut s = snap.clone();
    rd(&mut s.response)
        .as_object_mut()
        .unwrap()
        .remove("total_no_proposed_residential_units_social_rent");
    let e = format!("{:#}", delivery_from_pld(&s, at.clone()).unwrap_err());
    assert!(
        e.contains("total_no_proposed_residential_units_social_rent"),
        "{e}"
    );
    let mut s = snap.clone();
    rd(&mut s.response)["residential_units"][0]["actual_completion_date"] = "2025-07-19".into();
    assert!(delivery_from_pld(&s, at.clone()).is_err());
    let mut s = snap;
    rd(&mut s.response)["total_no_proposed_residential_units"] = 2.5.into();
    assert!(delivery_from_pld(&s, at).is_err());
}

#[test]
fn southwark_against_pld_snapshot() {
    let r = reconcile(
        DEV,
        vec![instrument_obligations(&southwark(), None)],
        Some(pld_southwark()),
    )
    .unwrap();
    let delivered = |m: Measure| row(&r, &m).delivered.value;
    assert_eq!(delivered(Measure::TotalUnits), Some(dec("253")));
    assert_eq!(delivered(Measure::MarketUnits), Some(dec("177")));
    assert_eq!(delivered(Measure::AffordableUnits), Some(dec("76")));
    assert_eq!(
        delivered(Measure::TenureUnits { tenure: SR }),
        Some(dec("58"))
    );
    assert_eq!(
        delivered(Measure::TenureUnits { tenure: INT }),
        Some(dec("18"))
    );
    assert_eq!(delivered(Measure::AffordableHabitableRooms), None);
    assert!(!r.rows.iter().any(|x| x.label.ends_with("(not in deed)")));
    assert!(r.warnings.is_empty(), "{:?}", r.warnings);
}

#[test]
fn a_deed_for_another_development_is_refused() {
    let mut v = dov();
    v.metadata.planning_references = vec!["20/AP/0001".into()];
    v.housing.tenures = vec![sr_tenure(Some(50))];
    let instruments = vec![
        instrument_obligations(&southwark(), None),
        instrument_obligations(&v, None),
    ];
    let e = reconcile(
        DEV,
        instruments.clone(),
        Some(delivery(177, &[(SR, 50), (INT, 18)], true)),
    )
    .unwrap_err()
    .to_string();
    assert!(
        e.contains("instrument 2") && e.contains("20/AP/0001"),
        "{e}"
    );
    // Without PLD, the first deed's reference is the one checked.
    assert!(reconcile(DEV, instruments, None).is_err());
}

#[test]
fn a_later_deed_without_a_planning_reference_goes_to_review() {
    let mut v = dov();
    v.metadata.planning_references.clear();
    v.housing.tenures = vec![sr_tenure(Some(50))];
    let r = run(
        &[southwark(), v],
        delivery(177, &[(SR, 50), (INT, 18)], true),
    );
    let sr = row(&r, &Measure::TenureUnits { tenure: SR });
    assert_eq!(sr.status, RowStatus::Varied);
    assert!(
        sr.reason.contains("states no planning reference"),
        "{}",
        sr.reason
    );
    assert!(
        r.warnings
            .iter()
            .any(|w| w.contains("instrument 2 states no planning reference"))
    );
    // Obligations it doesn't touch are unaffected.
    assert_eq!(
        row(&r, &Measure::MarketUnits).status,
        RowStatus::CountsMatch
    );
}

#[test]
fn an_unproven_deletion_is_not_applied() {
    // Cites a page the deed doesn't have.
    let mut v = dov();
    v.variations = vec![delete_bus(999)];
    let r = with_pages(&[southwark()], &v, full());
    let b = row(&r, &bus());
    assert_eq!(b.status, RowStatus::Varied);
    assert_eq!(b.current, Some(dec("683100")));
    assert!(
        b.reason.contains("not re-locatable on p999"),
        "{}",
        b.reason
    );
    // Not checked against pages at all: also not applied.
    let r = run(&[southwark(), v], full());
    assert_eq!(row(&r, &bus()).status, RowStatus::Varied);
}

#[test]
fn a_later_citation_without_a_figure_keeps_a_deletion() {
    let mut v = dov();
    v.variations = vec![delete_bus(2)];
    let mut cited = dov();
    cited.provenance.sha256 = "dov2".into();
    cited.metadata.agreement_date = Some("1 March 2024".into());
    let mut c = southwark()
        .contributions
        .into_iter()
        .find(|c| c.amount.stated_sum_gbp().is_none() && c.amount.evidence.is_some())
        .unwrap();
    c.name = "Bus Contribution".into();
    cited.contributions = vec![c];
    let mut instruments = vec![
        instrument_obligations(&southwark(), None),
        instrument_obligations(&v, Some(&dov_pages())),
    ];
    instruments.push(instrument_obligations(&cited, None));
    let r = reconcile(DEV, instruments, Some(full())).unwrap();
    let b = row(&r, &bus());
    let ops: Vec<&Op> = b.events.iter().map(|e| &e.op).collect();
    assert!(
        matches!(ops[..], [Op::Introduce { .. }, Op::Delete, Op::Unchanged]),
        "{ops:?}"
    );
    assert_eq!(b.status, RowStatus::Removed);
}

#[test]
fn a_later_figure_resolves_an_earlier_conflict() {
    let mut a = southwark();
    let mut other = sr_tenure(Some(60));
    other.evidence = Some(Evidence {
        page: 31,
        snippet: "60 Social Rented Units".into(),
    });
    a.housing.tenures.push(other);
    let mut v = dov();
    v.housing.tenures = vec![sr_tenure(Some(50))];
    let r = run(&[a.clone(), v], delivery(177, &[(SR, 50), (INT, 18)], true));
    let sr = row(&r, &Measure::TenureUnits { tenure: SR });
    assert_eq!(sr.current, Some(dec("50")));
    assert_eq!(sr.status, RowStatus::CountsMatch);
    // The conflict stays in the history.
    assert!(sr.events[0].conflict.as_deref().unwrap().contains("58, 60"));
    // With no later figure it is still indeterminate.
    let r = run(&[a], full());
    assert_eq!(
        row(&r, &Measure::TenureUnits { tenure: SR }).status,
        RowStatus::Indeterminate
    );
}

#[test]
fn a_deletion_snippet_must_name_what_it_deletes() {
    // The target says Bus Contribution, but the snippet is the Carbon Green Fund clause.
    let mut v = dov();
    let mut del = delete_bus(2);
    del.evidence = Some(Evidence {
        page: 2,
        snippet: "The definition of Carbon Green Fund Contribution is deleted".into(),
    });
    v.variations = vec![del];
    let r = with_pages(&[southwark()], &v, full());
    let b = row(&r, &bus());
    assert_eq!(b.status, RowStatus::Varied);
    assert!(
        b.reason.contains("doesn't name it: the Bus Contribution"),
        "{}",
        b.reason
    );
    // A bare "is deleted" names nothing.
    let mut del = delete_bus(2);
    del.evidence.as_mut().unwrap().snippet = "is deleted".into();
    v.variations = vec![del];
    let r = with_pages(&[southwark()], &v, full());
    assert_eq!(row(&r, &bus()).status, RowStatus::Varied);
    // A snippet that names it but doesn't delete anything isn't a deletion either.
    let mut del = delete_bus(2);
    del.evidence.as_mut().unwrap().snippet = "The definition of Bus Contribution".into();
    v.variations = vec![del];
    let r = with_pages(&[southwark()], &v, full());
    assert!(
        row(&r, &bus())
            .reason
            .contains("doesn't say anything is deleted")
    );
}

#[test]
fn an_obligation_introduced_by_an_unverified_deed_goes_to_review() {
    let mut v = dov();
    v.metadata.planning_references.clear();
    let mut a = southwark();
    a.housing.tenures.retain(|t| t.tenure != SR);
    v.housing.tenures = vec![sr_tenure(Some(58))];
    let r = run(&[a, v], full());
    let sr = row(&r, &Measure::TenureUnits { tenure: SR });
    assert!(matches!(sr.events[0].op, Op::Introduce { .. }));
    assert_eq!(sr.status, RowStatus::Varied);
    assert!(
        sr.reason.contains("states no planning reference"),
        "{}",
        sr.reason
    );
}

#[test]
fn a_verified_figure_clears_an_unverified_change() {
    let mut unverified = dov();
    unverified.metadata.planning_references.clear();
    unverified.housing.tenures = vec![sr_tenure(Some(50))];
    let mut verified = dov();
    verified.provenance.sha256 = "dov2".into();
    verified.metadata.agreement_date = Some("1 March 2024".into());
    verified.housing.tenures = vec![sr_tenure(Some(55))];
    let r = run(
        &[southwark(), unverified, verified],
        delivery(177, &[(SR, 55), (INT, 18)], true),
    );
    let sr = row(&r, &Measure::TenureUnits { tenure: SR });
    assert_eq!(sr.current, Some(dec("55")));
    assert_eq!(sr.status, RowStatus::CountsMatch);
    // The history still shows the unverified step.
    assert!(!sr.events[1].verified && sr.events[2].verified);
}

#[test]
fn a_deletion_must_apply_to_the_named_obligation() {
    let deleting = |snippet: &str| {
        let mut v = dov();
        let mut del = delete_bus(2);
        del.evidence.as_mut().unwrap().snippet = snippet.into();
        v.variations = vec![del];
        let pages: Pages = [(2, snippet.to_string())].into_iter().collect();
        let instruments = vec![
            instrument_obligations(&southwark(), None),
            instrument_obligations(&v, Some(&pages)),
        ];
        let r = reconcile(DEV, instruments, Some(full())).unwrap();
        row(&r, &bus()).clone()
    };
    // The deletion is in a different clause from the one naming Bus Contribution.
    let b = deleting(
        "The Bus Contribution is retained. The Carbon Green Fund Contribution is deleted.",
    );
    assert_eq!(b.status, RowStatus::Varied);
    assert!(
        b.reason.contains("doesn't clearly say it is deleted"),
        "{}",
        b.reason
    );
    // One clause that keeps it and deletes something else.
    let b =
        deleting("The Carbon Green Fund Contribution is deleted but the Bus Contribution is not");
    assert_eq!(b.status, RowStatus::Varied);
    // One clause deleting both is proof for each.
    let b = deleting("The Bus Contribution and the Carbon Green Fund Contribution are deleted");
    assert_eq!(b.status, RowStatus::Removed);
}

#[test]
fn pld_fields_must_be_present_even_when_null() {
    let (snap, at) = pld_snapshot();
    // Renamed: an error, not 253 units silently uncompleted.
    let mut s = snap.clone();
    let u = rd(&mut s.response)["residential_units"][0]
        .as_object_mut()
        .unwrap();
    let v = u.remove("actual_completion_date").unwrap();
    u.insert("completion_date".into(), v);
    let e = format!("{:#}", delivery_from_pld(&s, at.clone()).unwrap_err());
    assert!(e.contains("actual_completion_date"), "{e}");
    let mut s = snap.clone();
    s.response.as_object_mut().unwrap().remove("status");
    assert!(delivery_from_pld(&s, at.clone()).is_err());
    // An explicit null is a unit not yet completed.
    let mut s = snap;
    rd(&mut s.response)["residential_units"][0]["actual_completion_date"] = serde_json::Value::Null;
    let d = delivery_from_pld(&s, at).unwrap();
    assert_eq!(
        d.units.iter().filter(|u| u.completed_on.is_none()).count(),
        1
    );
}
