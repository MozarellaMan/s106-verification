//! Tier 3: targeted real-PDF regression. Difficult pages are read afresh (PDFium text layer or
//! OCR, into an empty cache) and must match their committed snapshots, and the hand-transcribed
//! gold pages (`gold/` in the data repository) must meet the OCR thresholds. A few dozen pages
//! rather than whole deeds, so this tier runs in under a minute; it needs the corpus
//! (scripts/fetch-corpus.sh) and runtime artifacts (scripts/fetch-models.sh):
//!     cargo test --release --test pdf_regression -- --ignored

mod common;

use common::{corpus, data, require_pdf, root, snapshot};
use s106::document::{self, Config};
use s106::ocrbench::score;

/// The configured pipeline with an empty cache, so every page is really read.
fn fresh() -> (Config, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = Config::from_env(&root());
    cfg.cache_root = dir.path().to_path_buf();
    (cfg, dir)
}

#[test]
#[ignore = "needs the corpus and runtime artifacts: cargo test --release --test pdf_regression -- --ignored"]
fn gold_pages_meet_their_thresholds() {
    let results: Vec<_> = common::gold_sets()
        .into_iter()
        .map(|(path, g, deed)| {
            require_pdf(&deed.pdf);
            let (cfg, _dir) = fresh();
            let d = document::load_pages(&deed.pdf, &cfg, Some(&g.pages()), false).unwrap();
            let s = score(&g, &d);
            eprintln!("{}\n{s}", path.display());
            (path, g.thresholds.failures(&s))
        })
        .collect();
    common::assert_gold(&results);
}

/// pages/difficult.tsv in the data repository: deed, page, and why the page is hard.
#[test]
#[ignore = "needs the corpus and runtime artifacts: cargo test --release --test pdf_regression -- --ignored"]
fn difficult_pages_match_their_snapshots() {
    let list = std::fs::read_to_string(data().join("pages/difficult.tsv")).unwrap();
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
        let snap = snapshot(id);
        let cfg = cfg.clone().with_models_of(&snap.producer).unwrap();
        let t0 = std::time::Instant::now();
        let doc = document::load_pages(&deed.pdf, &cfg, Some(pages), false).unwrap();
        assert_eq!(doc.page_count(), pages.len(), "{id}: pages {pages:?}");
        eprintln!(
            "{id}: {} pages in {:.1}s",
            pages.len(),
            t0.elapsed().as_secs_f64()
        );
        problems.extend(
            snap.compare_pages(&doc)
                .into_iter()
                .map(|d| format!("{id} {d}")),
        );
    }
    assert!(
        problems.is_empty(),
        "fresh page text differs from the snapshots (if intended, bump PAGE_TEXT_LOGIC and \
         regenerate with `s106 pages --write data/pages`):\n{}",
        problems.join("\n")
    );
}
