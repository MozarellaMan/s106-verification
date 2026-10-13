#!/usr/bin/env bash
# Fetch the runtime artifacts into ./models (gitignored) and verify each against a pinned SHA-256.
#   PDFium:  bblanchon/pdfium-binaries chromium/8100 (renders pages, reads text layers)
#   OCR:     PP-OCRv5 mobile detection + English recognition and dictionary, published by oar-ocr
#            (PaddleOCR models, Apache-2.0). The English dictionary includes "£".
# Pass --bench to also fetch the alternative model sets compared in `s106 bench-ocr`.
set -euo pipefail
cd "$(dirname "$0")/.."
mkdir -p models
O=https://github.com/GreatV/oar-ocr/releases/download
fetch() { # url file sha256
  local f="models/$2"
  [ -f "$f" ] || curl -sSfL -o "$f" "$1"
  echo "$3  $f" | sha256sum -c --quiet || { echo "checksum mismatch: $f" >&2; rm -f "$f"; exit 1; }
}
fetch https://github.com/bblanchon/pdfium-binaries/releases/download/chromium/8100/pdfium-linux-x64.tgz pdfium-linux-x64.tgz \
      1c1894a12d4d96ee01400c3874444126254180c5c7d2ab628576b67f8120320a
[ -f models/pdfium/lib/libpdfium.so ] || { mkdir -p models/pdfium && tar -xzf models/pdfium-linux-x64.tgz -C models/pdfium; }
fetch $O/v0.3.0/pp-ocrv5_mobile_det.onnx    pp-ocrv5_mobile_det.onnx    1eb7b4f7ab657ebd1c66d5f79bca7497f29768a2e3c15e52daecbba1a8e4a039
fetch $O/v0.3.0/en_pp-ocrv5_mobile_rec.onnx en_pp-ocrv5_mobile_rec.onnx 8307465d3c9ef2ba4055c3bd0be55aafe11f518630212b7598b70ccb376028ac
fetch $O/v0.3.0/ppocrv5_en_dict.txt         ppocrv5_en_dict.txt         e025a66d31f327ba0c232e03f407ae8d105e1e709e7ccb3f408aa778c24e70d6
if [ "${1:-}" = "--bench" ]; then
  fetch $O/v0.3.0/pp-ocrv4_mobile_det.onnx       pp-ocrv4_mobile_det.onnx       ab2a50dcd2c340852f2d0fbfa547d5eec79a0d04a774eb0b622d96d0d9d2ceeb
  fetch $O/v0.3.0/pp-ocrv4_mobile_rec.onnx       pp-ocrv4_mobile_rec.onnx       5d54b59bac0f49d4561f0462630d8a6822b5b495db064f58096fd3d2392fbc4e
  fetch $O/v0.3.0/ppocr_keys_v1.txt              ppocr_keys_v1.txt              a1c84d9bdb9ab29043c58896224d32941783eb821629618416dcb08f12886492
  fetch $O/v0.3.0/latin_pp-ocrv5_mobile_rec.onnx latin_pp-ocrv5_mobile_rec.onnx e3a6bfeea1c8a01d6fccfd480a0bd363fd907f8c65931e228bb2736f5c3e142f
  fetch $O/v0.3.0/ppocrv5_latin_dict.txt         ppocrv5_latin_dict.txt         ccbcc45730b3fbbd9050c5bc74db6a99067141ef1035e3d14889a84a6b9b1aff
  fetch $O/v0.7.0/pp-ocrv6_small_det.onnx        pp-ocrv6_small_det.onnx        d73e0058b7a8086bbd57f3d10b8bcd4ff95363f67e06e2762b5e814fe9c9410e
  fetch $O/v0.7.0/pp-ocrv6_small_rec.onnx        pp-ocrv6_small_rec.onnx        5435fd747c9e0efe15a96d0b378d5bd157e9492ed8fd80edf08f30d02fa24634
  fetch $O/v0.7.0/ppocrv6_dict.txt               ppocrv6_dict.txt               b5f2bfe2bdd9448429e3e82b51c789775d9b42f2403d082b00662eb77e401c5d
fi
echo "models ok"
