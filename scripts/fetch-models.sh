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
[ -f models/pdfium/lib/libpdfium.so ] || { mkdir -p models/pdfium && tar -xzf models/pdfium-linux-x64.tgz -C models/pdfium; }
[ -f models/onnxruntime/lib/libonnxruntime.so ] || {
  mkdir -p models/onnxruntime && tar -xzf models/onnxruntime-linux-x64-1.28.0.tgz -C models/onnxruntime --strip-components=1
}
echo "models ok"
