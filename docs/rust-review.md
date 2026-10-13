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
| `extract::to_agreement` JSON conversion | `serde_path_to_error` 0.1.20 | Exact nested failure location without handwritten traversal | Investigate with a valid/corrupt fixture trial |
| `text::words_to_int` / `words_to_amount` | `text2num` 2.8.0 | Maintained grammar for number words | Keep existing OCR and pounds/pence handling; investigate only a narrowly defined grammar replacement |
| `report::gbp`, `verify::gbp`, `eval::thousands` | `num-format` 0.4.4 | Integer grouping and locale support | Keep existing; decimals/sign placement remain custom and no locale requirement exists |
| `ocrbench::semi_global` | `bio` 4.0.1 / `rapidfuzz` 0.5.0 | Established alignment/string-distance implementations | Keep existing small character-based recurrence; global edit distance is not this metric |

The evidence matcher's number-preservation rules, OCR digit repair, margin filtering,
trigger vocabulary, payment/assessment distinction, and basis/variation gating are
project-specific. Generic fuzzy similarity must not replace them.

## Implementation and validation record

Pending focused implementation. Baseline `cargo test --offline` passed. A cached
Salford gold pilot passed in 0.02 seconds after a 0.35-second Cargo startup; this
measures cached scoring, not fresh OCR. The corpus and runtime models are present.
Tests and frozen outputs provide regression evidence, not independent proof of
extraction accuracy. Existing OCR gold was used to select models; it is not held-out
validation. Synthetic corruption controls will be run before replay comparisons.
