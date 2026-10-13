"""Deterministic payment-trigger classification.

The label for a payment trigger is a function of the cited trigger phrase, not of
whichever backend (or model run) read the deed. Both backends cite the phrase as
`trigger_evidence`; `classify` maps it to a TriggerEvent with a fixed vocabulary,
so the same phrase always gets the same label.

Matching runs on squashed text (lowercase alphanumerics only) because OCR drops
spaces ("onorbeforeOccupation"). The earliest match in the phrase wins; ties go
to the longest match. Order of PATTERNS does not matter.
"""
from __future__ import annotations

import re

from schema import TriggerEvent

_NUM = (r"(?:\d+|(?:one|two|three|four|five|six|seven|eight|nine|ten|eleven|twelve|thirteen|fourteen|"
        r"fifteen|sixteen|seventeen|eighteen|nineteen|twenty|thirty|forty|fifty|sixty|ninety)+)")
_BEFORE = r"(?:priorto|onorbefore|before|upon|on)(?:the)?"

PATTERNS: list[tuple[TriggerEvent, str]] = [
    (TriggerEvent.PRE_IMPLEMENTATION, _BEFORE + r"implementation"),
    (TriggerEvent.PRE_IMPLEMENTATION, r"not(?:to)?implement"),
    (TriggerEvent.PRE_COMMENCEMENT, _BEFORE + r"commencement"),
    (TriggerEvent.PRE_COMMENCEMENT, r"not(?:to)?commence"),
    (TriggerEvent.PRE_OCCUPATION, _BEFORE + r"(?:first)?occupation"),
    (TriggerEvent.PRE_OCCUPATION, r"not(?:to)?occupy"),
    # completion/date of *this agreement or deed* only; "on completion of construction" is not it
    (TriggerEvent.ON_COMPLETION, r"(?:on|upon)(?:the)?(?:completion|date)of(?:this)?(?:agreement|deed)"),
    (TriggerEvent.ON_COMPLETION, r"(?:on|upon)(?:the)?(?:completion|date)hereof"),
    (TriggerEvent.ON_DEMAND, r"within" + _NUM + r"(?:working)?days?"),
    (TriggerEvent.ON_DEMAND, r"ondemand"),
    (TriggerEvent.OTHER, r"inaccordancewiththeprocedure"),
]
_COMPILED = [(event, re.compile(p)) for event, p in PATTERNS]


# Wording that imposes a payment, as opposed to merely mentioning one ("the Council shall assess
# the amount payable" schedules an assessment). Matched on squashed text, like PATTERNS.
_PAYER = r"(?:shall|will|must|agrees?to|covenants?(?:with\w{0,40}?)?to|undertakes?to)"
OBLIGATIONS = [  # (pattern, bare): bare "is payable" also gets the conditional-"if" check
    (_PAYER + r"(?:pay|bepaid)", False),                                   # "the Owner shall pay"
    (r"(?:shall|will)(?:then)?(?:be|become)(?:due(?:and)?)?payable", False),  # "shall be payable"
    (r"(?:is|are|becomes?)(?:due(?:and)?)?payable", True),                 # "is payable"
    (r"until\w*(?:has|have)beenpaid", False),                              # "until ... have been paid"
]
_OBLIGATIONS = [(re.compile(p), bare) for p, bare in OBLIGATIONS]
# Covenant list items ("1.3 To pay the ...", "Unless ..., to pay the ...") need the clause or
# comma boundary, which squashing removes, so this one runs on the raw text.
_COVENANT_ITEM = re.compile(r"(?:^|[,;:])\s*(?:\d[\d.]*\s*)?(?:\([a-z0-9]+\)\s*)?to\s*pay", re.I | re.M)
# Sentence or sub-clause breaks; a full stop inside a number ("1.3", "£10,058.60") is not one.
_SEGMENT = re.compile(r"(?<!\d)\.(?!\d)|;")
# Question/assessment wording before a payment phrase ("assess whether ... is payable",
# "determine if ... is payable"). Squashed, so it holds whether or not OCR kept the spaces;
# "if" only counts straight after an assessment verb, since squashed "if" also sits inside
# words ("specified").
_QUESTION = re.compile(r"whether|(?:assess|determin|consider|decid|review|confirm|establish|calculat|verif"
                       r"|check|ascertain)\w{0,3}?if")
# A segment opening with a conditional "If ..." (after any "3.1" / "(a)" numbering) makes a bare
# "is payable" a condition, not an obligation. Anchored at the start, so "Ifthe" still counts.
_CONDITIONAL = re.compile(r"\s*(?:\d[\d.]*\s*)?(?:\([a-z0-9]+\)\s*)?if(?!\s*any)", re.I)


def _asked(seg: str, pos: int, bare: bool) -> bool:
    """Is the payment phrase at `pos` of squash(seg) asked about rather than imposed?"""
    return bool(_QUESTION.search(squash(seg)[:pos]) or (bare and _CONDITIONAL.match(seg)))


def imposes_payment(clause: str | None) -> bool:
    """Does the clause oblige someone to pay (or make a sum payable), not just mention or ask about it?"""
    for seg in _SEGMENT.split(clause or ""):
        sq = squash(seg)
        for rx, bare in _OBLIGATIONS:
            if any(not _asked(seg, m.start(), bare) for m in rx.finditer(sq)):
                return True
        for m in _COVENANT_ITEM.finditer(seg):
            if not _QUESTION.search(squash(seg[:m.start()])):
                return True
    return False


def squash(s: str) -> str:
    return re.sub(r"[^0-9a-z]", "", s.lower())


def classify(text: str | None) -> TriggerEvent | None:
    """TriggerEvent for the first trigger phrase in `text`, or None if none is recognised."""
    if not text:
        return None
    s = squash(text)
    best: tuple[int, int, TriggerEvent] | None = None
    for event, rx in _COMPILED:
        m = rx.search(s)
        if m and (best is None or (m.start(), -(m.end() - m.start())) < (best[0], best[1])):
            best = (m.start(), -(m.end() - m.start()), event)
    return best[2] if best else None


def classify_cited(phrase: str | None, clause: str | None) -> TriggerEvent | None:
    """Label from the cited phrase, falling back to the surrounding clause."""
    return classify(phrase) or classify(clause)
