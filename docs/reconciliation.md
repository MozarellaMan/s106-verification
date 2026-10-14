# Reconciliation: obligations against delivery (M5, first cut)

Status: draft, 13 Oct 2026. Code: `src/reconcile.rs`, `src/pld.rs`; tests: `tests/reconcile.rs`,
on an invented deed and an invented PLD record (`tests/synthetic/`).

This sets what a development's deeds oblige against what the Planning London Datahub (PLD)
records as permitted and delivered, for one development at a time.

## Data model

```
InstrumentObligations   what one deed says, before it meets the deeds before it
  instrument            sha256, kind, date, legal reference
  variation             deed of variation or supplemental deed
  planning_references   checked against PLD's lpa_app_no
  statements[]          one per obligation (duplicates merged, see below)
    measure             total_units | market_units | affordable_units | affordable_habitable_rooms
                        | tenure_units{tenure} | tenure_habitable_rooms{tenure}
                        | affordable_pct{basis} | contribution{key}
    label, value        the deed's own wording and figure (none for a formula, rate or no figure)
    evidence            page + verbatim snippet
    grounding           grounded | ungrounded(why) | not_checked   (the verifier's own rule)
    note, conflict      a condition; differing repeats within the deed
  deletions[]           targets the variation deletes, as the deed names them

ObligationEvent         one per obligation per instrument that states or deletes it (never overwritten)
  action                states | deletes
  value                 the figure this deed states, if any
  instrument_no         position in the order given, from 1
  verified, scope       whether the deed's planning reference was checked; its partial scope

Delivery                PLD's view of one development, read from one saved snapshot
  record_id, lpa_app_no PLD's `<borough>-<reference>` id, and the borough's planning reference
  snapshot, as_of       path, sha256, retrieved_at; every figure cites it
  mapping_version       the version of the PLD reading below
  status, started_on, completed_on, scheme_complete
  permitted             PLD's permission figures: total / market / affordable units, units by tenure
  units[]               id, tenure (market | affordable(T) | unrecognised(raw) | missing),
                        habitable rooms, completion date

Row                     one per obligation
  agreed                the first deed's figure
  current               the last figure a verified, whole-scheme deed states (shown, not applied)
  events[]              every deed's statement or deletion, with its citation and grounding
  permitted, delivered  PLD permission figure; count over unit records with a completion date
  status, reason        the status and why, in words
  cause                 a code when indeterminate (below)
  review[]              review reasons when a later deed did something (below)

Reconciliation          instruments, delivery, rows, warnings, and a summary:
  summary               rows; decided (rows not indeterminate or varied); rows by status;
                        indeterminate rows by cause; review rows by reason code
```

### Later deeds are recorded, not applied

Instruments are read in the order given, original first. The output warns when that order
disagrees with the deeds' dates. Every deed's statement of every obligation is kept as an event,
and the first deed's figure is the one compared with delivery. A later event leaves the row
automatic only when it restates the same figure, or cites the obligation with no figure, from a
deed that is verified (below), covers the whole scheme, and whose figure is not ungrounded.
Anything else adds a review reason, and the row becomes "varied: compare by hand". An obligation
that a later deed states first is checked the same way from that first statement, which counts as
a replacement of nothing (`replaced` with no `from`):

| Later event | Review reason (`reason`) |
|---|---|
| a different figure from the one before it | `replaced` (instrument, from, to) |
| a deletion naming this obligation | `deleted` (instrument, target) |
| from a deed with a partial scope (`instrument.scope`) | `scoped` (instrument, scope) |
| from a deed with no planning reference | `unverified` (instrument) |
| differing figures within that deed | `conflict` (instrument, figures) |
| a figure not found on its cited page | `ungrounded` (instrument, why) |
| a deletion matching no obligation (reaches every row stated before it) | `unmatched_deletion` (instrument, target) |

A deletion (the extraction's `variations` with op `Delete`) names a contribution when the
contribution's key appears in the deleted target. Its wording is not interpreted: whether it
deletes the whole obligation or some words of it is for the person reviewing the row.

One exception keeps a match automatic: when every review reason is a replacement and delivery
equals the latest figure from a verified, whole-scheme, grounded deed, the row is "counts match
(varied figure)". A match either way needs no judgement.

This is deliberately conservative. What counts as the current obligation after a variation needs
legal input (research plan, open question 3), and a wrong automatic answer would be a silent error
while a review row is only a cost. Applying variations automatically later changes only
`reconcile::facts`, not the event log; it should be built from the extraction's own variation
operations, each citing its operative clause, once the summary shows variation rows are a large
share of the review queue.

Contribution keys are their names in lower case with punctuation dropped and a leading "the"
removed, so "the Bus Contribution" in a variation is the same obligation as "Bus Contribution" in
the original. Within one deed, repeats with the same figure merge (a sum defined, then restated in
a schedule). Repeats with different figures (phased sums, or a misread) keep no figure and carry a
conflict: in the first deed the row is indeterminate, naming the figures; in a later deed it is a
`conflict` review reason.

### Same development

`reconcile` refuses a PLD record whose id isn't the `--development` given. Then every instrument
is checked on its own against one reference: PLD's `lpa_app_no`, or the first deed's planning
references when there is no PLD record (compared without case or separators). A deed citing a
different reference is refused, naming the instrument. A later deed citing no reference is read,
with a warning, and everything it says about an obligation is an `unverified` review reason.

## Status rules

`reconcile::decide`, evaluated in order. `tests/reconcile.rs::decision_table` lists the cases.

| Condition | Status | Cause |
|---|---|---|
| a later deed gave a review reason, and delivery equals a trusted latest replacement (above) | counts match (varied figure) | |
| a later deed gave a review reason | varied: compare by hand | |
| the first deed states differing figures for it | indeterminate | `conflict` |
| no stated figure in the deed (formula, rate, none) | indeterminate | `no_figure` |
| the deed value isn't grounded on its page | indeterminate | `ungrounded` |
| no delivered figure | indeterminate | `no_delivery_data`, `no_unit_records`, `no_payments`, `no_rooms`, `unknown_tenure` or `no_floorspace_basis` |
| delivered = agreed (percentages within the verifier's 0.05-point tolerance) | counts match | |
| PLD reports the scheme complete, delivered ≠ agreed | **differs: review** | |
| scheme not complete, delivered < agreed | not yet due | |
| scheme not complete, delivered > agreed | differs: review | |
| scheme status unknown | indeterminate | `scheme_status_unknown` |

When the deed splits affordable housing by tenure, affordable units delivered in a tenure no deed
tenure covers get their own "(not in deed)" row, marked "differs: review". A deed with only an
affordable total gets no such rows.

PLD permitted figures are not independent of the deed: the borough enters them from the decision
and the deed. A deed figure equal to PLD permitted checks the borough's data entry, not the
obligation; only delivered figures are an outcome, and they are borough-reported too.

"Counts match" compares completed-unit counts only. It says nothing about tenure in use, rents or
quality. "Differs: review" is never a finding that an obligation was unmet. PLD completions are
borough-reported and not quality-checked on receipt, and tenure at completion is not tenure in
use. A row in this state goes to a person, who checks the borough's records.

### Tenure matching

A deed tenure covers a delivered tenure when they're equal, or the deed uses an umbrella term:

- **Intermediate** covers Shared Ownership, London Shared Ownership, London Living Rent and
  Discount Market Rent.
- **Shared Ownership** covers London Shared Ownership.

PLD's "Intermediate Other" (Southwark records most intermediate homes this way) is an
intermediate home whose product isn't recorded. It counts towards affordable totals and towards a
deed's "Intermediate". A deed's named intermediate product (Shared Ownership, say) can't be counted
while such units exist, so its row is indeterminate (`unknown_tenure`), and they get no
"(not in deed)" row.

Social Rent does **not** cover London Affordable Rent or Affordable Rent. Their rents differ, so a
swap is a difference to review. Discount Market Sale and First Homes are not treated as
intermediate unless the deed says so. This matters in practice: deeds often say "Intermediate"
where PLD's records say "London Shared Ownership".

### Bases

Every percentage carries its basis. A deed percentage by habitable room is compared only with a
delivered percentage by habitable room, which needs rooms on every unit record; PLD's
application-level percentage (by units) is not compared with it.

## PLD access

`s106 pld get <index> <id>` and `s106 pld search <index> <query.json> --subject <label>` save each
response verbatim, with its request and retrieval time, to
`<data>/snapshots/pld/<index>/<subject>/<retrieved_at>.json`, where `<data>` is the s106-data
checkout (`--data`, `S106_DATA`, or `data/` here). Snapshots are never overwritten, and they are
committed to the data repository, not this one (PLD is OGL v3). Analyses read snapshots, never the
live API. `--pld` paths are relative to `<data>`, and each reconciliation cites its snapshot by that
path and its SHA-256.

## PLD → `Delivery` mapping (v2)

`reconcile::delivery_from_pld` reads one saved `applications/_source/<id>` snapshot
(`s106 reconcile … --pld <snapshot>`; there is no way to pass hand-written delivery JSON). It
reads the record through a typed view of only the fields below. Every one must be present. Those
PLD genuinely leaves null (status and dates before a scheme starts, a unit's number or tenure) may
be null, but not missing. So a renamed or retyped field is an error, not a table of indeterminate
rows or zero completions. Counts may be numbers or
numeric strings; a non-whole count or a date that isn't dd/mm/yyyy is an error. The field names
come from a real application snapshot taken 13 Oct 2026; `tests/synthetic/pld/application.json` is
an invented record with the same fields. Unit records are embedded in the application
record (`application_details.residential_details.residential_units`), so no separate units index
is read. `PLD_MAPPING_VERSION` is stamped into each reconciliation, and `as_of` is the snapshot's
retrieval time: a later snapshot is a new data point, not a correction.

| `Delivery` field | PLD field |
|---|---|
| `status`, `scheme_complete` | `status`; complete only when "Completed", not yet for "Started"/"Commenced"/"Not started" (any case), otherwise unknown (including "Lapsed") |
| `started_on`, `completed_on` | `actual_commencement_date`, `actual_completion_date` (dd/mm/yyyy) |
| `permitted.total_units` | `residential_details.total_no_proposed_residential_units` |
| `permitted.market_units` | `…_units_market_for_sale` + `…_units_market_for_rent` |
| `permitted.affordable_units` | `residential_details.total_no_proposed_residential_affordable_units` |
| `permitted.tenure_units` | `…_units_<tenure>` for social rent, London affordable rent, affordable rent, London living rent, London shared ownership, intermediate, discount market rent and discount market sale |
| `units[]` | `residential_units[]` with `change_type` "Gain" (a "Loss" is an existing home, not delivery) |
| unit tenure | `tenure`: "Market for sale"/"Market for rent" are market; an affordable tenure when spelled as a deed tenure (any case), and PLD's "Affordable Rent (not at LAR benchmark rents)" is Affordable Rent and "Intermediate Other" is Intermediate (below); null or empty is missing; anything else is unrecognised and kept verbatim |
| unit completion | `actual_completion_date` |
| unit id | `unit_no` when set, else `residential_units[<index>]` (records often have no `unit_no`) |
| habitable rooms | none: PLD unit records give `no_bedrooms`, not habitable rooms, so room-based rows stay indeterminate |

## Output

`s106 reconcile` prints the table and writes `<data>/processed/<development>.reconcile.{json,md}`.
Tables for real developments are kept in the data repository.

## Not done yet

- **Applying variations.** Later deeds go to review (above). When they are applied, it should be
  from the extraction's `variations` operations (`Replace` and `Amend` with `new_text`), each
  citing its operative clause, with the deed's `instrument.parent` as its identity check, which is
  better than a planning reference that variations often omit. Deletions of housing obligations
  (not only named contributions) also need a target to match.
- **Alignment by clause.** Obligations are matched across deeds by measure and normalised name.
  Matching by defined term, clause reference and amount, shared with PR #10's obligation
  inventory, would let a replacement point at what it replaces.
- **Triggers.** "Not yet due" only means the scheme isn't complete. Housing triggers (for example
  no more than 50% of market units occupied before the affordable units are delivered) can be
  checked against PLD's per-unit completion dates once the extraction records them.
- **Compressed snapshots.** `pld::load` reads plain JSON; large search snapshots in the data
  repository may be gzipped, and would need reading through a decompressor.
- **Amendments.** Non-material amendments (NMAs) and S106 discharge notices are linked to the
  parent only in PLD free text, and deeds of variation come from Idox. Each becomes events when it
  is extracted.
- **Contribution payments.** PLD doesn't record them, so contribution rows stay indeterminate
  until a payment source (an Infrastructure Funding Statement, or borough records) is added.
