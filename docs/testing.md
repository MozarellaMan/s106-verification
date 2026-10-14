# Testing tiers

The expensive stage is reading PDFs (PDFium, plus OCR for scans: the corpus's 109-page deed takes
about four minutes). Its output is kept as a reproducible artifact with its provenance, so later
stages can be tested without re-running it:

| Stage | Artifact | Keyed by |
|---|---|---|
| PDF -> page text | `data/interim/ocr/<sha>-<ocr_version>/` (cache), `data/pages/<stem>.pages.json` (snapshot, in the data repository) | PDF SHA-256; line-logic version (`PAGE_TEXT_LOGIC`), OCR settings, pinned SHA-256 of PDFium, ONNX Runtime and each model file |

A change to any key gives a new cache directory, and a snapshot recorded under the old key fails
tier 2 until it is regenerated. Model files that differ from their pins
(`scripts/runtime-artifacts.tsv`) are refused before use. PDFium and ONNX Runtime are loaded from
directories named by their verified archive's SHA-256, so the library loaded is the one the key
names. The cache manifest is validated on read, and each writer writes cache files through its own
temporary file. Snapshot comparison is exact: every page, its source, its dimensions (page width
decides which margin specks are dropped from the page text), lines and boxes. Each image-only
page is OCR'd on its own, because oar-ocr pads a batch to a common size and pages recognised in
batches of six read differently from the same pages read alone (5 of 9 pages of one scanned deed,
106 of 260 corpus pages). Page text is therefore a function of the page alone, so a partly warm
cache, a partial load and a full cold run all agree.

## The tiers

| Tier | What | Command | Needs | When |
|---|---|---|---|---|
| 1 | Unit tests: page-text layout, labels, results, figures, explorer data | `cargo test` | nothing | every change; CI on every push |
| 2 | The real corpus's snapshots: each is current for its PDF and settings; OCR gold thresholds on the snapshots | `cargo test-data` (`tests/replay.rs`) | the data checkout (`data/` or `S106_DATA`) | every change; CI on every push (second job) |
| 3 | Real PDFs, targeted: 25 difficult pages (`data/pages/difficult.tsv`) read afresh into an empty cache must match their snapshots; OCR gold pages must meet the thresholds | `cargo test-pdf` | runtime artifacts, data checkout, corpus | before merging OCR/layout changes; CI weekly |
| 4 | Real PDFs, whole corpus: every deed read through PDFium/OCR must reproduce its snapshot | `cargo test-corpus` | runtime artifacts, data checkout, corpus | CI weekly, on a cold cache; on demand |

`cargo test` (tier 1) needs no data. `cargo test-data` (tier 2) needs the data checkout and fails
with "data missing" without it. Neither tier opens a PDF, loads PDFium or ONNX Runtime, reads a
model file, or touches the network. Tiers 3 and 4 fail with "corpus missing" rather than skip.

Corpus downloads are checked before they are installed. `scripts/fetch-corpus.sh` exits 1 when a
file fails its checksum (the weekly job fails) and 2 when a council site can't be reached (the
weekly job skips the PDF tiers with a warning; third-party outages never fail CI).

The tiers establish repeatability: the same PDF gives the same page text, and changes to it are
visible in review. They do not establish that the text is correct. The OCR gold set, which an AI
agent transcribed and no person has yet checked, helped choose the OCR model.

## Timings

One 4-vCPU Linux container, release builds, measured October 2026. *Cold* means an empty page-text
cache; *warm* means the cache already holds every page.

| | Cold | Warm |
|---|---:|---:|
| OCR gold pages with the 25 difficult pages (tier 3, always cold by design) | 46 s | n/a |
| Every deed read from its PDF and checked (tier 4) | 494 s | 0.6 s |
| Writing all ten snapshots (`s106 pages --write`) | 504 s (484 s with batches of six) | |

A warm page-text load does not open the PDF in PDFium: it hashes the file and reads the cached
pages. OCR of one page at a time costs about 4% more than batches of six.

### Scaling

Snapshots average about 3.5 KB per page (1.2 MB for 341 pages); a thousand 40-page deeds would be
roughly 140 MB, which is more than belongs in git. At that size, keep the snapshots outside the
repository (a release asset or a bucket, listed with their SHA-256s like `corpus/manifest.tsv`) and
point the data checkout's `pages/` at a local copy; nothing in the code assumes they are committed.
Tiers 3 and 4 scale with the pages needing OCR (about 2 s each here), so tier 4 is the one to
schedule rather than run per change.

## Updating artifacts

- **Line logic, OCR settings or a runtime artifact**: bump `PAGE_TEXT_LOGIC` (line logic) or the
  pins, then regenerate the snapshots from the PDFs with
  `s106 pages data/raw/*.pdf --write data/pages`, and review the diff (one page per line).
- **New deed**: add it to `corpus/manifest.tsv` (data repository), fetch it and write its snapshot.
- **A difficult page found in review**: add it to `pages/difficult.tsv` (data repository) with the reason.
