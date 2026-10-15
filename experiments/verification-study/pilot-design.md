# Smallest real pilot — design (awaiting Ayo's go; no paid call made)

## Question the pilot answers
On items with published reference answers, does the project's verification chain (independent readers ->
quote check -> entailment check -> adjudication -> agreement-based accuracy estimate) catch the cases where
a single reader is wrong, and how often does it report high confidence in a wrong answer? Secondary: on
the same items, does the agreement-based estimator's accuracy figure match the measured accuracy?

## Data
ContractNLI test split, 40 (document, hypothesis) pairs, fixed seed, stratified by label
(`pilot/data/contractnli/sample40.jsonl`; selection rule in the README beside it). Prompts are
developed only on the 10 dev-split pairs (dev10.jsonl). The test 40 are run once per condition.
Label space: Entailment / Contradiction / NotMentioned. NotMentioned doubles as the abstention label, so
"insufficient evidence" is scored, not just allowed.

## Conditions (same 40 items in every condition)
C0 single reader, label + 0-100 confidence, no evidence. Reader: Haiku 5.5.
C1 C0 + required verbatim quotes. Deterministic check: each quote must appear in the document text
   (exact after whitespace normalisation; record near-misses with edit distance <= 3). A reading with a
   failed quote is a failed reading.
C2 C1 + entailment check: a second call (Sonnet 5.5) sees only the hypothesis, the label and the quoted
   spans, never the document, and says whether the quotes support the label. Disagreement = flag.
C3 three independent readers (Haiku 5.5, Sonnet 5.5, Opus 5.5), each as in C1. Majority label; the
   estimator from accuracy.rs is run on the three readers' labels treating each model as a family.
C4 C3 + targeted follow-up: only where readers disagree or a quote check fails, an adjudicator (Opus 5.5,
   fresh context) reads the document, both readings and both quote sets, and must quote.
C5 "always everything": every item gets the adjudicator regardless of disagreement (cost ceiling for C4).

External corroboration (an independent dataset) does not exist for NDAs; that stage is studied on the S106
case material instead (part 3B of the report), qualitatively.

## Measures per condition
accuracy vs gold label; false confirmations (confidence >= 90 and wrong; and for C3-C5, unanimous and
wrong); abstention rate (NotMentioned predicted when gold is not NotMentioned, and vice versa); expected
calibration error over 5 bins; shared errors (items every reader got wrong, and whether their wrong labels
coincide); quote-check failure rate and whether gold evidence spans overlap the quotes (span-level
precision/recall against the published evidence spans); latency; input/output tokens; cost from the
price table. For C3: the estimator's accuracy point estimate and lower bound for each reader vs that
reader's measured accuracy on the 40 items.

## Cost (list prices 14 Oct 2026; Haiku 5.5 $0.10/$0.50, Sonnet 5.5 $2/$10, Opus 5.5 $4/$20 per M tokens)
Per item: ~3.5k input tokens (median NDA 1.4k words + prompt), ~300 output.
- Haiku pass over 40: $0.02. Sonnet pass: $0.40. Opus pass: $0.80.
- Entailment check (Sonnet, ~800 in / 150 out): $0.12.
- Adjudication C4 (Opus, ~4.5k in / 400 out, on at most ~15 flagged items): $0.40. C5 on 40: $1.05.
- Dev-set prompt work (10 items, Haiku + Sonnet): ~$0.15.
Total if everything runs: about $3.00. Batch API halves it (about $1.50) at the cost of latency figures.
Proposed cap: $3.00. The project API budget has about $0.50 left of $5, so this needs either a raised cap
or the readers run as Claude Code subagents (no API charge; token counts still recorded per call, but
prompts are less controllable and latency is not comparable).

## What would count as a negative finding
- Unanimous agreement among three Claude models on a wrong label on >= 3 of 40 items while the estimator
  reports >= 90% accuracy: the family-independence assumption fails for same-vendor readers.
- Quote-check pass rate >= 95% but entailment-check disagreement on >= 20% of passed items: verbatim
  quotes do not guarantee support.
- C4 no better than C3 on false confirmations: targeted follow-up adds cost without catching the shared
  errors.
Each of these would be reported with a Wilson interval on 40 items; 40 items detect only large effects,
and the report says so.
