# Numeric parsing and diagnostics

The numeric parser reads selected expressions. It does not establish that an extraction is
supported by a PDF, decide what a covenant means, or repair source values. `numeric` owns syntax
and exact interpretation; `text` locates candidates; `wire` converts extracted fields;
`verify` retains the existing grounding, domain checks and status rules; `report` renders the
same structured facts that audit JSON contains.

## Grammar ownership

`numeric::integer`, `amount`, `decimal` and `percent_suffix` return typed failures. The older
Option/bool helpers remain as compatibility interfaces, but production conversion and
verification retain the failures. All selected expression parsers consume their entire input.

The money grammar is read in document order:

```rust
let whole = number_words.parse_next(input)?;
pounds.parse_next(input)?;
let fraction = opt(alt((
    preceded(conjunction, cut_err(pence)),
    pence,
))).parse_next(input)?;
```

`number_words`, `pounds`, `conjunction`, and `pence` are named helpers. A present conjunction
commits to a complete pence expression. A pence expression without a conjunction is also
supported. The outer `.parse(...)` requires end of input. Interpretation of units, tens,
hundreds and descending scale groups remains ordinary Rust, with checked scale arithmetic.
There is no combinator-based arithmetic or implicit numerical recovery.

Previously `letters_only` removed every digit and punctuation character before tokenisation:
`fifty-eight` and corrupted `fifty8eight` both became `fiftyeight`. Now the first reads 58 and
the second fails at the digit. Previously the money suffix branch treated `ten pounds and`
as £10. Now that branch requires a number and pence/penny. `ten pounds and zero pence` still
reads exactly £10. Before wire conversion, `1,23` became 123 by removing commas; now named
`grouped_integer` / `three_digit_group` grammar helpers reject the malformed grouping.

### Compatibility decisions and individual changes

1. Joined number words, ASCII case, whitespace and word hyphens remain supported. The fixed
   vocabulary is longest-first, so `seventeen` is not split into `seven`. No digit substitutions
   happen in expression parsing. The existing prose convention of OCR currency `f` before
   digits remains supported (`f10,058.60`); it does not change any numeric digit.
2. Commas between million/thousand groups are explicit syntax. Recorded forms such as
   `one million, six hundred and eleven thousand and five hundred and two pounds` and
   `five million and three hundred and thirty six thousand and five hundred pounds` remain
   accepted. The prose locator also reads such complete comma-separated scale groups as one
   number; previously it split them. This is an explicit acceptance expansion for numeric
   grounding when only words are cited, and must trigger the possible-weakening review below
   when compared with an unchanged older input. Arbitrary commas such as `twenty,five` are rejected.
3. The former additive interpreter could make `one one` equal 2 and ignore misplaced or
   repeated conjunctions. These forms now report unsupported number order. A hundreds group
   has one unit (one through nine) followed by hundred and an optional complete remainder.
   Million/thousand groups must descend. Billion, fractional words, alternative currencies,
   scientific notation and other languages remain outside the supported grammar.
4. A dangling `and`, unknown/incomplete pence, unexpected trailing input, embedded digits,
   slashes and other unrecognised punctuation cannot be discarded. This also applies to
   located word expressions: `one hundred and` no longer establishes 100. Tests retain these
   old defect inputs as negative controls rather than editing them into easier cases.
5. Digit expressions support signs, optional £ (including following whitespace), proper
   comma groups and fractional digits. Internal whitespace between digits is rejected.
   Prose selection retains malformed decimal/grouping runs and signs, including a Unicode
   minus as unsupported input. A single sentence-final dot/comma can be selected out as prose
   punctuation; it is not accepted by the entire-expression decimal parser. Repeated decimal
   points, malformed comma groups and spaced signs are not split into acceptable fragments.
6. Decimal conversion uses `Decimal::from_str_exact`. Overflows and over-precision input fail;
   conversion never rounds, saturates or clamps. This does not remove the verifier's existing
   explicit percentage comparison tolerances. Money reporting retains significant fractional
   digits beyond two places, so a precise mismatch cannot disappear in rendering.
7. Complete `%`, `per cent` and joined `percent` suffixes are supported. `per cento`,
   `percentbanana` and repeated percent signs fail. An ordinal such as `50th Dwelling` does
   not establish a share.
8. Existing OCR digit repair remains available as a normalisation tool and inside the bounded
   citation relocation algorithm. A repaired numeric reading no longer establishes a verified
   value. Numeric grounding also requires relocation to work without digit substitution.
   Otherwise the established PASS/FAIL/REVIEW decision procedure requests review. No generic
   fuzzy matching was introduced.
9. Empty optional numeric extraction fields still represent absence. Invalid nonempty fields
   retain their original strings and failures before conversion to None. An existing failed
   check receives its conversion diagnostic; if its ordinary outcome was SKIP/REVIEW, an
   additional `*.parse` FAIL / Parser Error check prevents silent success of invalid conversion.
   Legitimate figureless formulas retain their existing treatment.
10. Extracted words fields now retain leading/trailing whitespace verbatim, as citations already
    did. Parsers accept surrounding whitespace without destroying the original observed input.

11. Money display still uses at least two decimal places, but preserves additional significant
    digits. A grounded figures/words mismatch of £10.001 versus £10 now prints £10.001 versus
    £10.00, rather than printing both sides as £10.00. Comparisons were already exact; this
    improves the diagnostic presentation without changing the comparison or error class.

The prototype's pence 0..99, percentage 0..100 and two-decimal-place policies were not adopted.
For example, `ten pounds and one hundred pence` still interprets as £11, and a representable
percentage with more than two fractional digits can parse. Existing verification rules decide
whether a parsed value supports a domain claim. Overflowing repeated-hundred expressions are
rejected as unsupported structure before arithmetic, rather than used as an overflow mechanism.

## Audit contract

Diagnostics are additive facts, separate from `status` and `error_class`:

| Code | Category | Established fact | Consumer action (a suggestion) |
| --- | --- | --- | --- |
| NUM001 | absent | No required verification value, or empty selected parser input | Locate a source-backed extraction; stop if it is genuinely unstated |
| NUM002 | invalid | Selected input violates supported syntax | Inspect the observed input and citation; correct extraction only with source support |
| NUM003 | unsupported | Vocabulary or number order is outside this bounded grammar | Escalate the syntax with a source example and clean/corrupted controls |
| NUM004 | numeric_range | Exact representation is unavailable | Stop conversion; investigate representation or transcription; never round to pass |
| NUM005 | ambiguous_source | A substitution candidate exists, or relocation needs one | Inspect PDF/source text/OCR; retain REVIEW until uncertainty is resolved |
| NUM006 | ungrounded | Value is not stated in its snippet, or citation did not relocate | Investigate the citation/field and claimed page; parsing success is insufficient |

`cause.kind` supplies the structured underlying fact. A syntax error does not identify OCR or
model failure. `ocr_substitution_candidate.normalised` is a candidate, not a proven correction.
`citation_relocated` is a Boolean result of the existing relocation algorithm, not proof of
legal interpretation or a PDF character position. The unmodified-digit grounding check is
additional to that Boolean. Re-running an unchanged deterministic parser cannot resolve a
syntax error.

Each diagnostic carries field/check identity, observed input when available, its SHA-256,
original cited excerpt when available, expected syntax/condition, and the claimed page.
Spans are nullable: missing verification inputs have neither an observed string nor a span.
All available spans are UTF-8 byte ranges. `original_observed_input` coordinates refer to a
local extraction field or observed citation string; `original_excerpt` coordinates refer to
selected prose in that excerpt. Neither is a PDF/page text span. `normalised_span` refers to
ASCII case folding of the selected observed input, which preserves byte lengths. It stays
local when an original span is translated into surrounding prose. OCR candidates have no
normalised span mapping. Whole-snippet grounding spans identify context, not the location of
an absent value. Zero-length parser spans identify a boundary or end of input.

The enclosing agreement provenance holds the PDF hash, path, text source and OCR version.
`agreement.parser_run` holds fixture reference/hash, canonical extraction hash, cached page
text/layout hash, parser policy/source hash, Cargo.lock hash, pinned Winnow version, compiled
base revision and dirty flag. Source hashes identify uncommitted parser builds; a base revision
alone does not claim that a dirty build equals that commit. Unavailable build or fixture
identities are null. Adopted escalation field groups are recorded in `replacements`; their
failures reference the corresponding extraction hash. A failed escalation does not overwrite
original provenance. Model fixtures and cached page text are replay inputs, not independent
legal ground truth.

### Actual replay example

Salford 20/75239, PDF page 24, recorded extraction: the cited snippet relocates but does not
state the extracted Shared Ownership count of 3. The production terminal report includes:

```text
[REVIEW] H2  Sum of tenure units == affordable units
NUM006 Ungrounded Shared Ownership units: ValueNotStated { expected_value: "3" }
  That all the Affordable Housing to be provided on the Site or if applicable in that Phase shall be Intermediate Housing Shared Ownership tenure and:
  bytes 0..148 (original_observed_input) expected extracted value stated in cited snippet
```

The same diagnostic in audit JSON (selected fields):

```json
{
  "code": "NUM006",
  "category": "ungrounded",
  "field": "Shared Ownership units",
  "check_id": "H2",
  "observed_input": "That all the Affordable Housing to be provided on the Site or if applicable in that Phase shall be Intermediate Housing Shared Ownership tenure and:",
  "input_sha256": "5334d2fe09cd2819ed95a8811333e59c5d0f1c7e35da503b00897cdafd04616c",
  "span": {"range": {"start": 0, "end": 148}, "units": "utf8_bytes", "coordinate_space": "original_observed_input"},
  "cause": {"kind": "value_not_stated", "expected_value": "3"},
  "claimed_page": 24,
  "citation_relocated": true
}
```

This establishes a citation-support failure, not an incorrect dwelling count or an OCR cause.
The source must be inspected before changing extraction. Existing REVIEW status and exit code
1 remain unchanged.

## Retry consumers and weakening detection

Consumers read `verification[].diagnostics` directly, not terminal formatting. Group by PDF
hash, field/check identity, code, cause kind and `input_sha256`; use `extraction_sha256` to
reference the enclosing run or adopted replacement. Messages are presentation, not identifiers.

For two audit snapshots, compare provenance before deciding what a later success means:

- Changed PDF hash or page-text hash: source input changed; inspect the source change.
- Changed extraction hash or adopted field-group hash: extraction changed. Reparse and repeat
  grounding; a successful correction must leave corrupted counterparts rejected.
- Unchanged source/extraction but changed parser source/policy or lock hash: acceptance changed.
  A previously rejected field becoming accepted is **possible weakening**, not evidence that
  the source/extraction was fixed. Stop promotion until a source-backed grammar review and
  negative controls justify it. For dirty builds compare source hashes, not only base revision.
- All identities unchanged: a deterministic retry should reproduce the same parser facts.
  A changed outcome needs investigation, not another blind retry.

The reporting controls demonstrate `1,0` failing conversion, corrected extraction `10`
reparsing and grounding against `£10 (ten pounds)`, and malformed pence/digit/trailing-input
counterparts still failing. The page-text hash stays unchanged while extraction hash changes.
A changed but readable extraction `11` / `eleven pounds` still fails grounding against that
same source. `£5o` preserves uncertainty; a `£50` citation matching only a repaired `£5o` page
also remains REVIEW. These are constructed controls, not invented goldens for real deeds.

## Reproducible offline validation

```sh
cargo test --locked --offline --tests
cargo clippy --locked --offline --all-targets -- -D warnings
S106_DIAGNOSTIC_DEMO_DIR=/tmp/s106-diagnostic-demo cargo test --locked --offline --test numeric_reporting
# With the local corpus, PDFium and cached OCR available:
cargo test --locked --offline --test golden_python_m1 --test ocr_gold -- --ignored --nocapture
cargo run --locked --offline -- audit path/to/deed.pdf --llm-mode replay --out /tmp/deed.audit.json
```

Set `S106_MODELS` and `S106_OCR_CACHE` to local model/cache directories as needed. Never use
Auto/Live/Record to validate this migration. No goldens should be regenerated from parser output.
The recorded-words test declares a denominator of 70 across v1/v2 fixtures and retains 70
appended-garbage negative twins. Full PDF replay still checks fixture PDF/prompt identities.

Remaining boundaries include the deliberately bounded English vocabulary, conservative prose
candidate selection, no PDF glyph span mapping, and Boolean evidence relocation. Multiline
source is preserved in plain rendering; no Ariadne/Miette dependency or alternate diagnostic
facts are required. The corpus demonstrates coverage of those deeds, not every development PDF.
