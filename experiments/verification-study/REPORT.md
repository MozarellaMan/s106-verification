# Can an AI research pipeline verify its own answers? Audit and pilot (15 Oct 2026)

## 1. Research question

When a language model reads a long document and reports a fact, the project needs to know how often that
report is wrong, without a person checking each one. The production rule (s106-data `docs/verification-rule.md`)
answers with an agreement-based estimate: three readers from three "families" (an LLM family, people, an
external dataset) read the same cells; their pairwise agreement rates give each reader's accuracy; a cell
every reader agrees on, in a group whose lower-bound accuracy clears 90%, is `estimated`; a cell whose
posterior chance of error is at most 1%, with a non-LLM reader among the agreeing readers, is `confirmed`.

The precise question this study asks: **under which conditions does that chain (independent readers,
evidence quotes, a quote check, an entailment check, an external source, targeted re-reading) report high
confidence in a wrong answer, and how often?** The hypotheses in the brief are tested as falsifiable
predictions: agreement inflates confidence when errors are correlated; quotes stop fabricated citations but
not unsupported ones; external sources corroborate only when they observe the fact separately; targeted
follow-up is cheaper than more readers.

## 2. What the estimator assumes (audit of `src/accuracy.rs`, s106-verification `main` at 162d8bc, before #76)

The code is 437 lines. The estimate is `a_i = sqrt(A_ij · A_ik / A_jk)` where `A_xy` is the share of
shared cells on which readers x and y agree (`triplet()`, accuracy.rs:315-333). The lower bound is the 2.5th
percentile of that quantity over 1000 resamples of schemes (`estimates()`, accuracy.rs:335-400). A cell's
posterior error (`error()`, accuracy.rs:405-437) uses each reader's *lower bound* as its accuracy, a uniform
prior over the distinct observed readings plus "none of them", and treats agreeing wrong readers as giving
the same wrong value. `results.rs:185-262` then marks a cell `estimated` only when every reader agrees, the
shown reader's group passes the gate and at least two readers voted, and `confirmed` when the posterior error
is at most 1% and one agreeing reader is not an LLM.

The assumptions, each tested in section 3:

| | assumption | where it is written | what breaks it |
|---|---|---|---|
| A1 | two readers agree only when both are right: P(agree) = a_i·a_j | module doc, accuracy.rs:17-21 | small answer spaces (status is 2-valued; V4 route has 3-4 values) let wrong readers coincide. Platanios, Blum & Mitchell (2014) keep the both-wrong term; this code drops it |
| A2 | errors are independent across the three families | accuracy.rs:19-21 | every LLM and every person reads the same OCR text; a human label made after seeing the model's reading; an external source describing another lineage stage |
| A3 | one accuracy per reader per variable group; no item difficulty | accuracy.rs:13-15 | hard pages make every reader wrong at once (GLAD, Whitehill et al. 2009) |
| A4 | the scheme-bootstrap 2.5th percentile is a 97.5% lower bound, with as few as 5 shared cells (`MIN_SHARED`) | accuracy.rs:40, 376-392 | percentile bootstraps on 5-20 clusters are known to under-cover |
| A5 | misfit shows as a lower bound above 1 (`fits`) | accuracy.rs:271-272, 395 | correlated errors move the estimate up without pushing it past 1; and a bound of exactly 1 passed `lower <= 1.0` (fixed in #76, section 5.5) |
| A6 | `error()` using lower bounds and a uniform prior is conservative | accuracy.rs:405-415 | the same correlated errors that bias the estimate also bias the posterior |
| A7 | the triplet is the one whose least-shared pair shares the most cells | accuracy.rs:355-372 | selection on the data; not tested here |

Two further facts from the data survey (`s106-case-survey.md`) bound what the estimator can do
today: no reader shares five cells with two readers from other families, so no accuracy figure exists and
every one of the 312 read cells is `proposed`; the only external reader (the Datahub) votes on V1 total and
affordable count, so V2, V3, V4, V6 and V7 have no reader outside the LLM family and people. The 17 human
labels were made after seeing the Claude reading (`labels/audit-notes/audit-vs-checker-2026-10-14.md`).

## 3. Synthetic falsification

Code: `experiments/verification-study/synthetic/` (Rust, calls the unchanged `estimates()` and `error()`;
written by one model, reviewed and corrected by a second; one scenario bug found and fixed before the
numbers below were taken). Every figure here is from `results/summary.md` in that folder, 200 replications
per scenario, 20 schemes × 4 cells unless stated. Monte Carlo error on a coverage figure is about ±2 points.

**3.1 With independent readers the point estimate is right and the interval is not.** Ideal case S1: mean
estimate 0.906 for a true 0.900. But the lower bound sits *above* the truth in 10.0% of runs against a
nominal 2.5%; with 5 schemes 17.0%, with 8 schemes 12.5%. The gate therefore passes a reader sitting exactly
at 90% four to seven times as often as the rule claims. The diagnostic line shows why: the interval has the
right width on average (0.066 ≈ 1.9 sd) but narrows when the point estimate comes out high, which is exactly
when the gate is decided. Power is also low: a reader at 92% passes 16% of the time with 20 schemes and 17.5%
with 50 (S9).

**3.2 Shared input errors inflate the estimate and the fit check never fires.** When a share ρ of cells
carries a corruption that every LLM and every person follows (an OCR digit error), the estimates for those
readers stay at 0.90 while their true accuracy is 0.86 (ρ=0.05), 0.82 (ρ=0.10) or 0.74 (ρ=0.20); the
independent external reader is pushed *down* by the same amount (S4). This matches the closed form: with
A_12 inflated and A_13 = A_23, the formula returns the uncorrupted accuracy for readers 1 and 2 and
A_13/a for reader 3. `fits` stayed true in every run. Cell labels survived in S4 only because the external
reader broke unanimity on every corrupted cell: 0 wrong `confirmed` cells.

**3.3 Without an independent source, `confirmed` cells are wrong 5-11% of the time.** S11 removes the shield:
every reader follows the corruption. Cells reaching `confirmed`, with a stated posterior error under 1%
(mean 0.8-0.9%), were wrong 4.5% of the time at ρ=0.05 and 10.7% at ρ=0.10. The stated error is 10-30 times
too small. This is the production case for V2, V3, V4, V6 and V7, which have no reader outside the OCR text.

**3.4 Anchored human labels inflate the gate.** If the person copies the model's reading with probability q
(S12), the model's estimate rises to 0.932 (q=0.3) and 0.960 (q=0.6) for a true 0.900, and a reader exactly at
the gate passes 24.5% and 62% of the time. No wrong cell was confirmed, because the external reader stayed
independent and wrong values never coincided; the harm is to the published accuracy figure, not yet to cells.

**3.5 What did little harm.** Chance agreement on a 2-5 value answer space (S3) biases the two weaker readers
by at most +0.03 and let one wrong cell through `confirmed` in 56 (not distinguishable from 1%). A reader at
0.95 with two 0.60 partners is underestimated by 0.07 when K=3 (S8). Item difficulty (S5) and the external
reader reporting another lineage stage on whole schemes (S7) left the estimate within 0.01 of the truth,
because those errors do not make readers agree on the same wrong value. Cells whose truth is `not_stated` (S6)
behaved like the ideal case.

**3.6 What section 3 does not show.** It does not show that any real cell is wrong. It says nothing about the
real values of ρ (how often the OCR misleads every reader the same way), q (how often a label copied the model)
or the answer-space size; the pilot in section 5 measures the first of these on clean, OCR-noised and edited text. The partial Datahub vote, groups G2/G3 and more than three readers were not simulated.

## 4. The workflow as verification stages

Each stage, what it can and cannot catch, and the evidence (project data, literature, or section 3):

- **PDF text vs OCR.** 13 of 40 batch deeds are OCR'd scans. OCR error is upstream of every reader, so it is
  the one error source agreement cannot see (section 3.2-3.3; OHRBench reports a 14% RAG quality drop even
  with the best OCR). Catching it needs a reader of the *image*, not the text: a second OCR engine, or a vision
  model on the cited page. Neither exists in the pipeline today.
- **Answers with and without cited evidence.** Quotes make a reading checkable and stop fabricated citations.
  They do not stop a correct quote supporting a wrong label (Liu, Zhang & Liang 2023: 74.5% of citations
  supported their sentence). Measured in the pilot as quote-fail rate vs entailment-fail rate on passed quotes.
- **Deterministic quote check.** Cheap, exact, and already in the rule (section 3 of verification-rule.md) but
  not built. It fails honestly on OCR text only if the reader quotes the OCR "errors and all", which the
  producer brief requires.
- **Semantic entailment check.** A second model that sees only the quotes and the label. It is correlated with
  the reader when both are the same vendor; the pilot measures how often it disagrees with a passed quote.
- **Independent readings by several agents or families.** Catches a reader's idiosyncratic slips. Does not
  catch shared slips; same-vendor readers share more (Kim et al. 2025: 60% agreement when both wrong). The
  project's two batch-2 readers are both Opus 5.5 and agreed on 36 of 40 cells; 14 of those 36 are joint
  abstentions, so the headline overstates agreement on values.
- **Corroboration from separate documents or datasets.** The Datahub permitted-units figure is an officer's
  entry from the same permission, not a separate observation of the deed, and it differs from the deed on 3 of
  8 batch-1 schemes for reasons that are not reader error (off-site units, a variation, tenure coding). EPC and
  Price Paid observe the built stock, which is a different stage. Section 3.2 shows that an independent reader
  is what keeps corrupted cells out of `confirmed`, so the value of a source is its independence, not its
  authority.
- **Contradictory or outdated sources.** 19/AP/1710 has four instruments taking it from 262 to 292 homes and
  28 intermediate homes to market; a reader given only the principal deed is right about the deed and wrong
  about the obligation. The rule's "compare at the same lineage stage" handles this only when the stage is known.
- **Targeted follow-up.** Adjudication on disagreement is a cascade whose trigger is disagreement. It can only
  fire on cells where readers disagree, so by construction it never touches a shared error (section 3.3). Its
  value is bounded by how many errors are idiosyncratic; the pilot measures that share.
- **Abstention.** 30-36% of the project's LLM readings are `not_stated` or similar. Joint abstention counts as
  agreement in the estimator. When the truth is "not stated" that is right; when a value exists on an unread
  schedule it is a shared miss that looks like agreement. S6 shows the estimator is fine when abstention is
  honest; nothing in the data says how often it is not.

## 5. Pilot on ContractNLI: clean text, OCR noise, seeded edits

Everything in this section comes from `pilot/results/<run>/{metrics,items,readers,estimate,followups,control-adjudication,replicate}.csv`,
written by `pilot/analyse.py`, `pilot/followups.py`, `pilot/control_adjudicate.py`, `pilot/replicate_compare.py` and the Rust crate
`pilot/estimate/` from the saved call records under `pilot/runs/`. The design was fixed before the test run
(`pilot-design.md`); prompts were tuned on 10 dev pairs only and are frozen at commit 868f16c (the dev records are
in `pilot/runs/dev/`; the dev run ended 11 seconds before the test run began). The follow-up analyses and the two
controls in 5.7 were added after the outside review of 15 Oct 2026 and were not pre-registered.

### 5.1 Setup

Data: 40 (document, hypothesis) pairs from the ContractNLI test split, stratified by label (14 Entailment,
13 Contradiction, 13 NotMentioned), seed 20211005, drawn from 34 distinct documents. NotMentioned is both a label
and the abstention answer. Three variants of the same 40 pairs:

- **clean**: the published text;
- **ocr**: the text with simulated OCR damage at about 2% character error rate (`pilot/corrupt.py`,
  character substitutions, dropped characters, joined and split words), gold unchanged;
- **seeded**: 31 of the 40 documents given one meaning-changing edit (13 inserted sentences that make a
  NotMentioned hypothesis entailed, 5 exception changes, 4 inserted negations, 2 deleted negations, 3 scope
  restrictions, 1 modal change, 3 number changes), recorded with the edit in `pilot/data/contractnli/seeded_edits.txt`;
  9 documents left unchanged where no single edit would flip the label. The three number changes keep the label
  (Entailment stays Entailment) and test only whether the quoted number follows the edit, so 28 edits change the
  label. After seeding the gold is 26 Entailment, 14 Contradiction and no NotMentioned.

**The seeded edits and their new gold labels were written by a model (Claude), not by a person.** The first
version of this report said "worked out by hand"; that was wrong, and `pilot/data/contractnli/CORRUPTION.md` had
said so all along. After the outside review, a blind second coder relabelled every edited item: an Opus 5.5
subagent that saw only the edited document and the hypothesis, with no gold, no edit description and no reader
output (`pilot/second-coder/`). It is a model too, of the same family as the readers, so it is a second opinion and
not ground truth. It agreed with the model-written gold on 29 of 31 edits. The two it rejected are test-07
(negation inserted; the coder, like the outside reviewers, reads the edited clause as still entailing the
hypothesis) and test-39 (negation deleted; the coder reads the result as Entailment, the old label). On test-24
(modal change), which the reviewers thought still Entailment, the coder gives Contradiction, agreeing with the new
gold. So two seeded gold labels are probably wrong and one is contested, and the seeded accuracies below are
against the model-written gold as recorded, with those items named wherever they matter.

Conditions, each built from primitive calls that are saved once and reused:

| | condition | calls |
|---|---|---|
| C0 | one cheap reader, no evidence | Haiku 5.5 |
| C1 | one cheap reader with verbatim quotes, deterministic quote-on-page check | Haiku 5.5 |
| C2 | C1 plus an entailment check that sees only hypothesis, label and quotes | + Sonnet 5.5 |
| C3 | three readers (Haiku, Sonnet, Opus 5.5) with quotes; majority label; estimator run on their labels | Haiku + Sonnet + Opus |
| C4 | C3 plus Opus adjudication of items where readers disagree or a quote fails | + Opus on flagged items |
| C5 | C3 plus Opus adjudication of every item | + Opus on all items |

How the calls were made matters for what the numbers can say. Dev and clean test items 00-21 went through the
API from the owner's machine ($1.23 in total, `pilot/runs/*/ledger.csv`). The API then hit the workspace spending
limit, and the owner asked for the rest to run as Claude Code subagents: clean test items 22-39, every OCR and
seeded call, and every adjudication. The bridge (`pilot/bridge.py`) exports each pending call as the exact prompt
and schema the API client would have sent; one subagent of the matching model answers it with no other tool or
context; the answer is ingested as a record marked `client: subagent`. The subagent's model name is self-reported
and checked only by substring. Those records have no token counts or latency, so **token, latency and dollar
columns cover API records only** and the variant runs report none. Everything else (labels, quotes, confidence,
flags) is comparable across clients, with the caveat tested in 5.7. The clean C0 condition was not rerun on the
variants; C5 on the seeded set has only the 9 adjudications that C4 triggered, so it is reported as incomplete.
Every call ran once; run-to-run variation was not measured except by the replication in 5.7.

### 5.2 Clean text: readers are good, the adjudicator makes things worse

| condition | accuracy (Wilson 95%) | wrong items | flagged |
|---|---|---|---|
| C0 Haiku, no quotes | 37/40 = 0.925 (0.80-0.97) | 22, 23, 29 | 0 |
| C1 Haiku + quotes + quote check | 37/40 = 0.925 | 22, 23, 29 | 1 (quote fail) |
| C2 C1 + Sonnet entailment check | 37/40 = 0.925 | 22, 23, 29 | 6 (5 entailment flags + the quote fail) |
| C3 three readers, majority | 38/40 = 0.950 (0.84-0.99) | 23, 29 | 5 (disagreement) |
| C4 C3 + adjudicate flagged | 35/40 = 0.875 (0.74-0.95) | 03, 14, 21, 23, 29 | 0 |
| C5 C3 + adjudicate all | 34/40 = 0.850 (0.71-0.93) | 03, 14, 21, 23, 29, 31 | 0 |

Per reader: Haiku 37/40, Sonnet 38/40, Opus 35/40. Dev (10 pairs) had the same shape: C0-C5 at 0.8-0.9,
no condition better than the cheapest. The Wilson intervals treat the 40 items as independent; they come from 34
documents, so the intervals are a little too narrow. None of the differences between conditions is distinguishable
at n=40: the exact McNemar test (two-sided, on the items where two conditions differ) gives p = 1.0 for C3 against
C1 (one item), p = 0.25 for C4 against C3 (three items, all worse) and p = 0.125 for C5 against C3 (four, all worse).

By gold label, every reader and every condition scored 14/14 on Entailment and 12/13 on Contradiction. The
differences are all on the 13 NotMentioned items: Haiku 11, Sonnet 12, Opus 9 (Wilson 0.42-0.87), C4 9, C5 8.

Three things follow.

- **Quotes and the quote check changed nothing on clean text.** Haiku's answers were identical with and
  without the evidence instruction. One quote of 28 failed the exact-match check. Gold evidence spans were
  hit on 26 of 26 items that had them. The check did what it can do: it confirmed the quotes were on the
  page. None of the three errors was a fabricated citation.
- **The entailment check flagged 5 items, 1 of them wrong** (test-22, where Haiku answered Entailment on a
  NotMentioned pair). The other four flags were on correct answers. Taken as a detector of the three C1
  errors: recall 1/3, precision 1/5. Two of the three errors (test-23, test-29) were never flagged by any
  condition: every reader gave the same label, and it differs from the ContractNLI gold. Whether that is a
  shared reader error or a gold dispute is taken up in 5.7; on this sample they are 2 of 40 items (5%, Wilson
  1.4-16.5%) of unanimous disagreement with the gold.
- **Adjudication by the strongest model lowered accuracy.** C4 asked Opus to re-read the 5 disputed items with
  all three readings in front of it. It overturned three correct NotMentioned majorities (test-03, 14, 21) to
  Entailment and left the other two as they were; test-23 and test-29 were unanimous, so C4 never saw them. C5 did
  the same to a fourth (test-31). Opus as a solo reader also had the lowest accuracy; four of its five errors were
  Entailment on NotMentioned pairs (the fifth is the shared test-29): the strongest model over-reads these
  contracts, finding an obligation the annotators did not. In all three reversals the adjudicator agreed with
  Opus's own reading, which the prompt always shows last, labelled C; 5.7 tests whether that is position or model.
  Whether the over-reading is reader error or gold dispute is discussed in 5.7; for the pipeline the lesson is the
  same, because the rule treats the adjudicator's answer as final.

### 5.3 Seeded edits: the cheap reader misses meaning changes, the checks mostly miss the misses

Each label-changing edit flips the recorded gold. A reader who follows the text gets the new gold; a reader who
answers from expectation gets the old one. Remember from 5.1 that the gold on test-07 and test-39 is probably wrong
and test-24 is contested.

| condition | accuracy | wrong items (edit kind) | flagged |
|---|---|---|---|
| C1 Haiku + quotes | 30/40 = 0.750 (0.60-0.86) | 06, 18, 25, 33 (exception); 07, 34 (negation in); 15, 39 (negation out); 24 (modal); 17 (number) | 0 |
| C2 + entailment check | 30/40 = 0.750 | same | 6 |
| C3 three readers | 38/40 = 0.950 (0.84-0.99) | 07, 24 | 9 |
| C4 + adjudicate flagged | 37/40 = 0.925 (0.80-0.97) | 07, 24, 34 | 0 |

Per reader: Haiku 30/40, Sonnet 38/40, Opus 39/40. The first version of this report said the order of the three
readers "reverses" between clean and seeded text. That claim is withdrawn. The seeded gold has no NotMentioned
items, and NotMentioned is the only label on which the readers differed on clean text (5.2), so Opus's clean
weakness cannot show here, and every NotMentioned answer is scored wrong, which is Haiku's main seeded error.
By label on seeded text: Haiku 21/26 Entailment and 9/14 Contradiction; Sonnet 26/26 and 12/14; Opus 26/26 and
13/14. Every seeded call was a subagent call, so a client effect cannot be separated either.

- **Haiku's ten errors have two shapes.** Four exception changes (a carve-out turned into a condition) were
  read as NotMentioned: the reader did not find the clause it was supposed to re-read. Four inserted or
  deleted negations and the modal change were read as Entailment: the reader answered the pre-edit meaning
  (on test-07 and test-39 the second coder agrees with Haiku, so those two may be gold errors instead).
  Its quotes contained the edited text on 20 of 31 edited items, so it often quoted the change and still
  answered as if it were not there. The 13 inserted sentences were all found by all three readers.
- **The entailment check cannot see six of the ten errors**, because a NotMentioned answer carries no quotes
  and the check has nothing to read. Of the four errors it could see, it flagged two (test-15, test-34). Its
  other four flags were on correct answers, three of them inserted-sentence items where the quote was a new
  sentence the check judged insufficient. Recall 2/10, precision 2/6. This is the structural limit of any
  evidence-based check: it verifies what the reader asserted, and abstention asserts nothing.
- **Three readers recovered almost everything that was idiosyncratic.** Majority voting fixed 8 of Haiku's 10
  errors and broke none (McNemar b=8, c=0, p = 0.008). The two it did not fix are test-24 (modal change; all three
  readers gave the pre-edit label, unanimous, never flagged; the second coder sides with the gold, so this is
  the one seeded item that looks like a shared reader error) and test-07 (negation inserted; Haiku and Sonnet
  gave the pre-edit label, Opus the new one; flagged, and the adjudicator sided with the majority; the second
  coder sides with the majority too, so this is probably a gold error).
- **Adjudication fixed nothing and broke one.** The majority already had 8 of the 9 flagged items right.
  The adjudicator kept those 8 and, on test-34, where two readers were right and Haiku wrong, overturned the
  correct majority (McNemar against C3: b=0, c=1, p = 1.0). Across both text conditions the adjudicator was asked
  to settle 14 disputes, improved 0 and worsened 4 (pooled b=0, c=4, p = 0.125; the two sets share item ids and
  most documents, so the pooled pairs are not independent). That is suggestive, not shown.
- **No claim about memory of the public text.** The first version read test-24 as proof that the readers
  answered from memory of the public dataset. With a model-written gold that a second coder disputes on two
  neighbouring items, the evidence supports only "unanimous disagreement with the recorded gold on one item", and
  that item's gold is itself contested between the second coder and the outside reviewers.

### 5.4 OCR noise: the quote check passes everything, by design

The OCR variant was run for C1 on all 40 items. No entailment check ran on OCR text, so C2 is complete only on
the 12 items where Haiku answered NotMentioned and no check was due; it is not reported.

| | clean | ocr |
|---|---|---|
| C1 accuracy | 37/40 | 36/40 (0.77-0.96) |
| wrong items | 22, 23, 29 | 15, 17, 22, 23 |
| quotes passing the exact check | 27/28 | 28/28 |
| gold evidence spans hit | 26/26 | 26/26 |

Accuracy moved by one item, well inside the interval. Two new errors appeared (test-15 read as Contradiction,
test-17 as NotMentioned) and one old error disappeared (test-29), which at n=40 is noise, and test-17 also flips
between the clean and seeded variants, so some of this is run-to-run variation that a single call per item cannot
separate from the damage. The finding is in the third row: the reader quoted the damaged text exactly as damaged, so
every quote passed. **The quote check verifies that the reader read the page, not that the page is right.** This
is the mechanism behind section 3.2: an input error upstream of every reader is invisible to every check downstream
of the text. At 2% character error the readers mostly recovered the meaning; the pilot did not try heavier damage or
damage aimed at numbers and negations, which is where the S106 deeds would be hurt.

### 5.5 Estimator against measured accuracy

The three readers in C3 are three Anthropic models, so the "families" of the production rule are here one
family. The estimator was run on their labels as if they were three families (`estimate.csv` in each
results folder). The table shows the output of `accuracy.rs` as it stood when the pilot ran (main at 162d8bc).

| run | reader | estimate | lower bound | measured | passes gate |
|---|---|---|---|---|---|
| clean | Haiku | 0.974 | 0.921 | 0.925 | yes |
| clean | Sonnet | 1.001 | 1.000 | 0.950 | yes |
| clean | Opus | 0.924 | 0.825 | 0.875 | no |
| seeded | Haiku | 0.797 | 0.666 | 0.750 | no |
| seeded | Sonnet | 1.003 | 1.000 | 0.950 | yes |
| seeded | Opus | 0.972 | 0.912 | 0.975 | yes |

On clean text every estimate is above the measured value, by 0.05 on average, and Sonnet is estimated at
100% with a lower bound of 100% while it was wrong twice. Part of that is the signature of section 3.2: the two
unanimous items count as agreement, and three same-vendor readers have no independent partner to pull the
estimate down. Part of it is arithmetic: Sonnet was never the lone dissenter among the three, so A_hs·A_so/A_ho
is above 1 whatever Sonnet's real accuracy, and the bootstrap's 2.5th percentile of a quantity pinned at 1 is 1.
The estimate is not identified in that configuration. The gate decisions were right in six rows of six in the
sense that they ordered the readers correctly within each run. Dev (10 items) gave the same: Sonnet and Opus
estimated at exactly 1.00, measured 0.90, with one item (dev-05) on which all three readers disagreed with the
gold with near-identical reasoning.

**The item error column was exactly zero on every item, including the unanimous disagreements.** This was reported
in the first version as "an order of magnitude too small"; it was zero. The chain, found by the outside
reviewers: Sonnet's point estimate is above 1; its lower bound is exactly 1.0; the fit check `lower <= 1.0`
accepts it; `error()` clamps its accuracy to 1, so every rival label gets likelihood 0 and every cell Sonnet read
gets a stated error of 0. The same code runs in `s106 results`, so this is a production defect as well as a report
error. The fix (`fits: lower < 1.0 || accuracy <= 1.0`, with a failing test) is a separate pull request for the
code reviewer, s106-verification #76. Rerun with the fix, Sonnet is a misfit on both runs and fails the gate, and
the item column reports errors: on clean text test-23 and test-29, the two unanimous items, each get a stated error
of 1.8%; the four Opus-only disputes get 33% (test-03, 14, 21) and 73% (test-22). On seeded text test-24 gets 4.6%
and test-07 85%. Under the production rule, a stated 1.8% would keep the two unanimous items out of `confirmed` but
not out of `estimated`, which needs only unanimity and a reader that passes the gate (Haiku does, at 0.921). The
dev run is unchanged by the fix: Sonnet and Opus agree with each other on all 10 items, so both estimates are
exactly 1, which the fix deliberately still accepts (three readers that never disagree would otherwise never pass),
and dev-05 still carries a stated error of 0. That residual case is flagged on the pull request.

### 5.6 Cost

API spend was $1.23: dev $0.52 for 58 calls, clean test $0.71 for 107 calls before the limit. The other 324
calls ran as subagents and have no token record; the ledger shows them at $0.00, which should read as "not
recorded", not as free. Pricing every one of the 489 calls at the mean API cost of the same model and kind gives
about $4.10 for a full rerun (`pilot/README.md`; its $6.56 figure is the dry run's fake-token estimate).

Cost per correct answer, from API records only (`followups.md`, section 5). On dev, where every call was an API
call: C0 $0.00051 per correct answer, C1 1.2×, C2 5.2×, C3 63×, C4 66×, C5 109×. C5 against C0 is 108.7× on raw
cost ($0.4995 against $0.0046 for 10 items); the first version said "about 50 times", which was wrong. On clean
test, over the items whose calls were all API calls in both C0 and the condition: C1 1.1×, C2 4.0×, C3 61× (22
items). C4 and C5 are not estimable from API records because every adjudication was a subagent call. The relative
costs therefore rest on dev, and one Haiku read is about $0.0005 and 1.3 s.

### 5.7 Two controls and a replication, added after review

**Blind second coding (gold disputes).** The same blind Opus coder as in 5.1 also read dev-05, test-23 and
test-29, the three clean items on which all three readers disagreed with ContractNLI. Seeing only the document and
hypothesis, it gave the readers' label on all three (Entailment, Entailment and NotMentioned against a gold of
NotMentioned, NotMentioned and Contradiction). Because the coder is the same vendor as the readers, this does not
decide whether the gold or the readers are wrong; it does mean that no clean-text item in this pilot is an
undisputed instance of a shared reader error. The headline finding of section 7 is therefore shown on synthetic
data and suggested, not shown, on real text.

**Shuffled, anonymised adjudication (position and self-preference).** The 14 C4 adjudications were re-asked
with the three readings shuffled (seeded permutation per item, `control_adjudicate.py`) and labelled A/B/C with no
model names, answered by a fresh Opus subagent. Two permutations came out as the identity (clean test-14, seeded
test-18), so those two are not controls. Results (`control-adjudication.csv`): 13 of 14 labels identical to the
original adjudication; the one change is seeded test-34, where the control sided with the correct majority
(Contradiction) and the original had not. On the four items where Opus's reading differed from the majority (clean
test-03, 14, 21 and seeded test-07), the control sided with Opus's reading on the three clean items, in two of which
that reading had moved to position B, and against it on test-07. So the clean reversals are not a position effect.
They are consistent with either self-preference (an Opus adjudicator agreeing with an Opus reading it cannot see
the name of) or with Opus genuinely reading these three NotMentioned items as Entailment, since it did so twice as a
reader and twice as an adjudicator; the control cannot separate those, and a non-Anthropic adjudicator was not
available. The 3-of-14 worsened under the control against 4-of-14 originally is within run-to-run variation.

**Cross-client replication.** Items test-00..09, originally read through the API, were re-read as subagents
for C1 Haiku, C3 Sonnet and C3 Opus (`replicate.csv`). 29 of 30 labels were the same. The one change is Sonnet
on test-03 (API NotMentioned, subagent Entailment; gold NotMentioned), the same over-reading Opus made on that
item. This is consistent with the two clients being exchangeable for labels, with one caveat: both unanimous
clean items (test-23, test-29) fell in the subagent block, and Opus's API-block accuracy (19/22) was below its
subagent-block accuracy (16/18), so a client effect is not excluded, only not seen in 30 pairs.

## 6. Literature and what is already settled

Full review with 45 references and a verification mark on each: `lit-review.md` beside this report.
The short version, as it bears on the findings:

- **Accuracy from agreement without labels is a solved estimation problem under independence.** Dawid & Skene
  (1979), Platanios, Blum & Mitchell (UAI 2014), Jaffe, Nadler & Kluger (AISTATS 2015), Zhang et al. (JMLR 2016).
  Three conditionally independent, better-than-chance raters identify the model exactly; with exactly three
  there is no slack to test independence from the data, which is why `fits` (A5) cannot detect correlation.
  Platanios keeps the both-wrong agreement term the project's formula drops. Nothing here is new.
- **LLM errors are correlated, and more so between capable models.** Kim et al. (ICML 2025): models agree 60% of
  the time when both are wrong on one leaderboard; Goel et al. (ICML 2025): errors converge with capability;
  Panickssery et al. (NeurIPS 2024): judges favour their own outputs. These papers measure correlation on
  benchmarks; none connects it to the bias of an agreement-based accuracy estimate used as a release gate.
- **Citations are often unsupported.** Liu, Zhang & Liang (2023): 74.5% of citations support their sentence;
  Gao et al. (ALCE, 2023): about half of answers lack full support. The quote check and the entailment check
  are the standard AIS operation; the open question is whether the checker fails together with the reader.
- **Selective prediction and conformal guarantees need labelled calibration data** (Geifman & El-Yaniv 2017;
  Quach et al. 2024). The project's gate replaces labels with an unlabelled estimate; no published result says
  the coverage survives that substitution when independence fails. Section 3 shows it does not.
- **Cascades escalate on confidence or a learned router** (FrugalGPT, Chen et al. 2023). Escalation on
  disagreement is natural but unevaluated against labels, and by construction blind to shared errors.
- **Self-correction without an outside signal is unreliable** (Huang et al., ICLR 2024). An adjudicator from the
  same vendor reading the same text is close to self-correction.

Not claimed as novel: OCR, retrieval, quotes, majority voting, abstention. The plausible contribution is the
audit itself: a measured gap between stated and realised error for an agreement-based gate under the error
structures real pipelines have (shared inputs, anchored labels, same-vendor readers), on clean and on
corrupted text, with the cost of each verification stage beside its effect.

## 7. What this study establishes, and what it does not

Established, with the evidence that carries each claim:

1. **The lower bound of the production estimator under-covers even when its assumptions hold.** Synthetic S1:
   the 97.5% bound sits above the truth in 10% of runs at 20 schemes and 17% at 5 schemes. This is a property
   of the percentile bootstrap on few clusters, not of the data.
2. **Errors shared by every reader inflate the estimate and pass through `confirmed` undetected.** Shown on
   synthetic data: S4 and S11 give the size (stated error under 1%, realised 4.5-10.7% at 5-10% shared corruption
   over `confirmed` cells). Suggested, not shown, on real text: the pilot has two unanimous disagreements with the
   gold per 40 clean items and one under a meaning-changing edit, none flagged by any check, and the estimator rated
   the reader that carried them at 100%; but a blind second coder of the same vendor sides with the readers on all
   three clean items, so they may be gold disputes rather than reader errors.
3. **The estimator can report an error of exactly zero on a cell all readers got wrong.** A reader that is never
   the lone dissenter gets an estimate above 1 and a lower bound of exactly 1, passes the fit check, and has its
   accuracy clamped to 1. Shown by the chain in 5.5 and by a failing unit test; fixed in s106-verification #76.
   A reader whose estimate is exactly 1 (two readers that never disagree with each other) still passes after the
   fix, by a rule decision left to the code reviewer.
4. **The quote check and the entailment check verify assertions, not abstentions, and not the page.** Six of
   ten seeded errors were NotMentioned answers with nothing to check; OCR-damaged quotes passed 28 of 28. On
   clean text the entailment check caught 1 of 3 errors and raised 4 false flags.
5. **Adjudication by a stronger model did not help here and may hurt.** Across clean and seeded text the Opus
   adjudicator improved 0 of 14 disputed items and worsened 4 (exact McNemar p = 0.125, so suggestive), every time
   by turning a correct NotMentioned or Contradiction into Entailment. Shuffling and anonymising the readings
   changed one of the 14 answers, so this is not a position effect; whether it is self-preference or Opus's own
   reading of these items the pilot cannot say. Adjudicate-everything (C5) was the worst condition on clean text.
6. **Three readers of the same vendor recover idiosyncratic errors well** (8 of 10 cheap-reader errors on
   seeded text, p = 0.008, with none broken) and shared ones not at all, which is what the theory predicts and
   what the production rule's family requirement is for.

Not established:

- **Any error rate for the S106 corpus.** None exists. Every one of the 312 read cells is `proposed`, no reader
  shares five cells with two readers from other families, and nothing in this study measures a deed. ContractNLI is
  English NDAs, clean or lightly damaged, with a three-way label; the deeds are OCR'd planning agreements with
  numeric answers. The structure of the failures transfers; the numbers do not.
- That the unanimous disagreements are reader errors. test-23, test-29 and dev-05 are disputed by a blind
  second coder; test-07 and test-39 on the seeded set are probably label errors; test-24 is contested. The study has
  no opinion on the gold from outside the vendor, which is itself the point: without an independent source the
  question is undecidable from inside.
- That the readers answered from memory of the public dataset. The first version claimed this from test-24; the
  claim is withdrawn (5.3).
- That OCR noise at realistic levels hurts readers. 2% character error moved accuracy by one item.
- The cost curve for the variants, since subagent calls carry no token counts.
- Anything about non-Anthropic readers. All readers, checkers, adjudicators and the second coder were one vendor.
- Any difference between conditions on accuracy. At n=40 none is distinguishable (5.2).

The one negative finding the study would defend in front of a reviewer: **a verification chain built from
agreement, quotes, an entailment check and adjudication, all reading the same text, cannot distinguish a
unanimous right answer from a unanimous wrong one, and the production estimator as deployed reported the second
with a stated error of exactly zero.** The first clause is shown by construction (section 2) and by simulation
(section 3), and is consistent with, but not proven by, the real-text pilot (sections 5 and 7). The second is
shown by the code and a failing test (5.5, #76).

## 8. Failures and limitations of the study itself

- **Sample size.** 40 items give a 95% interval about ±13 points wide on accuracy near 0.9. The conditions
  are not distinguishable from each other on accuracy; the findings rest on which items failed and why, not
  on the aggregate rates. Dev had 10 items and was used only for prompt development. The 40 items come from 34
  documents, so intervals that treat them as independent are slightly narrow. Each call ran once.
- **Mixed clients.** The clean test run is part API (items 00-21), part subagent (22-39); every variant,
  adjudication and control call was a subagent. The bridge sends the identical prompt and schema, but a subagent
  runs inside a Claude Code session with its own system prompt, its effort setting is not the API's, and its model
  name is self-reported. A 30-pair replication (5.7) found 29 labels the same; that is reassurance, not proof of
  exchangeability. `items.csv` and `followups.md` carry the client per call.
- **Gold labels.** ContractNLI's gold is a 2021 human annotation that the readers unanimously disputed on 2 of 40
  clean items, and a blind coder of the same vendor sided with the readers. The seeded gold was written by a model
  and checked only by another model, which rejected 2 of 31 labels. No person has labelled anything in this study.
  A human relabelling of the 34 disputed items is the obvious next step and was outside the brief's constraint of
  no human in the loop.
- **Contamination.** ContractNLI is public since 2021. Clean-text accuracy is an upper bound on what the
  readers can do on unseen contracts. The seeded variant was designed to detect answering from memory; after the
  second coding it found at most one such item (test-24) in 28 label-changing edits, and that item is contested.
- **Seeded edits are model-made and unblinded.** 9 of 40 items could not be edited by the rule and are unchanged,
  so the seeded accuracy mixes edited and unedited items, and the seeded label mix (26 Entailment, 14
  Contradiction, 0 NotMentioned) differs from the clean mix, which is why reader comparisons across the two
  variants are not made. The 13 inserted sentences were found by every reader, which suggests they were easy.
- **OCR simulation is crude.** Uniform character noise at 2% does not model real OCR, which damages tables,
  numbers and layout more than running prose. The OCR variant ran C1 only; C2 is complete on the 12 items that needed no entailment call.
- **C5 incomplete on seeded; C0 not run on either variant.** Reported as such; no number in this report
  depends on them.
- **No human reader, no external source.** The production rule's other two families were not in the pilot.
  The pilot therefore tests the LLM family alone and cannot say how much an external reader would have
  caught; section 3.2 says it would have caught every corrupted cell it independently observed.
- **The adjudicator prompt was tuned on dev with the other prompts and frozen.** Its over-reading may be a
  prompt artefact rather than a model property; the study did not try a second prompt after seeing test
  results, by design. The shuffled control (5.7) was added after review and is not pre-registered.
- **Cost figures are partial.** Only 165 of 489 calls have a recorded price; the ledger shows the others at
  $0.00. Relative costs come from the dev run.
- **The analysis code was written and reviewed by models.** The synthetic crate, the pilot scripts, the
  follow-up scripts and the estimator fix were each written by one model and reviewed by another, with defects
  found and fixed at each review (two in the synthetic crate, five in the follow-up scripts, one rule change in the
  fix). The owner has not reviewed any of it line by line. Three outside model reviewers with no project context
  found the five misstatements that this version corrects.

## 9. A four-month agenda (fellowship proposal)

Question: can an automated research pipeline bound its own error rate without labels, and allocate
verification effort to where errors are?

- **Month 1. Estimator that admits correlation.** Replace the three-reader moment formula with a latent-class
  model that includes a shared-input error component and the both-wrong agreement term, fitted by EM or
  Bayesian sampling; recover ρ from a fourth reader that does *not* share the input (a vision model on the page
  image, or a second OCR engine). Validate on the synthetic generator from this study and on ContractNLI with
  held-out labels; report coverage of the lower bound with a second interval method beside the percentile
  bootstrap, not just bias. Deliverable: an estimator whose interval covers at the stated rate in the S4/S11/S12
  scenarios, or a proof that it cannot with the readers available.
- **Month 2. Where do errors live, and which way do they lean?** On ContractNLI (clean, OCR-noised, seeded) and
  50-100 S106 cells with reference answers from primary instruments (deeds of variation already in hand), measure
  per stage the share of errors that are idiosyncratic (caught by disagreement), shared across same-vendor
  readers, shared across vendors, and shared with the input. Add a numeric task (unit counts and tenure splits
  from OCR'd deeds, where errors are magnitudes rather than labels) and an aggregate-bias measure: the signed
  error of a borough total built from the pipeline's cells against the same total from reference answers. A
  policy total needs the sign and size of the bias, not a per-cell error rate. This is the number that decides
  whether more readers or a different kind of reader buys reliability.
- **Month 3. Adaptive verification.** Treat the stages as actions with costs; the trigger is the posterior
  error from month 1, not raw disagreement. Compare budgets (always-everything, disagreement-only, posterior-
  threshold) on accuracy-versus-cost curves, and measure how often each budget confirms a wrong answer.
  Pre-register the comparison before running it, with a non-Anthropic adjudicator if one is available.
- **Month 4. Real-world transfer and write-up.** Run the chosen design over the London S106 corpus at scale
  with an independent image reader and the external oracles (Datahub, EPC, Price Paid) at their proper lineage
  stage; publish the error budget per variable and the cases where the pipeline abstains. Write up the negative
  results from months 1-3 as carefully as the positive ones.

Risks: no non-Anthropic model family may be available, which caps how independent "another family" can be; the
S106 reference answers come from documents, not people, and carry their own error; ContractNLI is likely in
pretraining data, so clean-text accuracy is an upper bound; and every label in this study so far was produced by a
model, so the first human relabelling may move the numbers.

## Appendix. Reproducing the smallest experiment

Everything needed is in this folder of the public repository (no S106 data is involved).

    cd experiments/verification-study
    (cd synthetic && cargo run --release)            # section 3, writes synthetic/results/
    cd pilot
    python3 analyse.py --split test                  # section 5.2 from the saved records in runs/test/
    python3 analyse.py --split test --dataset seeded # section 5.3
    python3 analyse.py --split test --dataset ocr    # section 5.4
    python3 followups.py --split test --pooled       # 5.2, 5.3, 5.6 follow-up tables (also --dataset seeded, --split dev)
    python3 control_adjudicate.py ingest --split test            # 5.7, from the saved control answers
    python3 control_adjudicate.py ingest --split test --dataset seeded
    python3 replicate_compare.py                     # 5.7 replication
    (cd estimate && cargo build --release)
    estimate/target/release/estimate results/test/votes.csv results/test/readers.csv

To repeat the calls rather than reanalyse them, move `runs/` aside and follow `pilot/README.md`; the
prompts are in `pilot/prompts/`, the data and selection script in `pilot/data/contractnli/`, the
damage scripts are `pilot/corrupt.py` and `pilot/verify_corrupt.py`, and every record under `runs/`
stores the request it answered. A full rerun of the 489 calls through the API costs about $4.10 at list prices,
priced at the mean recorded cost per call of the same model and kind (5.6). The 34 blind second-coder prompts and
answers are in `pilot/second-coder/`, the 14 shuffled adjudication prompts and answers in `pilot/control/`, and the
30 replication records in `pilot/replicate/runs/`.
