# Numeric parsing and findings

Numbers enter the audit from two places: fields the extractor wrote (`amount_gbp`,
`amount_in_words`, `share_pct`, ...) and prose quoted as evidence. Both are read by
`quantity.rs`. The parser reads selected expressions; it does not decide that an extraction is
supported by a PDF, what a covenant means, or repair source values. Nothing is guessed: a
candidate that is not a clean number is reported with a cause and a span, never silently fixed.

## Tokenizer and grammar

Text is lexed once into typed tokens with UTF-8 byte spans: digit runs, whitespace, number
words from a fixed vocabulary (longest first, so `seventeen` is not `seven` + `teen`), other
alphabetic runs, and symbols. A Winnow grammar over those tokens returns values, not
booleans. `scan`, `decimal`, `integer` and `amount` share the tokenizer and grammar, so the
prose path and the exact path cannot drift apart.

| Form | Supported | Reported as unsupported or invalid |
| --- | --- | --- |
| Digits | Optional sign, optional `£`, proper comma groups, fractional digits | Malformed groups (`1,23`), repeated decimal points, scientific notation |
| Words | English cardinals up to millions: ASCII case, joined words, hyphens, `and`, commas between scale groups | Billions, fractions in words, repeated or misplaced conjunctions (`one one`, `and and`), scales out of descending order (`one thousand two million`), other languages |
| Units | `£`, `pounds`, `%`, `per cent`, joined `percent` | Other currencies |

Values are `rust_decimal::Decimal` from `from_str_exact`: overflow or over-precision fails
(`UnrepresentableDecimal`), and nothing is rounded, saturated or clamped.

## Units

Each prose quantity carries a `Unit`: `Gbp` (a `£` prefix, or `pounds`), `Percent` (`%`,
`per cent`, `percent`) or `Count`. An ordinal such as `50th Dwelling` is not a share.
Grounding compares a value against quantities of the unit the field expects.

## Mixed letters and digits

OCR often swaps look-alike characters (`5o%`, `fif5ty`, `f10,058.60`). The scanner finds
letter/digit mixes and reports `MixedLettersAndDigits`, or `OcrSubstitutionCandidate` with the
normalised candidate when one exists. A candidate is a suggestion, never a reading: the
quantity is not produced.

## Exact and prose paths

- Exact (`decimal`, `integer`, `amount`): the whole field is one expression; leading and
  trailing whitespace is allowed, anything else outside it is an error. `decimal` takes the
  digits field (no percent suffix, no words); `integer` takes words with no unit; `amount`
  takes words with pounds required, with optional pence (`ten pounds and sixty pence`). A
  dangling `and` or an unknown pence word is an error, never dropped.
- Prose (`scan`): every quantity in running text and every rejected candidate, each with its
  span. A single sentence-final dot or comma is punctuation, not part of the number.
  Quantity spans never overlap.

## Observed fields

Schema numeric fields are `Observed<T> { raw, parsed: Result<T, ParseError> }`. The extractor's
string is kept verbatim (surrounding whitespace included) next to the outcome. Absence is an
empty field, not a failed parse. `ParseError` holds the rejected `candidate` span, the precise
failure `span` (empty at end of input), a `Cause` and what was `expected`; all spans are byte
offsets into the text that was scanned.

## Findings and codes

Each numeric problem becomes one `audit::Finding`, built in one place and used by both the
terminal report and the audit JSON (`verification[].findings`). A finding carries the check
id, the field path, the code and category, the cause, what was expected, the observed input
with its SHA-256 and span, the citation, and `extraction_sha256`. Findings are additive facts;
they do not change `status` or `error_class`.

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
among quantities of the right unit. `Located` gives the page, the matched byte range in the
page text and `repairs`: the OCR digit substitutions the match relied on. A value is never
verified against a page that had to be rewritten; a match with repairs is
`RelocationRequiresDigitSubstitution` (NUM005) and stays REVIEW. A snippet whose numbers are
unreadable yields one finding per unreadable number in addition to the reason. Citation
matching itself is unchanged and is not a fuzzy score.

## Provenance and the snapshot gate

`agreement.run` (`audit::RunIdentity`) records what produced an audit:

| Field | Meaning |
| --- | --- |
| `parser_policy`, `parser_fingerprint` | The number-reading policy name, and a hash of parser results on the probe corpus |
| `crate_version`, `cargo_lock_sha256` | The build's version and dependency lock |
| `software_revision` | `S106_SOFTWARE_REVISION` at build time; null if unset, never inferred from git |
| `fixture_reference`, `fixture_sha256` | The fixture that supplied the extraction (replay) or was written from it (record); null for an unrecorded live response |
| `extraction_sha256`, `page_text_sha256` | Hashes of the wire output and of the page text used for grounding |
| `replacements` | For each field group escalation adopted (`housing`, `instrument` or a contribution name), that group's own extraction and fixture hashes |

`Finding.extraction_sha256` names the extraction a finding's field came from: the latest
replacement for its field group, else the base extraction. Rejected candidates leave the
base provenance untouched.

### The parser fingerprint

`fixtures/numeric-probes.txt` holds one probe per line, with newline, carriage return and
backslash escaped. The probes are every citation snippet and every `amount_in_words`,
`amount_gbp`, `share_pct` and `claimed_affordable_pct` string in the recorded fixtures, plus
constructed controls.

`audit::parser_fingerprint()` runs `scan`, `decimal` and `amount` over every probe. It renders
quantities with unit and span, and errors with cause, spans and diagnostic wording, then
hashes the rendering. A comment or refactor leaves it unchanged.

The fingerprint covers only the probe corpus. It changes when parser results on those probes
change, including diagnostic wording, and when probes are added or removed. A change in what
the parser accepts on inputs outside the corpus can leave it unchanged.

### The snapshot gate

`tests/numeric_snapshot.rs` snapshots the same rendering with `insta` (`tests/snapshots/`).
A change that accepts something previously rejected shows up as a snapshot diff. Justify it
with a source example and a corrupted counterpart that must still fail.

The same file has a property test: `scan` never panics on arbitrary text, every span is on
char boundaries and inside the text, and quantity spans do not overlap.

`probes_cover_fixtures` fails when a fixture string is missing from the probe file. To add
the missing strings and review their effect:

1. Regenerate the probe file:

   ```sh
   S106_REGENERATE_PROBES=1 cargo test --test numeric_snapshot probes_cover_fixtures
   ```

2. Rerun the snapshot test. The test binary embeds the probe file at compile time, so this
   run rebuilds it with the new probes and records the snapshot diff:

   ```sh
   cargo test --test numeric_snapshot parser_behaviour_on_probes
   ```

3. Review the diff:

   ```sh
   cargo insta review
   ```

### Comparing two audits

- A changed PDF or page-text hash means the source changed.
- A changed extraction hash means the extraction changed.
- A changed `parser_fingerprint` means parser results on the probe corpus changed. That can be
  an acceptance change, a diagnostic change or a change to the probes themselves; the snapshot
  diff between the two builds says which.
- An unchanged `parser_fingerprint` does not prove acceptance is unchanged outside the corpus.

A field that now parses is not, by itself, evidence that the extraction was fixed. Fixtures and
cached page text are replay inputs, not independent legal ground truth.

## Validating offline

```sh
cargo test
cargo clippy --all-targets -- -D warnings
cargo run -- audit path/to/deed.pdf --llm-mode replay --out /tmp/deed.audit.json
```

Never use Auto, Live or Record to validate a parser change, and never regenerate expected
values from parser output.
