//! Regression against the frozen outputs of the former Python harness (M1), recorded before it
//! was retired (`fixtures/golden-python-m1/*.llm-pdf.audit.json`).
//!
//! For each of the ten deeds: load the page text (cached OCR or text layer), replay the recorded
//! model response, verify, and compare every Python check (by id and description) on status and
//! error class. The only differences allowed are those listed, with a reason, in
//! `fixtures/golden-python-m1/expected-differences.json`.
//!
//! Needs the deeds locally (data/raw/southwark_s106_sample.pdf, data/raw/eval/*.pdf; see README)
//! and the OCR models. Ignored by default because the first run OCRs ~230 pages:
//!     cargo test --release --test golden_python_m1 -- --ignored

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use s106::document::{self, Config};
use s106::extract;
use s106::wire::PageMap;
use s106::verify::{ErrorClass, Status, verify};
use serde_json::Value;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn pdf_for(deed: &str) -> PathBuf {
    let direct = root().join(format!("data/raw/{deed}.pdf"));
    if direct.exists() { direct } else { root().join(format!("data/raw/eval/{deed}.pdf")) }
}

fn status_name(s: Status) -> &'static str {
    match s { Status::Pass => "PASS", Status::Review => "REVIEW", Status::Fail => "FAIL", Status::Skip => "SKIP" }
}

fn class_name(c: Option<ErrorClass>) -> Option<&'static str> {
    c.map(|c| match c { ErrorClass::ParserError => "Parser Error", ErrorClass::DocumentDiscrepancy => "Document Discrepancy" })
}

#[test]
#[ignore = "needs the local deed corpus and OCR models; run with --ignored"]
fn rust_reproduces_python_m1() {
    let golden_dir = root().join("fixtures/golden-python-m1");
    let expected: Vec<Value> = serde_json::from_str(&std::fs::read_to_string(golden_dir.join("expected-differences.json")).unwrap()).unwrap();
    let allowed: BTreeMap<(String, String), (String, String)> = expected.iter().map(|e| (
        (e["deed"].as_str().unwrap().into(), e["check"].as_str().unwrap().into()),
        (e["python"].as_str().unwrap().into(), e["rust"].as_str().unwrap().into()),
    )).collect();

    let mut goldens: Vec<PathBuf> = std::fs::read_dir(&golden_dir).unwrap().filter_map(|e| {
        let p = e.unwrap().path();
        p.to_string_lossy().ends_with(".llm-pdf.audit.json").then_some(p)
    }).collect();
    goldens.sort();
    assert_eq!(goldens.len(), 10, "expected ten frozen Python audits");

    let missing: Vec<String> = goldens.iter().map(|g| deed_of(g)).filter(|d| !pdf_for(d).exists()).collect();
    assert!(missing.is_empty(), "corpus missing: {missing:?} (fetch the deeds listed in data/reference/candidates.tsv and README)");

    let cfg = Config::from_env(&root());
    let (mut compared, mut problems) = (0, vec![]);
    let mut used: Vec<(String, String)> = vec![];
    for g in &goldens {
        let deed = deed_of(g);
        let doc = document::load(&pdf_for(&deed), &cfg, false).unwrap();
        // replay the v1 fixture Python used, even where a newer v2 fixture exists
        let fx: Value = serde_json::from_str(&std::fs::read_to_string(
            root().join(format!("fixtures/llm-v1/{deed}/claude-haiku-5-5.extract.json"))).unwrap()).unwrap();
        assert_eq!(fx["pdf_sha256"].as_str(), Some(doc.sha256.as_str()), "{deed}: v1 fixture is for a different PDF");
        let pm = PageMap(serde_json::from_value(fx["pages"].clone()).unwrap());
        let info = extract::Info { model: "claude-haiku-5-5".into(), served_by: "claude-haiku-5-5".into(),
                                   source: "replay-v1".into(), usage: None, fixture: PathBuf::new() };
        let agreement = extract::to_agreement(&doc, &fx["output"], &pm, &info).unwrap();
        let ours: BTreeMap<String, (String, Option<&str>)> = verify(&agreement, &doc.pages()).into_iter()
            .map(|r| (format!("{}|{}", r.check_id, r.description), (status_name(r.status).to_string(), class_name(r.error_class))))
            .collect();
        let py: Value = serde_json::from_str(&std::fs::read_to_string(g).unwrap()).unwrap();
        for r in py["verification"].as_array().unwrap() {
            let id = r["check_id"].as_str().unwrap();
            let key = format!("{id}|{}", r["description"].as_str().unwrap());
            let (ps, pc) = (r["status"].as_str().unwrap(), r["error_class"].as_str());
            compared += 1;
            match ours.get(&key) {
                None => problems.push(format!("{deed} {key}: missing in Rust")),
                Some((rs, rc)) if rs == ps && *rc == pc => {}
                Some((rs, _)) => match allowed.get(&(deed.clone(), id.to_string())) {
                    Some((a, b)) if a == ps && b == rs => used.push((deed.clone(), id.to_string())),
                    _ => problems.push(format!("{deed} {key}: python {ps} {pc:?} -> rust {rs}")),
                },
            }
        }
    }
    let unused: Vec<_> = allowed.keys().filter(|k| !used.contains(k)).collect();
    assert!(problems.is_empty(), "{} of {compared} checks differ:\n{}", problems.len(), problems.join("\n"));
    assert!(unused.is_empty(), "expected differences that no longer occur (remove them): {unused:?}");
    eprintln!("{compared} Python checks reproduced ({} documented differences)", used.len());
}

fn deed_of(path: &Path) -> String {
    path.file_name().unwrap().to_string_lossy().split('.').next().unwrap().to_string()
}
