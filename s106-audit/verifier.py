"""BootLoops-style programmatic gatekeeper for extracted S106 data.

Every check is a hard, exact assertion over numbers. Nothing here is a model
judgement. When a check fails, we re-locate each input value in the raw page text:

  * PARSER_ERROR          an input is missing, or its evidence can't be found on the
                          cited page. The extraction is suspect, so fix the parser
                          before blaming the deed.
  * DOCUMENT_DISCREPANCY  every input is verbatim in the deed and the numbers still
                          disagree. The deed itself is inconsistent, so escalate to
                          a planning officer.
"""
from __future__ import annotations

import re
from dataclasses import dataclass, field
from decimal import ROUND_HALF_UP, Decimal
from enum import Enum

from schema import AffordableBasis, Evidence, Section106Agreement

PCT_TOLERANCE = Decimal("0.05")  # percentage points; deeds quote 2 dp


class Status(str, Enum):
    PASS = "PASS"
    FAIL = "FAIL"
    SKIP = "SKIP"


class ErrorClass(str, Enum):
    PARSER_ERROR = "Parser Error"
    DOCUMENT_DISCREPANCY = "Document Discrepancy"


@dataclass
class CheckResult:
    check_id: str
    description: str
    status: Status
    expected: str | None = None
    actual: str | None = None
    error_class: ErrorClass | None = None
    detail: str = ""
    pages: list[int] = field(default_factory=list)


# --------------------------------------------------------------------------- helpers

def squash(s: str) -> str:
    """OCR drops/adds spaces arbitrarily; compare on alphanumerics only."""
    return re.sub(r"[^0-9a-z]", "", s.lower())


# Line-wrap-aware token match, for snippets read cleanly off the page image. On
# two-column definition pages OCR splices the left-column defined term into the
# right-column sentence at every line wrap:
#     "Affordable Housing means the sumof f10,058.60(ten thousand and fifty eightpounds
#     MonitoringContribution" andsixtypence)Index Linked ...
# so the squashed snippet is not a contiguous substring. Tokens must still appear in
# order, and intervening OCR text is allowed only where the deed's layout explains it:
#   * within a line, at most 1 non-digit char (OCR reads "£" as "f");
#   * across a line wrap, nothing left on the line above, then up to MAX_GAP chars of
#     left-column fragment at the start of the next line, holding no number words.
# Numbers and number words are never skipped and digits must be digit-bounded, so a
# changed amount ("12,650" -> "12,560") or a dropped word ("six hundred [and fifty]
# pounds") still fails. Up to MAX_SKIP_SHARE of the other words may be OCR-garbled.
MAX_GAP = 40          # chars of left-column fragment allowed at a line start
MIN_TOKENS = 4        # shorter snippets must match contiguously
MAX_SKIP_SHARE = 0.1  # share of plain words OCR may have garbled ("fifty" -> "fty")


def _tokens(s: str) -> list[str]:
    """Words and numbers; '£12,650.00' -> '1265000', the same digits squash() keeps."""
    return [re.sub(r"[.,]", "", t) for t in re.findall(r"[a-z]+|\d+(?:[.,]\d+)*", s.lower())]


def _is_number(tok: str) -> bool:
    return tok.isdigit() or tok in _UNITS or tok in _TENS or tok in _SCALES or tok == "hundred"


def _layout_match(tokens: list[str], page: str) -> bool:
    raw = page.lower()
    idx = [i for i, c in enumerate(raw) if c.isascii() and c.isalnum()]
    hay = "".join(raw[i] for i in idx)  # == squash(page)
    max_skips = int(sum(not _is_number(t) for t in tokens) * MAX_SKIP_SHARE)
    n = len(tokens)

    def gap_ok(end: int, start: int, extra: int) -> bool:
        lines = raw[idx[end - 1] + 1:idx[start]].split("\n")
        tail, head = squash(lines[0]), squash(lines[-1])
        if len(lines) == 1:
            return len(tail) <= 1 + extra and not any(c.isdigit() for c in tail)
        return (len(tail) <= 1 + extra and not any(c.isdigit() for c in tail)
                and not any(squash(l) for l in lines[1:-1])
                and len(head) <= MAX_GAP + extra
                and not any(_is_number(w) for w in re.findall(r"[a-z]+", lines[-1])))

    def occurrences(tok: str, lo: int, hi: int):
        i = hay.find(tok, lo, hi)
        while i != -1:
            end = i + len(tok)
            if not tok.isdigit() or ((i == 0 or not hay[i - 1].isdigit())
                                     and (end == len(hay) or not hay[end].isdigit())):
                yield i, end
            i = hay.find(tok, i + 1, hi)

    seen: set[tuple[int, int, int, int]] = set()

    def search(i: int, pos: int | None, extra: int, skips: int) -> bool:
        # pos: end of the last matched token (None before the first match);
        # extra: chars of slack owed to skipped (OCR-garbled) words since then
        if i == n:
            return pos is not None
        key = (i, -1 if pos is None else pos, extra, skips)
        if key in seen:
            return False
        seen.add(key)
        tok = tokens[i]
        if pos is None:
            cands = occurrences(tok, 0, len(hay))
        else:
            hi = pos + MAX_GAP + 2 * extra + 2 + len(tok)
            cands = ((s, e) for s, e in occurrences(tok, pos, hi) if gap_ok(pos, s, extra))
        if any(search(i + 1, e, 0, skips) for _, e in cands):
            return True
        return (not _is_number(tok) and skips < max_skips
                and search(i + 1, pos, extra + len(tok) + 1, skips + 1))

    return search(0, None, 0, 0)


def evidence_found(ev: Evidence | None, pages: dict[int, str]) -> bool:
    if ev is None or ev.page not in pages:
        return False
    needle = squash(ev.snippet)
    if not needle:
        return False
    if needle in squash(pages[ev.page]):
        return True
    tokens = _tokens(ev.snippet)
    return len(tokens) >= MIN_TOKENS and _layout_match(tokens, pages[ev.page])


_UNITS = {w: i for i, w in enumerate(
    "zero one two three four five six seven eight nine ten eleven twelve thirteen "
    "fourteen fifteen sixteen seventeen eighteen nineteen".split())}
_TENS = {w: 10 * i for i, w in enumerate(
    "_ _ twenty thirty forty fifty sixty seventy eighty ninety".split()) if w != "_"}
_SCALES = {"thousand": 1_000, "million": 1_000_000}


def _words_to_int(text: str) -> int | None:
    total = current = 0
    seen = False
    # tolerate OCR run-togethers like "fiftyeight" by greedy tokenising
    vocab = sorted(list(_UNITS) + list(_TENS) + list(_SCALES) + ["hundred", "and"], key=len, reverse=True)
    s = re.sub(r"[^a-z]", "", text.lower())
    i = 0
    while i < len(s):
        for w in vocab:
            if s.startswith(w, i):
                break
        else:
            return None
        i += len(w)
        if w == "and":
            continue
        seen = True
        if w in _UNITS:
            current += _UNITS[w]
        elif w in _TENS:
            current += _TENS[w]
        elif w == "hundred":
            current *= 100
        else:
            total += current * _SCALES[w]
            current = 0
    return total + current if seen else None


def words_to_amount(text: str) -> Decimal | None:
    """'ten thousand and fifty eight pounds and sixty pence' -> Decimal('10058.60')."""
    t = text.lower()
    m = re.match(r"(.*?)pounds?(.*)", t, re.S)
    if not m:
        return None
    pounds = _words_to_int(m.group(1))
    if pounds is None:
        return None
    pence_txt = re.sub(r"[^a-z]", "", m.group(2)).replace("pence", "").replace("penny", "")
    pence = _words_to_int(pence_txt) if pence_txt.strip("and") else 0
    return Decimal(pounds) + Decimal(pence or 0) / 100


def _classify(evs: list[Evidence | None], values: list, pages: dict[int, str]) -> tuple[ErrorClass, str]:
    missing = [i for i, v in enumerate(values) if v is None]
    if missing:
        return ErrorClass.PARSER_ERROR, f"{len(missing)} input value(s) were not extracted"
    unverified = [e for e in evs if not evidence_found(e, pages)]
    if unverified:
        where = ", ".join(f"p{e.page}" if e else "no evidence" for e in unverified)
        return ErrorClass.PARSER_ERROR, f"evidence not re-locatable in source ({where})"
    return ErrorClass.DOCUMENT_DISCREPANCY, "all inputs verified verbatim in the deed"


def _pages(evs) -> list[int]:
    return sorted({e.page for e in evs if e})


# --------------------------------------------------------------------------- checks

def check_unit_consistency(a: Section106Agreement, pages) -> CheckResult:
    h = a.housing
    cid, desc = "H1", "Market units + affordable units == total residential units"
    vals = [h.total_units, h.market_units, h.affordable_units]
    evs = [h.total_units_evidence, h.market_units_evidence, h.affordable_units_evidence]
    if h.total_units is None or h.affordable_units is None:
        cls, why = _classify(evs, vals, pages)
        return CheckResult(cid, desc, Status.FAIL, error_class=cls, detail=why)
    if h.market_units is None:
        return CheckResult(cid, desc, Status.SKIP, detail="deed does not state market unit count separately",
                           pages=_pages(evs))
    got = h.market_units + h.affordable_units
    if got == h.total_units:
        return CheckResult(cid, desc, Status.PASS, str(h.total_units), str(got), pages=_pages(evs))
    cls, why = _classify(evs, vals, pages)
    return CheckResult(cid, desc, Status.FAIL, str(h.total_units), f"{h.market_units} + {h.affordable_units} = {got}",
                       cls, why, _pages(evs))


def check_tenure_units(a: Section106Agreement, pages) -> CheckResult:
    h = a.housing
    cid, desc = "H2", "Sum of tenure units == affordable units"
    evs = [h.affordable_units_evidence] + [t.evidence for t in h.tenures]
    vals = [h.affordable_units] + [t.units for t in h.tenures]
    if not h.tenures:
        return CheckResult(cid, desc, Status.FAIL, error_class=ErrorClass.PARSER_ERROR,
                           detail="no tenure breakdown extracted")
    if any(v is None for v in vals):
        cls, why = _classify(evs, vals or [None], pages)
        return CheckResult(cid, desc, Status.FAIL, error_class=cls, detail=why)
    got = sum(t.units for t in h.tenures)
    breakdown = " + ".join(f"{t.units} {t.tenure.value}" for t in h.tenures)
    if got == h.affordable_units:
        return CheckResult(cid, desc, Status.PASS, str(h.affordable_units), f"{breakdown} = {got}", pages=_pages(evs))
    cls, why = _classify(evs, vals, pages)
    return CheckResult(cid, desc, Status.FAIL, str(h.affordable_units), f"{breakdown} = {got}", cls, why, _pages(evs))


def check_tenure_rooms(a: Section106Agreement, pages) -> CheckResult:
    h = a.housing
    cid, desc = "H3", "Sum of tenure habitable rooms == affordable habitable rooms"
    if h.affordable_habitable_rooms is None or not any(t.habitable_rooms for t in h.tenures):
        return CheckResult(cid, desc, Status.SKIP, detail="habitable-room breakdown not stated")
    evs = [h.affordable_units_evidence] + [t.evidence for t in h.tenures]
    vals = [h.affordable_habitable_rooms] + [t.habitable_rooms for t in h.tenures]
    if any(v is None for v in vals):
        cls, why = _classify(evs, vals, pages)
        return CheckResult(cid, desc, Status.FAIL, error_class=cls, detail=why)
    got = sum(t.habitable_rooms for t in h.tenures)
    breakdown = " + ".join(f"{t.habitable_rooms} {t.tenure.value}" for t in h.tenures)
    if got == h.affordable_habitable_rooms:
        return CheckResult(cid, desc, Status.PASS, str(h.affordable_habitable_rooms), f"{breakdown} = {got}",
                           pages=_pages(evs))
    cls, why = _classify(evs, vals, pages)
    return CheckResult(cid, desc, Status.FAIL, str(h.affordable_habitable_rooms), f"{breakdown} = {got}",
                       cls, why, _pages(evs))


def check_claimed_pct(a: Section106Agreement, pages) -> CheckResult:
    h = a.housing
    cid = "H4"
    basis = h.claimed_affordable_basis
    desc = f"Claimed affordable % vs calculated % (basis: {basis.value if basis else '?'})"
    if h.claimed_affordable_pct is None:
        return CheckResult(cid, desc, Status.SKIP, detail="no affordable % stated in the deed")
    if basis == AffordableBasis.HABITABLE_ROOMS:
        num, den = h.affordable_habitable_rooms, h.total_habitable_rooms
    else:
        num, den = h.affordable_units, h.total_units
    evs = [h.claimed_affordable_evidence, h.affordable_units_evidence, h.total_units_evidence]
    if basis == AffordableBasis.HABITABLE_ROOMS and num is not None and not den:
        # Deeds commonly quote the % by habitable room without the scheme-wide room count;
        # the check can't be closed from the deed alone. Report the implied denominator instead.
        implied = (Decimal(num) * 100 / h.claimed_affordable_pct).quantize(Decimal("0.1"))
        return CheckResult(cid, desc, Status.SKIP, f"{h.claimed_affordable_pct}%", None, None,
                           f"total habitable rooms not stated in the deed; implied total = {num}/"
                           f"{h.claimed_affordable_pct}% = {implied} rooms (confirm against the approved plans)",
                           _pages(evs))
    if num is None or not den:
        cls, why = _classify(evs, [num, den], pages)
        return CheckResult(cid, desc, Status.FAIL, f"{h.claimed_affordable_pct}%", None, cls, why, _pages(evs))
    calc = (Decimal(num) * 100 / Decimal(den)).quantize(Decimal("0.01"), ROUND_HALF_UP)
    diff = abs(calc - h.claimed_affordable_pct)
    actual = f"{num}/{den} = {calc}%"
    if diff <= PCT_TOLERANCE:
        return CheckResult(cid, desc, Status.PASS, f"{h.claimed_affordable_pct}%", actual, pages=_pages(evs))
    cls, why = _classify(evs, [num, den, h.claimed_affordable_pct], pages)
    return CheckResult(cid, desc, Status.FAIL, f"{h.claimed_affordable_pct}%", actual, cls,
                       f"{why}; off by {diff} pp", _pages(evs))


def check_contribution(c, pages, idx: int) -> list[CheckResult]:
    out = []
    pg = _pages([c.evidence])
    cid = f"F{idx}"
    if c.amount_gbp is None and c.contingent:
        out.append(CheckResult(cid + ".amt", f"{c.name}: amount extracted", Status.SKIP,
                               detail="contingent / formula-based: no fixed sum in the deed", pages=pg))
    elif c.amount_gbp is None:
        out.append(CheckResult(cid + ".amt", f"{c.name}: amount extracted", Status.FAIL,
                               error_class=ErrorClass.PARSER_ERROR, detail="no figure parsed", pages=pg))
        return out
    if c.amount_gbp is None:
        return out
    # figures vs words: the deed states every sum twice, so an exact cross-check is free
    if c.amount_in_words:
        w = words_to_amount(c.amount_in_words)
        desc = f"{c.name}: figures == words"
        if w is None:
            out.append(CheckResult(cid + ".words", desc, Status.FAIL, f"£{c.amount_gbp:,.2f}", c.amount_in_words,
                                   ErrorClass.PARSER_ERROR, "words form not parseable", pg))
        elif w == c.amount_gbp:
            out.append(CheckResult(cid + ".words", desc, Status.PASS, f"£{c.amount_gbp:,.2f}", f"£{w:,.2f}", pages=pg))
        else:
            # Both forms come from the same snippet; if it's verbatim on the page the deed disagrees with itself.
            cls = ErrorClass.DOCUMENT_DISCREPANCY if evidence_found(c.evidence, pages) else ErrorClass.PARSER_ERROR
            out.append(CheckResult(cid + ".words", desc, Status.FAIL, f"£{c.amount_gbp:,.2f}", f"£{w:,.2f}", cls,
                                   "figures and words disagree", pg))
    else:
        out.append(CheckResult(cid + ".words", f"{c.name}: figures == words", Status.SKIP,
                               detail="no words form captured", pages=pg))
    desc = f"{c.name}: payment trigger located"
    if c.trigger_event and c.trigger_clause:
        out.append(CheckResult(cid + ".trig", desc, Status.PASS, actual=c.trigger_event.value,
                               pages=_pages([c.trigger_evidence])))
    else:
        out.append(CheckResult(cid + ".trig", desc, Status.FAIL, error_class=ErrorClass.PARSER_ERROR,
                               detail="no trigger clause matched in the schedules", pages=pg))
    return out


def check_mix_table(a: Section106Agreement, pages) -> list[CheckResult]:
    """Schedule mix table is an independent restatement of the tenure counts: cross-check it."""
    h, t = a.housing, a.housing.mix_table
    if t is None:
        return [CheckResult("H5", "Approved mix table: rows sum to subtotals", Status.SKIP,
                            detail="no mix table found")]
    pg = _pages([t.evidence])
    out = []
    for ten, sub in t.subtotals.items():
        col = sum(r.units_by_tenure.get(ten, 0) for r in t.rows)
        desc = f"Mix table: Σ {ten.value} rows == sub-total"
        parts = " + ".join(str(r.units_by_tenure.get(ten, 0)) for r in t.rows)
        ok = col == sub
        out.append(CheckResult("H5", desc, Status.PASS if ok else Status.FAIL, str(sub), f"{parts} = {col}",
                               None if ok else _classify([t.evidence], [sub, col], pages)[0],
                               "" if ok else "table rows and sub-total disagree", pg))
    if t.subtotals:
        desc = "Mix table: Σ sub-totals == table total"
        got = sum(t.subtotals.values())
        ok = got == t.total_units
        out.append(CheckResult("H5", desc, Status.PASS if ok else Status.FAIL, str(t.total_units), str(got),
                               None if ok else _classify([t.evidence], [t.total_units], pages)[0], "", pg))
    for alloc in h.tenures:
        if alloc.tenure not in t.subtotals:
            continue
        desc = f"Definitions vs mix table: {alloc.tenure.value} units"
        ok = alloc.units == t.subtotals[alloc.tenure]
        evs = [alloc.evidence, t.evidence]
        out.append(CheckResult("H6", desc, Status.PASS if ok else Status.FAIL, f"{t.subtotals[alloc.tenure]} (schedule)",
                               f"{alloc.units} (definition)", None if ok else _classify(evs, [alloc.units], pages)[0],
                               "", _pages(evs)))
    for label, defn, tbl in (("units", h.affordable_units, t.total_units),
                             ("habitable rooms", h.affordable_habitable_rooms, t.total_habitable_rooms)):
        if defn is None or tbl is None:
            continue
        evs = [h.affordable_units_evidence, t.evidence]
        ok = defn == tbl
        out.append(CheckResult("H6", f"Definitions vs mix table: affordable {label}", Status.PASS if ok else Status.FAIL,
                               f"{tbl} (schedule)", f"{defn} (definition)",
                               None if ok else _classify(evs, [defn, tbl], pages)[0], "", _pages(evs)))
    return out


def verify(a: Section106Agreement, pages: dict[int, str]) -> list[CheckResult]:
    results = [
        check_unit_consistency(a, pages),
        check_tenure_units(a, pages),
        check_tenure_rooms(a, pages),
        check_claimed_pct(a, pages),
        *check_mix_table(a, pages),
    ]
    for i, c in enumerate(a.contributions, 1):
        results.extend(check_contribution(c, pages, i))
    return results
