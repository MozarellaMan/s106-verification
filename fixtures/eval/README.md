# Evaluation inputs

- `candidates.tsv`: the nine evaluation deeds (id, council, planning reference, type, URL), as
  used since the register evaluation began. The PDFs themselves are in `../corpus.tsv`.
- `register/developer-agreement.csv`, `register/developer-agreement-contribution.csv`: the rows of
  the national S106 register (planning.data.gov.uk, Open Government Licence) for these deeds. They
  were filtered from the full dataset files
  (`https://files.planning.data.gov.uk/dataset/developer-agreement.csv` and
  `.../developer-agreement-contribution.csv`, local copies dated 13 Oct 2026, SHA-256 `edc89267…`
  and `07b09015…`): every agreement whose `planning-application` is a candidate's
  `planning_ref`, and every contribution whose `developer-agreement` is one of those
  agreements' `reference`. The evaluation reads nothing else from the register, so the subset
  scores identically to the full files. For new deeds, download the full files and pass
  `s106 eval --register <dir>`.
