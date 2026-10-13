# s106-verification

A verification harness for UK Section 106 planning obligations, written in Rust.

Claude reads an executed deed (agreement, unilateral undertaking, deed of variation or
supplemental deed) into a typed schema. A deterministic gate then checks every number against
the others **and** against the deed's own text: each value must appear in the passage the model
cites, and that passage must be found on the cited page. Nothing in the gate is a model judgement.

Architecture and contributor reading path: [docs/architecture.md](docs/architecture.md).
Independent Rust review: [docs/rust-review.md](docs/rust-review.md).

Scope, roadmap and open research questions: [docs/research-plan.md](docs/research-plan.md).
Schema v2 and the real deeds behind it: [docs/schema-v2.md](docs/schema-v2.md).
How the Rust harness was accepted against the former Python one: [docs/migration.md](docs/migration.md).

```
src/
  document.rs   PDF pages -> text with boxes: PDFium text layer, or PP-OCRv5 OCR; cached by content hash
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
  ocrbench.rs   OCR scored against hand-transcribed gold (gold/*.json)
```

## Setup

Rust 1.85+ (edition 2024). Then fetch PDFium and the OCR models; each file is checked against a
pinned SHA-256:

```bash
scripts/fetch-models.sh
```

```bash
cargo build --release
```

Live extraction needs an Anthropic API key in `~/.config/s106/anthropic_key` (or
`ANTHROPIC_API_KEY`). Replaying recorded fixtures needs nothing.

### The deeds

Deeds come from public planning registers and are not committed. The Southwark sample is the S106
for **19/AP/1773** (227–255 Ilderton Road SE15, dated 28 Jan 2021):

```bash
mkdir -p data/raw && curl -sSL -c /tmp/sk -b /tmp/sk \
  "https://planning.southwark.gov.uk/online-applications/applicationDetails.do?keyVal=ZZZV0MKBWR618&activeTab=documents" -o /dev/null && \
curl -sSL -c /tmp/sk -b /tmp/sk -o data/raw/southwark_s106_sample.pdf \
  "https://planning.southwark.gov.uk/online-applications/files/CFCAC9FF6638E1A5FE9D3B098E9E71CD/pdf/19_AP_1773-AGREEMENT_28_01_2021-1089043.pdf"
```

The register needs a session cookie, which the first request picks up. The server also omits
its Sectigo intermediate certificate. If curl reports an SSL error, add the intermediate from
`http://crt.sectigo.com/SectigoPublicServerAuthenticationCAOVR36.crt` to a CA bundle and pass it
with `--cacert`.

The nine evaluation deeds are listed, with URLs, in `data/reference/candidates.tsv` and go in
`data/raw/eval/<id>.pdf`. Their SHA-256s are in the recorded fixtures (`fixtures/llm/<id>/`),
which refuse to replay against a different file.

## Run

```bash
s106 audit data/raw/southwark_s106_sample.pdf                      # replay if no credentials, else live
s106 audit data/raw/southwark_s106_sample.pdf --llm-mode record    # call the API and save the fixture
s106 audit <pdf> --escalate                                        # re-read Parser-Error fields with Opus 5.5
s106 eval --replay                                                 # all candidate deeds vs the register, offline
s106 eval --runs 3                                                 # live, three runs per deed (run-to-run agreement)
s106 bench-ocr gold/southwark_19_AP_1773.json data/raw/southwark_s106_sample.pdf --models ppocrv5-en-mobile,ppocrv4-mobile
```

(`cargo run --release --` instead of `s106` without installing.)

`audit` prints a report and writes JSON to `data/processed/<stem>.audit.json`. Exit codes are `0`
clean, `1` Parser Errors or REVIEW flags, and `2` Document Discrepancies.

**What a run costs.** Extraction is three structured-output calls over the whole PDF. A single
schema for everything compiles to a grammar the API rejects, and the PDF can't be cached across
calls whose schemas differ. With Haiku 5.5 a 30-page deed costs a few cents; the 109-page
Southwark scan costs about $0.30. Escalation to Opus 5.5 costs more and only runs with
`--escalate`.

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

- **Page text** is cached under `data/interim/ocr/<sha256[:16]>-<ocr version>/`, keyed by
  document content and OCR settings (renderer, model set, DPI), with a `manifest.json`. A
  replaced PDF or changed version string gets a different cache. Local model-file changes
  are not detected automatically; invalidate the cache when changing runtime artifacts.
- **Model responses** are recorded in `fixtures/llm/<stem>/<model>.extract.v2.json`, tied to the
  PDF's SHA-256 and to a hash of the prompts and schemas. A different deed or a changed prompt
  refuses to replay. v1 fixtures from the former Python harness (`fixtures/llm-v1/`) still replay.
- **Runtime artifacts** (PDFium, OCR models) are pinned by SHA-256 in `scripts/fetch-models.sh`.
- **Determinism:** trigger labels come from the cited phrase, not the model. Plural group terms
  are dropped. Truncated structured output is retried.

## Tests

```bash
cargo test                                              # offline, no deeds, no credentials
cargo test --release -- --ignored                       # also: OCR vs hand-verified gold, and the
                                                        # frozen Python M1 results (needs the deeds)
```

The ignored tests fail with "corpus missing" if the deeds aren't present; they never skip silently.
