//! Quantities in prose and wire fields: one tokenizer, one value-returning grammar.
//!
//! Text is lexed once into typed tokens with UTF-8 byte spans. [`scan`] walks the tokens for
//! digit and English-word quantities; [`decimal`], [`integer`] and [`amount`] run the same
//! tokenizer and grammar over a whole field, so the prose and exact paths cannot drift.
//! Nothing is guessed: OCR letter/digit mixes and malformed candidates are reported, not repaired.

use std::ops::Range;

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use winnow::{
    ModalResult, Parser,
    combinator::{alt, cut_err, fail, opt, preceded, terminated},
    token::{any, take_while},
};

pub const NUMBER_WORDS: [&str; 31] = [
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
    "twenty",
    "thirty",
    "forty",
    "fifty",
    "sixty",
    "seventy",
    "eighty",
    "ninety",
    "hundred",
    "thousand",
    "million",
];
const OTHER_WORDS: [&str; 8] = [
    "and", "pound", "pounds", "pence", "penny", "per", "cent", "percent",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Unit {
    Gbp,
    Percent,
    Count,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Quantity {
    pub value: Decimal,
    pub unit: Unit,
    pub span: Range<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    Absent,
    Invalid,
    Unsupported,
    NumericRange,
    AmbiguousSource,
    Ungrounded,
}

impl Category {
    pub fn code(self) -> &'static str {
        match self {
            Category::Absent => "NUM001",
            Category::Invalid => "NUM002",
            Category::Unsupported => "NUM003",
            Category::NumericRange => "NUM004",
            Category::AmbiguousSource => "NUM005",
            Category::Ungrounded => "NUM006",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Cause {
    Empty,
    UnexpectedToken,
    /// An alphabetic token outside the vocabulary, inside an exact words parse.
    UnsupportedWord,
    IncompleteExpression,
    UnsupportedNumberOrder,
    UnrepresentableDecimal,
    MixedLettersAndDigits,
    OcrSubstitutionCandidate {
        normalised: String,
    },
    // Grounding causes, raised by the verifier rather than the parser.
    MissingPercentageWording {
        expected_value: String,
    },
    ValueNotStated {
        expected_value: String,
    },
    CitationNotRelocated,
    RelocationRequiresDigitSubstitution,
}

impl Cause {
    pub fn category(&self) -> Category {
        match self {
            Cause::Empty => Category::Absent,
            Cause::UnexpectedToken
            | Cause::IncompleteExpression
            | Cause::MissingPercentageWording { .. } => Category::Invalid,
            Cause::UnsupportedWord | Cause::UnsupportedNumberOrder => Category::Unsupported,
            Cause::UnrepresentableDecimal => Category::NumericRange,
            Cause::MixedLettersAndDigits
            | Cause::OcrSubstitutionCandidate { .. }
            | Cause::RelocationRequiresDigitSubstitution => Category::AmbiguousSource,
            Cause::ValueNotStated { .. } | Cause::CitationNotRelocated => Category::Ungrounded,
        }
    }
}

/// A pure parse failure. It knows nothing about checks, pages or extractions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    /// The whole candidate that was rejected (e.g. "1,23"), in the coordinates of the scanned text.
    pub candidate: Range<usize>,
    /// The precise failure point inside the text (e.g. the "2" of "1,23"); empty at end of input.
    pub span: Range<usize>,
    pub cause: Cause,
    pub expected: &'static str,
}

impl ParseError {
    pub fn category(&self) -> Category {
        self.cause.category()
    }
}

#[derive(Debug, Default)]
pub struct Scan {
    pub quantities: Vec<Quantity>,
    pub errors: Vec<ParseError>,
}

// ---- lexer

#[derive(Debug, Clone, Copy, PartialEq)]
enum Kind {
    Num,
    Space,
    Alpha,
    Word(&'static str),
    Sym(char),
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Tok {
    kind: Kind,
    start: usize,
    end: usize,
}

impl Tok {
    fn span(&self) -> Range<usize> {
        self.start..self.end
    }
}

fn vocabulary() -> impl Iterator<Item = &'static str> {
    NUMBER_WORDS.into_iter().chain(OTHER_WORDS)
}

/// Segment a whole ASCII run into vocabulary words ("fiftyeight"), longest first with
/// backtracking; `None` if any part is left over. `ok[k]` is whether the tail from `k` segments.
fn segment(run: &str) -> Option<Vec<&'static str>> {
    if let Some(w) = vocabulary().find(|w| w.eq_ignore_ascii_case(run)) {
        return Some(vec![w]);
    }
    let lower = run.to_ascii_lowercase().into_bytes();
    // `next[k]`: the longest word at `k` after which the rest still segments
    let mut next: Vec<Option<&'static str>> = vec![None; lower.len() + 1];
    for k in (0..lower.len()).rev() {
        let fits = |w: &&'static str| {
            lower[k..].starts_with(w.as_bytes())
                && (k + w.len() == lower.len() || next[k + w.len()].is_some())
        };
        next[k] = vocabulary().filter(fits).max_by_key(|w| w.len());
    }
    let (mut k, mut words) = (0, vec![]);
    while k < lower.len() {
        let w = next[k]?;
        words.push(w);
        k += w.len();
    }
    Some(words)
}

fn lex(text: &str) -> Vec<Tok> {
    let mut toks = vec![];
    let mut at = 0;
    while let Some(c) = text[at..].chars().next() {
        let rest = &text[at..];
        let (kind, len) = if c.is_ascii_digit() {
            // digits with internal '.'/',' that are followed by a digit
            let reach = rest
                .find(|c: char| !(c.is_ascii_digit() || c == '.' || c == ','))
                .unwrap_or(rest.len());
            let len = rest[..reach].trim_end_matches(['.', ',']).len();
            (Kind::Num, len)
        } else if c.is_whitespace() {
            let len = rest
                .find(|c: char| !c.is_whitespace())
                .unwrap_or(rest.len());
            (Kind::Space, len)
        } else if c.is_alphabetic() {
            let len = rest
                .find(|c: char| !c.is_alphabetic())
                .unwrap_or(rest.len());
            if let Some(words) = rest[..len]
                .is_ascii()
                .then(|| segment(&rest[..len]))
                .flatten()
            {
                for w in words {
                    toks.push(Tok {
                        kind: Kind::Word(w),
                        start: at,
                        end: at + w.len(),
                    });
                    at += w.len();
                }
                continue;
            }
            (Kind::Alpha, len)
        } else {
            (Kind::Sym(c), c.len_utf8())
        };
        toks.push(Tok {
            kind,
            start: at,
            end: at + len,
        });
        at += len;
    }
    toks
}

// ---- scanner

/// A found quantity and whether it was written in digits.
struct Hit {
    quantity: Quantity,
    digits: bool,
    /// An ordinal or percent suffix was read; an exact field must not carry one.
    suffixed: bool,
}

struct Scanner<'a> {
    text: &'a str,
    toks: Vec<Tok>,
    /// Tokens inside a run of adjacent digits and letters, which produce no value.
    mixed: Vec<bool>,
    hits: Vec<Hit>,
    errors: Vec<ParseError>,
    /// Token where scanning resumed after splitting a cluster at a prose "and"; a number word
    /// there starts a quantity even though a word precedes it.
    resume: Option<usize>,
}

const OPENING: &str = "([{\"'“‘";
const CLOSING: &str = ")]}\"'”’";

impl<'a> Scanner<'a> {
    fn new(text: &'a str) -> Self {
        let toks = lex(text);
        let mixed = vec![false; toks.len()];
        let mut s = Scanner {
            text,
            toks,
            mixed,
            hits: vec![],
            errors: vec![],
            resume: None,
        };
        s.find_mixed();
        s
    }

    fn kind(&self, i: usize) -> Option<Kind> {
        self.toks.get(i).map(|t| t.kind)
    }

    fn is_alpha(&self, i: usize, any_of: &[&str]) -> bool {
        self.kind(i) == Some(Kind::Alpha)
            && any_of
                .iter()
                .any(|a| self.text[self.toks[i].span()].eq_ignore_ascii_case(a))
    }

    fn error(
        &mut self,
        candidate: Range<usize>,
        span: Range<usize>,
        cause: Cause,
        expected: &'static str,
    ) {
        self.errors.push(ParseError {
            candidate,
            span,
            cause,
            expected,
        });
    }

    /// Runs of adjacent Num/word/Alpha/'|' tokens holding both digits and letters ("5o", "fif5ty").
    /// An OCR "f" for "£" before a number and an ordinal suffix after one are readable.
    fn find_mixed(&mut self) {
        let part = |k: Kind| matches!(k, Kind::Num | Kind::Alpha | Kind::Word(_) | Kind::Sym('|'));
        let mut a = 0;
        while a < self.toks.len() {
            if !part(self.toks[a].kind) {
                a += 1;
                continue;
            }
            let mut b = a;
            while b < self.toks.len() && part(self.toks[b].kind) {
                b += 1;
            }
            if b - a == 1 {
                a = b;
                continue;
            }
            let readable = |s: &Self, i: usize| {
                let after_num = i > 0 && s.kind(i - 1) == Some(Kind::Num);
                (s.is_alpha(i, &["f"]) && s.kind(i + 1) == Some(Kind::Num))
                    || (after_num && s.is_alpha(i, &["st", "nd", "rd", "th"]))
                    // "50percent", "50per cent"
                    || (after_num && s.kind(i) == Some(Kind::Word("percent")))
                    || (after_num
                        && s.kind(i) == Some(Kind::Word("per"))
                        && (s.kind(i + 1) == Some(Kind::Word("cent"))
                            || (s.kind(i + 1) == Some(Kind::Space)
                                && s.kind(i + 2) == Some(Kind::Word("cent")))))
                    || (i > 1
                        && s.kind(i) == Some(Kind::Word("cent"))
                        && s.kind(i - 1) == Some(Kind::Word("per"))
                        && s.kind(i - 2) == Some(Kind::Num))
            };
            let rest: Vec<usize> = (a..b).filter(|&i| !readable(self, i)).collect();
            let has = |f: fn(Kind) -> bool| rest.iter().any(|&i| f(self.toks[i].kind));
            if has(|k| k == Kind::Num) && has(|k| matches!(k, Kind::Alpha | Kind::Word(_))) {
                let run = self.toks[a].start..self.toks[b - 1].end;
                let normalised = crate::text::ocr_digits(&self.text[run.clone()]);
                let cause = if normalised.chars().any(|c| c.is_ascii_alphabetic()) {
                    Cause::MixedLettersAndDigits
                } else {
                    Cause::OcrSubstitutionCandidate { normalised }
                };
                self.mixed[a..b].fill(true);
                self.error(
                    run.clone(),
                    run,
                    cause,
                    "directly readable digits or number words",
                );
            }
            a = b;
        }
    }

    fn left_ok(&self, i: usize) -> bool {
        i == 0
            || match self.toks[i - 1].kind {
                Kind::Space => true,
                Kind::Sym(c) => OPENING.contains(c),
                _ => false,
            }
    }

    /// `j` is the first token after the candidate.
    fn right_ok(&self, j: usize, digits: bool) -> bool {
        match self.kind(j) {
            None | Some(Kind::Space) => true,
            Some(Kind::Sym(c)) if CLOSING.contains(c) || ";:".contains(c) => true,
            Some(Kind::Sym('.' | ',' | '!' | '?')) => match self.kind(j + 1) {
                None | Some(Kind::Space) => true,
                Some(Kind::Sym(c)) => CLOSING.contains(c),
                _ => false,
            },
            Some(Kind::Sym('-' | '–')) => {
                digits
                    && (self.kind(j + 1) == Some(Kind::Num)
                        || (self.kind(j + 1) == Some(Kind::Sym('£'))
                            && self.kind(j + 2) == Some(Kind::Num)))
            }
            _ => false,
        }
    }

    /// Reject the candidate `first..end` (token indices) at a neighbouring char.
    fn boundary_error(&mut self, first: usize, end: usize, left: bool) {
        let candidate = self.toks[first].start..self.toks[end - 1].end;
        let span = if left {
            let c = self.text[..candidate.start].chars().next_back().unwrap();
            candidate.start - c.len_utf8()..candidate.start
        } else {
            let c = self.text[candidate.end..].chars().next().unwrap();
            candidate.end..candidate.end + c.len_utf8()
        };
        self.error(
            candidate,
            span,
            Cause::UnexpectedToken,
            "a number set apart from surrounding text",
        );
    }

    fn scan(&mut self) {
        let mut i = 0;
        while i < self.toks.len() {
            if self.mixed[i] {
                i += 1;
                continue;
            }
            i = match self.toks[i].kind {
                Kind::Num => self.digits(i),
                Kind::Word(w) if NUMBER_WORDS.contains(&w) => self.words(i),
                _ => i + 1,
            };
        }
    }

    /// Digit quantity at Num token `i`; returns the next token to look at.
    fn digits(&mut self, i: usize) -> usize {
        let num = self.toks[i];
        let is_sign = |c: char| matches!(c, '+' | '-' | '−');
        let sym = |s: &Self, k: usize| match s.kind(k) {
            Some(Kind::Sym(c)) => Some(c),
            _ => None,
        };
        let mut start = i;
        while start > 0 && sym(self, start - 1).is_some_and(is_sign) {
            start -= 1;
        }
        let signs = i - start;
        // "3-5" is a range, not a sign
        let range = i >= 2
            && matches!(sym(self, i - 1), Some('-' | '–'))
            && self.kind(i - 2) == Some(Kind::Num);
        let sign_start = if range || signs == 0 { i } else { start };
        if signs >= 2 {
            let span = self.toks[start + 1].span();
            self.error(
                self.toks[start].start..num.end,
                span,
                Cause::UnexpectedToken,
                "at most one sign",
            );
            return i + 1;
        }
        let mut first = sign_start;
        let mut currency = false;
        if first > 0 && sym(self, first - 1) == Some('£') {
            (first, currency) = (first - 1, true);
        } else if first > 1
            && self.kind(first - 1) == Some(Kind::Space)
            && sym(self, first - 2) == Some('£')
        {
            (first, currency) = (first - 2, true);
        } else if first > 0 && self.is_alpha(first - 1, &["f"]) {
            (first, currency) = (first - 1, true);
        }
        // A sign may also precede the currency symbol ("-£1,234.50"), unless the dash joins
        // a range ("£10,000-£20,000").
        let mut range = range;
        let mut sign_before = None;
        if currency && signs == 0 && first > 0 {
            match sym(self, first - 1) {
                Some('-' | '–') if first >= 2 && self.kind(first - 2) == Some(Kind::Num) => {
                    range = true
                }
                Some(c) if is_sign(c) => {
                    if first >= 2 && sym(self, first - 2).is_some_and(is_sign) {
                        self.error(
                            self.toks[first - 2].start..num.end,
                            self.toks[first - 1].span(),
                            Cause::UnexpectedToken,
                            "at most one sign",
                        );
                        return i + 1;
                    }
                    (first, sign_before) = (first - 1, Some(c));
                }
                _ => {}
            }
        }

        // suffix: ordinal, "%", "percent" or "per cent"
        let mut end = i + 1;
        let (mut percent, mut sign_suffix, mut ordinal) = (false, false, false);
        if self.is_alpha(end, &["st", "nd", "rd", "th"]) {
            (end, ordinal) = (end + 1, true);
        } else {
            let p = end + usize::from(self.kind(end) == Some(Kind::Space));
            match self.kind(p) {
                Some(Kind::Sym('%')) => (end, percent, sign_suffix) = (p + 1, true, true),
                Some(Kind::Word("percent")) => (end, percent) = (p + 1, true),
                Some(Kind::Word("per")) => {
                    let q = p + 1 + usize::from(self.kind(p + 1) == Some(Kind::Space));
                    if self.kind(q) == Some(Kind::Word("cent")) {
                        (end, percent) = (q + 1, true);
                    }
                }
                _ => {}
            }
        }

        let candidate = self.toks[sign_start].start..self.toks[end - 1].end;
        if !range && !self.left_ok(first) {
            self.boundary_error(first, end, true);
            return end;
        }
        let digits = &self.text[num.span()];
        let neg = (signs == 1 && !range && sym(self, start) != Some('+'))
            || sign_before.is_some_and(|c| c != '+');
        let value = match digits_grammar.parse(digits) {
            Ok(v) => v,
            Err(e) => {
                let at = num.start + e.offset();
                let width = digits[e.offset()..]
                    .chars()
                    .next()
                    .map_or(0, char::len_utf8);
                let candidate = self.toks[sign_start].start..num.end;
                self.error(
                    candidate,
                    at..at + width,
                    Cause::UnexpectedToken,
                    "digits with optional thousands commas and decimals",
                );
                return i + 1;
            }
        };
        let Ok(value) = Decimal::from_str_exact(&value) else {
            let candidate = self.toks[sign_start].start..num.end;
            self.error(
                candidate.clone(),
                candidate,
                Cause::UnrepresentableDecimal,
                "a number a Decimal can hold exactly",
            );
            return i + 1;
        };
        let value = if neg { -value } else { value };

        if sign_suffix && self.kind(end) == Some(Kind::Sym('%')) {
            self.boundary_error(first, end, false);
            return end + 1;
        }
        // "50%of" is how OCR runs a sentence together
        let run_on = sign_suffix && matches!(self.kind(end), Some(Kind::Alpha | Kind::Word(_)));
        if !run_on && !self.right_ok(end, true) {
            self.boundary_error(first, end, false);
            return end;
        }
        // "£50%" or "£50th" is neither an amount nor a share
        if currency && (percent || ordinal) {
            self.error(
                self.toks[first].start..candidate.end,
                self.toks[i + 1].start..candidate.end,
                Cause::UnexpectedToken,
                "a £ amount without a percent or ordinal suffix",
            );
            return end;
        }
        let unit = if currency {
            Unit::Gbp
        } else if percent {
            Unit::Percent
        } else {
            Unit::Count
        };
        let span = self.toks[first].start..candidate.end;
        self.hits.push(Hit {
            quantity: Quantity { value, unit, span },
            digits: true,
            suffixed: percent || ordinal,
        });
        end
    }

    /// Word cluster starting at number word `i`; returns the next token to look at.
    fn words(&mut self, i: usize) -> usize {
        if i > 0 && matches!(self.toks[i - 1].kind, Kind::Word(_)) && self.resume != Some(i) {
            return i + 1;
        }
        let mut end = cluster_end(&self.toks, i);
        if self.mixed[i..end].iter().any(|&m| m) {
            return end;
        }
        let mut cluster = self.toks[i].start..self.toks[end - 1].end;
        if self.resume != Some(i) && !self.left_ok(i) {
            self.boundary_error(i, end, true);
            return end;
        }
        let idx: Vec<usize> = (i..end)
            .filter(|&k| self.toks[k].kind != Kind::Space)
            .collect();
        let pieces: Vec<Tok> = idx.iter().map(|&k| self.toks[k]).collect();
        match quantity.parse(pieces.as_slice()) {
            Ok((value, mut unit)) => {
                // "fifty%" and "fifty %": a percent sign after number words
                let p = end + usize::from(self.kind(end) == Some(Kind::Space));
                let mut run_on = false;
                if unit == Unit::Count && self.kind(p) == Some(Kind::Sym('%')) {
                    (unit, end) = (Unit::Percent, p + 1);
                    cluster.end = self.toks[p].end;
                    run_on = matches!(self.kind(end), Some(Kind::Alpha | Kind::Word(_)));
                }
                if run_on || self.right_ok(end, false) {
                    self.word_hit(value, unit, cluster);
                } else {
                    self.boundary_error(i, end, false);
                }
            }
            Err(e) => match self.conjunction_cut(&pieces, end) {
                // A completed quantity, then "and" as prose: keep the quantity and resume
                // scanning after the conjunction.
                Some((cut, value, unit)) => {
                    self.word_hit(value, unit, cluster.start..pieces[cut - 1].end);
                    self.resume = Some(idx[cut] + 1);
                    return idx[cut] + 1;
                }
                None => {
                    let err = cluster_error(&pieces, cluster, e.offset());
                    self.errors.push(err);
                }
            },
        }
        end
    }

    fn word_hit(&mut self, value: Decimal, unit: Unit, span: Range<usize>) {
        self.hits.push(Hit {
            quantity: Quantity { value, unit, span },
            digits: false,
            suffixed: unit != Unit::Count,
        });
    }

    /// Where a cluster that does not parse as a whole splits at a prose "and": the index of
    /// that "and" in `pieces`, with the value of everything before it.
    ///
    /// Mid-cluster, only a quantity that cannot continue splits ("fifty per cent and sixty per
    /// cent"); "ten pounds and fifty" may be incomplete pence, so it does not. A trailing "and"
    /// splits when prose follows ("one hundred and the Owner"), but not after pounds when a
    /// pence clause is attempted ("ten pounds and unknown pence"), and not at the end of text.
    fn conjunction_cut(&self, pieces: &[Tok], end: usize) -> Option<(usize, Decimal, Unit)> {
        let prose_follows =
            self.kind(end) == Some(Kind::Space) && self.kind(end + 1) == Some(Kind::Alpha);
        (1..pieces.len()).rev().find_map(|cut| {
            if pieces[cut].kind != Kind::Word("and") {
                return None;
            }
            let (value, unit) = quantity.parse(&pieces[..cut]).ok()?;
            let ok = match pieces[cut - 1].kind {
                Kind::Word("percent" | "cent" | "pence" | "penny") => true,
                // "ten pounds and twenty pounds": the second amount states its own unit
                Kind::Word("pound" | "pounds") if cut + 1 < pieces.len() => {
                    pieces[cut + 1..]
                        .iter()
                        .any(|t| matches!(t.kind, Kind::Word("pound" | "pounds")))
                        && quantity.parse(&pieces[cut + 1..]).is_ok()
                }
                _ => {
                    // only a quantity that can take pence is held back by a later pence clause
                    cut + 1 == pieces.len()
                        && prose_follows
                        && !(unit == Unit::Gbp && self.pence_ahead(end))
                }
            };
            ok.then_some((cut, value, unit))
        })
    }

    /// Does a "pence" or "penny" follow before the end of the clause?
    fn pence_ahead(&self, from: usize) -> bool {
        self.toks[from..]
            .iter()
            .take_while(|t| !matches!(t.kind, Kind::Sym('.' | ';' | ':' | '(' | ')')))
            .any(|t| matches!(t.kind, Kind::Word("pence" | "penny")))
    }
}

/// End (exclusive token index) of the word cluster starting at Word token `i`: words joined by
/// space, adjacency, a hyphen between words, or a comma after thousand/million.
fn cluster_end(toks: &[Tok], i: usize) -> usize {
    let kind = |k: usize| toks.get(k).map(|t| t.kind);
    let is_word = |k: usize| matches!(kind(k), Some(Kind::Word(_)));
    let mut e = i + 1;
    loop {
        if is_word(e) {
            e += 1;
        } else if matches!(kind(e), Some(Kind::Space | Kind::Sym('-'))) && is_word(e + 1) {
            e += 2;
        } else if kind(e) == Some(Kind::Sym(','))
            && matches!(kind(e - 1), Some(Kind::Word("thousand" | "million")))
            && kind(e + 1) == Some(Kind::Space)
            && is_word(e + 2)
        {
            e += 3;
        } else {
            break;
        }
    }
    // "per" only belongs to the cluster as "per cent"
    if kind(e - 1) == Some(Kind::Word("per")) {
        e -= 1;
        while e > i + 1 && !is_word(e - 1) {
            e -= 1;
        }
    }
    e
}

/// Turn the grammar's failure offset (an index into `pieces`) into a [`ParseError`].
fn cluster_error(pieces: &[Tok], cluster: Range<usize>, offset: usize) -> ParseError {
    let at = pieces.get(offset);
    let incomplete = at.is_none_or(|t| t.kind == Kind::Word("and") && offset + 1 == pieces.len());
    let after_pounds = pieces[..offset.min(pieces.len())]
        .iter()
        .any(|t| matches!(t.kind, Kind::Word("pound" | "pounds")));
    ParseError {
        span: at.map_or(cluster.end..cluster.end, Tok::span),
        candidate: cluster,
        cause: if incomplete {
            Cause::IncompleteExpression
        } else {
            Cause::UnsupportedNumberOrder
        },
        expected: if after_pounds {
            "below one hundred followed by pence or penny"
        } else {
            "units, tens, hundreds and descending million/thousand groups"
        },
    }
}

// ---- grammars

/// `(\d{1,3}(,\d{3})+ | \d+)(\.\d+)?`, returned as a plain digit string.
fn digits_grammar(input: &mut &str) -> ModalResult<String> {
    let int = alt((
        (
            take_while(1..=3, '0'..='9'),
            winnow::combinator::repeat(1.., preceded(',', cut_err(take_while(3..=3, '0'..='9')))),
        )
            .map(|(head, tail): (&str, Vec<&str>)| format!("{head}{}", tail.concat())),
        take_while(1.., '0'..='9').map(str::to_string),
    ));
    let frac = opt(preceded('.', take_while(1.., '0'..='9')));
    (int, frac)
        .map(|(int, frac)| frac.map_or(int.clone(), |f| format!("{int}.{f}")))
        .parse_next(input)
}

type Words<'a> = &'a [Tok];

const ONES: [(&str, u64); 9] = [
    ("one", 1),
    ("two", 2),
    ("three", 3),
    ("four", 4),
    ("five", 5),
    ("six", 6),
    ("seven", 7),
    ("eight", 8),
    ("nine", 9),
];
const TEENS: [(&str, u64); 10] = [
    ("ten", 10),
    ("eleven", 11),
    ("twelve", 12),
    ("thirteen", 13),
    ("fourteen", 14),
    ("fifteen", 15),
    ("sixteen", 16),
    ("seventeen", 17),
    ("eighteen", 18),
    ("nineteen", 19),
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

/// One word token, as its value from `table`.
fn value_of<'a>(
    table: &'static [(&'static str, u64)],
) -> impl FnMut(&mut Words<'a>) -> ModalResult<u64> {
    move |i| {
        any.verify_map(|t: Tok| match t.kind {
            Kind::Word(w) => table.iter().find(|e| e.0 == w).map(|e| e.1),
            _ => None,
        })
        .parse_next(i)
    }
}

fn token<'a>(kind: Kind) -> impl FnMut(&mut Words<'a>) -> ModalResult<()> {
    move |i| {
        any.verify(move |t: &Tok| t.kind == kind)
            .void()
            .parse_next(i)
    }
}

fn word<'a>(w: &'static str) -> impl FnMut(&mut Words<'a>) -> ModalResult<()> {
    token(Kind::Word(w))
}

fn below_100(i: &mut Words<'_>) -> ModalResult<u64> {
    alt((
        (
            value_of(&TENS),
            opt(preceded(opt(token(Kind::Sym('-'))), value_of(&ONES))),
        )
            .map(|(tens, ones)| tens + ones.unwrap_or(0)),
        value_of(&ONES),
        value_of(&TEENS),
    ))
    .parse_next(i)
}

fn below_1000(i: &mut Words<'_>) -> ModalResult<u64> {
    alt((
        (
            value_of(&ONES),
            word("hundred"),
            opt(preceded(opt(word("and")), below_100)),
        )
            .map(|(hundreds, _, rest)| hundreds * 100 + rest.unwrap_or(0)),
        below_100,
    ))
    .parse_next(i)
}

/// "and" joins a group to an earlier one, never starts a cardinal.
fn and_after<'a>(earlier: bool) -> impl FnMut(&mut Words<'a>) -> ModalResult<()> {
    move |i| {
        if earlier {
            opt(word("and")).void().parse_next(i)
        } else {
            Ok(())
        }
    }
}

fn group<'a>(scale: &'static str, earlier: bool) -> impl FnMut(&mut Words<'a>) -> ModalResult<u64> {
    move |i| {
        // cheap guard: most numbers lack this scale, so skip parsing a group that cannot match
        if !i.iter().any(|t| t.kind == Kind::Word(scale)) {
            return fail.parse_next(i);
        }
        preceded(
            and_after(earlier),
            terminated(
                terminated(below_1000, word(scale)),
                opt(token(Kind::Sym(','))),
            ),
        )
        .parse_next(i)
    }
}

fn cardinal(i: &mut Words<'_>) -> ModalResult<u64> {
    if opt(word("zero")).parse_next(i)?.is_some() {
        return Ok(0);
    }
    let (mut total, mut seen) = (0, false);
    if let Some(n) = opt(group("million", false)).parse_next(i)? {
        (total, seen) = (n * 1_000_000, true);
    }
    if let Some(n) = opt(group("thousand", seen)).parse_next(i)? {
        (total, seen) = (total + n * 1_000, true);
    }
    if let Some(n) = opt(preceded(and_after(seen), below_1000)).parse_next(i)? {
        (total, seen) = (total + n, true);
    }
    if seen { Ok(total) } else { fail.parse_next(i) }
}

fn pence(i: &mut Words<'_>) -> ModalResult<u64> {
    terminated(
        alt((word("zero").value(0), below_100)),
        alt((word("pence"), word("penny"))),
    )
    .parse_next(i)
}

fn money(i: &mut Words<'_>) -> ModalResult<Decimal> {
    let pounds = terminated(cardinal, alt((word("pounds"), word("pound")))).parse_next(i)?;
    // once "and" appears, complete pence are mandatory
    let pence = opt(alt((preceded(word("and"), cut_err(pence)), pence))).parse_next(i)?;
    Ok(Decimal::from(pounds) + Decimal::new(pence.unwrap_or(0) as i64, 2))
}

fn quantity(i: &mut Words<'_>) -> ModalResult<(Decimal, Unit)> {
    alt((
        money.map(|v| (v, Unit::Gbp)),
        terminated(
            cardinal,
            alt((word("percent"), (word("per"), word("cent")).void())),
        )
        .map(|n| (Decimal::from(n), Unit::Percent)),
        cardinal.map(|n| (Decimal::from(n), Unit::Count)),
    ))
    .parse_next(i)
}

// ---- public API

/// Every quantity in prose, and every candidate that was rejected.
pub fn scan(text: &str) -> Scan {
    let mut s = Scanner::new(text);
    s.scan();
    Scan {
        quantities: s.hits.into_iter().map(|h| h.quantity).collect(),
        errors: s.errors,
    }
}

fn error_at(
    span: Range<usize>,
    candidate: Range<usize>,
    cause: Cause,
    expected: &'static str,
) -> ParseError {
    ParseError {
        candidate,
        span,
        cause,
        expected,
    }
}

/// The wire digits field: exactly one number (optionally `£`, signed), no percent, whole input.
pub fn decimal(input: &str) -> Result<Decimal, ParseError> {
    let whole = input.trim_matches(char::is_whitespace);
    let from = input.len() - input.trim_start_matches(char::is_whitespace).len();
    let trimmed = from..from + whole.len();
    if whole.is_empty() {
        return Err(error_at(
            0..input.len(),
            0..input.len(),
            Cause::Empty,
            "a decimal number",
        ));
    }
    let mut s = Scanner::new(input);
    s.scan();
    if let Some(e) = s.errors.into_iter().next() {
        return Err(e);
    }
    let outside = |at: usize| {
        let width = input[at..].chars().next().map_or(0, char::len_utf8);
        error_at(
            at..at + width,
            trimmed.clone(),
            Cause::UnexpectedToken,
            "a single decimal number",
        )
    };
    let Some(hit) = s.hits.first() else {
        return Err(outside(trimmed.start));
    };
    let q = &hit.quantity;
    if q.span.start > trimmed.start {
        return Err(outside(trimmed.start));
    }
    if q.span.end < trimmed.end {
        return Err(outside(q.span.end));
    }
    if !hit.digits || hit.suffixed {
        return Err(outside(q.span.end - 1));
    }
    Ok(q.value)
}

/// Lex `input` as a single word cluster and run `grammar` over all of it.
fn exact<T>(
    input: &str,
    expected: &'static str,
    mut grammar: fn(&mut Words<'_>) -> ModalResult<T>,
) -> Result<T, ParseError> {
    let mut s = Scanner::new(input);
    let Some(first) = s.toks.iter().position(|t| t.kind != Kind::Space) else {
        return Err(error_at(
            0..input.len(),
            0..input.len(),
            Cause::Empty,
            expected,
        ));
    };
    if let Some(e) = s.errors.drain(..).next() {
        return Err(e);
    }
    let last = s.toks.iter().rposition(|t| t.kind != Kind::Space).unwrap();
    let whole = s.toks[first].start..s.toks[last].end;
    let end = if matches!(s.toks[first].kind, Kind::Word(_)) {
        cluster_end(&s.toks, first)
    } else {
        first
    };
    if let Some(stray) = s.toks[end..].iter().find(|t| t.kind != Kind::Space) {
        let cause = match stray.kind {
            Kind::Alpha => Cause::UnsupportedWord,
            Kind::Word("per") => Cause::IncompleteExpression,
            _ => Cause::UnexpectedToken,
        };
        return Err(error_at(stray.span(), whole, cause, expected));
    }
    let pieces: Vec<Tok> = s.toks[first..=last]
        .iter()
        .filter(|t| t.kind != Kind::Space)
        .copied()
        .collect();
    grammar
        .parse(pieces.as_slice())
        .map_err(|e| cluster_error(&pieces, whole, e.offset()))
}

/// Words, whole input, no unit: "sixteen thousand nine hundred and thirteen".
pub fn integer(input: &str) -> Result<u64, ParseError> {
    exact(input, "a number in words", cardinal)
}

/// Words, whole input, pounds required: "ten thousand pounds and sixty pence".
pub fn amount(input: &str) -> Result<Decimal, ParseError> {
    exact(input, "an amount in pounds and pence in words", money)
}
