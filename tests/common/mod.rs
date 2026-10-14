//! Shared by the tiered tests: the corpus list and page-text snapshots from the data repository
//! (`s106::data`) and OCR thresholds.
#![allow(dead_code)]

use std::path::{Path, PathBuf};

use s106::document::Snapshot;
use s106::ocrbench::Gold;

pub fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// The data checkout; the data tiers fail (never skip) without it.
pub fn data() -> PathBuf {
    let root = s106::data::root();
    assert!(
        s106::data::manifest().exists(),
        "data missing: no {} (clone https://github.com/MozarellaMan/s106-data into data/, or set S106_DATA)",
        s106::data::manifest().display()
    );
    root
}

/// One row of the data repository's corpus/manifest.tsv.
pub struct Deed {
    pub id: String,
    pub pdf: PathBuf,
    pub sha256: String,
}

pub fn corpus() -> Vec<Deed> {
    let data = data();
    std::fs::read_to_string(s106::data::manifest())
        .unwrap()
        .lines()
        .filter(|l| !l.starts_with('#') && !l.starts_with("id\t"))
        .map(|l| {
            let f: Vec<&str> = l.split('\t').collect();
            Deed {
                id: f[0].into(),
                pdf: data.join(f[1]),
                sha256: f[2].into(),
            }
        })
        .collect()
}

pub fn snapshot_dir() -> PathBuf {
    data().join("pages")
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

/// Every gold file in the data repository, with the deed it describes (matched by SHA-256).
pub fn gold_sets() -> Vec<(PathBuf, Gold, Deed)> {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(data().join("gold"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .collect();
    paths.sort();
    let deeds = corpus();
    paths
        .into_iter()
        .map(|p| {
            let g = Gold::load(&p).unwrap();
            let deed = deeds
                .iter()
                .find(|d| d.sha256 == g.sha256)
                .unwrap_or_else(|| {
                    panic!("{}: no deed in the manifest has its SHA-256", p.display())
                });
            let deed = Deed {
                id: deed.id.clone(),
                pdf: deed.pdf.clone(),
                sha256: deed.sha256.clone(),
            };
            (p, g, deed)
        })
        .collect()
}

/// Fail with every gold set that misses its thresholds.
pub fn assert_gold(results: &[(PathBuf, Vec<String>)]) {
    assert!(!results.is_empty(), "no gold sets in the data repository");
    let failed: Vec<String> = results
        .iter()
        .filter(|(_, f)| !f.is_empty())
        .map(|(p, f)| format!("{}: {}", p.display(), f.join("; ")))
        .collect();
    assert!(
        failed.is_empty(),
        "OCR below the gold thresholds:\n{}",
        failed.join("\n")
    );
}
