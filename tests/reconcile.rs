//! Reconciliation: obligation events from the recorded 19/AP/1773 extraction, and status rules
//! against delivery records.
//!
//! The delivery records here are synthetic, built to exercise each rule. They are not PLD data;
//! the real reconciliation reads saved PLD snapshots.

use std::path::Path;

use chrono::NaiveDate;
use rust_decimal::Decimal;
use s106::evidence::Pages;
use s106::pld::{self, Request, Snapshot};
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
        variations: vec![],
        warnings: vec![],
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
        snapshot: 0,
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
        record_id: "Southwark-19_AP_1773".into(),
        snapshots: vec![],
        status: Some(if complete { "Completed" } else { "Started" }.into()),
        started_on: None,
        completed_on: None,
        scheme_complete: Some(complete),
        permitted: Permitted::default(),
        units,
    }
}

const SR: Tenure = Tenure::SocialRent;
const INT: Tenure = Tenure::Intermediate;

#[test]
fn events_carry_the_deed_values_and_citations() {
    let events = obligation_events(&southwark(), None);
    let get = |m: Measure| {
        events
            .iter()
            .find(|e| e.measure == m)
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

    let de = get(Measure::Contribution {
        name: "Default Employment in the End Use Contribution".into(),
    });
    assert_eq!(de.value, Some(dec("73100")));
    assert!(de.note.as_deref().unwrap().starts_with("conditional"));
    // A formula contribution has no stated sum.
    let ciet = get(Measure::Contribution {
        name: "Construction Industry Employment and Training Contribution".into(),
    });
    assert_eq!(ciet.value, None);
    assert!(events.iter().all(|e| e.kind == EventKind::Original));
    assert!(events.iter().all(|e| e.grounding == Grounding::NotChecked));
}

#[test]
fn grounding_uses_the_verifier_rules() {
    let events = obligation_events(&southwark(), Some(&southwark_pages()));
    let g = |m: Measure| {
        events
            .iter()
            .find(|e| e.measure == m)
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
    let events = obligation_events(&southwark(), Some(&southwark_pages()));
    let r = reconcile(
        "Southwark-19_AP_1773",
        events,
        Some(delivery(
            177,
            &[(SR, 58), (Tenure::LondonSharedOwnership, 18)],
            true,
        )),
    );
    let total = row(&r, &Measure::TotalUnits);
    assert_eq!(total.delivered.value, Some(dec("253")));
    assert_eq!(total.status, RowStatus::Indeterminate);
    assert!(total.reason.starts_with("deed value unproven"));
    assert_eq!(
        row(&r, &Measure::AffordableUnits).status,
        RowStatus::Consistent
    );
}

#[test]
fn matching_delivery_is_consistent_and_intermediate_covers_london_shared_ownership() {
    let r = reconcile(
        "Southwark-19_AP_1773",
        obligation_events(&southwark(), None),
        Some(delivery(
            177,
            &[(SR, 58), (Tenure::LondonSharedOwnership, 18)],
            true,
        )),
    );
    for m in [
        Measure::TotalUnits,
        Measure::MarketUnits,
        Measure::AffordableUnits,
        Measure::TenureUnits { tenure: SR },
        Measure::TenureUnits { tenure: INT },
    ] {
        assert_eq!(row(&r, &m).status, RowStatus::Consistent, "{m:?}");
    }
    assert!(!r.rows.iter().any(|x| x.label.ends_with("(not in deed)")));
    // Synthetic rooms (3 per affordable unit) don't match the deed's 271: a difference to review.
    let rooms = row(&r, &Measure::AffordableHabitableRooms);
    assert_eq!(rooms.delivered.value, Some(dec("228")));
    assert_eq!(rooms.status, RowStatus::DiffersReviewRequired);
    // Contributions have no PLD delivery source.
    let bus = row(
        &r,
        &Measure::Contribution {
            name: "Bus Contribution".into(),
        },
    );
    assert_eq!(bus.status, RowStatus::Indeterminate);
    assert_eq!(bus.delivered.basis, "PLD records no S106 payments");
}

#[test]
fn shortfall_on_a_complete_scheme_goes_to_review_never_unmet() {
    let r = reconcile(
        "dev",
        obligation_events(&southwark(), None),
        Some(delivery(187, &[(SR, 48), (INT, 18)], true)),
    );
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
    let r = reconcile("dev", obligation_events(&southwark(), None), Some(d));
    let market = row(&r, &Measure::MarketUnits);
    assert_eq!(market.delivered.value, Some(dec("100")));
    assert_eq!(market.status, RowStatus::NotYetDue);
}

#[test]
fn unknown_scheme_status_is_indeterminate() {
    let mut d = delivery(170, &[(SR, 58), (INT, 18)], true);
    d.scheme_complete = None;
    let r = reconcile("dev", obligation_events(&southwark(), None), Some(d));
    assert_eq!(
        row(&r, &Measure::MarketUnits).status,
        RowStatus::Indeterminate
    );
}

#[test]
fn social_rent_does_not_cover_london_affordable_rent() {
    let r = reconcile(
        "dev",
        obligation_events(&southwark(), None),
        Some(delivery(
            177,
            &[(Tenure::LondonAffordableRent, 58), (INT, 18)],
            true,
        )),
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
fn unrecognised_tenure_blocks_tenure_counts_but_not_the_total() {
    let mut d = delivery(177, &[(SR, 57), (INT, 18)], true);
    d.units.push(unit(
        999,
        UnitTenure::Unrecognised("Key worker".into()),
        2,
        true,
    ));
    let r = reconcile("dev", obligation_events(&southwark(), None), Some(d));
    assert_eq!(row(&r, &Measure::TotalUnits).status, RowStatus::Consistent);
    let sr = row(&r, &Measure::TenureUnits { tenure: SR });
    assert_eq!(sr.status, RowStatus::Indeterminate);
    assert!(sr.delivered.basis.contains("unrecognised tenure"));
}

#[test]
fn a_variation_becomes_the_current_obligation() {
    let original = southwark();
    let mut dov = southwark();
    dov.provenance.sha256 = "dov".into();
    dov.instrument.kind = InstrumentKind::DeedOfVariation;
    dov.housing = Housing {
        tenures: vec![TenureAllocation {
            tenure: SR,
            label: None,
            units: Some(50),
            habitable_rooms: None,
            evidence: Some(Evidence {
                page: 3,
                snippet: "50 Social Rented Housing Units".into(),
            }),
        }],
        ..Default::default()
    };
    dov.contributions.clear();
    let mut events = obligation_events(&original, None);
    events.extend(obligation_events(&dov, None));
    let r = reconcile(
        "dev",
        events,
        Some(delivery(177, &[(SR, 50), (INT, 18)], true)),
    );
    assert_eq!(r.instruments.len(), 2);
    let sr = row(&r, &Measure::TenureUnits { tenure: SR });
    assert_eq!((sr.agreed, sr.current), (Some(dec("58")), Some(dec("50"))));
    assert_eq!(sr.events.len(), 2);
    assert_eq!(sr.status, RowStatus::Consistent);

    let r = reconcile(
        "dev",
        {
            let mut e = obligation_events(&original, None);
            e.extend(obligation_events(&dov, None));
            e
        },
        Some(delivery(177, &[(SR, 45), (INT, 18)], true)),
    );
    assert_eq!(
        row(&r, &Measure::TenureUnits { tenure: SR }).status,
        RowStatus::Varied
    );
}

#[test]
fn reconciliation_round_trips_as_json() {
    let r = reconcile(
        "dev",
        obligation_events(&southwark(), None),
        Some(delivery(177, &[(SR, 58), (INT, 18)], true)),
    );
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
            url: format!(
                "{}/applications/_source/Southwark-19_AP_1773",
                pld::BASE_URL
            ),
            body: None,
        },
        retrieved_at: "2026-10-13T20:00:00Z".into(),
        response: serde_json::json!({"id": "Southwark-19_AP_1773"}),
    };
    let r = pld::save(dir.path(), "applications", "Southwark-19_AP_1773", &snap).unwrap();
    assert_eq!(
        r.path,
        "snapshots/pld/applications/Southwark-19_AP_1773/2026-10-13T20_00_00Z.json"
    );
    let (back, r2) = pld::load(dir.path(), Path::new(&r.path)).unwrap();
    assert_eq!(r, r2);
    assert_eq!(back.response, snap.response);
    assert!(pld::save(dir.path(), "applications", "Southwark-19_AP_1773", &snap).is_err());
}
