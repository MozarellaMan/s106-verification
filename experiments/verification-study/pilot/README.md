# Verification pilot on ContractNLI

The runner for the pilot in `/home/user/study/pilot-design.md`: conditions C0-C5 on 40 ContractNLI test
pairs, with prompts developed on 10 dev pairs. **No live call has been made.** Every live call is paid
and needs the project owner's go. Everything here was built and tested in `--dry-run` mode.

Data: ContractNLI, Koreeda & Manning 2021, CC BY 4.0, https://stanfordnlp.github.io/contract-nli/.
`data/contractnli/` holds `sample40.jsonl` (test), `dev10.jsonl` (dev) and the README that gives the
selection rule. `data/contractnli_sample.py` is the script that drew them. It is copied verbatim, so its
input and output paths still point at `/home/user/study/data/`; edit `SRC`/`OUT` to rerun it elsewhere.

## Run

Do dev first, then test. Prompts are tuned on dev only. Never change a prompt, a threshold or the
analysis after you have looked at test results. Test is run once per condition.

    # no key, no cost: fake client, writes under dryrun/
    python3 run.py --split dev  --dry-run
    python3 run.py --split test --dry-run
    python3 analyse.py --split dev  --dry-run
    python3 analyse.py --split test --dry-run
    (cd estimate && cargo build --release)
    estimate/target/release/estimate dryrun/results/test/votes.csv dryrun/results/test/readers.csv \
        > dryrun/results/test/estimate.csv

    # live (paid): asks you to type "yes" unless --yes
    python3 run.py --split dev  --max-items 1 --cap 3.00       # smoke test: one dev item, every condition
    python3 run.py --split dev  --conditions C0,C1,C2,C3,C4,C5 --cap 3.00   # resumes; the rest of dev
    python3 analyse.py --split dev
    python3 run.py --split test --conditions C0,C1,C2,C3,C4,C5 --cap 3.00
    python3 analyse.py --split test
    estimate/target/release/estimate results/test/votes.csv results/test/readers.csv > results/test/estimate.csv

The key comes from `ANTHROPIC_API_KEY`. If that is not set, it comes from `--key-file` (default
`~/.config/s106/anthropic_key`, surrounding whitespace trimmed). The key is never printed or saved.

Flags: `--conditions` (any subset of C0-C5; only the calls those conditions need are made),
`--cap` (USD, default 3.00), `--workers` (parallel items, default 4), `--max-items N` (only the first
N items of the split; a later run without it resumes), `--dry-run`, `--yes`.

With no key, a live run exits (status 1) before it creates any file or directory. Exit status 2: the
cap stopped the run, or a non-retryable API error (400, 401, 403, 404...) did, at its first occurrence.
Exit status 1 after a finished run: some calls failed after the SDK's retries (see below).

## Calls, resume and the cap

Each condition is built from a small set of primitive calls. Each call is made at most once:

| call | model | saved under `runs/<split>/` | used by |
|---|---|---|---|
| reader, no evidence (`prompts/reader_c0.txt`) | Haiku 5.5 | `C0/claude-haiku-5-5/` | C0 |
| reader, verbatim quotes (`prompts/reader_quotes.txt`) | Haiku 5.5 | `C1/claude-haiku-5-5/` | C1, C2, C3-C5 |
| same reader prompt | Sonnet 5.5, Opus 5.5 | `C3/claude-sonnet-5-5/`, `C3/claude-opus-5-5/` | C3-C5 |
| entailment check of the Haiku quote reading (`prompts/entailment.txt`; hypothesis, label and quotes only) | Sonnet 5.5 | `C2/claude-sonnet-5-5/` | C2 |
| adjudication (`prompts/adjudicator.txt`; document, all three readings with their quote-check results) | Opus 5.5 | `C4/claude-opus-5-5/` for items C4 flags, `C5/claude-opus-5-5/` for the rest | C4 (flagged items), C5 (all) |

- Each file `<item_id>.json` holds the request (model, max_tokens, output_config with the JSON schema,
  and the prompt with the document replaced by its doc id and hash), the raw response, the token usage,
  the latency in ms, a UTC timestamp, the cost and a `dry_run` flag. If the file exists, the call is
  never made again. A stopped run therefore resumes where it stopped. A truncated or refused answer is
  saved like any other. Delete its file only if you mean to pay for a retry. A saved record is reused
  only if its stored request equals the request the current code would send; if a prompt, schema,
  effort or max_tokens has changed, the run exits rather than mix versions (move `runs/<split>/` aside).
- Errors: the SDK retries 408/409/429/5xx/529 and connection errors 8 times with backoff. A call that
  still fails is not saved as a record; it goes to `runs/<split>/errors.csv`, the item's later calls
  (entailment, adjudication) are not made, the run carries on with other items, ends with status 1,
  and the same command retries it. analyse.py prints every incomplete item and `metrics.csv` has
  `n_incomplete`. A non-retryable error stops the whole run at once.
- `runs/<split>/ledger.csv` gets one row per call made, with tokens, `stop_reason` and cost from the
  price table below, computed from the response's `usage.input_tokens` and `usage.output_tokens`
  (output includes thinking). No `cache_control` is sent, so the cache fields should be 0; if any
  appear they are counted at 1.25x the input price, never ignored. The ledger row is written before
  the call's reservation is released, so the cap never misses a paid call.
- Cap: before each call the runner adds up every split's ledger (dev and test together, so one $3.00
  covers the whole pilot). It also adds the worst case of the calls in flight and the worst case of
  the next call (input at 2 characters per token, output at `max_tokens`). If that sum would pass
  `--cap`, it starts no more calls, prints the reason and exits with status 2. Because the check uses
  the worst case, a run can stop a little below the cap.
- C4 flags an item when the three readers' labels are not all the same, or any reader's quote check
  fails, or any reader gave no usable answer. The entailment check is not used for this flag.
- Dry-run output goes to `dryrun/runs` and `dryrun/results`, never to `runs/`. A live run refuses any
  record marked `dry_run` (and a dry run any live record); analyse.py refuses records whose flag does
  not match `--dry-run`, and `metrics.csv` and `readers.csv` carry a `dry_run` column.
- Ctrl-C stops the run before its next call; calls in flight finish and are recorded.

API usage (Python SDK `anthropic` 1.14, `client.messages.create`): structured outputs through
`output_config={"format": {"type": "json_schema", "schema": ...}}`, with no prefill and no
temperature. Thinking is left at each model's default (adaptive), with no `budget_tokens`. **Readers
and the entailment check run at `output_config.effort = "low"` to keep cost down.** The adjudicator
runs at the model default (medium on Opus 5.5). `max_tokens` (thinking included): readers 3000,
entailment 1500, adjudicator 8000. A refusal (`stop_reason: "refusal"`, no fallback model, so each
reader stays one model) or a truncation is saved and counts as no usable answer. The JSON schemas leave out numeric bounds. Confidence
is clamped to 0-100 when it is read back.

## Cost table (list prices 14 Oct 2026, per million tokens)

| model | input | output |
|---|---|---|
| claude-haiku-5-5 | $0.10 | $0.50 |
| claude-sonnet-5-5 | $2.00 | $10.00 |
| claude-opus-5-5 | $4.00 | $20.00 |

The design estimates about $3.00 for everything. The dry run's fake token counts (input = prompt
characters / 4, output 150-450) give $0.50 for dev and $2.02 for test. Real output counts include
thinking tokens, and Haiku 5.5's tokenizer counts about 30% more tokens. Both could push real cost
above that estimate, mostly through Opus. The cap is the safeguard. Read the dev ledger before you
start test.

## Outputs (`results/<split>/`)

**Every figure in the report comes from `results/*.csv`** (or `dryrun/results/*.csv` for the dry run).
Nothing is typed in by hand.

- `metrics.csv`: one row per condition. Every rate column `x` comes with `x_k`, `x_n` and a Wilson 95%
  interval `x_lo`, `x_hi`.
- `items.csv`: one row per item and condition, with the final label and every reading behind it. That
  is each reader's label, confidence, quote-check status (`exact`, `near(d)`, `missing` per quote) and
  reason, the entailment verdict, and the adjudication. Failure cases for the report come from here.
- `votes.csv`: the three quote readers' labels in the estimator's format
  (`scheme,variable,source,reader,kind,family,status,value` = item, V1, deed, model, llm, model, found, label).
- `readers.csv`: each reader's measured accuracy on the split, to compare with the estimate.
- `estimate.csv` (from the Rust crate): `reader` rows with the estimator's point estimate, lower bound
  (2.5th percentile over resampled items), `fits` and gate pass, beside the measured accuracy. `item`
  rows give the majority label and its posterior chance of error from `accuracy::error()`.

### What the final answer is, per condition

- C0, C1: the Haiku reading. C1 is flagged when its quote check fails.
- C2: the C1 reading. It is flagged when the quote check fails or Sonnet says the quotes do not support
  the label. NotMentioned readings have no quotes and get no check.
- C3: the majority of the three readers. Its confidence is the mean confidence of the majority readers
  times their share of the three (2/3 or 1; a reader with no usable answer still counts in the three). A three-way split gives NotMentioned with confidence 0. C3 is flagged
  exactly when C4 would adjudicate.
- C4: the adjudicator's answer on flagged items, otherwise the C3 answer. C5: the adjudicator's answer
  on every item. Both are flagged when the adjudicator's own quote check fails.

### Metric definitions

- `accuracy`: final label = gold label.
- `false_conf_high`: confidence >= 90 and wrong. `false_conf_high_unflagged` counts only those the chain
  did not flag, which are the confident errors it would have let through.
- `unanimous_wrong` (C3-C5): the three readers agree, the final label is theirs, and it is wrong.
  `all_readers_wrong`: every reader is wrong. `all_wrong_same_label`: every reader is wrong with the
  same label (a shared error).
- `flag_rate`: share of items the condition flags (see above).
- Abstention: `abstain_when_answer` is NotMentioned predicted when gold is Entailment or Contradiction
  (n = gold non-NM). `answer_when_not_mentioned` is Entailment or Contradiction predicted when gold is
  NotMentioned. An item with no usable final answer counts as neither (but as wrong for accuracy).
- `ece_5bin`: expected calibration error. Final confidence goes into five equal bins on 0-100. The
  figure is the sum over bins of |accuracy - mean confidence|, weighted by bin size.
- `quote_fail_rate`: over every Entailment or Contradiction reading the condition uses (Haiku for
  C1/C2; three readers for C3; three readers plus the adjudicator for C4/C5), the share whose check
  fails. A check fails on any quote that is not `exact` after whitespace, quote-mark and dash
  normalisation (also ligatures such as `ﬁ` -> `fi` and `…` -> `...`, which occur in these
  documents), or on no quotes at all. `near` means Levenshtein distance <= 3 to some window of the
  document. It is recorded but still counts as a failure.
- `entail_unsupported_on_passed` (C2): of the readings that passed the quote check and got a usable
  entailment answer, the share Sonnet judged unsupported (a refused check flags the item but is left
  out here). This is design negative finding 2.
- `gold_evidence_precision` / `gold_evidence_recall`: character-level overlap of the located quotes
  with the gold evidence spans. Precision is quoted characters inside gold / quoted characters; recall
  is gold characters quoted / gold characters. Both are means over readings with a label on items whose
  gold is not NotMentioned. `gold_evidence_hit` is the share of those readings with any overlap.
- `mean_latency_ms_per_item`: mean over items of the summed latency of the condition's calls, as if run
  in sequence. `input_tokens`, `output_tokens`, `cost_usd`: totals over the condition's calls. Shared
  calls count in every condition that uses them, so condition costs do not add up to the ledger.

## Files

    prompts/        reader_c0.txt, reader_quotes.txt, entailment.txt, adjudicator.txt
    run.py          runner, fake client, cost ledger and cap
    quote_check.py  deterministic quote check and gold-span overlap (python3 quote_check.py runs a self-test)
    analyse.py      runs/ -> results/<split>/*.csv
    estimate/       Rust crate calling s106::accuracy::{estimates, error} unchanged
    data/           ContractNLI samples, their README, and the sampling script
    dryrun/         dry-run outputs (fake data; not results)
