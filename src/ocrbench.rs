//! Score page text against a hand-transcribed gold set (`gold/*.json` in the data repository).
//!
//! * Line CER: for each gold line, the best semi-global alignment against any line on the same
//!   page, or any row rebuilt from boxes whose vertical centres align (table cells come back as
//!   separate boxes). Reported on spaced text and on squashed text (whitespace removed).
//! * Facts: is the value on the gold page (raw, and after the `£` repair the verifier applies)?
//! * Printed page numbers: a short numeric line in the bottom margin, away from the side margins.
//! * Two-column: is each gold definition on its own line, or merged with its left-column term?
//!
//! The gold sets are small; differences of a line or a fact are within noise.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::document::{Document, Line};

#[derive(Deserialize)]
pub struct Gold {
    pub document: String,
    pub sha256: String,
    pub lines: Vec<GoldLine>,
    pub facts: Vec<GoldFact>,
    #[serde(default)]
    pub term_definitions: Vec<GoldTerm>,
}
#[derive(Deserialize)]
pub struct GoldLine {
    pub page: usize,
    pub printed: String,
    pub kind: String,
    pub text: String,
}
#[derive(Deserialize)]
pub struct GoldFact {
    pub page: usize,
    pub label: String,
    pub value: String,
}
#[derive(Deserialize)]
pub struct GoldTerm {
    pub page: usize,
    pub term: String,
    pub definition_starts: String,
}

impl Gold {
    pub fn load(path: &Path) -> Result<Gold> {
        Ok(serde_json::from_str(&std::fs::read_to_string(path)?)?)
    }
    pub fn pages(&self) -> Vec<usize> {
        let mut p: Vec<usize> = self
            .lines
            .iter()
            .map(|l| l.page)
            .chain(self.facts.iter().map(|f| f.page))
            .collect();
        p.sort();
        p.dedup();
        p
    }
}

#[derive(Debug, Serialize)]
pub struct Score {
    pub mean_cer_spaced: f64,
    pub mean_cer_squashed: f64,
    pub exact_lines: usize,
    pub lines: usize,
    pub facts_raw: usize,
    pub facts_repaired: usize,
    pub facts: usize,
    pub printed_found: usize,
    pub printed: usize,
    pub definitions_on_own_line: usize,
    pub definitions: usize,
    /// one row per gold line: (page, cer_spaced, cer_squashed, gold, best match)
    pub detail: Vec<(usize, f64, f64, String, String)>,
}

// Not `text::squash`: that keeps ASCII alphanumerics only (drops "£" and punctuation); the OCR
// benchmark measures CER on everything but whitespace, after folding typographic quotes and dashes.
fn norm(s: &str) -> String {
    let s = s
        .replace(['“', '”', '„'], "\"")
        .replace(['‘', '’'], "'")
        .replace(['–', '—'], "-")
        .to_lowercase();
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn squash(s: &str) -> String {
    norm(s).chars().filter(|c| !c.is_whitespace()).collect()
}

/// The repair the verifier applies: OCR reads "£" before a digit as f, F or ?.
fn currency_repair(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    (0..chars.len())
        .map(|i| {
            let c = chars[i];
            let before_digit = chars.get(i + 1).is_some_and(|n| n.is_ascii_digit());
            let word_start = i == 0 || !chars[i - 1].is_alphanumeric();
            if matches!(c, 'f' | 'F' | '?') && before_digit && word_start {
                '£'
            } else {
                c
            }
        })
        .collect()
}

/// Edit distance of `pat` against the best-matching substring of `text`.
fn semi_global(pat: &[char], text: &[char]) -> usize {
    let mut prev = vec![0usize; text.len() + 1];
    for (i, &pc) in pat.iter().enumerate() {
        let mut cur = vec![i + 1; text.len() + 1];
        for (j, &tc) in text.iter().enumerate() {
            cur[j + 1] = (prev[j] + usize::from(pc != tc))
                .min(prev[j + 1] + 1)
                .min(cur[j] + 1);
        }
        prev = cur;
    }
    *prev.iter().min().unwrap()
}

fn best_cer(gold: &str, candidates: &[String]) -> (f64, usize) {
    let g: Vec<char> = gold.chars().collect();
    candidates
        .iter()
        .enumerate()
        .map(|(i, c)| {
            (
                semi_global(&g, &c.chars().collect::<Vec<_>>()) as f64 / g.len().max(1) as f64,
                i,
            )
        })
        .fold((1.0, 0), |a, b| if b.0 < a.0 { b } else { a })
}

/// Group boxes into rows: same row when vertical centres differ by < half the smaller height.
/// Deliberately not shared with `document::page_text`: this sorts by centre and compares against the
/// last box only, and keeps multi-box rows only (it measures table-cell rebuilding); page_text keeps
/// every row and compares against any box in the row, over lines already in row order.
fn rows_by_geometry(lines: &[Line]) -> Vec<String> {
    let mut items: Vec<&Line> = lines.iter().collect();
    items.sort_by(|a, b| (a.bbox[1] + a.bbox[3]).total_cmp(&(b.bbox[1] + b.bbox[3])));
    let mut rows: Vec<Vec<&Line>> = vec![];
    for it in items {
        let c = (it.bbox[1] + it.bbox[3]) / 2.0;
        let h = it.bbox[3] - it.bbox[1];
        let same = rows.last().is_some_and(|r| {
            let l = r.last().unwrap();
            ((l.bbox[1] + l.bbox[3]) / 2.0 - c).abs() < 0.5 * h.min(l.bbox[3] - l.bbox[1])
        });
        if same {
            rows.last_mut().unwrap().push(it)
        } else {
            rows.push(vec![it])
        }
    }
    rows.into_iter()
        .filter(|r| r.len() > 1)
        .map(|mut r| {
            r.sort_by(|a, b| a.bbox[0].total_cmp(&b.bbox[0]));
            r.iter()
                .map(|l| l.text.as_str())
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect()
}

pub fn score(gold: &Gold, doc: &Document) -> Score {
    let page = |n: usize| doc.pages.iter().find(|p| p.page == n);
    let candidates = |n: usize| -> Vec<String> {
        page(n).map_or(vec![], |p| {
            let mut c: Vec<String> = p.lines.iter().map(|l| l.text.clone()).collect();
            c.extend(rows_by_geometry(&p.lines));
            c
        })
    };
    let mut s = Score {
        mean_cer_spaced: 0.0,
        mean_cer_squashed: 0.0,
        exact_lines: 0,
        lines: gold.lines.len(),
        facts_raw: 0,
        facts_repaired: 0,
        facts: gold.facts.len(),
        printed_found: 0,
        printed: 0,
        definitions_on_own_line: 0,
        definitions: gold.term_definitions.len(),
        detail: vec![],
    };
    for gl in &gold.lines {
        let cands = candidates(gl.page);
        let (sp, _) = best_cer(
            &norm(&gl.text),
            &cands.iter().map(|c| norm(c)).collect::<Vec<_>>(),
        );
        let (sq, i) = best_cer(
            &squash(&gl.text),
            &cands.iter().map(|c| squash(c)).collect::<Vec<_>>(),
        );
        s.mean_cer_spaced += sp;
        s.mean_cer_squashed += sq;
        s.exact_lines += usize::from(sq == 0.0);
        s.detail.push((
            gl.page,
            sp,
            sq,
            gl.text.clone(),
            cands.get(i).cloned().unwrap_or_default(),
        ));
    }
    let n = gold.lines.len().max(1) as f64;
    s.mean_cer_spaced /= n;
    s.mean_cer_squashed /= n;

    for f in &gold.facts {
        let needle = squash(&f.value);
        let text = |repair: bool| -> String {
            page(f.page).map_or(String::new(), |p| {
                // digit-only fragments (margin specks, folios) would split facts that wrap lines
                let mut t: Vec<String> = p
                    .lines
                    .iter()
                    .map(|l| l.text.clone())
                    .filter(|t| {
                        !(t.trim().len() <= 2 && t.trim().chars().all(|c| c.is_ascii_digit()))
                    })
                    .collect();
                t.extend(rows_by_geometry(&p.lines));
                let t = t.join(" ");
                squash(&if repair { currency_repair(&t) } else { t })
            })
        };
        s.facts_raw += usize::from(text(false).contains(&needle));
        s.facts_repaired += usize::from(text(true).contains(&needle));
    }

    let printed: BTreeMap<usize, &str> = gold
        .lines
        .iter()
        .map(|l| (l.page, l.printed.as_str()))
        .collect();
    s.printed = printed.len();
    for (&n, want) in &printed {
        let Some(p) = page(n) else { continue };
        let numeric = |t: &str| {
            !t.trim().is_empty()
                && t.trim().len() <= 4
                && t.trim().chars().all(|c| c.is_ascii_digit())
        };
        let got = p
            .lines
            .iter()
            .filter(|l| {
                numeric(&l.text) && l.bbox[1] > 0.88 * p.height && l.bbox[0] > 0.25 * p.width
            })
            .max_by(|a, b| a.bbox[3].total_cmp(&b.bbox[3]))
            .map(|l| l.text.trim().to_string());
        s.printed_found += usize::from(got.as_deref() == Some(*want));
    }

    for t in &gold.term_definitions {
        let lines: Vec<String> =
            page(t.page).map_or(vec![], |p| p.lines.iter().map(|l| l.text.clone()).collect());
        let (cer, i) = best_cer(
            &squash(&t.definition_starts),
            &lines.iter().map(|c| squash(c)).collect::<Vec<_>>(),
        );
        if lines.is_empty() || cer > 0.15 {
            continue;
        }
        let line = squash(&lines[i]);
        let first = squash(t.term.split_whitespace().next().unwrap_or(""));
        let merged = line
            .find(&first)
            .is_some_and(|pos| pos < line.find("means").unwrap_or(0));
        s.definitions_on_own_line += usize::from(!merged);
    }
    s
}

impl std::fmt::Display for Score {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(
            f,
            "lines: CER spaced {:.3}  squashed {:.3}  exact {}/{}",
            self.mean_cer_spaced, self.mean_cer_squashed, self.exact_lines, self.lines
        )?;
        writeln!(
            f,
            "facts on the gold page: raw {}/{}  after £ repair {}/{}",
            self.facts_raw, self.facts, self.facts_repaired, self.facts
        )?;
        writeln!(
            f,
            "printed page numbers: {}/{}",
            self.printed_found, self.printed
        )?;
        write!(
            f,
            "two-column definitions on their own line: {}/{}",
            self.definitions_on_own_line, self.definitions
        )
    }
}
