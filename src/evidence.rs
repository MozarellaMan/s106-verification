//! Re-locating a cited snippet in the page text, robust to OCR (dropped spaces, two-column
//! interleaving at line wraps, a few garbled plain words) without accepting changed numbers.

use std::collections::{BTreeMap, HashSet};
use std::ops::Range;
use std::sync::LazyLock;

use regex::Regex;
use serde::Serialize;

use crate::quantity::NUMBER_WORDS;
use crate::schema::Evidence;
use crate::text::{ocr_digits, squash};

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
/// Share of non-number words OCR may have garbled ("the" -> "thie").
const MAX_SKIP_SHARE: f64 = 0.1;

static TOKEN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[a-z]+|[0-9]+(?:[.,][0-9]+)*").unwrap());

/// Words and numbers; ignore grouping commas, but preserve decimal points.
fn tokens(s: &str) -> Vec<String> {
    TOKEN
        .find_iter(&s.to_lowercase())
        .map(|m| m.as_str().chars().filter(|c| *c != ',').collect())
        .collect()
}

fn is_numeric_token(tok: &str) -> bool {
    tok.bytes().any(|b| b.is_ascii_digit()) && tok.bytes().all(|b| b.is_ascii_digit() || b == b'.')
}

// Whitespace and currency punctuation may vary in OCR; decimal position may not.
// Used by both matching paths so the contiguous shortcut cannot weaken the fallback.
fn matching_char(chars: &[char], i: usize) -> bool {
    chars[i].is_ascii_alphanumeric()
        || (chars[i] == '.'
            && i > 0
            && chars[i - 1].is_ascii_digit()
            && chars.get(i + 1).is_some_and(char::is_ascii_digit))
        // Keep one separator between numeric runs. Otherwise "1.1.1. 50%" becomes
        // one number and the digit-boundary check rejects the valid 50% citation.
        || (chars[i].is_ascii_whitespace()
            && chars.get(i + 1).is_some_and(char::is_ascii_digit)
            && chars[..i]
                .iter()
                .rev()
                .find(|c| !c.is_ascii_whitespace())
                .is_some_and(|c| c.is_ascii_digit() || *c == '.'))
}

fn matching_text(text: &str) -> String {
    let chars: Vec<char> = text.to_lowercase().chars().collect();
    (0..chars.len())
        .filter(|&i| matching_char(&chars, i))
        .map(|i| {
            if chars[i].is_ascii_whitespace() {
                ' '
            } else {
                chars[i]
            }
        })
        .collect()
}

fn numeric_char(b: u8) -> bool {
    b.is_ascii_digit() || b == b'.'
}

fn is_number(tok: &str) -> bool {
    is_numeric_token(tok) || NUMBER_WORDS.contains(&tok)
}

// "one", "two", "ten" also sit inside common words (network, maintenance), so they
// count at a word's start or end or after "and". A skipped gap stops where matching
// resumes, so an amount word OCR joined to a column fragment ("Contributionandten
// pounds") ends its gap word; this rejects some terms ending in them ("Zone").
const SHORT_NUMS: [&str; 3] = ["one", "two", "ten"];
static NUM_WORD: LazyLock<Regex> = LazyLock::new(|| {
    let words: Vec<&str> = NUMBER_WORDS
        .iter()
        .copied()
        .filter(|w| !SHORT_NUMS.contains(w))
        .collect();
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
    /// Normalised page bytes, retaining decimal points.
    hay: Vec<u8>,
    tokens: &'a [String],
    max_skips: usize,
    seen: HashSet<(usize, usize, usize, usize)>,
}

impl Layout<'_> {
    fn gap_ok(&self, end: usize, start: usize, extra: usize) -> bool {
        let gap: String = self.raw[self.idx[end - 1] + 1..self.idx[start]]
            .iter()
            .collect();
        let lines: Vec<&str> = gap.split('\n').collect();
        if numeric_gap(&gap) || squash(lines[0]).len() > 1 + extra {
            return false;
        }
        lines.len() == 1
            || (!lines[1..lines.len() - 1]
                .iter()
                .any(|l| !squash(l).is_empty())
                && squash(lines[lines.len() - 1]).len() <= MAX_GAP + extra)
    }

    /// Where `tok` occurs within hay[lo..hi]; numbers only when not part of a longer digit run.
    fn occurrences(&self, tok: &str, lo: usize, hi: usize) -> Vec<(usize, usize)> {
        let t = tok.as_bytes();
        let hi = hi.min(self.hay.len());
        let digits = is_numeric_token(tok);
        let mut out = Vec::new();
        let mut i = lo;
        while i + t.len() <= hi {
            if &self.hay[i..i + t.len()] == t {
                let end = i + t.len();
                if !digits
                    || ((i == 0 || !numeric_char(self.hay[i - 1]))
                        && (end == self.hay.len() || !numeric_char(self.hay[end])))
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
    /// `first`: start of the first matched token. Returns (first start, last end) in `hay`.
    fn search(
        &mut self,
        i: usize,
        pos: Option<usize>,
        first: Option<usize>,
        extra: usize,
        skips: usize,
    ) -> Option<(usize, usize)> {
        let tokens = self.tokens;
        if i == tokens.len() {
            return first.zip(pos);
        }
        if !self
            .seen
            .insert((i, pos.unwrap_or(usize::MAX), extra, skips))
        {
            return None;
        }
        let tok = &tokens[i];
        let spans: Vec<(usize, usize)> = match pos {
            None => self.occurrences(tok, 0, self.hay.len()),
            Some(p) => {
                let hi = p + MAX_GAP + 2 * extra + 2 + tok.len();
                self.occurrences(tok, p, hi)
                    .into_iter()
                    .filter(|&(s, _)| self.gap_ok(p, s, extra))
                    .collect()
            }
        };
        for (s, e) in spans {
            if let Some(found) = self.search(i + 1, Some(e), first.or(Some(s)), 0, skips) {
                return Some(found);
            }
        }
        if !is_number(tok) && skips < self.max_skips {
            return self.search(i + 1, pos, first, extra + tok.len() + 1, skips + 1);
        }
        None
    }
}

/// Lowercased page chars, the index into them of each matching char, and the normalised bytes.
fn normalise(page: &str) -> (Vec<char>, Vec<usize>, Vec<u8>) {
    let raw: Vec<char> = page.to_lowercase().chars().collect();
    let idx: Vec<usize> = (0..raw.len()).filter(|&i| matching_char(&raw, i)).collect();
    let hay: Vec<u8> = idx
        .iter()
        .map(|&i| {
            if raw[i].is_ascii_whitespace() {
                b' '
            } else {
                raw[i] as u8
            }
        })
        .collect();
    (raw, idx, hay)
}

/// Span in `hay` (first start, last end) of the snippet on this page text.
fn layout_match(
    tokens: &[String],
    raw: Vec<char>,
    idx: Vec<usize>,
    hay: Vec<u8>,
) -> Option<(usize, usize)> {
    let plain = tokens.iter().filter(|t| !is_number(t)).count();
    let mut layout = Layout {
        raw,
        idx,
        hay,
        tokens,
        max_skips: (plain as f64 * MAX_SKIP_SHARE) as usize,
        seen: HashSet::new(),
    };
    layout.search(0, None, None, 0, 0)
}

/// Span in the normalised page text (`idx` positions) where the snippet matches.
fn match_span(needle: &str, tokens: &[String], page: &str) -> Option<Range<usize>> {
    let (raw, idx, hay) = normalise(page);
    let n = needle.as_bytes();
    let text = String::from_utf8_lossy(&hay);
    let contiguous = text.match_indices(needle).find_map(|(start, _)| {
        let end = start + n.len();
        let left_ok = !n[0].is_ascii_digit() || start == 0 || !numeric_char(hay[start - 1]);
        let right_ok =
            !n[n.len() - 1].is_ascii_digit() || end == hay.len() || !numeric_char(hay[end]);
        (left_ok && right_ok).then_some((start, end))
    });
    let (start, end) = match contiguous {
        Some(span) => span,
        None if tokens.len() >= MIN_TOKENS => layout_match(tokens, raw, idx.clone(), hay)?,
        None => return None,
    };
    Some(idx[start]..idx[end - 1] + 1)
}

/// One OCR letter-for-digit substitution the winning match relied on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Repair {
    /// Byte offset in the original page text.
    pub at: usize,
    pub from: char,
    pub to: char,
}

/// Where a snippet re-locates on its cited page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Located {
    pub page: usize,
    /// Byte range in the original page text, from the first to the last matched char.
    pub range: Range<usize>,
    /// Empty when the snippet matches the page as printed.
    pub repairs: Vec<Repair>,
}

/// Where `ev`'s snippet is on its cited page, if anywhere.
///
/// Acceptance is decided by the digit-repaired page (so a snippet that literally contains
/// "5o" is not found, as before). If the page as printed also matches, that match wins and
/// no repairs are reported; otherwise `repairs` lists each substitution inside the range.
pub fn locate(ev: &Evidence, pages: &Pages) -> Option<Located> {
    let page = pages.get(&ev.page)?;
    let needle = matching_text(&ev.snippet);
    if needle.is_empty() {
        return None;
    }
    let tokens = tokens(&ev.snippet);
    let fixed = ocr_digits(page);
    let fixed_span = match_span(&needle, &tokens, &fixed)?;
    let raw_span = (fixed != *page)
        .then(|| match_span(&needle, &tokens, page))
        .flatten();
    let (span, repaired) = match raw_span {
        Some(span) => (span, false),
        None => (fixed_span, fixed != *page),
    };

    let offsets: Vec<usize> = page.char_indices().map(|(b, _)| b).collect();
    // `to_lowercase()` can change the char count, which breaks the char index mapping;
    // then the range conservatively covers the whole page.
    let aligned = page.to_lowercase().chars().count() == offsets.len();
    let (first, last) = if aligned {
        (span.start, span.end)
    } else {
        (0, offsets.len())
    };
    let byte = |c: usize| offsets.get(c).copied().unwrap_or(page.len());
    let repairs = if repaired {
        page.chars()
            .zip(fixed.chars())
            .enumerate()
            .skip(first)
            .take(last - first)
            .filter(|(_, (a, b))| a != b)
            .map(|(i, (from, to))| Repair {
                at: byte(i),
                from,
                to,
            })
            .collect()
    } else {
        Vec::new()
    };
    Some(Located {
        page: ev.page,
        range: byte(first)..byte(last),
        repairs,
    })
}

/// Is `ev`'s snippet present on its cited page?
pub fn evidence_found(ev: Option<&Evidence>, pages: &Pages) -> bool {
    ev.and_then(|e| locate(e, pages)).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pages(text: &str) -> Pages {
        Pages::from([(1, text.to_string())])
    }

    fn ev(snippet: &str) -> Evidence {
        Evidence {
            page: 1,
            snippet: snippet.to_string(),
        }
    }

    #[test]
    fn clean_contiguous_match_has_exact_range() {
        let page = "aaa The sum of £10,000 bbb";
        let loc = locate(&ev("The sum of £10,000"), &pages(page)).unwrap();
        assert!(loc.repairs.is_empty());
        assert_eq!(&page[loc.range], "The sum of £10,000");
    }

    #[test]
    fn digit_repair_is_reported_at_byte_offset() {
        let page = "Price £5o,000 total";
        let loc = locate(&ev("£50,000"), &pages(page)).unwrap();
        let at = page.find('o').unwrap();
        assert_eq!(
            loc.repairs,
            vec![Repair {
                at,
                from: 'o',
                to: '0'
            }]
        );
        assert!(loc.range.contains(&at));
    }

    #[test]
    fn literal_ocr_lookalike_in_snippet_is_not_found() {
        assert!(!evidence_found(Some(&ev("5o%")), &pages("pay 5o% now")));
    }

    #[test]
    fn two_column_layout_returns_range_inside_page() {
        let page = "The Tenant shall pay the Rent\nand the\nLandlord a sum of rent each quarter in advance";
        let loc = locate(
            &ev("The Tenant shall pay the Rent and the Landlord a sum of rent"),
            &pages(page),
        );
        let loc = loc.expect("layout match");
        assert!(loc.range.end <= page.len() && loc.range.start < loc.range.end);
        assert!(page.is_char_boundary(loc.range.start) && page.is_char_boundary(loc.range.end));
    }

    #[test]
    fn non_ascii_prefix_keeps_byte_offsets() {
        let page = "£€ é — The sum of £10,000 end";
        let loc = locate(&ev("The sum of £10,000"), &pages(page)).unwrap();
        assert_eq!(&page[loc.range], "The sum of £10,000");
    }
}
