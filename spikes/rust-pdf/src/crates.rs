//! Text-layer crates other than PDFium, run on the same pages into the same output shape:
//! pdfplumber (pdfplumber-rs), pdf-extract and pdf-inspector. pdf-inspector is primarily a
//! "does this PDF need OCR?" classifier, so its classification is printed too.

use std::time::Instant;

use anyhow::Result;

use crate::{Doc, LineOut, PageOut, WordOut, parse_pages, sha256_hex};

pub fn run(pdf: &str, pages: &str, out_prefix: &str) -> Result<()> {
    let sha = sha256_hex(pdf)?;

    // pdfplumber: lines with word boxes (PDF points, top-left origin).
    let doc = pdfplumber::Pdf::open_path(pdf, None)?;
    let selected = parse_pages(pages, doc.page_count())?;
    let mut d = Doc { source: pdf.into(), sha256: sha.clone(), backend: "pdfplumber 0.4.1 (pdfplumber-rs)".into(), ..Default::default() };
    for &p in &selected {
        let t0 = Instant::now();
        let page = doc.page(p - 1)?;
        let lines = page
            .extract_text_lines(&pdfplumber::TextOptions::default())
            .into_iter()
            .map(|l| LineOut {
                text: l.words.iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" "),
                bbox: [l.bbox.x0 as f32, l.bbox.top as f32, l.bbox.x1 as f32, l.bbox.bottom as f32],
                words: l.words.iter().map(|w| WordOut {
                    text: w.text.clone(),
                    bbox: [w.bbox.x0 as f32, w.bbox.top as f32, w.bbox.x1 as f32, w.bbox.bottom as f32],
                }).collect(),
            })
            .collect();
        d.pages.push(PageOut { page: p, width: page.width() as f32, height: page.height() as f32, millis: t0.elapsed().as_millis(), lines });
    }
    std::fs::write(format!("{out_prefix}_pdfplumber.json"), serde_json::to_string_pretty(&d)?)?;

    // pdf-extract: plain text per page, no geometry.
    let t0 = Instant::now();
    let texts = pdf_extract::extract_text_by_pages(pdf)?;
    let ms = t0.elapsed().as_millis() / texts.len().max(1) as u128;
    let mut d = Doc { source: pdf.into(), sha256: sha.clone(), backend: "pdf-extract 0.12.1".into(), ..Default::default() };
    for &p in &selected {
        let lines = texts.get(p - 1).map_or(vec![], |t| {
            t.lines().filter(|l| !l.trim().is_empty()).map(|l| LineOut { text: l.to_string(), ..Default::default() }).collect()
        });
        d.pages.push(PageOut { page: p, millis: ms, lines, ..Default::default() });
    }
    std::fs::write(format!("{out_prefix}_pdf-extract.json"), serde_json::to_string_pretty(&d)?)?;

    // pdf-inspector: classification for the whole file, then markdown one page at a time so the
    // page attribution is ours, not inferred from the markdown.
    let bytes = std::fs::read(pdf)?;
    let class = pdf_inspector::classify_pdf_mem(&bytes)?;
    println!("pdf-inspector: type {:?}, confidence {:.2}, {} of {} pages need OCR",
             class.pdf_type, class.confidence, class.pages_needing_ocr.len(), class.page_count);
    let mut d = Doc { source: pdf.into(), sha256: sha, backend: "pdf-inspector 1.25.2 (markdown)".into(), ..Default::default() };
    for &p in &selected {
        let t0 = Instant::now();
        let opts = pdf_inspector::PdfOptions::new().pages([p as u32]);
        let res = pdf_inspector::process_pdf_with_options(pdf, opts)?;
        let lines = res.markdown.unwrap_or_default().lines().filter(|l| !l.trim().is_empty())
            .map(|l| LineOut { text: l.to_string(), ..Default::default() }).collect();
        d.pages.push(PageOut { page: p, millis: t0.elapsed().as_millis(), lines, ..Default::default() });
    }
    std::fs::write(format!("{out_prefix}_pdf-inspector.json"), serde_json::to_string_pretty(&d)?)?;
    Ok(())
}
