# S106 readings as a verification test bed: survey (14 Oct 2026)

Read-only survey of `/home/user/s106-data` (main at origin/main, after `git checkout -q -B main origin/main`) with comparison rules taken from `/home/user/s106-verification/src/accuracy.rs` (`compared()` lines 100-137, `agree()`, `totals()`) and `src/labels.rs` (`Reading`, `fields()`, `normal()`). Neither repo was modified. Every figure names the script that produced it; scripts are in `survey-scripts/`, raw outputs are not committed, since they are derived from the private data repository. Line numbers are physical file lines (`scripts/rows.py` returns the line a record starts on, so they match `grep -n`; several CSVs have multi-line quotes, so a plain record count would be wrong).

Paths are relative to `/home/user/s106-data/` unless stated. Abbreviations: `B1doc` = `proposals/2026-10-14-claude-opus-batch1-documents.csv`; `AGdoc` = `proposals/2026-10-14-agents-batch1-documents.csv`; `FU` = `proposals/2026-10-14-claude-opus-followup-documents.csv`; `DS` = `proposals/2026-10-14-claude-opus-batch1-delivery-sources.csv`; `EPC` = `proposals/2026-10-14-claude-opus-batch1-epc.csv`; `DH` = `proposals/2026-10-14-datahub-mapping-batch1.csv`; `LAB` = `labels/2026-10-14-ab-audit-priority-batch1.csv`; `R1B` = `results/2026-10-14-batch1/README.md`.

## Caveats that apply to everything below

- No cell is above `proposed`. `R1B:9-14` and `results/2026-10-14-batch2/README.md:9-14` report 0 confirmed, 0 estimated. The accuracy section (`R1B:20`) says "No estimate yet": no reader shares at least 5 cells with two readers from two other families.
- The 17 human labels (`LAB`) all have `method=audited` (direct count): the person saw the proposal first, so they are anchored on a Claude reading, not independent reads. `labels/README.md` ("Auditing proposals", `method` table) says `audited` labels are "anchored on the proposal they answered".
- Batch 1 cells "never count towards an accuracy estimate" (accuracy.rs header; `docs/audit-rule.md:17-18`), because Ayo has seen them.
- `runs/*reconcile*` and `runs/2026-10-14-southwark-joined` were built from Haiku extractions that were later deleted (`README.md`: "The recorded Haiku extractions ... were removed on 14 Oct 2026"). Treat their numbers as a historical record, not a reference.

## 1. Inventory

Scripts: `inventory.py` (`inventory.out`), `status_counts.py` (`status_counts.out`), `multireader.py` (`multireader.out`).

### 1.1 Files and producers

| file | labeller(s) | rows | content |
|---|---|---|---|
| `AGdoc` | `claude-fable-5-1` 136; `claude-opus-5-5` 16; `sources-status` 104 | 256 | 8 batch-1 schemes; Fable rows are superseded (`proposals/superseded.csv:2`); sources-status is the retrieval record (all `source_missing`), not a reader (`accuracy.rs reader()` returns None for it) |
| `B1doc` | `claude-opus-5-5` | 136 | deed 64, committee_report 32, gla 40; V1-V8 x 17 scheme-source contexts |
| `EPC` | `claude-opus-5-5` | 48 | 6 schemes, source `epc` |
| `DS` | `claude-opus-5-5` | 24 | `southwark_delivery`, `southwark_ifs`, `gla_programme`, 8 schemes |
| `FU` | `claude-opus-5-5` | 32 | committee reports of 19/AP/1710 and 19/AP/2307; new source `variation` for 09/AP/1870 and 19/AP/1710 |
| `DH` | `datahub-mapping` | 16 | V1 and V8 only, source `datahub`, 8 batch-1 schemes. No batch-2 Datahub rows exist (`STATUS.md` says the batch-2 mapping is next work) |
| `proposals/2026-10-14-claude-opus-batch2-r1.csv` / `-r2.csv` | `claude-opus-5-5-r1` / `-r2` | 40 each | 5 deeds x V1-V8, source `deed` only |
| `LAB` | `AB` | 17 | batch 1, all `audited`; 16 found, 1 ambiguous |
| `labels/2026-10-14-deck-decisions-export.json` | AB | 24 decisions | 18 confirm, 6 cant_tell (cant_tell cells have no label row) |
| `proposals/2026-10-14-audit-*.csv` (3 files) | none | - | routing/selection lists, no readings (no `labeller` column) |

Total readings loaded: 609 (including 104 retrieval-record rows). 312 distinct (scheme, variable, source) cells have at least one real reading.

### 1.2 Readings per (scheme, variable, source)

`multireader.py`: distinct labellers per cell (retrieval record excluded): 1 reader on 132 cells, 2 on 167, 3 on 13.

Schemes with 2+ readers:
- Batch 2, two Opus readers r1/r2 on all 8 deed cells each: 09/AP/1783, 11/AP/0139, 11/AP/1097, 11/AP/1180, 12/AP/1308 (5 schemes x 8 = 40 cells).
- Batch 1, Fable + Opus on the same cells (Fable is superseded, so it no longer votes): 09/AP/1870 (7 cells), 12/AP/3940 (22), 13/AP/3815 (13), 14/AP/4640 (21), 15/AP/2217 (14), 17/AP/4546 (14), 19/AP/1710 (16), 19/AP/2307 (16); plus 17 cells where AB also has an `audited` label.
- Cells with readings from 2+ distinct families: 17 (`multireader.py`), all Claude + the person AB. Claude + Datahub on one cell key: 0, because Datahub rows are keyed source `datahub`; `accuracy.rs votes()` re-reads Datahub V1 as a partial vote on the deed cell (skipped for schemes with a `variation` source: 09/AP/1870, 19/AP/1710).

### 1.3 Batch 2: r1 vs r2 agreement

Script: `batch2_agree.py` (`batch2_agree.out`). It re-implements `fields()`, `normal()` and `compared()` from accuracy.rs: status always compared; V1 all fields except `basis`; V2 `share, basis, split`; V3 `target` (first token) and `first_offer`; V4 `route, reviews`; V6 `cascade, pil, pil_amount`; V7 months (YYYY-MM) of dates when found; V8 `completed, completed_homes`; V5 status only (free text is not compared).

| variable | cells | status agrees | full agree (status + compared fields) |
|---|---|---|---|
| V1 | 5 | 5 | 5 (100%) |
| V2 | 5 | 3 | 3 (60%) |
| V3 | 5 | 5 | 5 (100%) |
| V4 | 5 | 5 | 4 (80%) |
| V5 | 5 | 5 | 5 (100%; status only) |
| V6 | 5 | 5 | 4 (80%) |
| V7 | 5 | 5 | 5 (100%) |
| V8 | 5 | 5 | 5 (100%) |
| all | 40 | 38 (95.0%) | 36 (90.0%) |

Status pairs (r1, r2): found/found 23, not_stated/not_stated 12, not_applicable/not_applicable 2, ambiguous/not_stated 1, ambiguous/found 1, ambiguous/ambiguous 1.

Per compared field, over cells where both readers said `found` (a field present on one side only counts as a difference):

| var | fields (agree/compared) |
|---|---|
| V1 | total 5/5, market 5/5, sr 4/4, int 3/3, so 2/2 |
| V2 | split 1/1 (only one V2 cell was found/found) |
| V3 | target 1/1, first_offer 1/1 |
| V4 | route 2/3, reviews 3/3 |
| V6 | cascade 3/3, pil 3/3, pil_amount 1/2 |
| V7 | months 5/5 |

V8 had no found/found cell (all five were not_stated); V5 is never field-compared; V2 and V3 samples are tiny. The 90% headline is dominated by abstention agreement: 12 of the 36 agreeing cells are not_stated/not_stated and 2 not_applicable.

### 1.4 Every r1/r2 disagreement (4 under the coded rule)

1. **09/AP/1783 V2** (`proposals/2026-10-14-claude-opus-batch2-r1.csv:3`, `-r2.csv:3`). Status differs.
   - r1: `ambiguous`, basis `units OR habitable_rooms`, pages `5-6,9,10,112-113`, value `share=26.5% (114/430 units) OR share=25.6% (367 of c.1,433 habitable rooms); split=74:40 SR:INT (units), 250:117 SR:INT (habitable rooms)`.
   - r2: `not_stated`, pages `5-6,9,23`, no value.
   - The deed states no percentage; r1 derived two shares, r2 declined.
2. **11/AP/0139 V2** (`-r1.csv:11`, `-r2.csv:11`). Status differs.
   - r1: `ambiguous`, pages `4,29`, `share=31.5% OR share=35.8%; split=22:18 SR:INT (units) OR split=94:50 SR:INT (habitable rooms)`.
   - r2: `found`, basis `units`, pages `4,10,29`, `share=31.5%; split=22:18 SR:INT`.
   - Same units-vs-habitable-rooms issue; r2 picked one basis.
3. **12/AP/1308 V4** (`-r1.csv:37`, `-r2.csv:37`). Field `route` differs.
   - r1: `found`, basis `other:viability_review`, pages `12,26,29-30`, `route=not_applicable; reviews=early`.
   - r2: `found`, basis `other:review`, pages `7,8,12,26,30`, `route=viability_tested; reviews=early`.
4. **09/AP/1783 V6** (`-r1.csv:7`, `-r2.csv:7`). Field `pil_amount` differs in wording only.
   - r1 `£100,000 per habitable room (deferred units only)`; r2 `£100,000 per habitable room of Deferred Affordable Housing Units`; both pages `9,12,86-87`, both `cascade=no; pil=yes`.
   - Semantically the same, but a string mismatch under the rule. If wording-only differences are discounted (my judgement, not the code's) agreement is 37/40 = 92.5%.

`results/2026-10-14-batch2/README.md:42,45,55` shows the same cells as "2 differ".

## 2. Datahub against deed readings

Script: `datahub_vs_deed.py` (`datahub_vs_deed.out`). The Datahub mapping covers V1 and V8 for batch 1 only. "Affordable" is computed as `accuracy.rs totals()` does: total minus market if both stated, else the sum of tenures; a total with no tenure is 0 affordable. `labels/` contains no Datahub values; its rows are AB's audited copies of Claude deed/committee readings.

| scheme | Datahub V1 (`DH` line) | deed reading (line in `B1doc`) | differs? |
|---|---|---|---|
| 09/AP/1870 | total 668, market 503, SR 118, INT 47 -> affordable 165 (`DH:2`) | total 668, SR 123, SO 47 -> 170 (`B1doc:2`; label `LAB:2`); committee report market 498, SR 123, SO 47 (`LAB:16`) | yes: 165 vs 170 (SR 118 vs 123; market 503 vs 498). Not voted in accuracy: the scheme has a `variation` source |
| 12/AP/3940 | total 489, market 489 -> 0 (`DH:12`) | deed total 489 only (`B1doc:18`) -> 0 affordable by `totals()` | no on the deed. The GLA-report reading is **492** (`B1doc:26`) vs 489 |
| 13/AP/3815 | total 55, market 41, AR 6, INT 7, SR 1 -> 14 (`DH:20`) | total 55, market 41, SR 1, AR 6, SO 7 -> 14 (`B1doc:34`) | no (INT vs SO label; tenures are summed) |
| 14/AP/4640 | total 119, market 119 -> 0 (`DH:31`) | total 119, SR 8, SO 8, `other=donor_site_habitable_rooms:46` -> 16 (`B1doc:58`; label `LAB:9`) | yes: 0 vs 16. The deed's 16 are off-site (Amelia Street) |
| 15/AP/2217 | total 94, market 75, INT 8, SR 11 -> 19 (`DH:39`) | 94/75/SR 11/SO 8 -> 19 (`B1doc:82`) | no |
| 17/AP/4546 | total 84, market 0, INT 68, SR 16 -> 84 (`DH:49`) | total 84, market 61, SR 16, SO 7 -> 23 (`B1doc:98`); committee report same 23 (`B1doc:90`) | yes: 84 vs 23 affordable; market 0 vs 61; intermediate 68 vs 7 |
| 19/AP/1710 | total 262, market 170, INT 28, SR 64 -> 92 (`DH:58`) | 262/170/64/28 -> 92 (`B1doc:106`) | no on the deed; the later `variation` source reads total 292, SR 64, INT 0 (`FU:18`) |
| 19/AP/2307 | total 219, market 160, INT 21, SR 38 -> 59 (`DH:68`) | 219/160/38/21 -> 59 (`B1doc:122`) | no |

3 of 8 schemes differ on the deed's V1 (09/AP/1870, 14/AP/4640, 17/AP/4546). The older Haiku-based reconcile found the same three: `runs/2026-10-14-southwark-reconcile/summary.md:28-36` (170 vs 165; 8 vs 0 twice; 61 vs 0; 23 vs 84) and `runs/2026-10-14-southwark-reconcile/README.md:25,28,29`.

Datahub V8 (completed homes, date): 09/AP/1870 668 on 2014-04-03 (`DH:8`); 13/AP/3815 55 on 2017-01-01 (`DH:27`); 15/AP/2217 94 on 2018-08-07 (`DH:45`); 17/AP/4546 84 on 2022-12-15 (`DH:54`); not_stated for 12/AP/3940 (`DH:16`), 14/AP/4640 (`DH:35`), 19/AP/1710 (`DH:64`), 19/AP/2307 (`DH:74`).

## 3. Case studies for Part 3B

Reference-answer key: **none** = no independent reading exists; **anchored label** = AB confirmed a Claude proposal (not independent); **primary instrument** = the document that settles it is in the repo but only one LLM reader has read it and no label exists.

### (a) Multiple documents describing one development

**A1. 09/AP/1870 (Canada Water, Completed in the Datahub).** Documents: original deed 17 Feb 2010 (`09AP1870-002`); deed of variation 17 Dec 2010 (recited, and read in `10AP2734-deed`); s73 permission 10/AP/2734 decision notice (`10AP2734-001`) and officer report (`10AP2734-002`); a further DoV dated 4 Feb 2014 (`09AP1870-001`; the register mislabels it 04/02/2004); 2009 committee report (`09AP1870-007`); applicant's planning statement (`09AP1870-010`).
- Original deed V1: SR 123 + SO 47 = 170 of 668 (`B1doc:2`).
- 2014 DoV: "no change to housing"; replaces fit-out with a £203,000 community contribution (`B1doc:8` notes).
- 2010 DoV/s73: "no housing change (report: undertakings not amended)" (`FU:8`); total 668 repeated, no tenure split (`FU:2`). `FU:8` records the doubt that the housing obligations may sit in a different agreement, because the DoV recital says "miscellaneous financial contributions".
- A verifier must resolve: whether 170 affordable is still the obligation after the 2010 and 2014 instruments, and why the Datahub says 165 (118 SR + 47 INT).
- Reference: none. Only the follow-up Opus reader has read the 2010 instruments. `runs/2026-10-14-southwark-reconcile/README.md:25` says "Check by hand whether [Affordable Rent in a 2010 deed] meant social rent."

**A2. 19/AP/1710 (Commenced).** Principal deed 4 Nov 2021 (the deed day is OCR-illegible, `B1doc:112`; the IFS gives 04/11/2021, `DS:16`); s73 24/AP/0255 (30/03/2025, +30 units to 292, affordable 39.8% -> 40.04%); first DoV 28 Mar 2025 (not on any register page: `runs/2026-10-14-source-audit-batch1/README.md:106-110`); second DoV 25 Jun 2026 under 25/AP/3794 (`FU:24`).
- A verifier must resolve: the current affordable obligation across four instruments, one not in hand.
- Reference: none for the sequence. The IFS (`DS:16`) independently dates the 2021 and 2025 deeds, which is the only independent evidence.

### (b) Conflicting or changing housing quantities

**B1. 19/AP/1710 total and tenure.**
- Original: 262 total, SR 64, INT 28, market 170 (deed `B1doc:106`; committee report `FU:10`; Datahub `DH:58`; Southwark OKR report `DS:8`).
- After variations: 292 total (+30), SR 64, INT 0 (`FU:18`, from `25AP3794-deed` pp 3,7,13-14 and the 24/AP/0255 decision notice). `FU:18`'s doubt: the deed's wheelchair definition (9 SR + 23 market = 32 = 12.2%) implies about 262 homes, so the text may not reflect 292.
- Within the original: GLA Stage 2 says "148 social rented units" while the bedroom breakdown sums to 64 (`B1doc:114`, status `inconsistent`); deed habitable rooms 366 vs 362 and SR 264 vs 260 (`B1doc:106` notes); committee dual-aspect table "46 of 90" vs 92/262 (`FU:10` notes).
- A verifier must resolve: which total and tenure count is in force, and whether "148" is a typo.
- Reference: none. `R1B:57-62` says the OCR reading of the 2026 deed "has not been checked against the page image and is not a finding".

**B2. 17/AP/4546 (Old Kent Road).** Three accounts of the same 84 homes:
- Deed: 23 affordable (SR 16, SO 7), 61 market (`B1doc:98`).
- Southwark OKR delivery report (May 2023): 61 affordable (SR 16, SO 45) plus the remaining 27.4% (23 by arithmetic) at London Affordable Rent, "not considered to be affordable" (`DS:7`).
- Datahub: 84 affordable (SR 16, INT 68), market 0 (`DH:49`).
- A verifier must resolve: whether the deed's 61 market homes became 45 SO + 23 LAR, and how the Datahub's 68 intermediate is built (45 + 23 = 68, `runs/2026-10-14-delivery-sources-batch1/README.md:38-47`). `R1B:49-55` says the report's p8 is not yet read by a person.
- Reference: none. `labels/audit-notes/audit-vs-checker-2026-10-14.md:9` says 17/AP/4546/V1/deed was held back from labels as a possible accidental swipe.

**B3. 12/AP/3940 total.** GLA Stage 1: 492 units (`B1doc:26`). Committee report, deed and Datahub: 489 (`B1doc:10`, `:18`; `DH:12`). The Fable committee-report reading notes "489 is after the April 2013 revision that removed 3 units (p5)" (`AGdoc:34`); the GLA Stage 1 report is dated 2013-02-27 (`AGdoc:76` notes, Fable). A verifier must resolve: that the 3 units are a revision, not a reading error. Reference: the committee report's own statement is a primary document; no label.

**B4. 15/AP/2217 tenure terms across drafts.** The Sept 2015 draft report (`15AP2217-004`) describes 11 "affordable rented" + 8 intermediate; the final report and deed say 11 social rented + 8 shared ownership (`B1doc:74` notes; `B1doc:82`). A verifier must resolve: which term binds (the executed deed). Reference: AB labelled the deed's values (`LAB:11`, anchored).

**B5. 13/AP/3815 offer change.** GLA Stage 1: 48 private + 7 "affordable rent", 12% (`B1doc:42` and `:43` notes); Stage 2, committee report and deed: 14 affordable (1 SR, 6 AR, 7 SO), 24%. A verifier must resolve: the final offer. Reference: deed label `LAB:5` (anchored).

**B6. 14/AP/4640 GLA unit count.** GLA Stage 2 says the Family Mosaic scheme gains 18 affordable units but gives "8 affordable rent and 8 shared ownership" (=16) (`B1doc:66`, `inconsistent`). Deed: 8 + 8 (`B1doc:58`). The checker notes the 8 + 8 are off-site (Amelia Street) and "not flagged as off-site" in the value (`labels/audit-notes/card-review-2026-10-14.md:12`). Reference: deed label `LAB:9` (anchored).

### (c) Measurement bases (units, habitable rooms, floorspace)

No floorspace basis appears in the readings; the contrasts are units vs habitable rooms (HR) vs cash.

| scheme | readings | where |
|---|---|---|
| 17/AP/4546 | Deed base 35% of HR; 89 HR of 252 = 35.3% (committee, `B1doc:91`); deed split `64:25 SR:SO (habitable rooms)` (`B1doc:99`); same affordable as units is 16:7 = 23. The results pipeline notes 64:25 adds to 89 not 100 and refuses to compare it (`R1B:37`, `R1B:43-47`) | `LAB:13-14` |
| 09/AP/1870 | Deed: units only (170 of 668 = 25.4%; no % or HR figure, `B1doc:3`). Committee report: 31.7% by HR, split 72.4:27.6 (`AGdoc:3`; label `LAB:17`), though its own note says 123:47 is a unit split | as listed |
| 13/AP/3815 | 24% on-site by HR; 25.5% by units; 31% HR including the £1.1m in-lieu payment (11 HR) (`AGdoc:79`, `B1doc:35`) | `LAB:6` |
| 14/AP/4640 | 0% on site; 28% by HR as an off-site equivalent (54 + 46 of 355 HR); the 8 + 8 are units while the 46 is HR (`B1doc:51`, `:58`); Datahub has 0 affordable | `labels/audit-notes/round2-review.md` item 3 |
| 12/AP/3940 | Deed share 0% (`B1doc:19`); GLA: 0% on-site vs 53% by units as the £65m off-site equivalent vs 17-18% by HR tested on-site (`B1doc:27`) | `LAB:3` |
| 19/AP/1710 | 39.8% HR (362 of 908) vs 35.1% by units (92/262) vs GLA 41% blended threshold (`FU:11`; round2 review item 8) | none |
| 15/AP/2217 | 30.0% by HR (93 of 310) vs 20.2% by units (`B1doc:75`, `:83`) | `LAB:12` |

A verifier needs to attach a basis to every stated share and refuse cross-basis comparison. Reference: AB labelled several (`LAB:6`, `:12`, `:13-14`, `:17`), all `audited` and so anchored. The checker flagged 17/AP/4546 V2 deed ("split=64:25 is room counts, not %") and 12/AP/3940 V2 deed ("share=0.0% marked found, deed states no %") (`labels/audit-notes/audit-vs-checker-2026-10-14.md:16,20`). No independent answer.

### (d) Deeds of variation changing obligations

**D1. 19/AP/1710, second DoV (25 Jun 2026).** Replaces all intermediate units with market units (`INT=0`, SR 64 = 260 HR); replaces the late stage review (triggered at 70% occupation, 35% of surplus profit after 16.28% developer profit, capped); an Early Stage Viability Review can no longer be triggered (`FU:18,21,24`). A verifier must resolve: the OCR reading against the page image, and the inconsistency that the 25/AP/3794 notice ties the variation to the 24/AP/0255 agreement while the deed recitals tie it to the 19/AP/1710 principal agreement (`FU:24` notes). The first DoV (28 Mar 2025) is not in the data. Reference: primary instrument, one reader.

**D2. 09/AP/1870, DoVs of 17 Dec 2010 and 4 Feb 2014.** Both read as not changing housing (see A1). Reference: primary instrument, one reader.

**D3. 12/AP/3940.** AB typed `orig 2014-03-28; DoV` for V7 before seeing the proposal, then confirmed the proposal `orig 2014-03-28` (`labels/audit-notes/audit-vs-checker-2026-10-14.md:21`; label `LAB:4`). The Opus reader notes the only deed in hand includes an unexecuted confirmatory-deed template, and clause 6 requires a deed of modification with any s73 (`B1doc:24`). Whether a variation exists is unresolved. Reference: none.

### (e) Obligations against recorded delivery

**E1. 17/AP/4546.** Datahub: 84 completed on 2022-12-15 (`DH:54`). The OKR report (as at May 2023) says under construction since 2020-03, completion expected summer 2023 (`DS:7`). EPCs: 84 new-dwelling certificates, Argo lodged 2023-11-02, Eden House 2024-11-06 (`EPC:49`). The Datahub decision date (13/09/2016) is two years before the register's 13 Sep 2018 (`runs/2026-10-14-source-audit-batch1/README.md:168-169`). Rule 2 counts it as met (84 vs 84) with EPCs lodged 322-692 days after the Datahub date (`R1B:52`). A verifier must resolve: when completion happened and what "completed" means in the Datahub. Reference: AB audited the EPC reading (84 certificates, `LAB:15`), which establishes an EPC count, not a completion date or tenure.

**E2. 09/AP/1870.** Datahub 668 completed 2014-04-03 (`DH:8`). EPCs are ambiguous between 283 (2013 cluster) and 668 (adding a 2011-04 to 2012-06 cluster); both last lodged 2013-05-17 (`EPC:9`; `R1B:47`). AB: cant_tell (`labels/2026-10-14-deck-decisions-export.json`, key `09/AP/1870/V8/epc`; checker: "283 subset unexplained"). Reference: none.

**E3. 13/AP/3815.** Datahub 55 completed 2017-01-01 (`DH:27`); EPCs first lodged 2017-04-20, 14 flats re-lodged 2017-12-08 (`EPC:25`). AB: cant_tell (checker: "last_lodged depends on definition"). Reference: none.

**E4. 15/AP/2217.** Datahub 94 completed 2018-08-07 (`DH:45`); 95 EPCs lodged 2018-01-30 to 2018-12-07 (`EPC:34,41`), one more than the Datahub, with a gap at 4 Maple Way. AB: cant_tell after 1,186 s (deck export). Reference: none.

**E5. 14/AP/4640 (Lapsed).** Deed obligations (16 off-site homes, £4.6m fallback) vs a Datahub with 119 market homes and no affordable (`DH:31`); EPC attribution `new_dwelling_epcs=0..9` with no certificate on the site address (`EPC:26`); the IFS shows no agreement under this reference in 2019-2025 (`DS:13`). A verifier must resolve: whether the Amelia Street scheme delivered the 8 + 8 and whether the £4.6m fell due. Reference: none.

**E6. 12/AP/3940 (Commenced).** £65m payment in lieu secured, no on-site affordable; the Datahub shows 489 market and no completion, but 49 EPCs for flats at Opus Tower were lodged 2026-10-07/08 (`EPC:10`). The IFS cannot show payments (`DS:11`). Reference: none.

### Independent reference summary

- Nothing above has an independently established answer. The 17 AB labels are all `audited` (anchored).
- Primary instruments in the repo that settle a question but are read by one LLM reader and are unlabelled: A1/D2 (2010 and 2014 DoVs), A2/D1 (`FU` variation rows; the 28 Mar 2025 DoV is absent).
- Cells where the deck recorded doubt (cant_tell, no label): E2, E3, E4, plus `19/AP/1710/V4/deed`, `19/AP/2307/V6/deed`, `09/AP/1870/V5/deed` (deck export).

## 4. Abstention (not_stated / source_missing / unreadable)

Script: `status_counts.py` (`status_counts.out`). Per reader, pooled across files:

| reader | n | found | ambiguous | inconsistent | not_applicable | not_stated | source_missing | no_source | unreadable |
|---|---|---|---|---|---|---|---|---|---|
| claude-opus-5-5 (5 files) | 256 | 112 | 10 | 2 | 41 | 78 | 8 | 5 | 0 |
| claude-fable-5-1 | 136 | 83 | 6 | 2 | 1 | 44 | 0 | 0 | 0 |
| claude-opus-5-5-r1 | 40 | 23 | 3 | 0 | 2 | 12 | 0 | 0 | 0 |
| claude-opus-5-5-r2 | 40 | 24 | 1 | 0 | 2 | 13 | 0 | 0 | 0 |
| datahub-mapping | 16 | 12 | 0 | 0 | 0 | 4 | 0 | 0 | 0 |
| AB (labels) | 17 | 16 | 1 | 0 | 0 | 0 | 0 | 0 | 0 |
| sources-status (retrieval record) | 104 | 0 | 0 | 0 | 0 | 0 | 104 | 0 | 0 |

Abstention (not_stated + source_missing + no_source + unreadable): Opus pooled 91/256 (36%); Fable 44/136 (32%); r1 12/40 (30%); r2 13/40 (32.5%); Datahub 4/16 (25%); AB 0/17. `unreadable` is used by nobody. `source_missing` is otherwise only the retrieval record (104 of 112 pooled). Per-file counts are in `status_counts.out`. AB only labelled cells that had a `found` or `ambiguous` proposal, so its 0% reflects selection, not behaviour. The 36 `not_applicable` EPC rows are by design (variables an EPC cannot carry).

## 5. Cases where a confirmed or agreed reading turned out wrong

There is no file titled as a post-mortem and no `confirmed` cell (`R1B:9-14`). The nearest records:

- `docs/audit-rule.md:10-12`: "In short, auditing every agent reading took about 45 minutes per scheme, and seeing the reading let wrong values through (17/AP/4546 V2 deed)."
- `labels/audit-notes/audit-vs-checker-2026-10-14.md:16` (row 10): "17/AP/4546/V2/deed | confirm | split=64:25 is room counts, not %; false rule-1 mismatch | **differ: needs Ayo**". The label is `LAB:14` (`share=35.0%; split=64:25 SR:SO (habitable rooms)`, audited).
- Same file, row 14: "12/AP/3940/V2/deed | confirm | share=0.0% marked found, deed states no % | **differ: needs Ayo**"; row 18: "12/AP/3940/V7/deed (blind) | typed \"orig 2014-03-28; DoV\", then confirmed \"orig 2014-03-28\" | no issue | **Ayo's own reading adds a variation: needs Ayo**".
- Same file, row 1: "17/AP/4546/V1/deed | confirm in 5 s | no issue | held back: possible accidental swipe", and line 5: "15 confirmed (17/AP/4546/V1/deed held back from labels), 0 corrected, 5 can't tell".
- `R1B:39-47` ("Pending in 2026-10-14-batch1"): "Ayo confirmed three cells in the deck, but the checker questioned them ... the cells are `proposed` like every other batch-1 cell ... `12/AP/3940/V2/deed` (share stated or not), `12/AP/3940/V7/deed` (deed of variation missing), and `17/AP/4546/V2/deed` (64:25 are habitable-room counts, not a split)."
- `R1B:49-51`: "c1 (17/AP/4546, deed 23 affordable vs Datahub 84) may be wrong as worded".
- `labels/audit-notes/round2-review.md` item 6 (14/AP/4640 V6): "'cascade=no' is questionable ... I would read cascade=yes, or at least flag ambiguous"; item 7 (17/AP/4546 V4): "I would mark the route as ambiguous (viability_tested OR fast_track), not settled." These are the checker's views on readings both Claude readers gave as settled.
- `labels/audit-notes/card-review-2026-10-14.md:11-21`: reader issues flagged against cells that Ayo then confirmed, e.g. "14 12/AP/3940/V2/deed: `share=0.0%` status found, but the deed states no %".
- `runs/2026-10-14-southwark-reconcile/README.md:44-45`: two scans of 17/AP/4546 give "1 vs 2 counts match and 2 vs 1 differs. That is extraction variation on one deed." (Haiku, now deleted.)

No file records a case where two independent readers agreed and later proved wrong; no independent third reader or ground truth exists to find one.

## Reproduction

```
python3 -I survey-scripts/inventory.py       > inventory.out   (outputs not committed: derived from the private data repository)
python3 -I survey-scripts/batch2_agree.py    > batch2_agree.out   (outputs not committed: derived from the private data repository)
python3 -I survey-scripts/datahub_vs_deed.py > datahub_vs_deed.out   (outputs not committed: derived from the private data repository)
python3 -I survey-scripts/status_counts.py   > status_counts.out   (outputs not committed: derived from the private data repository)
python3 -I survey-scripts/multireader.py     > multireader.out   (outputs not committed: derived from the private data repository)
python3 -I survey-scripts/cell.py <scheme> <V1,V2> [sources]   # any reading with file:line
```
