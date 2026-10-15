# Benchmark survey: verifying AI-extracted claims against long documents

Date: 2026-10-14. Pick: ContractNLI. Download worked; samples are written.

## 1. ContractNLI (Koreeda & Manning, Findings of EMNLP 2021)

- Source: https://stanfordnlp.github.io/contract-nli/resources/contract-nli.zip (65 MB). Downloaded and unzipped through the proxy. Code: https://github.com/stanfordnlp/contract-nli-bert.
- Licence: CC BY 4.0 (LICENSE file in the zip) plus a TERMS file from Hitachi America that defers to CC BY 4.0 and disclaims third-party links. The contracts themselves are public web and EDGAR documents; the paper argues fair use and gives a takedown contact form.
- Size: 607 NDAs, 17 fixed hypotheses (same in every document). Splits I counted from the files:

| split | docs | pairs | Entailment | Contradiction | NotMentioned | words/doc min / median / mean / max |
|---|---|---|---|---|---|---|
| train | 423 | 7,191 | 3,530 | 841 | 2,820 | 211 / 1,504 / 1,673 / 8,428 |
| dev | 61 | 1,037 | 519 | 95 | 423 | 342 / 1,727 / 1,823 / 4,796 |
| test | 123 | 2,091 | 968 (46%) | 220 (10%) | 903 (43%) | 205 / 1,417 / 1,708 / 6,267 |

- Length: the paper reports 2,254 BERT-tokens per document on average and says 86% exceed 512 tokens. By words the test median is about 1,400, so roughly 2-3k LLM tokens. That fits any current context window; the reading is non-trivial but small. Cost per call is low.
- Evidence format: each document has `spans`, a list of [start,end] character offsets (a sentence or an inline list item, about 78 per document). Each hypothesis annotation holds `choice` and `spans`, a list of indices into that list. Evidence is empty for NotMentioned. Annotators were told to find all redundant evidence, so recall of evidence is well defined. Spans need not be contiguous.
- Published results (test, Span NLI BERT, mean of 3 runs): BERT-large Acc .875, F1(Contradiction) .357, F1(Entailment) .834, evidence mAP .922, P@R80 .793. Best: DeBERTa-v2-xlarge fine-tuned on CUAD span identification, Acc .892, F1(C) .405, F1(E) .859, mAP .936, P@R80 .860. Baselines: majority vote Acc .674; doc-level TF-IDF+SVM .733. Accuracy is inflated by the majority classes, so report per-label results; Contradiction is the hard class. I found no LLM-era leaderboard figures from a primary source (a third-party leaderboard lists only BERT-style models).
- Caveat on "expert-annotated": the paper (appendix A.1.1) says annotation was done by one computational-linguistics researcher (primary annotator) with two Mechanical Turk workers redundantly annotating most of train. The hypotheses were reviewed by paralegals. No lawyer labelled the data. The primary annotator went through most of the test split and some dev items alone. I found no inter-annotator agreement figure in the paper text I extracted. Describe it as "researcher-curated, paralegal-reviewed", not "lawyer-annotated". Label noise is possible, so check disagreements between model and gold by hand.
- Samples (script: /home/user/study/scripts/contractnli_sample.py):
  - /home/user/study/data/contractnli/sample40.jsonl: test, seed 20211005, 14 E / 13 C / 13 NM, 34 distinct documents, document length 586-3,582 words (median 1,436). Every E/C item has evidence spans.
  - /home/user/study/data/contractnli/dev10.jsonl: dev, seed 20240607, 4 E / 3 C / 3 NM, 9 documents. No document overlaps with sample40. Dev and test documents are different files, so no leakage.
  - /home/user/study/data/contractnli/README.md records seed, split and rule. Stratified sampling over-weights Contradiction (33% versus 10% in the real test split), so reweight before quoting population rates. With n=40, per-label n is 13-14: intervals are wide (about +-25 points at 50% accuracy).

## 2. Alternatives (two lines each)

I checked ContractNLI's files directly. The rest is from my knowledge plus a thin web search; licences marked (verify) were not re-checked today.

- CUAD (Hendrycks et al. 2021): 510 contracts, 41 clause types, extractive spans, CC BY 4.0, evidence is the answer span itself. Better on contract diversity and legal-lawyer annotation (Atticus volunteers); worse here because it is span extraction, not a verifiable claim with a three-way label, and "no clause" is only an empty answer (abstention is scorable but the claim framing is lost). Contracts are much longer (tens of thousands of tokens), so cost is higher.
- LegalBench (Guha et al. 2023): 162 tasks, mixed licences per task (verify per task). Some contract tasks (e.g. contract NLI variants, CUAD-derived clause tasks, SARA) have yes/no labels, but most have no evidence spans and passages are short. Worse for this purpose.
- MAUD (Wang et al. 2023): merger agreements annotated by lawyers (ABA deal-point study), multiple-choice questions with an "other/not stated" style option and supporting text spans, CC BY 4.0 (verify). Strongest on expertise and difficulty; worse for a small cost-capped run because the agreements are very long (often 50k+ tokens), the 92 questions use complex multi-choice label sets, and expensive per call.
- FinQA / TAT-QA / DocFinQA: numeric reasoning over financial reports; FinQA and TAT-QA have gold supporting facts, DocFinQA has long filings; licences differ (verify). No "not mentioned" label, so abstention cannot be scored. Worse.
- QASPER (Dasigi et al. 2021): 1,585 NLP papers, 5,049 questions, answer types include unanswerable, evidence paragraphs given, CC BY 4.0 (verify). Best alternative if you want a non-legal domain: abstention (unanswerable) and evidence are both scored, 4-6k word papers. Weakness: questions are free-form, answers are not claims to verify, and the papers are on arXiv (heavy pretraining exposure); annotators were NLP practitioners.
- SciFact (Wadden et al. 2020): claims labelled SUPPORTS / REFUTES / NOT ENOUGH INFO with rationale sentences, expert annotators (biomedical). It fits the label space, but documents are abstracts (about 200-300 words), so reading is trivial; licence is non-commercial as I recall (verify). Worse.
- 2024-2026 legal extraction work: the ContractEval-style evaluations I could find re-use CUAD (Jaccard>0.15 matching, e.g. in the DocETL paper), and Harvey's contract-intelligence benchmark (Nov 2025) is a vendor blog without public data. I found no new public, permissively licensed, evidence-annotated, abstention-aware long contract benchmark that beats ContractNLI on fit. My search was shallow (two queries), so a newer one may exist.

Verdict: ContractNLI fits all four requirements best (spans plus NotMentioned, CC BY, short enough to be cheap, direct download). Use QASPER as a second domain if budget allows.

## 3. Contamination

- The NDAs came from Google PDF search in seven countries' domains and from EDGAR exhibits 1996-2020 (236 EDGAR NDAs kept out of 28,780 candidates). These are public web documents, so the raw contract text is very likely in web-crawl pretraining data of current LLMs. The dataset JSON itself (annotations) has been on GitHub since October 2021, with a project page and a zip, so it may be in code and data crawls too.
- The paper does not report any contamination or memorisation analysis. It predates the LLM era, and its only related text is the fair-use note and the takedown form. My searches found no published ContractNLI-specific contamination audit (only general surveys and a 2026 tax-law paper).
- What it means: (a) a model may have seen the documents, but memorising the contract text does not give it the 17 labels, and the hypotheses are generic, so the practical risk is modest compared with benchmarks whose answers are published in text form. (b) It could have seen the label file, which is a possible leak of gold answers for test items; the test labels have been public for five years. (c) For a verification experiment this mostly matters for the absolute accuracy numbers: they may be optimistic. Comparisons between conditions (with versus without evidence citation, different verifier set-ups) on the same items are less affected.
- Cheap mitigations: report this as a limitation; do not claim absolute performance; optionally run a quick probe (ask the model to complete the start of a test contract, or to name the dataset's labels with the document withheld) on a few items; optionally add a small number of perturbed items (edit a clause so the label flips, with the gold span updated) to check that answers follow the text, not memory.
