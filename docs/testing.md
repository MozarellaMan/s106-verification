# Testing and evaluation tiers

The expensive stages are reading PDFs (PDFium, and OCR for scans: the 109-page Southwark deed
takes about four minutes) and calling the model. Each stage's output is kept as a reproducible
artifact with its provenance, so everything after it can be tested without re-running it:

| Stage | Artifact | Keyed by |
|---|---|---|
| PDF -> page text | `data/interim/ocr/<sha>-<ocr_version>/` (cache), `fixtures/pages/<stem>.pages.json` (committed snapshot) | PDF SHA-256; line-logic version (`PAGE_TEXT_LOGIC`), OCR settings, pinned SHA-256 of PDFium, ONNX Runtime and each model file |
| PDF -> model answer | `fixtures/llm/<stem>/<model>.extract.v2.json` | PDF SHA-256; hash of prompts and schemas |
| answer + page text -> checks | `fixtures/expected/<stem>.checks.json`, `fixtures/expected/eval-replay.json` | the code (reviewed in the diff) |

A change to any key gives a new cache directory, and a snapshot or fixture recorded under the old
key fails tier 2 until it is regenerated. Model files that differ from their pins
(`scripts/runtime-artifacts.tsv`) are refused before use; PDFium and ONNX Runtime are loaded from
directories named by their verified archive's SHA-256, so the library loaded is the one the key
names. The cache manifest is validated on read, and cache files are written through a temporary
file unique to each writer. Snapshot comparison is exact: every page, its source, dimensions
(page width decides which margin specks are dropped from the evidence text), lines and boxes. Each image-only page is OCR'd on its own: oar-ocr pads a batch to a common size, and pages
recognised in batches of six read differently from the same pages read alone (5 of 9 TMBC pages,
106 of 260 corpus pages). Page text is now a function of the page alone, so a partly warm cache, a
partial load and a full cold run all agree.

## The tiers

| Tier | What | Command | Needs | When |
|---|---|---|---|---|
| 1 | Unit tests: verifier, evidence matching, numbers, triggers, wire conversion, register scoring | `cargo test` | nothing | every change; CI on every push |
| 2 | Offline replay: committed page text + recorded answers -> expected outcomes for all ten deeds; the frozen Python M1 outcomes; OCR gold thresholds on the snapshots; register evaluation | `cargo test` | nothing | every change; CI on every push |
| 3 | Real PDFs, targeted: 25 difficult pages (`fixtures/pages/difficult.tsv`) read afresh into an empty cache must match their snapshots; OCR gold pages must meet the thresholds | `cargo test-pdf` | runtime artifacts, corpus | before merging OCR/layout changes; CI weekly |
| 4 | Real PDFs, whole corpus: every deed read through the pipeline must reproduce its snapshot; register evaluation on the PDFs | `cargo test-corpus`, `s106 eval --replay` | runtime artifacts, corpus | CI weekly, on a cold cache; on demand |
| 5 | Live model calls: new extractions, run-to-run agreement | `s106 eval --runs 3`, `s106 audit <pdf> --llm-mode record` | API key; costs money | only when requested (manual CI workflow) |

`cargo test` (tiers 1 and 2) never opens a PDF, loads PDFium or ONNX Runtime, reads a model file,
or touches the network, and the build no longer downloads ONNX Runtime (it is loaded at run time
from `models/`, like PDFium). Tiers 3 and 4 fail with "corpus missing" rather than skip.

Corpus downloads are checked before they are installed. `scripts/fetch-corpus.sh` exits 1 when a
file fails its checksum (the weekly job fails) and 2 when a council site can't be reached (the
weekly job skips the PDF tiers with a warning; third-party outages never fail CI). `s106 eval`
fails, after writing its summary, unless every selected deed completed every requested run.

Tiers 3 and 4 are what make tier 2 trustworthy. Everything after page text is deterministic, so
"every snapshot reproduces from its PDF" (tier 4) plus "replay from snapshots gives these
outcomes" (tier 2) covers what the old replay-from-PDF tests covered. It was checked directly:
the Southwark audit and the nine-deed register evaluation give identical results from the PDFs
and from the snapshots.

What these tiers establish is repeatability: the same inputs give the same page text and the same
outcomes, and changes to either are visible in review. They do not establish that the outcomes
are correct. The expected outcomes were blessed from the pipeline's own output, and the OCR gold
set helped choose the OCR model. Correctness claims still need independently annotated evidence,
ideally on held-out deeds.

## Timings

One 4-vCPU Linux container, release builds unless stated. *Cold* means an empty page-text cache;
*warm* means the cache already holds every page. "Before" is `main` with only the change that
lets ONNX Runtime load at run time (the container cannot reach its build-time download).

| | Before, cold | Before, warm | After, cold | After, warm |
|---|---:|---:|---:|---:|
| Frozen Python M1 comparison (10 deeds) | 502 s (ignored test) | 0.8 s | in `cargo test`, 0.07 s, no cache involved | same |
| OCR gold pages | 13.5 s (ignored test) | 0.6 s | 46 s with the 25 difficult pages (tier 3, always cold by design) | n/a |
| Every deed read from its PDF and checked | no such test | | 494 s (tier 4) | 0.6 s |
| `s106 eval --replay` (9 deeds, on PDFs) | 274 s | 0.4 s | 275 s | 0.2 s |
| `s106 eval --replay --pages fixtures/pages` | not available | | 0.08 s, no cache involved | same |
| `s106 audit` Southwark, replay | 230 s | 0.08 s | 235 s | 0.07 s |
| `s106 audit fixtures/pages/southwark_s106_sample.pages.json` | not available | | 0.05 s | same |
| Writing all ten snapshots (`s106 pages --write`) | | | 504 s (484 s with batches of six) | |

Default `cargo test` (debug), tiers 1-2: 131 s from an empty `target/` (almost all compiling
dependencies; 146 s before, with no replay tests), 19 s after touching `src/verify.rs`, 2.6 s with
nothing changed. The test binaries themselves take about 1.5 s in debug, of which tier 2 is about 1 s (0.2 s in a release build). Tests in the default run: 113, up from 108 (the frozen Python comparison and four replay tests joined it); 4 are ignored real-PDF tests, previously 3.

A warm page-text load no longer opens the PDF in PDFium: it hashes the file and reads the cached
pages. OCR of one page at a time costs about 4% more than batches of six.

Live tier 5 was not run for this change (it needs a key and costs money). Recorded costs are in
[migration.md](migration.md): about $1.03 for one run over all ten deeds.

### Scaling

Per deed, tier 2 costs one snapshot read and one verification: under 0.1 s for all ten deeds
here, so a thousand deeds would add seconds, not minutes. Snapshots average about 3.5 KB per page
(1.2 MB for 341 pages); a thousand 40-page deeds would be roughly 140 MB, which is past what
belongs in git. At that point keep the snapshots outside the repository (a release asset or
bucket, listed with their SHA-256s like `fixtures/corpus.tsv`) and point `fixtures/pages` at a
local copy; nothing in the code assumes they are committed. Tiers 3 and 4 scale with pages
needing OCR (about 2 s each here), so tier 4 is the one to schedule rather than run per change.

## Updating artifacts

- **Verifier, extraction conversion or eval change** that alters outcomes: tier 2 fails and
  prints each changed check. If intended: `S106_BLESS=1 cargo test --test replay`, then review
  the diff of `fixtures/expected/`.
- **Line logic, OCR settings or a runtime artifact**: bump `PAGE_TEXT_LOGIC` (line logic) or the
  pins, then regenerate the snapshots from the PDFs with
  `s106 pages data/raw/southwark_s106_sample.pdf data/raw/eval/*.pdf --write fixtures/pages`,
  bless the expected outcomes, and review both diffs (one page per line).
- **Prompt or schema**: re-record the fixtures (`--llm-mode record`, tier 5), then bless.
- **New deed**: add it to `fixtures/corpus.tsv`, fetch it, record its fixture, write its
  snapshot, bless.
- **A difficult page found in review**: add it to `fixtures/pages/difficult.tsv` with the reason.
