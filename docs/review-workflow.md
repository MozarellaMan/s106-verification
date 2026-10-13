# Multiple cases, deferred human review and dynamic reports

The workflow supports any number of explicitly registered developments. Adding a
case, committing tooling or running offline diagnostics does not wait for all
human reviews to finish. Select further developments later; this change does not
collect additional agreements or start scraping.

Each case has immutable agent-proposed `reference.json` and `provenance.json`, a
reference hash, a source PDF kept locally, and a separate human-review ledger.
Human decisions can confirm, correct, reject, flag ambiguity or add an omitted
obligation. Generated dossiers and reports combine these layers on demand; the
original agent output remains available for comparison and debugging.

## Registry and source registration

`validation/cases.json` maps a stable case ID to its input directory, local PDF and
optional existing Rust audit JSON. The current registry contains the existing
Southwark case; the plumbing is tested with two synthetic cases. It does not
pretend that multiple real developments have already been independently validated.

```bash
python3 scripts/manage-validation.py list

python3 scripts/manage-validation.py add \
  --id southwark-application-slug \
  --pdf data/raw/another-executed-agreement.pdf \
  --planning-ref APPLICATION_REFERENCE \
  --site-address 'SITE ADDRESS' \
  --council Southwark \
  --source-url 'OFFICIAL_PDF_URL' \
  --register-url 'OFFICIAL_APPLICATION_URL'
```

Supply `--reference FILE.json` for already prepared source-based agent annotations,
and `--audit FILE.json` for an existing Rust extraction/audit. Otherwise the case
starts with no claims and no accuracy result. Source registration records local
hash/page count; it **does not authenticate execution, official provenance or
legal type**. Actual retrieval time can be supplied using `--retrieved-at` with a
timezone; it is never invented from the registration time. Write source-reading
scope, annotation author and model-output exposure into reference metadata.

The registry paths are relative to its directory unless absolute. Source PDFs and
generated artifacts stay ignored; register only compact inputs and later human
records. This tool does not fetch PDFs, extract new obligations or call an LLM.

## Generate all reports, or select cases

```bash
python3 scripts/manage-validation.py build --serve
# http://127.0.0.1:8767/index.html

python3 scripts/manage-validation.py build --case southwark-19-ap-1773
```

The hub lists each case’s claim/review counts and status. Each dossier has source
images, imported human decisions, editable interpretation/field/evidence JSON,
explicit provenance/completeness decisions, and export. Outputs under
`data/processed/review-hub/` include `summary.json`, per-case `report.md`,
`comparison.json`, `reviewed-reference.json` and `review-history.json`. Source
images are cached by PDF/render settings and checked by hash before reuse.
A rebuild updates reports after review imports; no rerun of an LLM is required.

The existing `build-southwark-dossier.py --case PATH --pdf PATH --audit PATH` entry
point is retained for compatibility; it now accepts any compatible case inputs.
Case-specific locator rules live in `comparison-spec.json`, rather than inside the
comparator. No Southwark page numbers are applied to another case automatically.

## Import actual human decisions and corrections

1. Inspect the original source pages in a dossier.
2. Select a decision and enter the actual reviewer/date. Use **corrected** for
   changed interpretation, extracted-value expectations or source locators.
   Corrected claims require nonempty source evidence. The expected-fields editor
   replaces the claim’s full comparison field map; removing a field is an explicit
   recorded change, not a silent improvement in the score.
3. Export human review JSON, then import it:

```bash
python3 scripts/manage-validation.py import-review \
  southwark-19-ap-1773 /path/to/exported-human-review.json
python3 scripts/manage-validation.py build
```

The export is bound to its case directory, source SHA-256, exact provenance SHA-256,
reference version **and exact reference SHA-256**. Changes to source metadata or
reference bytes require an explicit new review/migration, not silent reassociation. Each decision also cites the previous review record it supersedes. Wrong
source/version, unknown claims, contradictory/stale updates, unsupported values,
missing reviewer/date and out-of-range pages are rejected before appending events.
Review imports use a local lock and exclusive creation; existing records are not
overwritten. Re-importing identical bytes is idempotent. A new reference version
requires deliberate re-review/migration, rather than silently reusing old decisions.

Records are appended under `<case>/reviews/` and can be committed independently of
reports. Each record carries import/file hashes, a predecessor hash, reviewer/date,
source/reference binding and the correction itself. Reports preserve historical
agent comparisons even after a human rejects or corrects those annotations.
These checks authenticate the **binding and structure**, not the person’s identity
or their legal reading. Nested table/instalment contents are not fully validated
against the Rust schema, and imported quotations are not automatically checked
against page pixels. These remain source-review tasks. Agents must never manufacture human decisions. Test records
are synthetic and are never registered as real human verification.

Browser edits are drafts until exported/imported. Existing CSVs are preserved on
regeneration; CSVs are not imported automatically. Old exports without the exact
reference hash should be re-exported using the current interface after checking
the source again. The previously preserved in-progress Southwark draft remains local;
it has not been silently imported or labelled as accepted ground truth.

## Add an omitted obligation during human review

Advanced review records may include a new claim without modifying the agent
reference. Use `human_review_status: "added"`, a fresh ID, actual reviewer/date,
`previous_review_sha256: null`, and a `new_claim` object:

```json
{
  "claim_id": "HUMAN01",
  "human_review_status": "added",
  "human_reviewer": "ACTUAL REVIEWER",
  "human_review_date": "YYYY-MM-DD",
  "human_notes": "Why this obligation was omitted",
  "previous_review_sha256": null,
  "new_claim": {
    "id": "HUMAN01",
    "title": "SOURCE OBLIGATION NAME",
    "priority": "high",
    "interpretation": "HUMAN SOURCE-BASED INTERPRETATION",
    "expected_fields": {},
    "evidence": [{"pdf_page": 1, "printed_page": "unknown", "clause": "SOURCE CLAUSE", "quote": "SOURCE QUOTATION", "coordinates": null}]
  }
}
```

This entry belongs inside the exported file’s `claims` array, retaining its binding
header and `format: "s106-human-review/1"` / `review_origin: "human_entered"`.
The example is a template, not a review or a real obligation. Supported comparison
paths use the housing, tenure and contribution schema; narrative-only obligations
may have empty expected fields and remain visible without an invented accuracy score.
Added obligations extend the human subset only. Later changes use explicit
supersession; colliding IDs are rejected. They appear as additional review cards
when reports are rebuilt.

## What reports establish

- **Agent-labelled agreement** compares Rust output to the original agent reference;
  it is a diagnostic, even after humans identify defects in that reference.
- **Human-confirmed agreement** compares only confirmed, corrected or human-added
  claims to the reviewed values. Rejected/ambiguous claims are excluded. Empty
  denominators produce unavailable metrics, never PASS or 100%.
- Counts for the two layers are kept separate, per case; the hub does not pool
  partial samples into a headline accuracy number.
- Numeric agreements, locator checks and document completeness are separate.
  Formula parameters or obligations absent from the schema remain coverage limits.
- `human_review_complete` requires all claims to be dispositioned and explicit
  human provenance and completeness confirmations. Confirming a few claims does
  not automatically mark the entire agreement complete.
- Nothing here establishes actual delivery, legal non-compliance or wrongdoing.

Controls: `python3 -m unittest discover -s tests/python -v` exercises synthetic
multi-case registration/building, immutable references, correction overlays,
source/version mismatch, stale conflicts, hash-chain corruption, additions and
separate metrics. Existing Rust mutation controls remain offline.

Change record — 13 October 2026: replaced the single-case review restriction with
a generic registry and separate correction ledger. Rendering/reporting can proceed
as the sample expands, while trusted-subset and completeness claims stay explicit.
