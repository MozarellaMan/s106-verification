# Corrupted variants of sample40.jsonl

Built by `../../corrupt.py` (stdlib only, run `python3 -I corrupt.py` from the pilot folder). Seed `SEED = 20261014`; each item gets its own generator, `random.Random(f"{SEED}|ocr|{item_id}")`, so the output is byte-identical on every run. This was checked by running it twice, the second time with a different PYTHONHASHSEED. `../../verify_corrupt.py` runs the assertions on both files. The source `sample40.jsonl` is not modified.

## 1. `sample40_ocr.jsonl`: OCR-style character noise

- Applied evenly over the whole `document_text`, evidence spans included. The hypothesis, labels and all other fields are unchanged.
- Each document draws a target CER from N(0.020, 0.004), clipped to [0.01, 0.03]. That edit budget is split by Levenshtein cost: 70% single-character confusions (l→1/I, I→l/1, 1→l/I, O↔0, e↔c, S↔5, B↔8), 10% multi-character glyph merges and splits (rn↔m, cl↔d), 12% space noise (a dropped inter-word space or a doubled space), and 8% broken line hyphenations (a word of 9+ letters split as `confi-\ndential`). Sites do not overlap and are at least one character apart.
- Evidence offsets are remapped through a char-level index map, and `gold_evidence_spans[i][2]` holds the corrupted span text.
- Added fields: `cer` (realised CER = summed Levenshtein cost of the applied ops ÷ original length; an upper bound on the true edit distance) and `ocr_ops` (count of ops per family).
- Realised CER over 40 docs: mean 0.0207, min 0.0100, median 0.0207, max 0.0273 (quartiles about 0.0178 and 0.0233). Over all 40 docs: 6,224 single-character substitutions, 454 multi-character confusions, 1,068 space errors and 367 hyphen breaks. An independent check of true Levenshtein distance over the 56 gold spans gives a span-level CER of 0.0216, so the spans get the same noise as the rest of the text.

## 2. `sample40_seeded.jsonl`: one seeded meaning edit per item

- Entailment/Contradiction items: one contiguous edit inside one gold evidence span. Where possible it flips the label (E↔C). Where no flip is possible it changes a number or duration and keeps the label, with the new fact recorded in `seed_edit.new_fact`.
- NotMentioned items: one sentence is inserted after the sentence that contains a related anchor phrase, using the document's own party and information terms. Gold becomes Entailment. The inserted sentence becomes the only gold evidence span, and `gold_evidence_span_ids` is set to `[]` because it has no original span id.
- All later spans are shifted by the edit's length change, and the edited span's end moves with it.
- Added field `seed_edit`, which is `null` when the item is unchanged, otherwise:
  `{kind, original_text, new_text, char_offset, new_gold_label, old_gold_label, rule, span_index, note}`, plus `new_fact` for `number_change`. `char_offset` indexes the original document. `rule` is one of `generic`, `override`, `nm_table`.

| kind | n |
|---|---|
| sentence_insert (NM → E) | 13 |
| exception_change | 5 |
| negation_insert | 4 |
| scope_restriction ("only technical") | 3 |
| number_change (label kept) | 3 |
| negation_delete | 2 |
| modal_change | 1 |
| **edited total** | **31** |
| unchanged (`seed_edit: null`) | 9 |

Label transitions: NM→E 13, E→C 8, C→E 7, E→E (number change) 3. Gold labels after seeding: 26 Entailment and 14 Contradiction (6 of these are unchanged C items), with no NotMentioned left. The label mix is therefore different from sample40, so compare per item, not by overall accuracy. The 9 unchanged items are test-01, 04, 05, 09, 26, 30 (nda-2 "only technical" definitions that list business information in many places) and test-16, 20, 37 (several independent spans support the label, so no single edit flips it). The reason for each is in `seeded_edits.txt`.

**How edits were produced.** A generic rule list (negate or un-negate `may`/`shall`/`will`, `regardless of whether`→`provided that`, `shall continue to`→`shall cease to`, number swaps) is applied to the gold spans. Its output was reviewed in `seeded_edits.txt`. Only 4 generic edits were kept (test-07, 24, 33, 34). The other 27 edits are hand-written rules stored in the script:
- 14 E/C `OVERRIDES`. In 11 of these the generic rule had fired but its edit was rejected.
- 13 NM `NM_INSERTIONS`.
- 9 `SKIP` entries.

Two NM insertion points were corrected after the first review. In test-02 the sentence landed after "Inc." in the middle of a sentence. In test-35 it landed before "and (d)". The sentence-end finder now skips abbreviations and lower-case continuations, and test-02 has a new anchor.

Weaker edits to treat with care:
- test-24 ("shall continue to" → "shall cease to" be bound after return or destruction) contradicts survival only indirectly.
- test-17 and test-10 change a duration that is not what the hypothesis is about.
- test-25 and test-18 narrow a definition that is still worded "includes, but is not limited to" or "together with any analyses".

### Three example edits (verbatim)

1. test-34 (E→C, negation_insert, generic): `However, PwC may retain a copy of the confidential information for our internal purposes.` → `However, PwC may not retain a copy of the confidential information for our internal purposes.`
2. test-33 (C→E, exception_change, generic): `... or gathered by inspection, and regardless of whether such information is specifically identified as "confidential".` → `... or gathered by inspection, and provided that such information is specifically identified as "confidential".`
3. test-21 (NM→E, sentence_insert), inserted after `...or certify the destruction thereof.`: ` Upon the termination of this Agreement, Recipient shall promptly return to Disclosing Party or destroy all Confidential Information of Disclosing Party.`

## Caveats

- **The seeded edits and their new gold labels were written by a model (Claude) without human review.** A model read every edit in `seeded_edits.txt` and adjusted them, but no person has checked them. A sensible next step is for a person to hand-check all 40 items (31 edits and 9 skips) before the variant is used for headline numbers.
- Each new gold label was judged from the gold evidence spans only. Text elsewhere in a document could still support the old label.
- Several items share a document (e.g. test-11/test-32, test-21/test-28, test-22/test-23). Each record is edited independently, so the same document appears with different edits.
- The OCR noise is synthetic. It has no layout errors, dropped lines or column merges, and its error profile is uniform rather than bursty.
