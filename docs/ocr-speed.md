# OCR speed (October 2026)

Measured in a 4-vCPU Linux container, release build, cold page-text cache, on 15 real deeds
that have committed page-text snapshots and recorded extractions (about 530 scanned pages).

## Where the time goes

Per scanned A4 page at 200 dpi (1654x2339) with `ppocrv5-en-mobile` (the default until October 2026), about 2 s:

| Stage | Time |
|---|---|
| Text detection | ~0.35 s |
| Cropping | ~0.03 s |
| Text recognition | ~1.6 s |

Recognition dominates because a full-width text line is a long crop (width/height up to ~30), and
its cost grows with that width.

## Settings that do not help (text unchanged or worse speed)

| Tried | Result |
|---|---|
| ONNX Runtime graph optimisation level 2 or 3 instead of oar-ocr's level 1 | same time (150 s vs 154 s on a 58-page deed), identical text |
| Recognition batch size 1, 8, 16 or 32 instead of 4 | all slower (28.8, 26.5, 31.9, 39.3 s vs 23.6 s on 12 pages), and the text changes |
| Several pages at once (`S106_OCR_WORKERS`) | see `architecture.md`: faster on 4 cores, slower on 12 |

## Model sets

`s106 bench-ocr` on the gold pages, and the full 15-deed run:

| Model set | 12 scanned pages | Gold lines exact | Gold facts | Printed page numbers |
|---|---|---|---|---|
| `ppocrv5-en-mobile` (default until October 2026) | 23.6 s | 20/20 | 8/9 | 6/6 |
| `ppocrv6-small` | 20.4 s | 20/20 | 8/9 | 2/6 |
| `ppocrv6-tiny` | 9.3 s | 19/20 | 8/9 | 5/6 |

`ppocrv6-tiny` over all 15 deeds: 486 s against 1284 s for `ppocrv5-en-mobile` (2.6x). Its text differs
from PP-OCRv5's on most pages, in both directions (each model misreads characters the other
gets right). Replaying the recorded extractions on its page text instead of PP-OCRv5's, out of
488 checks: 15 move from REVIEW (cited text not re-locatable) to PASS, 11 move from PASS to
REVIEW, and 3 trigger checks on one page fail because the tiny model read "Implementation" as
"lmplementation". No figure that was found changed value.

`ppocrv6-tiny` is the default since October 2026; `S106_OCR_MODELS=ppocrv5-en-mobile` reads with
the old set. The snapshots committed before the switch were not regenerated: they keep their
PP-OCRv5 text and record that model set, and are checked against PDFs read with it, so citations
made from them (pages, quotes, highlight boxes) stay valid. New snapshots are made with tiny. It
meets the gold thresholds (19/20 lines exact, 8/9 facts, 5/6 printed page numbers on the scanned
gold deed), but the gold set is too small to show the accuracy cost precisely.
