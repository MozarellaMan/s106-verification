#!/usr/bin/env python3
"""Evaluate the llm-pdf backend on a set of deeds against the national S106 register.

    python s106-audit/run_eval.py [--runs 3] [--model claude-haiku-5-5] [--only ID ...]

Inputs
  data/reference/candidates.tsv                 deeds to run (id, council, planning_ref, ..., url)
  data/raw/eval/<id>.pdf                        the downloaded deeds
  data/reference/developer-agreement*.csv       planning.data.gov.uk register (reference values)

Per deed: k live extractions (the first is recorded as a replay fixture), each verified, then
  * register scoring: extracted fixed sums vs register amounts, affordable units vs register units
  * run agreement: do the k runs report the same housing values, sums, flags and trigger labels?
  * gate outcome: pass / fail / skip counts and error classes per run

The register is a reference, not ground truth: councils split sums into instalments, duplicate
rows, and record indexed or partial figures. So a sum "matches" when it equals one register
amount (±£1) or the total of one purpose's amounts, and misses are listed for a human to check.

Writes data/processed/eval/<id>/run<k>.audit.json and data/processed/eval/summary.{json,md}.
"""
from __future__ import annotations

import argparse
import csv
import json
import re
import sys
import time
from collections import defaultdict
from concurrent.futures import ThreadPoolExecutor
from dataclasses import asdict
from decimal import Decimal
from pathlib import Path

from extractor import PDF_DEFAULT_MODEL, extract
from run_audit import harness_client
from verifier import ErrorClass, Status, verify

ROOT = Path(__file__).resolve().parent.parent
REF = ROOT / "data" / "reference"
RAW = ROOT / "data" / "raw" / "eval"
OUT = ROOT / "data" / "processed" / "eval"
TOLERANCE = Decimal("1.00")
# Haiku 5.5 per-MTok rates (README): under / over 100k prompt tokens
RATES = {"claude-haiku-5-5": ((0.10, 0.50), (0.50, 2.50))}


# --------------------------------------------------------------------------- register

def load_register() -> tuple[dict, dict]:
    """({planning ref: [agreement rows]}, {agreement reference: [contribution rows]})."""
    agreements = defaultdict(list)
    for a in csv.DictReader(open(REF / "developer-agreement.csv")):
        agreements[a["planning-application"].strip()].append(a)
    contributions = defaultdict(list)
    for c in csv.DictReader(open(REF / "developer-agreement-contribution.csv")):
        contributions[c["developer-agreement"]].append(c)
    return agreements, contributions


def register_reference(ref: str, agreements, contributions) -> dict:
    # CIL is a separate levy, not an obligation in the deed
    deeds = [a for a in agreements.get(ref, []) if a["developer-agreement-type"] != "community-infrastructure-levy"]
    rows = [c for a in deeds for c in contributions.get(a["reference"], [])]
    amounts = [(c["contribution-purpose"], Decimal(c["amount"])) for c in rows
               if c["amount"] and Decimal(c["amount"]) > 0]
    by_purpose = defaultdict(Decimal)
    for purpose, amt in amounts:
        by_purpose[purpose] += amt
    ah_units = sorted({int(Decimal(c["units"])) for c in rows
                       if c["contribution-purpose"] == "affordable-housing" and c["units"].strip()
                       and Decimal(c["units"]) > 0})
    return {"amounts": amounts, "purpose_totals": dict(by_purpose), "affordable_units": ah_units,
            "agreement_types": sorted({a["developer-agreement-type"] for a in deeds})}


def score_against_register(agreement, reference: dict) -> dict:
    sums = [c.amount_gbp for c in agreement.contributions if c.amount_gbp is not None]
    reg_amounts = [a for _, a in reference["amounts"]]
    reg_totals = list(reference["purpose_totals"].values())

    def close(x, ys):
        return any(abs(x - y) <= TOLERANCE for y in ys)

    matched = [s for s in sums if close(s, reg_amounts) or close(s, reg_totals)]
    # a register amount is "found" if some extracted sum equals it, or equals its purpose total
    found = [(p, a) for p, a in reference["amounts"]
             if close(a, sums) or close(reference["purpose_totals"][p], sums)]
    unmatched_extracted = [str(s) for s in sums if s not in matched]
    missed_register = [f"{p}={a}" for p, a in reference["amounts"] if (p, a) not in found]
    h = agreement.housing
    ah = h.affordable_units
    units_ok = None if not reference["affordable_units"] else (
        ah is not None and (ah in reference["affordable_units"] or ah == sum(reference["affordable_units"])))
    return {
        "extracted_sums": len(sums), "extracted_sums_matched": len(matched),
        "register_amounts": len(reg_amounts), "register_amounts_found": len(found),
        "unmatched_extracted": unmatched_extracted, "missed_register": missed_register,
        "affordable_units": ah, "register_affordable_units": reference["affordable_units"],
        "affordable_units_match": units_ok,
    }


# --------------------------------------------------------------------------- runs

def usage_from(agreement) -> tuple[int, int, int]:
    for w in agreement.warnings:
        m = re.search(r"([\d,]+) in / ([\d,]+) out tokens", w)
        if m:
            att = re.search(r"attempts\D*(\d+)", w)
            return int(m.group(1).replace(",", "")), int(m.group(2).replace(",", "")), int(att.group(1)) if att else 1
    return 0, 0, 1


def cost(model: str, tin: int, tout: int) -> float:
    (lo_in, lo_out), (hi_in, hi_out) = RATES.get(model, ((0, 0), (0, 0)))
    r_in, r_out = (hi_in, hi_out) if tin > 100_000 else (lo_in, lo_out)
    return tin / 1e6 * r_in + tout / 1e6 * r_out


def run_once(pdf: Path, model: str, mode: str, out: Path, client) -> dict:
    t0 = time.time()
    try:
        agreement, pages = extract(pdf, backend="llm-pdf", model=model, llm_mode=mode, client=client)
    except Exception as e:  # one failed run must not sink the deed's other runs
        return {"error": f"{type(e).__name__}: {e}"[:400], "seconds": round(time.time() - t0, 1)}
    results = verify(agreement, pages)
    out.write_text(json.dumps({
        "agreement": agreement.model_dump(mode="json"),
        "verification": [{**asdict(r), "status": r.status.value,
                          "error_class": r.error_class.value if r.error_class else None} for r in results],
    }, indent=2))
    tin, tout, _ = usage_from(agreement)
    return {"agreement": agreement, "results": results, "seconds": round(time.time() - t0, 1),
            "tokens_in": tin, "tokens_out": tout, "cost_usd": 0.0 if mode == "replay" else cost(model, tin, tout)}


def gate_summary(results) -> dict:
    return {"pass": sum(r.status == Status.PASS for r in results),
            "review": sum(r.status == Status.REVIEW for r in results),
            "fail": sum(r.status == Status.FAIL for r in results),
            "skip": sum(r.status == Status.SKIP for r in results),
            "parser_errors": sum(r.error_class == ErrorClass.PARSER_ERROR for r in results),
            "document_discrepancies": sum(r.error_class == ErrorClass.DOCUMENT_DISCREPANCY for r in results),
            "failures": [f"{r.check_id} {r.description}: {r.detail}" for r in results if r.status == Status.FAIL],
            "reviews": [f"{r.check_id} {r.description}: {r.detail}" for r in results if r.status == Status.REVIEW]}


def fingerprint(a) -> dict:
    """The run's reported values, minus evidence snippets and free text that may legitimately vary."""
    h = a.housing
    return {
        "housing": {"total_units": h.total_units, "market_units": h.market_units,
                    "affordable_units": h.affordable_units, "affordable_habitable_rooms": h.affordable_habitable_rooms,
                    "tenures": sorted((t.tenure.value, t.units, t.habitable_rooms) for t in h.tenures),
                    "claimed_affordable_pct": str(h.claimed_affordable_pct) if h.claimed_affordable_pct else None},
        "sums": sorted(str(c.amount_gbp) for c in a.contributions if c.amount_gbp is not None),
        "contributions": {re.sub(r"\W+", " ", c.name.lower()).strip(): (
            str(c.amount_gbp), c.index_linked, c.is_cap, c.contingent,
            c.trigger_event.value if c.trigger_event else None) for c in a.contributions},
    }


def agreement_across(runs: list[dict]) -> dict:
    fps = [fingerprint(r["agreement"]) for r in runs if "agreement" in r]
    if len(fps) < 2:
        return {"runs_compared": len(fps)}
    same = lambda key: all(f[key] == fps[0][key] for f in fps)  # noqa: E731
    names = set().union(*(f["contributions"] for f in fps))
    common = [n for n in names if all(n in f["contributions"] for f in fps)]
    field_diffs = []
    for n in sorted(common):
        vals = [f["contributions"][n] for f in fps]
        for i, field in enumerate(("amount", "index_linked", "is_cap", "contingent", "trigger")):
            if len({v[i] for v in vals}) > 1:
                field_diffs.append(f"{n}: {field} {[v[i] for v in vals]}")
    housing_diffs = [k for k in fps[0]["housing"] if len({json.dumps(f["housing"][k]) for f in fps}) > 1]
    return {"runs_compared": len(fps), "housing_identical": same("housing"), "housing_diffs": housing_diffs,
            "sums_identical": same("sums"),
            "contribution_names": {"union": len(names), "in_every_run": len(common),
                                   "only_some_runs": sorted(names - set(common))},
            "field_diffs": field_diffs}


# --------------------------------------------------------------------------- main

def evaluate(row: dict, k: int, model: str, register, client, replay: bool = False) -> dict:
    pdf = RAW / f"{row['id']}.pdf"
    out = OUT / row["id"]
    out.mkdir(parents=True, exist_ok=True)
    # Run 1 records the fixture (and warms the OCR cache); the rest run in parallel.
    if replay:  # re-verify the recorded run-1 response: free, offline, reproducible
        k = 1
    runs = [run_once(pdf, model, "replay" if replay else "record", out / "run1.audit.json", client)]
    with ThreadPoolExecutor(max_workers=k - 1 or 1) as pool:
        runs += list(pool.map(lambda i: run_once(pdf, model, "live", out / f"run{i}.audit.json", client),
                              range(2, k + 1)))
    reference = register_reference(row["planning_ref"], *register)
    ok = [r for r in runs if "agreement" in r]
    return {
        "id": row["id"], "council": row["council"], "planning_ref": row["planning_ref"], "type": row["type"],
        "register_types": reference["agreement_types"],
        "pages": ok[0]["agreement"].metadata.page_count if ok else None,
        "text_source": ok[0]["agreement"].metadata.text_source if ok else None,
        "runs": [{"run": i + 1, **({"error": r["error"]} if "error" in r else {
                  "gate": gate_summary(r["results"]), "register": score_against_register(r["agreement"], reference),
                  "contributions": len(r["agreement"].contributions),
                  "seconds": r["seconds"], "tokens_in": r["tokens_in"], "tokens_out": r["tokens_out"],
                  "cost_usd": round(r["cost_usd"], 4)})} for i, r in enumerate(runs)],
        "agreement": agreement_across(ok),
    }


def markdown(summary: list[dict], model: str) -> str:
    lines = [f"# Register eval: {model}", "",
             "| deed | pp | src | runs ok | gate (pass/review/fail/skip, parser err) | sums matched | register found | "
             "AH units (ours / register) | runs agree | cost |",
             "|---|---|---|---|---|---|---|---|---|---|"]
    total = 0.0
    for d in summary:
        ok = [r for r in d["runs"] if "gate" in r]
        c = sum(r["cost_usd"] for r in ok)
        total += c
        if not ok:
            lines.append(f"| {d['id']} | | | 0/{len(d['runs'])} | {d['runs'][0].get('error', '')[:60]} | | | | | |")
            continue
        g = ok[0]["gate"]
        reg = ok[0]["register"]
        ag = d["agreement"]
        agree = ("n/a" if ag.get("runs_compared", 0) < 2 else
                 "yes" if ag["housing_identical"] and ag["sums_identical"] and not ag["field_diffs"]
                 and not ag["contribution_names"]["only_some_runs"] else "no")
        units = f"{reg['affordable_units']} / {reg['register_affordable_units'] or '-'}"
        lines.append(f"| {d['id']} | {d['pages']} | {d['text_source']} | {len(ok)}/{len(d['runs'])} | "
                     f"{g['pass']}/{g['review']}/{g['fail']}/{g['skip']}, {g['parser_errors']} | "
                     f"{reg['extracted_sums_matched']}/{reg['extracted_sums']} | "
                     f"{reg['register_amounts_found']}/{reg['register_amounts']} | {units} | {agree} | ${c:.3f} |")
    lines += ["", f"Total cost: ${total:.2f}", "", "Gate and register columns are for run 1; *runs agree* compares "
              "all runs (housing, sums, contribution set, flags, trigger labels).", ""]
    for d in summary:
        lines.append(f"## {d['id']} ({d['council']} {d['planning_ref']}, {d['type']})")
        for r in d["runs"]:
            if "error" in r:
                lines.append(f"- run {r['run']}: **error** {r['error']}")
                continue
            reg = r["register"]
            lines.append(f"- run {r['run']}: gate {r['gate']['pass']}/{r['gate']['review']}/{r['gate']['fail']}/"
                         f"{r['gate']['skip']}; "
                         f"{r['contributions']} contributions; {r['tokens_in']:,} in / {r['tokens_out']:,} out; "
                         f"{r['seconds']}s")
            for f in r["gate"]["failures"]:
                lines.append(f"  - FAIL {f}")
            for f in r["gate"]["reviews"]:
                lines.append(f"  - REVIEW {f}")
            if reg["unmatched_extracted"]:
                lines.append(f"  - sums not in register: {', '.join(reg['unmatched_extracted'])}")
            if reg["missed_register"]:
                lines.append(f"  - register amounts not extracted: {', '.join(reg['missed_register'])}")
        ag = d["agreement"]
        if ag.get("runs_compared", 0) >= 2:
            if ag["housing_diffs"]:
                lines.append(f"- housing differs across runs: {', '.join(ag['housing_diffs'])}")
            if not ag["sums_identical"]:
                lines.append("- extracted sums differ across runs")
            if ag["contribution_names"]["only_some_runs"]:
                lines.append(f"- contributions in only some runs: {', '.join(ag['contribution_names']['only_some_runs'])}")
            for f in ag["field_diffs"]:
                lines.append(f"- field differs: {f}")
        lines.append("")
    return "\n".join(lines)


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--runs", type=int, default=3)
    ap.add_argument("--model", default=PDF_DEFAULT_MODEL)
    ap.add_argument("--only", nargs="*", help="candidate ids to run")
    ap.add_argument("--replay", action="store_true",
                    help="re-verify each deed's recorded fixture instead of calling the API (one run per deed)")
    args = ap.parse_args()

    rows = list(csv.DictReader(open(REF / "candidates.tsv"), delimiter="\t"))
    if args.only:
        rows = [r for r in rows if r["id"] in args.only]
    register = load_register()
    client = None if args.replay else harness_client()  # same key file as run_audit; None falls back to SDK credential resolution
    OUT.mkdir(parents=True, exist_ok=True)
    summary = []
    for row in rows:
        print(f"== {row['id']} ({row['council']} {row['planning_ref']})", file=sys.stderr, flush=True)
        summary.append(evaluate(row, args.runs, args.model, register, client, args.replay))
        (OUT / "summary.json").write_text(json.dumps(summary, indent=2, default=str))
    (OUT / "summary.md").write_text(markdown(summary, args.model))
    print(markdown(summary, args.model))
    return 0


if __name__ == "__main__":
    sys.exit(main())
