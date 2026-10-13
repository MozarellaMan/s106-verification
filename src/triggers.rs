//! Deterministic payment-trigger vocabulary and payment-obligation wording.
//!
//! The label for a payment trigger is a function of the cited trigger phrase, not of
//! whichever backend (or model run) read the deed, so the same phrase always gets the same label.
//!
//! Matching runs on squashed text (lowercase alphanumerics only) because OCR drops
//! spaces ("onorbeforeOccupation"). The earliest match in the phrase wins; ties go
//! to the longest match. Order of PATTERNS does not matter.

use std::sync::LazyLock;

use regex::Regex;

use crate::schema::TriggerEvent;
use crate::text::squash;

const NUM: &str = r"(?:\d+|(?:one|two|three|four|five|six|seven|eight|nine|ten|eleven|twelve|thirteen|fourteen|fifteen|sixteen|seventeen|eighteen|nineteen|twenty|thirty|forty|fifty|sixty|ninety)+)";
const BEFORE: &str = r"(?:priorto|onorbefore|before|upon|on)(?:the)?";

pub static PATTERNS: LazyLock<Vec<(TriggerEvent, String)>> = LazyLock::new(|| {
    use TriggerEvent::*;
    vec![
        (PreImplementation, format!("{BEFORE}implementation")),
        (PreImplementation, r"not(?:to)?implement".into()),
        (PreCommencement, format!("{BEFORE}commencement")),
        (PreCommencement, r"not(?:to)?commence".into()),
        (PreOccupation, format!("{BEFORE}(?:first)?occupation")),
        (PreOccupation, r"not(?:to)?occupy".into()),
        // completion/date of *this agreement or deed* only; "on completion of construction" is not it
        (
            OnCompletion,
            r"(?:on|upon)(?:the)?(?:completion|date)of(?:this)?(?:agreement|deed)".into(),
        ),
        (
            OnCompletion,
            r"(?:on|upon)(?:the)?(?:completion|date)hereof".into(),
        ),
        (OnDemand, format!("within{NUM}(?:working)?days?")),
        (OnDemand, "ondemand".into()),
        (Other, "inaccordancewiththeprocedure".into()),
    ]
});
static COMPILED: LazyLock<Vec<(TriggerEvent, Regex)>> = LazyLock::new(|| {
    PATTERNS
        .iter()
        .map(|(e, p)| (*e, Regex::new(p).unwrap()))
        .collect()
});

/// TriggerEvent for the first trigger phrase in `text`, or None if none is recognised.
pub fn classify(text: Option<&str>) -> Option<TriggerEvent> {
    let text = text.filter(|t| !t.is_empty())?;
    let s = squash(text);
    let mut best: Option<(usize, std::cmp::Reverse<usize>, TriggerEvent)> = None;
    for (event, rx) in COMPILED.iter() {
        if let Some(m) = rx.find(&s) {
            let key = (m.start(), std::cmp::Reverse(m.len()));
            if best.is_none_or(|(start, len, _)| key < (start, len)) {
                best = Some((key.0, key.1, *event));
            }
        }
    }
    best.map(|(_, _, event)| event)
}

/// Label from the cited phrase, falling back to the surrounding clause.
pub fn classify_cited(phrase: Option<&str>, clause: Option<&str>) -> Option<TriggerEvent> {
    classify(phrase).or_else(|| classify(clause))
}

// Wording that imposes a payment, as opposed to merely mentioning one ("the Council shall assess
// the amount payable" schedules an assessment). Matched on squashed text, like PATTERNS.
const PAYER: &str = r"(?:shall|will|must|agrees?to|covenants?(?:with\w{0,40}?)?to|undertakes?to)";

/// (pattern, bare): bare "is payable" also gets the conditional-"if" check.
pub static OBLIGATIONS: LazyLock<Vec<(String, bool)>> = LazyLock::new(|| {
    vec![
        (format!("{PAYER}(?:pay|bepaid)"), false), // "the Owner shall pay"
        (
            r"(?:shall|will)(?:then)?(?:be|become)(?:due(?:and)?)?payable".into(),
            false,
        ), // "shall be payable"
        (r"(?:is|are|becomes?)(?:due(?:and)?)?payable".into(), true), // "is payable"
        (r"until\w*(?:has|have)beenpaid".into(), false), // "until ... have been paid"
    ]
});
static COMPILED_OBLIGATIONS: LazyLock<Vec<(Regex, bool)>> = LazyLock::new(|| {
    OBLIGATIONS
        .iter()
        .map(|(p, bare)| (Regex::new(p).unwrap(), *bare))
        .collect()
});

// Covenant list items ("1.3 To pay the ...", "Unless ..., to pay the ...") need the clause or
// comma boundary, which squashing removes, so this one runs on the raw text.
static COVENANT_ITEM: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?mi)(?:^|[,;:])\s*(?:\d[\d.]*\s*)?(?:\([a-z0-9]+\)\s*)?to\s*pay").unwrap()
});

// Question/assessment wording before a payment phrase ("assess whether ... is payable",
// "determine if ... is payable"). Squashed, so it holds whether or not OCR kept the spaces;
// "if" only counts straight after an assessment verb, since squashed "if" also sits inside
// words ("specified").
static QUESTION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"whether|(?:assess|determin|consider|decid|review|confirm|establish|calculat|verif|check|ascertain)\w{0,3}?if",
    )
    .unwrap()
});

// A segment opening with a conditional "If ..." (after any "3.1" / "(a)" numbering) makes a bare
// "is payable" a condition, not an obligation. Anchored at the start, so "Ifthe" still counts.
static CONDITIONAL_IF: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^\s*(?:\d[\d.]*\s*)?(?:\([a-z0-9]+\)\s*)?if").unwrap());

/// "If any ..." does not introduce the conditional assessment handled here.
fn conditional(seg: &str) -> bool {
    CONDITIONAL_IF.find(seg).is_some_and(|m| {
        let rest: String = seg[m.end()..].trim_start().chars().take(3).collect();
        !rest.eq_ignore_ascii_case("any")
    })
}

/// Is the payment phrase at `pos` of squash(seg) asked about rather than imposed?
fn asked(seg: &str, pos: usize, bare: bool) -> bool {
    QUESTION.is_match(&squash(seg)[..pos]) || (bare && conditional(seg))
}

/// Sentence or sub-clause breaks; a full stop inside a number ("1.3", "£10,058.60") is not one.
fn segments(clause: &str) -> Vec<String> {
    let chars: Vec<char> = clause.chars().collect();
    let mut out = vec![String::new()];
    for (i, &c) in chars.iter().enumerate() {
        let in_number = (i > 0 && chars[i - 1].is_ascii_digit())
            || chars.get(i + 1).is_some_and(|n| n.is_ascii_digit());
        if c == ';' || (c == '.' && !in_number) {
            out.push(String::new());
        } else {
            out.last_mut().unwrap().push(c);
        }
    }
    out
}

/// Does the clause oblige someone to pay (or make a sum payable), not just mention or ask about it?
pub fn imposes_payment(clause: Option<&str>) -> bool {
    segments(clause.unwrap_or("")).iter().any(|seg| {
        let sq = squash(seg);
        COMPILED_OBLIGATIONS
            .iter()
            .any(|(rx, bare)| rx.find_iter(&sq).any(|m| !asked(seg, m.start(), *bare)))
            || COVENANT_ITEM
                .find_iter(seg)
                .any(|m| !QUESTION.is_match(&squash(&seg[..m.start()])))
    })
}
