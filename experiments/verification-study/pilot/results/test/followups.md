# Follow-ups: test

## 1. Accuracy by client

Correct/n for each reader (its own label) and each condition (its final label), split by client; a condition row is mixed when its readings came from both clients.

| reader / condition | api | subagent | mixed |
|---|---|---|---|
| haiku | 22/23 | 15/17 | - |
| sonnet | 22/22 | 16/18 | - |
| opus | 19/22 | 16/18 | - |
| C0 | 22/24 | 15/16 | - |
| C1 | 22/23 | 15/17 | - |
| C2 | 22/22 | 15/17 | 0/1 |
| C3 | 22/22 | 15/17 | 1/1 |
| C4 | 18/18 | 15/17 | 2/5 |
| C5 | - | 14/17 | 20/23 |

## 2. Accuracy by gold label

Correct/n per gold label for each reader and condition, with Wilson 95% intervals in brackets.

| reader / condition | Entailment | Contradiction | NotMentioned |
|---|---|---|---|
| haiku | 14/14 (0.7847-1.0) | 12/13 (0.6669-0.9863) | 11/13 (0.5776-0.9567) |
| sonnet | 14/14 (0.7847-1.0) | 12/13 (0.6669-0.9863) | 12/13 (0.6669-0.9863) |
| opus | 14/14 (0.7847-1.0) | 12/13 (0.6669-0.9863) | 9/13 (0.4237-0.8732) |
| C0 | 14/14 (0.7847-1.0) | 12/13 (0.6669-0.9863) | 11/13 (0.5776-0.9567) |
| C1 | 14/14 (0.7847-1.0) | 12/13 (0.6669-0.9863) | 11/13 (0.5776-0.9567) |
| C2 | 14/14 (0.7847-1.0) | 12/13 (0.6669-0.9863) | 11/13 (0.5776-0.9567) |
| C3 | 14/14 (0.7847-1.0) | 12/13 (0.6669-0.9863) | 12/13 (0.6669-0.9863) |
| C4 | 14/14 (0.7847-1.0) | 12/13 (0.6669-0.9863) | 9/13 (0.4237-0.8732) |
| C5 | 14/14 (0.7847-1.0) | 12/13 (0.6669-0.9863) | 8/13 (0.3552-0.8229) |

## 3. Exact McNemar tests

Paired exact McNemar tests (two-sided) on the items both conditions have; C2 vs C1 is skipped (same labels).

| comparison | pairs | b = first right and second wrong | c = first wrong and second right | p |
|---|---|---|---|---|
| C3 vs C1 | 40 | 1 | 0 | 1.0 |
| C4 vs C3 | 40 | 0 | 3 | 0.25 |
| C5 vs C3 | 40 | 0 | 4 | 0.125 |
| C4 vs C3, pooled clean+seeded | 80 | 0 | 4 | 0.125 |

Pooled caveat: clean and seeded share the same 40 item ids and mostly the same documents, so the pooled pairs are not independent.

## 4. Adjudication detail

Every item C4 sent to the adjudicator; readings A/B/C are Haiku/Sonnet/Opus in the order show() renders them, and change compares C4 with the C3 majority.

| item | gold | haiku (A) | sonnet (B) | opus (C) | majority | adjudicator | agreed with | change |
|---|---|---|---|---|---|---|---|---|
| test-03 | NotMentioned | NotMentioned | NotMentioned | Entailment | NotMentioned | Entailment | C | right->wrong |
| test-06 | Contradiction | Contradiction | Contradiction | Contradiction | Contradiction | Contradiction | A,B,C | unchanged |
| test-14 | NotMentioned | NotMentioned | NotMentioned | Entailment | NotMentioned | Entailment | C | right->wrong |
| test-21 | NotMentioned | NotMentioned | NotMentioned | Entailment | NotMentioned | Entailment | C | right->wrong |
| test-22 | NotMentioned | Entailment | NotMentioned | NotMentioned | NotMentioned | NotMentioned | B,C | unchanged |

Counts over those items (an adjudicator label can agree with more than one reading).

| count | items |
|---|---|
| right->wrong | 3 |
| wrong->right | 0 |
| unchanged | 2 |
| agreed with A | 1 |
| agreed with B | 2 |
| agreed with C | 5 |
| agreed with none | 0 |

## 5. Cost per correct answer

API cost per correct answer for each condition and for C0, both over the items whose calls are all API calls (ledger cost) in C0 and in that condition, and the ratio of the two. API records dropped are the condition's API records on items outside that set; subagent records are never in the API cost. A condition whose distinctive call (adjudication for C4/C5) has no API record is not estimable from API records; its last column uses the subagent records' estimated cost_usd.

| condition | items | API records used | API records dropped | subagent records (not costed) | cost USD | correct | cost per correct | C0 cost per correct (same items) | ratio to C0 | estimated ratio (incl. subagent cost_usd) |
|---|---|---|---|---|---|---|---|---|---|---|
| C0 | 24 | 24 | 0 | 16 | 0.012114 | 22 | 0.000551 | 0.000551 | 1.0 |  |
| C1 | 23 | 23 | 0 | 17 | 0.012736 | 22 | 0.000579 | 0.000523 | 1.11 |  |
| C2 | 22 | 38 | 1 | 29 | 0.044229 | 22 | 0.00201 | 0.000497 | 4.04 |  |
| C3 | 22 | 66 | 1 | 53 | 0.662115 | 22 | 0.030096 | 0.000497 | 60.51 |  |
| C4 | 18 | 54 | 13 | 58 | 0.564242 | 18 | 0.031347 | 0.000513 | not estimable from API records | none (10 subagent records have no cost_usd) |
| C5 | 0 | 0 | 67 | 93 | 0 | 0 | - | - | not estimable from API records | none (29 subagent records have no cost_usd) |
