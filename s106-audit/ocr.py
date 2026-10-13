"""Page text acquisition: pdfplumber text layer first, RapidOCR fallback for scans.

Results are cached per page under data/interim/ocr/<pdf-stem>/page_NNN.txt so
repeated audits don't re-OCR the deed.
"""
from __future__ import annotations

import os
import sys
from concurrent.futures import ProcessPoolExecutor
from pathlib import Path

import pdfplumber
import pypdfium2 as pdfium

MIN_TEXT_CHARS = 50  # below this a page is treated as image-only
OCR_SCALE = 200 / 72  # render at 200 dpi
THREADS_PER_WORKER = 2


def _cache_dir(pdf_path: Path) -> Path:
    root = pdf_path.resolve().parent.parent  # data/raw -> data
    d = root / "interim" / "ocr" / pdf_path.stem
    d.mkdir(parents=True, exist_ok=True)
    return d


_engine = None


def _ocr_engine():
    global _engine
    if _engine is None:
        from rapidocr_onnxruntime import RapidOCR
        # Cap ONNX threads per process: the default (-1 = all cores) in every worker
        # oversubscribes the CPU and is slower than a single process.
        _engine = RapidOCR(intra_op_num_threads=THREADS_PER_WORKER, inter_op_num_threads=1, use_cls=False)
    return _engine


def _ocr_page(doc: pdfium.PdfDocument, index: int) -> str:
    import numpy as np

    img = doc[index].render(scale=OCR_SCALE).to_pil().convert("RGB")
    result, _ = _ocr_engine()(np.array(img))
    if not result:
        return ""
    # result: [[box, text, score], ...]; rebuild lines by y-centre then x.
    items = []
    for box, text, _score in result:
        ys = [p[1] for p in box]
        xs = [p[0] for p in box]
        items.append(((min(ys) + max(ys)) / 2, min(xs), max(ys) - min(ys), text))
    items.sort()
    lines, current, line_y = [], [], None
    for y, x, h, text in items:
        if line_y is not None and abs(y - line_y) > max(h * 0.6, 8):
            lines.append(current)
            current = []
        if not current:
            line_y = y
        current.append((x, text))
    if current:
        lines.append(current)
    return "\n".join(" ".join(t for _, t in sorted(line)) for line in lines)


def _ocr_worker(args: tuple[str, int]) -> tuple[int, str]:
    # Each worker process owns its own pdfium handle and ONNX session.
    pdf_path, n = args
    doc = pdfium.PdfDocument(pdf_path)
    return n, _ocr_page(doc, n - 1)


def page_texts(pdf_path: str | Path, pages: list[int] | None = None, verbose: bool = False,
               workers: int | None = None) -> dict[int, str]:
    """Return {1-based page number: text}. Uses cache, text layer, then parallel OCR."""
    pdf_path = Path(pdf_path)
    cache = _cache_dir(pdf_path)
    out: dict[int, str] = {}
    todo: list[int] = []
    with pdfplumber.open(pdf_path) as pdf:
        total = len(pdf.pages)
        for n in pages or range(1, total + 1):
            f = cache / f"page_{n:03d}.txt"
            if f.exists():
                out[n] = f.read_text()
                continue
            text = pdf.pages[n - 1].extract_text() or ""
            if len(text.strip()) < MIN_TEXT_CHARS:
                todo.append(n)
            else:
                f.write_text(text)
                out[n] = text
    if todo:
        workers = workers or max(1, (os.cpu_count() or 2) // THREADS_PER_WORKER)
        if verbose:
            print(f"  OCR: {len(todo)} image-only pages on {workers} workers", file=sys.stderr)
        with ProcessPoolExecutor(max_workers=workers) as pool:
            for n, text in pool.map(_ocr_worker, [(str(pdf_path), n) for n in todo]):
                (cache / f"page_{n:03d}.txt").write_text(text)
                out[n] = text
                if verbose:
                    print(f"  page {n}/{total}: {len(text)} chars", file=sys.stderr)
    return dict(sorted(out.items()))


if __name__ == "__main__":
    page_texts(sys.argv[1], verbose=True)
