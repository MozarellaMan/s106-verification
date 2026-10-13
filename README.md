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

`--backend llm-pdf` replaces the OCR + regex reading layer with Claude reading the page images.

1. RapidOCR still runs (it's cached) and does three jobs. It locates the definitions clause and the
   later pages that impose payments or hold the mix table, which is 42 of the 109 pages for the sample.
   It feeds the rules pass for metadata. And it is the **independent corpus the verifier checks
   against**.
2. Those pages are split out with pypdfium2 and sent as one base64 PDF `document` block to
   `client.messages.parse` with a Pydantic output schema (`WireExtraction`). The default model is
   `claude-haiku-5-5`; pick another with `--model`.
3. The model cites evidence by its page position in the attachment. That position is mapped back to
   the deed page, and the verifier re-locates each snippet in the OCR text. It never checks against
   anything the model wrote.

`--cascade` takes every field with a Parser Error (the housing block, or a named contribution) and
re-reads only those fields on `claude-opus-5-5` (`--cascade-model`). The Opus call enables server-side refusal
fallbacks (`fallbacks="default"`). A replacement is kept only if the verifier then reports fewer
Parser Errors for that field, so the cascade can't make a field worse.

`--llm-mode` controls where responses come from:

| mode | behaviour |
|------|-----------|
| `auto` (default) | live if Anthropic credentials resolve (env key, auth token or `ant auth login` profile), otherwise replay |
| `live` | always call the API; fails without credentials |
| `record` | call the API and save the response to `s106-audit/fixtures/<stem>/<model>.<kind>.json` |
| `replay` | load the saved response; errors if the page selection changed since it was recorded |

The committed Haiku fixture is a live `claude-haiku-5-5` recording of the sample deed. Re-record it
after changing the prompt, the schema or the page selection:

```bash
ANTHROPIC_API_KEY=... .venv/bin/python s106-audit/run_audit.py --backend llm-pdf --llm-mode record
```

The report's warning line shows which source was used and, for live calls, the token usage.

**Cost.** The recorded sample run used 70,216 input tokens (42 scanned pages, about 1.7k each) and
14,553 output tokens (thinking included). That run took about 6 s.

| call | rate (per MTok in / out) | per deed |
|------|-------------------|----------|
| Haiku 5.5 extraction | $0.10 / $0.50 under 100k prompt tokens, $0.50 / $2.50 above | **$0.014** measured |
| Opus 5.5 cascade (only when Parser Errors remain) | $4 / $20 | ≈ $0.30–0.60 per call (estimate; same pages, more thinking) |

A deed of over ~58 targeted pages crosses Haiku's 100k-token price step.

**Known limitation.** On two-column definition pages, OCR splices the left-column term into the
right-column sentence wherever a line wraps. A clean snippet that the model read off the image then
won't re-locate in the OCR text. In the live sample run, 7 of 31 cited snippets fail to re-locate, all of
them figures-and-words snippets that wrap a line. Every check still passes, but if one of those fields failed it would be classed as a Parser Error, never a Document
Discrepancy. That errs on the safe side, but it weakens the classification for the llm-pdf backend.

## Checks

| ID | Assertion |
|----|-----------|
| H1 | market units + affordable units == total residential units |
| H2 | Σ tenure units == affordable units |
| H3 | Σ tenure habitable rooms == affordable habitable rooms |
| H4 | claimed affordable % == calculated % (on the deed's stated basis, ±0.05pp) |
| F:*:words | each contribution's figure == its words form ("£10,058.60 (ten thousand and fifty eight pounds and sixty pence)") |
| F:*:trigger | a payment trigger was located in the schedules |

**Classification.** When a check fails, each input value's evidence snippet is re-located in the
cited page, ignoring whitespace (OCR drops spaces). If any input is missing or can't be found,
the failure is a **Parser Error**. If every input is found verbatim, the failure is a
**Document Discrepancy**.
