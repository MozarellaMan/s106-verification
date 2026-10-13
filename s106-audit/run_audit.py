#!/usr/bin/env python3
"""Audit a Section 106 deed: extract -> verify -> terminal report + JSON.

    python s106-audit/run_audit.py [PDF] [--backend rules|llm|llm-pdf] [--model M] [--cascade]
                                   [--llm-mode auto|live|replay|record] [--out data/processed/<stem>.audit.json]

Exit code: 0 all checks pass/skip, 1 parser errors present, 2 document discrepancies present.
"""
from __future__ import annotations

import argparse
import json
import sys
from collections import Counter
from dataclasses import asdict
from pathlib import Path

from extractor import CASCADE_MODEL, cascade_llm_pdf, extract
from verifier import ErrorClass, Status, verify

DEFAULT_PDF = "./data/raw/southwark_s106_sample.pdf"

C = {"PASS": "\033[32m", "FAIL": "\033[31m", "SKIP": "\033[33m", "dim": "\033[2m", "b": "\033[1m", "0": "\033[0m"}


def color(s: str, k: str) -> str:
    return f"{C[k]}{s}{C['0']}" if sys.stdout.isatty() else s


def report(a, results) -> None:
    m, h = a.metadata, a.housing
    w = 78
    print(color("=" * w, "b"))
    print(color(" SECTION 106 AUDIT", "b"))
    print("=" * w)
    print(f" Council        {m.council}")
    print(f" Planning ref   {m.planning_reference}    Legal ref {m.legal_reference}")
    print(f" Dated          {m.agreement_date}    Site {m.site_address}")
    print(f" Parties        {'; '.join(m.parties)}")
    print(f" Source         {m.source_pdf} ({m.page_count} pp, {m.text_source}, backend={a.extraction_backend})")

    print("\n" + color(" HOUSING", "b"))
    print(f"   Total residential units   {h.total_units}" + (f"  ({h.total_habitable_rooms} hab rooms)" if h.total_habitable_rooms else ""))
    print(f"   Market units              {h.market_units}")
    print(f"   Affordable units          {h.affordable_units}" + (f"  ({h.affordable_habitable_rooms} hab rooms)" if h.affordable_habitable_rooms else ""))
    for t in h.tenures:
        hr = f"  ({t.habitable_rooms} hab rooms)" if t.habitable_rooms else ""
        print(f"     - {t.tenure.value:<22}  {t.units}{hr}   {color(f'p{t.evidence.page}', 'dim') if t.evidence else ''}")
    if h.claimed_affordable_pct is not None:
        print(f"   Claimed affordable        {h.claimed_affordable_pct}% by {h.claimed_affordable_basis.value}"
              f"   {color(f'p{h.claimed_affordable_evidence.page}', 'dim')}")

    print("\n" + color(" FINANCIAL CONTRIBUTIONS", "b"))
    print(f"   {'Obligation':<40}{'Amount':>14}  {'Idx':<4}{'Trigger':<26}")
    for c in a.contributions:
        amt = f"£{c.amount_gbp:,.2f}" if c.amount_gbp is not None else "—"
        trig = c.trigger_event.value if c.trigger_event else "—"
        pg = f"p{c.evidence.page}" if c.evidence else ""
        print(f"   {c.name[:39]:<40}{amt:>14}  {'Y' if c.index_linked else 'N':<4}{trig:<26}{color(pg, 'dim')}")
    print(f"   {'TOTAL (stated maxima, unindexed)':<40}{f'£{a.total_contributions_gbp:,.2f}':>14}")

    print("\n" + color(" VERIFICATION GATE", "b"))
    for r in results:
        tag = color(f"[{r.status.value}]", r.status.value)
        line = f"   {tag} {r.check_id:<6} {r.description}"
        print(line)
        if r.status != Status.PASS or r.actual:
            bits = []
            if r.expected is not None:
                bits.append(f"expected {r.expected}")
            if r.actual is not None:
                bits.append(f"got {r.actual}")
            if r.error_class:
                bits.append(color(r.error_class.value.upper(), "FAIL"))
            if r.detail:
                bits.append(r.detail)
            if r.pages:
                bits.append("p" + ",".join(map(str, r.pages)))
            print(color(f"            {' | '.join(bits)}", "dim"))

    counts = Counter(r.status for r in results)
    classes = Counter(r.error_class for r in results if r.error_class)
    print("\n" + "-" * w)
    print(f" {counts[Status.PASS]} pass · {counts[Status.FAIL]} fail · {counts[Status.SKIP]} skip   |   "
          f"{classes[ErrorClass.PARSER_ERROR]} parser error(s) · "
          f"{classes[ErrorClass.DOCUMENT_DISCREPANCY]} document discrepanc(ies)")
    for wmsg in a.warnings:
        print(color(f" ! {wmsg}", "SKIP"))
    print("=" * w)


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("pdf", nargs="?", default=DEFAULT_PDF)
    ap.add_argument("--backend", choices=["rules", "llm", "llm-pdf"], default="rules")
    ap.add_argument("--model", help="Claude model (default: claude-opus-5-5 for llm, claude-haiku-5-5 for llm-pdf)")
    ap.add_argument("--cascade", action="store_true",
                    help="llm-pdf: re-read fields with Parser Errors on --cascade-model, keep only verified improvements")
    ap.add_argument("--cascade-model", default=CASCADE_MODEL)
    ap.add_argument("--llm-mode", choices=["auto", "live", "replay", "record"], default="auto",
                    help="llm-pdf: auto = live if Anthropic credentials resolve, else replay the recorded fixture")
    ap.add_argument("--out", help="JSON output path (default data/processed/<stem>.audit.json)")
    ap.add_argument("-q", "--quiet", action="store_true")
    args = ap.parse_args()

    pdf = Path(args.pdf)
    if args.cascade and args.backend != "llm-pdf":
        ap.error("--cascade needs --backend llm-pdf")
    agreement, pages = extract(pdf, backend=args.backend, model=args.model, verbose=not args.quiet,
                               llm_mode=args.llm_mode)
    results = verify(agreement, pages)
    if args.cascade:
        agreement = cascade_llm_pdf(pdf, agreement, pages, results, model=args.cascade_model, mode=args.llm_mode,
                                    verbose=not args.quiet)
        results = verify(agreement, pages)
    report(agreement, results)

    out = Path(args.out) if args.out else pdf.resolve().parent.parent / "processed" / f"{pdf.stem}.audit.json"
    out.parent.mkdir(parents=True, exist_ok=True)
    payload = {
        "agreement": agreement.model_dump(mode="json"),
        "verification": [{**asdict(r), "status": r.status.value,
                          "error_class": r.error_class.value if r.error_class else None} for r in results],
    }
    out.write_text(json.dumps(payload, indent=2))
    print(f"\nJSON written to {out}")

    classes = {r.error_class for r in results if r.error_class}
    if ErrorClass.DOCUMENT_DISCREPANCY in classes:
        return 2
    return 1 if ErrorClass.PARSER_ERROR in classes else 0


if __name__ == "__main__":
    sys.exit(main())
