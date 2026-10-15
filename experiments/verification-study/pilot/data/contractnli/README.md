# ContractNLI samples (CC BY 4.0, Koreeda & Manning 2021)
Source: https://stanfordnlp.github.io/contract-nli/resources/contract-nli.zip (raw copy in ../contractnli_raw/).
Script: /home/user/study/scripts/contractnli_sample.py (run with `python3 -I`; reads ../contractnli_raw/x/contract-nli/).

- sample40.jsonl: TEST split, seed 20211005, random.Random(seed). Pairs (document, hypothesis) = 2,091 test pairs.
  Rule: per label, sort pairs by (doc_id, hypothesis_id), rng.sample without replacement: 14 Entailment, 13 Contradiction, 13 NotMentioned; then shuffle. Not de-duplicated by document (34 distinct docs).
- dev10.jsonl: DEV split, seed 20240607, same rule: 4 Entailment, 3 Contradiction, 3 NotMentioned. Use ONLY this for prompt tuning.
- Fields: item_id, doc_id, file_name, document_type, hypothesis_id, hypothesis, gold_label, gold_evidence_span_ids, gold_evidence_spans ([start,end,text] char offsets into document_text), document_text, n_words.
- Test items must not be used for tuning. Stratification is by label, so the sample is NOT label-representative (real test is 46% E / 10% C / 43% NM); reweight if estimating population rates.
