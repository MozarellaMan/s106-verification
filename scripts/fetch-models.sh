#!/usr/bin/env bash
# Fetch the runtime artifacts into ./models (gitignored) and verify each against its pinned SHA-256
# in scripts/runtime-artifacts.tsv:
#   PDFium:       bblanchon/pdfium-binaries chromium/8100 (renders pages, reads text layers)
#   ONNX Runtime: microsoft/onnxruntime 1.28.0, loaded at run time by the OCR
#   OCR:          PP-OCRv5 mobile detection + English recognition and dictionary, published by oar-ocr
#                 (PaddleOCR models, Apache-2.0). The English dictionary includes "£".
# Pass --bench to also fetch the alternative model sets compared in `s106 bench-ocr`.
set -euo pipefail
cd "$(dirname "$0")/.."
mkdir -p models
sets="default"
[ "${1:-}" = "--bench" ] && sets="default bench"
grep -v '^#' scripts/runtime-artifacts.tsv | while IFS=$'\t' read -r set file sha url; do
  case " $sets " in *" $set "*) ;; *) continue ;; esac
  f="models/$file"
  [ -f "$f" ] || curl -sSfL -o "$f" "$url"
  echo "$sha  $f" | sha256sum -c --quiet || { echo "checksum mismatch: $f" >&2; rm -f "$f"; exit 1; }
done
# Unpack each verified archive into a directory named by its SHA-256 (what the code loads), via a
# temporary directory so an interrupted unpack is never mistaken for a complete one.
unpack() { # archive name [tar args]
  local sha dir
  sha=$(awk -F'\t' -v f="$1" '$2 == f { print $3 }' scripts/runtime-artifacts.tsv)
  dir="models/$2-${sha:0:12}"
  [ -d "$dir" ] && return
  rm -rf "$dir.part" && mkdir -p "$dir.part"
  tar -xzf "models/$1" -C "$dir.part" "${@:3}"
  mv "$dir.part" "$dir"
}
unpack pdfium-linux-x64.tgz pdfium
unpack onnxruntime-linux-x64-1.28.0.tgz onnxruntime --strip-components=1
echo "models ok"
