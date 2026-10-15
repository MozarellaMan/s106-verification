"""Subagent bridge: lets Claude Code subagents answer the pilot's pending calls instead of the API.

    python3 bridge.py export --split test [--dataset clean|ocr|seeded] [--conditions C0,...,C5]
    python3 bridge.py ingest --split test [--dataset ...]

export walks the same call plan as run.py (same items, conditions, prompts, schemas, C4 flag rule and
C4/C5 adjudication dirs), skips every call whose record exists, and writes one pending call per file:

    bridge/<run>/pending/<dir>__<model>__<item>.json         record_skeleton (+ the full request)
    bridge/<run>/pending/<dir>__<model>__<item>.prompt.txt   exact user message + the schema trailer
    bridge/<run>/pending/<dir>__<model>__<item>.schema.json  the JSON schema the answer must satisfy
    bridge/<run>/MANIFEST.csv                                one row per call pending now

Calls whose inputs do not exist yet (entailment needs the C1 Haiku reading; adjudication needs all
three quote readings) are not written; export prints how many are blocked. Rerun export after each
ingest until nothing is pending or blocked.

The parent writes, beside each pending call, <id>.response.json:

    {"text": "<the subagent's JSON answer as a string>", "input_tokens": int|null,
     "output_tokens": int|null, "latency_ms": int|null, "model": "claude-...", "client": "subagent"}

ingest validates each answer against its schema, writes the record in run.py's shape to
runs/<run>/<dir>/<model>/<item>.json (plus "client": "subagent" and "cost_usd_estimated": true),
appends a ledger row (stop_reason "subagent") and moves the call's files to bridge/<run>/done/.
An invalid answer stays in pending/ with <id>.invalid.txt giving the reason.

--root DIR puts runs/ and bridge/ under DIR (default: this folder, or dryrun/ with --dry-run).
--dry-run marks ingested records dry_run=true, for testing in a copy of the dry-run tree.
"""
import argparse, csv, json, shutil, sys
from collections import Counter
from datetime import datetime, timezone
from pathlib import Path

from run import (ENTAIL, HAIKU, HERE, LABELS, LEDGER_COLS, OPUS, READING, READING_C0, SONNET, Runner,
                 build_request, c4_flag, checked, cost, load_items, prompt, record_path, run_name, show,
                 stored_request)

CONDS = {"C0", "C1", "C2", "C3", "C4", "C5"}
MANIFEST_COLS = ["id", "dir", "model", "kind", "item", "prompt_path", "schema_path"]
FAMILY = {HAIKU: "haiku", SONNET: "sonnet", OPUS: "opus"}
# The API enforces the schema through output_config; a subagent only sees text, so the prompt file
# ends with this trailer after the exact user message.
TRAILER = ("\n\n---\nRespond with only the JSON object, with no other text before or after it and no "
           "code fence. It must satisfy this JSON schema:\n{schema}\n")


def call_id(d, model, item_id):
    return f"{d}__{model}__{item_id}"


class Planner:
    """run.py's Runner.item, with each missing call written as a pending file instead of being made."""

    def __init__(self, name, runs, dry_run):
        self.name, self.runs, self.dry_run = name, runs, dry_run
        self.pending, self.existing = [], 0
        self.blocked = Counter()

    def want(self, d, model, kind, item, text, schema):
        path = record_path(self.runs, self.name, d, model, item["item_id"])
        params, request = build_request(model, kind, item, text, schema)
        if path.exists():  # the same checks as Runner.call before it reuses a record
            rec = json.loads(path.read_text())
            if bool(rec.get("dry_run")) != self.dry_run:
                sys.exit(f"{path} has dry_run={rec.get('dry_run')} but this export has dry_run={self.dry_run}.")
            if rec["request"] != request:
                sys.exit(f"{path} was made with a different request (prompt, schema or settings changed); "
                         f"the bridge will not mix versions.")
            self.existing += 1
            return rec
        self.pending.append({"id": call_id(d, model, item["item_id"]), "dir": d, "model": model, "kind": kind,
                             "item": item["item_id"], "params": params, "schema": schema,
                             "skeleton": {"item_id": item["item_id"], "kind": kind, "dir": d,
                                          "dry_run": self.dry_run, "request": params}})
        return None

    def item(self, item, conds):
        """Mirrors Runner.item. Differences: the Sonnet and Opus readers do not wait for the Haiku reading
        (they never depended on it; run.py only stops there after a failed call), and the calls a missing
        input blocks are counted instead of made."""
        doc, hyp = item["document_text"], item["hypothesis"]
        if "C0" in conds:
            self.want("C0", HAIKU, "reader", item, prompt("reader_c0", DOCUMENT=doc, HYPOTHESIS=hyp), READING_C0)
        if not conds & {"C1", "C2", "C3", "C4", "C5"}:
            return
        text = prompt("reader_quotes", DOCUMENT=doc, HYPOTHESIS=hyp)
        h = self.want("C1", HAIKU, "reader", item, text, READING)
        if "C2" in conds:
            if h is None:
                self.blocked["C2 entailment (only if the Haiku reading has a label and quotes)"] += 1
            else:
                r = checked(h, item)
                if r and r["label"] != "NotMentioned" and r["quotes"]:
                    quotes = "\n".join(f"<quote>{q}</quote>" for q in r["quotes"])
                    self.want("C2", SONNET, "entailment", item,
                              prompt("entailment", HYPOTHESIS=hyp, LABEL=r["label"], QUOTES=quotes), ENTAIL)
        if not conds & {"C3", "C4", "C5"}:
            return
        recs = [h] + [self.want("C3", m, "reader", item, text, READING) for m in (SONNET, OPUS)]
        if None in recs:
            if conds & {"C4", "C5"}:
                self.blocked["C4/C5 adjudication" + ("" if "C5" in conds else " (only if C4 flags)")] += 1
            return
        readings = [checked(r, item) for r in recs]
        flag = c4_flag(readings)
        if ("C4" in conds and flag) or "C5" in conds:
            self.want("C4" if flag else "C5", OPUS, "adjudicate", item,
                      prompt("adjudicator", DOCUMENT=doc, HYPOTHESIS=hyp, READINGS=show(readings)), READING)


def export(a, name, runs, bdir):
    conds = set(a.conditions.split(","))
    if conds - CONDS:
        sys.exit(f"unknown condition in {a.conditions}")
    plan = Planner(name, runs, a.dry_run)
    for it in load_items(a.split, a.dataset):
        plan.item(it, conds)
    pend = bdir / "pending"
    pend.mkdir(parents=True, exist_ok=True)
    rows = []
    for p in plan.pending:
        base = pend / p["id"]
        schema_text = json.dumps(p["schema"], indent=1)
        Path(f"{base}.json").write_text(json.dumps({"id": p["id"], "record_skeleton": p["skeleton"]}, indent=1))
        Path(f"{base}.prompt.txt").write_text(p["params"]["messages"][0]["content"] + TRAILER.format(schema=schema_text))
        Path(f"{base}.schema.json").write_text(schema_text + "\n")
        rows.append({"id": p["id"], "dir": p["dir"], "model": p["model"], "kind": p["kind"], "item": p["item"],
                     "prompt_path": f"{base}.prompt.txt", "schema_path": f"{base}.schema.json"})
    with open(bdir / "MANIFEST.csv", "w", newline="") as f:
        w = csv.DictWriter(f, MANIFEST_COLS)
        w.writeheader()
        w.writerows(rows)
    print(f"{name}: {plan.existing} calls already recorded; {len(rows)} pending written to {pend}")
    for m, n in sorted(Counter(r["model"] for r in rows).items()):
        kinds = Counter(r["kind"] for r in rows if r["model"] == m)
        print(f"  {m}: {n} ({', '.join(f'{k} {v}' for k, v in sorted(kinds.items()))})")
    print(f"manifest: {bdir / 'MANIFEST.csv'}")
    if plan.blocked:
        print(f"blocked until an earlier wave is ingested: {sum(plan.blocked.values())} calls")
        for k, v in sorted(plan.blocked.items()):
            print(f"  {k}: {v}")
        print("ingest the pending answers, then rerun export.")
    else:
        print("nothing blocked." if rows else "nothing pending or blocked: this run is complete.")


# ------------------------------------------------------------------------------- ingest

class Invalid(Exception):
    pass


def check_type(v, spec, where):
    t = spec.get("type")
    if t == "object":
        if not isinstance(v, dict):
            raise Invalid(f"{where}: expected an object")
        props = spec.get("properties", {})
        missing = [k for k in spec.get("required", []) if k not in v]
        if missing:
            raise Invalid(f"{where}: missing {missing}")
        extra = [k for k in v if k not in props]
        if extra and spec.get("additionalProperties") is False:
            raise Invalid(f"{where}: unexpected keys {extra}")
        for k, sub in props.items():
            if k in v:
                check_type(v[k], sub, f"{where}.{k}")
    elif t == "array":
        if not isinstance(v, list):
            raise Invalid(f"{where}: expected a list")
        for i, x in enumerate(v):
            check_type(x, spec.get("items", {}), f"{where}[{i}]")
    elif t == "string":
        if not isinstance(v, str):
            raise Invalid(f"{where}: expected a string")
    elif t == "integer":
        if isinstance(v, bool) or not isinstance(v, int):
            raise Invalid(f"{where}: expected an integer, got {v!r}")
    elif t == "boolean":
        if not isinstance(v, bool):
            raise Invalid(f"{where}: expected true or false, got {v!r}")
    if "enum" in spec and v not in spec["enum"]:
        raise Invalid(f"{where}: {v!r} is not one of {spec['enum']}")


def answer_text(raw):
    """The JSON object in the subagent's text: surrounding whitespace and one code fence are tolerated."""
    t = raw.strip()
    if t.startswith("```"):
        t = t.split("\n", 1)[1] if "\n" in t else ""
        if t.rstrip().endswith("```"):
            t = t.rstrip()[:-3]
        t = t.strip()
    try:
        return t, json.loads(t)
    except ValueError as e:
        raise Invalid(f"answer is not valid JSON: {e}")


def opt_int(resp, key):
    v = resp.get(key)
    if v is not None and (isinstance(v, bool) or not isinstance(v, int) or v < 0):
        raise Invalid(f"{key} must be a non-negative integer or null, got {v!r}")
    return v


def ingest_one(cid, pend, done, runs, name, items, dry_run):
    sk = json.loads((pend / f"{cid}.json").read_text())["record_skeleton"]
    try:
        resp = json.loads((pend / f"{cid}.response.json").read_text())
    except ValueError as e:
        raise Invalid(f"{cid}.response.json is not valid JSON: {e}")
    if not isinstance(resp, dict) or not isinstance(resp.get("text"), str):
        raise Invalid('response file must be an object with "text" a string')
    if resp.get("client") != "subagent":
        raise Invalid(f'"client" must be "subagent", got {resp.get("client")!r}')
    model = sk["request"]["model"]
    got = resp.get("model")
    if not isinstance(got, str) or not got.startswith("claude-") or FAMILY[model] not in got:
        raise Invalid(f'"model" {got!r} is not a {FAMILY[model]} model (this call is for {model})')
    tin, tout, lat = opt_int(resp, "input_tokens"), opt_int(resp, "output_tokens"), opt_int(resp, "latency_ms")
    text, ans = answer_text(resp["text"])
    check_type(ans, sk["request"]["output_config"]["format"]["schema"], "answer")
    if "confidence" in ans and not 0 <= ans["confidence"] <= 100:
        print(f"  note {cid}: confidence {ans['confidence']} is outside 0-100; clamped when read, as for the API")
    item = items[sk["item_id"]]
    path = record_path(runs, name, sk["dir"], model, sk["item_id"])
    if path.exists():
        raise Invalid(f"{path} already exists; not overwritten")
    usage = {"input_tokens": tin, "output_tokens": tout}
    c = cost(model, usage) if tin is not None and tout is not None else None
    now = datetime.now(timezone.utc).isoformat(timespec="seconds")
    rec = {"item_id": sk["item_id"], "kind": sk["kind"], "dir": sk["dir"], "dry_run": dry_run,
           "request": stored_request(sk["request"], item),
           "response": {"id": f"subagent-{cid}", "type": "message", "role": "assistant", "model": got,
                        "content": [{"type": "text", "text": text}], "stop_reason": "end_turn",
                        "stop_sequence": None, "usage": usage},
           "usage": usage, "latency_ms": lat, "timestamp": now, "cost_usd": c,
           "client": "subagent", "cost_usd_estimated": True}
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp = path.with_suffix(".tmp")
    tmp.write_text(json.dumps(rec, indent=1))
    tmp.replace(path)
    Runner._append(runs / name / "ledger.csv", LEDGER_COLS, {
        "timestamp": now, "split": name, "dir": sk["dir"], "model": model, "item_id": sk["item_id"],
        "kind": sk["kind"], "input_tokens": "" if tin is None else tin, "output_tokens": "" if tout is None else tout,
        "cost_usd": f"{c or 0:.6f}", "dry_run": dry_run, "stop_reason": "subagent"})
    done.mkdir(parents=True, exist_ok=True)
    for suffix in (".json", ".prompt.txt", ".schema.json", ".response.json", ".invalid.txt"):
        f = pend / f"{cid}{suffix}"
        if f.exists():
            shutil.move(str(f), str(done / f.name))
    return c


def ingest(a, name, runs, bdir):
    pend, done = bdir / "pending", bdir / "done"
    items = {it["item_id"]: it for it in load_items(a.split, a.dataset)}
    ok, bad, spent = 0, 0, 0.0
    for rp in sorted(pend.glob("*.response.json")):
        cid = rp.name[:-len(".response.json")]
        try:
            if not (pend / f"{cid}.json").exists():
                raise Invalid(f"no pending call {cid}.json beside this response")
            spent += ingest_one(cid, pend, done, runs, name, items, a.dry_run) or 0.0
            ok += 1
        except Invalid as e:
            bad += 1
            (pend / f"{cid}.invalid.txt").write_text(f"{e}\n")
            print(f"INVALID {cid}: {e}", file=sys.stderr)
    left = len(list(pend.glob("*.prompt.txt")))
    print(f"{name}: {ok} ingested (estimated ${spent:.4f}), {bad} invalid left in {pend}; "
          f"{left} pending calls still in pending/. Rerun export for the next wave.")
    if bad:
        sys.exit(1)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("action", choices=["export", "ingest"])
    ap.add_argument("--split", choices=["dev", "test"], required=True)
    ap.add_argument("--dataset", choices=["clean", "ocr", "seeded"], default="clean")
    ap.add_argument("--conditions", default="C0,C1,C2,C3,C4,C5")
    ap.add_argument("--dry-run", action="store_true", help="records are dry_run=true; default root dryrun/")
    ap.add_argument("--root", default=None, help="folder holding runs/ and bridge/")
    a = ap.parse_args()
    name = run_name(a.split, a.dataset)
    root = Path(a.root).resolve() if a.root else (HERE / "dryrun" if a.dry_run else HERE)
    runs, bdir = root / "runs", root / "bridge" / name
    (export if a.action == "export" else ingest)(a, name, runs, bdir)


if __name__ == "__main__":
    main()
