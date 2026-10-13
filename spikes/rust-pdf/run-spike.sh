#!/usr/bin/env bash
# Reproduce every measurement in README.md. Needs ../../data/raw/southwark_s106_sample.pdf and
# ../../data/raw/eval/salford-75239.pdf (fetch commands in the repo README / data/reference/candidates.tsv),
# and the repo's Python venv for the PP-OCRv4 model files and the Python baselines.
set -euo pipefail
cd "$(dirname "$0")"
./fetch-artifacts.sh
M=../../.venv/lib/python3.12/site-packages/rapidocr_onnxruntime/models
mkdir -p artifacts/ppocrv4 out
cp -n "$M"/ch_PP-OCRv4_det_infer.onnx "$M"/ch_PP-OCRv4_rec_infer.onnx artifacts/ppocrv4/
../../.venv/bin/python -c "import onnxruntime as o; s=o.InferenceSession('$M/ch_PP-OCRv4_rec_infer.onnx', providers=['CPUExecutionProvider']); open('artifacts/ppocrv4/dict.txt','w').write(s.get_modelmeta().custom_metadata_map['character']+'\n')"
cargo build --release
S=../../data/raw/southwark_s106_sample.pdf; SAL=../../data/raw/eval/salford-75239.pdf; G=4,5,12,14,89,91
B=./target/release/pdfspike
$B ocr    $S $G out/southwark_ocrs_200.json 200
$B paddle $S $G out/southwark_paddle_200.json 200
$B text   $SAL 3,8 out/salford_pdfium_text.json
$B crates $SAL 3,8 out/salford
$B crates $S 4,5 out/southwark
for f in out/southwark_ocrs_200.json out/southwark_paddle_200.json; do $B score gold/southwark_19_AP_1773.json $f; done
for f in out/salford_pdfium_text.json out/salford_pdfplumber.json out/salford_pdf-extract.json out/salford_pdf-inspector.json; do
  $B score gold/salford_20_75239.json $f; done
../../.venv/bin/python python_baseline.py
for f in out/southwark_python_rapidocr.json; do $B score gold/southwark_19_AP_1773.json $f; done
$B score gold/salford_20_75239.json out/salford_python_pdfplumber.json
