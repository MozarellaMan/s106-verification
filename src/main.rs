//! s106 command line. Run `s106 --help`.

use std::path::{Path, PathBuf};

use anyhow::{Result, bail};
use clap::{Parser, Subcommand};

use s106::document::{self, OcrModels};
use s106::extract::{self, Mode};
use s106::{eval, llm, ocrbench, pld, reconcile, report, verify};

#[derive(Parser)]
#[command(
    name = "s106",
    about = "Verification harness for UK Section 106 planning agreements"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Extract and verify one deed: terminal report + JSON. Exit 0 clean, 1 parser errors or
    /// REVIEW flags, 2 document discrepancies.
    Audit {
        pdf: PathBuf,
        #[arg(long, default_value = llm::DEFAULT_MODEL)]
        model: String,
        #[arg(long, value_enum, default_value = "auto")]
        llm_mode: Mode,
        /// re-read with a stronger model when Parser Errors remain; adopt only answers that verify better
        #[arg(long)]
        escalate: bool,
        #[arg(long, default_value = llm::ESCALATION_MODEL)]
        escalation_model: String,
        /// JSON output (default data/processed/<stem>.audit.json)
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
        /// candidate ids to run (default: all in data/reference/candidates.tsv)
        #[arg(long, value_delimiter = ',')]
        only: Vec<String>,
    },
    /// Score OCR model sets against a hand-verified gold file (only the gold pages are read).
    BenchOcr {
        /// gold/*.json
        gold: PathBuf,
        /// the PDF the gold file describes
        pdf: PathBuf,
        /// model sets to compare (see document::OcrModels::named)
        #[arg(long, value_delimiter = ',', default_value = "ppocrv5-en-mobile")]
        models: Vec<String>,
        /// print every gold line with its best match
        #[arg(long)]
        detail: bool,
    },
    /// Fetch Planning London Datahub records and save them verbatim under snapshots/pld/.
    Pld {
        #[command(subcommand)]
        cmd: PldCmd,
    },
    /// Set a development's deeds (oldest first, replayed from fixtures) against PLD delivery.
    /// Writes data/processed/<development>.reconcile.{json,md}.
    Reconcile {
        /// the executed deeds, original first, then any variations in date order
        #[arg(required = true)]
        pdfs: Vec<PathBuf>,
        /// PLD record id, e.g. Southwark-19_AP_1773; must match the snapshot's record
        #[arg(long)]
        development: String,
        /// a saved PLD application snapshot (snapshots/pld/applications/<id>/<time>.json)
        #[arg(long)]
        pld: Option<PathBuf>,
        #[arg(long, default_value = llm::DEFAULT_MODEL)]
        model: String,
    },
}

#[derive(Subcommand)]
enum PldCmd {
    /// GET <index>/_source/<id>, e.g. `applications Southwark-19_AP_1773`
    Get { index: String, id: String },
    /// POST <index>/_search with an Elasticsearch query from a JSON file
    Search {
        index: String,
        query: PathBuf,
        /// names the snapshot directory, e.g. `19_AP_1773-units`
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

fn main() -> Result<()> {
    match Cli::parse().cmd {
        Cmd::Audit {
            pdf,
            model,
            llm_mode,
            escalate,
            escalation_model,
            out,
            quiet,
        } => {
            let cfg = document::Config::from_env(&repo_root());
            let doc = document::load(&pdf, &cfg, !quiet)?;
            let client = credentials(llm_mode)?;
            let (mut agreement, info) = extract::extract(&doc, &model, llm_mode, client.as_ref())?;
            let pages = doc.pages();
            let mut results = verify::verify(&agreement, &pages);
            if escalate {
                let (merged, log) = extract::escalate(
                    &doc,
                    agreement,
                    &results,
                    &escalation_model,
                    llm_mode,
                    client.as_ref(),
                )?;
                for l in &log {
                    eprintln!("  escalation: {l}");
                }
                agreement = merged;
                results = verify::verify(&agreement, &pages);
            }
            agreement.warnings.extend(info.usage_note());
            println!("{}", report::render(&agreement, &results));
            let out = out.unwrap_or_else(|| {
                repo_root().join("data/processed").join(format!(
                    "{}.audit.json",
                    pdf.file_stem().unwrap_or_default().to_string_lossy()
                ))
            });
            std::fs::create_dir_all(out.parent().unwrap())?;
            std::fs::write(
                &out,
                serde_json::to_string_pretty(&report::AuditJson {
                    agreement: &agreement,
                    verification: &results,
                })?,
            )?;
            println!("\nJSON written to {}", out.display());
            std::process::exit(report::exit_code(&results));
        }
        Cmd::Eval {
            runs,
            model,
            replay,
            only,
        } => eval::run(
            &eval::EvalArgs {
                runs,
                model,
                replay,
                only,
            },
            &repo_root(),
        ),
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
            let root = repo_root();
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
                let doc = document::load(pdf, &cfg, true)?;
                let (agreement, _) = extract::extract(&doc, &model, Mode::Replay, None)?;
                instruments.push(reconcile::instrument_obligations(
                    &agreement,
                    Some(&doc.pages()),
                ));
            }
            let delivery = match pld_snapshot {
                Some(p) => {
                    let (snap, at) = pld::load(&repo_root(), &p)?;
                    Some(reconcile::delivery_from_pld(&snap, at)?)
                }
                None => None,
            };
            let r = reconcile::reconcile(&development, instruments, delivery)?;
            let md = reconcile::markdown(&r);
            println!("{md}");
            let dir = repo_root().join("data/processed");
            std::fs::create_dir_all(&dir)?;
            let name = pld::safe(&development);
            std::fs::write(
                dir.join(format!("{name}.reconcile.json")),
                serde_json::to_string_pretty(&r)?,
            )?;
            std::fs::write(dir.join(format!("{name}.reconcile.md")), md)?;
            Ok(())
        }
    }
}
