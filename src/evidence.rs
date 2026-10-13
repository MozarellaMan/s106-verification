//! Re-locating a cited snippet in the page text, robust to OCR (dropped spaces, two-column
//! interleaving at line wraps, a few garbled plain words) without accepting changed numbers.
//! Port of `evidence_found` / `_layout_match` from the former `s106-audit/verifier.py`.

use std::collections::{BTreeMap, HashSet};
use std::sync::LazyLock;

use regex::Regex;

use crate::schema::Evidence;
use crate::text::{NUMBER_WORDS, ocr_digits, squash};

/// 1-based PDF page index -> page text (lines separated by '\n').
pub type Pages = BTreeMap<usize, String>;

// Fallback when the squashed snippet is not contiguous in the OCR text (two-column
// definition pages splice the left-column term into the sentence at line wraps).
// Tokens must appear in order; intervening text is allowed only as
//   * at most 1 non-digit char within a line (OCR reads "£" as "f"), or
//   * a line wrap followed by up to MAX_GAP chars of left-column fragment,
// and no gap may hold digits or number words, even joined ("andtwenty"). Numbers
// are digit-bounded and never skipped; up to MAX_SKIP_SHARE of plain words may be.
/// Chars of left-column fragment allowed at a line start.
const MAX_GAP: usize = 40;
/// Shorter snippets must match contiguously.
const MIN_TOKENS: usize = 4;
/// Share of plain words OCR may have garbled ("fifty" -> "fty").
const MAX_SKIP_SHARE: f64 = 0.1;

static TOKEN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[a-z]+|[0-9]+(?:[.,][0-9]+)*").unwrap());

/// Words and numbers; '£12,650.00' -> '1265000', the same digits squash() keeps.
fn tokens(s: &str) -> Vec<String> {
    TOKEN
        .find_iter(&s.to_lowercase())
        .map(|m| m.as_str().chars().filter(|c| !matches!(c, '.' | ',')).collect())
        .collect()
}

fn is_digits(tok: &str) -> bool {
    !tok.is_empty() && tok.bytes().all(|b| b.is_ascii_digit())
}

fn is_number(tok: &str) -> bool {
    is_digits(tok) || NUMBER_WORDS.contains(&tok)
}

// "one", "two", "ten" also sit inside common words (network, maintenance), so they
// count at a word's start or end or after "and". A skipped gap stops where matching
// resumes, so an amount word OCR joined to a column fragment ("Contributionandten
// pounds") ends its gap word; this rejects some terms ending in them ("Zone").
const SHORT_NUMS: [&str; 3] = ["one", "two", "ten"];
static NUM_WORD: LazyLock<Regex> = LazyLock::new(|| {
    let words: Vec<&str> = NUMBER_WORDS.iter().copied().filter(|w| !SHORT_NUMS.contains(w)).collect();
    Regex::new(&words.join("|")).unwrap()
});
static SHORT_NUM_EDGE: LazyLock<Regex> = LazyLock::new(|| {
    let short = SHORT_NUMS.join("|");
    Regex::new(&format!("(?:^|and)(?:{short})|(?:{short})$")).unwrap()
});
static LETTERS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[a-z]+").unwrap());

/// Could this skipped OCR text carry part of an amount the snippet left out?
fn numeric_gap(text: &str) -> bool {
    text.chars().any(|c| c.is_ascii_digit())
        || LETTERS
            .find_iter(text)
            .any(|w| NUM_WORD.is_match(w.as_str()) || SHORT_NUM_EDGE.is_match(w.as_str()))
}

struct Layout<'a> {
    raw: Vec<char>,
    /// Char index in `raw` of each char of `hay`.
    idx: Vec<usize>,
    /// == squash(page)
    hay: Vec<u8>,
    tokens: &'a [String],
    max_skips: usize,
    seen: HashSet<(usize, usize, usize, usize)>,
}

impl Layout<'_> {
    fn gap_ok(&self, end: usize, start: usize, extra: usize) -> bool {
        let gap: String = self.raw[self.idx[end - 1] + 1..self.idx[start]].iter().collect();
        let lines: Vec<&str> = gap.split('\n').collect();
        if numeric_gap(&gap) || squash(lines[0]).len() > 1 + extra {
            return false;
        }
        lines.len() == 1
            || (!lines[1..lines.len() - 1].iter().any(|l| !squash(l).is_empty())
                && squash(lines[lines.len() - 1]).len() <= MAX_GAP + extra)
    }

    /// Where `tok` occurs within hay[lo..hi]; numbers only when not part of a longer digit run.
    fn occurrences(&self, tok: &str, lo: usize, hi: usize) -> Vec<(usize, usize)> {
        let t = tok.as_bytes();
        let hi = hi.min(self.hay.len());
        let digits = is_digits(tok);
        let mut out = Vec::new();
        let mut i = lo;
        while i + t.len() <= hi {
            if &self.hay[i..i + t.len()] == t {
                let end = i + t.len();
                if !digits
                    || ((i == 0 || !self.hay[i - 1].is_ascii_digit())
                        && (end == self.hay.len() || !self.hay[end].is_ascii_digit()))
                {
                    out.push((i, end));
                }
            }
            i += 1;
        }
        out
    }

    /// `pos`: end of the last matched token (None before the first match);
    /// `extra`: chars of slack owed to skipped (OCR-garbled) words since then.
    fn search(&mut self, i: usize, pos: Option<usize>, extra: usize, skips: usize) -> bool {
        let tokens = self.tokens;
        if i == tokens.len() {
            return pos.is_some();
        }
        if !self.seen.insert((i, pos.unwrap_or(usize::MAX), extra, skips)) {
            return false;
        }
        let tok = &tokens[i];
        let ends: Vec<usize> = match pos {
            None => self.occurrences(tok, 0, self.hay.len()).into_iter().map(|(_, e)| e).collect(),
            Some(p) => {
                let hi = p + MAX_GAP + 2 * extra + 2 + tok.len();
                self.occurrences(tok, p, hi)
                    .into_iter()
                    .filter(|&(s, _)| self.gap_ok(p, s, extra))
                    .map(|(_, e)| e)
                    .collect()
            }
        };
        if ends.into_iter().any(|e| self.search(i + 1, Some(e), 0, skips)) {
            return true;
        }
        !is_number(tok) && skips < self.max_skips && self.search(i + 1, pos, extra + tok.len() + 1, skips + 1)
    }
}

fn layout_match(tokens: &[String], page: &str) -> bool {
    let raw: Vec<char> = page.to_lowercase().chars().collect();
    let idx: Vec<usize> = (0..raw.len()).filter(|&i| raw[i].is_ascii_alphanumeric()).collect();
    let hay: Vec<u8> = idx.iter().map(|&i| raw[i] as u8).collect();
    let plain = tokens.iter().filter(|t| !is_number(t)).count();
    let mut layout = Layout {
        raw,
        idx,
        hay,
        tokens,
        max_skips: (plain as f64 * MAX_SKIP_SHARE) as usize,
        seen: HashSet::new(),
    };
    layout.search(0, None, 0, 0)
}

/// Is `ev`'s snippet present on its cited page?
pub fn evidence_found(ev: Option<&Evidence>, pages: &Pages) -> bool {
    let Some(ev) = ev else { return false };
    let Some(page) = pages.get(&ev.page) else { return false };
    let needle = squash(&ev.snippet);
    if needle.is_empty() {
        return false;
    }
    let page = ocr_digits(page);
    if squash(&page).contains(&needle) {
        return true;
    }
    let tokens = tokens(&ev.snippet);
    tokens.len() >= MIN_TOKENS && layout_match(&tokens, &page)
}
