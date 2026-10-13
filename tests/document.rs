//! The page text the evidence matcher sees: OCR boxes joined per visual row, side-margin specks
//! (binding marks read as digits) left out, folios and text-layer lines untouched.

use std::path::PathBuf;

use s106::document::{Document, Line, PageText, TextSource};
use s106::evidence::evidence_found;
use s106::schema::Evidence;

fn line(text: &str, bbox: [f32; 4]) -> Line {
    Line {
        text: text.into(),
        bbox,
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
    // Verbatim boxes from North Tyneside 19/01280 p5 (PP-OCRv5): a binding mark at x=25 read as "1"
    // shares a row with the wrapped amount, which made "twelve 1 thousand" unmatchable.
    let d = doc(
        TextSource::Ocr,
        vec![
            line(
                "Primary Education Contribution: means £212,500.00 (Two hundred and twelve",
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
    let ev = Evidence {
        page: 5,
        snippet: "means: £212,500.00 (Two hundred and twelve thousand five hundred pounds)".into(),
    };
    assert!(evidence_found(Some(&ev), &pages));
}

#[test]
fn same_row_boxes_join_and_rows_stay_separate() {
    let d = doc(
        TextSource::Ocr,
        vec![
            line("\"Affordable Housing", [130.0, 960.0, 400.0, 990.0]),
            line(
                "means the sum of £10,058.60 (ten thousand and fifty eight pounds",
                [470.0, 958.0, 1450.0, 992.0],
            ),
            line("Monitoring Contribution\"", [130.0, 1000.0, 420.0, 1030.0]),
            line(
                "and sixty pence) Index Linked to be paid by the Owner to the",
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
    // the snippet runs across the wrapped right column, with the term's second half in between
    let ev = Evidence { page: 5, snippet: "means the sum of £10,058.60 (ten thousand and fifty eight pounds and sixty pence) Index Linked".into() };
    assert!(evidence_found(Some(&ev), &d.pages()));
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
