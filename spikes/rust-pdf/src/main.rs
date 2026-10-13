//! M1.5 Rust feasibility spike: PDF rendering, text-layer extraction and OCR with provenance.
//!
//!   pdfspike ocr   <pdf> <pages> <out.json> [dpi]   render with PDFium, OCR with ocrs
//!   pdfspike paddle <pdf> <pages> <out.json> [dpi]  render with PDFium, OCR with oar-ocr using the
//!                                                   same PP-OCRv4 ONNX models as the Python pipeline
//!   pdfspike text  <pdf> <pages> <out.json>         PDFium text layer, per-char boxes
//!   pdfspike crates <pdf> <pages> <out-prefix>      pdfplumber, pdf-extract, pdf-inspector
//!   pdfspike score <gold.json> <pages.json>          compare against hand-transcribed gold
//!
//! <pages> is "all" or a list like "4,5,12-14,89". Output is one JSON shape for every
//! backend (see `PageOut`) so the scorer can compare them, including the Python baseline.
//! Coordinates are page pixels at the render DPI (ocr) or PDF points from the top-left (text).

mod crates;
mod score;

use std::path::Path;
use std::time::Instant;

use anyhow::{Context, Result, bail};
use ocrs::{ImageSource, OcrEngine, OcrEngineParams, TextItem};
use pdfium_render::prelude::*;
use rten::Model;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Default)]
pub struct Doc {
    pub source: String,
    pub sha256: String,
    pub backend: String,
    pub pages: Vec<PageOut>,
}

#[derive(Serialize, Deserialize, Default)]
pub struct PageOut {
    /// 1-based PDF page index (not the number printed on the page)
    pub page: usize,
    pub width: f32,
    pub height: f32,
    pub millis: u128,
    pub lines: Vec<LineOut>,
}

#[derive(Serialize, Deserialize, Default)]
pub struct LineOut {
    pub text: String,
    /// [left, top, right, bottom]
    pub bbox: [f32; 4],
    #[serde(default)]
    pub words: Vec<WordOut>,
}

#[derive(Serialize, Deserialize, Default)]
pub struct WordOut {
    pub text: String,
    pub bbox: [f32; 4],
}

fn artifacts() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("artifacts")
}

fn pdfium() -> Result<Pdfium> {
    let lib = artifacts().join("pdfium/lib");
    let bindings = Pdfium::bind_to_library(Pdfium::pdfium_platform_library_name_at_path(&lib))
        .context("load libpdfium.so (run ./fetch-artifacts.sh)")?;
    Ok(Pdfium::new(bindings))
}

pub fn parse_pages(spec: &str, count: usize) -> Result<Vec<usize>> {
    if spec == "all" {
        return Ok((1..=count).collect());
    }
    let mut out = vec![];
    for part in spec.split(',') {
        match part.split_once('-') {
            Some((a, b)) => out.extend(a.parse::<usize>()?..=b.parse::<usize>()?),
            None => out.push(part.parse()?),
        }
    }
    if let Some(bad) = out.iter().find(|&&p| p == 0 || p > count) {
        bail!("page {bad} out of range 1..={count}");
    }
    Ok(out)
}

pub fn sha256_hex(path: &str) -> Result<String> {
    // Shell out rather than add a hashing crate: the spike only needs provenance, not speed.
    let out = std::process::Command::new("sha256sum").arg(path).output()?;
    Ok(String::from_utf8(out.stdout)?.split_whitespace().next().unwrap_or("").to_string())
}

fn ocr(pdf: &str, pages: &str, out: &str, dpi: f32) -> Result<()> {
    let pdfium = pdfium()?;
    let doc = pdfium.load_pdf_from_file(pdf, None)?;
    let engine = OcrEngine::new(OcrEngineParams {
        detection_model: Some(Model::load_file(artifacts().join("text-detection.rten"))?),
        recognition_model: Some(Model::load_file(artifacts().join("text-recognition.rten"))?),
        ..Default::default()
    })?;
    let mut result = Doc {
        source: pdf.into(),
        sha256: sha256_hex(pdf)?,
        backend: format!("pdfium-render 0.9.4 (PDFium 8100) + ocrs 0.13.1 @ {dpi} dpi"),
        ..Default::default()
    };
    for p in parse_pages(pages, doc.pages().len() as usize)? {
        let t0 = Instant::now();
        let page = doc.pages().get((p - 1) as i32)?;
        // PDFium applies the page's /Rotate when rendering, so the bitmap is upright as displayed.
        let cfg = PdfRenderConfig::new().scale_page_by_factor(dpi / 72.0);
        let img = page.render_with_config(&cfg)?.as_image()?.into_rgb8();
        let (w, h) = img.dimensions();
        let input = engine.prepare_input(ImageSource::from_bytes(img.as_raw(), (w, h))?)?;
        let words = engine.detect_words(&input)?;
        let line_rects = engine.find_text_lines(&input, &words);
        let mut lines = vec![];
        for line in engine.recognize_text(&input, &line_rects)?.into_iter().flatten() {
            let text = line.to_string();
            let t = text.trim();
            if t.chars().count() < 2 && !t.chars().all(|c| c.is_ascii_digit()) {
                continue; // single non-digit chars are mostly specks and binding marks
            }
            let r = line.bounding_rect();
            lines.push(LineOut {
                text,
                bbox: [r.left() as f32, r.top() as f32, r.right() as f32, r.bottom() as f32],
                words: line
                    .words()
                    .map(|wd| {
                        let r = wd.bounding_rect();
                        WordOut {
                            text: wd.to_string(),
                            bbox: [r.left() as f32, r.top() as f32, r.right() as f32, r.bottom() as f32],
                        }
                    })
                    .collect(),
            });
        }
        eprintln!("page {p}: {} lines, {} ms", lines.len(), t0.elapsed().as_millis());
        result.pages.push(PageOut { page: p, width: w as f32, height: h as f32, millis: t0.elapsed().as_millis(), lines });
    }
    std::fs::write(out, serde_json::to_string_pretty(&result)?)?;
    Ok(())
}

/// PP-OCRv4 (the model files RapidOCR ships, so this isolates pipeline from model differences)
/// through oar-ocr / ONNX Runtime. Each detected region is kept as a line with its box.
fn paddle(pdf: &str, pages: &str, out: &str, dpi: f32) -> Result<()> {
    use oar_ocr::prelude::*;
    let pdfium = pdfium()?;
    let doc = pdfium.load_pdf_from_file(pdf, None)?;
    let m = artifacts().join("ppocrv4");
    let engine = OAROCRBuilder::new(m.join("ch_PP-OCRv4_det_infer.onnx"), m.join("ch_PP-OCRv4_rec_infer.onnx"),
                                    m.join("dict.txt"))
        .return_word_box(true)
        .build()?;
    let mut result = Doc {
        source: pdf.into(),
        sha256: sha256_hex(pdf)?,
        backend: format!("pdfium-render 0.9.4 (PDFium 8100) + oar-ocr 0.10.0 (ONNX Runtime) PP-OCRv4 @ {dpi} dpi"),
        ..Default::default()
    };
    for p in parse_pages(pages, doc.pages().len() as usize)? {
        let t0 = Instant::now();
        let page = doc.pages().get((p - 1) as i32)?;
        let img = page.render_with_config(&PdfRenderConfig::new().scale_page_by_factor(dpi / 72.0))?.as_image()?.into_rgb8();
        let (w, h) = img.dimensions();
        let res = engine.predict(vec![img])?;
        let mut lines: Vec<LineOut> = res[0]
            .text_regions
            .iter()
            .filter_map(|r| {
                let text = r.text.as_deref()?.trim().to_string();
                if text.chars().count() < 2 && !text.chars().all(|c| c.is_ascii_digit()) {
                    return None; // keep single digits: page numbers and table cells
                }
                let b = &r.bounding_box;
                let bbox = [b.x_min(), b.y_min(), b.x_max(), b.y_max()];
                // Word boxes come back without per-word text; keep them only as geometry.
                let words = r.word_boxes.as_ref().map_or(vec![], |wb| {
                    wb.iter().map(|w| WordOut { text: String::new(), bbox: [w.x_min(), w.y_min(), w.x_max(), w.y_max()] }).collect()
                });
                Some(LineOut { text, bbox, words })
            })
            .collect();
        lines.sort_by(|a, b| (a.bbox[1] / 8.0).floor().total_cmp(&(b.bbox[1] / 8.0).floor()).then(a.bbox[0].total_cmp(&b.bbox[0])));
        eprintln!("page {p}: {} regions, {} ms", lines.len(), t0.elapsed().as_millis());
        result.pages.push(PageOut { page: p, width: w as f32, height: h as f32, millis: t0.elapsed().as_millis(), lines });
    }
    std::fs::write(out, serde_json::to_string_pretty(&result)?)?;
    Ok(())
}

/// PDFium text layer: characters with tight bounds, grouped into words and lines by geometry.
fn text(pdf: &str, pages: &str, out: &str) -> Result<()> {
    let pdfium = pdfium()?;
    let doc = pdfium.load_pdf_from_file(pdf, None)?;
    let mut result = Doc {
        source: pdf.into(),
        sha256: sha256_hex(pdf)?,
        backend: "pdfium-render 0.9.4 (PDFium 8100) text layer".into(),
        ..Default::default()
    };
    for p in parse_pages(pages, doc.pages().len() as usize)? {
        let t0 = Instant::now();
        let page = doc.pages().get((p - 1) as i32)?;
        let (pw, ph) = (page.width().value, page.height().value);
        let text = page.text()?;
        // (char, [l, t, r, b]) in top-left origin points
        let mut chars: Vec<(char, [f32; 4])> = vec![];
        for c in text.chars().iter() {
            // Loose bounds = the glyph's full font box: consistent heights, so punctuation (whose
            // tight box is tiny) doesn't look like a line break or a word gap.
            let (Some(ch), Ok(b)) = (c.unicode_char(), c.loose_bounds()) else { continue };
            chars.push((ch, [b.left().value, ph - b.top().value, b.right().value, ph - b.bottom().value]));
        }
        let lines = group_lines(&chars);
        eprintln!("page {p}: {} chars, {} lines, {} ms", chars.len(), lines.len(), t0.elapsed().as_millis());
        result.pages.push(PageOut { page: p, width: pw, height: ph, millis: t0.elapsed().as_millis(), lines });
    }
    std::fs::write(out, serde_json::to_string_pretty(&result)?)?;
    Ok(())
}

/// Characters in content-stream order -> lines (break on a vertical jump or a return to the left)
/// -> words (break on whitespace or a horizontal gap wider than ~0.3 em).
fn group_lines(chars: &[(char, [f32; 4])]) -> Vec<LineOut> {
    let mut lines: Vec<LineOut> = vec![];
    let mut cur: Vec<(char, [f32; 4])> = vec![];
    let flush = |cur: &mut Vec<(char, [f32; 4])>, lines: &mut Vec<LineOut>| {
        if cur.iter().all(|(c, _)| c.is_whitespace()) {
            cur.clear();
            return;
        }
        let mut words: Vec<WordOut> = vec![];
        let mut prev_right = f32::NEG_INFINITY;
        for (c, b) in cur.iter() {
            let em = (b[3] - b[1]).abs().max(1.0);
            if c.is_whitespace() {
                prev_right = f32::NEG_INFINITY;
                continue;
            }
            let new_word = words.is_empty() || prev_right == f32::NEG_INFINITY || b[0] - prev_right > 0.3 * em;
            if new_word {
                words.push(WordOut { text: String::new(), bbox: *b });
            }
            let w = words.last_mut().unwrap();
            w.text.push(*c);
            w.bbox = [w.bbox[0].min(b[0]), w.bbox[1].min(b[1]), w.bbox[2].max(b[2]), w.bbox[3].max(b[3])];
            prev_right = b[2];
        }
        let bbox = words.iter().fold([f32::MAX, f32::MAX, f32::MIN, f32::MIN], |a, w| {
            [a[0].min(w.bbox[0]), a[1].min(w.bbox[1]), a[2].max(w.bbox[2]), a[3].max(w.bbox[3])]
        });
        let text = words.iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" ");
        lines.push(LineOut { text, bbox, words });
        cur.clear();
    };
    for &(c, b) in chars {
        if c == '\r' || c == '\n' {
            flush(&mut cur, &mut lines);
            continue;
        }
        if let Some(&(_, last)) = cur.iter().rev().find(|(ch, _)| !ch.is_whitespace()) {
            let line_h = (last[3] - last[1]).abs().max(1.0);
            let new_line = (b[1] - last[1]).abs() > 0.6 * line_h || b[0] < last[0] - 2.0 * line_h;
            if new_line && !c.is_whitespace() {
                flush(&mut cur, &mut lines);
            }
        }
        cur.push((c, b));
    }
    flush(&mut cur, &mut lines);
    lines
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("ocr") if args.len() >= 5 => {
            let dpi = args.get(5).map(|d| d.parse()).transpose()?.unwrap_or(200.0);
            ocr(&args[2], &args[3], &args[4], dpi)
        }
        Some("paddle") if args.len() >= 5 => {
            let dpi = args.get(5).map(|d| d.parse()).transpose()?.unwrap_or(200.0);
            paddle(&args[2], &args[3], &args[4], dpi)
        }
        Some("text") if args.len() == 5 => text(&args[2], &args[3], &args[4]),
        Some("crates") if args.len() == 5 => crates::run(&args[2], &args[3], &args[4]),
        Some("score") if args.len() == 4 => score::run(&args[2], &args[3]),
        _ => bail!("usage: pdfspike ocr <pdf> <pages> <out.json> [dpi] | text <pdf> <pages> <out.json> | score <gold.json> <pages.json>"),
    }
}
