//! Score a backend's page output against the hand-transcribed gold set.
//!
//! * Line CER: for each gold line, the best semi-global alignment against any extracted line on
//!   the same page (free leading and trailing text in the extracted line, so a right-column line
//!   merged with its left-column term isn't penalised for the term). Reported on spaced text and on
//!   "squashed" text (whitespace removed), because some OCR engines drop most spaces.
//! * Facts: is the value present (squashed) on the gold page, and is it on any *other* page?
//! * Two-column: for each term/definition pair, does the definition's line also contain the term
//!   (columns merged into one line), and do words carry boxes that would let us split them?

use anyhow::Result;
use serde::Deserialize;

use crate::Doc;

#[derive(Deserialize)]
struct Gold {
    lines: Vec<GoldLine>,
    facts: Vec<GoldFact>,
    term_definitions: Vec<GoldTerm>,
}
#[derive(Deserialize)]
struct GoldLine {
    page: usize,
    printed: String,
    kind: String,
    text: String,
}
#[derive(Deserialize)]
struct GoldFact {
    page: usize,
    label: String,
    value: String,
}
#[derive(Deserialize)]
struct GoldTerm {
    page: usize,
    term: String,
    definition_starts: String,
}

/// Group boxes into rows: same row when vertical centres differ by < half the smaller height.
fn rows_by_geometry(lines: &[crate::LineOut]) -> Vec<String> {
    let mut items: Vec<&crate::LineOut> = lines.iter().collect();
    items.sort_by(|a, b| (a.bbox[1] + a.bbox[3]).total_cmp(&(b.bbox[1] + b.bbox[3])));
    let mut rows: Vec<Vec<&crate::LineOut>> = vec![];
    for it in items {
        let c = (it.bbox[1] + it.bbox[3]) / 2.0;
        let h = it.bbox[3] - it.bbox[1];
        match rows.last_mut() {
            Some(r) if {
                let l = r.last().unwrap();
                ((l.bbox[1] + l.bbox[3]) / 2.0 - c).abs() < 0.5 * h.min(l.bbox[3] - l.bbox[1])
            } => r.push(it),
            _ => rows.push(vec![it]),
        }
    }
    rows.into_iter()
        .filter(|r| r.len() > 1)
        .map(|mut r| {
            r.sort_by(|a, b| a.bbox[0].total_cmp(&b.bbox[0]));
            r.iter().map(|l| l.text.as_str()).collect::<Vec<_>>().join(" ")
        })
        .collect()
}

/// The repair the Python verifier applies: OCR reads "£" before a digit as f, F or ?.
fn currency_repair(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    chars
        .iter()
        .enumerate()
        .map(|(i, &c)| {
            let before_digit = chars.get(i + 1).is_some_and(|n| n.is_ascii_digit());
            let word_start = i == 0 || !chars[i - 1].is_alphanumeric();
            if matches!(c, 'f' | 'F' | '?') && before_digit && word_start { '£' } else { c }
        })
        .collect()
}

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

/// Edit distance of `pat` against the best-matching substring of `text`.
fn semi_global(pat: &[char], text: &[char]) -> usize {
    let mut prev: Vec<usize> = vec![0; text.len() + 1]; // free start anywhere in text
    for (i, &pc) in pat.iter().enumerate() {
        let mut cur = vec![i + 1; text.len() + 1];
        for (j, &tc) in text.iter().enumerate() {
            let sub = prev[j] + usize::from(pc != tc);
            cur[j + 1] = sub.min(prev[j + 1] + 1).min(cur[j] + 1);
        }
        prev = cur;
    }
    *prev.iter().min().unwrap() // free end
}

fn best_cer(gold: &str, candidates: &[String]) -> (f64, usize) {
    let g: Vec<char> = gold.chars().collect();
    candidates
        .iter()
        .enumerate()
        .map(|(i, c)| (semi_global(&g, &c.chars().collect::<Vec<_>>()) as f64 / g.len().max(1) as f64, i))
        .fold((f64::MAX, 0), |a, b| if b.0 < a.0 { b } else { a })
}

pub fn run(gold_path: &str, doc_path: &str) -> Result<()> {
    let gold: Gold = serde_json::from_str(&std::fs::read_to_string(gold_path)?)?;
    let doc: Doc = serde_json::from_str(&std::fs::read_to_string(doc_path)?)?;
    // Candidates per page: the lines as emitted, plus (when boxes exist) rows rebuilt by grouping
    // boxes whose vertical centres align. Table cells come back as separate boxes; rows don't.
    let page_lines = |p: usize| -> Vec<String> {
        let mut out = vec![];
        for pg in doc.pages.iter().filter(|pg| pg.page == p) {
            out.extend(pg.lines.iter().map(|l| l.text.clone()));
            if pg.lines.iter().any(|l| l.bbox != [0.0; 4]) {
                out.extend(rows_by_geometry(&pg.lines));
            }
        }
        out
    };
    println!("backend: {}", doc.backend);
    let ms: u128 = doc.pages.iter().map(|p| p.millis).sum();
    println!("pages: {}  total {:.1}s  ({:.2}s/page)", doc.pages.len(), ms as f64 / 1000.0,
             ms as f64 / 1000.0 / doc.pages.len().max(1) as f64);
    let has_boxes = doc.pages.iter().flat_map(|p| &p.lines).any(|l| !l.words.is_empty());
    println!("word boxes: {}", if has_boxes { "yes" } else { "no" });

    println!("\nLINES (CER spaced / squashed; lower is better)");
    let (mut sum_sp, mut sum_sq, mut n, mut exact) = (0.0, 0.0, 0, 0);
    let mut by_kind: std::collections::BTreeMap<&str, (f64, usize)> = Default::default();
    for gl in &gold.lines {
        let cands = page_lines(gl.page);
        let (sp, _) = best_cer(&norm(&gl.text), &cands.iter().map(|c| norm(c)).collect::<Vec<_>>());
        let (sq, i) = best_cer(&squash(&gl.text), &cands.iter().map(|c| squash(c)).collect::<Vec<_>>());
        let sp = if cands.is_empty() { 1.0 } else { sp };
        let sq = if cands.is_empty() { 1.0 } else { sq };
        sum_sp += sp;
        sum_sq += sq;
        n += 1;
        exact += usize::from(sq == 0.0);
        let e = by_kind.entry(gl.kind.as_str()).or_default();
        e.0 += sq;
        e.1 += 1;
        let got = cands.get(i).map(String::as_str).unwrap_or("<no text on page>");
        println!("  p{:<3} {:>5.3} {:>5.3}  {}\n              got: {}", gl.page, sp, sq, gl.text,
                 got.chars().take(110).collect::<String>());
    }
    println!("  mean CER spaced {:.3}  squashed {:.3}  exact(squashed) {exact}/{n}", sum_sp / n as f64, sum_sq / n as f64);
    for (k, (s, c)) in by_kind {
        println!("    {k:<26} squashed CER {:.3} over {c}", s / c as f64);
    }

    println!("\nFACTS (squashed substring on the gold page; raw | after the £ repair the verifier applies)");
    let (mut found_raw, mut found_rep) = (0, 0);
    for f in &gold.facts {
        let needle = squash(&f.value);
        let on = |repair: bool| -> Vec<usize> {
            doc.pages
                .iter()
                .filter(|p| {
                    // digit-only fragments (margin specks, folios) would split facts that wrap lines
                    let mut t = p.lines.iter().map(|l| l.text.clone())
                        .filter(|t| !(t.trim().len() <= 2 && t.trim().chars().all(|c| c.is_ascii_digit())))
                        .collect::<Vec<_>>();
                    t.extend(rows_by_geometry(&p.lines));
                    let t = t.join(" ");
                    squash(&if repair { currency_repair(&t) } else { t }).contains(&needle)
                })
                .map(|p| p.page)
                .collect()
        };
        let (raw, rep) = (on(false), on(true));
        found_raw += usize::from(raw.contains(&f.page));
        found_rep += usize::from(rep.contains(&f.page));
        println!("  p{:<3} {:<48} raw {:<6} repaired {:<6} {}", f.page, f.label,
                 if raw.contains(&f.page) { "ok" } else { "MISS" }, if rep.contains(&f.page) { "ok" } else { "MISS" },
                 if rep.iter().any(|&p| p != f.page) { format!("also on {:?}", rep) } else { String::new() });
    }
    println!("  facts on the correct page: raw {found_raw}/{n}  repaired {found_rep}/{n}", n = gold.facts.len());

    println!("\nPRINTED PAGE NUMBERS (PDF index is explicit in every backend; the printed number isn't)");
    let printed: std::collections::BTreeMap<usize, String> =
        gold.lines.iter().map(|l| (l.page, l.printed.clone())).collect();
    let mut ok_pp = 0;
    for (&page, want) in &printed {
        let Some(pg) = doc.pages.iter().find(|p| p.page == page) else { continue };
        let numeric = |t: &str| !t.trim().is_empty() && t.trim().len() <= 4 && t.trim().chars().all(|c| c.is_ascii_digit());
        let got = if pg.lines.iter().any(|l| l.bbox != [0.0; 4]) {
            // geometry: a numeric line in the bottom 10% of the page, rightmost (binding-mark specks sit
            // in the side margins, so position, not just "numeric", is what identifies the folio)
            pg.lines.iter()
                .filter(|l| numeric(&l.text) && l.bbox[1] > 0.88 * pg.height
                        && l.bbox[0] > 0.25 * pg.width) // folios are centred or right; specks sit in the side margins
                .max_by(|a, b| a.bbox[3].total_cmp(&b.bbox[3]))
                .map(|l| l.text.trim().to_string())
        } else {
            // text only: the last purely numeric line in emitted order
            pg.lines.iter().rev().find(|l| numeric(&l.text)).map(|l| l.text.trim().to_string())
        };
        let ok = got.as_deref() == Some(want.as_str());
        ok_pp += usize::from(ok);
        println!("  PDF p{page:<3} printed {want:<4} detected {:<6} {}", got.unwrap_or("-".into()), if ok { "ok" } else { "WRONG" });
    }
    println!("  printed page detected: {ok_pp}/{}", printed.len());

    println!("\nTWO-COLUMN definitions");
    for t in &gold.term_definitions {
        let cands = page_lines(t.page);
        let (cer, i) = best_cer(&squash(&t.definition_starts), &cands.iter().map(|c| squash(c)).collect::<Vec<_>>());
        if cands.is_empty() || cer > 0.15 {
            println!("  p{:<3} {:<48} definition line not found", t.page, t.term);
            continue;
        }
        let line = &cands[i];
        let first_term_word = squash(t.term.split_whitespace().next().unwrap_or(""));
        let merged = squash(line).find(&first_term_word).is_some_and(|pos| pos < squash(line).find("means").unwrap_or(0));
        let split_note = if !has_boxes {
            "no word boxes to split by"
        } else {
            let pg = doc.pages.iter().find(|p| p.page == t.page).unwrap();
            let l = pg.lines.iter().find(|l| &l.text == line).unwrap();
            // A column gap: the largest horizontal space between consecutive words in the line.
            let gap = l.words.windows(2).map(|w| w[1].bbox[0] - w[0].bbox[2]).fold(0.0f32, f32::max);
            if gap > 0.04 * pg.width { "columns separable by word x-gap" } else { "no column gap in this line" }
        };
        println!("  p{:<3} {:<48} {}  ({split_note})", t.page, t.term,
                 if merged { "MERGED with term" } else { "definition on its own line" });
    }
    Ok(())
}
