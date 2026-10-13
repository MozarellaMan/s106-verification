#!/usr/bin/env bash
# Fetch the deed corpus listed in fixtures/corpus.tsv. Each download is checked against its SHA-256
# before it is put in place; a file that doesn't match is never installed.
# Needed only for the real-PDF tiers (tests/pdf_regression.rs, tests/corpus.rs, `s106 eval` on
# PDFs); `cargo test` replays committed page text and never opens a PDF.
#   scripts/fetch-corpus.sh [id ...]      (default: every deed)
# Exit status: 0 every deed present and verified; 1 a file failed its checksum (an integrity
# failure, never to be ignored); 2 a source could not be reached (third-party outage).
# Southwark's server omits its Sectigo intermediate certificate; if curl reports an SSL error, add
# http://crt.sectigo.com/SectigoPublicServerAuthenticationCAOVR36.crt to a CA bundle and set
# CURL_CA_BUNDLE to it.
set -euo pipefail
cd "$(dirname "$0")/.."
jar=$(mktemp)
trap 'rm -f "$jar"' EXIT
mismatch=0
unavailable=0
while IFS=$'\t' read -r id path sha url session; do
  if [ $# -gt 0 ] && [[ ! " $* " == *" $id "* ]]; then continue; fi
  if [ -f "$path" ]; then
    echo "$sha  $path" | sha256sum -c --quiet || { echo "checksum mismatch: $path" >&2; mismatch=1; }
    continue
  fi
  mkdir -p "$(dirname "$path")"
  [ "$session" = "-" ] || curl -sSL -c "$jar" -b "$jar" -o /dev/null "$session" || true
  if ! curl -sSfL -c "$jar" -b "$jar" -o "$path.part" "$url"; then
    echo "unavailable: $id ($url)" >&2; rm -f "$path.part"; unavailable=1; continue
  fi
  if echo "$sha  $path.part" | sha256sum -c --quiet; then
    mv "$path.part" "$path"
  else
    echo "checksum mismatch: $id ($url) served a different file" >&2; rm -f "$path.part"; mismatch=1
  fi
done < <(grep -v -e '^#' -e '^id	' fixtures/corpus.tsv)
[ $mismatch = 1 ] && exit 1
[ $unavailable = 1 ] && exit 2
echo "corpus ok"
