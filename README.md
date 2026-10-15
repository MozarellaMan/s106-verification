# s106-verification

Research tools for UK Section 106 planning obligations, written in Rust.

The pipeline is automated. `s106 pages` turns an executed deed (agreement, unilateral
undertaking, deed of variation or supplemental deed) into page text, from the PDF's text layer or
by OCR. LLM readers, run as agent sessions, read that text and the other sources (committee
reports, GLA reports, the Planning London Datahub, EPC certificates) into proposals under
`proposals/` in the data repository. `s106 results` builds each batch's scheme ledger from those
proposals and any labels, and the results explorer (`explorer/`) shows it on a map.

**Status.** The readings have no measured error rate yet. The data repository's
`docs/verification-rule.md` says how it is to be measured: independent LLM readers and external
sources (Datahub, EPC, Price Paid, Infrastructure Funding Statements) read the same cells, and their
agreement bounds the error. That estimate is not built. An AI agent transcribed the OCR gold pages,
so the OCR tests show repeatability, not correctness ([docs/testing.md](docs/testing.md)).

Architecture and contributor reading path: [docs/architecture.md](docs/architecture.md).

OCR speed, the default model set (PP-OCRv6 tiny) and choosing another with `S106_OCR_MODELS`: [docs/ocr-speed.md](docs/ocr-speed.md).

This repository holds code only; it describes no real planning application. The data and the
notes about real deeds are in the private [s106-data](https://github.com/MozarellaMan/s106-data)
repository.

What the outputs must not be read as saying: [docs/claims.md](docs/claims.md).

```
src/
  document.rs   PDF pages -> text with boxes: PDFium text layer, or PP-OCRv6 tiny OCR; cached by content hash
  ocrbench.rs   OCR scored against gold transcriptions (`gold/*.json` in the data repository)
  data.rs       where the research data lives (the s106-data checkout at `data/`)
  pld.rs        read-only Planning London Datahub client; saves raw snapshots under <data>/snapshots/pld/
  labels.rs     agent proposals scored against human labels (`labels/` in the data repository)
  results.rs    a batch's scheme ledger, decision rules and claims, each with its check level
  accuracy.rs   each reader's estimated accuracy from how often readers agree; sets the check levels
  explore.rs    data for the results explorer (`explorer/`): Datahub, plus deed cells or a private ledger
  figures.rs    `s106 check-figures`: each cited figure must still be in its cited file
```

## Setup

Rust 1.85+ (edition 2024). `cargo build` and `cargo test` need nothing else: no data, network, PDFs,
PDFium, ONNX Runtime or models. Reading PDFs needs PDFium, ONNX Runtime and the OCR models; the
fetch script checks each file against its pinned SHA-256 in `scripts/runtime-artifacts.tsv`:

```bash
scripts/fetch-models.sh
```

```bash
cargo build --release
```

### The data

The deeds' page text, OCR gold pages, proposals, labels and results live in the private repository `MozarellaMan/s106-data`, which needs granted access. Clone it
into `data/` (ignored by git), or point the `S106_DATA` environment variable or the global
`--data <dir>` flag at a checkout elsewhere. Then fetch the PDFs. They come from public planning
registers and are not committed anywhere:

```bash
git clone git@github.com:MozarellaMan/s106-data.git data
scripts/fetch-corpus.sh
```

`scripts/fetch-corpus.sh` reads `corpus/manifest.tsv` from the data checkout, downloads every
deed into `data/raw/<id>.pdf`, and refuses a file whose SHA-256 differs from the manifest.

### Starting without the data repository

Any folder with the same layout works as a data checkout (`src/data.rs` names every path). To
start one, find the executed deed for a planning application on the council's register, then:

1. Add it to `corpus/manifest.tsv` (id, `raw/<id>.pdf`, SHA-256, URL, session page or `-`) and
   run `scripts/fetch-corpus.sh <id>`.
2. Write its page text with `s106 pages data/raw/<id>.pdf --write data/pages` (needs
   `scripts/fetch-models.sh`).

The OCR test corpus was built from these applications, a mix of agreements, a unilateral
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
s106 pages <pdf>... --write data/pages                             # page text snapshots for the readers
s106 pages <pdf>... --check data/pages                             # read PDFs afresh and compare with the snapshots
s106 pld get applications <borough>-<reference>                    # save a Datahub record under <data>/snapshots/pld/
s106 labels                                                        # score the proposals against the human labels
s106 results --sheet runs/<batch>/labels.csv --out results/<batch>  # a batch's ledger, rules and claims
s106 explore                                                       # bake the results explorer's data (explorer/README.md)
s106 check-figures                                                 # every 29.8%<!-- src: path --> still in its cited file
```

(`cargo run --release --` instead of `s106` without installing.)

## Human labels

Under the data repository's `docs/verification-rule.md`, a person's label is one reader among
several, with its own error rate, not ground truth. Labels are optional: nothing waits for them.
A labeller saves a filled-in sheet (the source-audit codebook's columns, signed with `labeller`
and `labelled_on`) as a new file under `labels/` in the data repository. `s106 labels` reads every file there and
writes `reports/labels/labels.{md,json}`:

- the agents' proposals (under `proposals/` in the data repository) scored field by field
  against the labels: precision and recall with their n and a 95% interval;
- labellers against each other where two labelled the same cell (agreement and Cohen's kappa);
- sources against each other (deed, committee report, GLA report, Datahub, EPC).

With no labels the report says so and the command succeeds. Submissions are append-only: a
correction is a new file, and the latest `labelled_on` per labeller and cell wins. A row signed
`machine:` is refused.

A label's optional `method` column says how the person arrived at it: `read` (from the source,
the default) or `blind` are independent readings; `audited` or `corrected` are decisions on an
agent's proposal shown with its evidence. Proposals live under `proposals/` in the data
repository, signed `machine:<producer>`. `s106 labels` scores them like any other machine reading
and reports accuracy against independent labels and against anchored labels separately: how often
a person changed a proposal is not the same measurement as how often an independent reading agreed
with the proposal.

## Results

`s106 results` turns a batch into files a researcher can read and load. From the batch's blank
sheet (which cells exist), the labels and the proposals, it writes a folder under `results/` in the
data repository: a ledger with one row per scheme, variable and source in lineage order (offer,
decision, obligation, variation, completion), the batch's pre-registered decision rules worked out
scheme by scheme, retrieval per source, and a note that lists the hand-written claims in the
folder's `claims.csv`. A batch with no pre-registered rules runs with `--no-rules`, and every run
names the sheets whose schemes were seen while the method was built with `--seen` (batch 1's).

Every cell, rule outcome and claim carries a check level from the data repository's
`docs/verification-rule.md`: `confirmed`, `estimated`, `proposed` or `unread`. `src/accuracy.rs`
estimates each reader's accuracy per variable group from how often three readers from three
families (an LLM model family, people, an external source such as the Datahub) agree, leaving out
the seen schemes; a person's label is one more reader, not the truth. A cell is `estimated` when
its readers agree and the shown reader's lower bound passes 90%, and `confirmed` when, on top of
that, a reader that is not an LLM agrees and the cell's own estimated chance of error is at most 1%.
A rule or claim stands at the weakest level of the cells it rests on. Like `s106 labels`,
`s106 results` never waits for labels.

## Reproducibility

- **Page text** is cached under `data/interim/ocr/<sha256[:16]>-<ocr version>/` and kept in the data
  repository, per deed, as a snapshot in `data/pages/<stem>.pages.json`. Both are keyed by the PDF's
  SHA-256 and a hash of everything that produces the text: line-logic version, OCR settings and
  the pinned SHA-256 of PDFium, ONNX Runtime and each model file. Changing any of those inputs
  gives a new key. Artifact files that differ from their pins are refused, and the cache manifest
  is validated on read. Each image-only page is OCR'd on its own, so its text does not depend on
  other pages.
- **Runtime artifacts** (PDFium, ONNX Runtime, OCR models) are pinned by SHA-256 in
  `scripts/runtime-artifacts.tsv`.

## Tests

Four tiers, from always-on to on-request. Details, timings and how to update the snapshots:
[docs/testing.md](docs/testing.md).

```bash
cargo test                      # 1 unit tests: no data, PDFs, models or network
cargo test-data                 # 2 the real corpus's page-text snapshots, from the data checkout
cargo test-pdf                  # 3 difficult pages read afresh from the PDFs, OCR gold thresholds
cargo test-corpus               # 4 every deed read through PDFium/OCR, compared with its snapshot
```

Tier 2 (`cargo test-data`) needs the data checkout and fails with "data missing" without it.
Tiers 3 and 4 also need `scripts/fetch-models.sh` and `scripts/fetch-corpus.sh`, and fail with
"corpus missing" when deeds are absent; neither tier skips silently. CI (`.github/workflows/`) runs
`cargo test` on every push, plus a second job that checks out the data repository (secret
`S106_DATA_DEPLOY_KEY`) and runs `cargo test-data`. Tiers 3-4 run weekly on a cold cache, also
with the data checkout. Only tiers 1-2 gate pull requests. The weekly job skips the PDF tiers with
a warning when the council sites it downloads from are unreachable.
