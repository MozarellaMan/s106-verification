//! Page text with provenance: the PDF's text layer where it has one, OCR where it doesn't.
//!
//! Every line keeps its bounding box, so evidence can later be tied to a position on the page.
//!
//! This is the expensive stage (a 109-page scan takes minutes), so its output is kept twice:
//!
//! * a working cache, `<cache root>/<sha256[:16]>-<ocr_version>/`, one JSON file per page plus a
//!   `manifest.json` that records the full [`Producer`]. A warm cache is read without opening the
//!   PDF in PDFium or loading the OCR runtime.
//! * a [`Snapshot`]: the whole document's page text in one reviewable file, committed under
//!   `pages/` in the data repository so replay, regression tests and evaluation run without
//!   PDFs or models.
//!
//! Both are keyed by the PDF's SHA-256 and by everything that produces the text: the line logic
//! version, the OCR settings, and the pinned SHA-256 of every runtime artifact (PDFium, ONNX
//! Runtime, model files; `scripts/runtime-artifacts.tsv`). Changing any of them gives a new key,
//! so a stale cache is never read. Model files are checked against their pins before use.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use pdfium_render::prelude::*;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::evidence::Pages;

/// A page needs OCR when its text layer has fewer non-whitespace characters than this.
const MIN_TEXT_CHARS: usize = 50;
const OCR_DPI: f32 = 200.0;
/// Bump when `text_layer`, `group_lines`, `ocr_lines` or the OCR filtering change their output for
/// the same input, then regenerate the snapshots (`s106 pages --write data/pages`). It is part
/// of the cache key. An unbumped change shows up as a snapshot mismatch in the real-PDF tiers.
pub const PAGE_TEXT_LOGIC: u32 = 3;
const PDFIUM_ARCHIVE: &str = "pdfium-linux-x64.tgz";
const ORT_ARCHIVE: &str = "onnxruntime-linux-x64-1.28.0.tgz";
const RUNTIME_ARTIFACTS: &str = include_str!("../scripts/runtime-artifacts.tsv");

/// Where scripts/fetch-models.sh unpacks a pinned archive: a directory named by its SHA-256, so
/// the library loaded is always the one the pin (and so the cache key) names.
fn unpacked(models_dir: &Path, name: &str, archive: &str) -> PathBuf {
    let sha = pinned_sha256(archive).expect("runtime archives are pinned");
    models_dir.join(format!("{name}-{}", &sha[..12]))
}
pub const SNAPSHOT_FORMAT: &str = "s106-page-text/1";
const CACHE_FORMAT: &str = "s106-page-text-cache/1";

/// The pinned SHA-256 of a runtime artifact (`scripts/runtime-artifacts.tsv`).
pub fn pinned_sha256(file: &str) -> Option<&'static str> {
    RUNTIME_ARTIFACTS
        .lines()
        .filter(|l| !l.starts_with('#'))
        .map(|l| l.split('\t').collect::<Vec<_>>())
        .find(|f| f.len() == 4 && f[1] == file)
        .map(|f| f[2])
}

/// Everything that determines a document's page text apart from the PDF itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Producer {
    pub stage: String,
    pub logic: u32,
    pub min_text_chars: usize,
    pub ocr_dpi: u32,
    pub ocr_models: String,
    /// runtime artifact file -> pinned SHA-256
    pub artifacts: BTreeMap<String, String>,
}

impl Producer {
    /// Short content hash of the producer, part of every cache and snapshot key.
    pub fn key(&self) -> String {
        let json = serde_json::to_string(self).expect("producer serializes");
        hex(&Sha256::digest(json.as_bytes()))[..12].to_string()
    }

    /// Readable cache key: logic version, model set, DPI and the producer hash.
    pub fn ocr_version(&self) -> String {
        format!(
            "v{}-pdfium8100-{}-{}dpi-{}",
            self.logic,
            self.ocr_models,
            self.ocr_dpi,
            self.key()
        )
    }
}

/// The OCR models, chosen by score against the agent-transcribed gold set (see README).
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
    /// Pages OCR'd at the same time, each on its own engine (`S106_OCR_WORKERS`, default 1).
    /// Opt-in: each engine holds its own ONNX Runtime session, so memory grows with it.
    pub ocr_workers: usize,
}

impl Config {
    /// Defaults: `models/` in the repository and `interim/ocr/` in the data checkout.
    /// `S106_MODELS` and `S106_OCR_CACHE` override them; `S106_OCR_WORKERS` sets `ocr_workers`.
    pub fn from_env(repo_root: &Path) -> Config {
        let models_dir = std::env::var_os("S106_MODELS")
            .map(PathBuf::from)
            .unwrap_or(repo_root.join("models"));
        let cache_root = std::env::var_os("S106_OCR_CACHE")
            .map(PathBuf::from)
            .unwrap_or(crate::data::root().join("interim/ocr"));
        let ocr = OcrModels::default_in(&models_dir);
        let ocr_workers = std::env::var("S106_OCR_WORKERS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(1)
            .max(1);
        Config {
            models_dir,
            cache_root,
            ocr,
            ocr_workers,
        }
    }

    /// Everything that produces page text, with the pinned SHA-256 of each runtime artifact.
    pub fn producer(&self) -> Producer {
        let artifacts = [PDFIUM_ARCHIVE, ORT_ARCHIVE]
            .into_iter()
            .map(String::from)
            .chain(
                [&self.ocr.det, &self.ocr.rec, &self.ocr.dict]
                    .iter()
                    .map(|p| file_name(p)),
            )
            .map(|f| {
                let sha = pinned_sha256(&f).unwrap_or("unpinned").to_string();
                (f, sha)
            })
            .collect();
        Producer {
            stage: "page-text".into(),
            logic: PAGE_TEXT_LOGIC,
            min_text_chars: MIN_TEXT_CHARS,
            ocr_dpi: OCR_DPI as u32,
            ocr_models: self.ocr.name.clone(),
            artifacts,
        }
    }

    pub fn ocr_version(&self) -> String {
        self.producer().ocr_version()
    }

    /// ONNX Runtime threads per OCR engine when `ocr_workers` > 1: the cores split between them.
    fn ocr_threads_per_worker(&self) -> usize {
        let cores = std::thread::available_parallelism().map_or(1, |n| n.get());
        (cores / self.ocr_workers).max(1)
    }

    /// Cache directory name for `sha256`. Parallel OCR keeps its own cache, keyed by the worker
    /// count and thread cap, until `cargo test-pdf` shows it reads pages identically.
    fn cache_dir_name(&self, sha256: &str) -> String {
        let name = format!("{}-{}", &sha256[..16], self.ocr_version());
        if self.ocr_workers > 1 {
            format!(
                "{name}-{}w{}t",
                self.ocr_workers,
                self.ocr_threads_per_worker()
            )
        } else {
            name
        }
    }

    /// PDFium can be bound once per process, so the binding is shared. It is also not
    /// thread-safe: every use goes through this lock.
    fn pdfium(&self) -> Result<std::sync::MutexGuard<'static, Pdfium>> {
        // `get_or_init` runs the binding once even when tests load documents in parallel.
        type Bound = std::result::Result<std::sync::Mutex<Pdfium>, String>;
        static PDFIUM: std::sync::OnceLock<Bound> = std::sync::OnceLock::new();
        let bound = PDFIUM.get_or_init(|| {
            let lib = unpacked(&self.models_dir, "pdfium", PDFIUM_ARCHIVE).join("lib");
            Pdfium::bind_to_library(Pdfium::pdfium_platform_library_name_at_path(&lib))
                .map(|b| std::sync::Mutex::new(Pdfium::new(b)))
                .map_err(|e| {
                    format!(
                        "load libpdfium from {} (run scripts/fetch-models.sh): {e}",
                        lib.display()
                    )
                })
        });
        match bound {
            Ok(m) => Ok(m.lock().unwrap_or_else(|e| e.into_inner())),
            Err(e) => bail!("{e}"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TextSource {
    TextLayer,
    Ocr,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Line {
    pub text: String,
    /// [left, top, right, bottom]: PDF points from the top-left (text layer) or pixels at OCR_DPI (OCR).
    pub bbox: [f32; 4],
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn file_name(p: &Path) -> String {
    p.file_name()
        .map(|f| f.to_string_lossy().into_owned())
        .unwrap_or_default()
}

pub fn sha256_file(path: &Path) -> Result<String> {
    let bytes = std::fs::read(path).with_context(|| format!("read {}", path.display()))?;
    Ok(hex(&Sha256::digest(&bytes)))
}

/// Refuse a runtime artifact whose bytes differ from its pin: its output would be filed under a
/// cache key that names different files.
fn check_pinned(path: &Path) -> Result<()> {
    let name = file_name(path);
    let Some(pin) = pinned_sha256(&name) else {
        bail!("{name} is not pinned in scripts/runtime-artifacts.tsv");
    };
    let got = sha256_file(path)
        .with_context(|| format!("{name} missing (run scripts/fetch-models.sh)"))?;
    if got != pin {
        bail!(
            "{} has sha256 {got}, not the pinned {pin} (run scripts/fetch-models.sh)",
            path.display()
        );
    }
    Ok(())
}

/// Write via a temporary file unique to this writer, so an interrupted run never leaves a
/// truncated cache entry and concurrent writers never share a temporary file.
fn write_atomic(path: &Path, contents: &str) -> Result<()> {
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let tmp = path.with_extension(format!("{}.{n}.tmp", std::process::id()));
    std::fs::write(&tmp, contents).with_context(|| format!("write {}", tmp.display()))?;
    std::fs::rename(&tmp, path).with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

#[derive(Debug, Serialize, Deserialize)]
struct Manifest {
    format: String,
    sha256: String,
    /// set once the PDF has been opened; a warm load needs it to know every page is cached
    page_count: Option<usize>,
    ocr_version: String,
    producer: Producer,
    source_path: String,
    created: String,
}

/// The cache directory for `sha256`, with its manifest validated (or created).
fn open_cache(pdf: &Path, sha256: &str, cfg: &Config) -> Result<(PathBuf, Manifest)> {
    let ocr_version = cfg.ocr_version();
    let producer = cfg.producer();
    let dir = cfg.cache_root.join(cfg.cache_dir_name(sha256));
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("manifest.json");
    if let Ok(s) = std::fs::read_to_string(&path) {
        let m: Manifest = serde_json::from_str(&s).with_context(|| {
            format!(
                "{} is not a current cache manifest; delete {}",
                path.display(),
                dir.display()
            )
        })?;
        if m.format != CACHE_FORMAT || m.sha256 != sha256 || m.producer != producer {
            bail!(
                "{} describes a different document or producer; delete {}",
                path.display(),
                dir.display()
            );
        }
        return Ok((dir, m));
    }
    let m = Manifest {
        format: CACHE_FORMAT.into(),
        sha256: sha256.into(),
        page_count: None,
        ocr_version,
        producer,
        source_path: pdf
            .canonicalize()
            .unwrap_or(pdf.to_path_buf())
            .display()
            .to_string(),
        created: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
    };
    write_atomic(&path, &(serde_json::to_string_pretty(&m)? + "\n"))?;
    Ok((dir, m))
}

fn read_page(path: &Path, n: usize) -> Result<Option<PageText>> {
    let Ok(s) = std::fs::read_to_string(path) else {
        return Ok(None);
    };
    let p: PageText =
        serde_json::from_str(&s).with_context(|| format!("parse {}", path.display()))?;
    if p.page != n {
        bail!("{} holds page {}, not {n}", path.display(), p.page);
    }
    Ok(Some(p))
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
    let (dir, mut manifest) = open_cache(pdf, &sha256, cfg)?;
    let page_file = |n: usize| dir.join(format!("page_{n:03}.json"));
    let wanted = |count: usize| -> Vec<usize> {
        only.map_or((1..=count).collect(), |o| {
            o.iter()
                .copied()
                .filter(|&n| n >= 1 && n <= count)
                .collect()
        })
    };
    let document = |pages: Vec<PageText>| Document {
        path: pdf.to_path_buf(),
        sha256: sha256.clone(),
        ocr_version: manifest.ocr_version.clone(),
        pages,
    };

    // Warm cache: every wanted page is on disk, so neither PDFium nor the OCR runtime is needed.
    if let Some(count) = manifest.page_count {
        let mut pages = vec![];
        for n in wanted(count) {
            match read_page(&page_file(n), n)? {
                Some(p) => pages.push(p),
                None => break,
            }
        }
        if pages.len() == wanted(count).len() {
            return Ok(document(pages));
        }
    }

    let pdfium = cfg.pdfium()?;
    let doc = pdfium
        .load_pdf_from_file(pdf, None)
        .with_context(|| format!("open {}", pdf.display()))?;
    let count = doc.pages().len() as usize;
    if manifest.page_count != Some(count) {
        manifest.page_count = Some(count);
        write_atomic(
            &dir.join("manifest.json"),
            &(serde_json::to_string_pretty(&manifest)? + "\n"),
        )?;
    }
    let mut pages: Vec<Option<PageText>> = vec![None; count];
    let mut need_ocr = vec![];
    let wanted = wanted(count);
    for &n in &wanted {
        if let Some(p) = read_page(&page_file(n), n)? {
            pages[n - 1] = Some(p);
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
            write_atomic(&page_file(n), &serde_json::to_string(&text)?)?;
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
        let render = |n: usize| -> Result<image::RgbImage> {
            let page = doc.pages().get((n - 1) as i32)?;
            // PDFium applies the page's /Rotate, so the bitmap is upright as displayed.
            let cfg_r = PdfRenderConfig::new().scale_page_by_factor(OCR_DPI / 72.0);
            Ok(page.render_with_config(&cfg_r)?.as_image()?.into_rgb8())
        };
        let save = |text: &PageText| -> Result<()> {
            write_atomic(&page_file(text.page), &serde_json::to_string(text)?)?;
            if verbose {
                eprintln!("  page {}/{count}: {} lines", text.page, text.lines.len());
            }
            Ok(())
        };
        // One page per call. oar-ocr pads a batch's images and text crops to a common size, so a
        // page recognised alongside others can read differently (measured: 5 of 9 pages of one
        // scanned deed changed between batches of 6 and of 1). Alone, a page's text depends only on that page,
        // whatever else is cached or requested. oar-ocr still parallelises within the page.
        let done = if cfg.ocr_workers == 1 {
            let engine = ocr_engine(&cfg.ocr, None)?;
            let mut done = vec![];
            for &n in &need_ocr {
                let text = ocr_page(&engine, n, render(n)?)?;
                save(&text)?;
                done.push(text);
            }
            done
        } else {
            ocr_pages_parallel(cfg, &need_ocr, render, save)?
        };
        for text in done {
            let n = text.page;
            pages[n - 1] = Some(text);
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
    Ok(document(pages))
}

// ------------------------------------------------------------------------------- snapshots

/// A whole document's page text with its provenance: the committed, reviewable form of the
/// page-text stage (`pages/<stem>.pages.json` in the data repository). Replaying from a snapshot needs no PDF,
/// PDFium or OCR models.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub format: String,
    /// the PDF's file name; its stem names the recorded model responses
    pub source_file: String,
    pub sha256: String,
    pub page_count: usize,
    pub ocr_version: String,
    pub producer: Producer,
    pub pages: Vec<PageText>,
}

impl Snapshot {
    pub fn of(doc: &Document, cfg: &Config) -> Result<Snapshot> {
        if doc.ocr_version != cfg.ocr_version() {
            bail!(
                "document was read with {}, not {}",
                doc.ocr_version,
                cfg.ocr_version()
            );
        }
        Ok(Snapshot {
            format: SNAPSHOT_FORMAT.into(),
            source_file: file_name(&doc.path),
            sha256: doc.sha256.clone(),
            page_count: doc.page_count(),
            ocr_version: doc.ocr_version.clone(),
            producer: cfg.producer(),
            pages: doc.pages.clone(),
        })
    }

    pub fn path_for(dir: &Path, source_file: &str) -> PathBuf {
        let stem = Path::new(source_file)
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        dir.join(format!("{stem}.pages.json"))
    }

    /// One page per line, so a changed page is a one-line diff.
    pub fn to_json(&self) -> Result<String> {
        let mut head = serde_json::to_value(self)?;
        head.as_object_mut().unwrap().remove("pages");
        let head = serde_json::to_string_pretty(&head)?;
        let pages = self
            .pages
            .iter()
            .map(serde_json::to_string)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(format!(
            "{},\n  \"pages\": [\n{}\n  ]\n}}\n",
            head.trim_end().trim_end_matches('}').trim_end(),
            pages.join(",\n")
        ))
    }

    pub fn write(&self, dir: &Path) -> Result<PathBuf> {
        std::fs::create_dir_all(dir)?;
        let path = Self::path_for(dir, &self.source_file);
        write_atomic(&path, &self.to_json()?)?;
        Ok(path)
    }

    pub fn read(path: &Path) -> Result<Snapshot> {
        let s =
            std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
        let snap: Snapshot =
            serde_json::from_str(&s).with_context(|| format!("parse {}", path.display()))?;
        if snap.format != SNAPSHOT_FORMAT {
            bail!(
                "{}: format {}, expected {SNAPSHOT_FORMAT}",
                path.display(),
                snap.format
            );
        }
        if snap.ocr_version != snap.producer.ocr_version() {
            bail!(
                "{}: ocr_version does not match its producer",
                path.display()
            );
        }
        let numbered: Vec<usize> = snap.pages.iter().map(|p| p.page).collect();
        if numbered != (1..=snap.page_count).collect::<Vec<_>>() {
            bail!("{}: pages are not 1..={}", path.display(), snap.page_count);
        }
        Ok(snap)
    }

    pub fn document(self) -> Document {
        Document {
            path: PathBuf::from(self.source_file),
            sha256: self.sha256,
            ocr_version: self.ocr_version,
            pages: self.pages,
        }
    }

    /// Differences between this snapshot and a whole document read afresh: the same PDF, producer
    /// and every page 1..=page_count, each page identical (source, dimensions, lines and boxes).
    /// An empty list means they agree.
    pub fn compare_document(&self, doc: &Document) -> Vec<String> {
        let numbers: Vec<usize> = doc.pages.iter().map(|p| p.page).collect();
        if numbers != (1..=self.page_count).collect::<Vec<_>>() {
            return vec![format!(
                "document has pages {numbers:?}, snapshot has 1..={}",
                self.page_count
            )];
        }
        self.compare_pages(doc)
    }

    /// As `compare_document`, for a document read for selected pages only: each page it holds
    /// must be identical to the snapshot's.
    pub fn compare_pages(&self, doc: &Document) -> Vec<String> {
        let mut out = vec![];
        if self.sha256 != doc.sha256 {
            out.push(format!("sha256 {} vs {}", self.sha256, doc.sha256));
            return out;
        }
        if self.ocr_version != doc.ocr_version {
            out.push(format!(
                "ocr_version {} vs {}",
                self.ocr_version, doc.ocr_version
            ));
        }
        for d in &doc.pages {
            match self.pages.iter().find(|p| p.page == d.page) {
                None => out.push(format!("page {}: not in the snapshot", d.page)),
                Some(s) if s != d => out.push(format!("page {}: {}", d.page, page_diff(s, d))),
                Some(_) => {}
            }
        }
        out
    }
}

/// The first way two versions of a page differ, for the mismatch report.
fn page_diff(a: &PageText, b: &PageText) -> String {
    if a.source != b.source {
        return format!("source {:?} vs {:?}", a.source, b.source);
    }
    if (a.width, a.height) != (b.width, b.height) {
        return format!("size {}x{} vs {}x{}", a.width, a.height, b.width, b.height);
    }
    if a.lines.len() != b.lines.len() {
        return format!("{} vs {} lines", a.lines.len(), b.lines.len());
    }
    match a.lines.iter().zip(&b.lines).position(|(x, y)| x != y) {
        Some(i) => format!(
            "line {}: {:?} {:?} vs {:?} {:?}",
            i + 1,
            a.lines[i].text,
            a.lines[i].bbox,
            b.lines[i].text,
            b.lines[i].bbox
        ),
        None => "differs".into(),
    }
}

/// OCR one rendered page on its own (never batched with other pages).
fn ocr_page(
    engine: &oar_ocr::prelude::OAROCR,
    n: usize,
    image: image::RgbImage,
) -> Result<PageText> {
    let (w, h) = image.dimensions();
    let res = engine
        .predict(vec![image])?
        .pop()
        .context("OCR returned no result")?;
    Ok(PageText {
        page: n,
        source: TextSource::Ocr,
        width: w as f32,
        height: h as f32,
        lines: ocr_lines(&res),
    })
}

/// `ocr_workers` pages at a time. PDFium is not thread-safe, so this thread renders and hands
/// pages to the workers; each worker has its own engine (oar-ocr runs one session at a time) with
/// ONNX Runtime's threads split between them, and still OCRs one page per call.
fn ocr_pages_parallel(
    cfg: &Config,
    need_ocr: &[usize],
    render: impl Fn(usize) -> Result<image::RgbImage>,
    save: impl Fn(&PageText) -> Result<()> + Sync,
) -> Result<Vec<PageText>> {
    let intra = cfg.ocr_threads_per_worker();
    let engines = (0..cfg.ocr_workers)
        .map(|_| ocr_engine(&cfg.ocr, Some(intra)))
        .collect::<Result<Vec<_>>>()?;
    let (tx, rx) = std::sync::mpsc::sync_channel::<(usize, image::RgbImage)>(cfg.ocr_workers);
    // shared by the workers only, so it is dropped (and a send fails) once they have all stopped
    let rx = std::sync::Arc::new(std::sync::Mutex::new(rx));
    std::thread::scope(|s| {
        let workers: Vec<_> = engines
            .iter()
            .map(|engine| {
                let (rx, save) = (rx.clone(), &save);
                s.spawn(move || -> Result<Vec<PageText>> {
                    let mut done = vec![];
                    loop {
                        // its own statement, so the lock is released before the page is OCR'd
                        let next = rx.lock().unwrap().recv();
                        let Ok((n, image)) = next else { break };
                        let text = ocr_page(engine, n, image)?;
                        save(&text)?;
                        done.push(text);
                    }
                    Ok(done)
                })
            })
            .collect();
        drop(rx);
        let mut rendered = Ok(());
        for &n in need_ocr {
            match render(n) {
                Ok(image) => {
                    // fails only when every worker has stopped on an error, reported below
                    if tx.send((n, image)).is_err() {
                        break;
                    }
                }
                Err(e) => {
                    rendered = Err(e);
                    break;
                }
            }
        }
        drop(tx);
        let mut done = vec![];
        for w in workers {
            done.extend(w.join().expect("OCR worker panicked")?);
        }
        rendered?;
        Ok(done)
    })
}

fn ocr_engine(m: &OcrModels, intra_threads: Option<usize>) -> Result<oar_ocr::prelude::OAROCR> {
    for p in [&m.det, &m.rec, &m.dict] {
        check_pinned(p)?;
    }
    // ONNX Runtime is bound once per process, from the copy fetched beside the models.
    static ORT: std::sync::OnceLock<Result<(), String>> = std::sync::OnceLock::new();
    let lib = unpacked(
        m.det.parent().unwrap_or(Path::new(".")),
        "onnxruntime",
        ORT_ARCHIVE,
    )
    .join("lib/libonnxruntime.so");
    ORT.get_or_init(|| {
        ort::init_from(&lib)
            .map(|env| {
                env.commit();
            })
            .map_err(|e| format!("load {} (run scripts/fetch-models.sh): {e}", lib.display()))
    })
    .clone()
    .map_err(anyhow::Error::msg)?;
    Ok({
        let mut b =
            oar_ocr::prelude::OAROCRBuilder::new(m.det.clone(), m.rec.clone(), m.dict.clone());
        if let Some(t) = intra_threads {
            b = b.ort_session(oar_ocr::core::config::OrtSessionConfig::new().with_intra_threads(t));
        }
        b.build()?
    })
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
