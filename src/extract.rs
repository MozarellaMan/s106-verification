//! Extraction: Claude reads the whole deed (as a PDF) into the wire schema, which is converted to
//! schema v2. Responses are recorded as fixtures so every audit can be replayed offline.
//!
//! Fixtures live in `fixtures/llm/<pdf-stem>/<model>.extract.v2.json` and are tied to the PDF's
//! SHA-256 and to a hash of the prompt and schema: a different deed or a changed prompt refuses to
//! replay instead of silently reusing stale answers. v1 fixtures recorded by the former Python
//! harness (`fixtures/llm-v1/`) still replay through an adapter.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::document::Document;
use crate::llm::{self, ExtractEffort, Usage};
use crate::schema::*;
use crate::verify::{CheckResult, ErrorClass, Status, verify};
use crate::wire::{
    self, PageMap, WireContributionsPart, WireExtraction, WireExtractionV1, WireHousingPart,
    WireInstrumentPart,
};

/// Whole deeds are attached up to this length (Southwark's 109 pages is ~180k input tokens).
const MAX_PAGES: usize = 300;
pub const FIXTURE_FORMAT: &str = "s106-llm-fixture/2";

const INTRO: &str = "The attached PDF is a UK planning obligation: a Section 106 agreement, unilateral undertaking, deed of variation or supplemental deed. It may be a scan.\n\nExtract:\n\n";

const RULES: &str = "\n\nRules:
- Cite pages by their position in the attached PDF (first page = 1), never by the printed page number.
- Every evidence snippet must be copied character for character from the cited page and must contain the value it supports. Keep it under 30 words. Use an empty snippet and page 0 when the deed does not state the value.
- Report only what the deed states. Do not compute, total or infer numbers. Use null or an empty string where the deed is silent.
";

const TASK_INSTRUMENT: &str = r#"Instrument: what kind of instrument this is. For a deed of variation or supplemental deed, the instrument it varies (as the deed describes it, with its date) and any limit on its scope (e.g. "in so far as it relates to the Affordable Housing Land").

Metadata: the council, the planning application reference(s), the council's legal reference if printed, the date of the deed as written, the site address, and the parties.

Variations: for a deed of variation or supplemental deed, every change it makes: amend, replace, insert or delete; the target as the deed names it (e.g. definition "Affordable Housing Units", Schedule 1 paragraph 4.3); and the new wording or value."#;

const TASK_HOUSING: &str = r#"Housing: the total number of residential units in the Development; market units; affordable units and their habitable rooms; each affordable tenure (from definitions such as "Social Rented Housing Units" or "Intermediate Housing Units") with the deed's own name for it, its units and habitable rooms; the approved affordable housing mix table if present (one row per unit type, the per-tenure sub-totals, the table total and its habitable rooms); and any stated affordable housing percentage with its basis (habitable rooms, units or floorspace). For a variation, report only housing values the variation itself states."#;

const TASK_CONTRIBUTIONS: &str = r#"Financial contributions: every defined term whose name ends in "Contribution" (singular), plus the "Administration Cost". Do not list group terms such as "Site and Development Contributions" on their own; use them to find the trigger of each member. For each contribution:
- amount: the structure (fixed sum, a maximum, a formula, a rate, or no figure), the figure, the words form exactly as written, whether it is Index Linked.
- conditionality: unconditional, or conditional when it is only payable if something happens (for example "which may be payable", a viability review finding a surplus, targets being missed). For a conditional sum, cite the words of the deed that state the condition, copied exactly.
- payment: the covenant that makes it payable, verbatim. The trigger is the clause that says when the money must be paid or becomes payable. If that covenant only refers elsewhere (e.g. "in accordance with the procedure in paragraph 3 of Schedule 11"), follow the reference and use the clause that says when payment falls due; a clause that only times an assessment, review or notice is not the trigger. If the sum is paid in instalments, give the lead-in covenant (e.g. "To pay to the Council the Contribution in the following instalments") and one entry per instalment with its share and its trigger phrase. Otherwise cite the single trigger phrase (e.g. "within 20 Working Days of", "on or before Implementation")."#;

pub fn prompt_instrument() -> String {
    format!("{INTRO}{TASK_INSTRUMENT}{RULES}")
}

pub fn prompt_housing() -> String {
    format!("{INTRO}{TASK_HOUSING}{RULES}")
}

pub fn prompt_contributions() -> String {
    format!("{INTRO}{TASK_CONTRIBUTIONS}{RULES}")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Mode {
    /// live if credentials resolve, otherwise replay
    Auto,
    /// always call the API (does not save)
    Live,
    /// call the API and save the response as a fixture
    Record,
    /// only use saved fixtures
    Replay,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Fixture {
    pub format: String,
    pub model: String,
    pub served_by: String,
    pub prompt_sha256: String,
    pub pdf_sha256: String,
    pub pages: Vec<usize>,
    pub usage: Usage,
    pub recorded_at: String,
    pub output: Value,
}

#[derive(Debug, Clone)]
pub struct Info {
    pub model: String,
    pub served_by: String,
    /// "live", "replay" or "replay-v1"
    pub source: String,
    pub usage: Option<Usage>,
    pub fixture: PathBuf,
}

impl Info {
    /// Include recorded usage in replay reports without suggesting a new API charge.
    pub fn usage_note(&self) -> Option<String> {
        let usage = self.usage.as_ref()?;
        let recorded = if self.source == "live" {
            ""
        } else {
            " when recorded"
        };
        Some(format!(
            "{}: {} in / {} out tokens, ~${:.3}{recorded}",
            self.source,
            usage.input_tokens,
            usage.output_tokens,
            llm::cost_usd(&self.model, usage),
        ))
    }
}

pub fn fixtures_root() -> PathBuf {
    std::env::var_os("S106_FIXTURES")
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures"))
}

fn stem(doc: &Document) -> String {
    doc.path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Changes to the prompt or the output schema invalidate recorded responses.
pub fn prompt_sha256() -> String {
    let mut h = Sha256::new();
    for part in [
        prompt_instrument(),
        llm::schema_for::<WireInstrumentPart>().to_string(),
        prompt_housing(),
        llm::schema_for::<WireHousingPart>().to_string(),
        prompt_contributions(),
        llm::schema_for::<WireContributionsPart>().to_string(),
    ] {
        h.update(part.as_bytes());
    }
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

pub fn fixture_path(doc: &Document, model: &str) -> PathBuf {
    fixtures_root()
        .join("llm")
        .join(stem(doc))
        .join(format!("{model}.extract.v2.json"))
}

fn v1_fixture_path(doc: &Document, model: &str) -> PathBuf {
    fixtures_root()
        .join("llm-v1")
        .join(stem(doc))
        .join(format!("{model}.extract.json"))
}

/// The model's answer for this deed: from a fixture, or live (and saved when recording).
fn answer(
    doc: &Document,
    model: &str,
    mode: Mode,
    client: Option<&rust_claude_sdk::Client>,
) -> Result<(Value, PageMap, Info)> {
    let path = fixture_path(doc, model);
    let pages: Vec<usize> = (1..=doc.page_count()).collect();
    let live = match mode {
        Mode::Live | Mode::Record => true,
        Mode::Replay => false,
        Mode::Auto => client.is_some(),
    };
    if live {
        if doc.page_count() > MAX_PAGES {
            bail!(
                "{} has {} pages; whole-deed extraction is capped at {MAX_PAGES}",
                doc.path.display(),
                doc.page_count()
            );
        }
        let client = client.context("live extraction needs Anthropic credentials")?;
        let pdf = std::fs::read(&doc.path)?;
        // Three calls over the same PDF: one schema for everything compiles to a grammar
        // the API rejects as too large (see wire.rs).
        let call = |prompt: String, schema: Value| {
            llm::extract(client, model, &pdf, &prompt, &schema, ExtractEffort::Medium)
        };
        let i = call(prompt_instrument(), llm::schema_for::<WireInstrumentPart>())?;
        let mut h = call(prompt_housing(), llm::schema_for::<WireHousingPart>())?;
        let mut c = call(
            prompt_contributions(),
            llm::schema_for::<WireContributionsPart>(),
        )?;
        let mut output = i.output;
        output["housing"] = h.output["housing"].take();
        output["contributions"] = c.output["contributions"].take();
        let mut usage = i.usage;
        usage.add(&h.usage);
        usage.add(&c.usage);
        let r = llm::Response {
            output,
            served_by: i.served_by,
            usage,
        };
        if mode == Mode::Record {
            let fx = Fixture {
                format: FIXTURE_FORMAT.into(),
                model: model.into(),
                served_by: r.served_by.clone(),
                prompt_sha256: prompt_sha256(),
                pdf_sha256: doc.sha256.clone(),
                pages: pages.clone(),
                usage: r.usage.clone(),
                recorded_at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                output: r.output.clone(),
            };
            std::fs::create_dir_all(path.parent().unwrap())?;
            std::fs::write(&path, serde_json::to_string_pretty(&fx)? + "\n")?;
        }
        let info = Info {
            model: model.into(),
            served_by: r.served_by,
            source: "live".into(),
            usage: Some(r.usage),
            fixture: path,
        };
        return Ok((r.output, PageMap(pages), info));
    }
    if path.exists() {
        let fx: Fixture = serde_json::from_str(&std::fs::read_to_string(&path)?)
            .with_context(|| format!("parse {}", path.display()))?;
        if fx.pdf_sha256 != doc.sha256 {
            bail!(
                "{} was recorded from a different PDF (sha256 mismatch); re-record it",
                path.display()
            );
        }
        if fx.prompt_sha256 != prompt_sha256() {
            bail!(
                "{} was recorded with a different prompt or schema; re-record it (--llm-mode record)",
                path.display()
            );
        }
        let info = Info {
            model: fx.model,
            served_by: fx.served_by,
            source: "replay".into(),
            usage: Some(fx.usage),
            fixture: path,
        };
        return Ok((fx.output, PageMap(fx.pages), info));
    }
    let v1 = v1_fixture_path(doc, model);
    if v1.exists() {
        let raw: Value = serde_json::from_str(&std::fs::read_to_string(&v1)?)?;
        if raw["pdf_sha256"].as_str() != Some(doc.sha256.as_str()) {
            bail!(
                "{} was recorded from a different PDF (sha256 mismatch)",
                v1.display()
            );
        }
        let pages: Vec<usize> = serde_json::from_value(raw["pages"].clone())?;
        let usage = raw["usage"].as_object().map(|u| Usage {
            input_tokens: u["input_tokens"].as_u64().unwrap_or(0),
            output_tokens: u["output_tokens"].as_u64().unwrap_or(0),
            attempts: u.get("attempts").and_then(Value::as_u64).unwrap_or(1) as u32,
            cache_write_tokens: 0,
            cache_read_tokens: 0,
        });
        let info = Info {
            model: model.into(),
            served_by: raw["served_by"].as_str().unwrap_or(model).into(),
            source: "replay-v1".into(),
            usage,
            fixture: v1,
        };
        return Ok((raw["output"].clone(), PageMap(pages), info));
    }
    bail!(
        "no recorded response for {} ({}); record one with --llm-mode record",
        stem(doc),
        path.display()
    )
}

/// Wire output (v2, or v1 from an old fixture) -> schema v2, with the deterministic clean-ups.
pub fn to_agreement(
    doc: &Document,
    output: &Value,
    pm: &PageMap,
    info: &Info,
) -> Result<Agreement> {
    let mut warnings = vec![];
    let (instrument, metadata, housing, contributions, variations) = if info.source == "replay-v1" {
        let w: WireExtractionV1 =
            serde_path_to_error::deserialize(output).context("v1 fixture output")?;
        warnings.push(
            "v1 fixture: instrument, metadata, conditions and instalments were not extracted"
                .into(),
        );
        let cs = w
            .contributions
            .iter()
            .filter(|c| !wire::is_group_term(&c.name))
            .map(|c| wire::contribution_v1(c, pm))
            .collect();
        (
            Instrument::default(),
            Metadata::default(),
            wire::housing(&w.housing, pm),
            cs,
            vec![],
        )
    } else {
        let w: WireExtraction = serde_path_to_error::deserialize(output)
            .context("fixture output does not match the v2 wire schema")?;
        let groups: Vec<&str> = w
            .contributions
            .iter()
            .filter(|c| wire::is_group_term(&c.name))
            .map(|c| c.name.as_str())
            .collect();
        if !groups.is_empty() {
            warnings.push(format!(
                "dropped group term(s) listed as contributions: {}",
                groups.join(", ")
            ));
        }
        let cs = w
            .contributions
            .iter()
            .filter(|c| !wire::is_group_term(&c.name))
            .map(|c| wire::contribution(c, pm))
            .collect();
        (
            wire::instrument(&w.instrument, pm),
            wire::metadata(&w.metadata),
            wire::housing(&w.housing, pm),
            cs,
            w.variations
                .iter()
                .map(|v| wire::variation(v, pm))
                .collect(),
        )
    };
    if doc.text_source() != "text-layer" {
        warnings.push(
            "pages without a text layer were OCR'd; evidence matching is whitespace-insensitive"
                .into(),
        );
    }
    Ok(Agreement {
        schema_version: SCHEMA_VERSION.into(),
        provenance: Provenance {
            source_path: doc.path.display().to_string(),
            sha256: doc.sha256.clone(),
            page_count: doc.page_count(),
            text_source: doc.text_source().into(),
            ocr_version: Some(doc.ocr_version.clone()),
            extracted_by: format!(
                "{} ({}, {})",
                info.served_by,
                info.source,
                info.fixture.display()
            ),
        },
        instrument,
        metadata,
        housing,
        contributions,
        variations,
        warnings,
    })
}

pub fn extract(
    doc: &Document,
    model: &str,
    mode: Mode,
    client: Option<&rust_claude_sdk::Client>,
) -> Result<(Agreement, Info)> {
    let (output, pm, info) = answer(doc, model, mode, client)?;
    Ok((to_agreement(doc, &output, &pm, &info)?, info))
}

// ------------------------------------------------------------------------------- escalation

/// Which part of the agreement a check belongs to: "housing", "instrument" or a contribution name.
fn field_of<'a>(a: &'a Agreement, r: &CheckResult) -> &'a str {
    if let Some(rest) = r.check_id.strip_prefix('F') {
        let n: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        if let Some(c) = n
            .parse::<usize>()
            .ok()
            .and_then(|i| a.contributions.get(i - 1))
        {
            return &c.name;
        }
    }
    if r.check_id.starts_with('V') {
        "instrument"
    } else {
        "housing"
    }
}

fn tally(a: &Agreement, results: &[CheckResult], field: &str) -> (usize, usize) {
    let mine = results.iter().filter(|r| field_of(a, r) == field);
    let (mut parser, mut run) = (0, 0);
    for r in mine {
        parser += usize::from(r.error_class == Some(ErrorClass::ParserError));
        run += usize::from(r.status != Status::Skip);
    }
    (parser, run)
}

/// Values present in `old` that `new` lost (a replacement must not drop data to dodge a check).
fn lost_contribution(old: &Contribution, new: &Contribution) -> Vec<&'static str> {
    let mut lost = vec![];
    if old.amount.value_gbp.is_some() && new.amount.value_gbp.is_none() {
        lost.push("amount");
    }
    if old.amount.words.is_some() && new.amount.words.is_none() {
        lost.push("words");
    }
    if old.payment.trigger_phrase.is_some()
        && new.payment.trigger_phrase.is_none()
        && new.payment.instalments.is_empty()
    {
        lost.push("trigger");
    }
    if old.payment.covenant.is_some() && new.payment.covenant.is_none() {
        lost.push("covenant");
    }
    lost
}

fn lost_housing(old: &Housing, new: &Housing) -> bool {
    let pairs = [
        (old.total_units, new.total_units),
        (old.market_units, new.market_units),
        (old.affordable_units, new.affordable_units),
        (
            old.affordable_habitable_rooms,
            new.affordable_habitable_rooms,
        ),
    ];
    pairs.iter().any(|(o, n)| o.is_some() && n.is_none())
        || new.tenures.len() < old.tenures.len()
        || (old.mix_table.is_some() && new.mix_table.is_none())
}

/// Re-read the deed with a stronger model and adopt, field by field, only answers that verify
/// better: fewer Parser Errors, at least as many checks run, and no value lost.
pub fn escalate(
    doc: &Document,
    current: Agreement,
    results: &[CheckResult],
    model: &str,
    mode: Mode,
    client: Option<&rust_claude_sdk::Client>,
) -> Result<(Agreement, Vec<String>)> {
    let failing: Vec<String> = {
        let mut f: Vec<String> = results
            .iter()
            .filter(|r| r.error_class == Some(ErrorClass::ParserError))
            .map(|r| field_of(&current, r).to_string())
            .collect();
        f.sort();
        f.dedup();
        f
    };
    if failing.is_empty() {
        return Ok((current, vec![]));
    }
    let (candidate, _) = extract(doc, model, mode, client)?;
    let pages = doc.pages();
    let cand_results = verify(&candidate, &pages);
    let mut merged = current.clone();
    let mut log = vec![];
    for field in &failing {
        let (p0, r0) = tally(&current, results, field);
        let (p1, r1) = tally(&candidate, &cand_results, field);
        let mut why = vec![];
        if p1 >= p0 {
            why.push("no fewer Parser Errors".to_string());
        }
        if r1 < r0 {
            why.push("fewer checks run".into());
        }
        let replace = match field.as_str() {
            "housing" => {
                if lost_housing(&current.housing, &candidate.housing) {
                    why.push("lost housing values".into());
                }
                why.is_empty()
                    .then(|| merged.housing = candidate.housing.clone())
            }
            "instrument" => why.is_empty().then(|| {
                merged.instrument = candidate.instrument.clone();
                merged.variations = candidate.variations.clone();
            }),
            name => match (
                current.contributions.iter().position(|c| c.name == *name),
                candidate.contributions.iter().find(|c| c.name == *name),
            ) {
                (Some(i), Some(new)) => {
                    let lost = lost_contribution(&current.contributions[i], new);
                    if !lost.is_empty() {
                        why.push(format!("lost {}", lost.join(", ")));
                    }
                    why.is_empty()
                        .then(|| merged.contributions[i] = new.clone())
                }
                _ => {
                    why.push("not found in the escalated answer".into());
                    None
                }
            },
        };
        log.push(match replace {
            Some(()) => format!("{field}: replaced with {model} ({p0} -> {p1} Parser Errors)"),
            None => format!("{field}: kept ({})", why.join("; ")),
        });
    }
    merged
        .warnings
        .push(format!("escalation to {model}: {}", log.join(" | ")));
    Ok((merged, log))
}
