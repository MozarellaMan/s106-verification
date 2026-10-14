//! Results (`s106::results`): the ledger's choice of reading per cell, the batch-1 decision rules
//! and the claims register, on invented schemes.

use std::path::Path;

use s106::results::{self, Check, Outcome};

const HEADER: &str = "scheme,variable_id,variable,source,doc_ids,value,basis,pages,status,minutes,notes,labeller,labelled_on,method\n";

fn write(root: &Path, rel: &str, body: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, body).unwrap();
}

/// A sheet listing every cell the rules read, for the given schemes.
fn sheet(root: &Path, schemes: &[&str]) {
    let mut s = String::from(
        "scheme,variable_id,variable,source,doc_ids,value,basis,pages,status,minutes,notes,labeller,labelled_on\n",
    );
    for scheme in schemes {
        for (v, src) in [
            ("V1", "committee_report"),
            ("V1", "deed"),
            ("V2", "committee_report"),
            ("V2", "deed"),
            ("V4", "committee_report"),
            ("V4", "gla"),
            ("V4", "deed"),
            ("V8", "datahub"),
            ("V8", "epc"),
        ] {
            s += &format!("{scheme},{v},x,{src},,,,,,,,,\n");
        }
    }
    write(root, "runs/b/labels.csv", &s);
}

fn proposal(scheme: &str, v: &str, src: &str, value: &str, basis: &str, status: &str) -> String {
    format!(
        "{scheme},{v},x,{src},,\"{value}\",{basis},p1,{status},,,machine:proposal/test,2026-10-14,\n"
    )
}

fn label(
    scheme: &str,
    v: &str,
    src: &str,
    value: &str,
    status: &str,
    who: &str,
    method: &str,
) -> String {
    format!("{scheme},{v},x,{src},,\"{value}\",units,p2,{status},3,,{who},2026-10-20,{method}\n")
}

fn run(root: &Path) -> results::Results {
    results::run(root, Path::new("runs/b/labels.csv"), Path::new("results/b")).unwrap()
}

fn entry<'a>(r: &'a results::Results, scheme: &str, v: &str, src: &str) -> &'a results::Entry {
    r.ledger
        .iter()
        .find(|e| e.scheme == scheme && e.variable_id == v && e.source == src)
        .unwrap()
}

#[test]
fn nothing_read_is_a_report_not_an_error() {
    let root = tempfile::tempdir().unwrap();
    sheet(root.path(), &["EX/1"]);
    let r = run(root.path());
    assert!(r.ledger.iter().all(|e| e.check == Check::Unread));
    assert!(r.rules.iter().all(|x| !x.met));
    for f in ["README.md", "ledger.csv", "rules.csv", "retrieval.csv"] {
        assert!(root.path().join("results/b").join(f).exists(), "{f}");
    }
}

#[test]
fn labels_outrank_proposals_and_later_proposal_files_win() {
    let root = tempfile::tempdir().unwrap();
    let p = root.path();
    sheet(p, &["EX/1"]);
    write(
        p,
        "proposals/2026-10-14-a.csv",
        &format!(
            "{HEADER}{}{}",
            proposal("EX/1", "V1", "deed", "total=10", "units", "found"),
            proposal("EX/1", "V1", "gla", "total=9", "units", "found"),
        ),
    );
    write(
        p,
        "proposals/2026-10-14-b.csv",
        &format!(
            "{HEADER}{}",
            proposal("EX/1", "V1", "gla", "total=11", "units", "found")
        ),
    );
    write(
        p,
        "labels/2026-10-20-ab.csv",
        &format!(
            "{HEADER}{}{}{}",
            label("EX/1", "V1", "deed", "total=12", "found", "AB", "corrected"),
            label("EX/1", "V2", "deed", "share=35%", "found", "AB", "blind"),
            label("EX/1", "V4", "deed", "reviews=early", "found", "AB", "read"),
        ),
    );
    write(
        p,
        "labels/2026-10-20-cd.csv",
        &format!(
            "{HEADER}{}{}",
            label("EX/1", "V2", "deed", "share=35%", "found", "CD", "audited"),
            label("EX/1", "V4", "deed", "reviews=none", "found", "CD", "read"),
        ),
    );
    let r = run(p);
    let deed = entry(&r, "EX/1", "V1", "deed");
    assert_eq!(
        (deed.check, deed.value.as_str()),
        (Check::Anchored, "total=12")
    );
    assert_eq!(deed.pages, "p2");
    let gla = entry(&r, "EX/1", "V1", "gla");
    assert_eq!(
        (gla.check, gla.value.as_str()),
        (Check::Proposed, "total=11")
    );
    assert_eq!(gla.stage, "decision");
    // one blind and one audited label that agree: the independent one stands
    assert_eq!(entry(&r, "EX/1", "V2", "deed").check, Check::Independent);
    assert_eq!(entry(&r, "EX/1", "V4", "deed").check, Check::Unsettled);
    assert_eq!(entry(&r, "EX/1", "V8", "epc").check, Check::Unread);
}

#[test]
fn rule1_compares_report_and_deed_tenures() {
    let root = tempfile::tempdir().unwrap();
    let p = root.path();
    sheet(p, &["EX/1", "EX/2", "EX/3", "EX/4", "EX/5"]);
    let rows = [
        proposal(
            "EX/1",
            "V1",
            "committee_report",
            "total=50; SR=5; SO=5",
            "units",
            "found",
        ),
        proposal(
            "EX/1",
            "V1",
            "deed",
            "total=50; market=40; SR=5; SO=5",
            "units",
            "found",
        ),
        proposal(
            "EX/2",
            "V1",
            "committee_report",
            "total=50; SR=6; SO=4",
            "units",
            "found",
        ),
        proposal(
            "EX/2",
            "V1",
            "deed",
            "total=50; SR=5; SO=5",
            "units",
            "found",
        ),
        proposal("EX/3", "V1", "committee_report", "", "", "source_missing"),
        proposal(
            "EX/4",
            "V1",
            "committee_report",
            "total=50; SR=5; SO=5",
            "units",
            "found",
        ),
        proposal(
            "EX/4",
            "V1",
            "deed",
            "total=50; SR=5; SO=5",
            "units",
            "found",
        ),
        proposal(
            "EX/4",
            "V2",
            "committee_report",
            "share=20%; split=50:50 SR:SO",
            "units",
            "found",
        ),
        proposal(
            "EX/4",
            "V2",
            "deed",
            "share=20%; split=60:40 SR:SO",
            "units",
            "found",
        ),
        proposal(
            "EX/5",
            "V1",
            "committee_report",
            "total=50; SR=5; SO=5",
            "units",
            "found",
        ),
        proposal(
            "EX/5",
            "V1",
            "deed",
            "total=50; SR=5; SO=5",
            "units",
            "found",
        ),
        proposal(
            "EX/5",
            "V2",
            "committee_report",
            "share=30%",
            "habitable_rooms",
            "found",
        ),
        proposal("EX/5", "V2", "deed", "", "not_stated", "not_stated"),
    ];
    write(p, "proposals/x.csv", &format!("{HEADER}{}", rows.concat()));
    let r = run(p);
    let rule = &r.rules[0];
    let outcome = |s: &str| rule.schemes.iter().find(|o| o.scheme == s).unwrap().outcome;
    assert_eq!(outcome("EX/1"), Outcome::Counts);
    assert_eq!(outcome("EX/2"), Outcome::DoesNotCount);
    assert_eq!(outcome("EX/3"), Outcome::LeftOut);
    assert_eq!(outcome("EX/4"), Outcome::DoesNotCount);
    // a basis only the report states can't be shown equal
    assert_eq!(outcome("EX/5"), Outcome::Indeterminate);
    // 1 of 4 counted, all but one needed
    assert_eq!(
        (rule.counts, rule.counted, rule.needed, rule.met),
        (1, 4, 3, false)
    );
    // EX/1's V2 cells are consulted but nobody has read them
    assert_eq!(rule.check, Check::Unread);
}

#[test]
fn rule2_allows_five_percent_and_leaves_out_uncompleted_schemes() {
    let root = tempfile::tempdir().unwrap();
    let p = root.path();
    sheet(p, &["EX/1", "EX/2", "EX/3", "EX/4", "EX/5", "EX/6"]);
    let rows = [
        proposal(
            "EX/1",
            "V8",
            "datahub",
            "completed=2020-01-01; completed_homes=100",
            "units",
            "found",
        ),
        proposal(
            "EX/1",
            "V8",
            "epc",
            "new_dwelling_epcs=105",
            "units",
            "found",
        ),
        proposal(
            "EX/2",
            "V8",
            "datahub",
            "completed=2020-01-01; completed_homes=100",
            "units",
            "found",
        ),
        proposal(
            "EX/2",
            "V8",
            "epc",
            "new_dwelling_epcs=94",
            "units",
            "found",
        ),
        proposal(
            "EX/3",
            "V8",
            "datahub",
            "completed=2020-01-01; completed_homes=100",
            "units",
            "found",
        ),
        proposal(
            "EX/3",
            "V8",
            "epc",
            "new_dwelling_epcs=90 OR new_dwelling_epcs=100",
            "units",
            "ambiguous",
        ),
        proposal(
            "EX/4",
            "V8",
            "datahub",
            "completed=2020-01-01; completed_homes=40",
            "units",
            "found",
        ),
        proposal(
            "EX/4",
            "V8",
            "epc",
            "new_dwelling_epcs=40",
            "units",
            "found",
        ),
        proposal("EX/5", "V8", "datahub", "", "", "not_stated"),
        proposal(
            "EX/6",
            "V8",
            "datahub",
            "completed=2020-01-01; completed_homes=0",
            "units",
            "found",
        ),
        proposal("EX/6", "V8", "epc", "new_dwelling_epcs=0", "units", "found"),
    ];
    write(p, "proposals/x.csv", &format!("{HEADER}{}", rows.concat()));
    let r = run(p);
    let rule = &r.rules[1];
    let outcome = |s: &str| rule.schemes.iter().find(|o| o.scheme == s).unwrap().outcome;
    assert_eq!(outcome("EX/1"), Outcome::Counts);
    assert_eq!(outcome("EX/2"), Outcome::DoesNotCount);
    assert_eq!(outcome("EX/3"), Outcome::Indeterminate);
    assert_eq!(outcome("EX/5"), Outcome::LeftOut);
    // no completed homes is not completed, not a 0/0 mismatch
    assert_eq!(outcome("EX/6"), Outcome::LeftOut);
    // 2 of 4 completed agree, 3 needed
    assert_eq!(
        (rule.counts, rule.counted, rule.needed, rule.met),
        (2, 4, 3, false)
    );
    assert!(rule.verdict().starts_with("Provisional, not a finding"));
}

#[test]
fn rule3_needs_a_deed_review_the_reports_in_hand_do_not_describe() {
    let root = tempfile::tempdir().unwrap();
    let p = root.path();
    sheet(p, &["EX/1", "EX/2", "EX/3"]);
    let rows = [
        proposal(
            "EX/1",
            "V4",
            "deed",
            "route=viability_tested; reviews=early,late",
            "",
            "found",
        ),
        proposal(
            "EX/1",
            "V4",
            "committee_report",
            "route=viability_tested; reviews=none",
            "",
            "found",
        ),
        proposal("EX/1", "V4", "gla", "", "", "source_missing"),
        proposal(
            "EX/2",
            "V4",
            "deed",
            "route=viability_tested; reviews=early",
            "",
            "found",
        ),
        proposal(
            "EX/2",
            "V4",
            "gla",
            "route=viability_tested; reviews=early",
            "",
            "found",
        ),
        proposal(
            "EX/3",
            "V4",
            "deed",
            "route=viability_tested; reviews=none",
            "",
            "found",
        ),
    ];
    write(p, "proposals/x.csv", &format!("{HEADER}{}", rows.concat()));
    let r = run(p);
    let rule = &r.rules[2];
    let outcome = |s: &str| rule.schemes.iter().find(|o| o.scheme == s).unwrap().outcome;
    assert_eq!(outcome("EX/1"), Outcome::Counts);
    assert_eq!(outcome("EX/2"), Outcome::DoesNotCount);
    assert_eq!(outcome("EX/3"), Outcome::DoesNotCount);
    assert!(!rule.met);
}

#[test]
fn a_claim_stands_at_its_weakest_cell() {
    let root = tempfile::tempdir().unwrap();
    let p = root.path();
    sheet(p, &["EX/1"]);
    write(
        p,
        "proposals/x.csv",
        &format!(
            "{HEADER}{}",
            proposal(
                "EX/1",
                "V8",
                "epc",
                "new_dwelling_epcs=40",
                "units",
                "found"
            )
        ),
    );
    write(
        p,
        "labels/2026-10-20-ab.csv",
        &format!(
            "{HEADER}{}",
            label(
                "EX/1",
                "V8",
                "datahub",
                "completed_homes=40",
                "found",
                "AB",
                "read"
            )
        ),
    );
    write(
        p,
        "results/b/claims.csv",
        "id,claim,cells,specific,signed_off\n\
         c1,The Datahub records 40 completed homes.,EX/1/V8/datahub,no,\n\
         c2,EPC certificates match the Datahub.,EX/1/V8/datahub EX/1/V8/epc,yes,\n\
         c3,A typo.,EX/1/V9/deed,no,\n",
    );
    let r = run(p);
    let checks: Vec<Check> = r.claims.iter().map(|c| c.check).collect();
    assert_eq!(checks, [Check::Independent, Check::Proposed, Check::Unread]);
    assert_eq!(r.claims[2].unknown, ["EX/1/V9/deed"]);
    let md = std::fs::read_to_string(p.join("results/b/README.md")).unwrap();
    assert!(md.contains("*Specific: not signed off.*"), "{md}");
}

#[test]
fn split_counts_are_not_compared_and_datahub_context_is_shown() {
    let root = tempfile::tempdir().unwrap();
    let p = root.path();
    sheet(p, &["EX/1"]);
    let v1 = "total=10; market=5; SR=3; SO=2";
    let rows = [
        proposal("EX/1", "V1", "committee_report", v1, "units", "found"),
        proposal("EX/1", "V1", "deed", v1, "units", "found"),
        proposal(
            "EX/1",
            "V2",
            "committee_report",
            "share=35%; split=72:28 SR:SO",
            "habitable_rooms",
            "found",
        ),
        // habitable-room counts written as a split
        proposal(
            "EX/1",
            "V2",
            "deed",
            "share=35%; split=64:25 SR:SO",
            "habitable_rooms",
            "found",
        ),
        "EX/1,V8,x,datahub,snapshots/EX-1.json,\"completed=2022-12-15; completed_homes=84\",units,,found,,,machine:proposal/test,2026-10-14,\n".into(),
        proposal(
            "EX/1",
            "V8",
            "epc",
            "first_lodged=2023-11-02; last_lodged=2024-11-06; new_dwelling_epcs=84",
            "units",
            "found",
        ),
    ];
    write(p, "proposals/x.csv", &format!("{HEADER}{}", rows.concat()));
    write(
        p,
        "snapshots/EX-1.json",
        r#"{"response": {"status": "Completed", "bo_system": "FME Legacy Data Import"}}"#,
    );
    let r = run(p);

    let one = &r.rules[0].schemes[0];
    assert_eq!(one.outcome, Outcome::Indeterminate);
    assert!(
        one.reason.contains("deed V2 split adds up to 89"),
        "{}",
        one.reason
    );

    let two = &r.rules[1].schemes[0];
    assert_eq!(two.outcome, Outcome::Counts);
    assert!(
        two.reason
            .contains("EPCs lodged 322 to 692 days from the Datahub completion date"),
        "{}",
        two.reason
    );

    let e = entry(&r, "EX/1", "V1", "deed");
    assert_eq!(
        (e.datahub_status.as_str(), e.datahub_system.as_str()),
        ("Completed", "FME Legacy Data Import")
    );
    let readme = std::fs::read_to_string(p.join("results/b/README.md")).unwrap();
    assert!(readme.contains("Datahub record: Completed, from FME Legacy Data Import."));
}

#[test]
fn an_unreadable_datahub_snapshot_is_reported() {
    let root = tempfile::tempdir().unwrap();
    let p = root.path();
    sheet(p, &["EX/1"]);
    let row = "EX/1,V8,x,datahub,snapshots/missing.json,\"completed=2022-12-15; completed_homes=84\",units,,found,,,machine:proposal/test,2026-10-14,\n";
    write(p, "proposals/x.csv", &format!("{HEADER}{row}"));
    let r = run(p);
    assert_eq!(entry(&r, "EX/1", "V8", "datahub").datahub_status, "");
    let reasons: Vec<&str> = r.rejected.iter().map(|x| x.reason.as_str()).collect();
    assert_eq!(reasons.len(), 1, "{reasons:?}");
    assert!(
        reasons[0].starts_with("Datahub snapshot snapshots/missing.json for EX/1 not read"),
        "{reasons:?}"
    );
}
