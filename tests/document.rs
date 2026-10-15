//! The page text a reader sees: OCR boxes joined per visual row, side-margin specks
//! (binding marks read as digits) left out, folios and text-layer lines untouched.

use std::path::PathBuf;

use s106::document::{Document, Line, PageText, TextSource};

fn line(text: &str, bbox: [f32; 4]) -> Line {
    Line {
        text: text.into(),
        bbox,
        conf: None,
    }
}

fn doc(source: TextSource, lines: Vec<Line>) -> Document {
    Document {
        path: PathBuf::from("x.pdf"),
        sha256: "0".repeat(64),
        ocr_version: "test".into(),
        pages: vec![PageText {
            page: 5,
            source,
            width: 1654.0,
            height: 2339.0,
            lines,
        }],
    }
}

#[test]
fn margin_speck_is_not_joined_into_a_sentence() {
    // Invented boxes modelled on PP-OCRv5 output: a binding mark at x=25 read as "1"
    // shares a row with the wrapped amount, which made "four 1 thousand" unmatchable.
    let d = doc(
        TextSource::Ocr,
        vec![
            line(
                "Primary Education Contribution: means £84,500.00 (Eighty four",
                [277.0, 1366.0, 1453.0, 1442.0],
            ),
            line("1", [25.0, 1456.0, 66.0, 1516.0]),
            line(
                "thousand five hundred pounds) towards increase capacity in new or existing provision",
                [278.0, 1418.0, 1457.0, 1497.0],
            ),
            line("5", [800.0, 2240.0, 830.0, 2280.0]),
        ],
    );
    let pages = d.pages();
    let text = &pages[&5];
    assert!(
        !text.contains("\n1 ") && !text.starts_with("1 "),
        "speck kept: {text:?}"
    );
    assert!(
        text.ends_with('5'),
        "the folio in the bottom margin is not a speck: {text:?}"
    );
}

#[test]
fn same_row_boxes_join_and_rows_stay_separate() {
    let d = doc(
        TextSource::Ocr,
        vec![
            line("\"Affordable Housing", [130.0, 960.0, 400.0, 990.0]),
            line(
                "means the sum of £6,042.30 (six thousand and forty two pounds",
                [470.0, 958.0, 1450.0, 992.0],
            ),
            line("Monitoring Contribution\"", [130.0, 1000.0, 420.0, 1030.0]),
            line(
                "and thirty pence) Index Linked to be paid by the Owner to the",
                [470.0, 998.0, 1450.0, 1032.0],
            ),
        ],
    );
    let text = d.pages()[&5].clone();
    assert_eq!(text.lines().count(), 2);
    assert!(
        text.lines()
            .next()
            .unwrap()
            .starts_with("\"Affordable Housing means the sum")
    );
}

#[test]
fn text_layer_lines_are_used_as_they_are() {
    let d = doc(
        TextSource::TextLayer,
        vec![
            line("1", [10.0, 10.0, 12.0, 12.0]),
            line("Plan the plan attached", [70.0, 10.0, 400.0, 22.0]),
        ],
    );
    assert_eq!(d.pages()[&5], "1\nPlan the plan attached");
    assert_eq!(d.text_source(), "text-layer");
}

// ------------------------------------------------------------------------------- snapshots

fn snapshot_of(d: &Document) -> s106::document::Snapshot {
    let producer = s106::document::Config::from_env(std::path::Path::new(".")).producer();
    s106::document::Snapshot {
        format: s106::document::SNAPSHOT_FORMAT.into(),
        source_file: "x.pdf".into(),
        sha256: d.sha256.clone(),
        page_count: 1,
        ocr_version: d.ocr_version.clone(),
        producer,
        pages: d.pages.clone(),
    }
}

fn speck_page() -> Document {
    // the binding-mark speck at x=25 is dropped from matching text only while it lies in the margin
    let mut d = doc(
        TextSource::Ocr,
        vec![
            line("twelve", [277.0, 1366.0, 400.0, 1442.0]),
            line("1", [25.0, 1366.0, 66.0, 1442.0]),
        ],
    );
    d.pages[0].page = 1;
    d
}

#[test]
fn a_whole_document_must_have_every_page() {
    let d = speck_page();
    let snap = snapshot_of(&d);
    assert!(snap.compare_document(&d).is_empty());
    let empty = Document {
        pages: vec![],
        ..d.clone()
    };
    assert!(
        snap.compare_pages(&empty).is_empty(),
        "a selection of no pages has nothing to differ"
    );
    let diffs = snap.compare_document(&empty);
    assert!(diffs[0].contains("snapshot has 1..=1"), "{diffs:?}");
}

#[test]
fn a_changed_page_size_is_a_difference_because_it_changes_the_evidence_text() {
    let d = speck_page();
    let snap = snapshot_of(&d);
    let mut narrow = d.clone();
    narrow.pages[0].width = 300.0; // the speck is no longer within 6% of the left edge
    assert_ne!(
        d.pages()[&1],
        narrow.pages()[&1],
        "width changes the matching text"
    );
    let diffs = snap.compare_document(&narrow);
    assert!(diffs.len() == 1 && diffs[0].contains("size"), "{diffs:?}");
}

#[test]
fn a_moved_box_is_a_difference() {
    let d = speck_page();
    let snap = snapshot_of(&d);
    let mut moved = d.clone();
    moved.pages[0].lines[0].bbox[0] += 0.5;
    assert_eq!(snap.compare_document(&moved).len(), 1);
}
