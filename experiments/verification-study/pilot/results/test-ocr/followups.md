# Follow-ups: test-ocr

## 1. Accuracy by client

Correct/n for each reader (its own label) and each condition (its final label), split by client; a condition row is mixed when its readings came from both clients.

| reader / condition | api | subagent | mixed |
|---|---|---|---|
| haiku | - | 36/40 | - |
| C1 | - | 36/40 | - |
| C2 | - | 11/12 | - |

## 2. Accuracy by gold label

Correct/n per gold label for each reader and condition, with Wilson 95% intervals in brackets.

| reader / condition | Entailment | Contradiction | NotMentioned |
|---|---|---|---|
| haiku | 12/14 (0.6006-0.9599) | 13/13 (0.7719-1.0) | 11/13 (0.5776-0.9567) |
| C1 | 12/14 (0.6006-0.9599) | 13/13 (0.7719-1.0) | 11/13 (0.5776-0.9567) |
| C2 | 0/1 (0.0-0.7935) | - | 11/11 (0.7412-1.0) |

## 3. Exact McNemar tests

Paired exact McNemar tests (two-sided) on the items both conditions have; C2 vs C1 is skipped (same labels).

| comparison | pairs | b = first right and second wrong | c = first wrong and second right | p |
|---|---|---|---|---|
| C3 vs C1 | 0 | - | - | - |
| C4 vs C3 | 0 | - | - | - |
| C5 vs C3 | 0 | - | - | - |

## 4. Adjudication detail

Every item C4 sent to the adjudicator; readings A/B/C are Haiku/Sonnet/Opus in the order show() renders them, and change compares C4 with the C3 majority.

| item | gold | haiku (A) | sonnet (B) | opus (C) | majority | adjudicator | agreed with | change |
|---|---|---|---|---|---|---|---|---|

Counts over those items (an adjudicator label can agree with more than one reading).

| count | items |
|---|---|
| right->wrong | 0 |
| wrong->right | 0 |
| unchanged | 0 |
| agreed with A | 0 |
| agreed with B | 0 |
| agreed with C | 0 |
| agreed with none | 0 |

## 5. Cost per correct answer

API cost per correct answer for each condition and for C0, both over the items whose calls are all API calls (ledger cost) in C0 and in that condition, and the ratio of the two. API records dropped are the condition's API records on items outside that set; subagent records are never in the API cost. A condition whose distinctive call (adjudication for C4/C5) has no API record is not estimable from API records; its last column uses the subagent records' estimated cost_usd.

| condition | items | API records used | API records dropped | subagent records (not costed) | cost USD | correct | cost per correct | C0 cost per correct (same items) | ratio to C0 | estimated ratio (incl. subagent cost_usd) |
|---|---|---|---|---|---|---|---|---|---|---|
| C1 | 0 | 0 | 0 | 40 | 0 | 0 | - | - | not estimable from API records | none (no API-only C0 item with a correct answer here) |
| C2 | 0 | 0 | 0 | 12 | 0 | 0 | - | - | - |  |

No condition shares an item made entirely of API calls with C0 in this run.
