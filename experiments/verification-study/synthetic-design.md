# Synthetic falsification of `accuracy.rs` — design

Target: `s106::accuracy::{votes, agree, estimates, error, Estimate, Vote, Kind, GATE, CONFIRM, MIN_SHARED}` in
`src/accuracy.rs`, called as a library (path dependency), unchanged.

## What the estimator assumes (to be tested, not fixed)
A1. Two readers agree on a cell only when both are right (so P(agree) = a_i a_j). Ignores chance agreement on
    the same wrong value, which happens when the answer space is small (status found/not_stated is 2-valued;
    V4 route has few values; V2 share mostly one of ~5 values).
A2. Errors are independent across the three families. Violated when all readers read one OCR text (a digit
    error in the OCR misleads every LLM the same way), when item difficulty varies (a hard page makes every
    reader wrong), and when the external reader reports a different lineage stage.
A3. One accuracy per reader per group (no item difficulty).
A4. The scheme bootstrap's 2.5th percentile is a valid lower 95% bound at the gate (lower >= 0.90) with as few
    as 5 schemes.
A5. `fits` = lower <= 1.0 detects model misfit.
A6. `error()` uses the lower bound as the accuracy, a uniform prior over distinct observed readings plus
    "none of them", and takes two agreeing wrong readers as giving the same wrong value. Claimed to be conservative.
A7. The triplet is chosen as the one whose least-shared pair shares the most cells (selection on the data).

## Generator (deterministic, seeded)
Parameters per scenario:
- `schemes` S (5, 8, 20, 50); cells per scheme: 4 variables (V1, V2, V3, V8 -> group G1).
- readers: three, families `claude`, `person`, `datahub` (any three distinct family strings; use Kind::Llm,
  Kind::Person, Kind::External so `error()`'s not_llm branch is exercised), true accuracies a = (a1, a2, a3).
- `k`: size of the answer space for a wrong reading. K = inf (wrong values never coincide: each wrong reader
  emits a unique string), K = 2, 3, 5, 10 (wrong value drawn uniformly from K-1 alternatives).
- `rho_shared`: probability a cell carries a "shared corruption" (OCR-type): when present, every reader of
  Kind::Llm AND Kind::Person that reads it reports the same wrong value with probability `p_follow` (0.9);
  the External reader is not affected. Baseline 0.
- `difficulty`: 0 (none) or a beta mixture: a share `h` of cells is hard; on a hard cell each reader's
  accuracy is a_i - delta (floor 0.05). Baseline none.
- `missing`: share `m` of cells whose truth is `not_stated`. On such a cell a reader says `not_stated`
  with prob a_i, else `found` with a hallucinated value. On a `found` cell a reader says `not_stated`
  with prob `miss` (default half of its error budget) else found with the value rules above. Statuses are
  compared like in `agree` (status equality then fields).
- `stage`: probability that the External reader reports a different-stage value (systematically differs
  from the truth, same for the whole scheme: e.g. varied figure). Baseline 0.
- `partial_external`: whether the external reader is a partial vote (only `total`, `affordable` fields) as
  the Datahub is. Model V1 cells as fields {total, affordable} where the external vote is partial and LLM
  votes carry {total, social_rent, intermediate, market}-style fields with totals() consistency. Keep this
  as a switch; baseline false (all full votes with one field `v`).

Encode a reading as Vote { cell: (scheme, variable, "deed"), reader, kind, family, status, fields: {"v": value},
partial: false, value }. Correct value string "T"; wrong value "W<j>" with j drawn per the K rule
(K = inf -> unique per vote).

## Replications and metrics
For each scenario run R = 200 seeded replications (fewer for S=50 if slow). Per replication: build votes,
call `estimates(&votes, &BTreeSet::new())`, take the estimate for reader 1 (and others), then for every cell
with >= 2 votes call `error(shown, &vs, &estimates_map)` where `shown` is the vote that the real
pipeline would show (the first agreeing majority, else the first LLM vote; mirror results.rs lines ~200-262:
read them) and record whether `shown` is actually right.

Report per scenario (CSV + a plain text table):
- mean and sd of point estimate a1_hat vs true a1; bias.
- coverage: share of replications where lower <= true a1 (target >= 0.975).
- gate false-pass rate: share of replications with passes() true when true a1 < 0.90; gate power when true a1 >= 0.95.
- share of replications with no estimate (triplet not formed) and share with fits = false.
- confirm calibration: among cells with error <= CONFIRM (0.01) in passing groups, the observed share wrong
  (target <= 0.01) and the count; also binned reliability (error bins 0-1%, 1-5%, 5-20%, 20-50%, 50%+) with
  observed wrong share per bin.
- distribution of the posterior error for cells where all three readers agree on a wrong value.

## Scenario grid (name each)
S1 baseline-ideal: S=20, a=(0.9,0.9,0.9), K=inf, no correlation. Expect unbiased, coverage ok.
S2 small-n: S=5 and S=8, same as S1. Tests A4 at the real sample size.
S3 chance-agreement: S=20, a=(0.9,0.8,0.8), K in {2,3,5}. Tests A1.
S4 shared-ocr: S=20, a=(0.9,0.9,0.9), rho_shared in {0.05,0.1,0.2}, K=inf. Tests A2.
S5 difficulty: S=20, h=0.3, delta=0.4, base a=(0.95,0.95,0.95) so marginal accuracy ~0.83. Tests A2/A3.
S6 missing: S=20, m in {0.2,0.4}, a=(0.9,0.9,0.9), K=2 for the hallucinated value. Tests how not_stated
   agreement inflates.
S7 stage-mismatch: S=20, stage in {0.1,0.3}, a=(0.9,0.9,0.9). Tests the external-reader dependence.
S8 asymmetric: S=20, a=(0.95,0.6,0.6), K=inf and K=3: can one good reader be estimated when its two
   partners are poor?
S9 near-gate: S=20 and 50, a=(0.88,0.9,0.9) and (0.92,0.9,0.9): gate error rates around the threshold.
S10 all-wrong-agree: combine K=3 with rho_shared=0.1 and a=(0.85,0.85,0.85): how many cells reach
   `confirmed` while wrong?

## Deliverable
Rust crate at `synthetic/` (Cargo.toml with
`s106 = { path = "../../.." }`; add it to no workspace; build with `cargo run --release -- --out results/`).
Keep the code simple and readable (~400 lines), one file per concern (generator, metrics, scenarios, main).
Write results to experiments/verification-study/synthetic/results/*.csv and summary.md (generated by the
program, with the scenario grid and the metrics table). Fixed seeds; the program is the only writer of
figures.
