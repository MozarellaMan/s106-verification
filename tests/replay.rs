//! Data tier: offline replay of the real corpus. Page text comes from the snapshots and model
//! output from the recorded responses in the data repository (`s106::data`), so these need no
//! PDF, PDFium, ONNX Runtime, OCR models, credentials or network, only the data checkout. They
//! are ignored by the default `cargo test` (which replays the invented deeds in
//! `tests/synthetic.rs`); run them with `cargo test-data`.
//!
//! The snapshots are only as good as their agreement with the real pipeline; the real-PDF tiers
//! check that (`tests/pdf_regression.rs` on difficult pages, `tests/corpus.rs` on whole deeds).
//! Expected outcomes live in the data repository's `expected/`; after an intended change, regenerate them with
//! `S106_BLESS=1 cargo test-data` and review the diff.

mod common;

use std::collections::{BTreeMap, BTreeSet};

use common::{bless, corpus, data, root, snapshot};
use s106::document::Config;
use s106::eval::{self, EvalArgs};
use s106::extract::{self, Fixture, Mode};
use s106::ocrbench;
use s106::verify::verify;
use serde_json::{Value, json};

#[test]
#[ignore = "data tier: needs the s106-data checkout; cargo test-data"]
fn every_deed_has_a_current_snapshot() {
    for deed in corpus() {
        let snap = snapshot(&deed.id);
        // a snapshot keeps the model set it was made with; everything else must be current
        let producer = Config::from_env(&root())
            .with_models_of(&snap.producer)
            .unwrap_or_else(|| panic!("{}: unknown OCR model set", deed.id))
            .producer();
        assert_eq!(
            snap.sha256, deed.sha256,
            "{}: snapshot is of another PDF",
            deed.id
        );
        assert_eq!(snap.source_file, format!("{}.pdf", deed.id));
        assert_eq!(
            snap.producer, producer,
            "{}: snapshot was produced by different page-text settings or runtime artifacts; \
             regenerate with `s106 pages --write data/pages <pdf>` and review the diff",
            deed.id
        );
        let fx: Fixture = serde_json::from_str(
            &std::fs::read_to_string(
                s106::data::extractions()
                    .join(&deed.id)
                    .join("claude-haiku-5-5.extract.v2.json"),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            fx.pdf_sha256, snap.sha256,
            "{}: fixture is for another PDF",
            deed.id
        );
        assert_eq!(fx.pages.len(), snap.page_count, "{}", deed.id);
        assert_eq!(
            fx.prompt_sha256,
            extract::prompt_sha256(),
            "{}: recorded with an older prompt or schema; re-record it",
            deed.id
        );
    }
}

/// Every check's id, status and error class: what a verifier or extraction change would move.
fn outcomes(id: &str) -> Vec<Value> {
    let doc = snapshot(id).document();
    let (agreement, info) = extract::extract(&doc, "claude-haiku-5-5", Mode::Replay, None).unwrap();
    assert_eq!(info.source, "replay");
    verify(&agreement, &doc.pages())
        .into_iter()
        .map(|r| {
            json!({
                "check": r.check_id,
                "description": r.description,
                "status": r.status,
                "error_class": r.error_class,
            })
        })
        .collect()
}

/// One check per line, so a changed outcome is a one-line diff.
fn lines(values: &[Value]) -> String {
    let body: Vec<String> = values.iter().map(|v| format!("  {v}")).collect();
    format!("[\n{}\n]\n", body.join(",\n"))
}

#[test]
#[ignore = "data tier: needs the s106-data checkout; cargo test-data"]
fn replay_reproduces_expected_outcomes() {
    let dir = s106::data::expected();
    let mut problems = vec![];
    for deed in corpus() {
        let got = lines(&outcomes(&deed.id));
        let path = dir.join(format!("{}.checks.json", deed.id));
        if bless() {
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(&path, &got).unwrap();
            continue;
        }
        let want = std::fs::read_to_string(&path).unwrap_or_default();
        if got != want {
            let (w, g): (Vec<&str>, Vec<&str>) = (want.lines().collect(), got.lines().collect());
            let changed: Vec<String> = g
                .iter()
                .filter(|l| !w.contains(l))
                .map(|l| format!("  now: {}", l.trim()))
                .chain(
                    w.iter()
                        .filter(|l| !g.contains(l))
                        .map(|l| format!("  was: {}", l.trim())),
                )
                .collect();
            problems.push(format!("{}:\n{}", deed.id, changed.join("\n")));
        }
    }
    assert!(
        problems.is_empty(),
        "replayed outcomes changed (if intended: S106_BLESS=1 cargo test-data):\n{}",
        problems.join("\n")
    );
}

#[test]
#[ignore = "data tier: needs the s106-data checkout; cargo test-data"]
fn snapshot_ocr_meets_the_gold_thresholds() {
    let results: Vec<_> = common::gold_sets()
        .into_iter()
        .map(|(path, g, deed)| {
            let score = ocrbench::score(&g, &snapshot(&deed.id).document());
            (path, g.thresholds.failures(&score))
        })
        .collect();
    common::assert_gold(&results);
}

/// The register evaluation in replay mode, from snapshots: gate counts, register matches and
/// misses for every candidate deed (run times removed).
#[test]
#[ignore = "data tier: needs the s106-data checkout; cargo test-data"]
fn eval_replay_matches_expected_summary() {
    data();
    let out = tempfile::tempdir().unwrap();
    let args = EvalArgs {
        replay: true,
        pages: Some(common::snapshot_dir()),
        out: out.path().to_path_buf(),
        ..EvalArgs::defaults()
    };
    let summary = eval::evaluate_all(&args, &root()).unwrap();
    assert!(eval::incomplete(&summary, &args).is_empty());
    let mut got = serde_json::to_value(&summary).unwrap();
    for deed in got.as_array_mut().unwrap() {
        for run in deed["runs"].as_array_mut().unwrap() {
            run.as_object_mut().unwrap().remove("seconds");
            assert!(run.get("error").is_none(), "{run}");
        }
    }
    let by_id: BTreeMap<String, Value> = got
        .as_array()
        .unwrap()
        .iter()
        .map(|d| (d["id"].as_str().unwrap().to_string(), d.clone()))
        .collect();
    let got = serde_json::to_string_pretty(&by_id).unwrap() + "\n";
    let path = s106::data::expected().join("eval-replay.json");
    if bless() {
        std::fs::write(&path, &got).unwrap();
        return;
    }
    let want = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(
        got == want,
        "eval --replay summary changed (if intended: S106_BLESS=1 cargo test-data)\n\
         got:\n{got}"
    );
}

#[test]
#[ignore = "data tier: needs the s106-data checkout; cargo test-data"]
fn eval_fails_unless_every_selected_deed_completes() {
    // a root with no PDFs: every deed is "corpus missing", so the command must not succeed
    data();
    let empty = tempfile::tempdir().unwrap();
    let args = EvalArgs {
        replay: true,
        out: empty.path().join("out"),
        pdfs: empty.path().to_path_buf(),
        ..EvalArgs::defaults()
    };
    let summary = eval::evaluate_all(&args, empty.path()).unwrap();
    assert_eq!(eval::incomplete(&summary, &args).len(), summary.len());
    let err = eval::run(&args, empty.path()).unwrap_err();
    assert!(err.to_string().contains("9 of 9 deeds incomplete"), "{err}");

    let unknown = EvalArgs {
        only: vec!["no-such-deed".into()],
        ..args
    };
    assert!(eval::evaluate_all(&unknown, empty.path()).is_err());

    let header_only = empty.path().join("candidates.tsv");
    std::fs::write(&header_only, "id\tcouncil\tplanning_ref\ttype\n").unwrap();
    let none = EvalArgs {
        only: vec![],
        candidates: header_only,
        ..unknown
    };
    let err = eval::evaluate_all(&none, empty.path()).unwrap_err();
    assert!(err.to_string().contains("lists no candidates"), "{err}");
}

/// What the number parser does on every string the recorded responses cite: the data-tier
/// counterpart of `tests/numeric_snapshot.rs`, whose probes are invented. A change shows up as a
/// diff of `expected/parser-on-extractions.txt`.
#[test]
#[ignore = "data tier: needs the s106-data checkout; cargo test-data"]
fn parser_behaviour_on_recorded_strings() {
    const KEYS: [&str; 5] = [
        "snippet",
        "amount_in_words",
        "amount_gbp",
        "share_pct",
        "claimed_affordable_pct",
    ];
    fn collect(v: &Value, key: Option<&str>, out: &mut BTreeSet<String>) {
        match v {
            Value::Object(m) => m.iter().for_each(|(k, v)| collect(v, Some(k), out)),
            Value::Array(a) => a.iter().for_each(|v| collect(v, key, out)),
            Value::String(s) if !s.is_empty() && key.is_some_and(|k| KEYS.contains(&k)) => {
                out.insert(s106::audit::escape_probe(s));
            }
            _ => {}
        }
    }
    data();
    let mut strings = BTreeSet::new();
    for dir in [s106::data::extractions(), s106::data::extractions_v1()] {
        for deed in std::fs::read_dir(dir).unwrap() {
            for f in std::fs::read_dir(deed.unwrap().path()).unwrap() {
                let v: Value =
                    serde_json::from_str(&std::fs::read_to_string(f.unwrap().path()).unwrap())
                        .unwrap();
                collect(&v["output"], None, &mut strings);
            }
        }
    }
    let probes: String = strings.iter().map(|s| format!("{s}\n")).collect();
    let got = s106::audit::render_probes(&probes);
    let path = s106::data::expected().join("parser-on-extractions.txt");
    if bless() {
        std::fs::write(&path, &got).unwrap();
        return;
    }
    let want = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(
        got == want,
        "parser results on recorded strings changed (if intended: S106_BLESS=1 cargo test-data, \
         then review the diff in the data repository)"
    );
}

#[test]
#[ignore = "data tier: needs the s106-data checkout; cargo test-data"]
fn recorded_words_replay_with_negative_twins() {
    // Recorded figures are compatibility references, not independently verified legal truth.
    data();
    let mut checked = 0;
    for dir in [s106::data::extractions(), s106::data::extractions_v1()] {
        for deed in std::fs::read_dir(dir).unwrap() {
            for fixture in std::fs::read_dir(deed.unwrap().path()).unwrap() {
                let path = fixture.unwrap().path();
                let raw: serde_json::Value =
                    serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
                for c in raw["output"]["contributions"].as_array().unwrap() {
                    let Some(words) = c["amount_in_words"].as_str().filter(|s| !s.is_empty())
                    else {
                        continue;
                    };
                    let figure =
                        rust_decimal::Decimal::from_str_exact(c["amount_gbp"].as_str().unwrap())
                            .unwrap();
                    assert_eq!(
                        s106::quantity::amount(words).unwrap(),
                        figure,
                        "{}: {words}",
                        path.display()
                    );
                    assert!(
                        s106::quantity::amount(&format!("{words} banana")).is_err(),
                        "{}: corrupted {words}",
                        path.display()
                    );
                    checked += 1;
                }
            }
        }
    }
    assert!(checked > 0, "no recorded words fields found");
    eprintln!("{checked} recorded words fields reparsed; {checked} corrupted twins rejected");
}
