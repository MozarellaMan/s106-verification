//! Invented deeds (`tests/synthetic/<id>/`): page text written by hand in `pages.txt` and the
//! extraction a model would return in `extraction.json`. They describe no real planning
//! application, so the fast tiers need no deed, register or recorded model response.
//!
//! `pages.txt` holds one `=== page <n> <text-layer|ocr> ===` header per page, then one line per
//! visual row. ` || ` splits a row into separate boxes, as OCR reads the columns of a two-column
//! definition. Boxes get made-up coordinates: rows 20 units apart, columns side by side.
#![allow(dead_code)]

use std::path::{Path, PathBuf};

use s106::document::{Document, Line, PageText, TextSource};
use s106::extract::{self, Info};
use s106::schema::Agreement;
use s106::verify::{self, CheckResult};
use s106::wire::PageMap;
use serde_json::Value;

pub const IDS: [&str; 3] = ["agreement", "discrepancies", "variation"];

pub fn dir(id: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/synthetic")
        .join(id)
}

pub fn document(id: &str) -> Document {
    let text = std::fs::read_to_string(dir(id).join("pages.txt")).unwrap();
    let mut pages: Vec<PageText> = vec![];
    for line in text.lines() {
        if let Some(header) = line
            .strip_prefix("=== page ")
            .and_then(|h| h.strip_suffix(" ==="))
        {
            let (n, source) = header.split_once(' ').expect("=== page <n> <source> ===");
            pages.push(PageText {
                page: n.parse().unwrap(),
                source: match source {
                    "text-layer" => TextSource::TextLayer,
                    "ocr" => TextSource::Ocr,
                    s => panic!("{id}: unknown text source {s}"),
                },
                width: 600.0,
                height: 800.0,
                lines: vec![],
            });
            continue;
        }
        let page = pages.last_mut().expect("text before the first page header");
        let y = page.lines.last().map_or(20.0, |l| l.bbox[1] + 20.0);
        let boxes: Vec<&str> = line.split(" || ").collect();
        let w = 540.0 / boxes.len() as f32;
        for (i, b) in boxes.iter().enumerate() {
            let x = 30.0 + w * i as f32;
            page.lines.push(Line {
                text: (*b).into(),
                bbox: [x, y, x + w - 10.0, y + 12.0],
            });
        }
    }
    Document {
        path: PathBuf::from(format!("{id}.pdf")),
        sha256: format!("synthetic-{id}"),
        ocr_version: "synthetic".into(),
        pages,
    }
}

pub fn extraction(id: &str) -> Value {
    serde_json::from_str(&std::fs::read_to_string(dir(id).join("extraction.json")).unwrap())
        .unwrap()
}

pub fn agreement(id: &str) -> Agreement {
    agreement_from(id, &extraction(id))
}

/// Convert a (possibly edited) extraction against the deed's pages, as replay would.
pub fn agreement_from(id: &str, output: &Value) -> Agreement {
    let doc = document(id);
    let info = Info {
        model: "synthetic".into(),
        served_by: "hand-written".into(),
        source: "replay".into(),
        usage: None,
        fixture: Some(dir(id).join("extraction.json")),
    };
    extract::to_agreement(&doc, output, &PageMap::identity(doc.page_count()), &info).unwrap()
}

pub fn verify(id: &str) -> (Agreement, Vec<CheckResult>) {
    let a = agreement(id);
    let r = verify::verify(&a, &document(id).pages());
    (a, r)
}
