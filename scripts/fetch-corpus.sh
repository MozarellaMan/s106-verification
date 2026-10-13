#!/usr/bin/env bash
# Fetch the deed corpus listed in fixtures/corpus.tsv and verify each PDF against its SHA-256.
# Needed only for the real-PDF tiers (tests/pdf_regression.rs, tests/corpus.rs, `s106 eval` on
# PDFs); `cargo test` replays committed page text and never opens a PDF.
#   scripts/fetch-corpus.sh [id ...]      (default: every deed)
# Southwark's server omits its Sectigo intermediate certificate; if curl reports an SSL error, add
# http://crt.sectigo.com/SectigoPublicServerAuthenticationCAOVR36.crt to a CA bundle and set
# CURL_CA_BUNDLE to it.
set -euo pipefail
cd "$(dirname "$0")/.."
jar=$(mktemp)
trap 'rm -f "$jar"' EXIT
failed=0
while IFS=$'\t' read -r id path sha url session; do
  if [ $# -gt 0 ] && [[ ! " $* " == *" $id "* ]]; then continue; fi
  if [ ! -f "$path" ]; then
    mkdir -p "$(dirname "$path")"
    [ "$session" = "-" ] || curl -sSL -c "$jar" -b "$jar" -o /dev/null "$session" || true
    curl -sSfL -c "$jar" -b "$jar" -o "$path.part" "$url" && mv "$path.part" "$path" || {
      echo "download failed: $id ($url)" >&2; rm -f "$path.part"; failed=1; continue; }
  fi
  echo "$sha  $path" | sha256sum -c --quiet || { echo "checksum mismatch: $path" >&2; failed=1; }
done < <(grep -v -e '^#' -e '^id	' fixtures/corpus.tsv)
[ $failed = 0 ] && echo "corpus ok"
exit $failed
