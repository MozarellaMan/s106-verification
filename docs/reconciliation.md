# Reconciliation: obligations against delivery (M5, first cut)

Status: draft, 13 Oct 2026 (revised after the PR #12 review). Code: `src/reconcile.rs`, `src/pld.rs`; tests: `tests/reconcile.rs`.

This sets what a development's deeds oblige against what the Planning London Datahub (PLD)
records as permitted and delivered. It starts with 19/AP/1773 (227–255 Ilderton Road).

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

ObligationEvent         one per obligation per instrument that touches it (never overwritten)
  op                    introduce{value} | restate{value} | replace{from, to} | unchanged | delete

Delivery                PLD's view of one development, read from one saved snapshot
  record_id, lpa_app_no e.g. Southwark-19_AP_1773, 19/AP/1773
  snapshot, as_of       path, sha256, retrieved_at; every figure cites it
  mapping_version       the version of the PLD reading below
  status, started_on, completed_on, scheme_complete
  permitted             PLD's permission figures: total / market / affordable units, units by tenure
  units[]               id, tenure (market | affordable(T) | unrecognised(raw) | missing),
                        habitable rooms, completion date

Row                     one per obligation
  agreed                the introducing figure
  current               the fold of every event; none once deleted
  events[]              every event, with its citation and grounding
  permitted, delivered  PLD permission figure; count over unit records with a completion date
  status, reason
```

### Variations are operations

Instruments are applied in the order given, original first. The output warns when that order
disagrees with the deeds' dates. For each obligation a deed states:

| Before | This deed | Event |
|---|---|---|
| not yet stated | any figure, or none | introduce |
| stated | the same figure | restate (not a change) |
| stated | a different figure | replace |
| stated | cites it with no figure | unchanged |

A variation's `Delete` (from the extraction's `variations`) becomes a `delete` event on each
contribution whose key appears in the deleted target. It is applied, and the row is "removed by
variation", only when its cited snippet is found on its page, it comes from a verified deed
(below), and both the target and the snippet delete the whole obligation:

- In the target, and in every clause of the snippet (split at `.`, `;` and line breaks) that names
  the contribution, the words just before its name (back to the start or an "and") must be nothing,
  "the definition of", "the obligation to pay" or "all references to". "The words payable on
  commencement in the definition of Bus Contribution" is a partial deletion.
- Each of those clauses must say it is deleted (deleted, omitted, revoked, removed, struck out),
  with no word that could keep it ("retained", "not", "except", "save", "but", "unless",
  "remains") or narrow the deletion ("words", "phrase", "sentence", "text", "figure", "part",
  "extent", "in so far").

So "the Bus Contribution is retained. The Carbon Green Fund Contribution is deleted." does not
delete the Bus Contribution. A partial or ambiguous deletion stays in the history, the obligation
keeps its figure, and the row goes to "varied: compare by hand".
Otherwise the event is kept in the history but not applied, and the row goes to "varied: compare
by hand". A deletion that matches no obligation is reported as a warning, and every obligation
stated before it goes to "varied: compare by hand", because it could be the one deleted.

A row's state is a fold over its events, so it doesn't depend on which event came last:

- a figure (introduce, restate, replace) sets the current value and reinstates a removed
  obligation;
- an applied delete removes it, and a later `unchanged` citation leaves it removed;
- a conflict (differing figures in one deed) lasts until a later deed states a figure without
  one. The conflict stays in the event history either way.

Contribution keys are their names in lower case with punctuation dropped and a leading "the"
removed, so "the Bus Contribution" in a variation is the same obligation as "Bus Contribution" in
the original. Within one deed, repeats with the same figure merge (a sum defined, then restated in
a schedule). Repeats with different figures (phased sums, or a misread) keep no figure and the row
is indeterminate, naming the figures.

### Partial scope

A deed that states a partial scope (the extraction's `instrument.scope`, e.g. "in so far as it
relates to Phase 2") covers only part of the development, while every row here is for the whole
scheme. So what a scoped deed introduces, replaces or deletes is kept in the history with its
scope but not applied: the figure stays as it was and the row goes to "varied: compare by hand",
until an unscoped, verified deed states the figure again.

### Same development

`reconcile` refuses a PLD record whose id isn't the `--development` given. Then every instrument
is checked on its own against one reference: PLD's `lpa_app_no`, or the first deed's planning
references when there is no PLD record (compared without case or separators). A deed citing a
different reference is refused, naming the instrument. A later deed citing no reference is
applied, with a warning, but every obligation it changes (replaces, deletes or introduces) goes to
"varied: compare by hand", until a verified deed states the figure again. The events keep the
unverified step either way.

## Status rules

`reconcile::decide`, evaluated in order. `tests/reconcile.rs::decision_table` lists every
combination for a stated figure.

| Condition | Status |
|---|---|
| a variation deleted it | removed by variation |
| the deed states differing figures for it | indeterminate |
| no stated figure in the deed (formula, rate, none) | indeterminate |
| the current deed value isn't grounded on its page | indeterminate |
| a later deed deletes it, but the deletion isn't grounded or its deed isn't verified | varied: compare by hand |
| a deed with no planning reference changed it | varied: compare by hand |
| a deed limited to part of the development changed it (checked first, unless removed) | varied: compare by hand |
| a later deed deletes something unmatched | varied: compare by hand |
| no delivered figure (no unit records, unrecognised or missing tenure, missing rooms, a contribution) | indeterminate |
| delivered = current (percentages within the verifier's 0.05-point tolerance) | counts match |
| the figure was replaced and delivered ≠ current | varied: compare by hand |
| PLD reports the scheme complete, delivered ≠ current | **differs: review** |
| scheme not complete, delivered < current | not yet due |
| scheme not complete, delivered > current | differs: review |
| scheme status unknown | indeterminate |

When the deed splits affordable housing by tenure, affordable units delivered in a tenure no deed
tenure covers get their own "(not in deed)" row, marked "differs: review". A deed with only an
affordable total gets no such rows.

"Counts match" compares completed-unit counts only. It says nothing about tenure in use, rents or
quality. "Differs: review" is never a finding that an obligation was unmet. PLD completions are
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

## PLD → `Delivery` mapping (v1)

`reconcile::delivery_from_pld` reads one saved `applications/_source/<id>` snapshot
(`s106 reconcile … --pld <snapshot>`; there is no way to pass hand-written delivery JSON). It
reads the record through a typed view of only the fields below. Every one must be present. Those
PLD genuinely leaves null (status and dates before a scheme starts, a unit's number or tenure) may
be null, but not missing. So a renamed or retyped field is an error, not a table of indeterminate
rows or zero completions. Counts may be numbers or
numeric strings; a non-whole count or a date that isn't dd/mm/yyyy is an error. The field names
come from the 19/AP/1773 snapshot of 13 Oct 2026. Unit records are embedded in the application
record (`application_details.residential_details.residential_units`), so no separate units index
is read. `PLD_MAPPING_VERSION` is stamped into each reconciliation, and `as_of` is the snapshot's
retrieval time: a later snapshot is a new data point, not a correction.

| `Delivery` field | PLD field |
|---|---|
| `status`, `scheme_complete` | `status`; complete only when "Completed", not yet for "Started"/"Not started" (any case), otherwise unknown |
| `started_on`, `completed_on` | `actual_commencement_date`, `actual_completion_date` (dd/mm/yyyy) |
| `permitted.total_units` | `residential_details.total_no_proposed_residential_units` |
| `permitted.market_units` | `…_units_market_for_sale` + `…_units_market_for_rent` |
| `permitted.affordable_units` | `residential_details.total_no_proposed_residential_affordable_units` |
| `permitted.tenure_units` | `…_units_<tenure>` for social rent, London affordable rent, affordable rent, London living rent, London shared ownership, intermediate, discount market rent and discount market sale |
| `units[]` | `residential_units[]` with `change_type` "Gain" (a "Loss" is an existing home, not delivery) |
| unit tenure | `tenure`: "Market for sale"/"Market for rent" are market; an affordable tenure only when spelled as a deed tenure (any case); null or empty is missing; anything else is unrecognised and kept verbatim |
| unit completion | `actual_completion_date` |
| unit id | `unit_no` when set, else `residential_units[<index>]` (19/AP/1773's records have no `unit_no`) |
| habitable rooms | none: PLD unit records give `no_bedrooms`, not habitable rooms, so room-based rows stay indeterminate |

## 19/AP/1773 result

`data/processed/Southwark-19_AP_1773.reconcile.md` is the output (not committed: `data/` is local).
The table as run on 13 Oct 2026 is in [`reconciliation-19-AP-1773.md`](reconciliation-19-AP-1773.md).

## Not done yet

- **Variation extraction as operations.** Replacements and restatements are worked out by
  comparing figures, and deletions come from the extraction's `variations`. The extraction prompt
  should ask a deed of variation for operations directly, each citing its operative clause, and
  deletions of housing obligations (not only named contributions) need a target to match.
- **Alignment by clause.** Obligations are matched across deeds by measure and normalised name.
  Matching by defined term, clause reference and amount, shared with PR #10's obligation
  inventory, would let a replacement point at what it replaces.
- **Triggers.** "Not yet due" only means the scheme isn't complete. Housing triggers (for example
  no more than 50% of market units occupied before the affordable units are delivered) can be
  checked against PLD's per-unit completion dates once the extraction records them.
- **Snapshot size.** The full snapshot is committed (OGL v3, about 4,100 lines), because its hash
  is what the table cites. With many schemes, full snapshots could move to Git LFS or a data
  branch, with a mapped-field excerpt and the full file's hash committed for tests.
- **Amendments.** Non-material amendments (NMAs) and S106 discharge notices are linked to the
  parent only in PLD free text, and deeds of variation come from Idox. Each becomes events when it
  is extracted.
- **Contribution payments.** PLD doesn't record them, so contribution rows stay indeterminate
  until a payment source (an Infrastructure Funding Statement, or borough records) is added.
