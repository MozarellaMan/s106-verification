# Results explorer

A map of London housing permissions of 10 or more homes, as boroughs recorded them in the Planning
London Datahub, with a panel showing what is known about each site at each stage. It is a static
site: Leptos (Rust compiled to WebAssembly) draws the page and Leaflet draws the map. Nothing runs on
a server.

**What it shows is public data only.** The data comes from `s106 explore`, which reads a Datahub
baseline run in the data repository: Datahub figures (Open Government Licence) and ONS borough
boundaries, nothing from deeds, proposals, labels or the results ledger. Stages the Datahub can't
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

## Publish it

`.github/workflows/explorer.yml` builds the site and deploys it to GitHub Pages. It only runs when
started by hand (Actions → explorer → Run workflow), so merging a change never publishes anything.
Pages must be set to "GitHub Actions" under Settings → Pages before the first run.
