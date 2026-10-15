# Synthetic test of `s106::accuracy`

This crate asks whether the agreement-based accuracy estimate in `src/accuracy.rs` (and the
`estimated` / `confirmed` levels built on it in `src/results.rs`) behaves as claimed when the truth is
known. It builds synthetic `Vote`s for three readers (an LLM `claude`, a `person`, an external
`datahub`) with chosen true accuracies, calls the unchanged `estimates()` and `error()` through a path
dependency on `s106`, and compares the output with the truth.

The estimator is a black box: nothing here re-implements or patches it, and `/src` is untouched. If
`accuracy.rs` changes, rerun to see the effect.

## Run

    cargo run --release -- --out results/

Flags: `--reps` (default 200), `--reps-s50` (replications for 50-scheme scenarios), `--only S3`
(scenario name prefix), `--threads` (default 4). Progress goes to stderr. Seeds are fixed (own
SplitMix64 generator), so reruns give identical files:

- `results/scenarios.csv`: one row per scenario with every metric.
- `results/cells.csv`: scenario x posterior-error bin: `n_cells`, `n_wrong`, `share_wrong`, `mean_error`.
- `results/summary.md`: the scenario grid, the metrics table, a second table (coverage of the percentile
  and basic bootstrap bounds with Monte Carlo standard errors, runs with a gate pass and runs with a wrong
  `confirmed` cell, realised/stated error ratios) and one-line metric definitions.

## What is simulated

Each replication has S schemes with four G1 cells each (V1, V2, V3, V8). Every reader reads every cell
and is right with its true accuracy; a wrong reading is a unique value (K = inf) or one of K-1
alternatives. Switches: shared OCR corruption (Llm and Person follow it with prob 0.9), hard cells,
cells whose truth is `not_stated`, an external reader on another lineage stage, and a partial external
vote on V1, a corruption the External reader follows too (S11), and a Person reader that copies the
Llm reading with probability q (S12). Scenarios S1-S12 are listed in `src/scenarios.rs` and in the summary. S5 has two variants: S5a
(unique wrong values, the original S5) and S5b (wrong values from a 3-value answer space, as S3-K3); S5b
is appended to the grid so the other scenarios keep their seeds.

Pipeline logic mirrored from `results::ledger`: the shown vote is the claude (proposal) reading; a
cell is looked at only when all its readers agree; `estimated` needs the shown reader's estimate to
pass the gate (fits and lower >= 0.90); `confirmed` also needs `error() <= CONFIRM` and a non-LLM reader.

## Metrics

- **true a**: population accuracy on a random cell (includes the effect of difficulty, corruption, stage).
- **bias**: mean point estimate minus true accuracy. **coverage**: share of replications with lower <= true (target >= 97.5%), with its Monte Carlo standard error sqrt(p(1-p)/n).
- **basic coverage**: the same for reader 1's basic bootstrap bound `2 * a1_hat - q97.5`, from the same draws as the library's percentile bound. `estimates()` does not expose its draws, so `src/bootstrap.rs` repeats its resampling and every replication asserts that the copy's 2.5th percentile equals the library's `lower`.
- **gate pass / false-pass / power**: share where `passes()` is true; false-pass when true a1 < 0.90, power when >= 0.95.
- **no est. / fits=false**: no triplet formed; the library's misfit (point estimate above 1 with a lower bound of 1 or more, since #76).
- **confirmed wrong**: observed share wrong among `confirmed` cells (target <= 1%). **estimated wrong**: same for `estimated`.
- **reliability bins** (`cells.csv`): observed wrong share against the posterior error `error()` reported.
- **all-agree-wrong**: cells where every reader agrees on a wrong reading, and how many reached `estimated` / `confirmed`.
- **runs**: replications in which the gate passed, and in which at least one wrong cell reached `confirmed`.
- **realised / stated**: realised wrong count over the sum of `error()`, over (a) all all-agree-wrong cells and (b) all `confirmed` cells.

## Deviations from the design note

- Missing readings: `miss` (a wrong reading of a found cell says `not_stated`) is 0 except in S6, where it is 0.5; otherwise S1 would carry chance agreement on `not_stated`.
- S6 uses K=2 only for the hallucinated value on `not_stated` cells (`k_missing`); other wrong values are unique, as the design says.
- "True accuracy" is the nominal population value, not the realised share right in a replication.

Runtime: about 2 min on 4 threads for all 26 scenarios at R = 200, including S = 50, so no scenario uses a reduced R.
