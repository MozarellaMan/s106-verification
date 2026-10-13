# Rust maintainability review

Assessment date: 13 October 2026. Baseline: `e938307` (`rust-migration`).
This assessment was written before implementation changes. Locations below name
functions at that revision; the implementation record at the end will distinguish
completed work from follow-ups.

## Assessment

The single-crate layout is appropriate. The domain model, transport schema,
PDF/OCR adapter, evidence matcher and verifier have distinct jobs. Ordinary structs,
enums, Decimal arithmetic, loops and narrowly scoped regexes dominate. There is no
justification for a framework, more crates, or a directory-wide rearrangement.
The verifier's length mostly reflects separate named checks; splitting it solely
by line count would not improve the reading path.

Read: every `src/*.rs`, both examples, all integration tests, README, the three
existing documents, model-fetch script, both OCR gold files and relevant recorded
fixture passages. Fixture replay will exercise the complete recorded outputs.
This is a source review, not a fresh manual transcription of the legal corpus.

### Essential: correctness and integrity

1. **Instalment shares are never grounded** — `verify::check_instalments`.
   It calls `resolve` with no numerical inputs and only tests phrase relocation.
   `tests/verify.rs::inst_shares_not_summing_to_100_with_grounded_phrases_is_discrepancy`
   even expects a document discrepancy for extracted 50 + 40 against a page saying
   50 + 50. Route each share through `Input`/`resolve`, as housing already does.
   Add both invented-but-consistent and genuinely inconsistent source controls.
   This intentionally moves unsupported PASS to REVIEW and unsupported discrepancies
   to Parser Error; it preserves the status definitions.

2. **Number normalisation can make changed figures match** —
   `evidence::evidence_found`, `tokens`, `Layout::occurrences`.
   The contiguous fast path uses `squash(page).contains(squash(snippet))` without
   numeric boundaries. A snippet `50` can match page `150`, and `£1.00` and `£100`
   collapse to the same digits. The fallback also discards decimal punctuation.
   Add adversarial controls before changing this: retain numeric identity and
   boundaries on both paths without broadening fuzzy matching. This is sensitive
   code: real OCR replay must accompany the fix.

3. **Unknown pence becomes zero** — `text::words_to_amount`.
   `words_to_int(&pence_txt).unwrap_or(0)` turns an unparseable suffix into a valid
   whole-pound value; `trim_matches` tests a character set, not the word "and".
   Reject unparseable nonempty suffixes; retain the existing empty/trailing-and
   convention. Likewise replace saturation in `words_to_int` with checked arithmetic:
   overflow is unknown, not a valid clamped amount. Add regression cases. No generic
   parser should silently broaden the accepted vocabulary.

4. **Rates and formula parameters enter the stated-sum total** —
   `Agreement::total_stated_gbp` and `eval::score_against_register`.
   Both collect every `value_gbp`, although a rate is not a contribution total.
   Restrict stated-sum aggregation/scoring to Fixed and Cap. Keep rates visible in
   the individual contribution report. Test mixed structures. This changes totals
   only for such inputs and should be a separate behavioural commit.

5. **Escalation's advertised safeguards are incomplete** — `extract::lost_housing`,
   `lost_contribution`, `tally`, `escalate`.
   The presence checks omit percentage/basis, total rooms, per-tenure values,
   conditions, instalments and variation contents. Counts of checks do not prove
   that the same facts survived. A same-length tenure list can replace a different
   tenure; duplicate contribution names are also conflated. Accepted replacements
   retain the original `Provenance.extracted_by`, with only a warning naming the
   requested escalation model. Add field-preservation and merged-result tests
   before relying on escalation for research. A full provenance/preservation change
   needs its own focused follow-up, not a cosmetic rewrite. No escalation tests
   existed at this baseline despite the migration document claiming them.

6. **Several PASS labels have narrower meaning than the documentation implies** —
   `verify::check_instrument`, `check_variation`, `check_mix_table`, `check_trigger`,
   `check_instalments`. V1 only relocates the parent's citation, not its description
   or date. V2 accepts the first matching target token and never grounds new wording
   or operation. Table grounding checks a multiset of numbers on a line, not column
   identity or row labels. Payment grounding does not prove that the covenant
   belongs to the named contribution (the North Tyneside Ecology fixture cites the
   Children's Equipped Playspace covenant). Document these boundaries now; stronger
   semantic association needs labelled cases and explicit schema decisions.
   Do not present these checks as confirmation of the complete legal obligation.

### Worthwhile: maintenance

7. **The common status decision is buried in repeated constructors** —
   `verify::resolve`. Keep the early missing-input failure, then make the four
   (assertion holds, grounded) outcomes one explicit match and build the result once.
   Preserve every existing status, error class and message. Existing outcome tests
   and frozen replay are suitable regression checks.

8. **Repeated audit usage-note construction** — `main` and `eval::run_once`.
   Move the identical formatting onto `extract::Info`, which already owns source,
   model and usage. Do not introduce a reporting service. Remove unnecessary clones
   of complete JSON parts when composing the three responses; owned responses can
   be moved. These changes have no intended effect on saved output.

9. **Nested deserialisation errors lose the field path** — `extract::to_agreement`.
   A generic "invalid type" in a large response leaves the maintainer searching the
   whole JSON tree. Trial `serde_path_to_error` on one valid and one corrupted wire
   fixture. Adopt only if it gives an actionable path with a small dependency cost;
   keep Serde responsible for type validation and the verifier for evidence.

10. **Documentation mixes old architecture and current guarantees** — module
    headers in `text`, `triggers`, `evidence`, `verify`, `eval`; `wire` says "Both
    calls" and "cached PDF", while there are three uncached calls. README and
    schema-v2 claim replacement wording is grounded. Correct those statements;
    retain legal/OCR explanations and keep migration history in its own document.
    Add `docs/architecture.md` with the actual trace and extension path.
    Wire doc comments are schema descriptions: editing them changes the prompt hash,
    so leave schema-generating comments alone in a readability-only pass.

### Follow-ups and justified complexity

- `eval::housing_print` omits percentage basis/total rooms; fingerprints omit
  instalment shares and collapse equal normalised names. "Runs agree" covers only
  selected values, not entire obligations. Extend the comparison contract separately.
- `document::load_pages` filters invalid requests silently, counts duplicate requests
  as missing, trusts cached page IDs, and does not validate an existing manifest.
  OCR version uses a model name rather than artifact bytes. The download script pins
  files, but custom/changed runtime files can reuse a cache. Cache integrity deserves
  targeted tests; don't claim the current key detects arbitrary local replacement.
- `Config::pdfium` performs binding before `OnceLock::set`, so simultaneous first
  callers can race, and the first model directory wins process-wide. Current CLI
  execution is sequential. Fix initialisation before offering parallel callers.
- `data/reference/` is ignored and has no tracked files at the baseline, including
  the candidate TSV and register CSVs required by `eval`. The local replay can run,
  but a clean clone needs these inputs supplied separately. Commit an appropriate
  retrieval manifest or document their reproducible acquisition in a follow-up.
- `wire::money` turns malformed nonempty decimals into missing fields; H4 can then
  SKIP. Stronger input validation would require returning conversion errors rather
  than only `Option`, with a deliberate fixture compatibility policy.
- `verify::cell` and table column sums use `u32` despite model-controlled counts;
  unlike H1–H3 they can overflow. Widen before using untrusted large tables.
- `llm::Usage` keeps only successful-attempt token usage; combined extraction keeps
  only the first served-model name. Cost tiers apply to summed rather than per-call
  input (already noted in migration.md). Preserve per-call records in a future
  fixture format rather than inventing missing provenance for old fixtures.
- PDF/OCR has native runtime costs, but caches and bounded rendering batches are
  already present. No measured case here justifies new concurrency or caches.
  Repeated page normalisation is a profiling candidate, not a demonstrated bottleneck.
- H4 lacks evidence for scheme-wide habitable rooms; complete grounding cannot be
  achieved by arithmetic. Coverage of omitted obligations and delivery reconciliation
  remain research milestones, not refactoring tasks.

Purely stylistic changes declined: replacing readable loops with combinators,
wrapping every primitive, converting every public helper to a trait, eliminating
small evidence clones with pervasive lifetimes, splitting checks into tiny modules,
or adding a builder merely to silence `too_many_arguments`.

## Rust ecosystem opportunities

Research sources and trial results are recorded below as they are checked. No
candidate is accepted solely because its API looks shorter.

| Existing code | Candidate crate | Expected benefit | Recommendation |
|---|---|---|---|
| `extract::to_agreement` JSON conversion | `serde_path_to_error` 0.1.20 | Exact nested failure location without handwritten traversal | Adopt after the valid/corrupt fixture trial below |
| `text::words_to_int` / `words_to_amount` | `text2num` 2.8.0 | Maintained grammar for number words | Keep existing OCR and pounds/pence handling; investigate only a narrowly defined grammar replacement |
| `report::gbp`, `verify::gbp`, `eval::thousands` | `num-format` 0.4.4 | Integer grouping and locale support | Keep existing; decimals/sign placement remain custom and no locale requirement exists |
| `ocrbench::semi_global` | `bio` 4.0.1 / `rapidfuzz` 0.5.0 | Established alignment/string-distance implementations | Keep existing small character-based recurrence; global edit distance is not this metric |

The evidence matcher's number-preservation rules, OCR digit repair, margin filtering,
trigger vocabulary, payment/assessment distinction, and basis/variation gating are
project-specific. Generic fuzzy similarity must not replace them.

### Ecosystem evidence and trials

Checked on 13 October 2026 against the registry links, docs.rs release pages and
upstream repositories. Popularity figures are GitHub stars, not quality guarantees.
GitHub `pushed_at` is activity metadata, not evidence of a particular reviewed commit.
The crates.io HTML pages require JavaScript; release/dependency details below come
from their published docs.rs packages and the resolved local dependency tree.

- **serde_path_to_error 0.1.20:** [published package](https://docs.rs/crate/serde_path_to_error/0.1.20),
  [registry](https://crates.io/crates/serde_path_to_error),
  [upstream](https://github.com/dtolnay/path-to-error). Releases since 2019; latest
  published release shown is 15 September 2025; upstream last push 24 June 2026,
  427 stars. MIT/Apache-2.0. Trialled on Rust 1.99 with this project's Serde types.
  In the isolated trial, plain Serde reported `invalid type ... expected a string`;
  the wrapper added `contributions[0].amount_gbp`. Both accepted the valid twin.
  Adoption adds one lockfile package; `cargo tree -p serde_path_to_error` resolves
  only existing `itoa` and `serde_core` beneath it. No native library or build
  system. Two conversion call sites use it directly; no new abstraction layer.
  `recorded_wire_outputs_deserialize_and_errors_identify_the_field` preserves a
  valid/corrupt test over all ten v2 fixtures. The final test corrupts
  `housing.total_units`, because some variation fixtures contain no contributions.

- **text2num 2.8.0:** [published package](https://docs.rs/crate/text2num/2.8.0),
  [registry](https://crates.io/crates/text2num),
  [upstream](https://github.com/allo-media/text2num-rs). MIT; last release/push
  21 August 2026, 9 stars; direct dependencies `bitflags`, `daachorse`, `phf`.
  The isolated Rust trial returned 58 for `fifty eight`, but errors for
  `fiftyeight`, `one hundred and`, and `ten pounds and twenty pence`. The current
  code deliberately handles those OCR/currency cases. Adoption would still require
  the tokenizer, currency split and evidence number vocabulary. Keep the existing
  implementation and its new fail-closed overflow/pence handling. No claim that
  the current word grammar validates every grammatical English numeral.

- **num-format 0.4.4:** [published package](https://docs.rs/crate/num-format/0.4.4),
  [registry](https://crates.io/crates/num-format),
  [upstream](https://github.com/bcmyers/num-format). MIT/Apache-2.0, 129 stars;
  last release shown 3 December 2022, last upstream push 26 June 2024. Default
  dependencies are `arrayvec` and `itoa`; system locale support is optional and
  adds platform machinery. Its integer-formatting API does not remove Decimal
  rounding, fractional formatting or the existing sign-placement difference between
  report and verifier. Replacing these small loops has no clear net benefit here;
  no adoption trial was warranted after API inspection.

- **Alignment candidates:** [bio 4.0.1](https://docs.rs/crate/bio/4.0.1) has a
  [semiglobal aligner](https://docs.rs/bio/4.0.1/bio/alignment/pairwise/struct.Aligner.html),
  MIT licensing, 1,840 upstream stars and an upstream push on 12 October 2026.
  Its many normal dependencies and byte-based alignment API are disproportionate
  to the small character-based `semi_global` recurrence. [RapidFuzz 0.5.0](https://docs.rs/crate/rapidfuzz/0.5.0)
  has no normal dependencies, Apache-2.0 licensing, 72 upstream stars, last release
  shown 1 December 2023 and last push 29 June 2024. Its documented global distance
  is not the best-substring metric used here. Keep the existing recurrence; neither
  library supplies the verifier's numeric-gap restrictions. No replacement trial
  or performance claim was made for these rejected candidates.

The existing `csv`, `clap`, `anyhow`, `regex`, `rust_decimal`, `schemars`, PDFium and
OCR libraries already cover the substantial generic mechanics. Replacing those
would incur fixture/runtime compatibility work without an identified maintenance
benefit. No whole dependency-stack replacement was attempted.

## Implementation and validation record

Baseline `cargo test --offline` passed. A cached
Salford gold pilot passed in 0.02 seconds after a 0.35-second Cargo startup; this
measures cached scoring, not fresh OCR. The corpus and runtime models are present.
Tests and frozen outputs provide regression evidence, not independent proof of
extraction accuracy. Existing OCR gold was used to select models; it is not held-out
validation. Synthetic corruption controls were run before replay comparisons.

### Completed changes

- **Readability:** `resolve` now shows its four outcomes together and constructs the
  result once. CLI and evaluation use one `Info::usage_note` implementation.
- **Idiomatic Rust:** move owned response parts instead of cloning entire JSON
  objects; deserialize from borrowed JSON. Checked number-word arithmetic returns
  `None` on overflow instead of saturating.
- **Organisation:** keep the current modules and single crate. Add one small method
  on `Amount` to define which structures contribute a stated sum, shared by reporting
  and register scoring. No trait hierarchy, framework, or new module tree.
- **Comments/docs:** remove source-header migration narration, correct the three-call
  and cache descriptions, correct the SDK-boundary claim and the migration document's
  claim of escalation tests. Keep OCR/legal explanations. Add the architecture guide
  and correct README/schema-v2's description of variation checking. Prompt-bearing
  schema descriptions and all recorded fixtures remain unchanged.
- **Dependency:** adopt only `serde_path_to_error`, following the isolated trial.
- **Correctness:** retain decimal position/numeric boundaries in relocation, ground
  instalment shares and require percentage wording, reject unreadable pence/overflow,
  and exclude rates/formula parameters from stated totals and register sum scoring.
  Behavioural work is separate from the readability commit.

The new controls exposed false matches before the fixes (`50` in `150`, `£1.00`
in `£100`), the stated-sum total test failed at 360 versus the intended 300, and
`dwelling_numbers_do_not_ground_instalment_percentages` failed with PASS before the
percentage-wording safeguard. Valid twins retain PASS; an actually grounded 50 + 40
source retains Document Discrepancy; invented 50 + 40 is Parser Error and invented
60 + 40 is REVIEW. The original discrepancy test was corrected to change both the
source page and citation, not simply to bless a different result.

### Final validation

Executed on Rust 1.99.0 with the local corpus, models and existing OCR caches:

| Command | Result |
|---|---|
| `cargo fmt --check` | Passed |
| `cargo clippy --offline --all-targets -- -D warnings` | Passed |
| `cargo test --offline` | 108 passed; 3 corpus tests ignored by default |
| `cargo test --release --offline -- --ignored` | All 3 passed: frozen replay and two OCR gold evaluations |
| `cargo run --release --offline -- eval --replay` | All 9 candidate audits completed without execution errors |
| `target/release/s106 audit data/raw/southwark_s106_sample.pdf --llm-mode replay --quiet --out /tmp/s106-review-southwark.audit.json` | Audit written; exit 1 reflects its Parser Error/REVIEW findings |

Built baseline `e938307` separately and replayed the same cached inputs. Comparing
all 281 check IDs, descriptions, statuses and error classes across nine candidate
audits found three PASS-to-REVIEW changes, all on North Tyneside page 15:
`F8.inst2`, `F9.inst` and `F9.inst1`. Cached OCR has `8.1.2.25%` and `9.1.1.50%`,
joining the clause number directly to the percentage. The stricter matcher cannot
separate these from decimal numbers safely. This is an intentional conservative
trade-off requiring source review, not evidence of incorrect deed arithmetic.

The first comparison also exposed a refactoring regression where even a spaced
clause number absorbed the following percentage. That was fixed and covered by
`numbered_clauses_do_not_absorb_a_following_percentage`; changed amounts still fail
the negative controls. No fixture or gold expectation was updated to force a pass.
The corpus tests use cached OCR, so they do not validate fresh rendering/model
execution. The README's older Rust minimum was not independently tested.

### Deliberately deferred

Findings 5–6 and the follow-up list remain open. This PR does not claim that
escalation is lossless, all provenance is captured, every legal association is
verified, or extraction is complete. Those need explicit contracts and labelled
cases. The pure refactor changes no status definitions; the numerical fixes make
unsupported claims less likely to pass. No live model calls, new OCR models,
concurrency strategy or test-infrastructure redesign were introduced.
