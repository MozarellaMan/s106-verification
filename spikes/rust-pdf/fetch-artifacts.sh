#!/usr/bin/env bash
# Fetch pinned runtime artifacts into ./artifacts (gitignored) and verify their SHA-256.
# PDFium: prebuilt by bblanchon/pdfium-binaries (the source pdfium-render's docs point to).
# ocrs:   the detection and recognition models published by the ocrs author.
set -euo pipefail
cd "$(dirname "$0")"
mkdir -p artifacts
fetch() { # url dest sha256
  [ -f "$2" ] || curl -sSfL -o "$2" "$1"
  echo "$3  $2" | sha256sum -c --quiet || { echo "checksum mismatch: $2" >&2; exit 1; }
}
PDFIUM_TAG=chromium/8100
fetch "https://github.com/bblanchon/pdfium-binaries/releases/download/${PDFIUM_TAG}/pdfium-linux-x64.tgz" \
      artifacts/pdfium-linux-x64.tgz 1c1894a12d4d96ee01400c3874444126254180c5c7d2ab628576b67f8120320a
[ -f artifacts/pdfium/lib/libpdfium.so ] || { mkdir -p artifacts/pdfium && tar -xzf artifacts/pdfium-linux-x64.tgz -C artifacts/pdfium; }
fetch https://ocrs-models.s3-accelerate.amazonaws.com/text-detection.rten artifacts/text-detection.rten f15cfb56bd02c4bf478a20343986504a1f01e1665c2b3a0ad66340f054b1b5ca
fetch https://ocrs-models.s3-accelerate.amazonaws.com/text-recognition.rten artifacts/text-recognition.rten e484866d4cce403175bd8d00b128feb08ab42e208de30e42cd9889d8f1735a6e
echo "artifacts ok"
