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

## PLD → `Delivery` mapping

`reconcile::delivery_from_pld` reads one saved `applications/_source/<id>` snapshot
(`s106 reconcile … --pld <snapshot>`). The field names come from the 19/AP/1773 snapshot of
13 Oct 2026. Unit records are embedded in the application record
(`application_details.residential_details.residential_units`), so no separate units index is read.

| `Delivery` field | PLD field |
|---|---|
| `status`, `scheme_complete` | `status`; complete only when "Completed", not yet for "Started"/"Not started", otherwise unknown |
| `started_on`, `completed_on` | `actual_commencement_date`, `actual_completion_date` (dd/mm/yyyy) |
| `permitted.total_units` | `residential_details.total_no_proposed_residential_units` |
| `permitted.market_units` | `…_units_market_for_sale` + `…_units_market_for_rent` |
| `permitted.affordable_units` | `residential_details.total_no_proposed_residential_affordable_units` |
| `permitted.tenure_units` | `…_units_<tenure>` for social rent, London affordable rent, affordable rent, London living rent, London shared ownership, intermediate, discount market rent and discount market sale |
| `units[]` | `residential_units[]` with `change_type` "Gain" (a "Loss" is an existing home, not delivery) |
| unit tenure | `tenure`: "Market for sale"/"Market for rent" are market; an affordable tenure only when spelled as a deed tenure; anything else is unrecognised and kept verbatim |
| unit completion | `actual_completion_date` |
| unit id | `unit_no` when set, else `residential_units[<index>]` (19/AP/1773's records have no `unit_no`) |
| habitable rooms | none: PLD unit records give `no_bedrooms`, not habitable rooms, so room-based rows stay indeterminate |

## 19/AP/1773 result

`data/processed/Southwark-19_AP_1773.reconcile.md` is the output (not committed: `data/` is local).
The table as run on 13 Oct 2026 is in [`reconciliation-19-AP-1773.md`](reconciliation-19-AP-1773.md).

## Not done yet

- **Amendments.** Non-material amendments (NMAs) and S106 discharge notices are linked to the
  parent only in PLD free text, and deeds of variation come from Idox. Each becomes a `varied`
  event when it is extracted.
- **Contribution payments.** PLD doesn't record them, so contribution rows stay indeterminate
  until a payment source (an Infrastructure Funding Statement, or borough records) is added.
