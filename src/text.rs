//! Text normalisation and number reading shared by evidence matching, triggers and the verifier.

use std::collections::BTreeSet;
use std::sync::LazyLock;

use regex::Regex;
use rust_decimal::Decimal;

/// OCR drops and adds spaces arbitrarily: compare on lowercase ASCII alphanumerics only.
pub fn squash(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .filter(|c| c.is_ascii_digit() || c.is_ascii_lowercase())
        .collect()
}

/// Replace each of `letters` by `digit` where it sits inside a number: a digit before it and a
/// digit, number punctuation or line end after it, or no letter before it and a digit after it.
/// Never a letter on both sides, so "paragraph1of" stays as it is. (The regex crate has no
/// lookaround, so this scans the characters.)
fn repair(text: &str, letters: &[char], digit: char) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    for (i, &c) in chars.iter().enumerate() {
        let prev = i.checked_sub(1).map(|j| chars[j]);
        let next = chars.get(i + 1).copied();
        let inside = letters.contains(&c)
            && ((prev.is_some_and(|p| p.is_ascii_digit())
                && next.is_none_or(|n| n == '\n' || n.is_ascii_digit() || "%,.)".contains(n)))
                || (!prev.is_some_and(|p| p.is_ascii_alphabetic())
                    && next.is_some_and(|n| n.is_ascii_digit())));
        out.push(if inside { digit } else { c });
    }
    out
}

/// Undo OCR letter-for-digit swaps only inside numbers: "5o%" -> "50%", "l0" -> "10",
/// but "paragraph1of" stays as it is.
pub fn ocr_digits(text: &str) -> String {
    repair(&repair(text, &['o', 'O'], '0'), &['l', 'I', '|'], '1')
}

use crate::numeric::{SCALES, TENS, UNITS};

/// Every number word, longest first so "seventeen" wins over "seven".
pub(crate) static NUMBER_WORDS: LazyLock<Vec<&'static str>> = LazyLock::new(|| {
    let mut v: Vec<&str> = UNITS.to_vec();
    v.extend(TENS);
    v.extend(SCALES.iter().map(|t| t.0));
    v.push("hundred");
    v.sort_by_key(|w| std::cmp::Reverse(w.len()));
    v
});

static VOCAB: LazyLock<Vec<&'static str>> = LazyLock::new(|| {
    let mut v = NUMBER_WORDS.clone();
    v.push("and");
    v.sort_by_key(|w| std::cmp::Reverse(w.len()));
    v
});

/// Parse an entire English integer expression; joined words and word separators are explicit.
pub fn words_to_int(text: &str) -> Option<u64> {
    crate::numeric::integer(text).ok()
}

/// Parse an entire pounds expression. A present suffix must be complete.
pub fn words_to_amount(text: &str) -> Option<Decimal> {
    crate::numeric::amount(text).ok()
}

// Candidate selection deliberately keeps signs and malformed decimal/grouping runs intact.
static NUMBER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[+−-]\s*[.0-9][0-9.,+−\-]*|[.]?[0-9][0-9.,+−\-]*").unwrap());
static WORD_RUN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!(r"(?i)(?:(?:{})[\s,-]*)+", VOCAB.join("|"))).unwrap());
/// Located numeric values and failures, in original excerpt UTF-8 byte coordinates.
#[derive(Debug, Default)]
pub struct LocatedNumbers {
    pub values: BTreeSet<Decimal>,
    pub diagnostics: Vec<crate::numeric::Diagnostic>,
}

fn digit_selections(text: &str) -> Vec<std::ops::Range<usize>> {
    NUMBER
        .find_iter(text)
        .filter_map(|m| {
            let before = text[..m.start()].chars().next_back();
            if before.is_some_and(|c| c.is_ascii_digit() || ".,+−-".contains(c)) {
                return None;
            }
            if text[m.end()..].starts_with(['o', 'O', 'l', 'I', '|'])
                || before.is_some_and(|c| matches!(c, 'l' | 'I' | '|'))
            {
                return None;
            }
            let left_word = text[..m.start()]
                .rsplit(|c: char| !c.is_ascii_alphabetic())
                .next()
                .unwrap_or("")
                .to_ascii_lowercase();
            let right_word = text[m.end()..]
                .split(|c: char| !c.is_ascii_alphabetic())
                .next()
                .unwrap_or("")
                .to_ascii_lowercase();
            // A digit embedded in an English number word is not an independent number.
            if (left_word != "f"
                && !left_word.is_empty()
                && NUMBER_WORDS.iter().any(|w| w.starts_with(&left_word)))
                || (!right_word.is_empty() && NUMBER_WORDS.iter().any(|w| w.ends_with(&right_word)))
            {
                return None;
            }
            let mut end = m.end();
            // A single sentence-final dot/comma is prose punctuation, explicitly selected out.
            let run = m.as_str();
            if text[end..].chars().next().is_none_or(char::is_whitespace)
                && ((run.ends_with('.') && run.matches('.').count() == 1)
                    || (run.ends_with(',') && run.matches(',').count() == 1))
            {
                end -= 1;
            }
            Some(m.start()..end)
        })
        .collect()
}

pub fn located_numbers(text: &str) -> LocatedNumbers {
    let mut out = LocatedNumbers::default();
    for selection in digit_selections(text) {
        let observed = &text[selection.clone()];
        match crate::numeric::decimal(observed) {
            Ok(value) => {
                out.values.insert(value);
            }
            Err(error) => out.diagnostics.push(error.selected(text, selection)),
        }
    }
    for run in WORD_RUN.find_iter(text) {
        if text[..run.start()]
            .chars()
            .next_back()
            .is_some_and(|c| c.is_alphabetic() || c.is_ascii_digit() || "#/-".contains(c))
        {
            continue;
        }
        let selected = run.as_str().trim();
        let end = run.start() + selected.len();
        if text[end..]
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_digit() || c == '#' || c == '/')
        {
            continue;
        }
        let tail = text[end..].to_ascii_lowercase();
        if tail.starts_with(|c: char| c.is_alphabetic())
            && !["pound", "pence", "penny", "percent"]
                .iter()
                .any(|unit| tail.starts_with(unit))
        {
            continue;
        }
        if !selected.is_empty() {
            match crate::numeric::integer(selected) {
                Ok(value) => {
                    out.values.insert(Decimal::from(value));
                }
                Err(error) => out.diagnostics.push(error.selected(text, run.start()..end)),
            }
        }
    }
    let repaired = ocr_digits(text);
    if repaired != text {
        let mut d = crate::numeric::Diagnostic::new(
            text,
            0..text.len(),
            crate::numeric::Category::AmbiguousSource,
            crate::numeric::Cause::OcrSubstitutionCandidate {
                normalised: repaired,
            },
            "directly readable digits in original source",
        );
        d.normalised_span = None;
        out.diagnostics.push(d);
    }
    out
}

/// Every directly readable number. OCR substitution candidates never establish a value.
pub fn numbers_in(text: &str) -> BTreeSet<Decimal> {
    located_numbers(text).values
}

/// Locate candidates separately from parsing their complete percentage suffix.
pub fn located_percentages(snippet: &str) -> LocatedNumbers {
    static SUFFIX_CANDIDATE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?i)^\s*(?:%[%\.]*|per\s*[a-z]*)").unwrap());
    let mut out = LocatedNumbers::default();
    for selection in digit_selections(snippet)
        .into_iter()
        .chain(WORD_RUN.find_iter(snippet).map(|m| m.range()))
    {
        if snippet[..selection.start]
            .chars()
            .next_back()
            .is_some_and(|c| c.is_alphabetic())
            && snippet[selection.clone()].starts_with(|c: char| c.is_alphabetic())
        {
            continue;
        }
        let Some(suffix) = SUFFIX_CANDIDATE.find(&snippet[selection.end..]) else {
            continue;
        };
        let suffix_start = selection.end;
        let suffix_end = suffix_start + suffix.end();
        if let Err(error) = crate::numeric::percent_suffix(suffix.as_str()) {
            out.diagnostics
                .push(error.selected(snippet, suffix_start..suffix_end));
            continue;
        }
        let number = &snippet[selection.clone()];
        let parsed = if number
            .trim_start()
            .starts_with(|c: char| c.is_ascii_digit() || "+−-.".contains(c))
        {
            crate::numeric::decimal(number)
        } else {
            crate::numeric::integer(number).map(Decimal::from)
        };
        match parsed {
            Ok(value) => {
                out.values.insert(value);
            }
            Err(error) => out.diagnostics.push(error.selected(snippet, selection)),
        }
    }
    out
}

/// A share needs a complete percentage suffix. Ordinal dwellings do not state shares.
pub fn percentage_in_snippet(value: Decimal, snippet: &str) -> bool {
    located_percentages(snippet).values.contains(&value)
}

pub fn percentage_diagnostics(value: Decimal, snippet: &str) -> Vec<crate::numeric::Diagnostic> {
    let located = located_percentages(snippet);
    if located.values.contains(&value) {
        return vec![];
    }
    let mut diagnostics = located.diagnostics;
    if diagnostics.is_empty() {
        diagnostics.push(crate::numeric::Diagnostic::new(
            snippet,
            0..snippet.len(),
            crate::numeric::Category::Invalid,
            crate::numeric::Cause::MissingPercentageWording {
                expected_value: value.to_string(),
            },
            "the stated share followed by % or per cent; an ordinal is not a percentage",
        ));
    }
    diagnostics
}

/// A value a check depends on.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Num(Decimal),
    Text(String),
}

impl From<u32> for Value {
    fn from(v: u32) -> Self {
        Value::Num(Decimal::from(v))
    }
}
impl From<Decimal> for Value {
    fn from(v: Decimal) -> Self {
        Value::Num(v)
    }
}
impl From<&str> for Value {
    fn from(v: &str) -> Self {
        Value::Text(v.to_string())
    }
}
impl std::fmt::Display for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Value::Num(d) => write!(f, "{}", d.normalize()),
            Value::Text(t) => write!(f, "{t}"),
        }
    }
}

/// Does the snippet state this value? Numbers compare numerically; text compares squashed.
pub fn value_in_snippet(value: &Value, snippet: &str) -> bool {
    match value {
        Value::Num(d) => numbers_in(snippet).contains(d),
        Value::Text(t) => {
            let t = squash(t);
            !t.is_empty() && squash(snippet).contains(&t)
        }
    }
}
