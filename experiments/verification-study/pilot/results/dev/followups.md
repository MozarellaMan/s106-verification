# Follow-ups: dev

## 1. Accuracy by client

Correct/n for each reader (its own label) and each condition (its final label), split by client; a condition row is mixed when its readings came from both clients.

| reader / condition | api | subagent | mixed |
|---|---|---|---|
| haiku | 8/10 | - | - |
| sonnet | 9/10 | - | - |
| opus | 9/10 | - | - |
| C0 | 9/10 | - | - |
| C1 | 8/10 | - | - |
| C2 | 8/10 | - | - |
| C3 | 9/10 | - | - |
| C4 | 9/10 | - | - |
| C5 | 9/10 | - | - |

## 2. Accuracy by gold label

Correct/n per gold label for each reader and condition, with Wilson 95% intervals in brackets.

| reader / condition | Entailment | Contradiction | NotMentioned |
|---|---|---|---|
| haiku | 3/4 (0.3006-0.9544) | 3/3 (0.4385-1.0) | 2/3 (0.2077-0.9385) |
| sonnet | 4/4 (0.5101-1.0) | 3/3 (0.4385-1.0) | 2/3 (0.2077-0.9385) |
| opus | 4/4 (0.5101-1.0) | 3/3 (0.4385-1.0) | 2/3 (0.2077-0.9385) |
| C0 | 4/4 (0.5101-1.0) | 3/3 (0.4385-1.0) | 2/3 (0.2077-0.9385) |
| C1 | 3/4 (0.3006-0.9544) | 3/3 (0.4385-1.0) | 2/3 (0.2077-0.9385) |
| C2 | 3/4 (0.3006-0.9544) | 3/3 (0.4385-1.0) | 2/3 (0.2077-0.9385) |
| C3 | 4/4 (0.5101-1.0) | 3/3 (0.4385-1.0) | 2/3 (0.2077-0.9385) |
| C4 | 4/4 (0.5101-1.0) | 3/3 (0.4385-1.0) | 2/3 (0.2077-0.9385) |
| C5 | 4/4 (0.5101-1.0) | 3/3 (0.4385-1.0) | 2/3 (0.2077-0.9385) |

## 3. Exact McNemar tests

Paired exact McNemar tests (two-sided) on the items both conditions have; C2 vs C1 is skipped (same labels).

| comparison | pairs | b = first right and second wrong | c = first wrong and second right | p |
|---|---|---|---|---|
| C3 vs C1 | 10 | 1 | 0 | 1.0 |
| C4 vs C3 | 10 | 0 | 0 | 1.0 |
| C5 vs C3 | 10 | 0 | 0 | 1.0 |

## 4. Adjudication detail

Every item C4 sent to the adjudicator; readings A/B/C are Haiku/Sonnet/Opus in the order show() renders them, and change compares C4 with the C3 majority.

| item | gold | haiku (A) | sonnet (B) | opus (C) | majority | adjudicator | agreed with | change |
|---|---|---|---|---|---|---|---|---|
| dev-00 | Entailment | Contradiction | Entailment | Entailment | Entailment | Entailment | B,C | unchanged |

Counts over those items (an adjudicator label can agree with more than one reading).

| count | items |
|---|---|
| right->wrong | 0 |
| wrong->right | 0 |
| unchanged | 1 |
| agreed with A | 0 |
| agreed with B | 1 |
| agreed with C | 1 |
| agreed with none | 0 |

## 5. Cost per correct answer

API cost per correct answer for each condition and for C0, both over the items whose calls are all API calls (ledger cost) in C0 and in that condition, and the ratio of the two. API records dropped are the condition's API records on items outside that set; subagent records are never in the API cost. A condition whose distinctive call (adjudication for C4/C5) has no API record is not estimable from API records; its last column uses the subagent records' estimated cost_usd.

| condition | items | API records used | API records dropped | subagent records (not costed) | cost USD | correct | cost per correct | C0 cost per correct (same items) | ratio to C0 | estimated ratio (incl. subagent cost_usd) |
|---|---|---|---|---|---|---|---|---|---|---|
| C0 | 10 | 10 | 0 | 0 | 0.004595 | 9 | 0.000511 | 0.000511 | 1.0 |  |
| C1 | 10 | 10 | 0 | 0 | 0.00504 | 8 | 0.00063 | 0.000511 | 1.23 |  |
| C2 | 10 | 18 | 0 | 0 | 0.02139 | 8 | 0.002674 | 0.000511 | 5.24 |  |
| C3 | 10 | 30 | 0 | 0 | 0.289554 | 9 | 0.032173 | 0.000511 | 63.02 |  |
| C4 | 10 | 31 | 0 | 0 | 0.305418 | 9 | 0.033935 | 0.000511 | 66.47 |  |
| C5 | 10 | 40 | 0 | 0 | 0.499502 | 9 | 0.0555 | 0.000511 | 108.71 |  |
