//! Text normalisation and number reading shared by evidence matching, triggers and the verifier.

use std::collections::BTreeSet;
use std::str::FromStr;
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

const UNITS: [&str; 20] = [
    "zero",
    "one",
    "two",
    "three",
    "four",
    "five",
    "six",
    "seven",
    "eight",
    "nine",
    "ten",
    "eleven",
    "twelve",
    "thirteen",
    "fourteen",
    "fifteen",
    "sixteen",
    "seventeen",
    "eighteen",
    "nineteen",
];
const TENS: [(&str, u64); 8] = [
    ("twenty", 20),
    ("thirty", 30),
    ("forty", 40),
    ("fifty", 50),
    ("sixty", 60),
    ("seventy", 70),
    ("eighty", 80),
    ("ninety", 90),
];
const SCALES: [(&str, u64); 2] = [("thousand", 1_000), ("million", 1_000_000)];

/// Every number word, longest first so "seventeen" wins over "seven".
pub(crate) static NUMBER_WORDS: LazyLock<Vec<&'static str>> = LazyLock::new(|| {
    let mut v: Vec<&str> = UNITS.to_vec();
    v.extend(TENS.iter().map(|t| t.0));
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

fn letters_only(text: &str) -> String {
    text.to_lowercase()
        .chars()
        .filter(|c| c.is_ascii_lowercase())
        .collect()
}

/// "sixteen thousand nine hundred and thirteen" -> 16913; tolerant of run-together words.
pub fn words_to_int(text: &str) -> Option<u64> {
    let (mut total, mut current, mut seen) = (0u64, 0u64, false);
    // tolerate OCR run-togethers like "fiftyeight" by greedy tokenising
    let s = letters_only(text);
    let mut rest = s.as_str();
    while !rest.is_empty() {
        let w = *VOCAB.iter().find(|w| rest.starts_with(**w))?;
        rest = &rest[w.len()..];
        if w == "and" {
            continue;
        }
        seen = true;
        if let Some(i) = UNITS.iter().position(|u| *u == w) {
            current = current.checked_add(i as u64)?;
        } else if let Some(t) = TENS.iter().find(|t| t.0 == w) {
            current = current.checked_add(t.1)?;
        } else if w == "hundred" {
            current = current.checked_mul(100)?;
        } else {
            let scale = SCALES.iter().find(|t| t.0 == w).map_or(1, |t| t.1);
            total = total.checked_add(current.checked_mul(scale)?)?;
            current = 0;
        }
    }
    if seen {
        total.checked_add(current)
    } else {
        None
    }
}

/// "ten thousand and fifty eight pounds and sixty pence" -> 10058.60
pub fn words_to_amount(text: &str) -> Option<Decimal> {
    let t = text.to_lowercase();
    let at = t.find("pound")?;
    let pounds = words_to_int(&t[..at])?;
    let mut rest = &t[at + "pound".len()..];
    rest = rest.strip_prefix('s').unwrap_or(rest);
    let suffix = letters_only(rest);
    let pence = if suffix.is_empty() || suffix == "and" {
        0
    } else {
        let number = suffix
            .strip_suffix("pence")
            .or_else(|| suffix.strip_suffix("penny"))?;
        words_to_int(number)?
    };
    Some(Decimal::from(pounds) + Decimal::new(i64::try_from(pence).ok()?, 2))
}

static NUMBER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"[0-9]{1,3}(?:,[0-9]{3})+(?:\.[0-9]+)?|[0-9]+(?:\.[0-9]+)?").unwrap()
});
static WORD_RUN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!(r"(?i)(?:(?:{})[\s-]*)+", VOCAB.join("|"))).unwrap());
static TRAILING_FILLER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)(?:\band\b|[\s-])+$").unwrap());

/// Every number written in `text`, as digits (with OCR repair) or number words.
pub fn numbers_in(text: &str) -> BTreeSet<Decimal> {
    let mut out: BTreeSet<Decimal> = NUMBER
        .find_iter(&ocr_digits(text))
        .filter_map(|m| Decimal::from_str(&m.as_str().replace(',', "")).ok())
        .collect();
    for run in WORD_RUN.find_iter(text) {
        let run = TRAILING_FILLER.replace(run.as_str().trim(), "");
        if let Some(n) = (!run.is_empty()).then(|| words_to_int(&run)).flatten() {
            out.insert(Decimal::from(n));
        }
    }
    out
}

/// A share needs percentage wording: "50th Dwelling" does not state a 50% share.
pub fn percentage_in_snippet(value: Decimal, snippet: &str) -> bool {
    static SUFFIX: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?i)^\s*(?:%|per\s*cent)").unwrap());
    let text = ocr_digits(snippet);
    NUMBER
        .find_iter(&text)
        .chain(WORD_RUN.find_iter(&text))
        .any(|m| {
            if !SUFFIX.is_match(&text[m.end()..]) {
                return false;
            }
            let number = m.as_str().trim().replace(',', "");
            let parsed = Decimal::from_str(&number)
                .ok()
                .or_else(|| words_to_int(&number).map(Decimal::from));
            parsed == Some(value)
        })
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
