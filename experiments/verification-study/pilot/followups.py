"""Follow-up tables asked for by outside reviewers, from a run's records and results/<run>/items.csv.

    python3 followups.py --split dev
    python3 followups.py --split test [--dataset clean|ocr|seeded]
    python3 followups.py --split test --pooled     # clean run, plus a pooled clean+seeded C4 vs C3 test

Writes results/<run>/followups.md (tables) and results/<run>/followups.csv (the same numbers, long
format: table,row,column,value). Tables:
    1 accuracy by client (api / subagent), per reader and per condition
    2 accuracy by gold label with Wilson 95% intervals, per reader and per condition
    3 exact McNemar tests: C3 vs C1, C4 vs C3, C5 vs C3 (and pooled clean+seeded C4 vs C3 with --pooled)
    4 adjudication detail for every item C4 adjudicated (a C4 record exists)
    5 cost per correct answer per condition relative to C0, on the items that are API-only in both
"""
import argparse, csv
from math import comb

from analyse import client, outcome, wilson
from run import DATASETS, LABELS, OPUS, load_items, paths, record_path, run_name

CONDS = ["C0", "C1", "C2", "C3", "C4", "C5"]
READERS = ["haiku", "sonnet", "opus"]
# the condition rows of items.csv that carry each reader's reading (Haiku first appears in C1)
READER_ROW = {"haiku": "C1", "sonnet": "C3", "opus": "C3"}
# the order show() renders the three readings in the adjudicator prompt
LETTER = {"haiku": "A", "sonnet": "B", "opus": "C"}


def read_items(results, name):
    """items.csv as {condition: {item_id: row}}."""
    by_cond = {}
    with open(results / name / "items.csv") as f:
        for r in csv.DictReader(f):
            by_cond.setdefault(r["condition"], {})[r["item_id"]] = r
    return by_cond


def is_true(v):
    return v == "True"


def reader_client(row, reader):
    """The reading's client from items.csv; older items.csv files (dev) have no client column: api."""
    return row.get(f"{reader}_client") or "api"


def row_client(row):
    """api or subagent when every reading behind the row has that client, else mixed."""
    clients = {v for k, v in row.items() if k.endswith("_client") and v}
    if not clients or clients == {"api"}:
        return "api"
    if clients == {"subagent"}:
        return "subagent"
    return "mixed"


def frac(k, n):
    return f"{k}/{n}" if n else "-"


def mcnemar(b, c):
    """Exact two-sided McNemar p: binomial(b + c, 1/2) on the discordant pairs."""
    n = b + c
    if n == 0:
        return 1.0
    k = min(b, c)
    p = 2 * sum(comb(n, i) for i in range(k + 1)) / 2 ** n
    return min(1.0, p)


class Out:
    """Collects the Markdown lines and the long-format CSV rows."""

    def __init__(self, title):
        self.md = [f"# {title}", ""]
        self.csv = []

    def table(self, sentence, header, rows):
        self.md += [sentence, "", "| " + " | ".join(header) + " |", "|" + "---|" * len(header)]
        for r in rows:
            self.md.append("| " + " | ".join(str(x) for x in r) + " |")
        self.md.append("")

    def value(self, table, row, column, value):
        self.csv.append({"table": table, "row": row, "column": column, "value": value})


def accuracy_by_client(out, items):
    cols = ["api", "subagent", "mixed"]
    rows = []
    groups = [(r, items[READER_ROW[r]], f"{r}_label", lambda row, r=r: reader_client(row, r))
              for r in READERS if READER_ROW[r] in items]
    groups += [(c, items[c], "label", row_client) for c in CONDS if c in items]
    for name, cond_rows, label_col, get_client in groups:
        counts = {c: [0, 0] for c in cols}
        for row in cond_rows.values():
            counts[get_client(row)][0] += row[label_col] == row["gold"]
            counts[get_client(row)][1] += 1
        rows.append([name] + [frac(*counts[c]) for c in cols])
        for c in cols:
            out.value("1_accuracy_by_client", name, f"{c}_correct", counts[c][0])
            out.value("1_accuracy_by_client", name, f"{c}_n", counts[c][1])
    out.md.append("## 1. Accuracy by client")
    out.md.append("")
    out.table("Correct/n for each reader (its own label) and each condition (its final label), split by "
              "client; a condition row is mixed when its readings came from both clients.",
              ["reader / condition"] + cols, rows)


def accuracy_by_gold(out, items):
    rows = []
    groups = [(r, items[READER_ROW[r]], f"{r}_label") for r in READERS if READER_ROW[r] in items]
    groups += [(c, items[c], "label") for c in CONDS if c in items]
    for name, cond_rows, label_col in groups:
        cells = [name]
        for gold in LABELS:
            got = [row for row in cond_rows.values() if row["gold"] == gold]
            k, n = sum(row[label_col] == gold for row in got), len(got)
            lo, hi = wilson(k, n)
            cells.append(f"{k}/{n} ({lo}-{hi})" if n else "-")
            for col, v in (("correct", k), ("n", n), ("lo", lo), ("hi", hi)):
                out.value("2_accuracy_by_gold", name, f"{gold}_{col}", v)
        rows.append(cells)
    out.md.append("## 2. Accuracy by gold label")
    out.md.append("")
    out.table("Correct/n per gold label for each reader and condition, with Wilson 95% intervals in brackets.",
              ["reader / condition"] + LABELS, rows)


def pairs(items, first, second):
    """(first correct, second correct) for each item that has both conditions."""
    a, b = items.get(first, {}), items.get(second, {})
    return [(is_true(a[i]["correct"]), is_true(b[i]["correct"])) for i in a if i in b]


def mcnemar_tests(out, items, pooled_items):
    tests = [("C3 vs C1", "C3", "C1"), ("C4 vs C3", "C4", "C3"), ("C5 vs C3", "C5", "C3")]
    rows, notes = [], []
    for name, first, second in tests:
        if first == "C5" and len(items.get(first, {})) < len(items.get(second, {})):
            notes.append(f"{name} skipped: {first} has {len(items.get(first, {}))} items, {second} has "
                         f"{len(items[second])} (no full {first} run here).")
            continue
        rows.append(mcnemar_row(out, name, pairs(items, first, second)))
    if pooled_items is not None:
        both = pairs(items, "C4", "C3") + pairs(pooled_items, "C4", "C3")
        rows.append(mcnemar_row(out, "C4 vs C3, pooled clean+seeded", both))
        notes.append("Pooled caveat: clean and seeded share the same 40 item ids and mostly the same documents, "
                     "so the pooled pairs are not independent.")
    out.md.append("## 3. Exact McNemar tests")
    out.md.append("")
    out.table("Paired exact McNemar tests (two-sided) on the items both conditions have; C2 vs C1 is skipped "
              "(same labels).",
              ["comparison", "pairs", "b = first right and second wrong", "c = first wrong and second right", "p"],
              rows)
    if notes:
        out.md += notes + [""]


def mcnemar_row(out, name, prs):
    b = sum(x and not y for x, y in prs)
    c = sum(y and not x for x, y in prs)
    p = round(mcnemar(b, c), 4)
    for col, v in (("pairs", len(prs)), ("b", b), ("c", c), ("p", p)):
        out.value("3_mcnemar", name, col, v)
    return [name, len(prs), b, c, p] if prs else [name, 0, "-", "-", "-"]


def adjudication_detail(out, items, runs, name):
    rows, counts, agree_counts = [], {}, {"A": 0, "B": 0, "C": 0, "none": 0}
    c3, c4 = items.get("C3", {}), items.get("C4", {})
    for iid in sorted(c4):
        if not record_path(runs, name, "C4", OPUS, iid).exists():
            continue
        row = c4[iid]
        adj = row["adj_label"]
        agreed = [LETTER[r] for r in READERS if adj and row[f"{r}_label"] == adj]
        for x in agreed or ["none"]:
            agree_counts[x] += 1
        before, after = is_true(c3[iid]["correct"]), is_true(row["correct"])
        change = "right->wrong" if before and not after else "wrong->right" if after and not before else "unchanged"
        counts[change] = counts.get(change, 0) + 1
        cells = [iid, row["gold"]] + [row[f"{r}_label"] or "(none)" for r in READERS]
        cells += [c3[iid]["label"], adj or "(none)", ",".join(agreed) or "none", change]
        rows.append(cells)
        for col, v in zip(["gold", "haiku", "sonnet", "opus", "majority", "adjudicator", "agreed_with", "change"],
                          cells[1:]):
            out.value("4_adjudication", iid, col, v)
    out.md.append("## 4. Adjudication detail")
    out.md.append("")
    out.table("Every item C4 sent to the adjudicator; readings A/B/C are Haiku/Sonnet/Opus in the order show() "
              "renders them, and change compares C4 with the C3 majority.",
              ["item", "gold", "haiku (A)", "sonnet (B)", "opus (C)", "majority", "adjudicator", "agreed with",
               "change"], rows)
    count_rows = [[k, counts.get(k, 0)] for k in ("right->wrong", "wrong->right", "unchanged")]
    count_rows += [[f"agreed with {k}", v] for k, v in agree_counts.items()]
    for k, v in count_rows:
        out.value("4_adjudication_counts", k, "items", v)
    out.table("Counts over those items (an adjudicator label can agree with more than one reading).",
              ["count", "items"], count_rows)


def cost_per_correct(out, runs, name, items_json):
    """Each condition's cost per correct answer over the items that are API-only (every call made by
    run.py, costed from the ledger) in both C0 and that condition, and its ratio to C0's on those items."""
    ledger = {}
    with open(runs / name / "ledger.csv") as f:
        for r in csv.DictReader(f):
            if r["stop_reason"] != "subagent":
                ledger[(r["dir"], r["model"], r["item_id"])] = float(r["cost_usd"])
    api_cost = lambda calls: sum(ledger[(c["dir"], c["request"]["model"], c["item_id"])] for c in calls)
    api_only = lambda calls: all(client(c) == "api" for c in calls)

    def per_item(cond):
        """{item_id: (correct, calls)} for every item the condition has."""
        res = {}
        for it in items_json:
            o = outcome(cond, it, runs, name)
            if o is not None:
                res[it["item_id"]] = (o[0] == it["gold_label"], o[4])
        return res

    def ratio(got, base, ids, cost):
        """(cost per correct, C0 cost per correct, ratio) on ids, unrounded; None where undefined."""
        k, k0 = sum(got[i][0] for i in ids), sum(base[i][0] for i in ids)
        per = sum(cost(got[i][1]) for i in ids) / k if k else None
        per0 = sum(cost(base[i][1]) for i in ids) / k0 if k0 else None
        return per, per0, (per / per0 if per is not None and per0 else None)

    def estimated_cost(calls):
        """Ledger cost for API calls plus the record's estimated cost_usd for subagent calls."""
        return api_cost([c for c in calls if client(c) == "api"]) + sum(
            c["cost_usd"] for c in calls if client(c) == "subagent")

    show = lambda v, nd: "-" if v is None else round(v, nd)
    base = per_item("C0")
    base_api = {i for i, (_, calls) in base.items() if api_only(calls)}
    table, notes = [], []
    for cond in CONDS:
        got = per_item(cond)
        if not got:
            continue
        ids = sorted(i for i in base_api if i in got and api_only(got[i][1]))
        recs = [c for i in got for c in got[i][1]]
        n_api = sum(client(c) == "api" for c in recs)
        used = sum(len(got[i][1]) for i in ids)
        per, per0, r = ratio(got, base, ids, api_cost)
        # the call that distinguishes the condition: adjudication for C4/C5, else the condition's own dir
        distinctive = [c for c in recs if (c["dir"] in ("C4", "C5") if cond in ("C4", "C5") else c["dir"] == cond)]
        estimable = not distinctive or any(client(c) == "api" for c in distinctive)
        est = ""
        if not estimable:
            est_ids = sorted(i for i in base_api if i in got)
            no_cost = sum(client(c) == "subagent" and c["cost_usd"] is None for i in est_ids for c in got[i][1])
            if no_cost:
                est = f"none ({no_cost} subagent records have no cost_usd)"
            elif ratio(got, base, est_ids, estimated_cost)[2] is None:
                est = "none (no API-only C0 item with a correct answer here)"
            else:
                est = f"estimated {round(ratio(got, base, est_ids, estimated_cost)[2], 2)} on {len(est_ids)} items"
        table.append([cond, len(ids), used, n_api - used, len(recs) - n_api,
                      round(sum(api_cost(got[i][1]) for i in ids), 6), sum(got[i][0] for i in ids),
                      show(per, 6), show(per0, 6),
                      show(r, 2) if estimable else "not estimable from API records", est])
        for col, v in (("items", len(ids)), ("api_records_used", used), ("api_records_dropped", n_api - used),
                       ("subagent_records", len(recs) - n_api), ("cost_usd", table[-1][5]),
                       ("correct", table[-1][6]), ("cost_per_correct", "" if per is None else per),
                       ("c0_cost_per_correct", "" if per0 is None else per0),
                       ("cost_per_correct_vs_C0", "" if r is None or not estimable else r),
                       ("estimable_from_api", estimable), ("estimated_vs_C0", est)):
            out.value("5_cost_per_correct", cond, col, v)
    out.md.append("## 5. Cost per correct answer")
    out.md.append("")
    out.table("API cost per correct answer for each condition and for C0, both over the items whose calls are all "
              "API calls (ledger cost) in C0 and in that condition, and the ratio of the two. API records dropped "
              "are the condition's API records on items outside that set; subagent records are never in the API "
              "cost. A condition whose distinctive call (adjudication for C4/C5) has no API record is not "
              "estimable from API records; its last column uses the subagent records' estimated cost_usd.",
              ["condition", "items", "API records used", "API records dropped", "subagent records (not costed)",
               "cost USD", "correct", "cost per correct", "C0 cost per correct (same items)", "ratio to C0",
               "estimated ratio (incl. subagent cost_usd)"], table)
    if not table or all(r[1] == 0 for r in table):
        out.md += ["No condition shares an item made entirely of API calls with C0 in this run.", ""]


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--split", choices=["dev", "test"], required=True)
    ap.add_argument("--dataset", choices=DATASETS, default="clean")
    ap.add_argument("--pooled", action="store_true",
                    help="clean test only: add a pooled clean+seeded C4 vs C3 McNemar test")
    a = ap.parse_args()
    name = run_name(a.split, a.dataset)
    if a.pooled and name != "test":
        raise SystemExit("--pooled needs --split test with the clean dataset")
    runs, results = paths(False)
    items = read_items(results, name)
    pooled_items = read_items(results, "test-seeded") if a.pooled else None
    out = Out(f"Follow-ups: {name}")
    accuracy_by_client(out, items)
    accuracy_by_gold(out, items)
    mcnemar_tests(out, items, pooled_items)
    adjudication_detail(out, items, runs, name)
    cost_per_correct(out, runs, name, load_items(a.split, a.dataset))
    (results / name / "followups.md").write_text("\n".join(out.md))
    with open(results / name / "followups.csv", "w", newline="") as f:
        w = csv.DictWriter(f, ["table", "row", "column", "value"])
        w.writeheader()
        w.writerows(out.csv)
    print(f"wrote {results / name}/followups.md and followups.csv")


if __name__ == "__main__":
    main()
