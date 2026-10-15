//! The Datahub mapping (`s106::datahub`): a snapshot's homes and completion as V1 and V8 rows.

use serde_json::json;

#[test]
fn a_record_maps_to_tenure_counts_and_a_completion() {
    let unit = |tenure: &str, change: &str, done: Option<&str>| json!({"tenure": tenure, "change_type": change, "actual_completion_date": done});
    let response = json!({
        "status": "Completed",
        "actual_completion_date": "01/01/2017",
        "application_details": {"residential_details": {
            "total_no_proposed_residential_units": 5,
            "total_no_proposed_residential_affordable_units": 3,
            "residential_units": [
                unit("Market for sale", "Gain", Some("01/01/2017")),
                unit("Market for sale", "Gain", Some("01/01/2017")),
                unit("Social Rent", "Gain", Some("01/01/2017")),
                unit("Intermediate Other", "Gain", None),
                unit("Affordable Rent (not at LAR benchmark rents)", "Gain", Some("01/01/2017")),
                // a demolished home is not a tenure count
                unit("Social Rent", "Loss", None),
            ],
        }},
    });
    let rows = s106::datahub::rows("EX/1", "snap.json", &response, "2026-10-15");
    assert_eq!(rows[0].value, "total=5; market=2; AR=1; INT=1; SR=1");
    assert_eq!(rows[0].proposal_id, "EX/1/V1/datahub");
    assert_eq!(rows[1].value, "completed=2017-01-01; completed_homes=4");

    let rows = s106::datahub::rows(
        "EX/2",
        "snap.json",
        &json!({"status": "Lapsed"}),
        "2026-10-15",
    );
    assert_eq!(
        (rows[0].status.as_str(), rows[1].status.as_str()),
        ("not_stated", "not_stated")
    );
}
