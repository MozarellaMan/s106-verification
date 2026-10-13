//! Shared by the tiered tests: the corpus list, committed page-text snapshots and thresholds.
#![allow(dead_code)]

use std::path::{Path, PathBuf};

use s106::document::Snapshot;
use s106::ocrbench::Score;

pub fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// One row of fixtures/corpus.tsv.
pub struct Deed {
    pub id: String,
    pub pdf: PathBuf,
    pub sha256: String,
}

pub fn corpus() -> Vec<Deed> {
    std::fs::read_to_string(root().join("fixtures/corpus.tsv"))
        .unwrap()
        .lines()
        .filter(|l| !l.starts_with('#') && !l.starts_with("id\t"))
        .map(|l| {
            let f: Vec<&str> = l.split('\t').collect();
            Deed {
                id: f[0].into(),
                pdf: root().join(f[1]),
                sha256: f[2].into(),
            }
        })
        .collect()
}

pub fn snapshot_dir() -> PathBuf {
    root().join("fixtures/pages")
}

pub fn snapshot(id: &str) -> Snapshot {
    Snapshot::read(&Snapshot::path_for(&snapshot_dir(), &format!("{id}.pdf"))).unwrap()
}

/// Fail (never skip) when a real-PDF tier runs without the corpus.
pub fn require_pdf(path: &Path) {
    assert!(
        path.exists(),
        "corpus missing: {} (run scripts/fetch-corpus.sh)",
        path.display()
    );
}

/// OCR quality on the Southwark gold pages. Thresholds sit just below today's scores, so a
/// regression fails while noise on a 6-page gold set does not.
pub fn assert_southwark_gold(s: &Score) {
    assert!(
        s.mean_cer_squashed <= 0.01,
        "line CER {}",
        s.mean_cer_squashed
    );
    assert!(
        s.facts_raw >= 8,
        "facts {}/{} (no £ repair needed with PP-OCRv5)",
        s.facts_raw,
        s.facts
    );
    assert!(
        s.printed_found >= 5,
        "printed page numbers {}/{}",
        s.printed_found,
        s.printed
    );
    assert_eq!(
        s.definitions_on_own_line, s.definitions,
        "two-column definitions merged with their terms"
    );
}

/// The Salford text layer must read exactly.
pub fn assert_salford_gold(s: &Score) {
    assert_eq!(s.exact_lines, s.lines);
    assert_eq!(s.facts_raw, s.facts);
    assert_eq!(s.printed_found, s.printed);
}

/// `S106_BLESS=1` rewrites expected-output files instead of comparing against them.
pub fn bless() -> bool {
    std::env::var_os("S106_BLESS").is_some()
}
