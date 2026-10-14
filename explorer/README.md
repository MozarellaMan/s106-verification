# Results explorer

A map of London housing permissions of 10 or more homes, as boroughs recorded them in the Planning
London Datahub, with a panel showing what is known about each site at each stage. It is a static
site: Leptos (Rust compiled to WebAssembly) draws the page and Leaflet draws the map. Nothing runs on
a server.

**What it shows is public data only.** The data comes from `s106 explore`, which reads a Datahub
baseline run in the data repository: Datahub figures (Open Government Licence) and ONS borough
boundaries, nothing from deeds, proposals, labels or the results ledger (except in a private build,
below). Stages the Datahub can't
speak to (offer, obligation, variation) are drawn as "not collected". Anything scheme-level from
the ledger stays out of this build until it has been reviewed and signed off.

## Build it locally

You need the `wasm32-unknown-unknown` target and [trunk](https://trunkrs.dev):

```bash
rustup target add wasm32-unknown-unknown
cargo install trunk --locked

cargo run --release -- explore --out explorer/data   # from the repository root, with the data checkout
cd explorer && trunk serve                           # http://127.0.0.1:8080
```

`s106 explore` takes `--applications`, `--snapshot` and `--boundaries` (paths in the data
repository) to point at a newer baseline run. `explorer/data/` and `explorer/dist/` are never
committed.

## A private build with results

`--results` adds a results folder's ledger (for example
`--results results/2026-10-14-batch1`, from `s106 results`) to the schemes it covers. Their Offer,
Decision, Obligation, Variation and Completion stages then show each source's cell with its check
level, a banner marks the page as private, and a list on the right finds those schemes. These are
unreviewed readings: view the build locally with `trunk serve` and never publish it. The Pages
workflow never passes `--results`, and refuses to deploy data that carries a results folder,
retrieval rows, or any reading other than the deed cells described below.

## Deed figures on the public site

`--deed-cells results/<batch>` adds, from that results folder, every cell read from the deed for the
obligation, variation and completion stages, with its pages and its check level. A deed's offer
cells only recite the committee report or policy, so they stay out. Unchecked AI readings are marked
UNCHECKED AI READING and drawn dashed, and cells a person has checked are tagged CHECKED BY A
PERSON. Nothing else from the ledger goes in: no other source, no retrieval rows, no rule outcome or
claim. The page says these are transcriptions of the obligations as first signed, mostly not yet
checked by a person, that only variations in the documents held are shown, and that a difference
from the Datahub is never evidence of a breach.

## Publish it

`.github/workflows/explorer.yml` builds the site and deploys it to GitHub Pages. It only runs when
started by hand (Actions → explorer → Run workflow), so merging a change never publishes anything.
Its `deed_cells` input names a results folder whose deed figures to publish; left empty, the
site carries Datahub figures only. A check before deploy refuses any other ledger cell.
Pages must be set to "GitHub Actions" under Settings → Pages before the first run.
