"""BootLoops-style programmatic gatekeeper for extracted S106 data.

Every check is a hard, exact assertion over numbers. Nothing here is a model judgement.
Each check's inputs are also *grounded*: a value counts as supported only when it appears
in its cited evidence snippet (digits, or number words) and that snippet re-locates on the
cited page of the OCR text. Outcomes:

  * PASS                  the assertion holds and every input is grounded
  * REVIEW                the assertion holds, but some input isn't grounded, so its
                          correctness is unproven; needs a human or a better citation
  * FAIL, PARSER_ERROR    an input is missing or ungrounded, and the assertion fails:
                          suspect the extraction before the deed
  * FAIL, DOCUMENT_DISCREPANCY
                          the assertion fails although every input is grounded. This is
                          what the deed *appears* to say through OCR, not an established
                          error in the deed: confirm against the original before relying on it
  * SKIP                  the deed doesn't state what the check needs
"""
from __future__ import annotations

import re
from collections import Counter
from dataclasses import dataclass, field
from decimal import ROUND_HALF_UP, Decimal
from enum import Enum

from schema import AffordableBasis, Evidence, Section106Agreement
from triggers import classify_cited, imposes_payment

PCT_TOLERANCE = Decimal("0.05")  # percentage points; deeds quote 2 dp


class Status(str, Enum):
    PASS = "PASS"
    REVIEW = "REVIEW"
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


_UNITS = {w: i for i, w in enumerate(
    "zero one two three four five six seven eight nine ten eleven twelve thirteen "
    "fourteen fifteen sixteen seventeen eighteen nineteen".split())}
_TENS = {w: 10 * i for i, w in enumerate(
    "_ _ twenty thirty forty fifty sixty seventy eighty ninety".split()) if w != "_"}
_SCALES = {"thousand": 1_000, "million": 1_000_000}


# Fallback when the squashed snippet is not contiguous in the OCR text (two-column
# definition pages splice the left-column term into the sentence at line wraps).
# Tokens must appear in order; intervening text is allowed only as
#   * at most 1 non-digit char within a line (OCR reads "£" as "f"), or
#   * a line wrap followed by up to MAX_GAP chars of left-column fragment,
# and no gap may hold digits or number words, even joined ("andtwenty"). Numbers
# are digit-bounded and never skipped; up to MAX_SKIP_SHARE of plain words may be.
MAX_GAP = 40          # chars of left-column fragment allowed at a line start
MIN_TOKENS = 4        # shorter snippets must match contiguously
MAX_SKIP_SHARE = 0.1  # share of plain words OCR may have garbled ("fifty" -> "fty")


def _tokens(s: str) -> list[str]:
    """Words and numbers; '£12,650.00' -> '1265000', the same digits squash() keeps."""
    return [re.sub(r"[.,]", "", t) for t in re.findall(r"[a-z]+|\d+(?:[.,]\d+)*", s.lower())]


def _is_number(tok: str) -> bool:
    return tok.isdigit() or tok in _UNITS or tok in _TENS or tok in _SCALES or tok == "hundred"


# "one", "two", "ten" also sit inside common words (network, maintenance), so they
# count at a word's start or end or after "and". A skipped gap stops where matching
# resumes, so an amount word OCR joined to a column fragment ("Contributionandten
# pounds") ends its gap word; this rejects some terms ending in them ("Zone").
_SHORT_NUMS = ("one", "two", "ten")
_NUM_WORD = re.compile("|".join(sorted(
    (w for w in [*_UNITS, *_TENS, *_SCALES, "hundred"] if w not in _SHORT_NUMS), key=len, reverse=True)))
_SHORT_NUM_EDGE = re.compile(r"(?:^|and)(?:{0})|(?:{0})$".format("|".join(_SHORT_NUMS)))


def _numeric_gap(text: str) -> bool:
    """Could this skipped OCR text carry part of an amount the snippet left out?"""
    return (any(c.isdigit() for c in text)
            or any(_NUM_WORD.search(w) or _SHORT_NUM_EDGE.search(w) for w in re.findall(r"[a-z]+", text)))


def _layout_match(tokens: list[str], page: str) -> bool:
    raw = page.lower()
    idx = [i for i, c in enumerate(raw) if c.isascii() and c.isalnum()]
    hay = "".join(raw[i] for i in idx)  # == squash(page)
    max_skips = int(sum(not _is_number(t) for t in tokens) * MAX_SKIP_SHARE)
    n = len(tokens)

    def gap_ok(end: int, start: int, extra: int) -> bool:
        gap = raw[idx[end - 1] + 1:idx[start]]
        lines = gap.split("\n")
        if _numeric_gap(gap) or len(squash(lines[0])) > 1 + extra:
            return False
        return len(lines) == 1 or (not any(squash(l) for l in lines[1:-1])
                                   and len(squash(lines[-1])) <= MAX_GAP + extra)

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


# Only inside a number: a digit on one side and a digit or number punctuation on the
# other, never a letter, so "paragraph1of" stays as it is.
_OCR_ZERO = re.compile(r"(?<=\d)[oO](?=[\d%,.)]|$)|(?<![A-Za-z])[oO](?=\d)", re.M)
_OCR_ONE = re.compile(r"(?<=\d)[lI|](?=[\d%,.)]|$)|(?<![A-Za-z])[lI|](?=\d)", re.M)


def ocr_digits(text: str) -> str:
    """Undo OCR letter-for-digit swaps next to digits: '5o%' -> '50%', 'l0' -> '10'."""
    return _OCR_ONE.sub("1", _OCR_ZERO.sub("0", text))


def evidence_found(ev: Evidence | None, pages: dict[int, str]) -> bool:
    if ev is None or ev.page not in pages:
        return False
    needle = squash(ev.snippet)
    if not needle:
        return False
    page = ocr_digits(pages[ev.page])
    if needle in squash(page):
        return True
    tokens = _tokens(ev.snippet)
    return len(tokens) >= MIN_TOKENS and _layout_match(tokens, page)



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


# --------------------------------------------------------------------------- grounding

@dataclass(frozen=True)
class Input:
    """One value a check depends on, with the evidence cited for it."""
    label: str
    value: object  # int | Decimal | str | None
    evidence: Evidence | None


_NUMBER = re.compile(r"\d{1,3}(?:,\d{3})+(?:\.\d+)?|\d+(?:\.\d+)?")
_WORD_RUN = re.compile(r"(?:(?:%s)[\s-]*)+" % "|".join(sorted(
    [*_UNITS, *_TENS, *_SCALES, "hundred", "and"], key=len, reverse=True)), re.I)


def numbers_in(text: str) -> set[Decimal]:
    """Every number written in `text`, as digits ("£10,058.60", "5o%" via OCR repair) or words."""
    out = {Decimal(m.replace(",", "")) for m in _NUMBER.findall(ocr_digits(text))}
    for run in _WORD_RUN.findall(text):
        run = re.sub(r"(?:\band\b|[\s-])+$", "", run.strip(), flags=re.I)
        n = _words_to_int(run) if run else None
        if n is not None:
            out.add(Decimal(n))
    return out


def value_in_snippet(value, snippet: str) -> bool:
    """Does the snippet state this value? Numbers compare numerically; text compares squashed."""
    if isinstance(value, bool) or value is None:
        return False
    if isinstance(value, (int, Decimal, float)):
        return Decimal(str(value)) in numbers_in(snippet)
    return bool(squash(str(value))) and squash(str(value)) in squash(snippet)


def grounding_problem(inp: Input, pages: dict[int, str]) -> str | None:
    """Why `inp` isn't supported by its evidence, or None if it is."""
    if inp.evidence is None:
        return f"{inp.label}: no evidence cited"
    if not value_in_snippet(inp.value, inp.evidence.snippet):
        return f"{inp.label}: {inp.value} not stated in its cited snippet"
    if not evidence_found(inp.evidence, pages):
        return f"{inp.label}: cited snippet not re-locatable on p{inp.evidence.page}"
    return None


def _pages(evs) -> list[int]:
    return sorted({e.page for e in evs if e})


def resolve(cid: str, desc: str, holds: bool, inputs: list[Input], pages: dict[int, str],
            expected: str | None = None, actual: str | None = None, extra_problems: list[str] = ()) -> CheckResult:
    """Turn an assertion result plus its inputs' grounding into a CheckResult."""
    pg = _pages(i.evidence for i in inputs)
    missing = [i.label for i in inputs if i.value is None]
    if missing:
        return CheckResult(cid, desc, Status.FAIL, expected, actual, ErrorClass.PARSER_ERROR,
                           "not extracted: " + ", ".join(missing), pg)
    problems = [p for p in (grounding_problem(i, pages) for i in inputs) if p] + list(extra_problems)
    if holds:
        if not problems:
            return CheckResult(cid, desc, Status.PASS, expected, actual, pages=pg)
        return CheckResult(cid, desc, Status.REVIEW, expected, actual, None,
                           "holds, but not grounded: " + "; ".join(problems), pg)
    if problems:
        return CheckResult(cid, desc, Status.FAIL, expected, actual, ErrorClass.PARSER_ERROR,
                           "ungrounded: " + "; ".join(problems), pg)
    return CheckResult(cid, desc, Status.FAIL, expected, actual, ErrorClass.DOCUMENT_DISCREPANCY,
                       "every input is stated in its cited snippet and the snippet re-locates in the OCR text; "
                       "confirm against the original deed before relying on this", pg)


# --------------------------------------------------------------------------- checks

def check_unit_consistency(a: Section106Agreement, pages) -> CheckResult:
    h = a.housing
    cid, desc = "H1", "Market units + affordable units == total residential units"
    inputs = [Input("total units", h.total_units, h.total_units_evidence),
              Input("market units", h.market_units, h.market_units_evidence),
              Input("affordable units", h.affordable_units, h.affordable_units_evidence)]
    if h.total_units is not None and h.affordable_units is not None and h.market_units is None:
        return CheckResult(cid, desc, Status.SKIP, detail="deed does not state market unit count separately",
                           pages=_pages(i.evidence for i in inputs))
    holds = None not in (h.total_units, h.market_units, h.affordable_units) and \
        h.market_units + h.affordable_units == h.total_units
    actual = (f"{h.market_units} + {h.affordable_units} = {h.market_units + h.affordable_units}"
              if None not in (h.market_units, h.affordable_units) else None)
    return resolve(cid, desc, holds, inputs, pages, str(h.total_units), actual)


def check_tenure_units(a: Section106Agreement, pages) -> CheckResult:
    h = a.housing
    cid, desc = "H2", "Sum of tenure units == affordable units"
    if not h.tenures:
        return CheckResult(cid, desc, Status.FAIL, error_class=ErrorClass.PARSER_ERROR,
                           detail="no tenure breakdown extracted")
    inputs = [Input("affordable units", h.affordable_units, h.affordable_units_evidence)] + \
             [Input(f"{t.tenure.value} units", t.units, t.evidence) for t in h.tenures]
    if any(i.value is None for i in inputs):
        return resolve(cid, desc, False, inputs, pages)
    got = sum(t.units for t in h.tenures)
    breakdown = " + ".join(f"{t.units} {t.tenure.value}" for t in h.tenures)
    return resolve(cid, desc, got == h.affordable_units, inputs, pages, str(h.affordable_units), f"{breakdown} = {got}")


def check_tenure_rooms(a: Section106Agreement, pages) -> CheckResult:
    h = a.housing
    cid, desc = "H3", "Sum of tenure habitable rooms == affordable habitable rooms"
    if h.affordable_habitable_rooms is None or not any(t.habitable_rooms for t in h.tenures):
        return CheckResult(cid, desc, Status.SKIP, detail="habitable-room breakdown not stated")
    inputs = [Input("affordable habitable rooms", h.affordable_habitable_rooms, h.affordable_units_evidence)] + \
             [Input(f"{t.tenure.value} habitable rooms", t.habitable_rooms, t.evidence) for t in h.tenures]
    if any(i.value is None for i in inputs):
        return resolve(cid, desc, False, inputs, pages)
    got = sum(t.habitable_rooms for t in h.tenures)
    breakdown = " + ".join(f"{t.habitable_rooms} {t.tenure.value}" for t in h.tenures)
    return resolve(cid, desc, got == h.affordable_habitable_rooms, inputs, pages,
                   str(h.affordable_habitable_rooms), f"{breakdown} = {got}")


def check_claimed_pct(a: Section106Agreement, pages) -> CheckResult:
    h = a.housing
    cid = "H4"
    basis = h.claimed_affordable_basis
    desc = f"Claimed affordable % vs calculated % (basis: {basis.value if basis else 'not stated'})"
    if h.claimed_affordable_pct is None:
        return CheckResult(cid, desc, Status.SKIP, detail="no affordable % stated in the deed")
    claimed = Input("claimed %", h.claimed_affordable_pct, h.claimed_affordable_evidence)
    if basis == AffordableBasis.HABITABLE_ROOMS:
        num = Input("affordable habitable rooms", h.affordable_habitable_rooms, h.affordable_units_evidence)
        # the schema carries no evidence for a scheme-wide habitable-room total
        den = Input("total habitable rooms", h.total_habitable_rooms, None)
    elif basis == AffordableBasis.UNITS:
        num = Input("affordable units", h.affordable_units, h.affordable_units_evidence)
        den = Input("total units", h.total_units, h.total_units_evidence)
    else:
        # Never fall back to units: a floorspace or unstated basis compared by units would be
        # a different measure, and a match or mismatch would mean nothing.
        why = ("floorspace basis: floorspace figures are not extracted" if basis == AffordableBasis.FLOORSPACE
               else "the deed's basis for the % was not extracted")
        return CheckResult(cid, desc, Status.SKIP, f"{h.claimed_affordable_pct}%", None, None,
                           f"cannot compute: {why}", _pages([h.claimed_affordable_evidence]))
    if basis == AffordableBasis.HABITABLE_ROOMS and num.value is not None and not den.value:
        # Deeds commonly quote the % by habitable room without the scheme-wide room count;
        # the check can't be closed from the deed alone. Report the implied denominator instead.
        implied = (Decimal(num.value) * 100 / h.claimed_affordable_pct).quantize(Decimal("0.1"))
        return CheckResult(cid, desc, Status.SKIP, f"{h.claimed_affordable_pct}%", None, None,
                           f"total habitable rooms not stated in the deed; implied total = {num.value}/"
                           f"{h.claimed_affordable_pct}% = {implied} rooms (confirm against the approved plans)",
                           _pages([claimed.evidence, num.evidence]))
    if num.value is None or not den.value:
        return resolve(cid, desc, False, [claimed, num, den], pages, f"{h.claimed_affordable_pct}%")
    calc = (Decimal(num.value) * 100 / Decimal(den.value)).quantize(Decimal("0.01"), ROUND_HALF_UP)
    diff = abs(calc - h.claimed_affordable_pct)
    r = resolve(cid, desc, diff <= PCT_TOLERANCE, [claimed, num, den], pages, f"{h.claimed_affordable_pct}%",
                f"{num.value}/{den.value} = {calc}%")
    if r.status == Status.FAIL:
        r.detail += f"; off by {diff} pp"
    return r


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
    amount = Input("amount", c.amount_gbp, c.evidence)
    out.append(resolve(cid + ".amt", f"{c.name}: amount stated in cited evidence", True, [amount], pages,
                       actual=f"£{c.amount_gbp:,.2f}"))
    # figures vs words: the deed states every sum twice, so an exact cross-check is free
    desc = f"{c.name}: figures == words"
    if not c.amount_in_words:
        out.append(CheckResult(cid + ".words", desc, Status.SKIP, detail="no words form captured", pages=pg))
    else:
        w = words_to_amount(c.amount_in_words)
        if w is None:
            out.append(CheckResult(cid + ".words", desc, Status.FAIL, f"£{c.amount_gbp:,.2f}", c.amount_in_words,
                                   ErrorClass.PARSER_ERROR, "words form not parseable", pg))
        else:
            out.append(resolve(cid + ".words", desc, w == c.amount_gbp,
                               [amount, Input("words form", c.amount_in_words, c.evidence)], pages,
                               f"£{c.amount_gbp:,.2f}", f"£{w:,.2f}"))
    out.extend(check_trigger(c, pages, cid, pg))
    return out


def _trigger_clause_problem(c, pages) -> str | None:
    """Why the cited clause can't be trusted as this contribution's payment covenant, if it can't.

    The timing phrase alone proves little: "Prior to Occupation" also schedules assessments. So
    the clause must itself be in the deed (on the phrase's page, or running onto the next), the
    phrase must sit inside it, and its wording must impose a payment.
    """
    ev = c.trigger_evidence
    if squash(ocr_digits(ev.snippet)) not in squash(ocr_digits(c.trigger_clause)):
        return "cited trigger phrase is not part of the cited clause"
    spread = {ev.page: pages[ev.page] + "\n" + pages.get(ev.page + 1, "")}
    if not evidence_found(Evidence(page=ev.page, snippet=c.trigger_clause), spread):
        return f"cited payment clause not re-locatable in source (p{ev.page})"
    if not imposes_payment(c.trigger_clause):
        # e.g. "Prior to Occupation ... the Council shall assess the amount payable" schedules an
        # assessment; the payment covenant is elsewhere.
        return "cited clause does not impose a payment"
    return None


def check_trigger(c, pages, cid: str, pg: list[int]) -> list[CheckResult]:
    """Trigger located, its cited phrase is in the deed, and the phrase decides the label."""
    desc = f"{c.name}: payment trigger located"
    if not (c.trigger_event and c.trigger_clause):
        return [CheckResult(cid + ".trig", desc, Status.FAIL, error_class=ErrorClass.PARSER_ERROR,
                            detail="no trigger clause matched in the schedules", pages=pg)]
    tpg = _pages([c.trigger_evidence])
    if not evidence_found(c.trigger_evidence, pages):
        where = f"p{c.trigger_evidence.page}" if c.trigger_evidence else "no evidence cited"
        return [CheckResult(cid + ".trig", desc, Status.FAIL, actual=c.trigger_event.value,
                            error_class=ErrorClass.PARSER_ERROR,
                            detail=f"cited trigger phrase not re-locatable in source ({where})", pages=tpg)]
    problem = _trigger_clause_problem(c, pages)
    if problem:
        return [CheckResult(cid + ".trig", desc, Status.FAIL, actual=c.trigger_event.value,
                            error_class=ErrorClass.PARSER_ERROR, detail=problem, pages=tpg)]
    out = [CheckResult(cid + ".trig", desc, Status.PASS, actual=c.trigger_event.value, pages=tpg)]
    desc = f"{c.name}: trigger label follows from cited phrase"
    derived = classify_cited(c.trigger_evidence.snippet, c.trigger_clause)
    note = f"model proposed '{c.trigger_event_model.value}'" if c.trigger_event_model else ""
    if derived is None:
        out.append(CheckResult(cid + ".tlabel", desc, Status.SKIP, actual=c.trigger_event.value,
                               detail="phrase not in the trigger vocabulary; label unverified", pages=tpg))
    elif derived == c.trigger_event:
        out.append(CheckResult(cid + ".tlabel", desc, Status.PASS, derived.value, c.trigger_event.value,
                               detail=note, pages=tpg))
    else:
        out.append(CheckResult(cid + ".tlabel", desc, Status.FAIL, derived.value, c.trigger_event.value,
                               ErrorClass.PARSER_ERROR, "label disagrees with the cited phrase", tpg))
    return out


def _row_on_page(counts: list[int], page: str | None) -> bool:
    """Is there a line on the page stating all of these counts (repeats included)?"""
    want = Counter(Decimal(n) for n in counts)
    return page is not None and any(
        not (want - Counter(Decimal(m.replace(",", "")) for m in _NUMBER.findall(ocr_digits(ln))))
        for ln in page.splitlines())


def check_mix_table(a: Section106Agreement, pages) -> list[CheckResult]:
    """Schedule mix table is an independent restatement of the tenure counts: cross-check it.

    Cells carry no snippets of their own, so a row counts as grounded when a line on the
    table's cited page states all of its counts.
    """
    h, t = a.housing, a.housing.mix_table
    if t is None:
        return [CheckResult("H5", "Approved mix table: rows sum to subtotals", Status.SKIP,
                            detail="no mix table found")]
    page = pages.get(t.evidence.page) if t.evidence else None
    row_problems = [f"row '{r.unit_type}' not found on p{t.evidence.page if t.evidence else '?'}"
                    for r in t.rows if not _row_on_page(list(r.units_by_tenure.values()), page)]
    sub_problem = ([] if not t.subtotals or _row_on_page(list(t.subtotals.values()), page)
                   else ["sub-total row not found on the cited page"])
    total = Input("table total", t.total_units, t.evidence)
    out = []
    for ten, sub in t.subtotals.items():
        col = sum(r.units_by_tenure.get(ten, 0) for r in t.rows)
        parts = " + ".join(str(r.units_by_tenure.get(ten, 0)) for r in t.rows)
        out.append(resolve("H5", f"Mix table: Σ {ten.value} rows == sub-total", col == sub, [], pages, str(sub),
                           f"{parts} = {col}", row_problems + sub_problem))
    if t.subtotals:
        got = sum(t.subtotals.values())
        out.append(resolve("H5", "Mix table: Σ sub-totals == table total", got == t.total_units, [total], pages,
                           str(t.total_units), str(got), sub_problem))
    for alloc in h.tenures:
        if alloc.tenure not in t.subtotals:
            continue
        out.append(resolve("H6", f"Definitions vs mix table: {alloc.tenure.value} units",
                           alloc.units == t.subtotals[alloc.tenure],
                           [Input(f"{alloc.tenure.value} units", alloc.units, alloc.evidence)], pages,
                           f"{t.subtotals[alloc.tenure]} (schedule)", f"{alloc.units} (definition)", sub_problem))
    for label, defn, tbl in (("units", h.affordable_units, t.total_units),
                             ("habitable rooms", h.affordable_habitable_rooms, t.total_habitable_rooms)):
        if defn is None or tbl is None:
            continue
        out.append(resolve("H6", f"Definitions vs mix table: affordable {label}", defn == tbl,
                           [Input(f"affordable {label} (definition)", defn, h.affordable_units_evidence),
                            Input(f"affordable {label} (table)", tbl, t.evidence)], pages,
                           f"{tbl} (schedule)", f"{defn} (definition)"))
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
