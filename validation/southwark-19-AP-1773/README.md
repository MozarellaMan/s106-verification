# Generate the review dossier when needed

Commit the tool, proposed claim inventory, provenance, checklist and tests now.
Generate the review site and full reports locally when ready to review. Human
confirmation is not required to merge this tooling; it is required to promote
annotations to human-verified ground truth. All case annotations currently await
confirmation. Do not use this case to claim validated accuracy or complete coverage.

## Build and open

After retrieving the original PDF with the command in the repository README:

```bash
python3 scripts/build-southwark-dossier.py \
  --case validation/southwark-19-AP-1773 \
  --pdf data/raw/southwark_s106_sample.pdf \
  --serve
```

Open `http://127.0.0.1:8766/dossier.html`. Ctrl-C stops the local server. Without
`--serve`, open the generated `data/processed/reviews/southwark-19-AP-1773/dossier.html`
directly. `--out PATH` selects another output directory outside the case inputs.
The tool accepts other case directories with the same reference/provenance format;
it neither collects other cases nor calls an LLM.

Python3 and Poppler's `pdftoppm` are required. The tool verifies the PDF and reference
hashes before rendering only the evidence pages. The current case rendered 51 pages
in the original pass; rendering time depends on the source. Source files, page images,
HTML, review CSV and generated audit/comparison reports are ignored by Git. The
previous full report and dossier have been preserved locally, outside the commit.

Each card contains proposed meaning, source quotation, PDF/printed locator and
expandable original page image. Start with high-priority claims. Save your later
review using **Export human review JSON**, or edit the review CSV. Regeneration
preserves an existing CSV; it does not change the agent reference. Import exported decisions through
`manage-validation.py import-review`, then rebuild to update reports. Browser form entries need exporting before closing/rebuilding.
Keep the exported review record with the source hash/reference version; human
confirmation must include an actual reviewer and date. Review completeness separately.

## Replay and compare

```bash
bash validation/southwark-19-AP-1773/replay.sh data/raw/southwark_s106_sample.pdf
```

This uses the existing frozen model response, local PDF/OCR runtime and cached OCR
where available, with Cargo offline and LLM replay. Runtime/dependencies must already
be installed using the repository setup. No network or live LLM calls are made.
The script checks source and fixture identity, expects the current diagnostic audit
exit1, checks a compact regression summary, and runs the mutation/comparison controls.
Detailed results remain local under `data/processed/validation/southwark-19-AP-1773`.

The compact agent-labelled baseline is 81 agreements,2 differences,11 missing fields
across94 selected comparisons;12 of14 structured payment names represented.
Human-confirmed metrics are unavailable. These counts are not whole-deed accuracy.
See protocol.md and completeness.md for contamination, unread-page and coverage limits.
The reference contacts retain the original extraction history; historical snapshots
and the long narrative report remain local. Human review can proceed alongside adding cases. A generated site or verifier
PASS does not establish human-verified accuracy or document completeness.

Controls: `cargo test --offline --test validation_mutations` and
`python3 -m unittest discover -s tests/python -v`. The renderer has a synthetic
page/link/hash-check regression test in the same Python suite.

Multi-case registry, human corrections/additions and dynamic reports: [workflow](../../docs/review-workflow.md).
