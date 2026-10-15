"""Pilot runner: makes the primitive calls the conditions C0-C5 need, once each, and logs their cost.

    python3 run.py --split dev --conditions C0,C1,C2,C3,C4,C5 --dry-run
    python3 run.py --split dev --conditions C0,C1 --cap 3.00          # live: needs a key and your go
    python3 run.py --split test --dataset seeded --dry-run            # damaged variant -> runs/test-seeded/

Primitive calls (each saved to runs/<run>/<dir>/<model>/<item_id>.json and never re-requested; <run> is the
split, or test-ocr / test-seeded for the damaged test variants):
    reader(haiku, no quotes)       -> C0/claude-haiku-5-5           (C0)
    reader(haiku, quotes)          -> C1/claude-haiku-5-5           (C1, C2, C3-C5)
    reader(sonnet|opus, quotes)    -> C3/claude-sonnet-5-5, C3/claude-opus-5-5   (C3-C5)
    entailment(sonnet) of the Haiku quote reading -> C2/claude-sonnet-5-5        (C2)
    adjudicate(opus) on items C4 flags -> C4/claude-opus-5-5; on the rest (C5 only) -> C5/claude-opus-5-5
"""
import argparse, csv, hashlib, json, os, random, sys, threading, time
from concurrent.futures import ThreadPoolExecutor
from datetime import datetime, timezone
from pathlib import Path

import quote_check

HERE = Path(__file__).resolve().parent
DATA = HERE / "data" / "contractnli"
SPLITS = {"dev": "dev10.jsonl", "test": "sample40.jsonl"}
# damaged variants of the test split (data/contractnli/CORRUPTION.md); only valid with --split test
DATASETS = {"clean": None, "ocr": "sample40_ocr.jsonl", "seeded": "sample40_seeded.jsonl"}
HAIKU, SONNET, OPUS = "claude-haiku-5-5", "claude-sonnet-5-5", "claude-opus-5-5"
READERS = [HAIKU, SONNET, OPUS]
PRICE = {HAIKU: (0.10, 0.50), SONNET: (2.00, 10.00), OPUS: (4.00, 20.00)}  # $ per M tokens in/out
LABELS = ["Entailment", "Contradiction", "NotMentioned"]
# max_tokens includes thinking. The adjudicator runs at medium effort on long documents, where 4000
# risked paid truncations (stop_reason max_tokens, no usable answer); only the reservation grows.
MAX_TOKENS = {"reader": 3000, "entailment": 1500, "adjudicate": 8000}
# Readers and the entailment check run at low effort to keep cost down; the adjudicator at the
# model default (medium on Opus 5.5), since it gets the hard items.
EFFORT = {"reader": "low", "entailment": "low", "adjudicate": None}
LEDGER_COLS = ["timestamp", "split", "dir", "model", "item_id", "kind", "input_tokens",
               "output_tokens", "cost_usd", "dry_run", "stop_reason"]
ERROR_COLS = ["timestamp", "split", "dir", "model", "item_id", "kind", "error"]

READING = {"type": "object", "additionalProperties": False,
           "required": ["label", "confidence", "quotes", "reason"],
           "properties": {"label": {"type": "string", "enum": LABELS},
                          "confidence": {"type": "integer", "description": "0-100"},
                          "quotes": {"type": "array", "items": {"type": "string"}},
                          "reason": {"type": "string"}}}
READING_C0 = {**READING, "required": ["label", "confidence", "reason"],
              "properties": {k: v for k, v in READING["properties"].items() if k != "quotes"}}
ENTAIL = {"type": "object", "additionalProperties": False, "required": ["supports", "reason"],
          "properties": {"supports": {"type": "boolean"}, "reason": {"type": "string"}}}


def paths(dry_run):
    base = HERE / "dryrun" if dry_run else HERE
    return base / "runs", base / "results"


def run_name(split, dataset="clean"):
    """Folder name under runs/ and results/: the split, or test-<dataset> for a damaged variant, so a
    variant run never mixes with or overwrites the clean one."""
    if dataset != "clean" and split != "test":
        sys.exit(f"--dataset {dataset} is only valid with --split test")
    return split if dataset == "clean" else f"{split}-{dataset}"


def load_items(split, dataset="clean"):
    run_name(split, dataset)
    with open(DATA / (DATASETS[dataset] or SPLITS[split])) as f:
        return [json.loads(line) for line in f]


def cost(model, usage):
    """From the response's usage. No cache_control is sent, so the cache fields should be 0 or None;
    if any appear they are counted at 1.25x the input price (an upper bound) rather than ignored."""
    pin, pout = PRICE[model]
    cache = (usage.get("cache_creation_input_tokens") or 0) + (usage.get("cache_read_input_tokens") or 0)
    return (usage["input_tokens"] * pin + cache * pin * 1.25 + usage["output_tokens"] * pout) / 1e6


def prompt(name, **fields):
    text = (HERE / "prompts" / f"{name}.txt").read_text()
    for k, v in fields.items():
        text = text.replace("{{" + k + "}}", v)
    return text


def record_path(runs, split, d, model, item_id):
    return runs / split / d / model / f"{item_id}.json"


def load(runs, split, d, model, item_id):
    p = record_path(runs, split, d, model, item_id)
    return json.loads(p.read_text()) if p.exists() else None


def parsed(rec):
    """The JSON answer of a saved call, or None (refusal, truncation, unparseable)."""
    if rec is None or rec["response"].get("stop_reason") not in ("end_turn", "stop_sequence"):
        return None
    text = next((b["text"] for b in rec["response"]["content"] if b.get("type") == "text"), None)
    try:
        out = json.loads(text)
    except (TypeError, ValueError):
        return None
    if "confidence" in out:
        out["confidence"] = max(0, min(100, int(out["confidence"])))
    return out


def checked(rec, item):
    """A reader/adjudicator answer with its quote check, or None."""
    a = parsed(rec)
    if a is None or a.get("label") not in LABELS:
        return None
    a["check"] = quote_check.check_reading(item["document_text"], a["label"], a.get("quotes", []),
                                           item["gold_label"], item["gold_evidence_spans"])
    return a


def c4_flag(readings):
    """C4 adjudicates where the three readers' labels differ, any quote check fails, or a reading failed."""
    if any(r is None for r in readings):
        return True
    return len({r["label"] for r in readings}) > 1 or any(r["check"]["status"] == "fail" for r in readings)


# ------------------------------------------------------------------------------- clients

class CallFailed(Exception):
    """A call that got no response. fatal: the same request would fail again (400, 401, 403, 404...)."""
    def __init__(self, msg, fatal):
        super().__init__(msg)
        self.fatal = fatal


class LiveClient:
    def __init__(self, key):
        import anthropic  # only needed for live calls
        self.anthropic = anthropic
        # the SDK retries 408/409/429/5xx/529 and connection errors with backoff (honouring retry-after)
        self.client = anthropic.Anthropic(api_key=key, max_retries=8)

    def create(self, params, ctx):
        a = self.anthropic
        try:
            return self.client.messages.create(**params).model_dump(mode="json")
        except (a.APIConnectionError, a.RateLimitError, a.InternalServerError) as e:  # retries exhausted
            raise CallFailed(f"{type(e).__name__}: {e}", fatal=False) from e
        except a.APIStatusError as e:
            fatal = e.status_code not in (408, 409, 429) and e.status_code < 500
            raise CallFailed(f"{type(e).__name__} {e.status_code}: {e}", fatal=fatal) from e


class FakeClient:
    """Deterministic stand-in: plausible JSON, sometimes wrong, sometimes with a bad quote."""
    ACC = {("reader", HAIKU, False): 0.70, ("reader", HAIKU, True): 0.75, ("reader", SONNET, True): 0.85,
           ("reader", OPUS, True): 0.90, ("adjudicate", OPUS, True): 0.90}

    def create(self, params, ctx):
        item, kind = ctx["item"], ctx["kind"]
        rng = random.Random(hashlib.sha256(f"{params['model']}|{item['item_id']}|{kind}".encode()).digest())
        hard = random.Random(item["item_id"]).random() < 0.15  # readers share one wrong label here
        gold = item["gold_label"]
        shared_wrong = random.Random(item["item_id"] + "w").choice([l for l in LABELS if l != gold])
        if kind == "entailment":
            supports = rng.random() < (0.85 if ctx["reading"]["label"] == gold else 0.4)
            answer = {"supports": supports, "reason": "Fake check."}
        else:
            acc = self.ACC[(kind, params["model"], ctx.get("quotes", True))]
            if hard and kind == "reader" and rng.random() < 0.8:
                label = shared_wrong
            elif rng.random() < (0.5 if hard else acc):
                label = gold
            else:
                label = rng.choice([l for l in LABELS if l != gold])
            seed = item.get("seed_edit")  # seeded variant only: sometimes ignore the planted change
            if seed and random.Random(hashlib.sha256(f"{params['model']}|{item['item_id']}|{kind}|seed".encode())
                                      .digest()).random() < 0.3:
                label = seed["old_gold_label"]
            conf = rng.randint(70, 99) if label == gold else rng.randint(50, 97)
            answer = {"label": label, "confidence": conf, "reason": "Fake reading."}
            if ctx.get("quotes", True):
                answer["quotes"] = [] if label == "NotMentioned" else [self.quote(rng, item, label == gold)]
        return {"id": "fake", "type": "message", "role": "assistant", "model": params["model"],
                "content": [{"type": "text", "text": json.dumps(answer)}], "stop_reason": "end_turn",
                "usage": {"input_tokens": len(params["messages"][0]["content"]) // 4,
                          "output_tokens": rng.randint(150, 450)},
                "fake_latency_ms": rng.randint(800, 6000)}

    @staticmethod
    def quote(rng, item, right):
        doc = item["document_text"]
        if right and item["gold_evidence_spans"]:
            s, e, _ = rng.choice(item["gold_evidence_spans"])
        else:
            s = rng.randrange(0, max(1, len(doc) - 150))
            e = s + 150
        q = doc[s:e][:120].strip()
        r = rng.random()
        if r < 0.08 and len(q) > 20:  # near miss: two characters changed
            i = rng.randrange(5, len(q) - 5)
            q = q[:i] + "xy" + q[i + 2:]
        elif r < 0.14:  # invented
            q = "The Receiving Party shall return all materials within five days of any request."
        return q


# ------------------------------------------------------------------------------- calls

class CapReached(Exception):
    pass


class Runner:
    def __init__(self, split, client, runs, cap, dry_run):
        self.split, self.client, self.runs, self.cap, self.dry_run = split, client, runs, cap, dry_run
        self.lock, self.reserved, self.made, self.stopped = threading.Lock(), 0.0, 0, None
        self.ledger = runs / split / "ledger.csv"
        self.errors_csv = runs / split / "errors.csv"
        self.failed, self.not_end_turn = [], []

    def spent(self):
        """Every ledger under runs/ counts towards the cap: dev, test, test-ocr and test-seeded."""
        total = 0.0
        for path in self.runs.glob("*/ledger.csv"):
            with open(path) as f:
                total += sum(float(r["cost_usd"]) for r in csv.DictReader(f))
        return total

    def call(self, d, model, kind, item, text, schema, ctx=None):
        """The saved or new record, or None when the call failed (logged to errors.csv; a rerun retries it)."""
        path = record_path(self.runs, self.split, d, model, item["item_id"])
        output_config = {"format": {"type": "json_schema", "schema": schema}}
        if EFFORT[kind]:
            output_config["effort"] = EFFORT[kind]
        params = {"model": model, "max_tokens": MAX_TOKENS[kind], "output_config": output_config,
                  "messages": [{"role": "user", "content": text}]}
        doc = item["document_text"]
        shown = text.replace(doc, f"<document {item['doc_id']} omitted, sha256 {hashlib.sha256(doc.encode()).hexdigest()[:16]}>")
        request = {**params, "messages": [{"role": "user", "content": shown}]}
        if path.exists():
            rec = json.loads(path.read_text())
            if bool(rec.get("dry_run")) != self.dry_run:
                sys.exit(f"{path} has dry_run={rec.get('dry_run')} but this run has dry_run={self.dry_run}; remove it.")
            if rec["request"] != request:  # a prompt, schema, effort or max_tokens changed since it was made
                sys.exit(f"{path} was made with a different request (prompt, schema or settings changed). "
                         f"Reusing it would mix versions: move runs/{self.split}/ aside, or delete the stale records.")
            return rec
        # worst case: input at 2 characters a token (real text is ~3-4), output at max_tokens
        worst = cost(model, {"input_tokens": len(text) // 2 + 100, "output_tokens": MAX_TOKENS[kind]})
        with self.lock:
            spent = self.spent()
            if self.stopped:
                raise CapReached(self.stopped)
            if spent + self.reserved + worst > self.cap:
                self.stopped = (f"cap ${self.cap:.2f}: spent ${spent:.4f}, in flight ${self.reserved:.4f}, "
                                 f"next call ({kind} {model} {item['item_id']}) could cost up to ${worst:.4f}")
                raise CapReached(self.stopped)
            self.reserved += worst
        try:
            t0 = time.monotonic()
            resp = self.client.create(params, {**(ctx or {}), "item": item, "kind": kind})
            latency = resp.pop("fake_latency_ms", None) or round((time.monotonic() - t0) * 1000)
        except CallFailed as e:
            with self.lock:
                self.reserved -= worst
                self.failed.append(f"{d}/{model}/{item['item_id']}")
                self._append(self.errors_csv, ERROR_COLS, {
                    "timestamp": datetime.now(timezone.utc).isoformat(timespec="seconds"), "split": self.split,
                    "dir": d, "model": model, "item_id": item["item_id"], "kind": kind, "error": str(e)[:500]})
                if e.fatal and not self.stopped:
                    self.stopped = f"fatal API error on {kind} {model} {item['item_id']}: {e}"
            if e.fatal:
                raise CapReached(self.stopped)
            print(f"call failed, item left incomplete (rerun to retry): {d}/{model}/{item['item_id']}: {e}",
                  file=sys.stderr)
            return None
        except BaseException:
            with self.lock:
                self.reserved -= worst
            raise
        rec = {"item_id": item["item_id"], "kind": kind, "dir": d, "dry_run": self.dry_run,
               "request": request, "response": resp, "usage": resp["usage"], "latency_ms": latency,
               "timestamp": datetime.now(timezone.utc).isoformat(timespec="seconds"),
               "cost_usd": cost(model, resp["usage"])}
        with self.lock:
            # ledger first, then release the reservation, so the cap never misses a paid call
            self._append(self.ledger, LEDGER_COLS, {
                "timestamp": rec["timestamp"], "split": self.split, "dir": d, "model": model,
                "item_id": item["item_id"], "kind": kind, "input_tokens": resp["usage"]["input_tokens"],
                "output_tokens": resp["usage"]["output_tokens"], "cost_usd": f"{rec['cost_usd']:.6f}",
                "dry_run": self.dry_run, "stop_reason": resp.get("stop_reason")})
            self.reserved -= worst
            self.made += 1
            if resp.get("stop_reason") != "end_turn":
                self.not_end_turn.append(f"{d}/{model}/{item['item_id']}: {resp.get('stop_reason')}")
        path.parent.mkdir(parents=True, exist_ok=True)
        tmp = path.with_suffix(".tmp")
        tmp.write_text(json.dumps(rec, indent=1))
        tmp.replace(path)  # atomic: a crash never leaves a half-written record
        return rec

    @staticmethod
    def _append(path, cols, row):
        path.parent.mkdir(parents=True, exist_ok=True)
        new = not path.exists()
        with open(path, "a", newline="") as f:
            w = csv.DictWriter(f, cols)
            if new:
                w.writeheader()
            w.writerow(row)

    def item(self, item, conds):
        doc, hyp = item["document_text"], item["hypothesis"]
        if "C0" in conds:
            self.call("C0", HAIKU, "reader", item, prompt("reader_c0", DOCUMENT=doc, HYPOTHESIS=hyp),
                      READING_C0, {"quotes": False})
        if not conds & {"C1", "C2", "C3", "C4", "C5"}:
            return
        text = prompt("reader_quotes", DOCUMENT=doc, HYPOTHESIS=hyp)
        h = self.call("C1", HAIKU, "reader", item, text, READING)
        if h is None:  # failed call: nothing downstream may be built on a missing reading
            return
        if "C2" in conds:
            r = checked(h, item)
            if r and r["label"] != "NotMentioned" and r["quotes"]:
                quotes = "\n".join(f"<quote>{q}</quote>" for q in r["quotes"])
                self.call("C2", SONNET, "entailment", item,
                          prompt("entailment", HYPOTHESIS=hyp, LABEL=r["label"], QUOTES=quotes), ENTAIL, {"reading": r})
        if not conds & {"C3", "C4", "C5"}:
            return
        recs = [h] + [self.call("C3", m, "reader", item, text, READING) for m in (SONNET, OPUS)]
        if None in recs:  # do not adjudicate on readings that a rerun would change
            return
        readings = [checked(r, item) for r in recs]
        flag = c4_flag(readings)
        if ("C4" in conds and flag) or "C5" in conds:
            self.call("C4" if flag else "C5", OPUS, "adjudicate", item,
                      prompt("adjudicator", DOCUMENT=doc, HYPOTHESIS=hyp, READINGS=show(readings)), READING)


def show(readings):
    out = []
    for name, r in zip("ABC", readings):
        if r is None:
            out.append(f'<reading id="{name}">No usable answer.</reading>')
            continue
        qs = "\n".join(f'<quote check="{c["status"]}">{c["quote"]}</quote>' for c in r["check"]["quotes"]) or "(no quotes)"
        out.append(f'<reading id="{name}">\nlabel: {r["label"]}\nconfidence: {r["confidence"]}\n'
                   f'reason: {r["reason"]}\n{qs}\n</reading>')
    return "\n".join(out)


def api_key(key_file):
    key = os.environ.get("ANTHROPIC_API_KEY", "").strip()
    if not key:
        p = Path(key_file).expanduser()
        key = p.read_text().strip() if p.exists() else ""
    if not key:
        sys.exit(f"No API key: set ANTHROPIC_API_KEY or put the key in {key_file} (or use --dry-run).")
    return key


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--split", choices=SPLITS, required=True)
    ap.add_argument("--dataset", choices=DATASETS, default="clean",
                    help="test variant (only with --split test): writes runs/test-<dataset>/")
    ap.add_argument("--conditions", default="C0,C1,C2,C3,C4,C5")
    ap.add_argument("--dry-run", action="store_true", help="fake client; writes under dryrun/")
    ap.add_argument("--cap", type=float, default=None,
                    help="USD, summed over every ledger under runs/ (default 3.00; 10.00 of fake money in --dry-run)")
    ap.add_argument("--key-file", default="~/.config/s106/anthropic_key")
    ap.add_argument("--workers", type=int, default=4)
    ap.add_argument("--yes", action="store_true", help="skip the live-run confirmation")
    ap.add_argument("--max-items", type=int, default=None,
                    help="only the first N items of the split (e.g. 1 for a smoke test); a later full run resumes")
    a = ap.parse_args()
    conds = set(a.conditions.split(","))
    if conds - {"C0", "C1", "C2", "C3", "C4", "C5"}:
        sys.exit(f"unknown condition in {a.conditions}")
    if a.cap is None:  # fake dry-run costs of all four runs (~$6.5) would pass $3.00
        a.cap = 10.00 if a.dry_run else 3.00
    name = run_name(a.split, a.dataset)
    runs, _ = paths(a.dry_run)
    items = load_items(a.split, a.dataset)[:a.max_items]
    if a.dry_run:
        client = FakeClient()
    else:
        client = LiveClient(api_key(a.key_file))  # exits before anything is written when there is no key
        if not a.yes and input(f"Live paid run: {name}, {len(items)} items, {sorted(conds)}, "
                               f"cap ${a.cap:.2f}. Type yes: ") != "yes":
            sys.exit("not started")
    (runs / name).mkdir(parents=True, exist_ok=True)
    runner = Runner(name, client, runs, a.cap, a.dry_run)
    pool = ThreadPoolExecutor(a.workers)
    try:
        for f in [pool.submit(runner.item, it, conds) for it in items]:
            f.result()
    except BaseException as e:  # cap, fatal API error, Ctrl-C, bug: start no further calls
        with runner.lock:
            runner.stopped = runner.stopped or f"{type(e).__name__}: {e}"
        pool.shutdown(wait=True, cancel_futures=True)  # calls already in flight finish and are recorded
        print(f"STOPPED before the next call: {runner.stopped}. {runner.made} new calls made; "
              f"total spent ${runner.spent():.4f}. Fix the cause (or raise --cap) and rerun to resume.",
              file=sys.stderr)
        if not isinstance(e, CapReached):
            raise
        sys.exit(2)
    pool.shutdown()
    print(f"{name}: {runner.made} new calls; total spent ${runner.spent():.4f} of cap ${a.cap:.2f}")
    if runner.not_end_turn:
        print(f"{len(runner.not_end_turn)} saved calls did not end with end_turn (no usable answer; paid, "
              f"not retried): " + "; ".join(runner.not_end_turn), file=sys.stderr)
    if runner.failed:
        print(f"{len(runner.failed)} calls FAILED and their items are incomplete (see "
              f"{runner.errors_csv}); rerun the same command to retry them.", file=sys.stderr)
        sys.exit(1)


if __name__ == "__main__":
    main()
