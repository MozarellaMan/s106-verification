//! Human labels (`s106::labels`): loading submissions, supersession, and the three figures, on
//! invented labels and the invented Datahub record in `tests/synthetic/pld/`.

use std::path::Path;

use s106::labels::{self, Reading};

const HEADER: &str = "scheme,variable_id,variable,source,doc_ids,value,basis,pages,status,minutes,notes,labeller,labelled_on\n";

fn submit(root: &Path, name: &str, rows: &str) {
    let dir = root.join("labels");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(name), format!("{HEADER}{rows}")).unwrap();
}

fn machine(scheme: &str, variable_id: &str, source: &str, value: &str, basis: &str) -> Reading {
    Reading {
        scheme: scheme.into(),
        variable_id: variable_id.into(),
        source: source.into(),
        value: value.into(),
        basis: basis.into(),
        status: if value.is_empty() {
            "not_stated"
        } else {
            "found"
        }
        .into(),
        labeller: "machine:test".into(),
        labelled_on: String::new(),
        minutes: None,
        doc_ids: String::new(),
        pages: String::new(),
        method: String::new(),
        origin: "test".into(),
    }
}

#[test]
fn no_labels_is_a_report_not_an_error() {
    let root = tempfile::tempdir().unwrap();
    let r = labels::run(root.path(), "claude-haiku-5-5").unwrap();
    assert_eq!(r.labels.cells, 0);
    assert!(r.accuracy.is_empty());
    let md = std::fs::read_to_string(root.path().join("reports/labels/labels.md")).unwrap();
    assert!(md.contains("No human labels yet"), "{md}");
    assert!(root.path().join("reports/labels/labels.json").exists());
}

#[test]
fn blank_rows_are_skipped_and_bad_rows_reported() {
    let root = tempfile::tempdir().unwrap();
    submit(
        root.path(),
        "sheet.csv",
        "EX/1,V1,x,deed,,,,,,,,,\n\
         EX/1,V1,x,gla,,total=10,units,3,found,4,,AB,2026-10-20\n\
         EX/1,V2,x,deed,,,,,maybe,,,AB,2026-10-20\n\
         EX/1,V3,x,deed,,x=1,,,found,,,AB,20/10/2026\n\
         EX/1,V4,x,deed,,x=1,,,found,,,machine:me,2026-10-20\n\
         EX/1,V5,x,deed,,,,,found,,,AB,2026-10-20\n",
    );
    std::fs::write(root.path().join("labels/notes.csv"), "a,b\n1,2\n").unwrap();
    let (got, rejected) = labels::load(&root.path().join("labels")).unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].source, "gla");
    assert_eq!(got[0].minutes, Some(4));
    let reasons: Vec<&str> = rejected.iter().map(|r| r.reason.as_str()).collect();
    assert_eq!(rejected.len(), 5, "{reasons:?}");
    assert!(reasons[0].starts_with("whole file"), "{reasons:?}");
    assert!(reasons.iter().any(|r| r.contains("not a label")));
    assert_eq!(rejected[1].origin, "sheet.csv:4");
}

#[test]
fn a_later_submission_supersedes_per_labeller_and_cell() {
    let root = tempfile::tempdir().unwrap();
    submit(
        root.path(),
        "2026-10-20-ab.csv",
        "EX/1,V1,x,deed,,total=10,units,3,found,,,AB,2026-10-20\n",
    );
    submit(
        root.path(),
        "2026-10-22-ab-fix.csv",
        "EX/1,V1,x,deed,,total=12,units,3,found,,,AB,2026-10-22\n",
    );
    let (got, _) = labels::load(&root.path().join("labels")).unwrap();
    let current = labels::latest(&got);
    assert_eq!(current.len(), 1);
    assert_eq!(current[0].value, "total=12");
}

#[test]
fn fields_compare_numbers_as_numbers() {
    let a = machine("EX/1", "V2", "deed", "share=35.0%", "units");
    let b = machine("EX/1", "V2", "deed", "share = 35", "Units");
    assert_eq!(a.fields(), b.fields());
    assert_eq!(a.fields()["basis"], "units");
    let c = machine("EX/1", "V5", "deed", "AR:  80% of  market rent", "");
    assert_eq!(c.fields()["value"], "ar: 80% of market rent");
}

#[test]
fn repeated_other_tenures_are_kept_apart() {
    let a = machine(
        "EX/1",
        "V1",
        "deed",
        "total=20; other=Off-site rented:8; other=off-site intermediate:8",
        "units",
    );
    let f = a.fields();
    assert_eq!(f["other:off-site rented"], "8");
    assert_eq!(f["other:off-site intermediate"], "8");
}

#[test]
fn pipeline_scored_field_by_field() {
    let root = tempfile::tempdir().unwrap();
    submit(
        root.path(),
        "a.csv",
        "EX/1,V1,x,deed,,total=120; market=84; SR=24; SO=12,units,3,found,10,,AB,2026-10-20\n\
         EX/2,V1,x,deed,,,units,3,not_stated,10,,AB,2026-10-20\n\
         EX/3,V1,x,deed,,,,,source_missing,1,,AB,2026-10-20\n",
    );
    let (got, rejected) = labels::load(&root.path().join("labels")).unwrap();
    let readings = [
        // SR wrong, SO missed, one field the label doesn't have
        machine(
            "EX/1",
            "V1",
            "deed",
            "total=120; market=84; SR=20; AR=5",
            "units",
        ),
        // the label says the deed doesn't state it
        machine("EX/2", "V1", "deed", "total=50", ""),
        // not scored: the source wasn't read
        machine("EX/3", "V1", "deed", "total=1", ""),
        // nobody labelled this cell
        machine("EX/9", "V1", "deed", "total=1", ""),
    ];
    let r = labels::report(&got, rejected, &readings, vec![]);
    assert_eq!(r.accuracy.len(), 1);
    let a = &r.accuracy[0];
    assert_eq!((a.cells, a.not_scored), (3, 1));
    // label fields: total, market, sr, so, basis -> right: total, market, basis
    assert_eq!(a.true_pos, 3);
    // sr wrong, so missed
    assert_eq!(a.false_neg, 2);
    // sr wrong, ar extra, EX/2's total
    assert_eq!(a.false_pos, 3);
    let p = a.precision.as_ref().unwrap();
    assert_eq!((p.k, p.n), (3, 6));
    assert!(p.low < 0.5 && p.high > 0.5);
    let md = labels::markdown(&r);
    assert!(md.contains("sr 20 (label 24)"), "{md}");
}

#[test]
fn labellers_and_sources_compared() {
    let root = tempfile::tempdir().unwrap();
    submit(
        root.path(),
        "ab.csv",
        "EX/1,V1,x,deed,,total=120; SR=24,units,3,found,,,AB,2026-10-20\n\
         EX/1,V1,x,committee_report,,total=120; SR=30,units,3,found,,,AB,2026-10-20\n\
         EX/2,V1,x,deed,,total=50,units,3,found,,,AB,2026-10-20\n\
         EX/2,V1,x,gla,,,,,no_source,,,AB,2026-10-20\n",
    );
    submit(
        root.path(),
        "cd.csv",
        "EX/1,V1,x,deed,,total=120; SR=24,units,3,found,,,CD,2026-10-21\n\
         EX/2,V1,x,deed,,total=55,units,3,found,,,CD,2026-10-21\n\
         EX/2,V1,x,gla,,,,,no_source,,,CD,2026-10-21\n",
    );
    let (got, rejected) = labels::load(&root.path().join("labels")).unwrap();
    let r = labels::report(&got, rejected, &[], vec![]);
    assert_eq!(r.labellers.len(), 1);
    let l = &r.labellers[0];
    assert_eq!(
        (l.cells, l.same_status, l.both_found, l.same_value),
        (3, 3, 2, 1)
    );
    // every status agrees, over two categories (found, no_source)
    assert_eq!(l.kappa, Some(1.0));
    assert_eq!(r.sources.len(), 1);
    let s = &r.sources[0];
    assert_eq!((s.a.as_str(), s.b.as_str()), ("committee_report", "deed"));
    assert_eq!((s.schemes, s.agree), (1, 0));
    // EX/2's deed cell is unsettled (AB 50, CD 55)
    assert_eq!(s.unsettled, 1);
}

#[test]
fn kappa_matches_a_worked_example() {
    // 50 items: both yes 20, both no 15, A yes B no 5, A no B yes 10 -> po .70, pe .50, kappa .40
    let mut pairs = vec![];
    pairs.extend(std::iter::repeat_n(("y", "y"), 20));
    pairs.extend(std::iter::repeat_n(("n", "n"), 15));
    pairs.extend(std::iter::repeat_n(("y", "n"), 5));
    pairs.extend(std::iter::repeat_n(("n", "y"), 10));
    let k = labels::kappa(&pairs).unwrap();
    assert!((k - 0.4).abs() < 1e-9, "{k}");
    assert_eq!(labels::kappa(&[("y", "y")]), None);
}

#[test]
fn datahub_record_read_in_codebook_format() {
    let root = tempfile::tempdir().unwrap();
    let dir = root
        .path()
        .join("snapshots/pld/applications/Exampleton-EX_23_0456");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/synthetic/pld/application.json"),
        dir.join("2026-01-02T03_04_05Z.json"),
    )
    .unwrap();
    std::fs::create_dir_all(root.path().join("snapshots/pld/applications/_search/x")).unwrap();
    let (readings, notes) = labels::machine_readings(root.path(), "claude-haiku-5-5");
    assert!(notes.is_empty(), "{notes:?}");
    let v8 = readings.iter().find(|r| r.variable_id == "V8").unwrap();
    assert_eq!(v8.scheme, "EX/23/0456");
    assert_eq!(v8.value, "completed=2025-07-19");
    let v1 = readings.iter().find(|r| r.variable_id == "V1").unwrap();
    assert!(v1.value.starts_with("total="), "{}", v1.value);
    assert!(
        !v1.value.contains("=0"),
        "zero tenures are left out: {}",
        v1.value
    );
}

#[test]
fn same_day_rows_resolve_by_row_order_not_text() {
    let root = tempfile::tempdir().unwrap();
    // the correction is on line 12, after a line 9 that would sort later as text
    let mut rows = String::new();
    for i in 2..=8 {
        rows.push_str(&format!("EX/{i},V1,x,gla,,,,,not_stated,,,AB,2026-10-20\n"));
    }
    rows.push_str("EX/1,V1,x,deed,,total=10,units,3,found,,,AB,2026-10-20\n");
    for i in 10..=11 {
        rows.push_str(&format!("EX/{i},V1,x,gla,,,,,not_stated,,,AB,2026-10-20\n"));
    }
    rows.push_str("EX/1,V1,x,deed,,total=12,units,3,found,,,AB,2026-10-20\n");
    submit(root.path(), "a.csv", &rows);
    let (got, _) = labels::load(&root.path().join("labels")).unwrap();
    let current = labels::latest(&got);
    let deed = current.iter().find(|r| r.source == "deed").unwrap();
    assert_eq!(deed.value, "total=12", "{}", deed.origin);
    assert_eq!(deed.origin, "a.csv:12");
}

#[test]
fn scheme_refs_match_the_pipeline_whatever_their_case() {
    let root = tempfile::tempdir().unwrap();
    submit(
        root.path(),
        "a.csv",
        "ex/2,v1,x,Deed,,total=50,units,3,found,,,AB,2026-10-20\n",
    );
    let (got, rejected) = labels::load(&root.path().join("labels")).unwrap();
    let r = labels::report(
        &got,
        rejected,
        &[machine("EX/2", "V1", "deed", "total=50", "units")],
        vec![],
    );
    assert_eq!(r.accuracy.len(), 1);
    assert_eq!((r.accuracy[0].true_pos, r.accuracy[0].false_pos), (2, 0));
}

#[test]
fn basis_scored_only_where_the_label_states_one() {
    let root = tempfile::tempdir().unwrap();
    submit(
        root.path(),
        "a.csv",
        "EX/1,V2,x,deed,,share=35%,,3,found,,,AB,2026-10-20\n",
    );
    let (got, rejected) = labels::load(&root.path().join("labels")).unwrap();
    let r = labels::report(
        &got,
        rejected,
        &[machine(
            "EX/1",
            "V2",
            "deed",
            "share=35%",
            "habitable_rooms",
        )],
        vec![],
    );
    let a = &r.accuracy[0];
    assert_eq!((a.true_pos, a.false_pos, a.false_neg), (1, 0, 0));
}

#[test]
fn sources_sharing_only_a_basis_are_not_comparable() {
    let root = tempfile::tempdir().unwrap();
    submit(
        root.path(),
        "a.csv",
        "EX/1,V2,x,deed,,share=35%,units,3,found,,,AB,2026-10-20\n\
         EX/1,V2,x,gla,,split=70:30 SR:INT,units,3,found,,,AB,2026-10-20\n",
    );
    let (got, rejected) = labels::load(&root.path().join("labels")).unwrap();
    let r = labels::report(&got, rejected, &[], vec![]);
    let s = &r.sources[0];
    assert_eq!((s.schemes, s.agree, s.not_comparable), (1, 0, 1));
    assert!(s.differing.is_empty());
}

#[test]
fn datahub_v8_scored_on_the_date_only() {
    let root = tempfile::tempdir().unwrap();
    submit(
        root.path(),
        "a.csv",
        "EX/1,V8,x,datahub,,completed=2025-07-19; completed_homes=120,,f,found,,,AB,2026-10-20\n",
    );
    let (got, rejected) = labels::load(&root.path().join("labels")).unwrap();
    let mut m = machine("EX/1", "V8", "datahub", "completed=2025-07-19", "");
    m.labeller = "machine:pld-mapping/v1".into();
    let r = labels::report(&got, rejected, &[m], vec![]);
    let a = &r.accuracy[0];
    assert_eq!((a.true_pos, a.false_pos, a.false_neg), (1, 0, 0));
}

#[test]
fn method_column_is_optional_and_checked() {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("labels");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("audit.csv"),
        "scheme,variable_id,source,value,basis,status,labeller,labelled_on,method\n\
         EX/1,V1,deed,total=10,units,found,AB,2026-10-20,audited\n\
         EX/1,V2,deed,share=35%,units,found,AB,2026-10-20,Blind\n\
         EX/1,V3,deed,,,not_stated,AB,2026-10-20,\n\
         EX/1,V4,deed,x=1,,found,AB,2026-10-20,guessed\n",
    )
    .unwrap();
    let (got, rejected) = labels::load(&dir).unwrap();
    let methods: Vec<&str> = got.iter().map(|r| r.method.as_str()).collect();
    assert_eq!(methods, ["audited", "blind", "read"]);
    assert_eq!(rejected.len(), 1);
    assert!(
        rejected[0].reason.contains("method `guessed`"),
        "{rejected:?}"
    );
    let r = labels::report(&got, rejected, &[], vec![]);
    assert_eq!(r.labels.by_method["read"], 1);
    assert!(labels::markdown(&r).contains("| audited | 1 |"));
}

#[test]
fn proposals_are_scored_as_machine_readings_split_by_label_kind() {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("labels");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("audit.csv"),
        "scheme,variable_id,source,value,basis,status,labeller,labelled_on,method\n\
         EX/1,V1,deed,total=10,units,found,AB,2026-10-20,audited\n\
         EX/2,V1,deed,total=12,units,found,AB,2026-10-20,corrected\n\
         EX/3,V1,deed,total=30,units,found,AB,2026-10-20,blind\n",
    )
    .unwrap();
    let proposals = root.path().join("proposals");
    std::fs::create_dir_all(&proposals).unwrap();
    std::fs::write(
        proposals.join("2026-10-14-agent-batch1.csv"),
        "scheme,variable_id,source,value,basis,status,labeller,labelled_on,quote\n\
         EX/1,V1,deed,total=10,units,found,machine:proposal/agent,2026-10-14,ten dwellings\n\
         EX/2,V1,deed,total=20,units,found,machine:proposal/agent,2026-10-14,twenty\n\
         EX/3,V1,deed,total=30,units,found,machine:proposal/agent,2026-10-14,thirty\n\
         EX/4,V1,deed,total=1,units,found,AB,2026-10-14,signed by a person\n",
    )
    .unwrap();
    let (proposed, bad) = labels::proposals(root.path()).unwrap();
    assert_eq!(proposed.len(), 3);
    assert_eq!(bad.len(), 1);
    assert!(bad[0].origin.starts_with("proposals/"), "{bad:?}");
    assert!(bad[0].reason.contains("signed"), "{bad:?}");
    // machine-signed rows never load as labels, wherever the file is
    let (as_labels, refused) = labels::load(&proposals).unwrap();
    assert!(as_labels.iter().all(|r| r.labeller == "AB"));
    assert_eq!(refused.len(), 3);

    let r = labels::run(root.path(), "claude-haiku-5-5").unwrap();
    let rows: Vec<(&str, usize, usize)> = r
        .accuracy
        .iter()
        .map(|a| (a.labels, a.cells, a.true_pos))
        .collect();
    // anchored: EX/1 confirmed, EX/2 corrected; independent: EX/3 blind
    assert_eq!(rows, [("anchored", 2, 3), ("independent", 1, 2)]);
    let md = std::fs::read_to_string(root.path().join("reports/labels/labels.md")).unwrap();
    assert!(
        md.contains("| machine:proposal/agent | V1 | deed | anchored | 2 |"),
        "{md}"
    );
    assert!(md.contains("total 20 (label 12)"), "{md}");
}
