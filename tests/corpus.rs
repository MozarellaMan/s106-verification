//! Tier 4: full documents. Every deed in the data repository's corpus/manifest.tsv is read through the real pipeline
//! (PDFium and OCR, via the page-text cache) and must reproduce its committed snapshot exactly:
//! every page, with the same source, dimensions, lines and boxes. Since everything downstream of page text is deterministic,
//! this is what licenses the replay tiers to stand in for the PDFs.
//!
//! With an empty cache this OCRs every image-only page (the longest scan dominates);
//! with a warm cache it re-checks the cache against the snapshots in seconds. For a cold run:
//!     S106_OCR_CACHE=$(mktemp -d) cargo test --release --test corpus -- --ignored --nocapture
//! The register evaluation on the PDFs is a command, not a test: `s106 eval --replay`.

mod common;

use common::{corpus, require_pdf, root, snapshot};
use s106::document::{self, Config};

#[test]
#[ignore = "needs the corpus and runtime artifacts; minutes on a cold cache: cargo test --release --test corpus -- --ignored"]
fn every_snapshot_reproduces_from_its_pdf() {
    let mut problems = vec![];
    for deed in corpus() {
        require_pdf(&deed.pdf);
        let snap = snapshot(&deed.id);
        // each snapshot is reproduced with the model set it records
        let cfg = Config::from_env(&root())
            .with_models_of(&snap.producer)
            .unwrap();
        let t0 = std::time::Instant::now();
        let doc = document::load(&deed.pdf, &cfg, false).unwrap();
        eprintln!(
            "{}: {} pages ({}) in {:.1}s",
            deed.id,
            doc.page_count(),
            doc.text_source(),
            t0.elapsed().as_secs_f64()
        );
        problems.extend(
            snap.compare_document(&doc)
                .into_iter()
                .map(|d| format!("{} {d}", deed.id)),
        );
    }
    assert!(
        problems.is_empty(),
        "page text differs from the snapshots:\n{}",
        problems.join("\n")
    );
}
