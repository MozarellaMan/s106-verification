//! Reconciliation: obligation events from an invented deed (`tests/synthetic/agreement`), and
//! status rules against delivery records. Delivery is synthetic too: unit lists built here, and
//! an invented Planning London Datahub record in `tests/synthetic/pld/application.json` with the
//! field names the mapping reads.

mod common;

use std::path::Path;

use chrono::NaiveDate;
use common::synthetic;
use rust_decimal::Decimal;
use s106::evidence::Pages;
use s106::pld::{self, Request, Snapshot, SnapshotRef};
use s106::reconcile::*;
use s106::schema::*;

fn dec(s: &str) -> Decimal {
    s.parse().unwrap()
}

fn gbp(s: &str) -> Observed<Decimal> {
    Observed {
        raw: s.into(),
        parsed: Ok(dec(s)),
    }
}

/// The invented agreement: 120 homes, 84 market and 36 affordable (24 Social Rent, 12 Shared
/// Ownership), and five contributions. Dated in full so the date-order check can read it.
fn original() -> Agreement {
    let mut a = synthetic::agreement("agreement");
    a.metadata.agreement_date = Some("12 March 2024".into());
    a
}

fn pages() -> Pages {
    synthetic::document("agreement").pages()
}

const DEV: &str = "Exampleton-EX_23_0456";
const SR: Tenure = Tenure::SocialRent;
const SO: Tenure = Tenure::SharedOwnership;
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
        lpa_app_no: Some("EX/23/0456".into()),
        snapshot: SnapshotRef {
            path: "synthetic".into(),
            sha256: "synthetic".into(),
            retrieved_at: "2026-01-02T03:04:05Z".into(),
        },
        as_of: "2026-01-02T03:04:05Z".into(),
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

/// Everything the deed asks for, delivered and complete, Shared Ownership as London Shared
/// Ownership.
fn full() -> Delivery {
    delivery(84, &[(SR, 24), (LSO, 12)], true)
}

/// A deed of variation that states only what the test gives it.
fn dov() -> Agreement {
    let mut d = original();
    d.provenance.sha256 = "dov".into();
    d.instrument.kind = InstrumentKind::DeedOfVariation;
    d.metadata.agreement_date = Some("1 March 2025".into());
    d.housing = Housing::default();
    d.contributions.clear();
    d
}

fn sr_tenure(units: Option<u32>) -> TenureAllocation {
    TenureAllocation {
        tenure: SR,
        label: None,
        units,
        habitable_rooms: None,
        evidence: Some(Evidence {
            page: 2,
            snippet: "Social Rent Units".into(),
        }),
    }
}

fn highways() -> Measure {
    Measure::contribution("Highways Contribution")
}

#[test]
fn statements_carry_the_deed_values_and_citations() {
    let inst = instrument_obligations(&original(), None);
    let get = |m: Measure| {
        inst.statements
            .iter()
            .find(|s| s.measure == m)
            .unwrap_or_else(|| panic!("{m:?}"))
    };
    assert_eq!(get(Measure::TotalUnits).value, Some(dec("120")));
    assert_eq!(get(Measure::MarketUnits).value, Some(dec("84")));
    assert_eq!(get(Measure::AffordableUnits).value, Some(dec("36")));
    assert_eq!(
        get(Measure::AffordableHabitableRooms).value,
        Some(dec("100"))
    );
    assert_eq!(
        get(Measure::TenureUnits { tenure: SR }).value,
        Some(dec("24"))
    );
    assert_eq!(
        get(Measure::TenureUnits { tenure: SO }).value,
        Some(dec("12"))
    );
    assert_eq!(
        get(Measure::TenureHabitableRooms { tenure: SR }).value,
        Some(dec("70"))
    );
    let pct = get(Measure::AffordablePct {
        basis: AffordableBasis::Units,
    });
    assert_eq!(pct.value, Some(dec("30")));
    assert_eq!(pct.evidence.as_ref().unwrap().page, 3);

    let tp = get(Measure::contribution("Travel Plan Contribution"));
    assert_eq!(tp.value, Some(dec("8000")));
    assert_eq!(tp.label, "Travel Plan Contribution (£)");
    assert!(tp.note.as_deref().unwrap().starts_with("conditional"));
    assert_eq!(
        get(Measure::contribution("Play Space Contribution")).value,
        Some(dec("12500.50"))
    );
    assert!(!inst.variation);
    assert_eq!(inst.planning_references, ["EX/23/0456"]);
    assert!(
        inst.statements
            .iter()
            .all(|s| s.grounding == Grounding::NotChecked)
    );
}

/// The deed with its total-units citation pointing at a phrase that doesn't state the figure.
fn with_bad_total_citation() -> Agreement {
    let mut a = original();
    a.housing.total_units_evidence = Some(Evidence {
        page: 3,
        snippet: "residential dwellings (C3)".into(),
    });
    a
}

#[test]
fn grounding_uses_the_verifier_rules() {
    let inst = instrument_obligations(&with_bad_total_citation(), Some(&pages()));
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
    // OCR read the pound sign as "f"; the verifier's matcher still grounds it.
    assert_eq!(g(highways()), Grounding::Grounded);
    assert!(matches!(g(Measure::TotalUnits), Grounding::Ungrounded(w) if w.contains("not stated")));
}

#[test]
fn ungrounded_deed_value_is_indeterminate_even_when_delivery_matches() {
    let inst = instrument_obligations(&with_bad_total_citation(), Some(&pages()));
    let r = reconcile(DEV, vec![inst], Some(full())).unwrap();
    let total = row(&r, &Measure::TotalUnits);
    assert_eq!(total.delivered.value, Some(dec("120")));
    assert_eq!(total.status, RowStatus::Indeterminate);
    assert_eq!(total.cause, Some(Cause::Ungrounded));
    assert!(total.reason.starts_with("deed value unproven"));
    assert_eq!(
        row(&r, &Measure::AffordableUnits).status,
        RowStatus::CountsMatch
    );
}

#[test]
fn matching_delivery_counts_match_and_umbrella_tenures_cover_their_kinds() {
    let r = run(&[original()], full());
    for m in [
        Measure::TotalUnits,
        Measure::MarketUnits,
        Measure::AffordableUnits,
        Measure::TenureUnits { tenure: SR },
        Measure::TenureUnits { tenure: SO },
    ] {
        assert_eq!(row(&r, &m).status, RowStatus::CountsMatch, "{m:?}");
    }
    assert!(!r.rows.iter().any(|x| x.label.ends_with("(not in deed)")));
    // Synthetic rooms (3 per affordable unit) don't match the deed's 100: a difference to review.
    let rooms = row(&r, &Measure::AffordableHabitableRooms);
    assert_eq!(rooms.delivered.value, Some(dec("108")));
    assert_eq!(rooms.status, RowStatus::DiffersReviewRequired);
    // Contributions have no PLD delivery source.
    let h = row(&r, &highways());
    assert_eq!(h.status, RowStatus::Indeterminate);
    assert_eq!(h.cause, Some(Cause::NoPayments));
    assert_eq!(h.delivered.basis, "PLD records no S106 payments");
    assert!(r.warnings.is_empty(), "{:?}", r.warnings);

    // "Intermediate" in a deed covers London Shared Ownership too.
    let mut a = original();
    a.housing.tenures[1].tenure = INT;
    let r = run(&[a], full());
    assert_eq!(
        row(&r, &Measure::TenureUnits { tenure: INT }).status,
        RowStatus::CountsMatch
    );
}

#[test]
fn shortfall_on_a_complete_scheme_goes_to_review_never_unmet() {
    let r = run(&[original()], delivery(94, &[(SR, 14), (LSO, 12)], true));
    let sr = row(&r, &Measure::TenureUnits { tenure: SR });
    assert_eq!(sr.delivered.value, Some(dec("14")));
    assert_eq!(sr.status, RowStatus::DiffersReviewRequired);
    assert!(sr.reason.contains("check the borough's records"));
    assert!(markdown(&r).contains("never a finding that an obligation was unmet"));
    assert!(markdown(&r).contains("checks the borough's data entry, not the obligation"));
}

#[test]
fn shortfall_before_completion_is_not_yet_due() {
    let mut d = delivery(84, &[(SR, 24), (LSO, 12)], false);
    for u in d.units.iter_mut().take(50) {
        u.completed_on = Some(NaiveDate::from_ymd_opt(2024, 1, 1).unwrap());
    }
    let r = run(&[original()], d);
    let market = row(&r, &Measure::MarketUnits);
    assert_eq!(market.delivered.value, Some(dec("50")));
    assert_eq!(market.status, RowStatus::NotYetDue);
}

#[test]
fn social_rent_does_not_cover_london_affordable_rent() {
    let r = run(
        &[original()],
        delivery(84, &[(Tenure::LondonAffordableRent, 24), (LSO, 12)], true),
    );
    let sr = row(&r, &Measure::TenureUnits { tenure: SR });
    assert_eq!(sr.delivered.value, Some(dec("0")));
    assert_eq!(sr.status, RowStatus::DiffersReviewRequired);
    let extra = r
        .rows
        .iter()
        .find(|x| x.label == "London Affordable Rent units (not in deed)")
        .unwrap();
    assert_eq!(extra.delivered.value, Some(dec("24")));
    assert_eq!(extra.status, RowStatus::DiffersReviewRequired);
}

#[test]
fn a_deed_without_a_tenure_split_gets_no_not_in_deed_rows() {
    let mut a = original();
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
    let mut d = delivery(84, &[(SR, 23), (LSO, 12)], true);
    d.units.push(unit(
        999,
        UnitTenure::Unrecognised("Key worker".into()),
        2,
        true,
    ));
    let r = run(&[original()], d.clone());
    assert_eq!(row(&r, &Measure::TotalUnits).status, RowStatus::CountsMatch);
    let sr = row(&r, &Measure::TenureUnits { tenure: SR });
    assert_eq!(sr.status, RowStatus::Indeterminate);
    assert_eq!(sr.cause, Some(Cause::UnknownTenure));
    assert!(sr.delivered.basis.contains("unrecognised tenure"));

    d.units.last_mut().unwrap().tenure = UnitTenure::Missing;
    let r = run(&[original()], d);
    let sr = row(&r, &Measure::TenureUnits { tenure: SR });
    assert_eq!(sr.status, RowStatus::Indeterminate);
    assert!(sr.delivered.basis.contains("give no tenure"));
}

#[test]
fn unnamed_intermediate_counts_as_affordable_but_not_as_a_named_product() {
    // PLD's "Intermediate Other": intermediate homes whose product isn't recorded.
    let d = delivery(84, &[(SR, 24), (INT, 12)], true);
    let r = run(&[original()], d.clone());
    assert_eq!(
        row(&r, &Measure::AffordableUnits).status,
        RowStatus::CountsMatch
    );
    assert_eq!(
        row(&r, &Measure::TenureUnits { tenure: SR }).status,
        RowStatus::CountsMatch
    );
    // The deed's Shared Ownership may or may not be those 12 homes: neither short nor extra.
    let so = row(&r, &Measure::TenureUnits { tenure: SO });
    assert_eq!(so.status, RowStatus::Indeterminate);
    assert_eq!(so.cause, Some(Cause::UnknownTenure));
    assert!(so.delivered.basis.contains("without a named product"));
    assert!(!r.rows.iter().any(|x| x.label.ends_with("(not in deed)")));

    // A deed that says "Intermediate" counts them.
    let mut a = original();
    a.housing.tenures[1].tenure = INT;
    let r = run(&[a], d);
    assert_eq!(
        row(&r, &Measure::TenureUnits { tenure: INT }).status,
        RowStatus::CountsMatch
    );
}

#[test]
fn duplicates_in_one_deed_merge_or_conflict() {
    let mut a = original();
    let mut again = highways_contribution("45000");
    again.name = "the Highways Contribution".into();
    a.contributions.push(again.clone());
    let inst = instrument_obligations(&a, None);
    let h: Vec<&Statement> = inst
        .statements
        .iter()
        .filter(|s| s.measure == highways())
        .collect();
    assert_eq!(h.len(), 1);
    assert_eq!(h[0].value, Some(dec("45000")));
    assert!(h[0].note.as_deref().unwrap().contains("stated 2 times"));

    again.amount.value_gbp = Some(gbp("100000"));
    a.contributions.push(again);
    let r = run(&[a], full());
    let h = row(&r, &highways());
    assert_eq!(h.status, RowStatus::Indeterminate);
    assert!(h.reason.contains("different figures: 45000, 45000, 100000"));
    assert_eq!(h.agreed, None);
}

#[test]
fn a_pld_record_for_another_scheme_is_refused() {
    let one = || vec![instrument_obligations(&original(), None)];
    let mut d = full();
    d.record_id = "Exampleton-EX_23_9999".into();
    let e = reconcile(DEV, one(), Some(d)).unwrap_err().to_string();
    assert!(e.contains("not Exampleton-EX_23_0456"), "{e}");

    let mut d = full();
    d.lpa_app_no = Some("EX/23/9999".into());
    let e = reconcile(DEV, one(), Some(d)).unwrap_err().to_string();
    assert!(e.contains("EX/23/9999"), "{e}");

    // Separators and case don't matter.
    let mut d = full();
    d.lpa_app_no = Some("ex-23-0456".into());
    assert!(reconcile(DEV, one(), Some(d)).is_ok());
}

#[test]
fn out_of_order_instruments_are_flagged() {
    let mut v = dov();
    v.metadata.agreement_date = Some("1 March 2019".into());
    let r = run(&[original(), v], full());
    assert!(
        r.warnings.iter().any(|w| w.contains("before instrument 1")),
        "{:?}",
        r.warnings
    );
}

#[test]
fn reconciliation_round_trips_as_json() {
    let r = run(&[original()], full());
    let json = serde_json::to_string(&r).unwrap();
    let back: Reconciliation = serde_json::from_str(&json).unwrap();
    assert_eq!(back.rows.len(), r.rows.len());
    assert_eq!(back.rows[0].measure, r.rows[0].measure);
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
        retrieved_at: "2026-01-02T03:00:00Z".into(),
        response: serde_json::json!({"id": DEV}),
    };
    let r = pld::save(dir.path(), "applications", DEV, &snap).unwrap();
    assert_eq!(
        r.path,
        "snapshots/pld/applications/Exampleton-EX_23_0456/2026-01-02T03_00_00Z.json"
    );
    let (back, r2) = pld::load(dir.path(), Path::new(&r.path)).unwrap();
    assert_eq!(r, r2);
    assert_eq!(back.response, snap.response);
    assert!(pld::save(dir.path(), "applications", DEV, &snap).is_err());
}

fn rd(r: &mut serde_json::Value) -> &mut serde_json::Value {
    &mut r["application_details"]["residential_details"]
}

/// The invented PLD application record, saved as `s106 pld get` would save it.
fn pld_snapshot() -> (Snapshot, SnapshotRef) {
    pld::load(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/synthetic"),
        Path::new("pld/application.json"),
    )
    .unwrap()
}

fn pld_delivery() -> Delivery {
    let (snap, at) = pld_snapshot();
    delivery_from_pld(&snap, at).unwrap()
}

#[test]
fn pld_snapshot_maps_to_delivery() {
    let d = pld_delivery();
    assert_eq!(d.record_id, DEV);
    assert_eq!(d.lpa_app_no.as_deref(), Some("EX/23/0456"));
    assert_eq!(d.as_of, "2026-01-02T03:04:05Z");
    assert_eq!(d.snapshot.path, "pld/application.json");
    assert_eq!(d.status.as_deref(), Some("Completed"));
    assert_eq!(d.scheme_complete, Some(true));
    assert_eq!(d.started_on, NaiveDate::from_ymd_opt(2023, 6, 1));
    assert_eq!(d.completed_on, NaiveDate::from_ymd_opt(2025, 7, 19));
    assert_eq!(d.permitted.total_units, Some(120));
    assert_eq!(d.permitted.market_units, Some(84));
    assert_eq!(d.permitted.affordable_units, Some(36));
    assert!(d.permitted.tenure_units.contains(&(SR, 24)));
    assert!(d.permitted.tenure_units.contains(&(LSO, 12)));
    // The record's one "Loss" (an existing home demolished) is not delivery.
    assert_eq!(d.units.len(), 120);
    let n = |t: &UnitTenure| d.units.iter().filter(|u| &u.tenure == t).count();
    assert_eq!(n(&UnitTenure::Market), 84);
    assert_eq!(n(&UnitTenure::Affordable(SR)), 24);
    assert_eq!(n(&UnitTenure::Affordable(LSO)), 12);
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
    assert_eq!(
        pld_unit_tenure(Some("Intermediate Other")),
        UnitTenure::Affordable(INT)
    );
    assert_eq!(
        pld_unit_tenure(Some("Affordable Rent (not at LAR benchmark rents)")),
        UnitTenure::Affordable(Tenure::AffordableRent)
    );
    assert_eq!(pld_unit_tenure(None), UnitTenure::Missing);
    assert_eq!(pld_unit_tenure(Some(" ")), UnitTenure::Missing);

    let (snap, at) = pld_snapshot();

    // Numeric strings and status case are accepted.
    let mut s = snap.clone();
    rd(&mut s.response)["total_no_proposed_residential_units"] = "120".into();
    s.response["status"] = "COMPLETED".into();
    let d = delivery_from_pld(&s, at.clone()).unwrap();
    assert_eq!(d.permitted.total_units, Some(120));
    assert_eq!(d.scheme_complete, Some(true));
    // Southwark says "Commenced" where others say "Started"; "Lapsed" stays unknown.
    s.response["status"] = "Commenced".into();
    assert_eq!(
        delivery_from_pld(&s, at.clone()).unwrap().scheme_complete,
        Some(false)
    );
    s.response["status"] = "Lapsed".into();
    assert_eq!(
        delivery_from_pld(&s, at.clone()).unwrap().scheme_complete,
        None
    );
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
fn the_deed_against_the_pld_record() {
    let r = reconcile(
        DEV,
        vec![instrument_obligations(&original(), Some(&pages()))],
        Some(pld_delivery()),
    )
    .unwrap();
    let delivered = |m: Measure| row(&r, &m).delivered.value;
    assert_eq!(delivered(Measure::TotalUnits), Some(dec("120")));
    assert_eq!(delivered(Measure::MarketUnits), Some(dec("84")));
    assert_eq!(delivered(Measure::AffordableUnits), Some(dec("36")));
    assert_eq!(
        delivered(Measure::TenureUnits { tenure: SR }),
        Some(dec("24"))
    );
    assert_eq!(
        delivered(Measure::TenureUnits { tenure: SO }),
        Some(dec("12"))
    );
    assert_eq!(delivered(Measure::AffordableHabitableRooms), None);
    assert_eq!(
        row(&r, &Measure::AffordableHabitableRooms).cause,
        Some(Cause::NoRooms)
    );
    assert!(!r.rows.iter().any(|x| x.label.ends_with("(not in deed)")));
    assert!(r.warnings.is_empty(), "{:?}", r.warnings);
    assert_eq!(r.summary.rows, r.rows.len());
    assert_eq!(
        r.summary.by_status.get(&RowStatus::CountsMatch),
        Some(&6),
        "{}",
        markdown(&r)
    );
}

#[test]
fn a_deed_for_another_development_is_refused() {
    let mut v = dov();
    v.metadata.planning_references = vec!["EX/23/9999".into()];
    v.housing.tenures = vec![sr_tenure(Some(20))];
    let instruments = vec![
        instrument_obligations(&original(), None),
        instrument_obligations(&v, None),
    ];
    let e = reconcile(DEV, instruments.clone(), Some(full()))
        .unwrap_err()
        .to_string();
    assert!(
        e.contains("instrument 2") && e.contains("EX/23/9999"),
        "{e}"
    );
    // Without PLD, the first deed's reference is the one checked.
    assert!(reconcile(DEV, instruments, None).is_err());
}

#[test]
fn pld_fields_must_be_present_even_when_null() {
    let (snap, at) = pld_snapshot();
    // Renamed: an error, not 120 units silently uncompleted.
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

fn highways_contribution(v: &str) -> Contribution {
    let mut c = original()
        .contributions
        .into_iter()
        .find(|c| c.name == "Highways Contribution")
        .unwrap();
    c.amount.value_gbp = Some(gbp(v));
    c
}

fn delete(target: &str) -> Variation {
    Variation {
        op: VariationOp::Delete,
        target: target.into(),
        new_text: None,
        evidence: None,
    }
}

fn codes(r: &Row) -> Vec<&'static str> {
    r.review.iter().map(ReviewReason::code).collect()
}

/// Later deeds are recorded, not applied: each kind of later event, and each deed flag, gives
/// one review reason and "varied: compare by hand". Only a restatement stays automatic.
#[test]
fn later_deeds_go_to_review_by_reason() {
    type Edit = fn(&mut Agreement);
    let cases: &[(&str, Edit, &[&str])] = &[
        (
            "restates the figure",
            |d| d.contributions.push(highways_contribution("45000")),
            &[],
        ),
        (
            "cites it without a figure",
            |d| {
                let mut c = highways_contribution("0");
                c.amount.value_gbp = None;
                d.contributions.push(c)
            },
            &[],
        ),
        (
            "states a different figure",
            |d| d.contributions.push(highways_contribution("60000")),
            &["replaced"],
        ),
        (
            "deletes it",
            |d| d.variations.push(delete("the Highways Contribution")),
            &["deleted"],
        ),
        (
            "restates it with a partial scope",
            |d| {
                d.contributions.push(highways_contribution("45000"));
                d.instrument.scope = Some(Quoted {
                    text: "in so far as it relates to Phase 2".into(),
                    evidence: None,
                });
            },
            &["scoped"],
        ),
        (
            "restates it with no planning reference",
            |d| {
                d.contributions.push(highways_contribution("45000"));
                d.metadata.planning_references.clear();
            },
            &["unverified"],
        ),
        (
            "states two figures",
            |d| {
                d.contributions.push(highways_contribution("45000"));
                d.contributions.push(highways_contribution("60000"));
            },
            &["conflict"],
        ),
        (
            "changes it with a partial scope",
            |d| {
                d.contributions.push(highways_contribution("60000"));
                d.instrument.scope = Some(Quoted {
                    text: "in so far as it relates to Phase 2".into(),
                    evidence: None,
                });
            },
            &["scoped", "replaced"],
        ),
    ];
    let base = row(&run(&[original()], full()), &highways()).status;
    for (what, edit, want) in cases {
        let mut d = dov();
        edit(&mut d);
        let r = run(&[original(), d], full());
        let b = row(&r, &highways());
        assert_eq!(&codes(b), want, "a later deed that {what}");
        let status = if want.is_empty() {
            base
        } else {
            RowStatus::Varied
        };
        assert_eq!(b.status, status, "a later deed that {what}: {}", b.reason);
        assert_eq!(
            b.agreed,
            Some(dec("45000")),
            "the first deed's figure is kept"
        );
        assert!(b.events.len() >= 2, "every deed's statement is recorded");
    }
}

#[test]
fn an_unmatched_deletion_reaches_every_earlier_row() {
    let mut d = dov();
    d.variations.push(delete("the Library Contribution"));
    let r = run(&[original(), d], full());
    for x in &r.rows {
        assert_eq!(codes(x), ["unmatched_deletion"], "{}", x.label);
        assert_eq!(x.status, RowStatus::Varied);
    }
    assert!(row(&r, &highways()).reason.contains("Library Contribution"));
}

#[test]
fn a_later_figure_matching_delivery_counts_match_as_varied() {
    let mut d = dov();
    d.housing.market_units = Some(80);
    let r = run(
        &[original(), d.clone()],
        delivery(80, &[(SR, 24), (LSO, 12)], true),
    );
    let m = row(&r, &Measure::MarketUnits);
    assert_eq!(m.status, RowStatus::CountsMatchVariedFigure);
    assert_eq!(m.status.label(), "counts match (varied figure)");
    assert_eq!(m.agreed, Some(dec("84")));
    assert_eq!(m.current, Some(dec("80")));

    // the same figure from a deed that couldn't be checked is not trusted
    d.metadata.planning_references.clear();
    let r = run(&[original(), d], delivery(80, &[(SR, 24), (LSO, 12)], true));
    assert_eq!(row(&r, &Measure::MarketUnits).status, RowStatus::Varied);
}

#[test]
fn a_conflict_in_the_first_deed_is_indeterminate() {
    let mut a = original();
    a.contributions.push(highways_contribution("60000"));
    let r = run(&[a], full());
    let b = row(&r, &highways());
    assert_eq!(b.status, RowStatus::Indeterminate);
    assert_eq!(b.cause, Some(Cause::Conflict));
}

#[test]
fn the_summary_counts_statuses_causes_and_reasons() {
    let mut d = dov();
    d.contributions.push(highways_contribution("60000"));
    let r = run(&[original(), d], full());
    let s = &r.summary;
    assert_eq!(s, &Summary::of(&r.rows));
    assert_eq!(s.rows, r.rows.len());
    assert_eq!(s.by_status.values().sum::<usize>(), s.rows);
    assert_eq!(
        s.decided,
        r.rows.iter().filter(|x| x.status.decided()).count()
    );
    assert_eq!(s.review_by_reason.get("replaced"), Some(&1));
    assert_eq!(s.by_status.get(&RowStatus::Varied), Some(&1));
    let indeterminate = s
        .by_status
        .get(&RowStatus::Indeterminate)
        .copied()
        .unwrap_or(0);
    assert_eq!(
        s.indeterminate_by_cause.values().sum::<usize>(),
        indeterminate
    );
    let json = serde_json::to_value(&r).unwrap();
    assert_eq!(json["summary"]["review_by_reason"]["replaced"], 1);
    assert!(json["rows"][0]["status"].is_string());
}

fn facts_with(current: Current, delivered: Option<&str>, complete: Option<bool>) -> Facts {
    Facts {
        current,
        grounding: Grounding::Grounded,
        review: vec![],
        latest: None,
        delivered: match delivered {
            Some(v) => Counted::some(dec(v), "delivered"),
            None => Counted::none(Cause::NoPayments, "no payments"),
        },
        percentage: false,
        scheme_complete: complete,
    }
}

#[test]
fn decision_table() {
    use Current::*;
    let v = |s: &str| Value(dec(s));
    let cases: Vec<(Facts, RowStatus, Option<Cause>)> = vec![
        (
            facts_with(v("10"), Some("10"), Some(true)),
            RowStatus::CountsMatch,
            None,
        ),
        (
            facts_with(v("10"), Some("8"), Some(true)),
            RowStatus::DiffersReviewRequired,
            None,
        ),
        (
            facts_with(v("10"), Some("8"), Some(false)),
            RowStatus::NotYetDue,
            None,
        ),
        (
            facts_with(v("10"), Some("8"), None),
            RowStatus::Indeterminate,
            Some(Cause::SchemeStatusUnknown),
        ),
        (
            facts_with(v("10"), None, Some(true)),
            RowStatus::Indeterminate,
            Some(Cause::NoPayments),
        ),
        (
            facts_with(NoFigure(None), Some("10"), Some(true)),
            RowStatus::Indeterminate,
            Some(Cause::NoFigure),
        ),
        (
            facts_with(Conflict("1 or 2".into()), Some("10"), Some(true)),
            RowStatus::Indeterminate,
            Some(Cause::Conflict),
        ),
        (
            Facts {
                grounding: Grounding::Ungrounded("not on page".into()),
                ..facts_with(v("10"), Some("10"), Some(true))
            },
            RowStatus::Indeterminate,
            Some(Cause::Ungrounded),
        ),
        (
            Facts {
                review: vec![ReviewReason::Unverified { instrument: 2 }],
                ..facts_with(v("10"), Some("10"), Some(true))
            },
            RowStatus::Varied,
            None,
        ),
        (
            Facts {
                review: vec![ReviewReason::Replaced {
                    instrument: 2,
                    from: Some(dec("10")),
                    to: dec("8"),
                }],
                latest: Some(dec("8")),
                ..facts_with(v("10"), Some("8"), Some(true))
            },
            RowStatus::CountsMatchVariedFigure,
            None,
        ),
        (
            Facts {
                review: vec![ReviewReason::Replaced {
                    instrument: 2,
                    from: Some(dec("10")),
                    to: dec("8"),
                }],
                latest: Some(dec("8")),
                ..facts_with(v("10"), Some("9"), Some(true))
            },
            RowStatus::Varied,
            None,
        ),
    ];
    for (i, (f, status, cause)) in cases.iter().enumerate() {
        let (s, reason, c) = decide(f);
        assert_eq!((s, c), (*status, *cause), "case {i}: {reason}");
        assert!(!reason.is_empty());
    }
}

#[test]
fn percentages_compare_within_the_verifier_tolerance() {
    let f = |d: &str| Facts {
        percentage: true,
        ..facts_with(Current::Value(dec("30")), Some(d), Some(true))
    };
    let near = dec("30") + s106::verify::PCT_TOLERANCE;
    assert_eq!(decide(&f(&near.to_string())).0, RowStatus::CountsMatch);
    let far = near + dec("0.01");
    assert_eq!(
        decide(&f(&far.to_string())).0,
        RowStatus::DiffersReviewRequired
    );
}

/// An obligation a later deed introduces is a change from nothing, checked like any later event.
#[test]
fn an_obligation_a_later_deed_introduces_goes_to_review() {
    let library = Measure::contribution("Library Contribution");
    let introduce = |edit: fn(&mut Agreement)| {
        let mut d = dov();
        let mut c = highways_contribution("50000");
        c.name = "Library Contribution".into();
        d.contributions.push(c);
        edit(&mut d);
        run(&[original(), d], full())
    };
    let r = introduce(|_| {});
    let l = row(&r, &library);
    assert_eq!(codes(l), ["replaced"]);
    assert!(matches!(
        l.review[0],
        ReviewReason::Replaced { from: None, .. }
    ));
    assert_eq!(l.status, RowStatus::Varied);

    let r = introduce(|d| d.metadata.planning_references.clear());
    let l = row(&r, &library);
    assert_eq!(codes(l), ["unverified", "replaced"]);
    assert_eq!(l.status, RowStatus::Varied);
    assert_eq!(
        l.current,
        Some(dec("50000")),
        "falls back to the only figure"
    );

    let r = introduce(|d| {
        d.instrument.scope = Some(Quoted {
            text: "in so far as it relates to Phase 2".into(),
            evidence: None,
        })
    });
    let l = row(&r, &library);
    assert_eq!(codes(l), ["scoped", "replaced"]);
    assert_eq!(l.status, RowStatus::Varied);
}

#[test]
fn the_current_figure_skips_scoped_and_unverified_deeds() {
    let mut d = dov();
    d.contributions.push(highways_contribution("60000"));
    d.metadata.planning_references.clear();
    let r = run(&[original(), d], full());
    assert_eq!(row(&r, &highways()).current, Some(dec("45000")));

    let mut d = dov();
    d.contributions.push(highways_contribution("60000"));
    d.instrument.scope = Some(Quoted {
        text: "in so far as it relates to Phase 2".into(),
        evidence: None,
    });
    let r = run(&[original(), d], full());
    assert_eq!(row(&r, &highways()).current, Some(dec("45000")));

    let mut d = dov();
    d.contributions.push(highways_contribution("60000"));
    let r = run(&[original(), d], full());
    assert_eq!(row(&r, &highways()).current, Some(dec("60000")));
}
