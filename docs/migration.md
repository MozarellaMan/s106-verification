# Python to Rust migration: acceptance and results

The Python harness (`s106-audit/`, M1) was replaced by the Rust crate in this repository. The
Python code is retired and remains only in git history; its last outputs are frozen as fixtures.
Figures below were re-run on the day of writing unless marked "recorded".

## Acceptance criteria and results

| Criterion | Command | Result |
|---|---|---|
| Offline suite passes (no deeds, no credentials) | `cargo test` | 95 passed, 0 failed, 3 ignored (document 3, eval 15, evidence 13, extract 8, text 10, triggers 7, verify 39) |
| Rust reproduces the frozen Python M1 outputs | `cargo test --release -- --ignored` (`golden_python_m1`; needs `data/raw` deeds and `models/`) | 195 Python checks compared: 193 identical, 2 documented improvements (`fixtures/golden-python-m1/expected-differences.json`: Southwark `F7.amt`, `F7.words`, REVIEW to PASS because PP-OCRv5 reads "fifty pounds)" where PP-OCRv4 read "ftypounds") |
| OCR meets the gold thresholds | same command (`ocr_gold`: `southwark_scan`, `salford_text_layer`) | 2 passed |
| Clean lints | `cargo clippy --all-targets -- -W clippy::all` | no warnings |

## OCR against hand-verified gold

Gold: `gold/southwark_19_AP_1773.json`, 6 pages. Reproduce with
`cargo run --release -- bench-ocr gold/southwark_19_AP_1773.json data/raw/southwark_s106_sample.pdf --models ppocrv5-en-mobile`.
Python numbers are as recorded before retirement.

| Measure | Python (RapidOCR PP-OCRv4) | Rust (PP-OCRv5 English) |
|---|---|---|
| Squashed CER | 0.009 | 0.000 |
| Exact gold lines | not recorded | 20/20 |
| Facts on the gold page | 7/9 after the `£` repair | 8/9 raw (it reads `£`) |
| Printed page numbers | 3/6 | 6/6 |
| Two-column definitions | merged | kept on their own lines (3/3) |

## Live v2 extraction (recorded)

`fixtures/llm/*`: all ten deeds, Haiku 5.5 (`claude-haiku-5-5`), three calls per deed (instrument,
housing, contributions), whole deed attached, one attempt each, no cache hits. Cost is
`llm::cost_usd` applied to each fixture's summed usage. It applies Haiku's higher tier when the
summed prompt exceeds 100k tokens, whereas the tier is per call, so the larger deeds are somewhat
overstated.

| Deed | Pages | Input tokens | Output tokens | Cost (USD) |
|---|---:|---:|---:|---:|
| tmbc-2400842 | 9 | 50,580 | 3,794 | 0.007 |
| dover-1900403 | 8 | 55,167 | 5,029 | 0.008 |
| dover-1800681 | 9 | 66,360 | 8,412 | 0.011 |
| tmbc-2002239 | 26 | 129,726 | 3,754 | 0.074 |
| bolton-0137317 | 28 | 139,170 | 9,141 | 0.092 |
| sunderland-2101544 | 40 | 195,834 | 8,118 | 0.118 |
| salford-75239 | 32 | 213,168 | 7,403 | 0.125 |
| ntyneside-1901280 | 45 | 219,444 | 8,858 | 0.132 |
| salford-73971 | 35 | 260,820 | 12,542 | 0.162 |
| southwark_s106_sample | 109 | 521,652 | 13,998 | 0.296 |
| **Total** | | 1,851,921 | 81,049 | **1.03** (mean 0.10) |

## Decisions

- **SDK:** `rust-claude-sdk` 0.2.0 over `threatflux-anthropic-sdk` (2.6k vs 26.7k lines, 5 vs 19
  dependencies). Pinned exactly (`=0.2.0`) and wrapped in `src/llm.rs`.
- **Whole-deed attachment** replaces the Southwark-tuned page selection. This also fixes the
  excerpt-mode page-citation offset.
- **Three calls per deed:** one schema exceeds the compiled-grammar limit, and the PDF cannot be
  cached across different schemas (measured: 3 cache writes, 0 reads).

## Decision: rules backend retired (confirmed by the maintainer, 13 Oct 2026)

The Southwark-specific rules (regex) backend was **not** ported and is retired. Its last outputs
are kept as history in `fixtures/golden-python-m1/*.rules.audit.json`; nothing reads them. If a
deterministic non-LLM baseline is wanted, it has to be written afresh.

## Open

- Opus escalation has unit-level logic and tests but has not been exercised live.
- The Message Batches API (asynchronous, discounted) is the cost lever for scale and is not used yet.
