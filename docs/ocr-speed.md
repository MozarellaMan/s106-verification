# OCR speed (October 2026)

Measured in a 4-vCPU Linux container, release build, cold page-text cache, on 15 real deeds
that have committed page-text snapshots and recorded extractions (about 530 scanned pages).

## Where the time goes

Per scanned A4 page at 200 dpi (1654x2339) with the default `ppocrv5-en-mobile` set, about 2 s:

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
| `ppocrv5-en-mobile` (default) | 23.6 s | 20/20 | 8/9 | 6/6 |
| `ppocrv6-small` | 20.4 s | 20/20 | 8/9 | 2/6 |
| `ppocrv6-tiny` | 9.3 s | 19/20 | 8/9 | 5/6 |

`ppocrv6-tiny` over all 15 deeds: 486 s against 1284 s for the default (2.6x). Its text differs
from the default's on most pages, in both directions (each model misreads characters the other
gets right). Replaying the recorded extractions on its page text instead of the default's, out of
488 checks: 15 move from REVIEW (cited text not re-locatable) to PASS, 11 move from PASS to
REVIEW, and 3 trigger checks on one page fail because the tiny model read "Implementation" as
"lmplementation". No figure that was found changed value.

`S106_OCR_MODELS=ppocrv6-tiny` selects it (fetch it with `scripts/fetch-models.sh --bench`). It
keeps its own cache key, so its page text never mixes with the default's, and it does not match
the committed snapshots. Making it the default would mean regenerating every snapshot and
re-blessing the replay outcomes; the gold set is too small to show the accuracy cost precisely.
