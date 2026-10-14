//! s106 command line. Run `s106 --help`.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};

use s106::document::{self, Document, OcrModels, Snapshot};
use s106::extract::{self, Mode};
use s106::{eval, explore, labels, llm, ocrbench, pld, reconcile, report, verify};

#[derive(Parser)]
#[command(
    name = "s106",
    about = "Verification harness for UK Section 106 planning agreements"
)]
struct Cli {
    /// the data repository checkout (default: $S106_DATA, else data/ in this repository)
    #[arg(long, global = true)]
    data: Option<PathBuf>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Extract and verify deeds: terminal report + JSON per deed. Exit 0 clean, 1 parser errors
    /// or REVIEW flags, 2 document discrepancies (the worst over all deeds), 3 if any deed could
    /// not be audited. With several deeds, one is read (OCR) while earlier ones wait on the model.
    Audit {
        /// the deeds' PDFs, or their page-text snapshots (`<data>/pages/<stem>.pages.json`) to
        /// replay without the PDFs
        #[arg(required = true)]
        pdfs: Vec<PathBuf>,
        #[arg(long, default_value = llm::DEFAULT_MODEL)]
        model: String,
        #[arg(long, value_enum, default_value = "auto")]
        llm_mode: Mode,
        /// re-read with a stronger model when Parser Errors remain; adopt only answers that verify better
        #[arg(long)]
        escalate: bool,
        #[arg(long, default_value = llm::ESCALATION_MODEL)]
        escalation_model: String,
        /// JSON output (default <data>/processed/<stem>.audit.json); only with one deed
        #[arg(long)]
        out: Option<PathBuf>,
        #[arg(short, long)]
        quiet: bool,
    },
    /// Evaluate extraction on the candidate deeds against the national S106 register.
    Eval {
        /// runs per deed (the first is recorded as a fixture)
        #[arg(long, default_value_t = 3)]
        runs: usize,
        #[arg(long, default_value = llm::DEFAULT_MODEL)]
        model: String,
        /// re-verify recorded fixtures instead of calling the API (one run per deed, no cost)
        #[arg(long)]
        replay: bool,
        /// candidate ids to run (default: all in the candidates file)
        #[arg(long, value_delimiter = ',')]
        only: Vec<String>,
        /// with --replay: read page text from snapshots in this directory instead of the PDFs
        #[arg(long)]
        pages: Option<PathBuf>,
        /// candidates.tsv (default <data>/eval/candidates.tsv)
        #[arg(long)]
        candidates: Option<PathBuf>,
        /// register directory (default <data>/eval/register, the rows for the candidates)
        #[arg(long)]
        register: Option<PathBuf>,
        /// output directory (default <data>/processed/eval)
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Produce page text (text layer or OCR, through the cache) and write or check its snapshots.
    Pages {
        pdfs: Vec<PathBuf>,
        /// write `<dir>/<stem>.pages.json` for each PDF
        #[arg(long)]
        write: Option<PathBuf>,
        /// compare with `<dir>/<stem>.pages.json` (every page, exactly); exit 1 on any difference
        #[arg(long)]
        check: Option<PathBuf>,
    },
    /// Score OCR model sets against a gold transcription file (only the gold pages are read).
    BenchOcr {
        /// a gold file (`gold/*.json` in the data repository)
        gold: PathBuf,
        /// the PDF the gold file describes
        pdf: PathBuf,
        /// model sets to compare (see document::OcrModels::named)
        #[arg(long, value_delimiter = ',', default_value = "ppocrv6-tiny")]
        models: Vec<String>,
        /// print every gold line with its best match
        #[arg(long)]
        detail: bool,
    },
    /// Fetch Planning London Datahub records and save them verbatim under <data>/snapshots/pld/.
    Pld {
        #[command(subcommand)]
        cmd: PldCmd,
    },
    /// Set a development's deeds (oldest first, replayed from recorded extractions) against PLD
    /// delivery. Writes <data>/processed/<development>.reconcile.{json,md}.
    Reconcile {
        /// the executed deeds (PDFs or committed `.pages.json` snapshots), original first, then
        /// any variations in date order
        #[arg(required = true)]
        pdfs: Vec<PathBuf>,
        /// PLD record id (`<borough>-<reference>`); must match the snapshot's record
        #[arg(long)]
        development: String,
        /// a saved PLD application snapshot, relative to <data>
        /// (snapshots/pld/applications/<id>/<time>.json)
        #[arg(long)]
        pld: Option<PathBuf>,
        #[arg(long, default_value = llm::DEFAULT_MODEL)]
        model: String,
    },
    /// Score the pipeline and the sources against whatever human labels are in <data>/labels/.
    /// Writes <data>/reports/labels/labels.{md,json}; with no labels it says so and succeeds.
    Labels {
        /// the recorded extractions to score
        #[arg(long, default_value = llm::DEFAULT_MODEL)]
        model: String,
    },
    /// Bake the results explorer's data (Datahub-derived only) into <out>/sites.json and
    /// <out>/boroughs.geojson. Paths are relative to the data root.
    Explore {
        /// a Datahub baseline run's applications table
        #[arg(
            long,
            default_value = "runs/2026-10-14-datahub-baseline/tables/applications.csv"
        )]
        applications: PathBuf,
        /// the Datahub search snapshot that table was built from (for site centroids)
        #[arg(
            long,
            default_value = "snapshots/pld/applications/_search/london-ge10/2026-10-14T00_14_09Z"
        )]
        snapshot: PathBuf,
        /// borough boundaries as GeoJSON
        #[arg(
            long,
            default_value = "snapshots/ons/lad-2023-bgc-london/2026-10-14T14_06_00Z.geojson"
        )]
        boundaries: PathBuf,
        #[arg(long, default_value = "explorer/data")]
        out: PathBuf,
    },
}

#[derive(Subcommand)]
enum PldCmd {
    /// GET <index>/_source/<id>, e.g. `applications <borough>-<reference>`
    Get { index: String, id: String },
    /// POST <index>/_search with an Elasticsearch query from a JSON file
    Search {
        index: String,
        query: PathBuf,
        /// names the snapshot directory, e.g. `<reference>-units`
        #[arg(long)]
        subject: String,
    },
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

fn credentials(mode: Mode) -> Result<Option<rust_claude_sdk::Client>> {
    match mode {
        Mode::Replay => Ok(None),
        Mode::Auto => Ok(llm::client().ok()),
        Mode::Live | Mode::Record => Ok(Some(llm::client()?)),
    }
}

/// Deeds whose model calls may be in flight at once during a batch audit.
const AUDITS_IN_FLIGHT: usize = 4;

struct AuditOpts {
    model: String,
    escalate: bool,
    escalation_model: String,
}

/// A deed's page text and the LLM mode to audit it with (a snapshot has no PDF, so it replays).
fn read_deed(
    pdf: &Path,
    cfg: &document::Config,
    mode: Mode,
    quiet: bool,
) -> Result<(Document, Mode)> {
    if pdf.to_string_lossy().ends_with(".pages.json") {
        if !matches!(mode, Mode::Replay | Mode::Auto) {
            bail!("a snapshot has no PDF to send; use --llm-mode replay");
        }
        Ok((Snapshot::read(pdf)?.document(), Mode::Replay))
    } else {
        Ok((document::load(pdf, cfg, !quiet)?, mode))
    }
}

/// The stem of a deed's audit JSON: its PDF's, or for a snapshot the PDF it was read from.
fn output_stem(pdf: &Path) -> Result<String> {
    let source = if pdf.to_string_lossy().ends_with(".pages.json") {
        PathBuf::from(Snapshot::read(pdf)?.source_file)
    } else {
        pdf.to_path_buf()
    };
    Ok(source
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned())
}

/// Extract, verify and write the audit JSON for one deed; returns the report text and exit code.
fn audit(
    doc: &Document,
    mode: Mode,
    client: Option<&rust_claude_sdk::Client>,
    o: &AuditOpts,
    out: Option<PathBuf>,
) -> Result<(String, i32)> {
    let (mut agreement, info) = extract::extract(doc, &o.model, mode, client)?;
    let pages = doc.pages();
    let mut results = verify::verify(&agreement, &pages);
    if o.escalate {
        let (merged, log) =
            extract::escalate(doc, agreement, &results, &o.escalation_model, mode, client)?;
        for l in &log {
            eprintln!("  escalation: {l}");
        }
        agreement = merged;
        results = verify::verify(&agreement, &pages);
    }
    agreement.warnings.extend(info.usage_note());
    let out = match out {
        Some(out) => out,
        None => s106::data::processed().join(format!("{}.audit.json", output_stem(&doc.path)?)),
    };
    std::fs::create_dir_all(out.parent().unwrap())?;
    std::fs::write(
        &out,
        serde_json::to_string_pretty(&report::AuditJson {
            agreement: &agreement,
            verification: &results,
        })?,
    )?;
    Ok((
        format!(
            "{}\n\nJSON written to {}\n",
            report::render(&agreement, &results),
            out.display()
        ),
        report::exit_code(&results),
    ))
}

/// Print a finished batch audit, keeping the worst exit code (3 for a deed that failed).
fn finish(
    pdf: &Path,
    h: std::thread::ScopedJoinHandle<'_, Result<(String, i32)>>,
    worst: &mut i32,
) {
    match h.join().expect("audit thread panicked") {
        Ok((report, code)) => {
            print!("{report}");
            *worst = (*worst).max(code);
        }
        Err(e) => {
            eprintln!("{}: {e:#}", pdf.display());
            *worst = 3;
        }
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    if let Some(dir) = cli.data {
        // SAFETY: set once at startup, before any thread is spawned or the variable is read
        unsafe { std::env::set_var("S106_DATA", dir) };
    }
    match cli.cmd {
        Cmd::Audit {
            pdfs,
            model,
            llm_mode,
            escalate,
            escalation_model,
            out,
            quiet,
        } => {
            if out.is_some() && pdfs.len() > 1 {
                bail!(
                    "--out takes one deed; with several, each goes to <data>/processed/<stem>.audit.json"
                );
            }
            let cfg = document::Config::from_env(&repo_root());
            let opts = AuditOpts {
                model,
                escalate,
                escalation_model,
            };
            if let [pdf] = pdfs.as_slice() {
                let (doc, mode) = read_deed(pdf, &cfg, llm_mode, quiet)?;
                let client = credentials(mode)?;
                let (report, code) = audit(&doc, mode, client.as_ref(), &opts, out)?;
                print!("{report}");
                std::process::exit(code);
            }
            // Each deed's JSON is named by its stem, so two inputs sharing one would overwrite
            // each other's (a PDF and its snapshot, or the same file twice).
            let mut stems = std::collections::BTreeMap::new();
            for pdf in &pdfs {
                if let Some(other) = stems.insert(output_stem(pdf)?, pdf) {
                    bail!(
                        "{} and {} would both write <data>/processed/{}.audit.json; audit them separately",
                        other.display(),
                        pdf.display(),
                        output_stem(pdf)?
                    );
                }
            }
            // This thread reads each deed in turn (the slow OCR) while up to AUDITS_IN_FLIGHT
            // earlier deeds wait on the model in their own threads. Reports print in input order.
            let client = credentials(llm_mode)?;
            let mut worst = 0;
            std::thread::scope(|s| {
                let mut running = std::collections::VecDeque::new();
                for pdf in &pdfs {
                    if running.len() == AUDITS_IN_FLIGHT {
                        let (pdf, h) = running.pop_front().unwrap();
                        finish(pdf, h, &mut worst);
                    }
                    match read_deed(pdf, &cfg, llm_mode, quiet) {
                        Ok((doc, mode)) => {
                            let (client, opts) = (client.as_ref(), &opts);
                            let h = s.spawn(move || audit(&doc, mode, client, opts, None));
                            running.push_back((pdf, h));
                        }
                        Err(e) => {
                            eprintln!("{}: {e:#}", pdf.display());
                            worst = 3;
                        }
                    }
                }
                while let Some((pdf, h)) = running.pop_front() {
                    finish(pdf, h, &mut worst);
                }
            });
            std::process::exit(worst);
        }
        Cmd::Eval {
            runs,
            model,
            replay,
            only,
            pages,
            candidates,
            register,
            out,
        } => {
            let d = eval::EvalArgs::defaults();
            eval::run(
                &eval::EvalArgs {
                    runs,
                    model,
                    replay,
                    only,
                    pages,
                    candidates: candidates.unwrap_or(d.candidates),
                    register: register.unwrap_or(d.register),
                    out: out.unwrap_or(d.out),
                    pdfs: d.pdfs,
                },
                &repo_root(),
            )
        }
        Cmd::Pages { pdfs, write, check } => {
            let cfg = document::Config::from_env(&repo_root());
            let mut differ = 0;
            for pdf in &pdfs {
                // A snapshot is checked with the model set that made it, which may not be the default.
                let expected = match &check {
                    Some(dir) => {
                        let path =
                            Snapshot::path_for(dir, &pdf.file_name().unwrap().to_string_lossy());
                        Some((Snapshot::read(&path)?, path))
                    }
                    None => None,
                };
                let cfg = match &expected {
                    Some((snap, path)) => {
                        cfg.clone()
                            .with_models_of(&snap.producer)
                            .with_context(|| {
                                format!(
                                    "{}: unknown OCR model set {}",
                                    path.display(),
                                    snap.producer.ocr_models
                                )
                            })?
                    }
                    None => cfg.clone(),
                };
                let t0 = std::time::Instant::now();
                let doc = document::load(pdf, &cfg, true)?;
                eprintln!(
                    "{}: {} pages ({}) in {:.1}s",
                    pdf.display(),
                    doc.page_count(),
                    doc.text_source(),
                    t0.elapsed().as_secs_f64()
                );
                if let Some(dir) = &write {
                    let path = Snapshot::of(&doc, &cfg)?.write(dir)?;
                    eprintln!("  wrote {}", path.display());
                }
                if let Some((snap, path)) = &expected {
                    let diffs = snap.compare_document(&doc);
                    for d in &diffs {
                        println!("{}: {d}", pdf.display());
                    }
                    if diffs.is_empty() {
                        eprintln!("  matches {}", path.display());
                    }
                    differ += usize::from(!diffs.is_empty());
                }
            }
            if differ > 0 {
                bail!(
                    "{differ} of {} documents differ from their snapshots",
                    pdfs.len()
                );
            }
            Ok(())
        }
        Cmd::BenchOcr {
            gold,
            pdf,
            models,
            detail,
        } => {
            let g = ocrbench::Gold::load(&gold)?;
            let sha = document::sha256_file(&pdf)?;
            if sha != g.sha256 {
                bail!(
                    "{} does not match the gold file's document (sha256 {} vs {})",
                    pdf.display(),
                    sha,
                    g.sha256
                );
            }
            let pages = g.pages();
            for name in models {
                let mut cfg = document::Config::from_env(&repo_root());
                cfg.ocr = OcrModels::named(&name, &cfg.models_dir)
                    .ok_or_else(|| anyhow::anyhow!("unknown model set {name}"))?;
                let t0 = std::time::Instant::now();
                let doc = document::load_pages(&pdf, &cfg, Some(&pages), false)?;
                let s = ocrbench::score(&g, &doc);
                println!(
                    "== {name} ({}; {:.1}s incl. cache)\n{s}",
                    doc.text_source(),
                    t0.elapsed().as_secs_f64()
                );
                if detail {
                    for (p, sp, sq, gold, got) in &s.detail {
                        println!("  p{p:<3} {sp:.3} {sq:.3}  {gold}\n                 got: {got}");
                    }
                }
            }
            Ok(())
        }
        Cmd::Pld { cmd } => {
            let root = s106::data::root();
            let (index, subject, snap) = match cmd {
                PldCmd::Get { index, id } => {
                    let snap = pld::get_source(&index, &id)?;
                    (index, id, snap)
                }
                PldCmd::Search {
                    index,
                    query,
                    subject,
                } => {
                    let body = serde_json::from_slice(&std::fs::read(&query)?)?;
                    let snap = pld::search(&index, body)?;
                    (index, subject, snap)
                }
            };
            let r = pld::save(&root, &index, &subject, &snap)?;
            println!("{} (sha256 {})", r.path, r.sha256);
            Ok(())
        }
        Cmd::Reconcile {
            pdfs,
            development,
            pld: pld_snapshot,
            model,
        } => {
            let cfg = document::Config::from_env(&repo_root());
            let mut instruments = vec![];
            for pdf in &pdfs {
                // a committed page snapshot replays with no PDF, as in `audit`
                let doc = if pdf.to_string_lossy().ends_with(".pages.json") {
                    Snapshot::read(pdf)?.document()
                } else {
                    document::load(pdf, &cfg, true)?
                };
                let (agreement, _) = extract::extract(&doc, &model, Mode::Replay, None)?;
                instruments.push(reconcile::instrument_obligations(
                    &agreement,
                    Some(&doc.pages()),
                ));
            }
            let delivery = match pld_snapshot {
                Some(p) => {
                    let (snap, at) = pld::load(&s106::data::root(), &p)?;
                    Some(reconcile::delivery_from_pld(&snap, at)?)
                }
                None => None,
            };
            let r = reconcile::reconcile(&development, instruments, delivery)?;
            let md = reconcile::markdown(&r);
            println!("{md}");
            let dir = s106::data::processed();
            std::fs::create_dir_all(&dir)?;
            let name = pld::safe(&development);
            std::fs::write(
                dir.join(format!("{name}.reconcile.json")),
                serde_json::to_string_pretty(&r)?,
            )?;
            std::fs::write(dir.join(format!("{name}.reconcile.md")), md)?;
            Ok(())
        }
        Cmd::Explore {
            applications,
            snapshot,
            boundaries,
            out,
        } => {
            let root = s106::data::root();
            let inputs = explore::Inputs {
                applications,
                snapshot,
                boundaries,
            };
            let bundle = explore::bake(&root, &inputs)?;
            std::fs::create_dir_all(&out)?;
            std::fs::write(out.join("sites.json"), serde_json::to_string(&bundle)?)?;
            std::fs::copy(root.join(&inputs.boundaries), out.join("boroughs.geojson"))
                .with_context(|| format!("copying {}", inputs.boundaries.display()))?;
            eprintln!(
                "wrote {} sites ({} without a centroid left out) to {}",
                bundle.sources.sites,
                bundle.sources.without_centroid,
                out.display()
            );
            Ok(())
        }
        Cmd::Labels { model } => {
            let root = s106::data::root();
            let r = labels::run(&root, &model)?;
            println!("{}", labels::markdown(&r));
            eprintln!(
                "wrote {}",
                root.join("reports/labels/labels.{md,json}").display()
            );
            Ok(())
        }
    }
}
