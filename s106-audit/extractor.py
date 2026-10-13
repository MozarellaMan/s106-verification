"""Extract a Section106Agreement from a deed PDF.

Two backends fill the same Pydantic schema:

  * rules  (default) deterministic regexes over the definitions clause and schedules.
           Tolerant of OCR space-dropping: every pattern matches with optional
           whitespace between characters.
  * llm-pdf Claude reads the scanned page images (see the LLM section below).
"""
from __future__ import annotations

import re
import threading
from datetime import datetime
from decimal import Decimal
from pathlib import Path

import pdfplumber
from pydantic import BaseModel, Field

from ocr import page_texts
from triggers import classify_cited
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
                            # ... to the end of the sentence, so "Not to Implement ... until the
                            # X Contributions have been\npaid" keeps its payment wording
                            pe = t.find(".", win_start + tm.end(), win_start + tm.end() + 400)
                            best = (dist, event, n, snippet(t, ps + 1, pe if pe > 0 else len(t)),
                                    snippet(t, win_start + tm.start(), win_start + tm.end()))
    if best:
        _, event, n, clause, phrase_snip = best
        return classify_cited(phrase_snip, clause) or event, clause, Evidence(page=n, snippet=phrase_snip)
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
                # cite the total line itself, so the totals are grounded in their own snippet
                table.evidence = Evidence(page=n, snippet=snippet(ln, 0, len(ln)))
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


# --------------------------------------------------------------------------- LLM native-PDF backend
#
# The definitions and payment/mix pages are split out with pypdfium2 and sent as a base64
# PDF document block, so OCR noise ("f" for "£", "ftypounds") never reaches the extraction.
# The model cites evidence by page; the verifier matches those snippets against the
# independent RapidOCR text (never model output) to classify failures. Offline, responses
# replay from s106-audit/fixtures/.

PDF_DEFAULT_MODEL = "claude-haiku-5-5"
CASCADE_MODEL = "claude-opus-5-5"
# Models that accept server-side refusal fallbacks (Haiku has none).
FALLBACK_MODELS = {"claude-fable-5-1", "claude-opus-5-5", "claude-opus-5", "claude-sonnet-5-5"}
FIXTURE_DIR = Path(__file__).resolve().parent / "fixtures"


class Cite(BaseModel):
    page: int = Field(description="1-based page position within the attached PDF (not the printed page number)")
    snippet: str = Field(description="Under 30 words copied character for character from that page, containing the "
                                     "value; empty string (page 0) when the deed does not state it")


class WireCount(BaseModel):
    tenure: Tenure
    units: int


class WireMixRow(BaseModel):
    unit_type: str = Field(description="e.g. '1 bed'")
    counts: list[WireCount]


class WireMixTable(BaseModel):
    rows: list[WireMixRow]
    subtotals: list[WireCount]
    total_units: int | None
    total_habitable_rooms: int | None
    evidence: Cite


class WireTenure(BaseModel):
    tenure: Tenure
    units: int | None
    habitable_rooms: int | None
    evidence: Cite


class WireHousing(BaseModel):
    total_units: int | None
    total_units_evidence: Cite
    market_units: int | None
    market_units_evidence: Cite
    affordable_units: int | None
    affordable_units_evidence: Cite
    affordable_habitable_rooms: int | None
    total_habitable_rooms: int | None
    tenures: list[WireTenure]
    mix_table: WireMixTable = Field(description="Empty rows when the deed has no mix table")
    claimed_affordable_pct: str | None = Field(description="Percentage exactly as printed, digits only, e.g. '35.75'")
    claimed_affordable_basis: AffordableBasis | None
    claimed_affordable_evidence: Cite


class WireContribution(BaseModel):
    name: str = Field(description="Defined term exactly as written, without quotes")
    amount_gbp: str | None = Field(description="Figure as printed, digits and decimal point only, e.g. '12650' or '10058.60'")
    amount_in_words: str | None = Field(description="Words form inside the brackets, exactly as written")
    index_linked: bool
    is_cap: bool
    contingent: bool
    trigger_event: TriggerEvent | None
    trigger_clause: str | None
    trigger_evidence: Cite
    evidence: Cite


class WireExtraction(BaseModel):
    """Structured-output schema. Lists instead of dicts (strict JSON schema has no additionalProperties maps),
    and evidence is never nullable: each `Cite | None` union inflates the compiled grammar past the API limit."""
    housing: WireHousing
    contributions: list[WireContribution]


PDF_PROMPT = """\
The attached PDF holds selected pages of a scanned UK Section 106 agreement: the definitions clause, \
plus the operative clauses and schedules that set payment triggers and the affordable housing mix.

{task}

Rules:
- Cite pages by their position in the attached PDF (first attached page = 1), never by the printed page number.
- Every evidence snippet must be copied character for character from the cited page and must contain the value it supports. Keep it under 30 words.
- Report only what the deed states. Do not compute, total or infer numbers. Use null where the deed is silent.
"""

HOUSING_TASK = """\
Housing: the total number of residential units in the Development; market units; affordable units and their \
habitable rooms; each affordable tenure (from definitions such as "Social Rented Housing Units" or \
"Intermediate Housing Units") with its units and habitable rooms; the approved affordable housing mix table if \
present (one row per unit type, the per-tenure sub-totals, the table total and its habitable rooms); and any \
stated affordable housing percentage with its basis (habitable rooms or units)."""

CONTRIB_TASK = """\
Financial contributions: {which}. For each give the figure, the words form exactly as written, whether it is \
Index Linked, whether the figure is a maximum ("up to a maximum of", "or such lesser amount"), and whether it is \
contingent (payable only by formula or review outcome, with no fixed figure: then amount_gbp is null). Find the \
covenant that makes it payable; a deed often imposes one trigger on a group term such as "Site and Development \
Contributions". The trigger is the clause that says when the money must be paid or becomes payable. If that \
covenant only refers elsewhere (e.g. "in accordance with the procedure in paragraph 3 of Schedule 11"), follow \
the reference and use the clause there that says when payment falls due; a clause that only times an assessment, \
review or notice is not the trigger. Give that clause verbatim as trigger_clause, classify it as trigger_event, and \
cite the trigger phrase (e.g. "within 20 Working Days of", "on or before Implementation") as trigger_evidence."""

ALL_CONTRIBUTIONS = 'every defined term whose name ends in "Contribution", plus the "Administration Cost"'


FULL_DEED_PAGES = 60  # shorter deeds go whole: cheap, and no layout-specific page selection to get wrong


def llm_pdf_pages(pages: dict[int, str]) -> list[int]:
    """Deed pages to send: all of a short deed; otherwise the definitions plus later pages that
    impose payment or hold the mix table (chosen from the OCR text: 42 of 109 pages for Southwark).
    """
    if len(pages) <= FULL_DEED_PAGES:
        return sorted(pages)
    def_pages, _ = locate_sections(pages)
    later = [n for n, t in pages.items() if n > def_pages[-1] and (
        (re.search(fuzzy("Contribution") + "|" + fuzzy("Administration Cost"), t, re.I) and PAY_RE.search(t))
        or re.search(fuzzy("Affordable Housing Mix"), t, re.I))]
    return def_pages + later


_PDFIUM_LOCK = threading.Lock()  # PDFium is not thread-safe; concurrent use corrupts its global state


def pdf_subset_b64(pdf_path: Path, page_numbers: list[int]) -> str:
    import base64
    import io

    import pypdfium2 as pdfium

    with _PDFIUM_LOCK:
        src = pdfium.PdfDocument(str(pdf_path))
        dst = pdfium.PdfDocument.new()
        try:
            dst.import_pages(src, [n - 1 for n in page_numbers])
            buf = io.BytesIO()
            dst.save(buf)
        finally:
            dst.close()
            src.close()
    return base64.standard_b64encode(buf.getvalue()).decode("ascii")


def _pdf_digest(pdf_path: Path) -> str:
    import hashlib
    return hashlib.sha256(pdf_path.read_bytes()).hexdigest()


def _fixture_path(pdf_path: Path, model: str, kind: str) -> Path:
    return FIXTURE_DIR / pdf_path.stem / f"{model}.{kind}.json"


def _call_claude(client, model: str, pdf_b64: str, prompt: str, effort: str):
    kwargs = dict(
        model=model,
        max_tokens=16000,
        output_config={"effort": effort},
        output_format=WireExtraction,
        messages=[{"role": "user", "content": [
            {"type": "document", "source": {"type": "base64", "media_type": "application/pdf", "data": pdf_b64}},
            {"type": "text", "text": prompt},
        ]}],
    )
    if model in FALLBACK_MODELS:
        resp = client.beta.messages.parse(betas=["server-side-fallback-2026-07-01"], fallbacks="default", **kwargs)
    else:
        resp = client.messages.parse(**kwargs)
    if resp.stop_reason == "refusal":
        cat = resp.stop_details.category if resp.stop_details else None
        raise RuntimeError(f"{model} declined the extraction (refusal category={cat})")
    if resp.stop_reason == "max_tokens" or resp.parsed_output is None:
        raise RuntimeError(f"{model} returned no structured output (stop_reason={resp.stop_reason})")
    u = resp.usage
    usage = {"input_tokens": u.input_tokens, "output_tokens": u.output_tokens}
    return resp.parsed_output, usage, resp.model


PARSE_ATTEMPTS = 3


def _call_with_retry(client, model: str, pdf_b64: str, prompt: str, effort: str, verbose: bool):
    """_call_claude, retried up to PARSE_ATTEMPTS times on invalid (truncated) structured output.

    Records the attempt count in usage["attempts"]; raises RuntimeError when every attempt fails.
    """
    import sys

    from pydantic import ValidationError

    for attempt in range(1, PARSE_ATTEMPTS + 1):
        try:
            out, usage, served_by = _call_claude(client, model, pdf_b64, prompt, effort)
        except ValidationError as e:
            if attempt == PARSE_ATTEMPTS:
                raise RuntimeError(f"{model} returned invalid structured output {PARSE_ATTEMPTS} times") from e
            if verbose:
                print(f"  {model}: truncated structured output, retrying ({attempt}/{PARSE_ATTEMPTS})", file=sys.stderr)
            continue
        usage["attempts"] = attempt
        return out, usage, served_by


def run_llm_pdf(pdf_path: Path, page_numbers: list[int], prompt: str, model: str, kind: str,
                mode: str = "auto", effort: str = "medium", client=None, verbose: bool = False
                ) -> tuple[WireExtraction, dict]:
    """One structured-output call over the page subset. Returns (output, info).

    mode: live (call the API), record (call and save a fixture), replay (load the saved
    fixture), auto (live when credentials resolve, otherwise replay).
    """
    import json
    import sys

    fx = _fixture_path(pdf_path, model, kind)
    if mode in ("auto", "live", "record"):
        try:
            if client is None:
                import anthropic
                client = anthropic.Anthropic()
            if verbose:
                print(f"  {model}: live call, {len(page_numbers)} pages as a PDF document block", file=sys.stderr)
            out, usage, served_by = _call_with_retry(client, model, pdf_subset_b64(pdf_path, page_numbers), prompt,
                                                     effort, verbose)
        except TypeError as e:  # the SDK raises this when no API key / token / profile resolves
            if "authentication" not in str(e):
                raise
            if mode != "auto":
                raise SystemExit(f"--llm-mode {mode} needs Anthropic credentials: set ANTHROPIC_API_KEY in the same "
                                 "command, or run `ant auth login`") from None
            if verbose:
                print(f"  {model}: no Anthropic credentials, replaying {fx.name}", file=sys.stderr)
        else:
            info = {"source": "live", "model": served_by, "usage": usage}
            if mode == "record":
                fx.parent.mkdir(parents=True, exist_ok=True)
                fx.write_text(json.dumps({"model": model, "served_by": served_by, "kind": kind, "provenance": "live",
                                          "pdf_sha256": _pdf_digest(pdf_path), "pages": page_numbers, "usage": usage,
                                          "output": out.model_dump(mode="json")}, indent=2) + "\n")
            return out, info
    if not fx.exists():
        raise FileNotFoundError(f"no recorded response at {fx}; run live with ANTHROPIC_API_KEY set and --llm-mode record")
    rec = json.loads(fx.read_text())
    # Fixtures are keyed by filename stem, so a revised deed under the same name must not reuse old values.
    if rec.get("pdf_sha256") != _pdf_digest(pdf_path):
        raise ValueError(f"{fx.name} was recorded from a different PDF than {pdf_path.name} (sha256 mismatch); "
                         "re-record it")
    if rec["pages"] != page_numbers:
        raise ValueError(f"{fx.name} was recorded over pages {rec['pages']}, but this run selected {page_numbers}; "
                         "re-record it")
    return WireExtraction.model_validate(rec["output"]), {
        "source": f"replay ({rec.get('provenance', 'live')})", "model": rec["model"], "usage": rec.get("usage"),
        "fixture": str(fx)}


def _ev(c: Cite, page_map: list[int]) -> Evidence | None:
    if not c.snippet.strip():  # the model's "not stated" marker
        return None
    # A page outside the attachment maps to 0, which the verifier can never re-locate.
    return Evidence(page=page_map[c.page - 1] if 1 <= c.page <= len(page_map) else 0, snippet=c.snippet)


def _money(s: str | None) -> Decimal | None:
    if not s:
        return None
    try:
        return Decimal(re.sub(r"[£,\s]", "", s))
    except ArithmeticError:
        return None


def wire_housing(w: WireHousing, page_map: list[int]) -> HousingProvision:
    mix = None
    if w.mix_table.rows or w.mix_table.subtotals:
        t = w.mix_table
        mix = MixTable(rows=[MixRow(unit_type=r.unit_type, units_by_tenure={c.tenure: c.units for c in r.counts})
                             for r in t.rows],
                       subtotals={c.tenure: c.units for c in t.subtotals}, total_units=t.total_units,
                       total_habitable_rooms=t.total_habitable_rooms, evidence=_ev(t.evidence, page_map))
    return HousingProvision(
        total_units=w.total_units, total_units_evidence=_ev(w.total_units_evidence, page_map),
        market_units=w.market_units, market_units_evidence=_ev(w.market_units_evidence, page_map),
        affordable_units=w.affordable_units, affordable_units_evidence=_ev(w.affordable_units_evidence, page_map),
        affordable_habitable_rooms=w.affordable_habitable_rooms, total_habitable_rooms=w.total_habitable_rooms,
        tenures=[TenureAllocation(tenure=t.tenure, units=t.units, habitable_rooms=t.habitable_rooms,
                                  evidence=_ev(t.evidence, page_map)) for t in w.tenures],
        mix_table=mix,
        claimed_affordable_pct=_money(w.claimed_affordable_pct), claimed_affordable_basis=w.claimed_affordable_basis,
        claimed_affordable_evidence=_ev(w.claimed_affordable_evidence, page_map),
    )


def wire_contribution(w: WireContribution, page_map: list[int]) -> FinancialContribution:
    # The label comes from the cited phrase, not the model, so repeat runs agree; the
    # model's own label is kept only when the phrase decides differently.
    cited = classify_cited(w.trigger_evidence.snippet if w.trigger_evidence else None, w.trigger_clause)
    event = cited or w.trigger_event
    amount = _money(w.amount_gbp)
    return FinancialContribution(
        name=w.name, amount_gbp=amount, amount_in_words=w.amount_in_words, index_linked=w.index_linked,
        # "contingent" means no fixed figure (see CONTRIB_TASK): forced False whenever a figure was
        # extracted; with no figure it is still Claude's flag (contingent vs. a missed amount)
        is_cap=w.is_cap, contingent=amount is None and w.contingent, trigger_event=event,
        trigger_event_model=w.trigger_event if w.trigger_event != event else None, trigger_clause=w.trigger_clause,
        trigger_evidence=_ev(w.trigger_evidence, page_map), evidence=_ev(w.evidence, page_map))


def _describe(info: dict) -> str:
    u = info.get("usage") or {}
    tokens = f", {u['input_tokens']:,} in / {u['output_tokens']:,} out tokens" if u else ""
    return f"{info['model']} {info['source']}{tokens}"


GROUP_TERM = re.compile(r"Contributions\s*$", re.I)


def extract_llm_pdf(pdf_path: Path, pages: dict[int, str], base: Section106Agreement, model: str = PDF_DEFAULT_MODEL,
                    mode: str = "auto", client=None, verbose: bool = False) -> Section106Agreement:
    """Housing + contributions from the page images; metadata stays from the rules pass."""
    page_map = llm_pdf_pages(pages)
    prompt = PDF_PROMPT.format(task=HOUSING_TASK + "\n\n" + CONTRIB_TASK.format(which=ALL_CONTRIBUTIONS))
    w, info = run_llm_pdf(pdf_path, page_map, prompt, model, "extract", mode, client=client, verbose=verbose)
    out = base.model_copy(deep=True)
    out.housing = wire_housing(w.housing, page_map)
    # Plural defined terms ("Site and Development Contributions", "TfL Contributions") only group
    # other contributions for a shared trigger; some runs list them as obligations of their own.
    groups = [c.name for c in w.contributions if GROUP_TERM.search(c.name)]
    out.contributions = [wire_contribution(c, page_map) for c in w.contributions if c.name not in groups]
    if groups:
        out.warnings.append(f"llm-pdf: dropped group term(s) listed as contributions: {', '.join(groups)}")
    out.extraction_backend = f"llm-pdf:{model}" + ("" if info["source"] == "live" else " (replay)")
    out.warnings.append(f"llm-pdf: {_describe(info)}; pages {_ranges(page_map)}")
    return out


def _ranges(ns: list[int]) -> str:
    out, start = [], None
    for i, n in enumerate(ns):
        if start is None:
            start = n
        if i + 1 == len(ns) or ns[i + 1] != n + 1:
            out.append(f"{start}-{n}" if n != start else str(n))
            start = None
    return ",".join(out)


def _field_results(a: Section106Agreement, results) -> dict[str, list]:
    """{'housing' or contribution name: its CheckResults}."""
    out: dict[str, list] = {}
    for r in results:
        m = re.match(r"F(\d+)\.", r.check_id)
        out.setdefault(a.contributions[int(m.group(1)) - 1].name if m else "housing", []).append(r)
    return out


def _failed_fields(a: Section106Agreement, results) -> dict[str, int]:
    """{'housing' or contribution name: parser-error count}, omitting fields with none."""
    from verifier import ErrorClass

    counts = {k: sum(r.error_class == ErrorClass.PARSER_ERROR for r in rs) for k, rs in _field_results(a, results).items()}
    return {k: n for k, n in counts.items() if n}


HOUSING_VALUES = ("total_units", "total_units_evidence", "market_units", "market_units_evidence", "affordable_units",
                  "affordable_units_evidence", "affordable_habitable_rooms", "total_habitable_rooms",
                  "claimed_affordable_pct", "claimed_affordable_basis", "claimed_affordable_evidence")
CONTRIBUTION_VALUES = ("amount_gbp", "amount_in_words", "trigger_event", "trigger_clause", "trigger_evidence", "evidence")


def _lost(old, new, fields, prefix: str = "") -> list[str]:
    return [prefix + f for f in fields if getattr(old, f) is not None and getattr(new, f) is None]


def _lost_housing(old: HousingProvision, new: HousingProvision) -> list[str]:
    """Housing values present in `old` but missing from `new`. Tenures match by tenure, mix rows by unit type."""
    lost = _lost(old, new, HOUSING_VALUES)
    new_tenures = {t.tenure: t for t in new.tenures}
    for t in old.tenures:
        label = f"tenures[{t.tenure.value}]"
        n = new_tenures.get(t.tenure)
        lost += [label] if n is None else _lost(t, n, ("units", "habitable_rooms", "evidence"), label + ".")
    if old.mix_table:
        om, nm = old.mix_table, new.mix_table
        if nm is None:
            return lost + ["mix_table"]
        lost += _lost(om, nm, ("total_units", "total_habitable_rooms", "evidence"), "mix_table.")
        unit = lambda s: re.sub(r"\W", "", s.lower())  # "1 bed" == "1-bed"
        new_rows = {unit(r.unit_type): r.units_by_tenure for r in nm.rows}
        for r in om.rows:
            have = new_rows.get(unit(r.unit_type), {})
            lost += [f"mix_table[{r.unit_type}][{t.value}]" for t in r.units_by_tenure if t not in have]
        lost += [f"mix_table.subtotals[{t.value}]" for t in om.subtotals if t not in nm.subtotals]
    return lost


def _regressions(key: str, cur, cur_results, cand, cand_results) -> list[str]:
    """Why `cand` is not a safe replacement for `key`, or [] if it is.

    Fewer Parser Errors alone isn't enough: dropping a value can turn a FAIL into a SKIP.
    So the field must keep its check coverage and must not lose any extracted value.
    """
    from verifier import Status

    before, after = _field_results(cur, cur_results).get(key, []), _field_results(cand, cand_results).get(key, [])
    why = []
    if _failed_fields(cand, cand_results).get(key, 0) >= _failed_fields(cur, cur_results).get(key, 0):
        why.append("no fewer Parser Errors")
    if sum(r.status != Status.SKIP for r in after) < sum(r.status != Status.SKIP for r in before):
        why.append("fewer checks run")
    if key == "housing":
        lost = _lost_housing(cur.housing, cand.housing)
    else:
        old, new = (next(c for c in x.contributions if c.name == key) for x in (cur, cand))
        if new.contingent and not old.contingent:
            why.append("newly marked contingent")
        lost = _lost(old, new, CONTRIBUTION_VALUES)
    if lost:
        why.append("lost " + ", ".join(lost))
    return why


def cascade_llm_pdf(pdf_path: Path, a: Section106Agreement, pages: dict[int, str], results,
                    model: str = CASCADE_MODEL, mode: str = "auto", client=None, verbose: bool = False
                    ) -> Section106Agreement:
    """Re-read only the fields with Parser Errors on a stronger model.

    A replacement is kept only if the verifier then reports fewer Parser Errors for that
    field, runs at least as many of its checks, and no previously extracted value is lost
    (see `_regressions`). The verifier stays the judge.
    """
    import hashlib

    from verifier import verify

    before = _failed_fields(a, results)
    if not before:
        return a
    names = sorted(k for k in before if k != "housing")
    tasks = [HOUSING_TASK] if "housing" in before else []
    if names:
        tasks.append(CONTRIB_TASK.format(which="only these defined terms: " + "; ".join(f'"{n}"' for n in names)))
    prompt = PDF_PROMPT.format(task="A previous extraction of this deed failed verification. Re-read only the "
                               "fields below; if housing is not listed, return it with every value null and empty lists.\n\n" + "\n\n".join(tasks))
    kind = "cascade-" + hashlib.sha1("|".join(sorted(before)).encode()).hexdigest()[:8]
    page_map = llm_pdf_pages(pages)
    w, info = run_llm_pdf(pdf_path, page_map, prompt, model, kind, mode, effort="high", client=client,
                          verbose=verbose)

    out, out_results = a.model_copy(deep=True), results
    kept, rejected = [], []

    def try_replace(key: str, apply) -> None:
        nonlocal out, out_results
        cand = out.model_copy(deep=True)
        apply(cand)
        cand_results = verify(cand, pages)
        why = _regressions(key, out, out_results, cand, cand_results)
        if why:
            rejected.append(f"{key}: {'; '.join(why)}")
        else:
            out, out_results = cand, cand_results
            kept.append(key)

    if "housing" in before:
        try_replace("housing", lambda c: setattr(c, "housing", wire_housing(w.housing, page_map)))
    by_name = {squash_name(c.name): c for c in w.contributions}
    for name in names:
        new = by_name.get(squash_name(name))
        if new is None:
            rejected.append(f"{name}: not returned")
            continue
        idx = next(i for i, c in enumerate(out.contributions) if c.name == name)

        def apply(c, idx=idx, new=new, name=name):
            c.contributions[idx] = wire_contribution(new, page_map).model_copy(update={"name": name})
        try_replace(name, apply)

    out.extraction_backend += f" + cascade:{model}"
    parts = [f"cascade: {_describe(info)}", f"re-read {len(before)} field(s) with Parser Errors"]
    parts.append(f"kept {len(kept)}" + (f" ({', '.join(kept)})" if kept else ""))
    if rejected:
        parts.append(f"rejected {len(rejected)} ({' | '.join(rejected)})")
    out.warnings.append("; ".join(parts))
    return out


def squash_name(s: str) -> str:
    return re.sub(r"[^a-z]", "", s.lower())


# --------------------------------------------------------------------------- entry point

def extract(pdf_path: str | Path, backend: str = "rules", model: str | None = None, verbose: bool = False,
            llm_mode: str = "auto", client=None) -> tuple[Section106Agreement, dict[int, str]]:
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
    if backend == "llm-pdf":
        agreement = extract_llm_pdf(pdf_path, pages, agreement, model=model or PDF_DEFAULT_MODEL, mode=llm_mode,
                                    client=client, verbose=verbose)
    if verbose:
        import sys
        print(f"  definitions: pages {def_pages[0]}-{def_pages[-1]} ({len(defs)} terms); "
              f"schedules from page {sched_pages[0] if sched_pages else '?'}", file=sys.stderr)
    return agreement, pages
