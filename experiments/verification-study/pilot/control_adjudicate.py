"""POST-HOC control for the C4 adjudication result (added after the test results were seen; not part
of the pre-registered design).

In the original prompt the three readings are always Haiku, Sonnet, Opus as A, B, C. This control asks
the adjudicator again on the same items with the readings in a random order, to see whether it still
sides with the Opus reading.

    python3 control_adjudicate.py export --split test [--dataset clean|seeded]
    python3 control_adjudicate.py ingest --split test [--dataset clean|seeded]

export finds the items C4 adjudicated in the run (a C4 record exists), rebuilds the adjudicator prompt
as run.py does, with the three readings shuffled by random.Random(f"control|{run}|{item_id}") before
they are labelled A/B/C, and writes, in the bridge's pending shape:

    control/<run>/pending/CTRL__claude-opus-5-5__<item>.prompt.txt   user message + bridge.py's trailer
    control/<run>/pending/CTRL__claude-opus-5-5__<item>.schema.json  the JSON schema of the answer
    control/<run>/pending/CTRL__claude-opus-5-5__<item>.json         skeleton with the request and the
                                                                     permutation (which reader is A/B/C)

The answer goes beside the prompt as <id>.answer.json: the bare JSON object the schema asks for.
ingest validates each answer as bridge.py does and writes results/<run>/control-adjudication.csv.
Nothing here calls the API, and run.py, the prompts and the original records are not changed.
"""
import argparse, csv, json, random, sys
from pathlib import Path

from analyse import majority
from bridge import TRAILER, Invalid, answer_text, check_type
from run import (HAIKU, HERE, OPUS, READING, SONNET, build_request, checked, load, load_items, paths, prompt,
                 record_path, run_name, show)

READERS = [("haiku", "C1", HAIKU), ("sonnet", "C3", SONNET), ("opus", "C3", OPUS)]
CSV_COLS = ["item", "gold", "permutation", "haiku", "sonnet", "opus", "majority", "original_label",
            "original_correct", "original_sided_with", "control_label", "control_correct", "control_sided_with",
            "control_sided_with_letter", "opus_moved"]


def call_id(item_id):
    return f"CTRL__{OPUS}__{item_id}"


def adjudicated(runs, name, items):
    """The items C4 sent to the adjudicator in this run, with their three readings in A/B/C order."""
    out = []
    for it in items:
        if not record_path(runs, name, "C4", OPUS, it["item_id"]).exists():
            continue
        readings = [checked(load(runs, name, d, m, it["item_id"]), it) for _, d, m in READERS]
        out.append((it, readings))
    return out


def adjudicator_text(item, readings):
    return prompt("adjudicator", DOCUMENT=item["document_text"], HYPOTHESIS=item["hypothesis"],
                  READINGS=show(readings))


def export(name, runs, cdir, items):
    pend = cdir / "pending"
    pend.mkdir(parents=True, exist_ok=True)
    n = 0
    for it, readings in adjudicated(runs, name, items):
        # sanity check: the unshuffled prompt must be exactly the one the original C4 call was sent
        original = load(runs, name, "C4", OPUS, it["item_id"])
        _, request = build_request(OPUS, "adjudicate", it, adjudicator_text(it, readings), READING)
        if request != original["request"]:
            sys.exit(f"{it['item_id']}: rebuilt prompt differs from the original C4 request; not exporting.")
        order = [0, 1, 2]
        random.Random(f"control|{name}|{it['item_id']}").shuffle(order)
        shuffled = [readings[i] for i in order]
        permutation = {letter: READERS[i][0] for letter, i in zip("ABC", order)}
        params, _ = build_request(OPUS, "adjudicate", it, adjudicator_text(it, shuffled), READING)
        base = pend / call_id(it["item_id"])
        schema_text = json.dumps(READING, indent=1)
        skeleton = {"item_id": it["item_id"], "kind": "adjudicate", "dir": "CTRL", "post_hoc_control": True,
                    "permutation": permutation, "request": params}
        Path(f"{base}.json").write_text(json.dumps({"id": call_id(it["item_id"]), "record_skeleton": skeleton},
                                                   indent=1))
        Path(f"{base}.prompt.txt").write_text(params["messages"][0]["content"] + TRAILER.format(schema=schema_text))
        Path(f"{base}.schema.json").write_text(schema_text + "\n")
        n += 1
    print(f"{name}: {n} control prompts written to {pend}")


def sided_with(label, labels):
    """The original readers whose label equals the adjudicator's, e.g. 'sonnet+opus', or 'none'."""
    return "+".join(r for r, lab in labels.items() if label and lab == label) or "none"


def ingest(name, runs, cdir, results, items):
    pend = cdir / "pending"
    rows, missing, bad = [], [], 0
    for it, readings in adjudicated(runs, name, items):
        cid = call_id(it["item_id"])
        if not (pend / f"{cid}.json").exists():
            sys.exit(f"{pend / cid}.json is missing: run export first (it does not change existing prompts).")
        sk = json.loads((pend / f"{cid}.json").read_text())["record_skeleton"]
        answer_path = pend / f"{cid}.answer.json"
        if not answer_path.exists():
            missing.append(it["item_id"])
            continue
        try:
            _, ans = answer_text(answer_path.read_text())
            check_type(ans, sk["request"]["output_config"]["format"]["schema"], "answer")
        except Invalid as e:
            bad += 1
            print(f"INVALID {cid}: {e}", file=sys.stderr)
            continue
        if "confidence" in ans and not 0 <= ans["confidence"] <= 100:
            print(f"WARNING {cid}: confidence {ans['confidence']} is outside 0-100", file=sys.stderr)
        labels = {r: (x["label"] if x else "") for (r, _, _), x in zip(READERS, readings)}
        ok = [x for x in readings if x is not None]
        original = checked(load(runs, name, "C4", OPUS, it["item_id"]), it)
        original_label = original["label"] if original else ""
        gold = it["gold_label"]
        rows.append({"item": it["item_id"], "gold": gold,
                     "permutation": " ".join(f"{k}={v}" for k, v in sk["permutation"].items()),
                     **labels, "majority": majority(ok)[0] if ok else "",
                     "original_label": original_label, "original_correct": original_label == gold,
                     "original_sided_with": sided_with(original_label, labels),
                     "control_label": ans["label"], "control_correct": ans["label"] == gold,
                     "control_sided_with": sided_with(ans["label"], labels),
                     "control_sided_with_letter": "+".join(k for k, r in sk["permutation"].items()
                                                           if ans["label"] and labels[r] == ans["label"]) or "none",
                     "opus_moved": sk["permutation"]["C"] != "opus"})
    out = results / name / "control-adjudication.csv"
    with open(out, "w", newline="") as f:
        w = csv.DictWriter(f, CSV_COLS)
        w.writeheader()
        w.writerows(rows)
    # only items where Opus's label differs from the three readers' majority can show a pull towards Opus
    informative = [r for r in rows if r["opus"] != r["majority"]]
    with_opus_orig = sum(r["original_label"] == r["opus"] for r in informative)
    with_opus_ctrl = sum(r["control_label"] == r["opus"] for r in informative)
    print(f"{name}: wrote {out} ({len(rows)} items; {len(missing)} without an answer yet, {bad} invalid)")
    print(f"sided with the Opus reading where Opus differs from the majority: control {with_opus_ctrl} of "
          f"{len(informative)}, original {with_opus_orig} of {len(informative)}")
    print(f"uninformative (Opus agrees with the majority): {len(rows) - len(informative)} items")
    print(f"correct: control {sum(r['control_correct'] for r in rows)}, original {sum(r['original_correct'] for r in rows)}")
    if bad:
        sys.exit(1)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("action", choices=["export", "ingest"])
    ap.add_argument("--split", choices=["dev", "test"], required=True)
    ap.add_argument("--dataset", choices=["clean", "ocr", "seeded"], default="clean")
    a = ap.parse_args()
    name = run_name(a.split, a.dataset)
    runs, results = paths(False)
    cdir = HERE / "control" / name
    items = load_items(a.split, a.dataset)
    if a.action == "export":
        export(name, runs, cdir, items)
    else:
        ingest(name, runs, cdir, results, items)


if __name__ == "__main__":
    main()
