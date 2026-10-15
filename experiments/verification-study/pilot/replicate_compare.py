"""Compares the subagent re-reading of test-00..test-09 with the original API reading of the same items.

    python3 replicate_compare.py [--root PILOT_FOLDER]

Original records: runs/test/<condition>/<model>/<item>.json (client api).
Re-read records:  replicate/runs/test/<condition>/<model>/<item>.json (client subagent).
Readers compared: C1 Haiku, C3 Sonnet, C3 Opus. Labels come from run.parsed, as in analyse.py.

Writes results/test/replicate.csv (item,reader,gold,api_label,subagent_label,same,api_correct,
subagent_correct) and prints how many pairs agree, per reader and overall, and which items differ.
"""
import argparse, csv, sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))  # so run.py imports when the script is run from another folder

from analyse import client
from run import HAIKU, OPUS, SONNET, load, parsed

ITEMS = [f"test-{i:02d}" for i in range(10)]
# (reader name, condition, model) for each replicated reading
READINGS = [("haiku", "C1", HAIKU), ("sonnet", "C3", SONNET), ("opus", "C3", OPUS)]
COLS = ["item", "reader", "gold", "api_label", "subagent_label", "same", "api_correct", "subagent_correct"]


def label(rec):
    """The record's label, or "" when it has no usable answer."""
    a = parsed(rec)
    return a.get("label", "") if a else ""


def gold_labels(results):
    with open(results / "test" / "items.csv") as f:
        return {r["item_id"]: r["gold"] for r in csv.DictReader(f)}


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--root", type=Path, default=HERE, help="the pilot folder (default: this script's folder)")
    root = ap.parse_args().root.resolve()
    api_runs, sub_runs, results = root / "runs", root / "replicate" / "runs", root / "results"
    gold = gold_labels(results)

    rows = []
    for reader, cond, model in READINGS:
        for iid in ITEMS:
            api, sub = load(api_runs, "test", cond, model, iid), load(sub_runs, "test", cond, model, iid)
            if api is None or sub is None:
                print(f"missing record: {cond} {model} {iid} ({'api' if api is None else 'subagent'})")
                continue
            if client(api) != "api" or client(sub) != "subagent":
                print(f"unexpected client: {cond} {model} {iid} api={client(api)} subagent={client(sub)}")
            a, s = label(api), label(sub)
            rows.append({"item": iid, "reader": reader, "gold": gold[iid], "api_label": a, "subagent_label": s,
                         "same": a == s, "api_correct": a == gold[iid], "subagent_correct": s == gold[iid]})

    out = results / "test" / "replicate.csv"
    with open(out, "w", newline="") as f:
        w = csv.DictWriter(f, COLS)
        w.writeheader()
        w.writerows(rows)

    same = sum(r["same"] for r in rows)
    print(f"wrote {out}")
    print(f"pairs compared: {len(rows)}, same label: {same}/{len(rows)}")
    for reader, _, _ in READINGS:
        mine = [r for r in rows if r["reader"] == reader]
        print(f"  {reader}: {sum(r['same'] for r in mine)}/{len(mine)}")
    diff = [r for r in rows if not r["same"]]
    print("differ:" if diff else "differ: none")
    for r in diff:
        print(f"  {r['reader']} {r['item']}: api {r['api_label'] or '(none)'}, "
              f"subagent {r['subagent_label'] or '(none)'}, gold {r['gold']}")


if __name__ == "__main__":
    main()
