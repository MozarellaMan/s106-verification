"""Export the Python pipeline's page text in the spike's JSON shape, for `pdfspike score`.

    ../../.venv/bin/python python_baseline.py

Southwark: RapidOCR text from the harness's OCR cache (s106-audit/ocr.py; it keeps no coordinates).
Salford: pdfplumber extract_text, as ocr.py uses it for text-layer pages.
"""
import json
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "s106-audit"))
import ocr  # noqa: E402
import pdfplumber  # noqa: E402


def doc(pdf: Path, backend: str, pages: dict[int, str], millis: dict[int, int]) -> dict:
    return {"source": str(pdf), "sha256": ocr.pdf_sha256(pdf), "backend": backend,
            "pages": [{"page": p, "width": 0, "height": 0, "millis": millis.get(p, 0),
                       "lines": [{"text": ln, "bbox": [0, 0, 0, 0], "words": []} for ln in t.splitlines()]}
                      for p, t in pages.items()]}


out = Path(__file__).parent / "out"
out.mkdir(exist_ok=True)
sw = ROOT / "data/raw/southwark_s106_sample.pdf"
texts = ocr.page_texts(sw, pages=[4, 5, 12, 14, 89, 91])
(out / "southwark_python_rapidocr.json").write_text(json.dumps(doc(
    sw, "python baseline: pypdfium2 200dpi + RapidOCR PP-OCRv4 (cached text, no coordinates)", texts,
    {p: 6000 for p in texts}), indent=1))  # ~6 s/page measured single-process; cache reads are instant
sal = ROOT / "data/raw/eval/salford-75239.pdf"
pages, ms = {}, {}
with pdfplumber.open(sal) as d:
    for p in (3, 8):
        t0 = time.time()
        pages[p] = d.pages[p - 1].extract_text() or ""
        ms[p] = int((time.time() - t0) * 1000)
(out / "salford_python_pdfplumber.json").write_text(json.dumps(doc(sal, "python baseline: pdfplumber 0.11", pages, ms), indent=1))
print("wrote out/southwark_python_rapidocr.json, out/salford_python_pdfplumber.json")
