# Schema v2: design notes (M1.6)

Schema v2 is designed from real instruments read on 13 Oct 2026, not from assumptions. Page
numbers are PDF page indices.

## What the documents actually do

### Deeds of variation and supplemental deeds

| Deed | Parent identified as | How it varies the parent | Scope |
|---|---|---|---|
| Dover DOV/19/00403 (second deed of variation, 2022) | "agreement … dated 6 February 2020", already varied by a "supplemental agreement dated 3 September 2021" | **Amend:** definition "Affordable Housing Scheme", replace "8" with "28". **Replace:** definition "Secondary School Contribution", now "the sum of £115,200". **Delete:** definitions "Qualifying Person" and "Restricted-Age Dwelling". **Delete:** Schedule 1 paras 4.3, 4.4, 4.6 | whole site |
| Dover DOV/18/00681 (deed of variation, 2022) | "Agreement … dated 14 September 2020" | **Amend:** "Chargee" (adds words). **Replace:** "Affordable Housing", "the Affordable Housing Units" (17 dwellings, 70% (12) Affordable Rented/Social and 30% (5) Shared Ownership), "Affordable Rented Housing", "Shared Ownership Housing". **Insert:** new definition 1.1.5A "Affordable Rent" | "in so far as it relates to the Affordable Housing Land" |
| T&M 24/00842/PA (DoV under s106A, 2024) | "Section 106 Agreement dated 13th April 2021" | **Replace:** "Affordable Housing Dwellings" (25%), "Affordable Rented Units" (≤ 80% of local market rent) | whole site |
| Bolton 01373/17 (supplemental deed, 2018) | "the Original Agreement" (Horwich Loco Works outline) | varies obligations for the Phase 1 land | Phase 1 Development Site only |

**Implications:**
- A variation is a list of **operations** (amend / replace / insert / delete) on **named targets**
  (a definition, a clause or a schedule paragraph). Some come with new wording.
- Variations name their **parent** by date, sometimes by parties. Chains exist: original →
  supplemental → second variation.
- **Scope can be partial**, e.g. limited to the affordable housing land or to one phase.
- A variation restates only what changes. Whole-scheme checks (market + affordable = total) don't
  apply to it, so they must SKIP for variations, not FAIL.
- Affordable-housing terms are what get varied, often with the new tenure split quoted in full.
  Housing values extracted from a variation are *replacement values*, not the scheme total.

### Conditional and structured payments

| Obligation | Amount | Condition | Payment trigger |
|---|---|---|---|
| Southwark Affordable Housing Contribution | none fixed: "any such Policy Surplus … shall constitute an Affordable Housing Contribution" (Sch 4 para 5.8, p66) | a viability review finds a surplus too small for whole units | "not Occupy … more than 50 per cent of the Market Housing Units … until … paid" (Sch 4 para 7, p67) |
| Southwark Default Employment in the End Use Contribution | £73,100 fixed (p12) | "which **may be** payable … in accordance with paragraph 3.4 of Part 1 of Schedule 9", i.e. job targets missed | the same paragraph |
| Southwark Carbon Green Fund Contribution | "up to a maximum" £475,991, which is a cap (p8) | the assessment may reduce it (Sch 11 para 3.3, p89) | "within 20 Working Days of such agreement or determination" (para 3.5) |
| North Tyneside, Sunderland (all sums) | fixed | none | **instalments:** "50% prior to Occupation of the 1st Dwelling; and …", "33% prior to …" |
| Salford 19/73971 | fixed | none | a lead-in "To pay to the Council" followed by percentage instalments on commencement and occupation |

**Implications:**
- Three independent facets, each with its own evidence:
  - **amount structure:** fixed, cap, formula, rate or none
  - **conditionality:** unconditional, or conditional with the condition quoted
  - **payment:** one covenant, plus zero or more instalments, each with a share and a trigger
- v1's single `contingent` flag conflated "no fixed figure" with "may never be payable".
  The £73,100 Default Employment sum has a fixed figure **and** is conditional.
- Payment *status* (paid or outstanding) is not in the deed. It belongs to Layer B
  delivery evidence, not to this schema.

## Schema v2 (summary; `src/schema.rs` is authoritative)

```
Agreement
  schema_version = "2"
  provenance     { source_path, sha256, page_count, text_source, ocr_version, extracted_by }
  instrument     { kind: agreement | unilateral_undertaking | deed_of_variation | supplemental | unknown,
                   evidence, parent: { description, date, evidence }?, scope: { description, evidence }? }
  metadata       { council, planning_references[], legal_reference, agreement_date, site_address, parties[] }
  housing        (v1 fields; tenures gain `label` = the deed's own wording; tenure list extended:
                  Affordable Rent, First Homes, Discount Market Sale, London Shared Ownership)
  contributions[]
    name, evidence
    amount         { structure: fixed | cap | formula | rate | none, value_gbp?, words?, index_linked, evidence? }
    conditionality { kind: unconditional | conditional | unknown, condition?, evidence? }
    payment        { covenant?: Quote, trigger_event?, trigger_phrase?: Evidence,
                     instalments[]: { share_pct?, trigger_event, trigger_phrase: Evidence } }
  variations[]   { op: amend | replace | insert | delete, target, new_text?, evidence }
  warnings[]
```

## Checks added for v2

- **V1 (instrument):** a variation or supplemental deed must cite a parent, and the parent's
  evidence is grounded. Whole-scheme housing checks (H1–H4) SKIP for variations and supplemental
  deeds.
- **V2 (variations):** each operation's evidence is grounded and states its target (and new
  value, if any).
- **F\*.cond:** a conditional contribution quotes its condition, and the quote is grounded.
- **F\*.inst:** when instalments are stated as percentages they sum to 100. Each instalment's
  trigger phrase is grounded and labelled by the trigger vocabulary. The covenant (or lead-in)
  imposes the payment.

## Migration of v1 data

v1 fixtures (recorded Haiku responses to the v1 prompt) are kept and still replay. They are mapped
to v2:
- `contingent` → `conditionality: unknown` and `amount.structure: none`
- `is_cap` → `structure: cap`
- the single trigger → `payment` with no instalments
- `instrument.kind: unknown`

New v2 fixtures are recorded from the v2 prompt.
