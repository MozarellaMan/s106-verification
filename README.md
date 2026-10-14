# s106-verification

A verification harness for UK Section 106 planning obligations, written in Rust.

Claude reads an executed deed (agreement, unilateral undertaking, deed of variation or
supplemental deed) into a typed schema. A deterministic gate then checks every number against
the others **and** against the deed's own text: each value must appear in the passage the model
cites, and that passage must be found on the cited page. Nothing in the gate is a model judgement.

**Status.** No person has yet labelled the obligations in any deed, so there is no measured
accuracy for extraction or for the gate. PASS means a value matches the model's own citation on
the cited page; it is not a check against independent evidence. The OCR gold pages were
transcribed by an AI agent and the pinned expected outcomes were blessed from the pipeline's own
output, so the test tiers show repeatability, not correctness ([docs/testing.md](docs/testing.md)).
Nine of the ten deeds the prompts were tuned on are from outside London.

Architecture and contributor reading path: [docs/architecture.md](docs/architecture.md).

OCR speed, the default model set (PP-OCRv6 tiny) and choosing another with `S106_OCR_MODELS`: [docs/ocr-speed.md](docs/ocr-speed.md).

This repository holds code and invented test deeds only; it describes no real planning
application. The data, and the research notes about specific deeds (the roadmap, the deeds behind
schema v2, how the Rust harness was accepted against the former Python one), are in the private
[s106-data](https://github.com/MozarellaMan/s106-data) repository's `docs/`.

Obligations against delivery: [docs/reconciliation.md](docs/reconciliation.md).

What the outputs must not be read as saying: [docs/claims.md](docs/claims.md).

```
src/
  document.rs   PDF pages -> text with boxes: PDFium text layer, or PP-OCRv6 tiny OCR; cached by content hash
  wire.rs       structured-output types the model fills in, and their conversion to schema v2
  llm.rs        the Anthropic API (rust-claude-sdk, pinned): strict JSON schemas, PDF input, retries
  extract.rs    prompts, recorded fixtures and offline replay, escalation to a stronger model
  schema.rs     schema v2: instrument, housing, contributions (amount / conditionality / payment), variations
  verify.rs     the gate: arithmetic + grounding -> PASS / REVIEW / FAIL / SKIP
  evidence.rs   re-locating a cited snippet in OCR text (dropped spaces, two-column wraps, OCR digits)
  triggers.rs   fixed vocabulary that labels payment triggers from the cited phrase
  text.rs       number and words-to-amount reading
  report.rs     terminal report and JSON
  eval.rs       evaluation against the national S106 register (planning.data.gov.uk)
  pld.rs        read-only Planning London Datahub client; saves raw snapshots under <data>/snapshots/pld/
  reconcile.rs  obligation events vs PLD delivery, one row per measure, with a status per row
  ocrbench.rs   OCR scored against gold transcriptions (`gold/*.json` in the data repository)
  labels.rs     scored against human labels (`labels/` in the data repository)
  results.rs    a batch's scheme ledger, decision rules and claims, each with its check level
  explore.rs    Datahub-only data for the public results explorer (`explorer/`)
```

## Setup

Rust 1.85+ (edition 2024). `cargo build` and `cargo test` need nothing else: no data, network, PDFs,
PDFium, ONNX Runtime or models. To read PDFs, fetch PDFium, ONNX Runtime and the OCR models;
each file is checked against its pinned SHA-256 in `scripts/runtime-artifacts.tsv`:

```bash
scripts/fetch-models.sh
```

```bash
cargo build --release
```

Live extraction needs an Anthropic API key in `~/.config/s106/anthropic_key` (or
`ANTHROPIC_API_KEY`). Replaying recorded fixtures needs nothing but the data checkout.

### The data

The deeds' recorded outputs (page text, model responses, expected outcomes, gold, register rows)
live in a separate private repository, `MozarellaMan/s106-data`; access must be granted. Clone it
into `data/` (ignored by git), or point the `S106_DATA` environment variable or the global
`--data <dir>` flag at a checkout elsewhere. Then fetch the PDFs, which come from public planning
registers and are not committed anywhere:

```bash
git clone git@github.com:MozarellaMan/s106-data.git data
scripts/fetch-corpus.sh
```

`scripts/fetch-corpus.sh` reads `corpus/manifest.tsv` from the data checkout, downloads every
deed into `data/raw/<id>.pdf`, and refuses a file with a different SHA-256, as do the recorded
responses (`data/extractions/<id>/`). The evaluation's candidate list and the register rows for
those deeds are in the data repository's `eval/`.

### Starting without the data repository

Any folder with the same layout works as a data checkout (`src/data.rs` names every path). To
start one, find the executed deed for a planning application on the council's register, then:

1. Add it to `corpus/manifest.tsv` (id, `raw/<id>.pdf`, SHA-256, URL, session page or `-`) and
   run `scripts/fetch-corpus.sh <id>`.
2. Write its page text with `s106 pages data/raw/<id>.pdf --write data/pages` (needs
   `scripts/fetch-models.sh`).
3. Record one extraction with `s106 audit data/raw/<id>.pdf --llm-mode record` (costs money; see
   below). From then on, `s106 audit data/pages/<id>.pages.json` replays it offline.
4. To compare with the national register, list it in `eval/candidates.tsv`, download the register
   files (see the `eval/` README in s106-data) and run `s106 eval --replay --register <dir>`.

These are the applications the current corpus was built from, a mix of agreements, a unilateral
undertaking and deeds of variation:

| Council | Planning reference | Instrument |
|---|---|---|
| Southwark | 19/AP/1773 | S106 agreement |
| Salford | 19/73971/FUL | S106 agreement (counterpart) |
| Salford | 20/75239/FUL | S106 agreement |
| Dover | DOV/18/00681/FULL | deed of variation (redacted) |
| Dover | DOV/19/00403/OUTLINE | deed of variation (redacted) |
| Sunderland | 21/01544/FU4 | S106 (redacted) |
| North Tyneside | 19/01280/FUL | S106 agreement |
| Bolton | 01373/17 | supplemental S106 |
| Tonbridge & Malling | 20/02239/FL | unilateral undertaking |
| Tonbridge & Malling | 24/00842/PA | deed of variation |

## Run

```bash
s106 audit data/raw/<id>.pdf                                       # replay if no credentials, else live
s106 audit data/raw/<id>.pdf --llm-mode record                     # call the API and save the fixture
s106 audit <pdf> --escalate                                        # re-read Parser-Error fields with Opus 5.5
s106 audit data/pages/<id>.pages.json                             # replay from committed page text, no PDF
s106 audit data/raw/a.pdf data/raw/b.pdf --llm-mode record          # several deeds: OCR one while others wait on the model
s106 eval --replay                                                 # all candidate deeds vs the register, offline
s106 eval --replay --pages data/pages                              # the same from committed page text, no PDFs
s106 pages <pdf>... --check data/pages                             # read PDFs afresh and compare with the snapshots
s106 eval --runs 3                                                 # live, three runs per deed (run-to-run agreement)
s106 bench-ocr data/gold/<gold>.json data/raw/<id>.pdf --models ppocrv6-tiny,ppocrv5-en-mobile
s106 pld get applications <borough>-<reference>                    # save a Datahub record under <data>/snapshots/pld/
s106 reconcile <deed>... --development <borough>-<reference> --pld snapshots/pld/applications/<id>/<time>.json
s106 labels                                                        # score everything against the human labels
s106 results --sheet runs/<batch>/labels.csv --out results/<batch>  # a batch's ledger, rules and claims
s106 explore                                                       # bake the results explorer's data (explorer/README.md)
```

(`cargo run --release --` instead of `s106` without installing.)

`audit` prints a report and writes JSON to `data/processed/<stem>.audit.json`. Exit codes are `0`
clean, `1` Parser Errors or REVIEW flags, `2` Document Discrepancies (the worst over all deeds), and
`3` if any deed could not be audited.

**What a run costs.** Extraction is three structured-output calls over the whole PDF, made at the
same time. A single schema for everything compiles to a grammar the API rejects, and the PDF can't be cached across
calls whose schemas differ. With Haiku 5.5 a 30-page deed costs a few cents; a 109-page scan
costs about $0.30. Escalation to Opus 5.5 costs more and only runs with
`--escalate`.

## Human labels

A person's reading of a source is the only ground truth here; everything else is the pipeline
agreeing with itself. Labels are a separate layer that nothing else waits for. A labeller saves a
filled-in sheet (the source-audit codebook's columns, signed with `labeller` and `labelled_on`)
as a new file under `labels/` in the data repository, and `s106 labels` reads every file there and
writes `reports/labels/labels.{md,json}`:

- the pipeline's readings (recorded deed extractions, Datahub snapshots) scored field by field
  against the labels: precision and recall with their n and a 95% interval;
- labellers against each other where two labelled the same cell (agreement and Cohen's kappa);
- sources against each other (deed, committee report, GLA report, Datahub, EPC).

With no labels it says so and succeeds. Submissions are append-only: a correction is a new file,
and the latest `labelled_on` per labeller and cell wins. A row signed `machine:` is refused.

A label's optional `method` column says how the person arrived at it: `read` (from the source,
the default) or `blind` are independent readings; `audited` or `corrected` are decisions on an
agent's proposal shown with its evidence. Proposals live under `proposals/` in the data
repository, signed `machine:<producer>`; `s106 labels` scores them like any other machine reading
and keeps the accuracy figures for independent and anchored labels apart, since how often a person
changed a proposal is not the same measurement as how often an independent reading agreed with it.

## Results

`s106 results` turns a batch into something a researcher can read and load. From the batch's blank
sheet (which cells exist), the labels and the proposals, it writes a folder under `results/` in the
data repository: a ledger with one row per scheme, variable and source in lineage order (offer,
decision, obligation, variation, completion), the batch's pre-registered decision rules worked out
scheme by scheme, retrieval per source, and a note that lists the hand-written claims in the
folder's `claims.csv`. Every cell, rule outcome and claim carries a check level: `independent` (a
person read the source), `anchored` (a person confirmed or corrected a proposal), `proposed` (an
agent only, not a result), `unsettled` (labellers disagree) or `unread`. A rule or claim stands at
the weakest level of the cells it rests on, so the note moves from proposed to checked on its own
as labels arrive. Like `s106 labels`, it never waits for them.

## Outcomes

An input is *grounded* when its value appears in its cited snippet (as digits, OCR `£`→`f` and
`5o`→`50` repaired, or as number words) and that snippet re-locates on the cited page of the page
text, ignoring whitespace. Mix-table cells have no snippets of their own, so a row is grounded
when one line on the table's page states all of its counts.

| outcome | meaning |
|---|---|
| PASS | the assertion holds and every input is grounded |
| REVIEW | the assertion holds, but some input isn't grounded: correctness unproven |
| FAIL, Parser Error | an input is missing or ungrounded, and the assertion fails |
| FAIL, Document Discrepancy | the assertion fails although every input is grounded: what the deed *appears* to say through OCR, to be confirmed against the original |
| SKIP | the deed doesn't state what the check needs; whole-scheme checks on a variation; a floorspace or unstated % basis (never recomputed by units) |

## Checks

| ID | Assertion |
|----|-----------|
| H1 | market units + affordable units == total residential units (SKIP for variations) |
| H2 | Σ tenure units == affordable units |
| H3 | Σ tenure habitable rooms == affordable habitable rooms |
| H4 | claimed affordable % == calculated % on the deed's stated basis (±0.05pp) |
| H5 | approved mix table: rows sum to each tenure's sub-total, sub-totals to the total |
| H6 | mix table agrees with the definitions |
| V1 | a variation or supplemental deed identifies its parent, and the citation is grounded |
| V2.n | an identifying token from each variation's target is grounded; operation and new wording remain unverified |
| F*.amt | a fixed or capped sum was extracted and is stated in its citation (SKIP for formula/rate/no figure) |
| F*.words | the figure equals its words form ("£10,058.60 (ten thousand and fifty eight pounds and sixty pence)") |
| F*.cond | a conditional sum's condition is the deed's own wording, found on its page |
| F*.trig | the payment covenant and its trigger phrase are both in the deed, the phrase lies inside the covenant, and the covenant imposes a payment |
| F*.tlabel | the trigger label follows from the cited phrase (fixed vocabulary in `triggers.rs`) |
| F*.inst… | instalments: grounded shares sum to 100%; the lead-in imposes payment; each instalment's phrase is in the deed and decides its label |

## Reproducibility

- **Page text** is cached under `data/interim/ocr/<sha256[:16]>-<ocr version>/` and kept in the data
  repository, per deed, as a snapshot in `data/pages/<stem>.pages.json`. Both are keyed by the PDF's
  SHA-256 and a hash of everything that produces the text: line-logic version, OCR settings and
  the pinned SHA-256 of PDFium, ONNX Runtime and each model file. Changing any of them gives a new
  key; artifact files that differ from their pins are refused; the cache manifest is validated on
  read. Each image-only page is OCR'd on its own, so its text does not depend on other pages.
- **Model responses** are recorded in `data/extractions/<stem>/<model>.extract.v2.json`, tied to the
  PDF's SHA-256 and to a hash of the prompts and schemas. A different deed or a changed prompt
  refuses to replay. v1 fixtures from the former Python harness (`data/extractions-v1/`) still replay.
- **Runtime artifacts** (PDFium, ONNX Runtime, OCR models) are pinned by SHA-256 in
  `scripts/runtime-artifacts.tsv`.
- **Determinism:** trigger labels come from the cited phrase, not the model. Plural group terms
  are dropped. Truncated structured output is retried.

## Tests

Five tiers, from always-on to on-request. Details, timings and how to update the snapshots:
[docs/testing.md](docs/testing.md).

```bash
cargo test                      # 1 unit tests and invented deeds (tests/synthetic/): no data, PDFs, models or network
cargo test-data                 # 2 replay of the real corpus from the data checkout (S106_BLESS=1 to bless)
cargo test-pdf                  # 3 difficult pages read afresh from the PDFs, OCR gold thresholds
cargo test-corpus               # 4 every deed read through PDFium/OCR, compared with its snapshot
s106 eval --replay              # 4 register evaluation on the PDFs
s106 eval --runs 3              # 5 live model calls (costs money; manual only)
```

Tier 2 (`cargo test-data`) needs the data checkout and fails with "data missing" without it.
Tiers 3 and 4 also need `scripts/fetch-models.sh` and `scripts/fetch-corpus.sh`. They fail with
"corpus missing" when deeds are absent; they never skip silently. CI runs `cargo test` on every push, plus a second job that checks out
the data repository (secret `S106_DATA_DEPLOY_KEY`) and runs `cargo test-data`; tiers 3-4 run weekly on
a cold cache (also with the data checkout), and 5 only when started by hand (`.github/workflows/`).
Only tiers 1-2 gate pull requests; the weekly job skips the PDF tiers with a warning when the
council sites it downloads from are unreachable.
