//! Page text against hand-transcribed gold (gold/*.json), with the default OCR models.
//! Thresholds sit just below today's scores, so a regression fails while noise on a 6-page gold
//! set does not. Needs the deeds and models locally:
//!     cargo test --release --test ocr_gold -- --ignored

use std::path::PathBuf;

use s106::document::{self, Config};
use s106::ocrbench::{Gold, score};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn run(gold: &str, pdf: &str) -> s106::ocrbench::Score {
    let pdf = root().join(pdf);
    assert!(pdf.exists(), "corpus missing: {} (see README)", pdf.display());
    let g = Gold::load(&root().join(gold)).unwrap();
    assert_eq!(document::sha256_file(&pdf).unwrap(), g.sha256, "{} is not the document the gold set describes", pdf.display());
    let d = document::load_pages(&pdf, &Config::from_env(&root()), Some(&g.pages()), false).unwrap();
    let s = score(&g, &d);
    eprintln!("{gold}\n{s}");
    s
}

#[test]
#[ignore = "needs the local deeds and OCR models; run with --ignored"]
fn southwark_scan() {
    let s = run("gold/southwark_19_AP_1773.json", "data/raw/southwark_s106_sample.pdf");
    assert!(s.mean_cer_squashed <= 0.01, "line CER {}", s.mean_cer_squashed);
    assert!(s.facts_raw >= 8, "facts {}/{} (no £ repair needed with PP-OCRv5)", s.facts_raw, s.facts);
    assert!(s.printed_found >= 5, "printed page numbers {}/{}", s.printed_found, s.printed);
    assert_eq!(s.definitions_on_own_line, s.definitions, "two-column definitions merged with their terms");
}

#[test]
#[ignore = "needs the local deeds and OCR models; run with --ignored"]
fn salford_text_layer() {
    let s = run("gold/salford_20_75239.json", "data/raw/eval/salford-75239.pdf");
    assert_eq!(s.exact_lines, s.lines);
    assert_eq!(s.facts_raw, s.facts);
    assert_eq!(s.printed_found, s.printed);
}
