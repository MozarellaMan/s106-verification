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
  extractor.py  definitions-clause + schedule parser (rules) or Claude structured outputs (--backend llm)
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
ANTHROPIC_API_KEY=... .venv/bin/python s106-audit/run_audit.py --backend llm --model claude-haiku-5-5
```

The JSON output goes to `data/processed/<stem>.audit.json`. Exit codes are `0` clean,
`1` parser errors, and `2` document discrepancies.

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
