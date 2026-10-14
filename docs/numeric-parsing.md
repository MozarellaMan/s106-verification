# Numeric parsing and findings

Numbers enter the audit from two places: fields the extractor wrote (`amount_gbp`,
`amount_in_words`, `share_pct`, ...) and prose quoted as evidence. `quantity.rs` reads both. The
parser reads selected expressions only; it does not decide whether a PDF supports an extraction,
what a covenant means, or how to repair a source value. Nothing is guessed: a candidate that is
not a clean number is reported with a cause and a span, never silently fixed.

## Tokenizer and grammar

The lexer turns text into typed tokens with UTF-8 byte spans: digit runs, whitespace, number
words from a fixed vocabulary (longest first, so `seventeen` is not `seven` + `teen`), other
alphabetic runs, and symbols. A Winnow grammar over those tokens returns values, not booleans.
`scan`, `decimal`, `integer` and `amount` share the tokenizer and grammar, so the prose path and
the exact path cannot drift apart.

| Form | Supported | Reported as an error |
| --- | --- | --- |
| Digits | Optional sign, optional `£`, comma groups, fractional digits | Malformed groups (`1,23`), repeated decimal points, scientific notation |
| Words | English cardinals up to millions: any ASCII case, joined words, hyphens, `and`, commas between scale groups | Billions, fractions in words, other languages, repeated or misplaced conjunctions (`one one`, `and and`), scales out of order (`one thousand two million`) |
| Units | `£`, `pounds`, `%`, `per cent`, `percent` | Other currencies |

Values are `rust_decimal::Decimal` from `from_str_exact`: overflow or excess precision fails
with `UnrepresentableDecimal`, and nothing is rounded, saturated or clamped.

## Units

Each prose quantity carries a `Unit`: `Gbp` (a `£` prefix, or `pounds`), `Percent` (`%`,
`per cent`, `percent`) or `Count`. An ordinal such as `50th Dwelling` is not a share.
Grounding compares a value against quantities of the unit the field expects.

## Mixed letters and digits

OCR often swaps look-alike characters (`5o%`, `fif5ty`, `f10,058.60`). The scanner finds
letter/digit mixes and reports `MixedLettersAndDigits`, or `OcrSubstitutionCandidate` with the
normalised candidate when one exists. A candidate is a suggestion, never a reading: the scanner
produces no quantity for it.

## Exact and prose paths

The exact path reads a whole field as one expression. Leading and trailing whitespace is
allowed; anything else outside the expression is an error.

- `decimal` takes the digits field: no percent suffix and no words.
- `integer` takes words with no unit.
- `amount` takes words with pounds required and optional pence (`ten pounds and sixty
  pence`). A dangling `and` or an unknown pence word is an error, never dropped.

The prose path (`scan`) returns every quantity in running text and every rejected candidate,
each with its span. A single dot or comma at the end of a sentence is punctuation, not part of
the number. Quantity spans never overlap.

## Observed fields

Schema numeric fields are `Observed<T> { raw, parsed: Result<T, ParseError> }`. The extractor's
string is kept verbatim, surrounding whitespace included, next to the outcome. Absence is an
empty field, not a failed parse.

`ParseError` holds the rejected `candidate` span, the precise failure `span` (empty at end of
input), a `Cause` and what was `expected`. All spans are byte offsets into the text that was
scanned.

## Findings and codes

Each numeric problem becomes one `audit::Finding`, built in one place and used by both the
terminal report and the audit JSON (`verification[].findings`). Findings are extra facts attached
to a check; they do not change `status` or `error_class`.

A finding carries the check id, the field path, the code and category, the cause, what was
expected, the observed input with its SHA-256 and span, the citation, and `extraction_sha256`.

| Code | Category | Established fact |
| --- | --- | --- |
| NUM001 | absent | A required value is missing, or the parser input is empty |
| NUM002 | invalid | The input violates supported syntax |
| NUM003 | unsupported | A word or number order outside the bounded grammar |
| NUM004 | numeric_range | No exact representation exists |
| NUM005 | ambiguous_source | A letter/digit mix, substitution candidate, or a relocation that needed digit substitution |
| NUM006 | ungrounded | The value is not stated in its snippet, or the citation did not relocate |

The span's coordinate space is explicit: `observed_input` for an extractor field, `excerpt` for
a cited snippet. Neither is a position in the PDF. A finding without an observation (NUM001)
has no span.

## Grounding

For each numeric input the verifier locates the cited snippet on its page once
(`evidence::locate`), reads the snippet with `scan`, and checks that the claimed value occurs
among quantities of the right unit.

`Located` gives the page, the matched byte range in the page text and `repairs`: the OCR digit
substitutions the match relied on. A value is never verified against a page the matcher had to
rewrite, so a match with repairs is `RelocationRequiresDigitSubstitution` (NUM005) and stays
REVIEW.

A snippet whose numbers are unreadable yields one finding per unreadable number, in addition
to the reason. Citation matching itself is unchanged and is not a fuzzy score.

## Provenance

`agreement.run` (`audit::RunIdentity`) says what produced an audit:

- `parser_policy`.
- `crate_version` and `cargo_lock_sha256`.
- `software_revision`, from `S106_SOFTWARE_REVISION` at build time. It is null if unset and is
  never inferred from git.
- `fixture_reference` and `fixture_sha256`: the fixture the extraction was replayed from or
  recorded to. Both are null for an unrecorded live answer, even if a fixture exists at the
  usual path.
- `extraction_sha256` of the wire output and `page_text_sha256` of the page text used for
  grounding.
- `replacements`: when escalation adopts a field group (`housing`, `instrument` or a
  contribution name) from a stronger model, that group's own extraction and fixture hashes.

`Finding.extraction_sha256` names the extraction a finding's field came from: the latest
replacement for its field group, else the base extraction. Rejected candidates leave the
base provenance untouched.

When comparing two audit files:

- A changed PDF or page-text hash means the source changed.
- A changed extraction hash means the extraction changed.
- With source and extraction unchanged, a different outcome means the parser changed; the
  data tier's `expected/parser-on-extractions.txt` diff shows how on real strings. A field that
  now parses is not evidence that the extraction was fixed.

Fixtures and cached page text are replay inputs, not independent legal ground truth.

## Validating offline

```sh
cargo test
cargo clippy --all-targets -- -D warnings
cargo run -- audit path/to/deed.pdf --llm-mode replay --out /tmp/deed.audit.json
```

Never use Auto, Live or Record to validate a parser change, and never regenerate expected
values from parser output.
