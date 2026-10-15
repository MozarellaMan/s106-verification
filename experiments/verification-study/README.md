# Verification study: can the pipeline verify its own answers?

A self-contained study of the agreement-based accuracy estimator in `src/accuracy.rs` and of the
verification stages the project's rule relies on. It does not change the production code or rule.

- `REPORT.md`: the technical report (question, audit, synthetic falsification, pilot results, literature,
  what is and is not established, limitations, four-month agenda, how to reproduce).
- `synthetic/`: Rust crate that drives the unchanged `estimates()` and `error()` through twelve
  scenarios; `synthetic/results/summary.md` holds the figures the report cites. Design in `synthetic-design.md`.
- `pilot/`: the ContractNLI pilot (conditions C0-C5 on 40 test pairs, plus OCR-noise and seeded-edit
  variants), with every saved call record under `pilot/runs/`, results under `pilot/results/`, and the
  subagent bridge. `pilot/README.md` explains how to run or reanalyse it. Design in `pilot-design.md`.
- `lit-review.md`: 45 references with a verification mark on each.
- `benchmark-survey.md`: how the evaluation dataset was chosen.
- `s106-case-survey.md` and `survey-scripts/`: read-only survey of the project's real data as it stood on
  14 Oct 2026 (the scripts read a checkout of the private data repository; their outputs are not committed).

Public data only: ContractNLI (CC BY 4.0). No S106 deed text or derived data is in this folder.
