//! Page text with provenance: the PDF's text layer where it has one, OCR where it doesn't.
//!
//! Every line keeps its bounding box, so evidence can later be tied to a position on the page.
//! Results are cached under `<cache root>/<sha256[:16]>-<OCR_VERSION>/` as one JSON file per page
//! plus a `manifest.json`. The key is the PDF's content hash and the OCR settings, so a different
//! document saved under the same name gets a different cache. Changing local model files under
//! the same model name requires explicit invalidation; artifact bytes are not part of the key.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use pdfium_render::prelude::*;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::evidence::Pages;

/// A page needs OCR when its text layer has fewer non-whitespace characters than this.
const MIN_TEXT_CHARS: usize = 50;
const OCR_DPI: f32 = 200.0;

/// The OCR models, chosen by score against the hand-verified gold set (see README).
#[derive(Debug, Clone)]
pub struct OcrModels {
    pub det: PathBuf,
    pub rec: PathBuf,
    pub dict: PathBuf,
    /// Short stable name, part of the cache key.
    pub name: String,
}

impl OcrModels {
    pub fn default_in(models_dir: &Path) -> OcrModels {
        Self::named("ppocrv5-en-mobile", models_dir).expect("default model set")
    }

    /// The model sets fetched by scripts/fetch-models.sh.
    pub fn named(name: &str, models_dir: &Path) -> Option<OcrModels> {
        let (det, rec, dict) = match name {
            "ppocrv4-mobile" => (
                "pp-ocrv4_mobile_det.onnx",
                "pp-ocrv4_mobile_rec.onnx",
                "ppocr_keys_v1.txt",
            ),
            "ppocrv5-en-mobile" => (
                "pp-ocrv5_mobile_det.onnx",
                "en_pp-ocrv5_mobile_rec.onnx",
                "ppocrv5_en_dict.txt",
            ),
            "ppocrv5-latin-mobile" => (
                "pp-ocrv5_mobile_det.onnx",
                "latin_pp-ocrv5_mobile_rec.onnx",
                "ppocrv5_latin_dict.txt",
            ),
            "ppocrv6-small" => (
                "pp-ocrv6_small_det.onnx",
                "pp-ocrv6_small_rec.onnx",
                "ppocrv6_dict.txt",
            ),
            _ => return None,
        };
        Some(OcrModels {
            det: models_dir.join(det),
            rec: models_dir.join(rec),
            dict: models_dir.join(dict),
            name: name.into(),
        })
    }
}

#[derive(Debug, Clone)]
pub struct Config {
    pub models_dir: PathBuf,
    pub cache_root: PathBuf,
    pub ocr: OcrModels,
}

impl Config {
    /// Defaults relative to the repository: `models/` and `data/interim/ocr/`.
    /// `S106_MODELS` and `S106_OCR_CACHE` override them.
    pub fn from_env(repo_root: &Path) -> Config {
        let models_dir = std::env::var_os("S106_MODELS")
            .map(PathBuf::from)
            .unwrap_or(repo_root.join("models"));
        let cache_root = std::env::var_os("S106_OCR_CACHE")
            .map(PathBuf::from)
            .unwrap_or(repo_root.join("data/interim/ocr"));
        let ocr = OcrModels::default_in(&models_dir);
        Config {
            models_dir,
            cache_root,
            ocr,
        }
    }

    /// Everything that changes page text. Bump the leading version when the line logic changes.
    pub fn ocr_version(&self) -> String {
        format!("v2-pdfium8100-{}-{}dpi", self.ocr.name, OCR_DPI as u32)
    }

    /// PDFium can be bound once per process, so the binding is shared. It is also not
    /// thread-safe: every use goes through this lock.
    fn pdfium(&self) -> Result<std::sync::MutexGuard<'static, Pdfium>> {
        static PDFIUM: std::sync::OnceLock<std::sync::Mutex<Pdfium>> = std::sync::OnceLock::new();
        if PDFIUM.get().is_none() {
            let lib = self.models_dir.join("pdfium/lib");
            let bindings =
                Pdfium::bind_to_library(Pdfium::pdfium_platform_library_name_at_path(&lib))
                    .with_context(|| {
                        format!(
                            "load libpdfium from {} (run scripts/fetch-models.sh)",
                            lib.display()
                        )
                    })?;
            let _ = PDFIUM.set(std::sync::Mutex::new(Pdfium::new(bindings)));
        }
        Ok(PDFIUM
            .get()
            .unwrap()
            .lock()
            .unwrap_or_else(|e| e.into_inner()))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TextSource {
    TextLayer,
    Ocr,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Line {
    pub text: String,
    /// [left, top, right, bottom]: PDF points from the top-left (text layer) or pixels at OCR_DPI (OCR).
    pub bbox: [f32; 4],
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageText {
    /// 1-based PDF page index.
    pub page: usize,
    pub source: TextSource,
    pub width: f32,
    pub height: f32,
    pub lines: Vec<Line>,
}

#[derive(Debug, Clone)]
pub struct Document {
    pub path: PathBuf,
    pub sha256: String,
    pub ocr_version: String,
    pub pages: Vec<PageText>,
}

impl Document {
    pub fn page_count(&self) -> usize {
        self.pages.len()
    }

    /// Page text for evidence matching: one text line per visual row, rows joined by newlines.
    ///
    /// OCR keeps each column's text as its own box (and `lines` keeps them apart for provenance),
    /// but for matching, boxes on the same row are joined with a space. A definition that wraps
    /// then reads "...and one" / "Officer Contribution\" hundred and twenty pounds)", which the
    /// evidence matcher tolerates (a short left-column fragment at a line start), rather than
    /// three separate lines, which it rightly does not.
    pub fn pages(&self) -> Pages {
        self.pages.iter().map(|p| (p.page, page_text(p))).collect()
    }

    /// "text-layer", "ocr" or "mixed".
    pub fn text_source(&self) -> &'static str {
        let ocr = self
            .pages
            .iter()
            .filter(|p| p.source == TextSource::Ocr)
            .count();
        match ocr {
            0 => "text-layer",
            n if n == self.pages.len() => "ocr",
            _ => "mixed",
        }
    }
}

fn page_text(p: &PageText) -> String {
    if p.source == TextSource::TextLayer {
        return p
            .lines
            .iter()
            .map(|l| l.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
    }
    // Binding marks and punch holes in the side margins OCR as stray digits ("1"); joined into a row
    // they would put a number inside a sentence, which the evidence matcher (rightly) never skips.
    // They stay in `lines` for provenance; they're left out of the matching text.
    let margin_speck = |l: &Line| {
        l.text.chars().count() <= 2 && (l.bbox[2] < 0.06 * p.width || l.bbox[0] > 0.94 * p.width)
    };
    // `lines` are already in row order (see ocr_lines); start a new row when the vertical centre moves
    let mut rows: Vec<Vec<&Line>> = vec![];
    for l in p.lines.iter().filter(|l| !margin_speck(l)) {
        let c = (l.bbox[1] + l.bbox[3]) / 2.0;
        let h = l.bbox[3] - l.bbox[1];
        let same = rows.last().is_some_and(|r| {
            r.iter().any(|o| {
                ((o.bbox[1] + o.bbox[3]) / 2.0 - c).abs() < 0.5 * h.min(o.bbox[3] - o.bbox[1])
            })
        });
        if same {
            rows.last_mut().unwrap().push(l)
        } else {
            rows.push(vec![l])
        }
    }
    rows.iter()
        .map(|r| {
            r.iter()
                .map(|l| l.text.as_str())
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn sha256_file(path: &Path) -> Result<String> {
    let bytes = std::fs::read(path).with_context(|| format!("read {}", path.display()))?;
    Ok(Sha256::digest(&bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

#[derive(Serialize, Deserialize)]
struct Manifest {
    sha256: String,
    source_path: String,
    ocr_version: String,
    created: String,
}

/// Load (from cache, text layer or OCR) every page of `pdf`.
pub fn load(pdf: &Path, cfg: &Config, verbose: bool) -> Result<Document> {
    load_pages(pdf, cfg, None, verbose)
}

/// As `load`, restricted to `only` (1-based page indices) when given.
pub fn load_pages(
    pdf: &Path,
    cfg: &Config,
    only: Option<&[usize]>,
    verbose: bool,
) -> Result<Document> {
    let sha256 = sha256_file(pdf)?;
    let ocr_version = cfg.ocr_version();
    let dir = cfg
        .cache_root
        .join(format!("{}-{}", &sha256[..16], ocr_version));
    std::fs::create_dir_all(&dir)?;
    let manifest = dir.join("manifest.json");
    if !manifest.exists() {
        let m = Manifest {
            sha256: sha256.clone(),
            source_path: pdf
                .canonicalize()
                .unwrap_or(pdf.to_path_buf())
                .display()
                .to_string(),
            ocr_version: ocr_version.clone(),
            created: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        };
        std::fs::write(&manifest, serde_json::to_string_pretty(&m)? + "\n")?;
    }

    let pdfium = cfg.pdfium()?;
    let doc = pdfium
        .load_pdf_from_file(pdf, None)
        .with_context(|| format!("open {}", pdf.display()))?;
    let count = doc.pages().len() as usize;
    let mut pages: Vec<Option<PageText>> = vec![None; count];
    let mut need_ocr = vec![];
    let wanted: Vec<usize> = only.map_or((1..=count).collect(), |o| {
        o.iter()
            .copied()
            .filter(|&n| n >= 1 && n <= count)
            .collect()
    });
    for &n in &wanted {
        let cached = dir.join(format!("page_{n:03}.json"));
        if let Ok(s) = std::fs::read_to_string(&cached) {
            pages[n - 1] = Some(
                serde_json::from_str(&s).with_context(|| format!("parse {}", cached.display()))?,
            );
            continue;
        }
        let page = doc.pages().get((n - 1) as i32)?;
        let text = text_layer(&page)?;
        if text
            .lines
            .iter()
            .map(|l| l.text.chars().filter(|c| !c.is_whitespace()).count())
            .sum::<usize>()
            >= MIN_TEXT_CHARS
        {
            let text = PageText { page: n, ..text };
            std::fs::write(&cached, serde_json::to_string(&text)?)?;
            pages[n - 1] = Some(text);
        } else {
            need_ocr.push(n);
        }
    }

    if !need_ocr.is_empty() {
        if verbose {
            eprintln!(
                "  OCR: {} image-only pages ({})",
                need_ocr.len(),
                cfg.ocr.name
            );
        }
        let engine = ocr_engine(&cfg.ocr)?;
        // PDFium isn't thread-safe, so pages are rendered here, one at a time, and recognised in
        // small batches (oar-ocr parallelises within a batch).
        for chunk in need_ocr.chunks(6) {
            let mut images = vec![];
            for &n in chunk {
                let page = doc.pages().get((n - 1) as i32)?;
                // PDFium applies the page's /Rotate, so the bitmap is upright as displayed.
                let cfg_r = PdfRenderConfig::new().scale_page_by_factor(OCR_DPI / 72.0);
                images.push(page.render_with_config(&cfg_r)?.as_image()?.into_rgb8());
            }
            let sizes: Vec<(u32, u32)> = images.iter().map(|i| i.dimensions()).collect();
            let results = engine.predict(images)?;
            for ((&n, res), (w, h)) in chunk.iter().zip(results).zip(sizes) {
                let text = PageText {
                    page: n,
                    source: TextSource::Ocr,
                    width: w as f32,
                    height: h as f32,
                    lines: ocr_lines(&res),
                };
                std::fs::write(
                    dir.join(format!("page_{n:03}.json")),
                    serde_json::to_string(&text)?,
                )?;
                if verbose {
                    eprintln!("  page {n}/{count}: {} lines", text.lines.len());
                }
                pages[n - 1] = Some(text);
            }
        }
    }
    let pages: Vec<PageText> = pages.into_iter().flatten().collect();
    if pages.len() != wanted.len() {
        bail!(
            "{} of {} requested pages missing after load",
            wanted.len() - pages.len(),
            wanted.len()
        );
    }
    Ok(Document {
        path: pdf.to_path_buf(),
        sha256,
        ocr_version,
        pages,
    })
}

fn ocr_engine(m: &OcrModels) -> Result<oar_ocr::prelude::OAROCR> {
    for p in [&m.det, &m.rec, &m.dict] {
        if !p.exists() {
            bail!(
                "OCR model file missing: {} (run scripts/fetch-models.sh)",
                p.display()
            );
        }
    }
    Ok(
        oar_ocr::prelude::OAROCRBuilder::new(m.det.clone(), m.rec.clone(), m.dict.clone())
            .build()?,
    )
}

/// OCR regions -> lines in reading order. Regions whose vertical centres align form a row; rows
/// go top to bottom and regions within a row left to right. Each region stays its own line, so
/// a two-column definition comes out as alternating left/right lines, never merged into one.
fn ocr_lines(res: &oar_ocr::prelude::OAROCRResult) -> Vec<Line> {
    let mut lines: Vec<Line> = res
        .text_regions
        .iter()
        .filter_map(|r| {
            let text = r.text.as_deref()?.trim().to_string();
            // single characters are mostly specks and binding marks; keep digits (folios, table cells)
            if text.is_empty()
                || (text.chars().count() < 2 && !text.chars().all(|c| c.is_ascii_digit()))
            {
                return None;
            }
            let b = &r.bounding_box;
            Some(Line {
                text,
                bbox: [b.x_min(), b.y_min(), b.x_max(), b.y_max()],
            })
        })
        .collect();
    lines.sort_by(|a, b| (a.bbox[1] + a.bbox[3]).total_cmp(&(b.bbox[1] + b.bbox[3])));
    let mut rows: Vec<Vec<Line>> = vec![];
    for l in lines {
        let c = (l.bbox[1] + l.bbox[3]) / 2.0;
        let h = l.bbox[3] - l.bbox[1];
        match rows.last_mut() {
            Some(row)
                if row.iter().any(|o| {
                    ((o.bbox[1] + o.bbox[3]) / 2.0 - c).abs() < 0.5 * h.min(o.bbox[3] - o.bbox[1])
                }) =>
            {
                row.push(l)
            }
            _ => rows.push(vec![l]),
        }
    }
    rows.into_iter()
        .flat_map(|mut row| {
            row.sort_by(|a, b| a.bbox[0].total_cmp(&b.bbox[0]));
            row
        })
        .collect()
}

/// PDFium text layer: characters (loose font boxes, so punctuation doesn't break lines) grouped
/// into lines on newlines, vertical jumps or a return to the left margin, and into words on
/// whitespace or a gap wider than ~0.3 em.
fn text_layer(page: &PdfPage) -> Result<PageText> {
    let (pw, ph) = (page.width().value, page.height().value);
    let text = page.text()?;
    let mut chars: Vec<(char, [f32; 4])> = vec![];
    for c in text.chars().iter() {
        let (Some(ch), Ok(b)) = (c.unicode_char(), c.loose_bounds()) else {
            continue;
        };
        chars.push((
            ch,
            [
                b.left().value,
                ph - b.top().value,
                b.right().value,
                ph - b.bottom().value,
            ],
        ));
    }
    Ok(PageText {
        page: 0,
        source: TextSource::TextLayer,
        width: pw,
        height: ph,
        lines: group_lines(&chars),
    })
}

fn group_lines(chars: &[(char, [f32; 4])]) -> Vec<Line> {
    let mut lines = vec![];
    let mut cur: Vec<(char, [f32; 4])> = vec![];
    for &(c, b) in chars {
        if c == '\r' || c == '\n' {
            flush(&mut cur, &mut lines);
            continue;
        }
        if let Some(&(_, last)) = cur.iter().rev().find(|(ch, _)| !ch.is_whitespace()) {
            let line_h = (last[3] - last[1]).abs().max(1.0);
            if !c.is_whitespace()
                && ((b[1] - last[1]).abs() > 0.6 * line_h || b[0] < last[0] - 2.0 * line_h)
            {
                flush(&mut cur, &mut lines);
            }
        }
        cur.push((c, b));
    }
    flush(&mut cur, &mut lines);
    lines
}

fn flush(cur: &mut Vec<(char, [f32; 4])>, lines: &mut Vec<Line>) {
    if cur.iter().all(|(c, _)| c.is_whitespace()) {
        cur.clear();
        return;
    }
    let mut words: Vec<(String, [f32; 4])> = vec![];
    let mut prev_right = f32::NEG_INFINITY;
    for &(c, b) in cur.iter() {
        if c.is_whitespace() {
            prev_right = f32::NEG_INFINITY;
            continue;
        }
        let em = (b[3] - b[1]).abs().max(1.0);
        if words.is_empty() || prev_right == f32::NEG_INFINITY || b[0] - prev_right > 0.3 * em {
            words.push((String::new(), b));
        }
        let w = words.last_mut().unwrap();
        w.0.push(c);
        w.1 = [
            w.1[0].min(b[0]),
            w.1[1].min(b[1]),
            w.1[2].max(b[2]),
            w.1[3].max(b[3]),
        ];
        prev_right = b[2];
    }
    let bbox = words
        .iter()
        .fold([f32::MAX, f32::MAX, f32::MIN, f32::MIN], |a, (_, w)| {
            [
                a[0].min(w[0]),
                a[1].min(w[1]),
                a[2].max(w[2]),
                a[3].max(w[3]),
            ]
        });
    lines.push(Line {
        text: words
            .iter()
            .map(|(t, _)| t.as_str())
            .collect::<Vec<_>>()
            .join(" "),
        bbox,
    });
    cur.clear();
}
