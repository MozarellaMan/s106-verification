//! Tier 2: offline replay. Page text comes from committed snapshots (`fixtures/pages/`) and model
//! output from recorded responses (`fixtures/llm/`), so these run in the default `cargo test`
//! without a PDF, PDFium, ONNX Runtime, OCR models, credentials or network.
//!
//! The snapshots are only as good as their agreement with the real pipeline; the real-PDF tiers
//! check that (`tests/pdf_regression.rs` on difficult pages, `tests/corpus.rs` on whole deeds).
//! Expected outcomes live in `fixtures/expected/`; after an intended change, regenerate them with
//! `S106_BLESS=1 cargo test --test replay` and review the diff.

mod common;

use std::collections::BTreeMap;

use common::{bless, corpus, root, snapshot};
use s106::document::Config;
use s106::eval::{self, EvalArgs};
use s106::extract::{self, Fixture, Mode};
use s106::ocrbench::{self, Gold};
use s106::verify::verify;
use serde_json::{Value, json};

#[test]
fn every_deed_has_a_current_snapshot() {
    let producer = Config::from_env(&root()).producer();
    for deed in corpus() {
        let snap = snapshot(&deed.id);
        assert_eq!(
            snap.sha256, deed.sha256,
            "{}: snapshot is of another PDF",
            deed.id
        );
        assert_eq!(snap.source_file, format!("{}.pdf", deed.id));
        assert_eq!(
            snap.producer, producer,
            "{}: snapshot was produced by different page-text settings or runtime artifacts; \
             regenerate with `s106 pages --write fixtures/pages <pdf>` and review the diff",
            deed.id
        );
        let fx: Fixture = serde_json::from_str(
            &std::fs::read_to_string(root().join(format!(
                "fixtures/llm/{}/claude-haiku-5-5.extract.v2.json",
                deed.id
            )))
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            fx.pdf_sha256, snap.sha256,
            "{}: fixture is for another PDF",
            deed.id
        );
        assert_eq!(fx.pages.len(), snap.page_count, "{}", deed.id);
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
fn replay_reproduces_expected_outcomes() {
    let dir = root().join("fixtures/expected");
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
        "replayed outcomes changed (if intended: S106_BLESS=1 cargo test --test replay):\n{}",
        problems.join("\n")
    );
}

#[test]
fn snapshot_ocr_meets_the_gold_thresholds() {
    for (gold, id, check) in [
        (
            "gold/southwark_19_AP_1773.json",
            "southwark_s106_sample",
            common::assert_southwark_gold as fn(&ocrbench::Score),
        ),
        (
            "gold/salford_20_75239.json",
            "salford-75239",
            common::assert_salford_gold,
        ),
    ] {
        let g = Gold::load(&root().join(gold)).unwrap();
        let doc = snapshot(id).document();
        assert_eq!(doc.sha256, g.sha256, "{gold} describes another document");
        check(&ocrbench::score(&g, &doc));
    }
}

/// The register evaluation in replay mode, from snapshots: gate counts, register matches and
/// misses for every candidate deed (run times removed).
#[test]
fn eval_replay_matches_expected_summary() {
    let out = tempfile::tempdir().unwrap();
    let args = EvalArgs {
        replay: true,
        pages: Some(common::snapshot_dir()),
        out: out.path().to_path_buf(),
        ..EvalArgs::defaults(&root())
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
    let path = root().join("fixtures/expected/eval-replay.json");
    if bless() {
        std::fs::write(&path, &got).unwrap();
        return;
    }
    let want = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(
        got == want,
        "eval --replay summary changed (if intended: S106_BLESS=1 cargo test --test replay)\n\
         got:\n{got}"
    );
}

#[test]
fn eval_fails_unless_every_selected_deed_completes() {
    // a root with no PDFs: every deed is "corpus missing", so the command must not succeed
    let empty = tempfile::tempdir().unwrap();
    let args = EvalArgs {
        replay: true,
        out: empty.path().join("out"),
        ..EvalArgs::defaults(&root())
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
}
