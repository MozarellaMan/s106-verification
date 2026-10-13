# M1.5: Rust feasibility spike (PDF text, rendering, OCR)

A standalone experiment, not a port. It asks one question: can the document layer of the S106
harness (page text, OCR, coordinates, page references) be built in Rust at least as well as the
current Python pipeline, and be maintained by a Rust/Java developer?

Everything here was verified by building and running it on 13 Oct 2026 (Rust 1.99), not taken
from documentation. `./run-spike.sh` reproduces every number below.

## Ground truth

The baselines aren't the truth: `gold/` holds **hand-transcribed** lines and facts, read from page
renders, with both the PDF page index and the *printed* page number:

- **`gold/southwark_19_AP_1773.json`:** the scanned Southwark deed, 6 pages, 20 lines, 9 facts and
  3 two-column term/definition pairs. Pages: two-column definitions (4, 5, 12, 14), the Carbon
  Green Fund schedule (89) and the mix table (91).
- **`gold/salford_20_75239.json`:** a born-digital Salford deed with a text layer, 2 pages,
  8 lines and 3 facts.

Printed and PDF page numbers diverge in the schedules: PDF p89 is printed "74", and PDF p91 is
printed "76". A "correct page reference" has to say which one it means.

The gold set is small: 28 lines and 12 facts. Differences of a line or a fact between engines are
within noise. The findings below rest on systematic behaviour, not on small score gaps.

## What was tested

| Role | Crate (version) | What it needs at runtime |
|---|---|---|
| Render pages and read the text layer | `pdfium-render` 0.9.4 | `libpdfium.so` (bblanchon build 8100, pinned with SHA-256 in `fetch-artifacts.sh`) |
| OCR, pure Rust | `ocrs` 0.13.1 + `rten` 0.26 | two `.rten` models (12 MB, pinned) |
| OCR, PaddleOCR | `oar-ocr` 0.10.0 → `ort` 2.0.0-**rc.13** | ONNX Runtime (statically linked; **downloaded from pyke's CDN at build time**), plus the PP-OCRv4 `.onnx` models |
| Text layer, pure Rust | `pdfplumber` 0.4.1 (pdfplumber-rs, "alpha") | none |
| Text layer, pure Rust | `pdf-extract` 0.12.1 | none |
| Classify the document and convert it to Markdown | `pdf-inspector` 1.25.2 | none |

The PaddleOCR path runs **the exact model files the Python pipeline uses** (RapidOCR's
`ch_PP-OCRv4_det/rec`, with the dictionary taken from the model's metadata). Any differences from
Python therefore come from the pipeline, not the model.

**Tesseract wasn't built.** `tesseract-rs` downloads the Leptonica and Tesseract sources from GitHub
during `cargo build` and compiles them with cmake. Neither the library nor the CLI is installed
here, so it would add a large C++ build for an engine the pipeline doesn't use.

## Results

### Scanned deed (Southwark), 6 gold pages

| | Rust `ocrs` | Rust `oar-ocr` PP-OCRv4 | Python RapidOCR PP-OCRv4 |
|---|---|---|---|
| Line CER, spaced / squashed (lower is better) | 0.010 / 0.011 | **0.008 / 0.009** | 0.054 / 0.009 |
| Facts on the correct page (raw / after the verifier's £ repair) | 5/9 · **8/9** | 5/9 · 7/9 | 5/9 · 7/9 |
| Printed page number recovered | 1/6 | **4/6** (by position) | 3/6 |
| Two-column definitions | kept apart | kept apart | **merged** with the term |
| Coordinates kept | per character | per line (word boxes, no word text) | **none** (discarded) |
| Time, single process | 1.3–1.5 s/page | 3.1–3.4 s/page | ~6 s/page |
| "£" | read as `?`: **not in the model's alphabet** | read as `f`/`F`/dropped | read as `f`/dropped |

**Full document (109 pages).**
- *Page selection:* fed into the existing Python page selector, Rust PP-OCRv4 text selects **the
  same 42 pages** as Python's own OCR. `ocrs` text makes the Southwark-tuned heuristic end the
  definitions at p14 instead of p32, so it would drop 14 pages. That says as much about the
  heuristic as about `ocrs`.
- *Time:* 6 m 15 s for PP-OCRv4 and 2 m 45 s for `ocrs`, both single-process.

**Findings that matter more than the scores:**
- **"£" is not readable by any engine tested.** `ocrs`'s alphabet is ASCII-only, and the PP-OCRv4
  dictionary (6,623 characters) has no "£" either. The Python verifier works today only because
  it repairs `f`/`?` before a digit. A model whose dictionary includes "£" (e.g. a Latin or English
  PP-OCR recogniser) is worth testing in either language.
- **`ocrs` drops isolated small digits.** It missed the 7/5/6 table cells and most page numbers.
  That's disqualifying for tables and folios until its models improve.
- **Coordinates fix the two-column problem at the source.** Both Rust OCR paths return each column
  as separate boxes, and table rows can be rebuilt by grouping boxes at the same height (the
  scorer does this). The Python pipeline merges columns and then needs the verifier's interleaving
  workaround.

### Born-digital deed (Salford), 2 gold pages

| | PDFium text | `pdfplumber` (rs) | `pdf-extract` | `pdf-inspector` | Python pdfplumber |
|---|---|---|---|---|---|
| Line CER, spaced | **0.000** | **0.000** | **0.000** | 0.014 (Markdown reflows) | **0.000** |
| Facts | 3/3 | 3/3 | 3/3 | 3/3 | 3/3 |
| Printed page number | 2/2 | 2/2 | 2/2 | 0/2 | 2/2 |
| Geometry | per character | per word and line | none | none | available, but `ocr.py` discards it |

`pdf-inspector`'s classifier was right on both deeds: Salford came back `TextBased` (1.00) and
Southwark `Scanned` (0.95, all 109 pages need OCR). It's useful for routing pages between the
text layer and OCR. Every text crate returns nothing on the scan.

None of our ten deeds has two-column definitions in a text layer; in this corpus that layout
occurs only in scans. Two-column text-layer extraction is untested here, but PDFium's and
pdfplumber's per-character and per-word boxes make it a geometry problem rather than a guess.

## Maintenance and reproducibility

| | Python (current) | Rust (this spike) |
|---|---|---|
| Native components | PDFium (in the pypdfium2 wheel), ONNX Runtime, OpenCV | PDFium (`libpdfium.so`, pinned), ONNX Runtime (static) |
| Packages | 36 in the venv (475 MB) | 290 crates in the full spike; `pdfium-render` 103, `oar-ocr` 190, `ocrs` 32 |
| Build-time network | pip | crates.io, **plus pyke's CDN for ONNX Runtime**, unless `ort` is set to load a pinned system library |
| Model files | in the RapidOCR wheel | fetched and checksummed (`fetch-artifacts.sh`) |
| Release binary | n/a | 58 MB (ONNX Runtime linked in) |
| Spike code | n/a | ~600 lines (`src/`), including the scorer |

**Ecosystem risks:**
- **Single maintainers.** Every crate here has one owner: `pdfium-render` (89 releases since
  2022), `ocrs` (21 since 2023), `oar-ocr` (26 since mid-2025) and `pdfplumber`-rs (4 since
  Feb 2026, self-described alpha).
- **A release-candidate runtime binding.** `ort` is at 2.0 rc.13.
- **API drift showed up immediately.** `ocrs` pins an older `rten` than the latest, and
  `pdfium-render` 0.9 takes `i32` page indices, not the `u16` its older documentation shows.
  Both were caught by the compiler, which is the point in Rust's favour: drift fails the build
  rather than an audit.
- **Bugs I made in my own code were caught by measurement, not by the type system.** Line grouping
  on tight boxes and a single-digit filter that hid page numbers each looked fine until scored
  against gold. The gold set and scorer, not the language, are what make this auditable.

**No official Anthropic SDK for Rust.** The SDKs cover Python, TypeScript, Java, Go, Ruby, C# and
PHP. A Rust harness would call the Messages API over HTTP, with structured-output JSON schemas
written or generated by hand. **Java has an official SDK.**

## Recommendation

**Migrate the document layer and the verifier to Rust, in stages; decide the LLM-client language
explicitly.**

1. **The document layer is feasible and better in Rust:**
   - PDFium for rendering and text with character boxes.
   - PP-OCRv4 through `oar-ocr`, which matches Python's OCR quality with identical models while
     keeping spacing, column separation and coordinates.
   - `pdf-inspector`'s classifier for routing pages between text layer and OCR.

   Coordinates also open up evidence provenance with boxes, which the Python pipeline throws away.
   Don't use `ocrs` yet, because of the missing "£" and isolated digits.
2. **The verifier** is plain logic (`verifier.py` + `triggers.py`, ~640 lines) with 58 tests and
   recorded fixtures. It's a good first port: the fixtures are language-neutral JSON and the
   tests become the acceptance suite.
3. **LLM extraction:** either Rust over raw HTTP (no official SDK; you own retries, typed errors
   and schema generation), or a small Java service or CLI on the official SDK. This is a
   maintainability trade-off for you to make; the research design is unaffected.
4. **Keep Python as the regression oracle until the Rust harness reproduces it.** That means the
   M1 verifier outputs on the Southwark deed and the nine evaluation deeds (rules and replay), with
   every difference explained against the gold set or the deed, not assumed to be a Rust bug.

**Before M2, pin the reproducibility gaps:**
- build `ort` against a pinned, checksummed ONNX Runtime instead of the build-time CDN download
- vendor or checksum the PP-OCRv4 models
- test a recogniser that includes "£"
