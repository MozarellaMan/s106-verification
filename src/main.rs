//! s106 command line. Run `s106 --help`.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};

use s106::document::{self, Snapshot};
use s106::{explore, figures, labels, pld, results};

#[derive(Parser)]
#[command(
    name = "s106",
    about = "Research tools for UK Section 106 planning agreements"
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
    /// Fetch Planning London Datahub records and save them verbatim under <data>/snapshots/pld/.
    Pld {
        #[command(subcommand)]
        cmd: PldCmd,
    },
    /// Score the agent proposals and the sources against whatever human labels are in <data>/labels/.
    /// Writes <data>/reports/labels/labels.{md,json}; with no labels it says so and succeeds.
    Labels,
    /// Build a batch's results: its scheme ledger, its decision rules and its claims, each with
    /// the check level behind it, from the labels and proposals that exist. Writes
    /// <data>/<out>/{README.md,ledger.csv,rules.csv,retrieval.csv}; reads <data>/<out>/claims.csv.
    Results {
        /// the batch's blank labelling sheet, relative to <data>
        #[arg(long)]
        sheet: PathBuf,
        /// the results folder, relative to <data>
        #[arg(long)]
        out: PathBuf,
        /// leave out the decision rules (batch 1's), for a batch that pre-registered none
        #[arg(long)]
        no_rules: bool,
    },
    /// Check every figure cited as `29.8%<!-- src: path -->` in the markdown under <dir>: the cited
    /// file (relative to <dir>) must exist and still contain the figure. Exit 1 if any fails.
    CheckFigures {
        /// the folder to check (default: the data root)
        dir: Option<PathBuf>,
    },
    /// Bake the results explorer's data into <out>/sites.json and <out>/boroughs.geojson: Datahub
    /// figures only, unless a flag below adds ledger cells. Paths are relative to the data root.
    /// With --results the build is private: it also carries a results ledger's unreviewed
    /// readings. View it locally; never publish it. With --deed-cells it adds every deed cell for
    /// the obligation, variation and completion stages and every variation-document cell for the
    /// variation stage, each labelled with its check level, which the public site may carry.
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
        /// a results folder (e.g. results/2026-10-14-batch1) whose ledger readings to add
        #[arg(long, conflicts_with = "deed_cells")]
        results: Option<PathBuf>,
        /// a results folder whose deed and variation-document cells (only, each with its check
        /// level) to add, for the public site
        #[arg(long)]
        deed_cells: Option<PathBuf>,
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

fn main() -> Result<()> {
    let cli = Cli::parse();
    if let Some(dir) = cli.data {
        // SAFETY: set once at startup, before any thread is spawned or the variable is read
        unsafe { std::env::set_var("S106_DATA", dir) };
    }
    match cli.cmd {
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
        Cmd::Explore {
            applications,
            snapshot,
            boundaries,
            results,
            deed_cells,
            out,
        } => {
            let root = s106::data::root();
            let inputs = explore::Inputs {
                applications,
                snapshot,
                boundaries,
                results,
                deed_cells,
            };
            let bundle = explore::bake(&root, &inputs)?;
            std::fs::create_dir_all(&out)?;
            std::fs::write(out.join("sites.json"), serde_json::to_string(&bundle)?)?;
            std::fs::copy(root.join(&inputs.boundaries), out.join("boroughs.geojson"))
                .with_context(|| format!("copying {}", inputs.boundaries.display()))?;
            eprintln!(
                "wrote {} sites ({} without a centroid left out) to {}",
                bundle.sites.len(),
                bundle.sources.without_centroid,
                out.display()
            );
            if let Some(results) = &bundle.sources.results {
                let read = bundle.sites.iter().filter(|s| !s.readings.is_empty());
                eprintln!(
                    "PRIVATE build: {} schemes carry readings from {} at data commit {} \
                     (not matched: {:?}). View it locally; never publish it.",
                    read.count(),
                    results,
                    bundle.sources.data_commit.as_deref().unwrap_or("unknown"),
                    bundle.sources.unmatched
                );
            }
            if let Some(from) = &bundle.sources.deed_cells {
                let cells: usize = bundle.sites.iter().map(|s| s.readings.len()).sum();
                let read = bundle.sites.iter().filter(|s| !s.readings.is_empty());
                eprintln!(
                    "{cells} deed cells on {} schemes from {from} (not matched: {:?})",
                    read.count(),
                    bundle.sources.unmatched
                );
            }
            Ok(())
        }
        Cmd::Labels => {
            let root = s106::data::root();
            let r = labels::run(&root)?;
            println!("{}", labels::markdown(&r));
            eprintln!(
                "wrote {}",
                root.join("reports/labels/labels.{md,json}").display()
            );
            Ok(())
        }
        Cmd::CheckFigures { dir } => {
            let dir = dir.unwrap_or_else(s106::data::root);
            let r = figures::check(&dir)?;
            for p in &r.problems {
                println!("{p}");
            }
            println!("{} cited figures, {} failed", r.cited, r.problems.len());
            if !r.problems.is_empty() {
                std::process::exit(1);
            }
            Ok(())
        }
        Cmd::Results {
            sheet,
            out,
            no_rules,
        } => {
            let root = s106::data::root();
            let r = results::run(&root, &sheet, &out, !no_rules)?;
            for rule in &r.rules {
                println!(
                    "rule {}: {} ({} of {} counted, {} needed)",
                    rule.id,
                    rule.verdict(),
                    rule.counts,
                    rule.counted,
                    rule.needed
                );
            }
            eprintln!("wrote {}", root.join(&out).display());
            Ok(())
        }
    }
}
