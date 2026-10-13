# Invented deeds

Three deeds that describe no real site, council or planning application. Each has the page text
a reader would produce (`pages.txt`), the extraction a model would return (`extraction.json`, in
the same wire format as a recorded response) and the gate's pinned outcomes (`checks.json`).
`tests/synthetic.rs` runs them in the default `cargo test`; `tests/common/synthetic.rs` documents
the `pages.txt` format.

| deed | built to show |
|---|---|
| `agreement` | every check passing, including through OCR damage: `£` read as `f`, dropped spaces, a two-column definition split across rows |
| `discrepancies` | each failure mode: a grounded arithmetic mismatch and a figures/words mismatch (Document Discrepancy), a missing count and an invented trigger phrase (Parser Error), a wrong page, an OCR-repaired figure and a value its snippet doesn't state (REVIEW), a formula sum (SKIP) |
| `variation` | a deed of variation: whole-scheme checks skipped, its parent and each change grounded, and a change the deed never makes (REVIEW) |

To add a case, edit `pages.txt` and `extraction.json`, say in `tests/synthetic.rs` which outcome
it should produce, then regenerate `checks.json` with `S106_BLESS=1 cargo test --test synthetic`
and review the diff.
