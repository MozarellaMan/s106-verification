//! Results (`s106::results`): the ledger's choice of reading per cell, the accuracy estimate and
//! check levels, the batch-1 decision rules and the claims register, on invented schemes.

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
    results::run(
        root,
        Path::new("runs/b/labels.csv"),
        Path::new("results/b"),
        true,
        &[],
    )
    .unwrap()
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
fn later_proposal_files_win_and_readers_who_differ_are_listed() {
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
    // a person's label is one more reader, not the truth: the proposal stays shown, so the rules
    // still read it, and both readings are listed beside it
    let deed = entry(&r, "EX/1", "V1", "deed");
    assert_eq!(
        (deed.check, deed.value.as_str()),
        (Check::Proposed, "total=10")
    );
    assert!(deed.readers.starts_with("2 differ: "), "{}", deed.readers);
    assert!(
        deed.readers.contains("AB found `total=12`"),
        "{}",
        deed.readers
    );
    assert!(
        deed.readers
            .contains("machine:proposal/test found `total=10`")
    );
    // one reader's later file supersedes its earlier one
    let gla = entry(&r, "EX/1", "V1", "gla");
    assert_eq!(
        (gla.check, gla.value.as_str(), gla.readers.as_str()),
        (Check::Proposed, "total=11", "1 reader")
    );
    assert_eq!(gla.stage, "decision");
    // two readers agree, but nobody's accuracy is estimated yet
    let v2 = entry(&r, "EX/1", "V2", "deed");
    assert_eq!(
        (v2.check, v2.readers.as_str()),
        (Check::Proposed, "2 agree")
    );
    assert_eq!(v2.error, "");
    assert!(
        entry(&r, "EX/1", "V4", "deed")
            .readers
            .starts_with("2 differ")
    );
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
        "id,claim,cells\n\
         c1,The Datahub records 40 completed homes.,EX/1/V8/datahub\n\
         c2,EPC certificates match the Datahub.,EX/1/V8/datahub EX/1/V8/epc\n\
         c3,A typo.,EX/1/V9/deed\n",
    );
    let r = run(p);
    let checks: Vec<Check> = r.claims.iter().map(|c| c.check).collect();
    assert_eq!(checks, [Check::Proposed, Check::Proposed, Check::Unread]);
    assert_eq!(r.claims[2].unknown, ["EX/1/V9/deed"]);
    let md = std::fs::read_to_string(p.join("results/b/README.md")).unwrap();
    assert!(md.contains("(cells: EX/1/V8/datahub, EX/1/V8/epc)"), "{md}");
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

#[test]
fn variation_sources_sit_in_the_variation_stage() {
    assert_eq!(results::stage("V1", "variation"), "variation");
    assert_eq!(results::stage("V7", "deed"), "variation");
    assert_eq!(results::stage("V1", "deed"), "obligation");
    assert_eq!(results::stage("V1", "committee_report"), "decision");
    assert_eq!(results::stage("V8", "variation"), "completion");
}

fn reading(scheme: &str, v: &str, src: &str, value: &str, who: &str) -> String {
    format!("{scheme},{v},x,{src},,\"{value}\",units,p1,found,,,{who},2026-10-14,\n")
}

const LLM: &str = "machine:proposal/claude-x";
const DATAHUB: &str = "machine:proposal/datahub-mapping";

/// 40 schemes (S/0 to S/39) whose deed V1 an LLM reader, a person and the Datahub read; the LLM
/// reader is wrong on the first `wrong` of them. EX/1's cells are added by the caller.
fn readers(p: &Path, wrong: usize, extra: &str) {
    let mut props = HEADER.to_string();
    let mut labels = HEADER.to_string();
    let mut seen = String::from("scheme,variable_id,variable,source\n");
    for i in 0..40 {
        let s = format!("S/{i}");
        let llm = if i < wrong {
            "total=10; SR=4"
        } else {
            "total=10; SR=3"
        };
        props += &reading(&s, "V1", "deed", llm, LLM);
        props += &reading(&s, "V1", "datahub", "total=10; market=7", DATAHUB);
        labels += &label(&s, "V1", "deed", "total=10; SR=3", "found", "AB", "read");
        seen += &format!("{s},V1,x,deed\n");
    }
    write(p, "proposals/2026-10-14-p.csv", &(props + extra));
    write(p, "labels/2026-10-20-ab.csv", &labels);
    write(p, "runs/seen/labels.csv", &seen);
}

fn run_seen(root: &Path, seen: &[&str]) -> results::Results {
    let seen: Vec<std::path::PathBuf> = seen.iter().map(|s| s.into()).collect();
    results::run(
        root,
        Path::new("runs/b/labels.csv"),
        Path::new("results/b"),
        false,
        &seen,
    )
    .unwrap()
}

#[test]
fn three_families_that_agree_give_estimated_and_confirmed_cells() {
    let root = tempfile::tempdir().unwrap();
    let p = root.path();
    sheet(p, &["EX/1"]);
    let extra = [
        // the Datahub agrees with the deed's total and affordable count
        reading("EX/1", "V1", "deed", "total=40; SR=10", LLM),
        reading("EX/1", "V1", "datahub", "total=40; market=30", DATAHUB),
        // two LLM readers from two families agree, and no other reader read it
        reading("EX/1", "V2", "deed", "share=25%", "machine:proposal/gpt-y"),
        reading("EX/1", "V2", "deed", "share=25%", LLM),
        // one reading only
        reading("EX/1", "V4", "deed", "route=fixed", LLM),
    ]
    .concat();
    readers(p, 0, &extra);
    let r = run_seen(p, &[]);
    let g1: Vec<_> = r.estimates.iter().filter(|e| e.group == "G1").collect();
    assert_eq!(g1.len(), 3, "{g1:?}");
    assert!(g1.iter().all(|e| e.passes() && e.shared == 40), "{g1:?}");

    let v1 = entry(&r, "EX/1", "V1", "deed");
    assert_eq!(
        (v1.check, v1.readers.as_str()),
        (Check::Confirmed, "2 agree")
    );
    // unanimous readers are bounded by their 40 shared cells, so the error is small, not zero
    assert_eq!(v1.error, "0.9%");
    // estimated, not confirmed: no reader that isn't an LLM agrees
    assert_eq!(entry(&r, "EX/1", "V2", "deed").check, Check::Estimated);
    assert_eq!(entry(&r, "EX/1", "V4", "deed").check, Check::Proposed);
    let md = std::fs::read_to_string(p.join("results/b/README.md")).unwrap();
    assert!(md.contains("| G1 | machine:proposal/claude-x |"), "{md}");

    // the same readings on schemes seen while the method was built give no estimate
    let r = run_seen(p, &["runs/seen/labels.csv"]);
    assert!(r.estimates.is_empty());
    assert_eq!(entry(&r, "EX/1", "V1", "deed").check, Check::Proposed);
    let md = std::fs::read_to_string(p.join("results/b/README.md")).unwrap();
    assert!(md.contains("No estimate yet"), "{md}");
}

#[test]
fn a_reader_wrong_on_a_tenth_of_cells_fails_the_gate() {
    let root = tempfile::tempdir().unwrap();
    let p = root.path();
    sheet(p, &["EX/1"]);
    readers(p, 4, &reading("EX/1", "V1", "deed", "total=40; SR=10", LLM));
    let r = run_seen(p, &[]);
    let est = |who: &str| r.estimates.iter().find(|e| e.reader == who).unwrap();
    // agreement 36 of 40 with each of two readers that always agree: sqrt(0.9 x 0.9 / 1)
    assert!((est(LLM).accuracy - 0.9).abs() < 1e-9, "{:?}", est(LLM));
    assert!(est(LLM).lower.is_some_and(|l| l < 0.9) && !est(LLM).passes());
    assert!((est("AB").accuracy - 1.0).abs() < 1e-9);
    // the cell's only reader is the LLM, whose estimate doesn't pass
    assert_eq!(entry(&r, "EX/1", "V1", "deed").check, Check::Proposed);
}

#[test]
fn the_datahub_is_no_reader_of_a_varied_deed() {
    let root = tempfile::tempdir().unwrap();
    let p = root.path();
    sheet(p, &["EX/1"]);
    let extra = [
        reading("EX/1", "V1", "deed", "total=40; SR=10", LLM),
        reading("EX/1", "V1", "datahub", "total=40; market=20", DATAHUB),
    ]
    .concat();
    readers(p, 0, &extra);
    let r = run_seen(p, &[]);
    let v1 = entry(&r, "EX/1", "V1", "deed");
    // the difference is recorded beside the deed's reading, which stays shown
    assert_eq!(
        (v1.check, v1.value.as_str()),
        (Check::Proposed, "total=40; SR=10")
    );
    assert!(
        v1.readers
            .contains("datahub-mapping affordable=20 total=40"),
        "{}",
        v1.readers
    );

    // once a variation document is read, the Datahub may hold the varied figures
    let varied = reading("EX/1", "V1", "variation", "total=40; SR=20", LLM);
    readers(p, 0, &(extra + &varied));
    let r = run_seen(p, &[]);
    assert_eq!(entry(&r, "EX/1", "V1", "deed").readers, "1 reader");
}

#[test]
fn a_batch_without_rules_says_so() {
    let root = tempfile::tempdir().unwrap();
    sheet(root.path(), &["EX/1"]);
    let r = results::run(
        root.path(),
        Path::new("runs/b/labels.csv"),
        Path::new("results/b"),
        false,
        &[],
    )
    .unwrap();
    assert!(r.rules.is_empty());
    let note = std::fs::read_to_string(root.path().join("results/b/README.md")).unwrap();
    assert!(note.contains("no pre-registered decision rules"));
    assert!(note.contains("Retrieval is not recorded for this batch yet"));
    assert!(note.contains("No estimate yet"));
    for (f, header) in [
        ("rules.csv", "rule,scheme,outcome,reason,check"),
        (
            "retrieval.csv",
            "scheme,source,documents,in_hand,not_in_hand",
        ),
    ] {
        let body = std::fs::read_to_string(root.path().join("results/b").join(f)).unwrap();
        assert_eq!(body.trim(), header, "{f}");
    }
}

#[test]
fn superseded_files_do_not_vote_and_non_reading_files_are_skipped() {
    let root = tempfile::tempdir().unwrap();
    let p = root.path();
    sheet(p, &["EX/1"]);
    write(
        p,
        "proposals/2026-10-14-a.csv",
        &format!(
            "{HEADER}{}",
            reading(
                "EX/1",
                "V1",
                "deed",
                "total=40; AR=10",
                "machine:proposal/old"
            )
        ),
    );
    write(
        p,
        "proposals/2026-10-14-b.csv",
        &format!(
            "{HEADER}{}{}{}{}",
            reading("EX/1", "V1", "deed", "total=40; SR=10", LLM),
            // a total with no tenure states no affordable homes, as the Datahub's 0 does
            reading("EX/1", "V1", "gla", "total=12", LLM),
            reading("EX/2", "V1", "deed", "total=12", LLM),
            reading("EX/2", "V1", "datahub", "total=12; market=12", DATAHUB),
        ),
    );
    write(
        p,
        "proposals/routing.csv",
        "proposal_id,scheme,variable_id,group,route\nEX/1/V1/deed,EX/1,V1,G1,agreed\n",
    );
    sheet(p, &["EX/1", "EX/2"]);
    let r = run_seen(p, &[]);
    assert!(
        entry(&r, "EX/1", "V1", "deed")
            .readers
            .starts_with("2 differ")
    );
    assert_eq!(entry(&r, "EX/2", "V1", "deed").readers, "2 agree");
    assert!(r.rejected.is_empty(), "{:?}", r.rejected);

    write(
        p,
        "proposals/superseded.csv",
        "file,labeller,reason\nproposals/2026-10-14-a.csv,machine:proposal/old,replaced by b\n",
    );
    let r = run_seen(p, &[]);
    let deed = entry(&r, "EX/1", "V1", "deed");
    assert_eq!(
        (deed.value.as_str(), deed.readers.as_str()),
        ("total=40; SR=10", "1 reader")
    );
    assert!(r.rejected.is_empty(), "{:?}", r.rejected);
}

#[test]
fn the_reading_most_readers_give_is_shown() {
    let root = tempfile::tempdir().unwrap();
    let p = root.path();
    sheet(p, &["EX/1"]);
    let file = |name: &str, value: &str, who: &str| {
        write(
            p,
            &format!("proposals/{name}.csv"),
            &format!("{HEADER}{}", reading("EX/1", "V1", "deed", value, who)),
        )
    };
    file(
        "2026-10-14-a",
        "total=10; SR=3",
        "machine:proposal/claude-x",
    );
    file("2026-10-14-b", "total=10; SR=3", "machine:proposal/gpt-x");
    // the latest file is outvoted two to one
    file(
        "2026-10-14-c",
        "total=12; SR=3",
        "machine:proposal/gemini-x",
    );
    let r = run(p);
    let deed = entry(&r, "EX/1", "V1", "deed");
    assert_eq!(
        (deed.check, deed.value.as_str()),
        (Check::Proposed, "total=10; SR=3")
    );
    assert!(deed.readers.starts_with("3 differ: "), "{}", deed.readers);
}

#[test]
fn one_family_counts_once_and_an_oracle_can_carry_the_majority() {
    let root = tempfile::tempdir().unwrap();
    let p = root.path();
    sheet(p, &["EX/1", "EX/2", "EX/3"]);
    let file = |name: &str, rows: &[(&str, &str, &str, &str)]| {
        let body: String = rows
            .iter()
            .map(|(s, src, value, who)| reading(s, "V1", src, value, who))
            .collect();
        write(
            p,
            &format!("proposals/{name}.csv"),
            &format!("{HEADER}{body}"),
        )
    };
    // EX/1: two runs of one family against one other family is a tie, so the latest file stays
    // EX/2: the Datahub backs the older reading, so two families outvote one
    // EX/3: two challengers with two families each tie, so the later file of the two wins
    file(
        "2026-10-14-a",
        &[
            (
                "EX/1",
                "deed",
                "total=10; SR=3",
                "machine:proposal/claude-r1",
            ),
            (
                "EX/2",
                "deed",
                "total=10; SR=3",
                "machine:proposal/claude-r1",
            ),
            (
                "EX/3",
                "deed",
                "total=10; SR=3",
                "machine:proposal/claude-r1",
            ),
            ("EX/3", "deed", "total=10; SR=3", "machine:proposal/gpt-r1"),
        ],
    );
    file(
        "2026-10-14-b",
        &[
            (
                "EX/1",
                "deed",
                "total=10; SR=3",
                "machine:proposal/claude-r2",
            ),
            ("EX/2", "datahub", "total=10; market=7", DATAHUB),
            (
                "EX/3",
                "deed",
                "total=11; SR=3",
                "machine:proposal/gemini-r1",
            ),
            (
                "EX/3",
                "deed",
                "total=11; SR=3",
                "machine:proposal/mistral-r1",
            ),
        ],
    );
    file(
        "2026-10-14-c",
        &[
            ("EX/1", "deed", "total=12; SR=3", "machine:proposal/gpt-r1"),
            ("EX/2", "deed", "total=12; SR=3", "machine:proposal/gpt-r1"),
            (
                "EX/3",
                "deed",
                "total=12; SR=3",
                "machine:proposal/llama-r1",
            ),
        ],
    );
    let r = run(p);
    let shown = |s: &str| entry(&r, s, "V1", "deed").value.clone();
    assert_eq!(shown("EX/1"), "total=12; SR=3");
    assert_eq!(shown("EX/2"), "total=10; SR=3");
    assert_eq!(shown("EX/3"), "total=11; SR=3");
}
