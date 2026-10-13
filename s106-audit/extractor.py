"""Extract a Section106Agreement from a deed PDF.

Two backends fill the same Pydantic schema:

  * rules  (default) deterministic regexes over the definitions clause and schedules.
           Tolerant of OCR space-dropping: every pattern matches with optional
           whitespace between characters.
  * llm    Claude structured outputs (`client.messages.parse`) over the targeted
           pages. Needs Anthropic credentials. Values still go through the same
           evidence-backed verifier, so the model is never trusted on its own.
"""
from __future__ import annotations

import re
from datetime import date, datetime
from decimal import Decimal
from pathlib import Path

import pdfplumber

from ocr import page_texts
from schema import (AffordableBasis, AgreementMetadata, Evidence, FinancialContribution, HousingProvision, MixRow, MixTable,
                    Section106Agreement, Tenure, TenureAllocation, TriggerEvent)

Q_OPEN = "\"“”"  # double quotes only: apostrophes occur inside terms ("Children's Play")
Q_CLOSE = "\"”“"


# --------------------------------------------------------------------------- text utils

def fuzzy(phrase: str) -> str:
    """Regex for `phrase` that tolerates any whitespace (incl. none) between characters."""
    chars = [c for c in phrase if not c.isspace()]
    return r"\s*".join(re.escape(c) for c in chars)


def tidy_term(s: str) -> str:
    s = re.sub(r"(?<=[a-z])(?=[A-Z])", " ", s)  # "AffordableHousing" -> "Affordable Housing"
    return re.sub(r"\s+", " ", s).strip(" \"“”'’")


def snippet(text: str, start: int, end: int, pad: int = 0) -> str:
    return re.sub(r"\s+", " ", text[max(0, start - pad):end + pad]).strip()


def to_decimal(s: str) -> Decimal:
    return Decimal(s.replace(",", ""))


# --------------------------------------------------------------------------- section finding

def locate_sections(pages: dict[int, str]) -> tuple[list[int], list[int]]:
    """(definition pages, schedule pages)."""
    start = next((n for n, t in pages.items() if re.search(fuzzy("Definitions and Interpretation"), t, re.I)), 1)
    # definitions end where the operative clauses begin ("2. ... Construction of this Agreement" or similar)
    end = next((n for n, t in pages.items()
                if n > start and re.search(r"^\s*2\.?\s*" + fuzzy("Construction of this Agreement"), t, re.I | re.M)),
               None)
    if end is None:
        end = next((n for n, t in pages.items()
                    if n > start and re.search(r"^\s*1\.2\b", t, re.M)), start + 20)
    defs = [n for n in pages if start <= n <= end]
    sched_start = next((n for n, t in pages.items()
                        if n > end and re.search(r"^\s*" + fuzzy("SCHEDULE 1") + r"\b", t, re.M)), end + 1)
    scheds = [n for n in pages if n >= sched_start]
    return defs, scheds


TERM_START = re.compile(rf"^(?:[^A-Za-z{Q_OPEN}]|[A-Za-z](?=\s+[{Q_OPEN}])){{0,4}}\s*[{Q_OPEN}](?P<t>[^{Q_CLOSE}\n]{{2,80}}?)"
                        rf"(?:(?P<closed>[{Q_CLOSE}]+)|\s*(?=means|has\s*the\s*meaning)|\s*$)", re.M)


TITLE_PREFIX = re.compile(r"^\s*((?:(?:[A-Z][\w'’\-]*|(?:and|of|the|in|for)\b|&)[\s,]*)+)(?=\s*[a-z(]|\s*$)")


def parse_definitions(pages: dict[int, str], def_pages: list[int]) -> dict[str, tuple[str, int, str]]:
    """{term: (body, page, raw_term_line)} from the two-column definitions clause.

    Long terms wrap: `"Affordable Housing   means the sum ...` / `MonitoringContribution"  and sixty...`.
    The closing quote on the following line marks the end of the term.
    """
    lines: list[tuple[int, str]] = [(n, ln) for n in def_pages for ln in pages[n].splitlines()]
    starts = []
    for i, (n, ln) in enumerate(lines):
        if TERM_START.match(ln):
            starts.append(i)
    out: dict[str, tuple[str, int, str]] = {}
    for k, i in enumerate(starts):
        n, ln = lines[i]
        m = TERM_START.match(ln)
        term = m.group("t")
        rest = ln[m.end():]
        body_lines = [rest]
        nxt = starts[k + 1] if k + 1 < len(starts) else len(lines)
        j = i + 1
        if not m.group("closed"):
            # The term wraps down the left column. Lines before the closing quote carry a
            # Title Case fragment of the term followed by right-column body text.
            for _ in range(3):
                if j >= nxt:
                    break
                ln_j = lines[j][1]
                cm = re.match(rf"\s*([A-Za-z][^{Q_CLOSE}\n]{{0,60}}?)[{Q_CLOSE}]+(.*)", ln_j)
                if cm:
                    term = f"{term} {cm.group(1)}"
                    body_lines.append(cm.group(2))
                    j += 1
                    break
                tm = TITLE_PREFIX.match(ln_j)
                if not tm or not tm.group(1).strip():
                    break
                term = f"{term} {tm.group(1)}"
                body_lines.append(ln_j[tm.end(1):])
                j += 1
        body_lines += [lines[x][1] for x in range(j, nxt)]
        body = re.sub(r"^\s*means\s*", "", "\n".join(body_lines).strip())
        out[tidy_term(term)] = (body, n, ln)
    return out


# --------------------------------------------------------------------------- financial contributions

AMOUNT_RE = re.compile(
    r"(?:sum|contribution|maximum)\s*(?:of)?\s*(?:up\s*to\s*a\s*maximum\s*(?:of)?\s*)?[f£€]?\s*"
    r"(?P<num>\d{1,3}(?:\s*,\s*\d{3})+(?:\s*\.\s*\d{2})?|\d+(?:\.\d{2})?)\s*"
    r"\(\s*(?P<words>[^()]*?pounds?[^()]*?)\)",
    re.I | re.S,
)

TRIGGERS = [
    (TriggerEvent.PRE_IMPLEMENTATION, "prior to Implementation"),
    (TriggerEvent.PRE_IMPLEMENTATION, "on or before Implementation"),
    (TriggerEvent.PRE_IMPLEMENTATION, "Not to Implement"),
    (TriggerEvent.PRE_COMMENCEMENT, "prior to Commencement"),
    (TriggerEvent.PRE_COMMENCEMENT, "on or before Commencement"),
    (TriggerEvent.PRE_OCCUPATION, "prior to first Occupation"),
    (TriggerEvent.PRE_OCCUPATION, "prior to the first Occupation"),
    (TriggerEvent.PRE_OCCUPATION, "prior to Occupation"),
    (TriggerEvent.PRE_OCCUPATION, "on or before first Occupation"),
    (TriggerEvent.PRE_OCCUPATION, "on or before Occupation"),
    (TriggerEvent.ON_COMPLETION, "on completion of this Agreement"),
    (TriggerEvent.ON_COMPLETION, "on the date of this Agreement"),
    (TriggerEvent.ON_DEMAND, "within 28 days of"),
    (TriggerEvent.ON_DEMAND, "within 20 Working Days of"),
    (TriggerEvent.OTHER, "in accordance with the procedure"),
]
PAY_RE = re.compile(r"p\s*a\s*y|p\s*a\s*i\s*d", re.I)


def find_trigger(names: list[str], pages: dict[int, str], search_pages: list[int]):
    """Find the covenant that makes any of `names` payable and classify its trigger.

    `names` is the contribution plus any group terms that include it (e.g. "Site and
    Development Contributions"), because deeds often impose one trigger on a group.
    Searches the operative clauses and schedules, i.e. everything after the definitions.
    """
    best = None
    for name in names:
        name_re = re.compile(fuzzy("the " + name) + r"(?!\s*M\s*e\s*t\s*h\s*o\s*d)", re.I)
        for n in search_pages:
            t = pages[n]
            for m in name_re.finditer(t):
                win_start = max(0, m.start() - 200)
                window = t[win_start:m.end() + 220]
                if not PAY_RE.search(window):
                    continue
                for event, phrase in TRIGGERS:
                    for tm in re.finditer(fuzzy(phrase), window, re.I):
                        dist = abs((win_start + tm.start()) - m.start())
                        if best is None or dist < best[0]:
                            # clause = the numbered paragraph around the match
                            ps = t.rfind("\n", 0, m.start())
                            pe = t.find("\n", win_start + tm.end())
                            best = (dist, event, n, snippet(t, ps + 1, pe if pe > 0 else len(t)),
                                    snippet(t, win_start + tm.start(), win_start + tm.end()))
    if best:
        _, event, n, clause, phrase_snip = best
        return event, clause, Evidence(page=n, snippet=phrase_snip)
    return None, None, None


def contribution_groups(defs) -> dict[str, list[str]]:
    """{member contribution: [group terms that list it]} from plural '...Contributions' definitions."""
    groups: dict[str, list[str]] = {}
    members = [t for t in defs if t.endswith("Contribution") or t.endswith("Cost")]
    for g, (body, _, _) in defs.items():
        if not g.endswith("Contributions"):
            continue
        sq = re.sub(r"\W", "", body.lower())
        for mbr in members:
            if re.sub(r"\W", "", mbr.lower()) in sq:
                groups.setdefault(mbr, []).append(g)
    return groups


def extract_contributions(defs, pages, search_pages) -> list[FinancialContribution]:
    out = []
    groups = contribution_groups(defs)
    for term, (body, n, _raw) in defs.items():
        if not re.search(r"Contribution$|Administration Cost$", term):
            continue
        names = [term] + groups.get(term, [])
        m = AMOUNT_RE.search(body)
        if not m:
            # Formula-based or review-contingent sums have no fixed figure in the deed.
            contingent = bool(re.search(r"formula|calculated|may\s*arise", body, re.I))
            event, clause, tev = find_trigger(names, pages, search_pages)
            out.append(FinancialContribution(
                name=term, contingent=contingent, trigger_event=event, trigger_clause=clause,
                trigger_evidence=tev, evidence=Evidence(page=n, snippet=snippet(body, 0, 160))))
            continue
        amount = to_decimal(re.sub(r"\s", "", m.group("num")))
        ev = Evidence(page=n, snippet=snippet(body, m.start("num"), m.end("words")))
        event, clause, tev = find_trigger(names, pages, search_pages)
        out.append(FinancialContribution(
            name=term,
            amount_gbp=amount,
            amount_in_words=re.sub(r"\s+", " ", m.group("words")).strip(),
            index_linked=bool(re.search(r"[Il1]\s*n\s*d\s*e\s*x\s*L\s*i\s*n\s*k", body[:m.end() + 320], re.I)),
            is_cap=bool(re.search(r"up\s*to\s*a\s*maximum|lesser\s*amount", body[:m.end() + 300], re.I)),
            trigger_event=event, trigger_clause=clause, trigger_evidence=tev, evidence=ev,
        ))
    return out


# --------------------------------------------------------------------------- housing

UNITS_RE = re.compile(r"(?P<u>\d{1,4})\s*" + fuzzy("Residential Units"), re.I)
HR_RE = re.compile(r"\(?\s*(?P<hr>\d{1,4})\s*\)?\s*" + fuzzy("Habitable Rooms"), re.I)

TENURE_TERMS = [
    (Tenure.SOCIAL_RENT, r"Social Rent(ed)? (Housing )?Units"),
    (Tenure.LONDON_AFFORDABLE_RENT, r"London Affordable Rent(ed)? (Housing )?Units"),
    (Tenure.LONDON_LIVING_RENT, r"London Living Rent (Housing )?Units"),
    (Tenure.SHARED_OWNERSHIP, r"Shared Ownership (Housing )?Units"),
    (Tenure.INTERMEDIATE, r"Intermediate (Housing )?Units"),
    (Tenure.DISCOUNT_MARKET_RENT, r"Discount(ed)? Market Rent (Housing )?Units"),
]


def _units_from_def(defs, pattern) -> tuple[int | None, int | None, Evidence | None]:
    for term, (body, n, _) in defs.items():
        if re.fullmatch(pattern, term, re.I):
            head = body[:400]
            um = UNITS_RE.search(head)
            hm = HR_RE.search(head)
            if um:
                end = hm.end() if hm and hm.start() - um.end() < 250 else um.end()
                return (int(um.group("u")), int(hm.group("hr")) if hm and hm.start() - um.end() < 250 else None,
                        Evidence(page=n, snippet=snippet(head, um.start(), end)))
            return None, None, Evidence(page=n, snippet=snippet(head, 0, 120))
    return None, None, None


PCT_RE = re.compile(
    r"(?P<pct>\d{1,2}(?:\s*\.\s*\d{1,2})?)\s*(?:%|" + fuzzy("per cent") + r")\s*" + fuzzy("affordable") +
    r"(?:\s*" + fuzzy("housing") + r")?\s*" + fuzzy("by") + r"\s*(?P<basis>" + fuzzy("habitable room") + "|" +
    fuzzy("unit") + ")", re.I)


TENURE_HEADERS = [(Tenure.SOCIAL_RENT, r"Social\s*Rent"), (Tenure.LONDON_AFFORDABLE_RENT, r"London\s*Affordable\s*Rent"),
                  (Tenure.LONDON_LIVING_RENT, r"London\s*Living\s*Rent"), (Tenure.SHARED_OWNERSHIP, r"Shared\s*Ownership"),
                  (Tenure.INTERMEDIATE, r"Intermediate")]


def extract_mix_table(pages) -> MixTable | None:
    """Parse the 'Approved Affordable Housing Mix' schedule table (unit type rows x tenure columns)."""
    for n, t in pages.items():
        if not re.search(fuzzy("Affordable Housing Mix"), t, re.I) or not re.search(fuzzy("Sub-Total"), t, re.I):
            continue
        lines = t.splitlines()
        hdr_i = next((i for i, ln in enumerate(lines) if sum(bool(re.search(p, ln, re.I)) for _, p in TENURE_HEADERS) >= 2), None)
        if hdr_i is None:
            continue
        hdr = lines[hdr_i]
        cols = sorted(((re.search(p, hdr, re.I).start(), ten) for ten, p in TENURE_HEADERS if re.search(p, hdr, re.I)))
        cols = [ten for _, ten in cols]
        table = MixTable(evidence=Evidence(page=n, snippet=snippet(t, t.find(hdr), t.find(hdr) + len(hdr))))
        for ln in lines[hdr_i + 1:]:
            nums = [int(x) for x in re.findall(r"\d+", re.sub(r"^\s*\d\s*-?\s*bed", "", ln, flags=re.I))]
            if re.match(r"\s*\d\s*-?\s*bed", ln, re.I) and len(nums) == len(cols):
                table.rows.append(MixRow(unit_type=re.match(r"\s*(\d)", ln).group(1) + " bed",
                                         units_by_tenure=dict(zip(cols, nums))))
            elif re.match(r"\s*" + fuzzy("Sub-Total"), ln, re.I) and len(nums) == len(cols):
                table.subtotals = dict(zip(cols, nums))
            elif re.match(r"\s*Total", ln, re.I):
                m = re.search(r"(\d+)\s*\(\s*(\d+)\s*" + fuzzy("habitable rooms"), ln, re.I)
                if m:
                    table.total_units, table.total_habitable_rooms = int(m.group(1)), int(m.group(2))
                elif nums:
                    table.total_units = nums[0]
        return table
    return None


def extract_housing(defs, pages) -> HousingProvision:
    h = HousingProvision()
    h.total_units, h.total_habitable_rooms, h.total_units_evidence = _units_from_def(defs, r"Residential Units?(\(s\))?")
    if h.total_units is None:
        # Southwark-style deeds define Residential Unit generically; the count lives in "Development".
        for term, (body, n, _) in defs.items():
            m = re.search(r"(\d{1,4})\s*" + fuzzy("residential") + r"\s*(?:" + fuzzy("apartments") + "|" + fuzzy("units") +
                          "|" + fuzzy("dwellings") + ")", body, re.I)
            if term == "Development" and m:
                h.total_units = int(m.group(1))
                h.total_units_evidence = Evidence(page=n, snippet=snippet(body, m.start(), m.end()))
    h.mix_table = extract_mix_table(pages)
    h.market_units, _, h.market_units_evidence = _units_from_def(defs, r"Market Housing Units")
    h.affordable_units, h.affordable_habitable_rooms, h.affordable_units_evidence = \
        _units_from_def(defs, r"Affordable Housing Units")
    for tenure, pat in TENURE_TERMS:
        u, hr, ev = _units_from_def(defs, pat)
        if ev:
            h.tenures.append(TenureAllocation(tenure=tenure, units=u, habitable_rooms=hr, evidence=ev))
    for n, t in pages.items():
        m = PCT_RE.search(t)
        if m:
            h.claimed_affordable_pct = to_decimal(re.sub(r"\s", "", m.group("pct")))
            h.claimed_affordable_basis = (AffordableBasis.HABITABLE_ROOMS if "hab" in m.group("basis").lower()
                                          else AffordableBasis.UNITS)
            h.claimed_affordable_evidence = Evidence(page=n, snippet=snippet(t, m.start(), m.end()))
            break
    return h


# --------------------------------------------------------------------------- metadata

def extract_metadata(pdf_path: Path, pages: dict[int, str], page_count: int, text_source: str) -> AgreementMetadata:
    first = "\n".join(pages.get(n, "") for n in (1, 2))
    ref = re.search(r"\b(\d{2}\s*/\s*AP\s*/\s*\d{4})\b", first)
    leg = re.search(r"Ref\s*:\s*(\S+)", first)
    site = re.search(r"known\s*as\s*\n?(.+?)(?:\n|$)", pages.get(1, ""), re.I)
    dated = re.search(r"Dated\s*(\d{1,2})\s*([A-Za-z]+)\s*(\d{4})", pages.get(1, ""))
    agreement_date = None
    if dated:
        month = dated.group(2)[:3]
        if month.lower() == "jan" or re.match(r"j[ae]nu", dated.group(2), re.I):  # OCR: "Januan", "Jenuan"
            month = "Jan"
        try:
            agreement_date = datetime.strptime(f"{dated.group(1)} {month} {dated.group(3)}", "%d %b %Y").date()
        except ValueError:
            pass
    parties = [tidy_term(p) for p in re.findall(r"^\s*(?:\w\s+)?\(\d\)\s*([A-Z][A-Z &,.]+?)(?:\s*\(|\s+of\b|$)",
                                                pages.get(2, ""), re.M)]
    return AgreementMetadata(
        council="London Borough of Southwark" if re.search("southwark", first, re.I) else "Unknown",
        planning_reference=re.sub(r"\s", "", ref.group(1)) if ref else None,
        legal_reference=leg.group(1) if leg else None,
        agreement_date=agreement_date,
        site_address=tidy_term(site.group(1)) if site else None,
        parties=parties,
        source_pdf=str(pdf_path),
        page_count=page_count,
        text_source=text_source,
    )


# --------------------------------------------------------------------------- LLM backend

def extract_llm(pdf_path: Path, pages: dict[int, str], target_pages: list[int], base: Section106Agreement,
                model: str = "claude-opus-5-5") -> Section106Agreement:
    """Re-extract housing + contributions with Claude structured outputs.

    Metadata comes from the rules pass. The model sees only the targeted pages,
    each tagged with its page number so it can cite evidence the verifier re-checks.
    """
    import anthropic
    from pydantic import BaseModel

    class LLMOutput(BaseModel):
        housing: HousingProvision
        contributions: list[FinancialContribution]

    corpus = "\n\n".join(f"<page n={n}>\n{pages[n]}\n</page>" for n in target_pages)
    client = anthropic.Anthropic()
    resp = client.messages.parse(
        model=model,
        max_tokens=16000,
        output_config={"effort": "medium"},
        messages=[{"role": "user", "content": (
            "Below is OCR text (spaces often missing; '£' often read as 'f') from the definitions and "
            "schedules of a UK Section 106 agreement. Extract the residential unit counts, affordable tenure "
            "breakdown, any stated affordable percentage and its basis, and every financial contribution "
            "with its amount in figures, its amount in words exactly as written, whether it is Index Linked, "
            "and the clause that triggers payment. For every value give evidence: the page number and a short "
            "verbatim snippet copied from that page. Use null where the deed does not state a value; do not "
            "compute or infer numbers.\n\n" + corpus)}],
        output_format=LLMOutput,
    )
    if resp.stop_reason == "refusal" or resp.parsed_output is None:
        raise RuntimeError(f"LLM extraction returned no structured output (stop_reason={resp.stop_reason})")
    out = base.model_copy(deep=True)
    out.housing = resp.parsed_output.housing
    out.contributions = resp.parsed_output.contributions
    out.extraction_backend = f"llm:{model}"
    return out


# --------------------------------------------------------------------------- entry point

def extract(pdf_path: str | Path, backend: str = "rules", model: str = "claude-opus-5-5", verbose: bool = False
            ) -> tuple[Section106Agreement, dict[int, str]]:
    pdf_path = Path(pdf_path)
    with pdfplumber.open(pdf_path) as pdf:
        page_count = len(pdf.pages)
        has_text = any(len((p.extract_text() or "").strip()) > 50 for p in pdf.pages[:3])
    pages = page_texts(pdf_path, verbose=verbose)
    def_pages, sched_pages = locate_sections(pages)
    defs = parse_definitions(pages, def_pages)

    warnings = []
    if not has_text:
        warnings.append("PDF has no text layer; values come from OCR and evidence matching is whitespace-insensitive")
    agreement = Section106Agreement(
        metadata=extract_metadata(pdf_path, pages, page_count, "text-layer" if has_text else "ocr"),
        housing=extract_housing(defs, pages),
        contributions=extract_contributions(defs, pages, [n for n in pages if n > def_pages[-1]]),
        warnings=warnings,
    )
    if backend == "llm":
        agreement = extract_llm(pdf_path, pages, def_pages + sched_pages, agreement, model=model)
    if verbose:
        import sys
        print(f"  definitions: pages {def_pages[0]}-{def_pages[-1]} ({len(defs)} terms); "
              f"schedules from page {sched_pages[0] if sched_pages else '?'}", file=sys.stderr)
    return agreement, pages
