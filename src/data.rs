//! Where the research data lives. Everything from a real planning application (the deed
//! manifest, recorded model responses, page-text snapshots, register rows, gold pages, Datahub
//! snapshots) is in the separate s106-data repository, checked out at `data/` in this one
//! (ignored by git) or wherever `S106_DATA` points. This repository holds code and invented
//! test deeds only.

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

/// Recorded model responses, `<id>/<model>.extract.v2.json`.
pub fn extractions() -> PathBuf {
    root().join("extractions")
}

/// Responses recorded by the former Python harness, `<id>/<model>.extract.json`.
pub fn extractions_v1() -> PathBuf {
    root().join("extractions-v1")
}

/// Page-text snapshots, `<id>.pages.json`.
pub fn pages() -> PathBuf {
    root().join("pages")
}

/// Pinned outcomes for the data tier.
pub fn expected() -> PathBuf {
    root().join("expected")
}

/// Deed PDFs (not committed; fetched by `scripts/fetch-corpus.sh`).
pub fn raw() -> PathBuf {
    root().join("raw")
}

/// Derived outputs (not committed).
pub fn processed() -> PathBuf {
    root().join("processed")
}
