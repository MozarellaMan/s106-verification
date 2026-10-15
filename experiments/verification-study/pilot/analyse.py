"""Reads runs/<run>/ and writes results/<run>/{metrics,items,votes,readers}.csv.

    python3 analyse.py --split dev [--dry-run]
    python3 analyse.py --split test --dataset seeded [--dry-run]     # runs/test-seeded -> results/test-seeded

Every figure in the report comes from these files. Conditions are built from the saved primitive
calls exactly as run.py makes them; an item whose calls are not all on disk is left out (n shows it).
"""
import argparse, csv, json, math, re
from collections import Counter

from run import (DATASETS, HAIKU, LABELS, OPUS, READERS, SONNET, c4_flag, checked, load, load_items, parsed,
                 paths, run_name)

CONDS = ["C0", "C1", "C2", "C3", "C4", "C5"]
SHORT = {HAIKU: "haiku", SONNET: "sonnet", OPUS: "opus"}


def wilson(k, n, z=1.96):
    if n == 0:
        return "", ""
    p = k / n
    centre, half = p + z * z / (2 * n), z * math.sqrt(p * (1 - p) / n + z * z / (4 * n * n))
    return (round(max(0.0, (centre - half) / (1 + z * z / n)), 4),
            round(min(1.0, (centre + half) / (1 + z * z / n)), 4))


def ece(rows, bins=5):
    total = 0.0
    for b in range(bins):
        inb = [r for r in rows if min(int(r["confidence"] / 100 * bins), bins - 1) == b]
        if inb:
            acc = sum(r["correct"] for r in inb) / len(inb)
            conf = sum(r["confidence"] for r in inb) / len(inb) / 100
            total += abs(acc - conf) * len(inb)
    return round(total / len(rows), 4) if rows else ""


def majority(readings, of=3):
    """Majority label and its confidence: mean confidence of the readers voting for it times their
    share of all `of` readers (a reader with no usable answer still counts in the share). A split
    with no label held by two readers abstains (NotMentioned, confidence 0)."""
    label, k = Counter(r["label"] for r in readings).most_common(1)[0]
    if k == 1:
        return "NotMentioned", 0
    confs = [r["confidence"] for r in readings if r["label"] == label]
    return label, round(sum(confs) / len(confs) * k / of)


def outcome(cond, item, runs, split):
    """(label, confidence, flagged, readings used, calls used) for one item, or None if incomplete."""
    iid = item["item_id"]
    get = lambda d, m: load(runs, split, d, m, iid)
    if cond == "C0":
        rec = get("C0", HAIKU)
        a = parsed(rec)
        if rec is None:
            return None
        if a is None or a.get("label") not in LABELS:
            return "", 0, True, {"c0": None}, [rec]
        return a["label"], a["confidence"], False, {"c0": a}, [rec]
    h_rec = get("C1", HAIKU)
    if h_rec is None:
        return None
    h = checked(h_rec, item)
    if cond in ("C1", "C2"):
        failed = h is None or h["check"]["status"] == "fail"
        lab, conf = (h["label"], h["confidence"]) if h else ("", 0)
        if cond == "C1":
            return lab, conf, failed, {"haiku": h}, [h_rec]
        needs_check = h is not None and h["label"] != "NotMentioned" and h["quotes"]
        e_rec = get("C2", SONNET)
        if needs_check and e_rec is None:
            return None
        e = parsed(e_rec) if needs_check else None
        unsupported = needs_check and (e is None or not e.get("supports"))
        return lab, conf, failed or unsupported, {"haiku": h, "entail": e}, [h_rec] + ([e_rec] if needs_check else [])
    recs = [h_rec, get("C3", SONNET), get("C3", OPUS)]
    if None in recs:
        return None
    readings = [h] + [checked(r, item) for r in recs[1:]]
    used = dict(zip(["haiku", "sonnet", "opus"], readings))
    ok = [r for r in readings if r is not None]
    label, conf = majority(ok) if ok else ("", 0)
    flag = c4_flag(readings)
    if cond == "C3" or (cond == "C4" and not flag):
        return label, conf, flag if cond == "C3" else False, used, recs
    a_rec = get("C4" if flag else "C5", OPUS)
    if a_rec is None:
        return None
    a = checked(a_rec, item)
    used["adj"] = a
    if a is None:
        return "", 0, True, used, recs + [a_rec]
    return a["label"], a["confidence"], a["check"]["status"] == "fail", used, recs + [a_rec]


def qstatus(r):
    if r is None:
        return "no answer"
    if "check" not in r:
        return ""
    return r["check"]["status"] + (": " + " | ".join(
        c["status"] + (f"({c['dist']})" if c["status"] == "near" else "") for c in r["check"]["quotes"])
        if r["check"]["quotes"] else "")


QUOTING = ("haiku", "sonnet", "opus", "adj")  # readings that carry quotes (not c0, not entail)


def edit_region(seed):
    """(start, end) of the changed characters in the seeded document: new_text minus the prefix and
    suffix it shares with original_text, trimmed of whitespace. start == end for a pure deletion."""
    o, n = seed["original_text"], seed["new_text"]
    p = 0
    while p < min(len(o), len(n)) and o[p] == n[p]:
        p += 1
    q = 0
    while q < min(len(o), len(n)) - p and o[len(o) - 1 - q] == n[len(n) - 1 - q]:
        q += 1
    s, e = seed["char_offset"] + p, seed["char_offset"] + len(n) - q
    while s < e and n[s - seed["char_offset"]].isspace():
        s += 1
    while e > s and n[e - 1 - seed["char_offset"]].isspace():
        e -= 1
    return s, e


def quotes_edit(x, region):
    """A located quote (exact or near) overlaps the changed characters; for a deletion, spans the cut."""
    s, e = region
    for c in x["check"]["quotes"]:
        if c["start"] is None:
            continue
        if (c["start"] < e and c["end"] > s) if e > s else (c["start"] < s < c["end"]):
            return True
    return False


def number_tokens(seed):
    """Words of the number phrase only in new_text, and only in original_text: 'twenty-four', '(24)'."""
    words = lambda t: {w.strip("().,;").lower() for w in t.split()} - {""}
    o, n = words(seed["original_text"]), words(seed["new_text"])
    return n - o, o - n


def has_word(x, toks):
    text = " ".join(c["quote"] for c in x["check"]["quotes"]).lower()
    return any(re.search(r"(?<![\w-])" + re.escape(t) + r"(?![\w-])", text) for t in toks)


def clean_exact_rate(path, cond):
    """The clean run's share of E/C readings whose quote check passed, rebuilt from its items.csv."""
    k = n = 0
    with open(path) as f:
        for r in csv.DictReader(f):
            if r["condition"] != cond:
                continue
            for s in QUOTING:
                if r.get(f"{s}_label") in ("Entailment", "Contradiction"):
                    n += 1
                    k += r.get(f"{s}_quotes", "").startswith("pass")
    return (round(k / n, 4) if n else ""), n


def rate(row, name, k, n):
    row[name] = round(k / n, 4) if n else ""
    row[name + "_k"], row[name + "_n"] = k, n
    row[name + "_lo"], row[name + "_hi"] = wilson(k, n)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--split", choices=["dev", "test"], required=True)
    ap.add_argument("--dataset", choices=DATASETS, default="clean", help="test variant (only with --split test)")
    ap.add_argument("--dry-run", action="store_true")
    a = ap.parse_args()
    name = run_name(a.split, a.dataset)
    runs, results = paths(a.dry_run)
    out = results / name
    for rec_path in (runs / name).glob("C*/*/*.json"):  # never mix fake and live records
        if bool(json.loads(rec_path.read_text()).get("dry_run")) != a.dry_run:
            raise SystemExit(f"{rec_path}: dry_run flag does not match --dry-run={a.dry_run}; refusing.")
    out.mkdir(parents=True, exist_ok=True)
    items = load_items(a.split, a.dataset)
    clean_items = results / a.split / "items.csv"
    metrics, item_rows = [], []
    for cond in CONDS:
        rows = []
        for it in items:
            o = outcome(cond, it, runs, name)
            if o is None:
                continue
            label, conf, flagged, used, calls = o
            readers = [used[s] for s in ("haiku", "sonnet", "opus") if s in used and cond >= "C3"]
            rows.append({"item": it, "label": label, "confidence": conf, "flagged": flagged, "used": used,
                         "calls": calls, "correct": label == it["gold_label"], "readers": readers})
        if not rows:
            continue
        missing = [it["item_id"] for it in items if it["item_id"] not in {r["item"]["item_id"] for r in rows}]
        if missing:
            print(f"WARNING {cond}: {len(missing)} of {len(items)} items incomplete, left out: {' '.join(missing)}")
        n, gold = len(rows), lambda r: r["item"]["gold_label"]
        row = {"condition": cond, "dry_run": a.dry_run, "n": n, "n_incomplete": len(missing)}
        rate(row, "accuracy", sum(r["correct"] for r in rows), n)
        rate(row, "false_conf_high", sum(r["confidence"] >= 90 and not r["correct"] for r in rows), n)
        rate(row, "false_conf_high_unflagged",
             sum(r["confidence"] >= 90 and not r["correct"] and not r["flagged"] for r in rows), n)
        rate(row, "flag_rate", sum(r["flagged"] for r in rows), n)
        if cond >= "C3":
            full = [r for r in rows if len(r["readers"]) == 3 and None not in r["readers"]]
            unan = [r for r in full if len({x["label"] for x in r["readers"]}) == 1]
            rate(row, "unanimous_wrong", sum(r["label"] == r["readers"][0]["label"] and not r["correct"] for r in unan), n)
            all_wrong = [r for r in full if all(x["label"] != gold(r) for x in r["readers"])]
            rate(row, "all_readers_wrong", len(all_wrong), n)
            rate(row, "all_wrong_same_label", sum(len({x["label"] for x in r["readers"]}) == 1 for r in all_wrong), n)
        not_nm = [r for r in rows if gold(r) != "NotMentioned"]
        nm = [r for r in rows if gold(r) == "NotMentioned"]
        rate(row, "abstain_when_answer", sum(r["label"] == "NotMentioned" for r in not_nm), len(not_nm))
        rate(row, "answer_when_not_mentioned", sum(r["label"] in ("Entailment", "Contradiction") for r in nm), len(nm))
        row["ece_5bin"] = ece(rows)
        quoted = [x for r in rows for k, x in r["used"].items() if k not in ("c0", "entail") and x and x["label"] != "NotMentioned"]
        rate(row, "quote_fail_rate", sum(x["check"]["status"] == "fail" for x in quoted), len(quoted))
        if cond == "C2":
            # readings that passed the quote check and got a usable entailment verdict (a refused or
            # truncated check flags the item but is not a judgement either way)
            passed = [r for r in rows if r["used"]["haiku"] and r["used"]["haiku"]["check"]["status"] == "pass"
                      and r["used"]["entail"] is not None]
            rate(row, "entail_unsupported_on_passed",
                 sum(not r["used"]["entail"].get("supports") for r in passed), len(passed))
        ov = [x["check"] for x in quoted if x["check"]["precision"] is not None]
        row["gold_evidence_precision"] = round(sum(c["precision"] for c in ov) / len(ov), 4) if ov else ""
        row["gold_evidence_recall"] = round(sum(c["recall"] for c in ov) / len(ov), 4) if ov else ""
        rate(row, "gold_evidence_hit", sum(c["precision"] > 0 for c in ov), len(ov))
        if a.dataset == "ocr":
            rate(row, "quote_exact_rate", sum(x["check"]["status"] == "pass" for x in quoted), len(quoted))
            here = row["quote_exact_rate"]
            clean, clean_n = clean_exact_rate(clean_items, cond) if clean_items.exists() else ("", 0)
            row["quote_exact_rate_clean"], row["quote_exact_rate_clean_n"] = clean, clean_n
            row["quote_exact_rate_vs_clean"] = round(here - clean, 4) if here != "" and clean != "" else ""
        if a.dataset == "seeded":
            seeded = [r for r in rows if r["item"].get("seed_edit")]
            row["seeded_items"] = len(seeded)
            rate(row, "follow_rate", sum(r["label"] == r["item"]["seed_edit"]["new_gold_label"] for r in seeded),
                 len(seeded))
            if cond >= "C3":
                for col, key in (("all_three_follow", "new_gold_label"), ("all_three_original", "old_gold_label")):
                    rate(row, col, sum(len(r["readers"]) == 3 and all(x is not None and x["label"] == r["item"]["seed_edit"][key]
                                                                          for x in r["readers"]) for r in seeded), len(seeded))
            sq = [(r, x) for r in seeded for k, x in r["used"].items() if k in QUOTING and x]
            rate(row, "quote_contains_edit", sum(quotes_edit(x, edit_region(r["item"]["seed_edit"])) for r, x in sq), len(sq))
            nq = [(number_tokens(r["item"]["seed_edit"]), x) for r, x in sq
                  if r["item"]["seed_edit"]["kind"] == "number_change" and x["check"]["quotes"]]
            rate(row, "number_quote_new", sum(has_word(x, new) for (new, old), x in nq), len(nq))
            rate(row, "number_quote_old", sum(has_word(x, old) for (new, old), x in nq), len(nq))
        row["mean_latency_ms_per_item"] = round(sum(c["latency_ms"] for r in rows for c in r["calls"]) / n)
        row["input_tokens"] = sum(c["usage"]["input_tokens"] for r in rows for c in r["calls"])
        row["output_tokens"] = sum(c["usage"]["output_tokens"] for r in rows for c in r["calls"])
        row["cost_usd"] = round(sum(c["cost_usd"] for r in rows for c in r["calls"]), 4)
        metrics.append(row)
        for r in rows:
            ir = {"item_id": r["item"]["item_id"], "gold": gold(r), "condition": cond, "label": r["label"],
                  "confidence": r["confidence"], "correct": r["correct"], "flagged": r["flagged"],
                  "hypothesis": r["item"]["hypothesis"]}
            if a.dataset == "seeded":
                ir["seed_edit_kind"] = (r["item"]["seed_edit"] or {}).get("kind", "")
            for k, x in r["used"].items():
                ir[f"{k}_label"] = (x.get("label") if k != "entail" else x.get("supports")) if x else ""
                ir[f"{k}_conf"] = x.get("confidence", "") if x else ""
                ir[f"{k}_quotes"] = qstatus(x) if k != "entail" else ""
                ir[f"{k}_reason"] = x.get("reason", "") if x else ""
            item_rows.append(ir)
    write(out / "metrics.csv", metrics)
    write(out / "items.csv", item_rows)
    votes, readers = [], []
    for m in READERS:
        k = n = 0
        for it in items:
            r = checked(load(runs, name, "C1" if m == HAIKU else "C3", m, it["item_id"]), it)
            if r is None:
                continue
            votes.append({"scheme": it["item_id"], "variable": "V1", "source": "deed", "reader": m, "kind": "llm",
                          "family": m, "status": "found", "value": r["label"]})
            k, n = k + (r["label"] == it["gold_label"]), n + 1
        row = {"reader": m, "dry_run": a.dry_run, "n": n}
        rate(row, "measured_accuracy", k, n)
        readers.append(row)
    write(out / "votes.csv", votes)
    write(out / "readers.csv", readers)
    print(f"wrote {out}/metrics.csv ({len(metrics)} conditions), items.csv, votes.csv, readers.csv")


def write(path, rows):
    cols = []
    for r in rows:
        cols += [c for c in r if c not in cols]
    with open(path, "w", newline="") as f:
        w = csv.DictWriter(f, cols)
        w.writeheader()
        w.writerows(rows)


if __name__ == "__main__":
    main()
