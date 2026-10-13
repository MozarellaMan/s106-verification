//! Bounded numeric expression syntax. Locators and evidence verification live elsewhere.
//! ASCII case folding preserves UTF-8 byte offsets; no OCR substitutions happen here.
// Errors own original input and citation context. Keeping this small API directly inspectable
// is preferable to boxing each error and unboxing it again into audit records.
#![allow(clippy::result_large_err)]
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::ops::Range;
use winnow::{
    Parser,
    combinator::{alt, cut_err, opt, preceded},
    token::{one_of, take_while},
};

pub const POLICY: &str = "s106-numeric/1";
pub fn identity(input: &str) -> String {
    Sha256::digest(input.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Cause {
    Empty,
    MissingPercentageWording { expected_value: String },
    ValueNotStated { expected_value: String },
    CitationNotRelocated,
    RelocationRequiresDigitSubstitution,
    UnexpectedToken,
    IncompleteExpression,
    UnsupportedNumberOrder,
    UnrepresentableDecimal,
    IntegerOverflow,
    OcrSubstitutionCandidate { normalised: String },
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Span {
    pub range: Range<usize>,
    /// Always UTF-8 bytes, never PDF glyph positions.
    pub units: String,
    /// original_observed_input (local field) or original_excerpt (selected prose).
    pub coordinate_space: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Diagnostic {
    pub code: String,
    pub category: Category,
    pub field: String,
    pub check_id: String,
    /// Reference to the enclosing run/extraction identity, when conversion supplies it.
    pub extraction_sha256: Option<String>,
    pub observed_input: Option<String>,
    pub input_sha256: Option<String>,
    pub original_excerpt: Option<String>,
    pub span: Option<Span>,
    /// ASCII case folding is length preserving. OCR candidates have no normalised span.
    pub normalised_span: Option<Span>,
    pub expected: String,
    pub cause: Cause,
    pub claimed_page: Option<usize>,
    /// Boolean relocation only; it does not establish a PDF character span.
    pub citation_relocated: Option<bool>,
}
impl Diagnostic {
    pub fn new(
        input: &str,
        range: Range<usize>,
        category: Category,
        cause: Cause,
        expected: &str,
    ) -> Self {
        let code = match category {
            Category::Absent => "NUM001",
            Category::Invalid => "NUM002",
            Category::Unsupported => "NUM003",
            Category::NumericRange => "NUM004",
            Category::AmbiguousSource => "NUM005",
            Category::Ungrounded => "NUM006",
        };
        Self {
            code: code.into(),
            category,
            field: String::new(),
            check_id: String::new(),
            extraction_sha256: None,
            observed_input: Some(input.into()),
            input_sha256: Some(identity(input)),
            original_excerpt: None,
            span: Some(Span {
                range: range.clone(),
                units: "utf8_bytes".into(),
                coordinate_space: "original_observed_input".into(),
            }),
            normalised_span: Some(Span {
                range,
                units: "utf8_bytes".into(),
                coordinate_space: "ascii_folded_observed_input".into(),
            }),
            expected: expected.into(),
            cause,
            claimed_page: None,
            citation_relocated: None,
        }
    }
    /// Missing verification input has no observed string or source span to invent.
    pub fn absent(field: &str) -> Self {
        let mut diagnostic = Self::new(
            "",
            0..0,
            Category::Absent,
            Cause::Empty,
            "required extracted numeric input",
        );
        diagnostic.field = field.into();
        diagnostic.observed_input = None;
        diagnostic.input_sha256 = None;
        diagnostic.span = None;
        diagnostic.normalised_span = None;
        diagnostic
    }
    pub fn selected(mut self, excerpt: &str, selection: Range<usize>) -> Self {
        if let Some(span) = &mut self.span {
            span.range = selection.start + span.range.start..selection.start + span.range.end;
            span.coordinate_space = "original_excerpt".into();
        }
        self.original_excerpt = Some(excerpt.into());
        self
    }
    pub fn render(&self) -> String {
        let source = if self
            .span
            .as_ref()
            .is_some_and(|s| s.coordinate_space == "original_excerpt")
        {
            self.original_excerpt
                .as_deref()
                .or(self.observed_input.as_deref())
        } else {
            self.observed_input.as_deref()
        };
        let location = self
            .span
            .as_ref()
            .map_or("no source span available".into(), |span| {
                format!(
                    "bytes {}..{} ({})",
                    span.range.start, span.range.end, span.coordinate_space
                )
            });
        format!(
            "{} {:?} {}: {:?}\n  {}\n  {} expected {}",
            self.code,
            self.category,
            self.field,
            self.cause,
            source.unwrap_or("<absent input>"),
            location,
            self.expected
        )
    }
}

pub(crate) const UNITS: [&str; 20] = [
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
pub(crate) const TENS: [&str; 8] = [
    "twenty", "thirty", "forty", "fifty", "sixty", "seventy", "eighty", "ninety",
];
pub(crate) const SCALES: [(&str, u64); 2] = [("thousand", 1_000), ("million", 1_000_000)];
const VOCAB: [&str; 34] = [
    "seventeen",
    "thirteen",
    "fourteen",
    "eighteen",
    "nineteen",
    "thousand",
    "million",
    "hundred",
    "fifteen",
    "sixteen",
    "seventy",
    "twenty",
    "thirty",
    "eighty",
    "ninety",
    "eleven",
    "twelve",
    "pounds",
    "pence",
    "penny",
    "forty",
    "fifty",
    "sixty",
    "seven",
    "eight",
    "three",
    "pound",
    "zero",
    "nine",
    "five",
    "four",
    "one",
    "two",
    "six",
];
#[derive(Debug, Clone)]
struct Token {
    word: String,
    span: Range<usize>,
}
type Input<'a> = &'a [Token];

fn word<'a>(input: &mut &'a str) -> winnow::ModalResult<&'a str> {
    // Vocabulary is longest first; joined OCR words use the same token grammar.
    alt((alt(VOCAB), "ten", "and")).parse_next(input)
}
fn lex(source: &str) -> Result<Vec<Token>, Diagnostic> {
    let folded = source.to_ascii_lowercase();
    let mut input = folded.as_str();
    let mut tokens: Vec<Token> = vec![];
    while !input.is_empty() {
        let at = source.len() - input.len();
        if let Some(c) = input.chars().next().filter(|c| c.is_whitespace()) {
            input = &input[c.len_utf8()..];
            continue;
        }
        if input.starts_with('-') || input.starts_with(',') {
            let prev = source[..at].chars().next_back();
            let next = input[1..].trim_start().chars().next();
            // Explicit word separators. Never remove signs, repeated punctuation or digits.
            let allowed_separator = input.starts_with('-')
                || tokens
                    .last()
                    .is_some_and(|t| matches!(t.word.as_str(), "million" | "thousand"));
            if allowed_separator
                && prev.is_some_and(|c| c.is_ascii_alphabetic())
                && next.is_some_and(|c| c.is_ascii_alphabetic())
            {
                input = &input[1..];
                continue;
            }
        }
        let token = word.parse_next(&mut input).map_err(|_| {
            let end = at + source[at..].chars().next().map_or(0, char::len_utf8);
            let category = if source[at..].starts_with(|c: char| c.is_alphabetic()) {
                Category::Unsupported
            } else {
                Category::Invalid
            };
            Diagnostic::new(
                source,
                at..end,
                category,
                Cause::UnexpectedToken,
                "English number words, spaces, word hyphens or scale commas",
            )
        })?;
        tokens.push(Token {
            word: token.into(),
            span: at..at + token.len(),
        });
    }
    if tokens.is_empty() {
        return Err(Diagnostic::new(
            source,
            0..source.len(),
            Category::Absent,
            Cause::Empty,
            "number expression",
        ));
    }
    Ok(tokens)
}
fn is_number(t: &Token) -> bool {
    UNITS.contains(&t.word.as_str())
        || TENS.contains(&t.word.as_str())
        || matches!(t.word.as_str(), "and" | "hundred" | "thousand" | "million")
}
fn number_words<'a>(input: &mut Input<'a>) -> winnow::ModalResult<&'a [Token]> {
    take_while(1.., |t: Token| is_number(&t)).parse_next(input)
}
fn pounds(input: &mut Input<'_>) -> winnow::ModalResult<()> {
    one_of(|t: Token| matches!(t.word.as_str(), "pound" | "pounds"))
        .void()
        .parse_next(input)
}
fn conjunction(input: &mut Input<'_>) -> winnow::ModalResult<()> {
    one_of(|t: Token| t.word == "and").void().parse_next(input)
}
fn pence<'a>(input: &mut Input<'a>) -> winnow::ModalResult<&'a [Token]> {
    let words = number_words.parse_next(input)?;
    one_of(|t: Token| matches!(t.word.as_str(), "pence" | "penny")).parse_next(input)?;
    Ok(words)
}
fn amount_grammar<'a>(
    input: &mut Input<'a>,
) -> winnow::ModalResult<(&'a [Token], Option<&'a [Token]>)> {
    let whole = number_words.parse_next(input)?;
    pounds.parse_next(input)?;
    // Once "and" appears, a complete pence expression is mandatory.
    let fraction = opt(alt((preceded(conjunction, cut_err(pence)), pence))).parse_next(input)?;
    Ok((whole, fraction))
}
fn syntax_error(source: &str, tokens: &[Token], offset: usize, expected: &str) -> Diagnostic {
    let span = tokens
        .get(offset)
        .map_or(source.len()..source.len(), |t| t.span.clone());
    Diagnostic::new(
        source,
        span,
        Category::Invalid,
        Cause::IncompleteExpression,
        expected,
    )
}
fn order_error(source: &str, tokens: &[Token]) -> Diagnostic {
    let span = tokens.first().map_or(0, |t| t.span.start)..tokens.last().map_or(0, |t| t.span.end);
    Diagnostic::new(
        source,
        span,
        Category::Unsupported,
        Cause::UnsupportedNumberOrder,
        "units, tens, hundreds and descending million/thousand groups",
    )
}
fn small(source: &str, tokens: &[Token]) -> Result<u64, Diagnostic> {
    if tokens.len() == 1
        && let Some(n) = UNITS.iter().position(|w| *w == tokens[0].word)
    {
        return Ok(n as u64);
    }
    if let Some(n) = tokens
        .first()
        .and_then(|t| TENS.iter().position(|w| *w == t.word))
    {
        let tens = (n as u64 + 2) * 10;
        if tokens.len() == 1 {
            return Ok(tens);
        }
        if tokens.len() == 2
            && let Some(u) = UNITS[1..10].iter().position(|w| *w == tokens[1].word)
        {
            return Ok(tens + u as u64 + 1);
        }
    }
    Err(order_error(source, tokens))
}
fn group(source: &str, tokens: &[Token]) -> Result<u64, Diagnostic> {
    if tokens.get(1).is_some_and(|t| t.word == "hundred") {
        let n = small(source, &tokens[..1])?;
        if !(1..=9).contains(&n) {
            return Err(order_error(source, tokens));
        }
        let mut rest = &tokens[2..];
        if rest.first().is_some_and(|t| t.word == "and") {
            rest = &rest[1..];
            if rest.is_empty() {
                return Err(order_error(source, tokens));
            }
        }
        return Ok(n * 100
            + if rest.is_empty() {
                0
            } else {
                small(source, rest)?
            });
    }
    small(source, tokens)
}
fn interpret(source: &str, tokens: &[Token]) -> Result<u64, Diagnostic> {
    let (mut total, mut start, mut previous) = (0u64, 0, u64::MAX);
    for (i, token) in tokens.iter().enumerate() {
        let Some(scale) = SCALES
            .iter()
            .find(|(word, _)| *word == token.word)
            .map(|(_, scale)| *scale)
        else {
            continue;
        };
        if scale >= previous {
            return Err(order_error(source, &tokens[i..]));
        }
        let mut segment = &tokens[start..i];
        if start > 0 && segment.first().is_some_and(|t| t.word == "and") {
            segment = &segment[1..];
        }
        let n = group(source, segment)?;
        total = n
            .checked_mul(scale)
            .and_then(|n| total.checked_add(n))
            .ok_or_else(|| {
                Diagnostic::new(
                    source,
                    token.span.clone(),
                    Category::NumericRange,
                    Cause::IntegerOverflow,
                    "u64 integer",
                )
            })?;
        start = i + 1;
        previous = scale;
    }
    let mut rest = &tokens[start..];
    if start > 0 && rest.first().is_some_and(|t| t.word == "and") {
        rest = &rest[1..];
        if rest.is_empty() {
            return Err(order_error(source, tokens));
        }
    }
    if rest.is_empty() && start > 0 {
        return Ok(total);
    }
    total.checked_add(group(source, rest)?).ok_or_else(|| {
        Diagnostic::new(
            source,
            0..source.len(),
            Category::NumericRange,
            Cause::IntegerOverflow,
            "u64 integer",
        )
    })
}
pub fn integer(source: &str) -> Result<u64, Diagnostic> {
    let tokens = lex(source)?;
    let words = number_words
        .parse(&tokens)
        .map_err(|e| syntax_error(source, &tokens, e.offset(), "complete English integer"))?;
    interpret(source, words)
}
pub fn amount(source: &str) -> Result<Decimal, Diagnostic> {
    let tokens = lex(source)?;
    let (whole, fraction) = amount_grammar.parse(&tokens).map_err(|e| {
        syntax_error(
            source,
            &tokens,
            e.offset(),
            "number words, pounds, optionally a complete number words and pence/penny suffix",
        )
    })?;
    let pounds = Decimal::from(interpret(source, whole)?);
    let pence = match fraction {
        None => Decimal::ZERO,
        Some(words) => Decimal::from_i128_with_scale(i128::from(interpret(source, words)?), 2),
    };
    pounds.checked_add(pence).ok_or_else(|| {
        Diagnostic::new(
            source,
            0..source.len(),
            Category::NumericRange,
            Cause::UnrepresentableDecimal,
            "exact Decimal amount",
        )
    })
}

fn digits<'a>(input: &mut &'a str) -> winnow::ModalResult<&'a str> {
    take_while(1.., |c: char| c.is_ascii_digit()).parse_next(input)
}
fn three_digit_group(input: &mut &str) -> winnow::ModalResult<()> {
    digits
        .verify(|group: &str| group.len() == 3)
        .void()
        .parse_next(input)
}
fn grouped_integer(input: &mut &str) -> winnow::ModalResult<()> {
    let start = *input;
    let first = digits.parse_next(input)?;
    if input.starts_with(',') && first.len() > 3 {
        *input = start;
        return Err(winnow::error::ErrMode::Cut(
            winnow::error::ContextError::new(),
        ));
    }
    while input.starts_with(',') {
        ','.parse_next(input)?;
        cut_err(three_digit_group).parse_next(input)?;
    }
    Ok(())
}
fn decimal_grammar(input: &mut &str) -> winnow::ModalResult<()> {
    if opt('£').parse_next(input)?.is_some() {
        take_while(0.., char::is_whitespace).parse_next(input)?;
    }
    opt(alt(('+', '-'))).parse_next(input)?;
    grouped_integer.parse_next(input)?;
    opt(preceded('.', cut_err(digits))).parse_next(input)?;
    Ok(())
}
pub fn decimal(source: &str) -> Result<Decimal, Diagnostic> {
    let trimmed = source.trim();
    let base = source.len() - source.trim_start().len();
    if trimmed.is_empty() {
        return Err(Diagnostic::new(
            source,
            0..source.len(),
            Category::Absent,
            Cause::Empty,
            "decimal",
        ));
    }
    decimal_grammar.parse(trimmed).map_err(|e| {
        let at = base + e.offset();
        let end = at + source[at..].chars().next().map_or(0, char::len_utf8);
        Diagnostic::new(
            source,
            at..end,
            Category::Invalid,
            Cause::UnexpectedToken,
            "signed decimal with optional £ and complete three-digit comma groups",
        )
    })?;
    let canonical = trimmed
        .trim_start_matches('£')
        .trim_start()
        .replace(',', "");
    Decimal::from_str_exact(&canonical).map_err(|_| {
        Diagnostic::new(
            source,
            base..base + trimmed.len(),
            Category::NumericRange,
            Cause::UnrepresentableDecimal,
            "Decimal coefficient within 96 bits and scale at most 28; no rounding",
        )
    })
}

/// Enclosing audit identities. Unknown build revision is explicit, never inferred from a PDF.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunIdentity {
    pub parser_policy: String,
    pub parser_source_sha256: String,
    pub cargo_lock_sha256: String,
    pub winnow_version: String,
    pub software_revision: Option<String>,
    pub software_dirty: Option<bool>,
    pub fixture_reference: String,
    pub fixture_sha256: Option<String>,
    pub extraction_sha256: String,
    pub page_text_sha256: String,
    #[serde(default)]
    pub replacements: Vec<ReplacementIdentity>,
}
/// Only adopted field groups are associated with an escalation extraction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplacementIdentity {
    pub field_group: String,
    pub extraction_sha256: String,
    pub fixture_reference: String,
    pub fixture_sha256: Option<String>,
}
impl RunIdentity {
    pub fn new(
        output: &serde_json::Value,
        doc: &crate::document::Document,
        fixture: &std::path::Path,
    ) -> Self {
        let page_text = serde_json::to_string(&doc.pages).expect("serializable page text");
        Self {
            parser_policy: POLICY.into(),
            parser_source_sha256: identity(concat!(
                include_str!("numeric.rs"),
                include_str!("text.rs"),
                include_str!("wire.rs"),
                include_str!("verify.rs"),
                include_str!("evidence.rs"),
                include_str!("extract.rs"),
                include_str!("schema.rs"),
                include_str!("report.rs")
            )),
            cargo_lock_sha256: identity(include_str!("../Cargo.lock")),
            winnow_version: "1.0.4".into(),
            software_revision: option_env!("S106_SOFTWARE_REVISION")
                .or(option_env!("S106_BUILD_REVISION"))
                .map(str::to_owned),
            software_dirty: option_env!("S106_BUILD_DIRTY").map(|s| s == "true"),
            fixture_reference: fixture.display().to_string(),
            fixture_sha256: std::fs::read_to_string(fixture).ok().map(|s| identity(&s)),
            extraction_sha256: identity(
                &serde_json::to_string(output).expect("serializable extraction"),
            ),
            page_text_sha256: identity(&page_text),
            replacements: vec![],
        }
    }
}

fn percent_unit(input: &mut &str) -> winnow::ModalResult<()> {
    "per".parse_next(input)?;
    take_while(0.., char::is_whitespace).parse_next(input)?;
    cut_err("cent").parse_next(input)?;
    Ok(())
}
fn percentage_suffix(input: &mut &str) -> winnow::ModalResult<()> {
    alt(('%'.void(), percent_unit)).parse_next(input)
}
/// Selected suffix syntax, including joined `percent`; complete input consumption is mandatory.
pub fn percent_suffix(source: &str) -> Result<(), Diagnostic> {
    let folded = source.to_ascii_lowercase();
    percentage_suffix.parse(folded.trim()).map_err(|e| {
        let at = source.len() - source.trim_start().len() + e.offset();
        Diagnostic::new(
            source,
            at..at,
            Category::Invalid,
            Cause::IncompleteExpression,
            "% or per cent",
        )
    })
}
