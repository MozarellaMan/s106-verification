//! Text normalisation and number lookup shared by evidence matching, triggers and the verifier.

use std::collections::BTreeSet;

use rust_decimal::Decimal;

use crate::quantity::{self, Unit};

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

/// Every number written in `text`, as digits or number words, whatever its unit.
pub fn numbers_in(text: &str) -> BTreeSet<Decimal> {
    quantity::scan(text)
        .quantities
        .into_iter()
        .map(|q| q.value)
        .collect()
}

/// A share needs percentage wording: "50th Dwelling" does not state a 50% share.
pub fn percentage_in_snippet(value: Decimal, snippet: &str) -> bool {
    quantity::scan(snippet)
        .quantities
        .iter()
        .any(|q| q.unit == Unit::Percent && q.value == value)
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
