# Reconciliation: obligations against delivery (M5, first cut)

Status: draft, 13 Oct 2026. Code: `src/reconcile.rs`, `src/pld.rs`; tests: `tests/reconcile.rs`.

This sets what a development's deeds oblige against what the Planning London Datahub (PLD)
records as permitted and delivered. It starts with 19/AP/1773 (227–255 Ilderton Road).

## Data model

```
ObligationEvent   one per measure per instrument (never overwritten)
  measure         total_units | market_units | affordable_units | affordable_habitable_rooms
                  | tenure_units{tenure} | tenure_habitable_rooms{tenure}
                  | affordable_pct{basis} | contribution{name}
  value           the deed's figure (a contribution's stated sum; none for formula/rate/none)
  kind            original (agreement, UU) | varied (deed of variation, supplemental deed)
  instrument      sha256, kind, date, legal reference
  evidence        page + verbatim snippet
  grounding       grounded | ungrounded(why) | not_checked   (the verifier's own rule)
  note            e.g. a contribution's condition

Delivery          PLD's view of one development, built from saved snapshots
  record_id       e.g. Southwark-19_AP_1773
  snapshots[]     path, sha256, retrieved_at          (every figure cites these)
  status, started_on, completed_on, scheme_complete
  permitted       PLD's permission figures: total / market / affordable units, units by tenure
  units[]         id, tenure (market | affordable(T) | unrecognised(raw)), habitable rooms,
                  completion date, snapshot index

Row               one per measure
  agreed          first event's value
  current         last event's value (instruments are given oldest first)
  events[]        every event, with its citation and grounding
  permitted       PLD permission figure
  delivered       count over PLD unit records with a completion date, and how it was counted
  status, reason
```

## Status rules

Evaluated in order:

| Condition | Status |
|---|---|
| no stated figure in the deed (formula, rate, none) | indeterminate |
| the current deed value isn't grounded on its page | indeterminate |
| no delivered figure (no unit records, unrecognised tenure, missing rooms, a contribution) | indeterminate (varied if a variation exists) |
| delivered = current | consistent |
| a variation exists and delivered ≠ current | varied: compare by hand |
| PLD reports the scheme complete, delivered ≠ current | **differs: review** |
| scheme not complete, delivered < current | not yet due |
| scheme not complete, delivered > current | differs: review |
| scheme status unknown | indeterminate |

Affordable units delivered in a tenure no deed tenure covers get their own "(not in deed)" row,
marked "differs: review".

"Differs: review" is never a finding that an obligation was unmet. PLD completions are
borough-reported and not quality-checked on receipt, and tenure at completion is not tenure in
use. A row in this state goes to a person, who checks the borough's records.

### Tenure matching

A deed tenure covers a delivered tenure when they're equal, or the deed uses an umbrella term:

- **Intermediate** covers Shared Ownership, London Shared Ownership, London Living Rent and
  Discount Market Rent.
- **Shared Ownership** covers London Shared Ownership.

Social Rent does **not** cover London Affordable Rent or Affordable Rent. Their rents differ, so a
swap is a difference to review. Discount Market Sale and First Homes are not treated as
intermediate unless the deed says so. This matters for 19/AP/1773: the deed says "Intermediate",
and PLD's application record says "London Shared Ownership".

### Bases

Every percentage carries its basis. The deed's 35.75% is by habitable room; it is compared only
with a delivered percentage by habitable room, which needs rooms on every unit record. PLD's
application-level 30.04% (by units) is not compared with it.

## PLD access

`s106 pld get <index> <id>` and `s106 pld search <index> <query.json> --subject <label>` save each
response verbatim, with its request and retrieval time, to
`snapshots/pld/<index>/<subject>/<retrieved_at>.json`. Snapshots are never overwritten. They are
committed: PLD is OGL v3. Analyses read snapshots, never the live API.

## Not done yet

- **PLD → `Delivery` mapping.** It needs the real field names for status, dates, tenure and unit
  records. The GLA API document names only `id`, `lpa_name`, `lpa_app_no`, `application_type`,
  `valid_date`, `decision_date` and `last_updated`. It also says unit details sit in a separate
  `residential_units` index. The mapping will be written against saved snapshots, not guessed.
- **The 19/AP/1773 reconciliation itself.** It follows once the snapshots are saved
  (`s106 reconcile data/raw/southwark_s106_sample.pdf --development Southwark-19_AP_1773
  --delivery <delivery.json>`).
- **Amendments.** Non-material amendments (NMAs) and S106 discharge notices are linked to the
  parent only in PLD free text, and deeds of variation come from Idox. Each becomes a `varied`
  event when it is extracted.
- **Contribution payments.** PLD doesn't record them, so contribution rows stay indeterminate
  until a payment source (an Infrastructure Funding Statement, or borough records) is added.
