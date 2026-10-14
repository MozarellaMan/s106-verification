//! Where the research data lives. Everything from a real planning application (the deed
//! manifest, page-text snapshots, gold pages, Datahub snapshots, proposals, labels and results)
//! is in the separate s106-data repository, checked out at `data/` in this one (ignored by git)
//! or wherever `S106_DATA` points. This repository holds code only.

use std::path::PathBuf;

/// `S106_DATA`, else `data/` in this repository.
pub fn root() -> PathBuf {
    std::env::var_os("S106_DATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("data"))
}

/// Every deed: id, path relative to the data root, SHA-256, URL, session page.
pub fn manifest() -> PathBuf {
    root().join("corpus/manifest.tsv")
}

/// Page-text snapshots, `<id>.pages.json`.
pub fn pages() -> PathBuf {
    root().join("pages")
}

/// Deed PDFs (not committed; fetched by `scripts/fetch-corpus.sh`).
pub fn raw() -> PathBuf {
    root().join("raw")
}
