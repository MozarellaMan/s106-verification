# s106-verification

A verification harness for UK Section 106 planning agreements, in the spirit of BootLoops-style
exact quantitative checking. It extracts housing and financial obligations from an executed
deed, then runs them through a programmatic gate of hard numeric assertions. Every failure is
classified as either a **Parser Error** (our extraction is wrong) or a **Document Discrepancy**
(the deed disagrees with itself).

```
s106-audit/
  schema.py     Pydantic v2 model: Section106Agreement (metadata, housing/tenure, contributions + triggers)
  ocr.py        pdfplumber text layer -> parallel RapidOCR fallback for scanned deeds, cached per page
  extractor.py  definitions-clause + schedule parser (rules) or Claude over the page images (llm-pdf)
  fixtures/     recorded Claude responses so --backend llm-pdf runs offline
  verifier.py   hard assertions + Parser Error / Document Discrepancy classification
  run_audit.py  CLI: terminal report + JSON
```

## Setup

```bash
python3 -m venv .venv && .venv/bin/pip install -r requirements.txt
```

The sample deed is Southwark's S106 for **19/AP/1773** (227–255 Ilderton Road SE15, dated 28 Jan 2021).
It comes from the public planning register. It is not committed to the repo, so fetch it with:

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

## Run

```bash
.venv/bin/python s106-audit/run_audit.py                      # rules backend
.venv/bin/python s106-audit/run_audit.py --backend llm-pdf            # Claude reads the scans (offline: replays fixture)
.venv/bin/python s106-audit/run_audit.py --backend llm-pdf --cascade  # + re-read Parser Error fields on Opus
.venv/bin/python -m unittest discover -s tests                        # offline tests, mocked client
```

The JSON output goes to `data/processed/<stem>.audit.json`. Exit codes are `0` clean,
`1` parser errors, and `2` document discrepancies.

## LLM-PDF backend

`--backend llm-pdf` has Claude read the page images instead of the OCR + regex layer. RapidOCR
still runs (cached): it picks the definitions and the payment and mix-table pages (42 of 109 for the
sample), which are sent as one PDF document block to `claude-haiku-5-5` (change it with `--model`), and it
stays the independent text the verifier uses to match snippets and classify failures. Model output is never
used as the reference text.

`--cascade` re-reads only the fields with Parser Errors on `claude-opus-5-5` (`--cascade-model`). A
replacement is kept only if the verifier reports fewer Parser Errors for that field, runs at least as many
of its checks, and no previously extracted value is lost.

`--llm-mode` controls where responses come from:

| mode | behaviour |
|------|-----------|
| `auto` (default) | live if Anthropic credentials resolve (key file, env key, auth token or `ant auth login` profile), otherwise replay |
| `live` | always call the API; fails without credentials |
| `record` | call the API and save the response to `s106-audit/fixtures/<stem>/<model>.<kind>.json` |
| `replay` | load the saved response; errors if the PDF (sha256) or the page selection differs from the recording |

**Credentials for unattended runs.** `run_audit.py` reads a key from `~/.config/s106/anthropic_key`
(override the path with `S106_KEY_FILE`). The key goes straight to the client and is never exported, so
it doesn't reach Claude Code or other tools. Create the key in a dedicated Console workspace with a
monthly spend limit, then:

```bash
mkdir -p ~/.config/s106 && install -m 600 /dev/null ~/.config/s106/anthropic_key
```

Paste the key into that file in your editor. Without the file, the SDK's normal resolution applies
(`ANTHROPIC_API_KEY`, then an `ant auth login` profile), and `auto` mode falls back to replay.

The committed fixture is a live Haiku recording of the sample deed. Re-record it after changing the
prompt, the schema, the page selection or the deed:

```bash
.venv/bin/python s106-audit/run_audit.py --backend llm-pdf --llm-mode record
```

**Cost.** Scanned pages are billed as images, at roughly 1.7k input tokens each. The report's warning
line shows the token usage of each live call.

| call | rate (per MTok in / out) | per deed |
|------|-------------------|----------|
| Haiku 5.5 extraction | $0.10 / $0.50 under 100k prompt tokens, $0.50 / $2.50 above | ≈ $0.015 |
| Opus 5.5 cascade (only when Parser Errors remain) | $4 / $20 | ≈ $0.30–0.60 per call (estimate) |

**Known limitation.** On two-column definition pages, OCR splices the left-column term into the
right-column sentence wherever a line wraps, so a clean snippet the model read off the image may not
match the OCR text. Failures on such fields can only be classed as Parser Errors, never Document
Discrepancies. That errs on the safe side, but it weakens the classification.

## Checks

| ID | Assertion |
|----|-----------|
| H1 | market units + affordable units == total residential units |
| H2 | Σ tenure units == affordable units |
| H3 | Σ tenure habitable rooms == affordable habitable rooms |
| H4 | claimed affordable % == calculated % (on the deed's stated basis, ±0.05pp) |
| H5 | approved mix table: rows sum to each tenure's sub-total, sub-totals sum to the table total |
| H6 | mix table agrees with the definitions (per-tenure units, affordable units and habitable rooms) |
| F*.amt | a figure was extracted (SKIP for formula/review-contingent sums with no fixed figure) |
| F*.words | each contribution's figure == its words form ("£10,058.60 (ten thousand and fifty eight pounds and sixty pence)") |
| F*.trig | a payment trigger was found; the cited phrase and the full cited clause both re-locate in the OCR text, the phrase lies inside the clause, and the clause imposes a payment ("shall pay", "shall be payable", "To pay …", "until … paid"), not merely mentions one |
| F*.tlabel | the trigger label follows from the cited phrase under the fixed vocabulary in `s106-audit/triggers.py` (SKIP if the phrase is outside it) |

**Determinism.** Values that can be decided from the deed text don't depend on the model run.
A trigger's label comes from its cited phrase via the fixed vocabulary in `triggers.py` (the model's
label is kept as `trigger_event_model` when it differs). `contingent` is forced false when a figure
was extracted. Plural group terms ("Site and Development Contributions") are dropped from the
contribution list with a warning. Truncated structured output is retried up to three times.
*Limitations:* which clause the model cites, the words form it copies, and `contingent` for sums
with no figure can still vary between runs. Phrases outside the vocabulary leave the label
unverified (`F*.tlabel` SKIP).

**Classification.** When a check fails, each input value's evidence snippet is re-located in the
cited page, ignoring whitespace (OCR drops spaces). If any input is missing or can't be found,
the failure is a **Parser Error**. If every input is found verbatim, the failure is a
**Document Discrepancy**.
