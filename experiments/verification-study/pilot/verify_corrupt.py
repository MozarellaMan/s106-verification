#!/usr/bin/env python3
"""Assertions for the outputs of corrupt.py.  python3 -I verify_corrupt.py"""
import json
from pathlib import Path

DATA = Path(__file__).resolve().parent / "data" / "contractnli"
load = lambda n: [json.loads(l) for l in (DATA / n).read_text(encoding="utf-8").splitlines() if l.strip()]
orig, ocr, seeded = load("sample40.jsonl"), load("sample40_ocr.jsonl"), load("sample40_seeded.jsonl")
assert len(orig) == len(ocr) == len(seeded) == 40


def lev(a, b):
    prev = list(range(len(b) + 1))
    for i, ca in enumerate(a, 1):
        cur = [i]
        for j, cb in enumerate(b, 1):
            cur.append(min(prev[j] + 1, cur[j - 1] + 1, prev[j - 1] + (ca != cb)))
        prev = cur
    return prev[-1]


# ---- OCR variant
span_err = span_len = nspans = 0
for o, r in zip(orig, ocr):
    for k in o:
        if k not in ("document_text", "gold_evidence_spans"):
            assert o[k] == r[k], (o["item_id"], k)
    assert 0.0095 <= r["cer"] <= 0.031, (r["item_id"], r["cer"])
    assert len(o["gold_evidence_spans"]) == len(r["gold_evidence_spans"])
    for (s0, e0, t0), (s, e, t) in zip(o["gold_evidence_spans"], r["gold_evidence_spans"]):
        assert r["document_text"][s:e] == t, (r["item_id"], s, e)
        d = lev(t0, t)
        assert d <= max(3, 0.08 * len(t0)), (r["item_id"], d, len(t0))   # corrupted, but the same span
        span_err, span_len, nspans = span_err + d, span_len + len(t0), nspans + 1
print(f"OCR ok: {nspans} spans slice correctly; span-level Levenshtein CER = {span_err/span_len:.4f}")

# ---- seeded variant
n_edit = n_null = 0
for o, r in zip(orig, seeded):
    se, iid = r["seed_edit"], o["item_id"]
    if se is None:
        n_null += 1
        assert {k: v for k, v in r.items() if k != "seed_edit"} == o, iid
        continue
    n_edit += 1
    off, old, new = se["char_offset"], se["original_text"], se["new_text"]
    od, nd = o["document_text"], r["document_text"]
    assert od[off:off + len(old)] == old and nd == od[:off] + new + od[off + len(old):], iid
    assert se["old_gold_label"] == o["gold_label"] and se["new_gold_label"] == r["gold_label"], iid
    assert r["hypothesis"] == o["hypothesis"], iid
    for s, e, t in r["gold_evidence_spans"]:
        assert nd[s:e] == t, (iid, s, e)
    if o["gold_label"] == "NotMentioned":
        assert se["kind"] == "sentence_insert" and r["gold_label"] == "Entailment"
        (s, e, t), = r["gold_evidence_spans"]
        assert " " + t == new and s == off + 1, iid
    else:
        assert len(r["gold_evidence_spans"]) == len(o["gold_evidence_spans"])
        si = se["span_index"]
        for i, ((s0, e0, t0), (s, e, t)) in enumerate(zip(o["gold_evidence_spans"], r["gold_evidence_spans"])):
            if i == si:
                rel = off - s0
                assert 0 <= rel and rel + len(old) <= len(t0), iid
                assert t == t0[:rel] + new + t0[rel + len(old):], iid   # edit sits inside this span
            else:
                assert t == t0, (iid, i)                                 # other spans untouched
        if se["kind"] == "number_change":
            assert r["gold_label"] == o["gold_label"] and "new_fact" in se
        else:
            assert r["gold_label"] != o["gold_label"], iid
print(f"seeded ok: {n_edit} edited, {n_null} unchanged; every span slices to its expected text")
