# Follow-ups: test-seeded

## 1. Accuracy by client

Correct/n for each reader (its own label) and each condition (its final label), split by client; a condition row is mixed when its readings came from both clients.

| reader / condition | api | subagent | mixed |
|---|---|---|---|
| haiku | - | 30/40 | - |
| sonnet | - | 38/40 | - |
| opus | - | 39/40 | - |
| C1 | - | 30/40 | - |
| C2 | - | 30/40 | - |
| C3 | - | 38/40 | - |
| C4 | - | 37/40 | - |
| C5 | - | 7/9 | - |

## 2. Accuracy by gold label

Correct/n per gold label for each reader and condition, with Wilson 95% intervals in brackets.

| reader / condition | Entailment | Contradiction | NotMentioned |
|---|---|---|---|
| haiku | 21/26 (0.6212-0.9149) | 9/14 (0.3876-0.8366) | - |
| sonnet | 26/26 (0.8713-1.0) | 12/14 (0.6006-0.9599) | - |
| opus | 26/26 (0.8713-1.0) | 13/14 (0.6853-0.9873) | - |
| C1 | 21/26 (0.6212-0.9149) | 9/14 (0.3876-0.8366) | - |
| C2 | 21/26 (0.6212-0.9149) | 9/14 (0.3876-0.8366) | - |
| C3 | 26/26 (0.8713-1.0) | 12/14 (0.6006-0.9599) | - |
| C4 | 26/26 (0.8713-1.0) | 11/14 (0.5241-0.9243) | - |
| C5 | 5/5 (0.5655-1.0) | 2/4 (0.15-0.85) | - |

## 3. Exact McNemar tests

Paired exact McNemar tests (two-sided) on the items both conditions have; C2 vs C1 is skipped (same labels).

| comparison | pairs | b = first right and second wrong | c = first wrong and second right | p |
|---|---|---|---|---|
| C3 vs C1 | 40 | 8 | 0 | 0.0078 |
| C4 vs C3 | 40 | 0 | 1 | 1.0 |

C5 vs C3 skipped: C5 has 9 items, C3 has 40 (no full C5 run here).

## 4. Adjudication detail

Every item C4 sent to the adjudicator; readings A/B/C are Haiku/Sonnet/Opus in the order show() renders them, and change compares C4 with the C3 majority.

| item | gold | haiku (A) | sonnet (B) | opus (C) | majority | adjudicator | agreed with | change |
|---|---|---|---|---|---|---|---|---|
| test-06 | Entailment | NotMentioned | Entailment | Entailment | Entailment | Entailment | B,C | unchanged |
| test-07 | Contradiction | Entailment | Entailment | Contradiction | Entailment | Entailment | A,B | unchanged |
| test-15 | Contradiction | Entailment | Contradiction | Contradiction | Contradiction | Contradiction | B,C | unchanged |
| test-17 | Entailment | NotMentioned | Entailment | Entailment | Entailment | Entailment | B,C | unchanged |
| test-18 | Entailment | NotMentioned | Entailment | Entailment | Entailment | Entailment | B,C | unchanged |
| test-25 | Entailment | NotMentioned | Entailment | Entailment | Entailment | Entailment | B,C | unchanged |
| test-33 | Entailment | NotMentioned | Entailment | Entailment | Entailment | Entailment | B,C | unchanged |
| test-34 | Contradiction | Entailment | Contradiction | Contradiction | Contradiction | Entailment | A | right->wrong |
| test-39 | Contradiction | NotMentioned | Contradiction | Contradiction | Contradiction | Contradiction | B,C | unchanged |

Counts over those items (an adjudicator label can agree with more than one reading).

| count | items |
|---|---|
| right->wrong | 1 |
| wrong->right | 0 |
| unchanged | 8 |
| agreed with A | 2 |
| agreed with B | 8 |
| agreed with C | 7 |
| agreed with none | 0 |

## 5. Cost per correct answer

API cost per correct answer for each condition and for C0, both over the items whose calls are all API calls (ledger cost) in C0 and in that condition, and the ratio of the two. API records dropped are the condition's API records on items outside that set; subagent records are never in the API cost. A condition whose distinctive call (adjudication for C4/C5) has no API record is not estimable from API records; its last column uses the subagent records' estimated cost_usd.

| condition | items | API records used | API records dropped | subagent records (not costed) | cost USD | correct | cost per correct | C0 cost per correct (same items) | ratio to C0 | estimated ratio (incl. subagent cost_usd) |
|---|---|---|---|---|---|---|---|---|---|---|
| C1 | 0 | 0 | 0 | 40 | 0 | 0 | - | - | not estimable from API records | none (no API-only C0 item with a correct answer here) |
| C2 | 0 | 0 | 0 | 74 | 0 | 0 | - | - | not estimable from API records | none (no API-only C0 item with a correct answer here) |
| C3 | 0 | 0 | 0 | 120 | 0 | 0 | - | - | not estimable from API records | none (no API-only C0 item with a correct answer here) |
| C4 | 0 | 0 | 0 | 129 | 0 | 0 | - | - | not estimable from API records | none (no API-only C0 item with a correct answer here) |
| C5 | 0 | 0 | 0 | 36 | 0 | 0 | - | - | not estimable from API records | none (no API-only C0 item with a correct answer here) |

No condition shares an item made entirely of API calls with C0 in this run.
