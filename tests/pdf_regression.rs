//! Tier 3: targeted real-PDF regression. Difficult pages are read afresh (PDFium text layer or
//! OCR, into an empty cache) and must match their committed snapshots, and the hand-transcribed
//! gold pages must meet the OCR thresholds. A few dozen pages rather than whole deeds, so this
//! runs in under a minute; it needs the corpus (scripts/fetch-corpus.sh) and runtime artifacts
//! (scripts/fetch-models.sh):
//!     cargo test --release --test pdf_regression -- --ignored

mod common;

use common::{corpus, require_pdf, root, snapshot};
use s106::document::{self, Config};
use s106::ocrbench::{Gold, Score, score};

/// The configured pipeline with an empty cache, so every page is really read.
fn fresh() -> (Config, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = Config::from_env(&root());
    cfg.cache_root = dir.path().to_path_buf();
    (cfg, dir)
}

fn gold(gold: &str, id: &str) -> Score {
    let deed = corpus().into_iter().find(|d| d.id == id).unwrap();
    require_pdf(&deed.pdf);
    let g = Gold::load(&root().join(gold)).unwrap();
    assert_eq!(g.sha256, deed.sha256, "{gold} describes another document");
    let (cfg, _dir) = fresh();
    let d = document::load_pages(&deed.pdf, &cfg, Some(&g.pages()), false).unwrap();
    let s = score(&g, &d);
    eprintln!("{gold}\n{s}");
    s
}

#[test]
#[ignore = "needs the corpus and runtime artifacts: cargo test --release --test pdf_regression -- --ignored"]
fn southwark_scan_gold() {
    common::assert_southwark_gold(&gold(
        "gold/southwark_19_AP_1773.json",
        "southwark_s106_sample",
    ));
}

#[test]
#[ignore = "needs the corpus and runtime artifacts: cargo test --release --test pdf_regression -- --ignored"]
fn salford_text_layer_gold() {
    common::assert_salford_gold(&gold("gold/salford_20_75239.json", "salford-75239"));
}

/// fixtures/pages/difficult.tsv: deed, page, and why the page is hard.
#[test]
#[ignore = "needs the corpus and runtime artifacts: cargo test --release --test pdf_regression -- --ignored"]
fn difficult_pages_match_their_snapshots() {
    let list = std::fs::read_to_string(root().join("fixtures/pages/difficult.tsv")).unwrap();
    let mut by_deed: std::collections::BTreeMap<String, Vec<usize>> = Default::default();
    for l in list
        .lines()
        .filter(|l| !l.starts_with('#') && !l.starts_with("id\t"))
    {
        let f: Vec<&str> = l.split('\t').collect();
        by_deed
            .entry(f[0].into())
            .or_default()
            .push(f[1].parse().unwrap());
    }
    let (cfg, _dir) = fresh();
    let mut problems = vec![];
    for (id, pages) in &by_deed {
        let deed = corpus().into_iter().find(|d| &d.id == id).unwrap();
        require_pdf(&deed.pdf);
        let t0 = std::time::Instant::now();
        let doc = document::load_pages(&deed.pdf, &cfg, Some(pages), false).unwrap();
        assert_eq!(doc.page_count(), pages.len(), "{id}: pages {pages:?}");
        eprintln!(
            "{id}: {} pages in {:.1}s",
            pages.len(),
            t0.elapsed().as_secs_f64()
        );
        problems.extend(
            snapshot(id)
                .compare_pages(&doc)
                .into_iter()
                .map(|d| format!("{id} {d}")),
        );
    }
    assert!(
        problems.is_empty(),
        "fresh page text differs from the snapshots (if intended, bump PAGE_TEXT_LOGIC and \
         regenerate with `s106 pages --write fixtures/pages`):\n{}",
        problems.join("\n")
    );
}
