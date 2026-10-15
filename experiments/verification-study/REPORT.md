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

## 2. What the estimator assumes (audit of `src/accuracy.rs`, s106-verification `main` at faf1d9c)

The code is 437 lines. The estimate is `a_i = sqrt(A_ij · A_ik / A_jk)` where `A_xy` is the share of
shared cells on which readers x and y agree (`triplet()`, accuracy.rs:276-281). The lower bound is the 2.5th
percentile of that quantity over 1000 resamples of schemes (`estimates()`, accuracy.rs:300-358). A cell's
posterior error (`error()`, accuracy.rs:364-397) uses each reader's *lower bound* as its accuracy, a uniform
prior over the distinct observed readings plus "none of them", and treats agreeing wrong readers as giving
the same wrong value. `results.rs:216-262` then marks a cell `estimated` only when every reader agrees, the
shown reader's group passes the gate and at least two readers voted, and `confirmed` when the posterior error
is at most 1% and one agreeing reader is not an LLM.

The assumptions, each tested in section 3:

| | assumption | where it is written | what breaks it |
|---|---|---|---|
| A1 | two readers agree only when both are right: P(agree) = a_i·a_j | module doc, accuracy.rs:17-19 | small answer spaces (status is 2-valued; V4 route has 3-4 values) let wrong readers coincide. Platanios, Blum & Mitchell (2014) keep the both-wrong term; this code drops it |
| A2 | errors are independent across the three families | accuracy.rs:19-22 | every LLM and every person reads the same OCR text; a human label made after seeing the model's reading; an external source describing another lineage stage |
| A3 | one accuracy per reader per variable group; no item difficulty | accuracy.rs:14-16 | hard pages make every reader wrong at once (GLAD, Whitehill et al. 2009) |
| A4 | the scheme-bootstrap 2.5th percentile is a 97.5% lower bound, with as few as 5 shared cells (`MIN_SHARED`) | accuracy.rs:38-40, 338-347 | percentile bootstraps on 5-20 clusters are known to under-cover |
| A5 | misfit shows as a lower bound above 1 (`fits`) | accuracy.rs:235-237 | correlated errors move the estimate up without pushing it past 1 |
| A6 | `error()` using lower bounds and a uniform prior is conservative | accuracy.rs:361-363 | the same correlated errors that bias the estimate also bias the posterior |
| A7 | the triplet is the one whose least-shared pair shares the most cells | accuracy.rs:322-331 | selection on the data; not tested here |

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

Everything in this section comes from `pilot/results/<run>/{metrics,items,readers,estimate}.csv`, written by
`pilot/analyse.py` and the Rust crate `pilot/estimate/` from the saved call records under `pilot/runs/`. The
design was fixed before the test run (`pilot-design.md`); prompts were tuned on 10 dev pairs only and are
frozen at commit 868f16c.

### 5.1 Setup

Data: 40 (document, hypothesis) pairs from the ContractNLI test split, stratified by label (14 Entailment,
13 Contradiction, 13 NotMentioned), seed 20211005. NotMentioned is both a label and the abstention answer.
Three variants of the same 40 pairs:

- **clean**: the published text;
- **ocr**: the text with simulated OCR damage at about 2% character error rate (`pilot/corrupt.py`,
  character substitutions, dropped characters, joined and split words), gold unchanged;
- **seeded**: 31 of the 40 documents given one meaning-changing edit (13 inserted sentences that make a
  NotMentioned hypothesis entailed, 5 exception changes, 4 inserted negations, 2 deleted negations, 3 scope
  restrictions, 1 modal change, 3 number changes), each with a new gold label worked out by hand and recorded
  with the edit in `pilot/data/contractnli/seeded_edits.txt`; 9 documents left unchanged where no single
  edit would flip the label. The three number changes keep the label (Entailment stays Entailment) and test
  only whether the quoted number follows the edit.

Conditions, each built from primitive calls that are saved once and reused:

| | condition | calls |
|---|---|---|
| C0 | one cheap reader, no evidence | Haiku 5.5 |
| C1 | one cheap reader with verbatim quotes, deterministic quote-on-page check | Haiku 5.5 |
| C2 | C1 plus an entailment check that sees only hypothesis, label and quotes | + Sonnet 5.5 |
| C3 | three readers (Haiku, Sonnet, Opus 5.5) with quotes; majority label; estimator run on their labels | Haiku + Sonnet + Opus |
| C4 | C3 plus Opus adjudication of items where readers disagree or a quote fails | + Opus on flagged items |
| C5 | C3 plus Opus adjudication of every item | + Opus on all items |

How the calls were made matters for what the numbers can say. Dev and the first part of the clean test run
went through the API from the owner's machine ($1.23 in total, `pilot/runs/*/ledger.csv`). The API then hit
the workspace spending limit, and the owner asked for the rest to run as Claude Code subagents. The bridge
(`pilot/bridge.py`) exports each pending call as the exact prompt and schema the API client would have sent;
one subagent of the matching model answers it with no other tool or context; the answer is ingested as a
record marked `client: subagent`. Those records have no token counts or latency, so **token, latency and
dollar columns cover API records only** and the variant runs (all subagent) report none. Everything else
(labels, quotes, confidence, flags) is comparable across clients. The clean C0 condition was not rerun on
the variants; C5 on the seeded set has only the 9 adjudications that C4 triggered, so it is reported as
incomplete.

### 5.2 Clean text: readers are good, the adjudicator makes things worse

| condition | accuracy (Wilson 95%) | wrong items | flagged |
|---|---|---|---|
| C0 Haiku, no quotes | 37/40 = 0.925 (0.80-0.97) | 22, 23, 29 | 0 |
| C1 Haiku + quotes + quote check | 37/40 = 0.925 | 22, 23, 29 | 1 (quote fail) |
| C2 C1 + Sonnet entailment check | 37/40 = 0.925 | 22, 23, 29 | 6 |
| C3 three readers, majority | 38/40 = 0.950 (0.84-0.99) | 23, 29 | 5 (disagreement) |
| C4 C3 + adjudicate flagged | 35/40 = 0.875 (0.74-0.95) | 03, 14, 21, 23, 29 | 0 |
| C5 C3 + adjudicate all | 34/40 = 0.850 (0.71-0.93) | 03, 14, 21, 23, 29, 31 | 0 |

Per reader: Haiku 37/40, Sonnet 38/40, Opus 35/40. Dev (10 pairs) had the same shape: C0-C5 at 0.8-0.9,
no condition better than the cheapest.

Three things follow.

- **Quotes and the quote check changed nothing on clean text.** Haiku's answers were identical with and
  without the evidence instruction. One quote of 28 failed the exact-match check. Gold evidence spans were
  hit on 26 of 26 items that had them. The check did what it can do: it confirmed the quotes were on the
  page. None of the three errors was a fabricated citation.
- **The entailment check flagged 6 items, 1 of them wrong** (test-22, where Haiku answered Entailment on a
  NotMentioned pair). The other five flags were on correct answers. Taken as a detector of the three C1
  errors: recall 1/3, precision 1/6. Two of the three errors (test-23, test-29) were never flagged by any
  condition: every reader and the adjudicator gave the same wrong label. These are the shared errors of
  section 3; on this sample they are 2 of 40 items (5%, Wilson 1.4-16.5%).
- **Adjudication by the strongest model lowered accuracy.** C4 asked Opus to re-read the 5 disputed items with
  all three readings in front of it. It kept the two shared errors and overturned three correct NotMentioned
  majorities (test-03, 14, 21) to Entailment. C5 did the same to a fourth (test-31). Opus as a solo reader
  also had the lowest accuracy; four of its five errors were Entailment on NotMentioned pairs (the fifth is
  the shared test-29): the strongest model over-reads these contracts, finding an obligation the annotators
  did not. Whether that is
  reader error or gold dispute is discussed in 5.5; for the pipeline the lesson is the same, because the
  rule treats the adjudicator's answer as final.

### 5.3 Seeded edits: the cheap reader misses meaning changes, the checks mostly miss the misses

Each edit flips the correct label. A reader who follows the text gets the new gold; a reader who answers from
expectation (or memory of the public dataset) gets the old one.

| condition | accuracy | wrong items (edit kind) | flagged |
|---|---|---|---|
| C1 Haiku + quotes | 30/40 = 0.750 (0.60-0.86) | 06, 18, 25, 33 (exception); 07, 34 (negation in); 15, 39 (negation out); 24 (modal); 17 (number) | 0 |
| C2 + entailment check | 30/40 = 0.750 | same | 6 |
| C3 three readers | 38/40 = 0.950 (0.84-0.99) | 07, 24 | 9 |
| C4 + adjudicate flagged | 37/40 = 0.925 (0.80-0.97) | 07, 24, 34 | 0 |

Per reader: Haiku 30/40, Sonnet 38/40, Opus 39/40. The order of the three readers reverses between clean
and seeded text: Opus was worst on clean and best on seeded, Haiku the reverse.

- **Haiku's ten errors have two shapes.** Four exception changes (a carve-out turned into a condition) were
  read as NotMentioned: the reader did not find the clause it was supposed to re-read. Four inserted or
  deleted negations and the modal change were read as Entailment: the reader answered the pre-edit meaning.
  Its quotes contained the edited text on 20 of 31 edited items, so it often quoted the change and still
  answered as if it were not there. The 13 inserted sentences were all found by all three readers.
- **The entailment check cannot see six of the ten errors**, because a NotMentioned answer carries no quotes
  and the check has nothing to read. Of the four errors it could see, it flagged two (test-15, test-34). Its
  other four flags were on correct answers, three of them inserted-sentence items where the quote was a new
  sentence the check judged insufficient. Recall 2/10, precision 2/6. This is the structural limit of any
  evidence-based check: it verifies what the reader asserted, and abstention asserts nothing.
- **Three readers recovered almost everything that was idiosyncratic.** Majority voting fixed 8 of Haiku's 10
  errors. The two it did not fix are the two shared errors: test-24 (modal change; all three readers gave
  the pre-edit label, unanimous, never flagged) and test-07 (negation inserted; Haiku and Sonnet gave the
  pre-edit label, Opus the right one; flagged, and the adjudicator sided with the majority).
- **Adjudication fixed 7 of the 9 flagged items and broke one.** On test-34 two readers were right and Haiku
  wrong; Opus as adjudicator overturned the correct majority. Net, C4 is one item worse than C3. Across both
  text conditions the adjudicator was asked to settle 14 disputes and worsened 4 of them.
- **Memory of the public text is visible in one item.** On test-24 all three readers answered the original
  gold against the edited text they were given. ContractNLI is in pretraining data, so this is the expected
  failure mode of a contaminated benchmark, and it is the one error that no agreement-based check can find.
  One item is not an estimate of the rate; it is an existence proof.

### 5.4 OCR noise: the quote check passes everything, by design

The OCR variant was run for C1 only (40 Haiku reads).

| | clean | ocr |
|---|---|---|
| C1 accuracy | 37/40 | 36/40 (0.77-0.96) |
| wrong items | 22, 23, 29 | 15, 17, 22, 23 |
| quotes passing the exact check | 27/28 | 28/28 |
| gold evidence spans hit | 26/26 | 26/26 |

Accuracy moved by one item, well inside the interval. Two new errors appeared (test-15 read as Contradiction,
test-17 as NotMentioned) and one old error disappeared (test-29), which at n=40 is noise. The finding is in
the third row: the reader quoted the damaged text exactly as damaged, so every quote passed. **The quote
check verifies that the reader read the page, not that the page is right.** This is the mechanism behind
section 3.2: an input error upstream of every reader is invisible to every check downstream of the text.
At 2% character error the readers mostly recovered the meaning; the pilot did not try heavier damage or
damage aimed at numbers and negations, which is where the S106 deeds would be hurt.

### 5.5 Estimator against measured accuracy

The three readers in C3 are three Anthropic models, so the "families" of the production rule are here one
family. The estimator was run on their labels as if they were three families (`estimate.csv` in each
results folder).

| run | reader | estimate | lower bound | measured | passes gate |
|---|---|---|---|---|---|
| clean | Haiku | 0.974 | 0.921 | 0.925 | yes |
| clean | Sonnet | 1.001 | 1.000 | 0.950 | yes |
| clean | Opus | 0.924 | 0.825 | 0.875 | no |
| seeded | Haiku | 0.797 | 0.666 | 0.750 | no |
| seeded | Sonnet | 1.003 | 1.000 | 0.950 | yes |
| seeded | Opus | 0.972 | 0.912 | 0.975 | yes |

On clean text every estimate is above the measured value, by 0.05 on average, and Sonnet is estimated at
100% with a lower bound of 100% while it was wrong twice. That is the signature of section 3.2: the two
shared errors count as agreement, and three same-vendor readers have no independent partner to pull the
estimate down. The `fits` check passed in every row. On seeded text the picture is mixed: Haiku and Opus are
estimated within 0.05, Sonnet again at 100% against 0.95. The gate decisions were right in four rows of six
in the sense that they ordered readers correctly, but the Sonnet rows show the lower bound saying something
the labels contradict. Dev (10 items) gave the same: Sonnet and Opus estimated at 1.00, measured 0.90, with
one shared error (dev-05) that all three readers made with near-identical reasoning.

### 5.6 Cost

API spend was $1.23: dev $0.52 for 58 calls, clean test $0.71 for 107 calls before the limit. From the
API records, one Haiku read cost about $0.0006 and 1.3 s; three readers with an Opus adjudication cost
about 50 times a single Haiku read (dev: C5 $0.50 against C0 $0.005 for 10 items). The subagent calls
(121 clean, 40 ocr, 163 seeded) were paid for by the Claude Code plan, not the API, and have no token
record. The dry-run price table in `pilot/README.md` estimates what they would have cost. Relative cost
between conditions therefore comes from the dev run, where every call was through the API.

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
2. **Errors shared by every reader inflate the estimate and pass through `confirmed` undetected.** Synthetic
   S4 and S11 give the size (stated error 1%, realised 4.5-10.7% at 5-10% shared corruption); the pilot gives
   two real instances per 40 items on clean text and one more under a meaning-changing edit, all unanimous
   and unflagged, and shows the estimator rating a reader at 100% while it carried them.
3. **The quote check and the entailment check verify assertions, not abstentions, and not the page.** Six of
   ten seeded errors were NotMentioned answers with nothing to check; OCR-damaged quotes passed 28 of 28. On
   clean text the entailment check caught 1 of 3 errors and raised 5 false flags.
4. **Adjudication by a stronger model is not a safe default.** Across clean and seeded text the Opus
   adjudicator worsened 4 of 14 disputed items, every time by turning a correct NotMentioned or Contradiction
   into Entailment. Adjudicate-everything (C5) was the worst condition on clean text.
5. **Three readers of the same vendor recover idiosyncratic errors well** (8 of 10 cheap-reader errors on
   seeded text) and shared ones not at all, which is what the theory predicts and what the production rule's
   family requirement is for.

Not established:

- Any rate for the S106 corpus. ContractNLI is English NDAs, clean or lightly damaged, with a three-way label;
  the deeds are OCR'd planning agreements with numeric answers. The structure of the failures transfers; the
  numbers do not.
- That the shared errors are reader errors. test-23, test-29 and dev-05 may be gold disputes; the clean
  adjudicator's three "mistakes" are on items whose seeded edit later made the entailment explicit, so the
  adjudicator may have read a weaker version of the same inference. The study has no third opinion on the
  gold, which is itself the point: without an independent source the question is undecidable from inside.
- That OCR noise at realistic levels hurts readers. 2% character error moved accuracy by one item.
- The cost curve for the variants, since subagent calls carry no token counts.
- Anything about non-Anthropic readers. All readers, checkers and adjudicators were one vendor.

The one negative finding the study would defend in front of a reviewer: **a verification chain built from
agreement, quotes, an entailment check and adjudication, all reading the same text, cannot distinguish a
unanimous right answer from a unanimous wrong one, and the production estimator reports the second as
confirmed with a stated error that is too small by an order of magnitude.** Every part of that sentence is
shown by construction (section 2), by simulation (section 3) and by at least one instance on real text
(section 5).

## 8. Failures and limitations of the study itself

- **Sample size.** 40 items give a 95% interval about ±13 points wide on accuracy near 0.9. The conditions
  are not distinguishable from each other on accuracy; the findings rest on which items failed and why, not
  on the aggregate rates. Dev had 10 items and was used only for prompt development.
- **Mixed clients.** The clean test run is part API, part subagent (see 5.1). The bridge sends the identical
  prompt and schema, but a subagent runs inside a Claude Code session with its own system prompt, and its
  effort setting is not the API's. The bridge was validated only mechanically (dry-run records removed, fed back and rebuilt
  identically); no item was answered by both clients, so the two populations are not shown to be
  exchangeable. `items.csv` carries the client per call for anyone who wants to split them.
- **Contamination.** ContractNLI is public since 2021. Clean-text accuracy is an upper bound on what the
  readers can do on unseen contracts, and test-24 shows at least one answer came from memory. The seeded
  variant was designed to detect exactly this and found it once in 29 label-changing edits.
- **Seeded edits are hand-made and unblinded.** One author wrote the edits and the new gold labels; 9 of 40
  items could not be edited by the rule and are unchanged, so the seeded accuracy mixes edited and unedited
  items. The 13 inserted sentences were found by every reader, which suggests they were easy.
- **OCR simulation is crude.** Uniform character noise at 2% does not model real OCR, which damages tables,
  numbers and layout more than running prose. The OCR variant ran one condition.
- **C5 incomplete on seeded; C0 not run on either variant.** Reported as such; no number in this report
  depends on them.
- **No human reader, no external source.** The production rule's other two families were not in the pilot.
  The pilot therefore tests the LLM family alone and cannot say how much an external reader would have
  caught; section 3.2 says it would have caught every corrupted cell it independently observed.
- **The adjudicator prompt was tuned on dev with the other prompts and frozen.** Its over-reading may be a
  prompt artefact rather than a model property; the study did not try a second prompt after seeing test
  results, by design.
- **The analysis code was written and reviewed by models**, with the synthetic crate reviewed by a second
  model and one bug fixed before results were taken. The owner has not reviewed it line by line.

## 9. A four-month agenda (fellowship proposal)

Question: can an automated research pipeline bound its own error rate without labels, and allocate
verification effort to where errors are?

- **Month 1. Estimator that admits correlation.** Replace the three-reader moment formula with a latent-class
  model that includes a shared-input error component and the both-wrong agreement term, fitted by EM or
  Bayesian sampling; recover ρ from a fourth reader that does *not* share the input (a vision model on the page
  image, or a second OCR engine). Validate on the synthetic generator from this study and on ContractNLI with
  held-out labels; report coverage of the lower bound, not just bias. Deliverable: an estimator whose interval
  covers at the stated rate in the S4/S11/S12 scenarios, or a proof that it cannot with the readers available.
- **Month 2. Where do errors live?** On ContractNLI (clean, OCR-noised, seeded) and 50-100 S106 cells with
  reference answers from primary instruments (deeds of variation already in hand), measure per stage: the share
  of errors that are idiosyncratic (caught by disagreement), shared across same-vendor readers, shared across
  vendors, and shared with the input. This is the number that decides whether more readers or a different kind
  of reader buys reliability.
- **Month 3. Adaptive verification.** Treat the stages as actions with costs; the trigger is the posterior
  error from month 1, not raw disagreement. Compare budgets (always-everything, disagreement-only, posterior-
  threshold) on accuracy-versus-cost curves, and measure how often each budget confirms a wrong answer.
  Pre-register the comparison before running it.
- **Month 4. Real-world transfer and write-up.** Run the chosen design over the London S106 corpus at scale
  with an independent image reader and the external oracles (Datahub, EPC, Price Paid) at their proper lineage
  stage; publish the error budget per variable and the cases where the pipeline abstains. Write up the negative
  results from months 1-3 as carefully as the positive ones.

Risks: no non-Anthropic model family may be available, which caps how independent "another family" can be; the
S106 reference answers come from documents, not people, and carry their own error; ContractNLI is likely in
pretraining data, so clean-text accuracy is an upper bound.

## Appendix. Reproducing the smallest experiment

Everything needed is in this folder of the public repository (no S106 data is involved).

    cd experiments/verification-study
    (cd synthetic && cargo run --release)            # section 3, writes synthetic/results/
    cd pilot
    python3 analyse.py --split test                  # section 5.2 from the saved records in runs/test/
    python3 analyse.py --split test --dataset seeded # section 5.3
    python3 analyse.py --split test --dataset ocr    # section 5.4
    (cd estimate && cargo build --release)
    estimate/target/release/estimate results/test/votes.csv results/test/readers.csv

To repeat the calls rather than reanalyse them, move `runs/` aside and follow `pilot/README.md`; the
prompts are in `pilot/prompts/`, the data and selection script in `pilot/data/contractnli/`, the
damage scripts are `pilot/corrupt.py` and `pilot/verify_corrupt.py`, and every record under `runs/`
stores the request it answered. A full rerun through the API costs about $3 at list prices.
