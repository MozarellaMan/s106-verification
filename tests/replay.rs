//! Data tier: offline checks of the real corpus's page-text snapshots in the data repository
//! (`s106::data`), so these need no PDF, PDFium, ONNX Runtime, OCR models, credentials or
//! network, only the data checkout. They are ignored by the default `cargo test`; run them with
//! `cargo test-data`.
//!
//! The snapshots are only as good as their agreement with the real pipeline; the real-PDF tiers
//! check that (`tests/pdf_regression.rs` on difficult pages, `tests/corpus.rs` on whole deeds).

mod common;

use common::{corpus, root, snapshot};
use s106::document::Config;
use s106::ocrbench;

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
    }
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
