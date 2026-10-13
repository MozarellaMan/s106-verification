# Research plan: S106 affordable-housing obligations in London

Status: draft, 13 Oct 2026. This is a working document; hypotheses here are to be tested, not assumed.

## 1. Repository assessment

### What is already strong

- **Deterministic gate.** Numeric checks (unit and tenure sums, mix table, figures vs words, trigger
  wording and labels) are plain code, unit-tested, and independent of whichever model read the deed.
- **Replayable model calls.** Each llm-pdf response is recorded with the PDF's SHA-256 and page
  selection, so audits re-run offline and refuse a different document.
- **OCR-robust evidence matching.** Two-column interleaving, `£`→`f` and `5o%`→`50%` are handled
  without accepting changed numbers. There are regression tests for each.
- **Run-to-run stability.** Trigger labels come from a fixed vocabulary applied to the cited phrase,
  not from the model. Across four live runs on 19/AP/1773, values were identical.

### Review concerns, checked against the code

| # | Concern | Finding | Severity |
|---|---|---|---|
| A | Stale OCR cache | **Confirmed.** The cache was keyed by filename stem only, so a replaced PDF reused old text, while replay fixtures *did* check SHA-256. OCR settings were not part of the key. | Correctness. **Fixed in M1** |
| B | Evidence validates the quote, not the value | **Confirmed, and worse than stated.** PASS paths never consulted evidence, so arithmetic alone passed. On FAIL, only snippet relocation was checked, never that the value appears in the snippet. | Research validity. **Fixed in M1** |
| C | `contingent` forced false when an amount exists | **Confirmed** (introduced in PR #3 for determinism). Southwark's £73,100 Default Employment contribution ("which *may be* payable") is reported as unconditional. Amount structure, conditionality and payment status are conflated. | Research validity. M2 |
| D | Passing checks ≠ complete extraction | **Confirmed.** Nothing compares what was extracted against what the deed contains. In excerpt mode (deeds over 60 pages), page selection can omit schedules silently. | Research validity. M3 |
| E | Basis fallback to units | **Confirmed.** A floorspace or unstated basis was computed by units. "Document Discrepancy" claimed "all inputs verified verbatim", which overstated OCR relocation. | Correctness. **Fixed in M1** |
| F | Tests depend on a local deed | **Confirmed.** `test_llm_pdf` skips without the Southwark PDF. There is no gold-labelled multi-deed set. The register "eval" mostly measured document mismatches (4 of 9 linked docs were variations or supplemental deeds). | Methodology. M4 |

### Further risks found during the audit

- **Page citations in excerpt mode.** In 4 of 31 Southwark citations, Haiku used the deed's own page
  number rather than the position in the excerpt. Every one is offset by 2, which is the start of
  the excerpt. Whole-deed sends (9 deeds, 79 citations) had none. M1 flags these as REVIEW instead
  of hiding them.
- **Instalment payments** (North Tyneside, Sunderland, Salford) don't fit one-trigger-per-contribution.
  They fail as Parser Errors.
- **Document type** isn't modelled. Deeds of variation and supplemental deeds are checked as if
  they were full agreements.
- **Tenure vocabulary** lacks Affordable Rent, First Homes and discount sale, and differs from PLD's
  London tenures.
- **No schema version or source provenance** in outputs (URL, retrieval date). The PDF hash is only in fixtures.
- **Rules backend** is tuned to Southwark's template. Treat it as a baseline, not a product.

## 2. London data sources

### Planning London Datahub (PLD)

A public, read-only Elasticsearch API (`planningdata.london.gov.uk/api-guest/`, guest header in the
[GLA API document](https://www.london.gov.uk/sites/default/files/planninglondondatahub_api_connection_technical_documentation_v1.pdf);
[PLD overview](https://www.london.gov.uk/programmes-strategies/planning/digital-planning/planning-london-datahub)).
It is licensed OGL v3 per the [London Datastore listing](https://data.london.gov.uk/dataset/planning-london-datahub).
The GLA warns that the data is not quality-checked on receipt.

Checked against 19/AP/1773 (record `Southwark-19_AP_1773`, retrieved 13 Oct 2026):

| Field | PLD | Deed | Usable? |
|---|---|---|---|
| Total / market / affordable units | 253 / 177 / 76 | 253 / 177 / 76 | Yes, as an independent cross-check |
| Tenure | 58 Social Rent, 18 London Shared Ownership | 58 Social Rented, 18 Intermediate | Yes, with a tenure mapping |
| Affordable % | 30.04 (computed by units) | 35.75 **by habitable room** | Only with the basis attached |
| Commencement / completion | 23 May 2022 / 19 Jul 2025, status Completed | n/a | Yes: delivery evidence (borough-reported) |
| Per-unit records | 253 units with tenure and completion date | n/a | Promising for delivery reconciliation |
| `s106_agreement` | **False** | the executed deed exists | **No.** Of 457 Southwark apps with ≥50 units, 40 have the flag set and 2 are True |
| Centroid | 49.77°N, 7.56°W (BNG origin) | n/a | No for this record |
| Amendments (NMAs, S106 discharge notices) | separate records, linked to the parent **only in free text** | n/a | Partly. Needs reference parsing |
| Deeds of variation (s106A) | not planning applications, so absent | n/a | No: borough documents needed |

**Implications.**

- **What PLD is good for:**
  - the sampling frame (approved major residential applications)
  - unit and tenure cross-checks
  - permission chains (`status: Superseded`, sparse `superseding_details`)
  - construction start and completion dates
- **What it can't provide:** finding or verifying S106 deeds, their terms, deeds of variation, or
  confirmation that obligations were met.
- **Southwark's Idox register is still required** for executed deeds and variations. That is one
  borough-specific fetcher, which the existing session-cookie approach already handles.
- **The national register** ([planning.data.gov.uk developer-agreement](https://www.planning.data.gov.uk/))
  has no usable London coverage with documents.

### Other sources to evaluate before relying on them

- **Southwark's own affordable-housing monitoring.** Its
  [monitoring page](https://services.southwark.gov.uk/planning-and-building-control/planning-policy-and-guidance/monitoring/monitoring-affordable-housing-delivery)
  sits alongside the annual audit agreed after a Local Government Ombudsman finding. Publication
  continuity is disputed by a campaign group
  ([35% Campaign](https://www.35percent.org/posts/southwark-abandon-annual-affordable-housing-monitoring-reports/));
  treat it as advocacy, not as a neutral source.
- **The GLA's legacy London Development Database** ([LDD, 2000–2019](https://www.data.gov.uk/dataset/8d0f9b32-89c8-4dcc-92d2-b3db10309182/planning-permissions-on-the-london-development-database-ldd))
  for historical permissions. Tenure is reliable only from 2004.
- **Councils' Infrastructure Funding Statements.** Annual and statutory, but aggregated. Sector
  lettings data on actual tenure in use is also worth evaluating.

## 3. Proposed architecture

A simple evolution. No database or service until flat files stop being enough.

```
documents/manifest.csv        one row per document: id, url, sha256, retrieved_at, doc_type,
  (in git; PDFs are not)      planning_refs, amends (id of the instrument it varies), licence note
        │
        ▼
extract  ──► <doc>.extraction.json  (schema_version, model + fixture ref, every value with evidence)
        │
        ▼
verify   ──► <doc>.audit.json       (checks with PASS / REVIEW / FAIL / SKIP + coverage report)
        │
        ▼  (Layer B, later)
obligation_events.jsonl   append-only: (development, obligation, value, source doc, evidence,
                          effective_date, kind = original | varied | superseded | discharged)
delivery_evidence.jsonl   append-only: (development, units/tenure/date, source = PLD | borough |
                          other, retrieved_at, confidence)
reconcile ──► per-development table: obligation as originally agreed → current (fold over events)
              → evidenced delivery → status ∈ {consistent, not yet due, varied, apparently unmet
              (human review required), indeterminate}
```

The principles behind it:

- **Nothing is overwritten.** Variations are new events, and the "current obligation" is computed.
- **Every row carries its source.** Each has a document hash and page or snippet, or a PLD record
  ID with its retrieval timestamp.
- **"Apparently unmet" is never a terminal conclusion.** It always routes to human review.
- **PLD is accessed through a thin read-only client** that stores raw JSON snapshots with
  timestamps. Analyses read the snapshots, never the live API.

## 4. Milestones

Each milestone is one or two PRs and has to pass its acceptance criteria before the next starts.

**M1: Evidence integrity** (this PR)
- **Changes:**
  - Content-addressed OCR cache (hash plus OCR version, with a manifest).
  - Field-level grounding: a value must appear in its cited snippet *and* the snippet must
    re-locate in the OCR text.
  - A REVIEW status for "holds but unproven".
  - Document Discrepancy only when every input is grounded, with a "confirm against original" caveat.
  - No units fallback for floorspace or unstated bases.
- **Acceptance:** regression tests for each review repro; the Southwark rules and replay workflow
  still runs; changes in outcome explained.

**M2: Schema v2 and provenance**
- **Changes:**
  - `schema_version`.
  - Document provenance: URL, SHA-256, retrieval date and document type (agreement, deed of
    variation, unilateral undertaking, supplemental).
  - Contributions split into amount structure (fixed / cap / formula / none), conditionality
    (unconditional / conditional + condition evidence / unknown) and instalments (share + trigger
    each).
  - Tenure vocabulary mapped to PLD's.
  - Checks gated by document type.
- **Acceptance:** Default Employment is reported as conditional with its condition quoted; North
  Tyneside and Sunderland instalments verify; variation deeds skip whole-scheme checks instead of
  failing; fixtures migrated with a documented script; old outputs still readable.

**M3: Coverage**
- **Changes:**
  - A deterministic inventory over *all* OCR pages: defined `…Contribution`/`…Cost` terms, `£`
    amounts, and unit-count definitions.
  - The inventory is compared with the extraction, giving per-deed coverage and a list of
    unexplained amounts (status `INCOMPLETE` when anything material is unaccounted for).
  - Fix or remove excerpt mode, which causes the page-citation offset.
- **Acceptance:** an omission-injection test (delete one contribution from a fixture) reports
  incomplete; Southwark coverage is 100% or every gap is explained.

**M4: Southwark gold set**
- **Changes:**
  - 5–10 major residential schemes, sampled from PLD (approved, ≥50 units, a range of decision
    years), with deeds and variations fetched by manifest.
  - Hand-labelled golden outputs: two passes, with disagreements logged.
  - Metrics: field-level precision and recall, grounding rate, REVIEW rate, coverage, run
    agreement, and the PLD unit/tenure cross-check.
  - Deeds aren't redistributed (licence unclear). The repo holds the manifest, hashes, fetch
    script and golden outputs.
- **Acceptance:** the eval report is reproducible from a clean checkout plus the fetch script.
  Missing-corpus tests report "corpus missing", not a silent skip.

**M5: Obligation history, one development**
- **Changes:** for 19/AP/1773 only, gather the amendments (NMAs and discharge notices found via
  PLD free text, and any deeds of variation from Idox) and PLD delivery data, then build the
  reconciliation table.
- **Acceptance:** every cell is traceable to a document page or a PLD snapshot; no automated
  "unmet" findings.

**M6: Scale-out.** 30–50 Southwark developments with measured failure rates, then a second borough.
Only after M4's metrics are acceptable.

**Layer C (economic research)** is out of scope until the questions below have answers.

## 5. Open research questions

1. **Sampling frame and selection bias.** Which permissions should carry an S106 with affordable
   housing, given that PLD's flag is unusable? How does deed availability on Idox vary with age,
   status (lapsed, superseded) and redaction? Missing documents are unlikely to be missing at random.
2. **What counts as "delivered"?** PLD completions are borough-reported. Tenure at completion
   isn't tenure in use. Which independent source (lettings data, borough audits) is reliable, and
   for which years?
3. **What is the "currently applicable" obligation?** Section 73 permissions with fresh deeds,
   deeds of variation, and early- or late-stage viability reviews all change it. That needs legal
   input before any status label beyond "varied" or "indeterminate".
4. **Measurement bases.** Habitable rooms (London Plan threshold), units and floorspace aren't
   interchangeable. 19/AP/1773 is 35.75% by rooms and 30.04% by units. Every comparison must
   carry its basis.
5. **Causality (Layer C).** Approved schemes alone can't show the effect of requirements on total
   supply: unbuilt and unsubmitted schemes are unobserved. It needs an identification strategy
   (policy thresholds or rule changes, borough variation), land-value and viability data, and
   pre-registration of hypotheses before looking at outcomes.
6. **Novelty.** A literature review of GLA monitoring reports, academic work on planning gain and
   viability, and prior S106 studies is needed before claiming anything is new.
7. **Neutrality and fairness.** Before any developer- or scheme-level finding is published: a human
   review protocol, an appeal and right-of-reply process, and a rule that "apparently unmet" is
   never reported without confirmation from the relevant authority's records.
